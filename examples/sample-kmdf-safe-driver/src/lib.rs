#![no_std]
#![allow(missing_docs)]

use wdf::{
    driver_entry, object_context, println, CancellableMarkedRequest, Request,
    RequestCancellationToken, Device, DeviceInit, Driver, Guid, IoQueue,
    IoQueueConfig, NtError, NtStatus, SpinLock, Timer, TimerConfig
};

use core::time::Duration;

#[object_context(IoQueue)]
struct QueueContext {
    request: SpinLock<Option<CancellableMarkedRequest>>,

    // The timer that is used to complete the request
    timer: Timer
}

#[object_context(Device)]
struct DeviceContext {
    queue: SpinLock<Option<IoQueue>>,
    second_queue: IoQueue,
}

#[driver_entry]
fn driver_entry(driver: &mut Driver, registry_path: &str) -> Result<(), NtError> {
    println!("Safe Rust driver entry called. Registry path: {registry_path}");

    // Set up the device add callback
    driver.on_evt_device_add(evt_device_add);

    // Enable tracing
    let control_guid = Guid::parse("cb94defb-592a-4509-8f2e-54f204929669").expect("GUID is valid");
    driver.enable_tracing(control_guid);

    Ok(())
}

/// Callback that is called when a device is added
fn evt_device_add(device_init: &mut DeviceInit) -> Result<(), NtError> {
    println!("evt_device_add called");

    // Create device
    let mut device = Device::create(device_init)?;

    // Create queue
    let mut queue_config = IoQueueConfig::default();

    queue_config.default_queue = true;
    queue_config.evt_io_write = Some(evt_io_write);

    let mut queue = IoQueue::create(&device, &queue_config)?; // The `?` operator is used to propagate errors to the caller

    let timer_config = TimerConfig::new_non_periodic(&queue, evt_timer);

    let timer = Timer::create(&timer_config)?;

    let queue_context = QueueContext {
        request: SpinLock::create(None)?,
        timer
    };

    QueueContext::attach(&mut queue, queue_context)?;


    let second_queue = IoQueue::create(&device, &queue_config)?; // The `?` operator is used to propagate errors to the caller

    let device_context = DeviceContext {
        queue: SpinLock::create(Some(queue))?,
        second_queue,
    };

    DeviceContext::attach(&mut device, device_context)?;

    Ok(())
}

fn evt_io_write(queue: &mut IoQueue, request: Request, _length: usize) {
    println!("evt_io_write called");

    if let Some(context) = QueueContext::get(&queue) {
        println!("Request processing started");

        match request.mark_cancellable(evt_request_cancel) {
            Ok(cancellable_req) => {
                *context.request.lock() = Some(cancellable_req);
                let _ = context.timer.start(&Duration::from_secs(5));

                println!("Request marked as cancellable");
            }
            Err(e) => {
                println!("Failed to mark request as cancellable: {e:?}");
            }
        }
    } else {
        println!("Queue context not found, forwarding request to second queue");

        let device = queue.get_device();
        let device_context = DeviceContext::get(&device).unwrap();
        let second_queue = &device_context.second_queue;
        let _ = request.forward_to_queue(second_queue);
    }
}

fn evt_request_cancel(token: &RequestCancellationToken) {
    println!("evt_request_cancel called");

    let queue = token.get_io_queue();

    if let Some(context) = QueueContext::get(&queue) {
        let mut req = context.request.lock();
        if let Some(req) = req.take() {
            req.complete(NtStatus::cancelled());
            println!("Request cancelled");
        } else {
            println!("Request already completed");
        }
    } else {
        println!("Could not cancel request. Failed to get queue context");
    }
}

fn evt_timer(timer: &mut Timer) {
    println!("evt_timer called");

    let device = timer.get_device();
    let device_context = DeviceContext::get(&device).unwrap();
    let queue = device_context.queue.lock();
    if let Some(queue) = queue.as_ref() {
        let queue_context = QueueContext::get(queue).unwrap();

        let req = queue_context.request.lock().take();
        if let Some(req) = req {
            req.complete(NtStatus::Success);
            println!("Request completed");
        } else {
            println!("Request already cancelled or completed");
        }
    }

    timer.stop(false);
}