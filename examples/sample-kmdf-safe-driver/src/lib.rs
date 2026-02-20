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

use core::time::Duration;

use wdf::{
    driver_entry,
    object_context,
    Opaque,
    println,
    status_codes,
    trace,
    Arc,
    CancellableRequest,
    Device,
    DeviceInit,
    Driver,
    Guid,
    IoQueue,
    IoQueueConfig,
    IoQueueDispatchType,
    NtResult,
    PnpPowerEventCallbacks,
    Request,
    RequestCancellationToken,
    SpinLock,
    Timer,
    TimerConfig,
};

extern crate alloc;
use alloc::{vec, vec::Vec};

const MAX_WRITE_LENGTH: usize = 1024 * 40;

///---- These traits and functions are part of the wdf crate/shim layer that we supply ----

pub trait Driver {    
    fn evt_device_add(&self, device_init: &mut DeviceInit) -> NtResult<()> {
        Ok(())
    }
}

fn driver_register<T: Driver>(driver: T) -> NtResult<&mut T> {
    //...
    // Similar to `device_register()` below 
}


pub trait Device {
    fn evt_device_self_managed_io_init(&self) -> NtResult<()> {
        // These are empty default implementations of callbacks.
        // (think of them as virtual methods in a base class)
        // It gives the user flexibility to not implement
        // some callbacks. For any ones the user doesnot implement
        // these default implementations get called instead       
        Ok(()) 
    }
    fn evt_device_self_managed_io_suspend(&self) -> NtResult<()> {
        Ok(())
    }
    fn evt_device_self_managed_io_restart(&self) -> NtResult<()> {
        Ok(())
    }

    // Others callbacks ...
}

// Creates WDFDEVICE wires up its callbacks with
fn device_register<'a, T: Device>(user_device: T, device_init: &DeviceInit) -> NtResult<&'a mut T> {
    // Set up raw callbacks (e.g. _evt_self_managed_io_suspend declared below)
    unsafe { WdfDeviceInitSetPnpPowerEventCallbacks, device_init.as_ptr_mut(), ...) };
    let attributes = // Set up attributes with the required size of context
       
    // Create WDFDEVICE
    let device = unsafe { WdfDeviceCreate(device_init.as_ptr(), attributes, ...) }; 
    
    
    // Create a Rust object for hidden context and store `user_device` in it
    let rust_ctxt  = HiddenDeviceCtxt { user_device };

    // Get the raw context memory from WDFDEVICE
    // and move the Rust context into it
    let raw_ctxt = // Get raw context from WDFDEVICE
    // Using `core::ptr::write` stops the compiler
    // from calling `drop()` on `rust_ctxt`
    unsafe { core::ptr::write(raw_ctxt, rust_ctxt) };

    // Return a mutable ref to `user_device` for the
    // user to use further
    Ok(&mut user_device)
}

struct HiddenDeviceCtxt<T: Device> {
    user_device: T
}

// Raw callback which the framework calls. It finds 
// the user's device object and forwards the call to
// the corresponding callback trait method
unsafe extern "C" fn _evt_self_managed_io_suspend(device: WDFDEVICE, ...) {
    let ctxt = // get context from WDFDEVICE in which we stored user's device 
    let user_device = ctxt.device; // Get user's device from context
    
    // Convert arguments from raw to safe objects here.

    // Call the user's callback method
    user_device.self_managed_io_suspend(... safe arguments ...);
}

pub trait IoQueue {
    fn evt_io_read(&self, request: Request, length: usize) {
        Ok(())
    }
    fn evt_io_write(&self, request: Request, length: usize) {
        Ok(())
    }
    
    // Others callbacks ...
}

fn queue_register<T: IoQueue>(queue: T, device: &Device, config: &IoQueueConfig) -> NtResult<Arc<T>> {
    //...
    // Similar to `device_register()` above
}


pub trait Timer {
    fn evt_timer(&self) {
        Ok(())
    }
}

fn timer_register<T: Timer>(timer: T, config: &TimerConfig) -> NtResult<Arc<T>> {
    //...
    // Similar to `device_register()` above
}



/// ----- User's driver code ------

/// The entry point for the driver. It initializes the driver and is the first
/// routine called by the system after the driver is loaded. `driver_entry`
/// specifies the other entry points in the function driver such as
/// `evt_device_add`.
///
/// The #[driver_entry] attribute is used to mark the entry point.
/// It is a proc macro that generates the shim code which enables WDF
/// to call this driver
///
/// # Arguments
///
/// * `driver` - Represents the instance of the function driver that is loaded
/// into memory. `driver` object is allocated by the system before the
/// driver is loaded, and it is released by the system after the system unloads
/// the function driver from memory.
///
/// * `registry_path` - Represents the driver specific path in the Registry.
/// The function driver can use the path to store driver related data between
/// reboots. The path does not store hardware instance specific data.
#[driver_entry(tracing_control_guid = "cb94defb-592a-4509-8f2e-54f204929669")]
fn driver_entry(driver: &mut Driver, _registry_path: &str) -> NtResult<()> {
    let my_driver = MyDriver {};
    let _ = driver_register(my_driver)?;
    
    trace("Trace: Safe Rust driver entry complete");

    Ok(())
}
                
