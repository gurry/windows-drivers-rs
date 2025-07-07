//! A sample driver written in 100% safe Rust.
//! Demonstrates request processing and cancellation.
//! 
//! When a write request arrives it stores the request
//! in context object and starts a timer. When the timer
//! fires it completes the request. This simulates I/O
//! processing on real hardware. At any time before its
//! completion the request can be cancelled. Cancellation is
//! supported through the request cancellation callback.
//! 
//! This driver uses safe Rust abstractions provided by the
//! `wdf` crate located at the path `../../crates/wdf` relative
//! to this directory.
//! 
//! The design of everything over here and in the `wdf` crate is
//! at a very early stage. Some parts may appear subotimal or even
//! wrong. That is likely to change and improve over time.

#![no_std]

use wdf::{
    Arc, driver_entry, object_context, println, trace, CancellableMarkedRequest, Request,
    RequestCancellationToken, Device, DeviceInit, Driver, Guid, IoQueue,
    IoQueueConfig, NtError, NtResult, NtStatus, PnpPowerEventCallbacks, SpinLock, Timer,
    TimerConfig
};

use core::time::Duration;

/// Context object to be attached to a queue
#[object_context(IoQueue)]
struct QueueContext {
    // Field that stores the in-flight request.
    // The spin lock prevents concurrency issues
    // between request completion and cancellation.
    // The lock is enforced at compile time (i.e. the
    // code will fail to compile if you do not use
    // the lock).
    request: SpinLock<Option<CancellableMarkedRequest>>,

    // The timer that is used to complete the request
    timer: Arc<Timer>
}

/// Context object to be attached to a timer
#[object_context(Timer)]
struct TimerContext {
    queue: Arc<IoQueue>
}

/// The entry point for the driver
/// 
/// The #[driver_entry] attribute is used to mark the entry point.
/// It is a proc macro that generates the shim code which enables WDF
/// to call this driver
#[driver_entry]
fn driver_entry(driver: &mut Driver, registry_path: &str) -> Result<(), NtError> {
    if cfg!(debug_assertions) {
        print_driver_version(driver)?;
    }

    println!("Registry path: {registry_path}");

    // Set up the device add callback
    driver.on_evt_device_add(evt_device_add);

    // Enable tracing
    let control_guid = Guid::parse("cb94defb-592a-4509-8f2e-54f204929669").expect("GUID is valid");
    driver.enable_tracing(control_guid);

    trace("Trace: Safe Rust driver entry complete");

    Ok(())
}

/// Callback that is called when a device is added
fn evt_device_add(device_init: &mut DeviceInit) -> Result<(), NtError> {
    println!("evt_device_add called");

    // Create device
    let mut pnp_power_callbacks = PnpPowerEventCallbacks::default();
    pnp_power_callbacks.evt_device_self_managed_io_init = Some(evt_device_self_managed_io_start);
    pnp_power_callbacks.evt_device_self_managed_io_suspend = Some(evt_device_self_managed_io_suspend);
    pnp_power_callbacks.evt_device_self_managed_io_restart = Some(evt_device_self_managed_io_start);

    let device = Device::create(device_init, Some(pnp_power_callbacks))?;

    // Create queue
    let mut queue_config = IoQueueConfig::default();

    queue_config.default_queue = true;
    queue_config.evt_io_write = Some(evt_io_write);

    let queue = IoQueue::create(&device, &queue_config)?; // The `?` operator is used to propagate errors to the caller

    // Create timer
    let timer_config = TimerConfig::new_periodic(&queue, evt_timer, 10_000, 0, false);

    let timer = Timer::create(&timer_config)?;

    // Attach context to the timer
    let timer_context = TimerContext {
        queue: queue.clone()
    };

    TimerContext::attach(&timer, timer_context)?;

    // Attach context to the queue
    let queue_context = QueueContext {
        request: SpinLock::create(None)?,
        timer
    };

    QueueContext::attach(&queue, queue_context)?;

    // Create device interface
    let _ = device.create_interface(
        &Guid::parse("2aa02ab1-c26e-431b-8efe-85ee8de102e4").expect("GUID is valid"),
        None
    )?; 

    trace("Trace: Safe Rust device add complete");
    Ok(())
}

/// Callback for starting self-managed I/O
fn evt_device_self_managed_io_start(device: &Device) -> NtResult<()>{
    println!("Self-managed I/O start called: {:?}", device);

    let queue = device.get_default_queue().
        expect("Failed to get default queue");

    queue.start();

    let context = QueueContext::get(&queue)
        .expect("Failed to get queue context"); 

    let _ = context.timer.start(&Duration::from_millis(100));

    Ok(())
}

/// Callback for stopping self-managed I/O
fn evt_device_self_managed_io_suspend(device: &Device) -> NtResult<()> {
    println!("Self-managed I/O suspend called: {:?}", device);

    let queue = device.get_default_queue().
        expect("Failed to get default queue");

    queue.stop_synchronously();

    let context = QueueContext::get(&queue)
        .expect("Failed to get queue context"); 

    context.timer.stop(false);

    Ok(())
}

/// Callback that is called when a write request is received
fn evt_io_write(queue: &IoQueue, request: Request, _length: usize) {
    println!("evt_io_write called");

    if let Some(context) = QueueContext::get(&queue) {
        println!("Request processing started");

        match request.mark_cancellable(evt_request_cancel) {
            Ok(cancellable_req) => {
                *context.request.lock() = Some(cancellable_req);

                println!("Request marked as cancellable");
            }
            Err(e) => {
                println!("Failed to mark request as cancellable: {e:?}");
            }
        }
    } else {
        println!("Failed to get queue context");
    }
}

/// Callback that is called when the request is cancelled.
/// It cancels the request identified by the `token` parameter
/// if it is found in the context.
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

/// Callback that is called when the timer fires.
/// It fetches the request stored in the context
/// and completes it
fn evt_timer(timer: &Timer) {
    println!("evt_timer called");

    let queue = &TimerContext::get(timer)
        .expect("Failed to get timer context")
        .queue;

    let req = QueueContext::get(queue)
        .and_then(|context| context.request.lock().take());

    if let Some(req) = req {
        req.complete(NtStatus::Success);
        println!("Request completed");
    } else {
        println!("No request pending");
    }
}

/// This routine shows how to retrieve framework version string and
/// also how to find out to which version of framework library the
/// client driver is bound to.
fn print_driver_version(driver: &Driver) -> NtResult<()> {
    let driver_version = driver.retrieve_version_string()?;
    println!("Echo Sample {driver_version}");

    if driver.is_version_available(1, 0) {
        println!("Yes, framework version is 1.0");
    } else {
        println!("No, framework verison is not 1.0");
    }

    Ok(())
}