//! A Sample KMDF driver implemented in safe Rust

#![no_std]

use wdf::{
    driver_entry, object_context, println, trace, CancellableMarkedRequest, Device,
    DeviceInit, Driver, Guid, IoQueue, IoQueueConfig, NtError, NtStatus, SpinLock,
    Timer, TimerConfig
};

#[object_context(IoQueue)]
struct QueueContext {
    request: SpinLock<Option<CancellableMarkedRequest>>,
    timer: Timer
}

#[driver_entry]
fn driver_entry(driver: &mut Driver, registry_path: &str) -> Result<(), i32> {
    println!("Safe Rust driver entry called. Registry path: {registry_path}");

    driver.on_evt_device_add(device_add);

    let control_guid = Guid::parse("cb94defb-592a-4509-8f2e-54f204929669").expect("GUID is valid");
    driver.enable_tracing(control_guid);

    trace("Trace: Safe Rust driver entry complete");

    Ok(())
}

fn device_add(device_init: &mut DeviceInit) -> Result<(), NtError> {
    println!("Safe Rust device add called");

    let device = Device::create(device_init)?;

    let mut queue_config = IoQueueConfig::default();


    queue_config.evt_io_read = Some(|queue, request, _| {
        println!("Safe Rust evt_io_read called");

        if let Some(context) = QueueContext::get(&queue) {
            println!("Request processing started");

            match request.mark_cancellable(|token| {
                println!("Request evt_cancel called");

                let queue = token.get_io_queue();

                if let Some(context) = QueueContext::get(&queue) {
                    let mut req = context.request.lock();
                    if let Some(req) = req.take() {
                        match req.unmark_cancellable() {
                            Ok(req) => {
                                req.complete(NtStatus::cancelled());
                                println!("Request cancelled");
                            }
                            Err(e) => {
                                println!("Failed to unmark request as cancellable: {e:?}");
                            }
                        }
                    }
                } else {
                    println!("Could not cancel request. Failed to get queue context");
                }
            }) {
                Ok(cancellable_req) => {
                    *context.request.lock() = Some(cancellable_req);
                    let _ = context.timer.start(5000);

                    println!("Request marked as cancellable");
                }
                Err(e) => {
                    println!("Failed to mark request as cancellable: {e:?}");
                }
            }
        } else {
            println!("Failed to get queue context");
        }
    });

    let mut queue = IoQueue::create(&device, &queue_config)?;

    let timer_config = TimerConfig::new_non_periodic(&queue, |timer| {
        println!("Safe Rust evt_timer_func called");
        if let Some(queue) = timer.get_parent_object::<IoQueue>() {
            let context = QueueContext::get(&queue).unwrap();
            let mut req = context.request.lock();
            if let Some(req) = req.take() {
                match req.unmark_cancellable() {
                    Ok(req) => {
                        req.complete(NtStatus::cancelled());
                        println!("Request cancelled");
                    }
                    Err(e) => {
                        println!("Failed to unmark request as cancellable: {e:?}");
                    }
                };
            }
        }

        timer.stop(false);
    });

    let timer = Timer::create(&timer_config)?;

    let context = QueueContext {
        request: SpinLock::create(None)?,
        timer
    };

    QueueContext::attach(&mut queue, context)?;

    trace("Trace: Safe Rust device add complete");
    Ok(())
}