struct MyDriver;

/// Driver impl for WDF callbacks
impl Driver for MyDriver {
    /// `evt_device_add` is called by the framework in response to AddDevice
    /// call from the PNP manager. We create and initialize a device object to
    /// represent a new instance of the device.
    ///
    /// # Arguments
    ///
    /// * `device_init` - Reference to a framework-allocated `DeviceInit` structure.
    fn evt_device_add(&self, device_init: &mut DeviceInit) -> NtResult<()> {
        println!("Enter evt_device_add");
        Self::set_up_device(device_init)
    }
}


/// Inherent impl for some helper methods
impl MyDriver {
    /// Worker routine called to create a device and its software resources.
    ///
    /// # Arguments
    ///
    /// * `device_init` - Pointer to an opaque init structure. Memory for
    /// this structure will be freed by the framework when the
    /// WdfDeviceCreate succeeds. So don't access the structure after
    /// that point.
    fn set_up_device(device_init: &mut DeviceInit) -> NtResult<()> { 
        let my_device = MyDevice { default_queue: None };
        let my_device = device_init(my_device, device_init)?
       
        // Create a device interface so that applications can find us and talk
        // to us.
        let _ = device.create_device_interface(
            &Guid::parse("2aa02ab1-c26e-431b-8efe-85ee8de102e4").expect("GUID is valid"),
            None,
        )?;

        Self::set_up_queue(&my_device)
    }

    /// The I/O dispatch callbacks for the frameworks device object
    /// are configured in this function.
    ///
    /// A single default I/O Queue is configured for serial request
    /// processing, and queue context is set up. The lifetime of the
    /// context is tied to the lifetime of the I/O Queue object.
    ///
    /// # Arguments
    ///
    /// * `device`` - Handle to a framework device object.
    fn set_up_queue(device: &MyDevice) -> NtResult<()> {
        // Create timer
        let timer_config = TimerConfig::new_periodic(&queue, 9_000, 0, false);

        let timer = timer_register(MyTimer {
            queue: None,
        });

        // Create queue
        let mut queue_config = IoQueueConfig::new_default(IoQueueDispatchType::Sequential);
        queue_config.default_queue = true; 

        let queue = queue_register(MyQueue {
            request: SpinLock::create(None)?,
            buffer: SpinLock::create(None)?,
            timer,
        }, &device, &queue_config)?;

        timer.queue = Some(queue.clone());

        device.default_queue = Some(queue.clone());
        
        Ok(())
    }
}

struct MyDevice {
    default_queue: Option<Arc<IoQueue>>
}

impl Device for MyDevice {
    /// This callback is called by the Framework when the device is started
    /// or restarted after a suspend operation.
    /// # Arguments
    ///
    /// * `device` - Handle to the device
    fn evt_self_managed_io_start(&self) -> NtResult<()> {
        println!("Self-managed I/O start called: {:?}", device);

        let queue = self
            .default_queue
            .expect("Failed to get default queue");

        queue.start();

        let _ = queue.timer.start(&Duration::from_millis(100));

        Ok(())
    }

    /// This callback is called by the Framework when the device is stopped
    /// for resource rebalance or suspended when the system is entering
    /// Sx state.
    ///
    /// # Arguments
    ///
    /// * `device` - Handle to the device
    fn evt_self_managed_io_suspend(&self) -> NtResult<()> {
        println!("Self-managed I/O suspend called: {:?}", self);

        let queue = self
            .default_queue
            .expect("Failed to get default queue");

        queue.stop_synchronously();

        let context = QueueContext::get(&queue);

        context.timer.stop(false);

        Ok(())
    }
}


/// Context object to be attached to a queue
struct MyQueue {
    // Field that stores the in-flight request.
    // The spin lock prevents concurrency issues
    // between request completion and cancellation.
    // The lock is enforced at compile time (i.e. the
    // code will fail to compile if you do not use
    // the lock).
    request: SpinLock<Option<CancellableRequest>>,

    // Buffer where data from incoming write request is stored
    buffer: SpinLock<Option<Vec<u8>>>,

    // The timer that is used to complete the request
    timer: Arc<Timer>,
}

impl IoQueue for MyQueue {
    /// This callback is invoked when the framework receives IRP_MJ_READ request.
    /// It copies the data from the queue context buffer to the request buffer.
    /// If the driver hasn't received any write request earlier, it returns 0.
    /// The actual completion of the request is deferred to the periodic timer.
    ///
    /// # Arguments
    ///
    /// * `queue` - Handle to the framework queue object that is associated with the
    ///   I/O request.
    /// * `Request` - Handle to a framework request object.
    ///
    /// * `Length`  - number of bytes to be read. The default property of the queue
    ///   is to not dispatch zero length read & write requests to the driver and
    ///   complete is with status success. So we will never get a zero length
    ///   request.
    fn evt_io_read(&self, mut request: Request, length: usize) {
        println!("evt_io_read called. Queue {self:?}, Request {request:?} Length {length}");

        let memory = match request.retrieve_output_memory() {
            Ok(memory) => memory,
            Err(e) => {
                println!("evt_io_read could not get request memory buffer {e:?}");
                request.complete(e.into());
                return;
            }
        };

        // Nested scope to limit the lifetime of the lock
        let length = {
            // TODO: this lock is problematic because we call out into
            // the framework while holding it. Doing so is generally
            // considered a recipe for deadlocks although copy_from_buffer
            // specifically won't cause any. Still this is a bad pattern
            // in general and we have to find a way to avoid it.
            let buffer = self.buffer.lock();
            let Some(buffer) = buffer.as_ref() else {
                println!("evt_io_read called but no request buffer is set");
                request.complete_with_information(status_codes::STATUS_SUCCESS.into(), 0);
                return;
            };

            let mut length = length;
            if buffer.len() < length {
                length = buffer.len();
            }

            if let Err(e) = memory.copy_from_buffer(0, buffer) {
                println!("evt_io_read failed to copy buffer: {e:?}");
                request.complete(e.into());
                return;
            }

            length
        };

        request.set_information(length);

        if let Err((e, request)) = request.mark_cancellable(evt_request_cancel, self.request) {
            println!("evt_io_write failed to mark request cancellable: {e:?}");
            request.complete(status_codes::STATUS_UNSUCCESSFUL.into()); // TODO: decide on the status code here
        }
    }

    /// This callback is invoked when the framework receives IRP_MJ_WRITE request.
    /// It copies the data from the request into a buffer stored in the queue
    /// context. The actual completion of the request is deferred to the periodic
    /// timer.
    ///
    /// # Arguments
    ///
    /// * `self` - Handle to the framework queue object that is associated with the
    ///   I/O request.
    /// * `Request` - Handle to a framework request object.
    ///
    /// * `Length`  - number of bytes to be read. The default property of the queue
    ///   is to not dispatch zero length read & write requests to the driver and
    ///   complete is with status success. So we will never get a zero length
    ///   request.
    fn evt_io_write(&self, request: Request, length: usize) {
        println!("evt_io_write called. Queue {self:?}, Request {request:?} Length {length}");

        if length > MAX_WRITE_LENGTH {
            println!("evt_io_write buffer length too big {length}. Max is {MAX_WRITE_LENGTH}");
            request.complete_with_information(status_codes::STATUS_BUFFER_OVERFLOW.into(), 0);
            return;
        }

        let memory = match request.retrieve_input_memory() {
            Ok(memory) => memory,
            Err(e) => {
                println!("evt_io_write could not get request memory buffer {e:?}");
                request.complete(e.into());
                return;
            }
        };

        let mut buffer = vec![0_u8; length];

        if let Err(e) = memory.copy_to_buffer(0, &mut buffer) {
            println!("evt_io_write failed to copy buffer: {e:?}");
            request.complete(e.into());
            return;
        }

        *self.buffer.lock() = Some(buffer);

        request.set_information(length);

        if let Err((e, request)) = request.mark_cancellable(evt_request_cancel, self.request) {
            println!("evt_io_write failed to mark request cancellable: {e:?}");
            request.complete(status_codes::STATUS_UNSUCCESSFUL.into());
        }
    }
}

/// Callback that is called when the request is cancelled.
/// It cancels the request identified by the `token` parameter
/// if it is found in the context.
///
/// # Arguments
///
/// `token` - The cancellation token that identifies the request to be cancelled
fn evt_request_cancel(token: &RequestCancellationToken) {
    println!("evt_request_cancel called");

    let queue = token.get_io_queue();

    let context = QueueContext::get(&queue);

    let mut req = context.request.lock();
    if let Some(req) = req.take() {
        req.complete(status_codes::STATUS_CANCELLED.into());
        println!("Request cancelled");
    } else {
        println!("Request already completed");
    }
}

struct MyTimer {
    queue: Option<Arc<IoQueue>>,
}


impl Timer for MyTimer {
    /// Callback that is called when the timer fires.
    /// It fetches the request stored in the context
    /// and completes it
    ///
    /// # Arguments
    ///
    /// * `timer` - Handle of the timer that fired
    fn evt_timer(&self) {
        println!("evt_timer called");

        let queue = self.queue.expect("queue must be set");

        let req = queue.request.lock().take();

        if let Some(req) = req {
            req.complete(status_codes::STATUS_SUCCESS.into());
            println!("Request completed");
        } else {
            println!("No request pending");
        }
    }
}
