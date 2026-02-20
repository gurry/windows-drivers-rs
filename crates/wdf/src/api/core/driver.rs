use alloc::string::String;
use core::{cell::UnsafeCell, ptr};

#[doc(hidden)]
pub use wdk_sys::{
    DRIVER_OBJECT,
    NT_SUCCESS,
    NTSTATUS,
    PCUNICODE_STRING,
    WDF_OBJECT_ATTRIBUTES,
    WDF_OBJECT_CONTEXT_TYPE_INFO,
    WDFOBJECT,
};
use wdf_macros::object_context;
use wdk_sys::{
    WDF_DRIVER_CONFIG,
    WDF_DRIVER_VERSION_AVAILABLE_PARAMS,
    WDF_NO_OBJECT_ATTRIBUTES,
    WDFDEVICE_INIT,
    WDFDRIVER,
    call_unsafe_wdf_function_binding,
};

use super::{
    device::DeviceInit,
    guid::Guid,
    init_wdf_struct,
    object::{Handle, impl_handle},
    result::{NtResult, StatusCodeExt, status_codes},
    string::{UnicodeString, WString},
    tracing::TraceWriter,
};
use crate::println;

static TRACE_WRITER: UnsafeOnceCell<TraceWriter> = UnsafeOnceCell::new();

/// A safe wrapper around `DRIVER_OBJECT`
#[repr(transparent)]
pub struct DriverObject(DRIVER_OBJECT);

/// Configuration for creating a WDF driver
pub struct DriverConfig {
    /// The pool tag to be used for all allocations made by the framework
    /// on behalf of this driver. A value of `0` means the framework will
    /// use the driver's image name as the pool tag.
    pub pool_tag: u32,

    /// The callback invoked by the framework when a new device is added
    pub evt_device_add: fn(&mut DeviceInit) -> NtResult<()>,
}

impl DriverConfig {
    pub fn new(evt_device_add: fn(&mut DeviceInit) -> NtResult<()>) -> Self {
        Self {
            pool_tag: 0,
            evt_device_add,
        }
    }
}

impl_handle!(
    /// Represents a WDF driver object
    Driver
);

#[object_context(Driver)]
struct DriverContext {
    evt_device_add: fn(&mut DeviceInit) -> NtResult<()>,
}

impl Driver {
    /// Creates a new driver object
    pub fn create<'a>(
        driver_object: &'a mut DriverObject,
        registry_path: &UnicodeString,
        config: DriverConfig,
    ) -> NtResult<&'a Driver> {
        let mut driver_config = init_wdf_struct!(WDF_DRIVER_CONFIG);
        driver_config.EvtDriverDeviceAdd = Some(evt_driver_device_add);
        driver_config.DriverPoolTag = config.pool_tag;

        let mut wdf_driver: WDFDRIVER = ptr::null_mut();

        let reg_path_ptr: PCUNICODE_STRING =
            (registry_path as *const UnicodeString).cast();

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDriverCreate,
                &mut driver_object.0,
                reg_path_ptr,
                WDF_NO_OBJECT_ATTRIBUTES,
                &mut driver_config,
                &raw mut wdf_driver,
            )
        }
        .ok()?;

        // SAFETY: `Driver` is a ZST handle type (via `impl_handle!`), so
        // casting the raw `WDFDRIVER` pointer to `&Driver` is sound.
        let driver: &Driver = unsafe { &*(wdf_driver.cast()) };

        DriverContext::attach(
            driver,
            DriverContext {
                evt_device_add: config.evt_device_add,
            },
        )?;

        Ok(driver)
    }

    pub fn retrieve_version_string(&self) -> NtResult<String> {
        let string = WString::create()?;

        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDriverRetrieveVersionString,
                self.as_ptr().cast(),
                string.as_ptr().cast(),
            )
        };

        if !NT_SUCCESS(status) {
            return Err(status.into());
        }

        Ok(string.to_rust_string_lossy())
    }

    pub fn is_version_available(&self, major_version: u32, minor_version: u32) -> bool {
        let mut params = init_wdf_struct!(WDF_DRIVER_VERSION_AVAILABLE_PARAMS);
        params.MajorVersion = major_version;
        params.MinorVersion = minor_version;

        let res = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDriverIsVersionAvailable,
                self.as_ptr().cast(),
                &raw mut params,
            )
        };

        res != 0
    }
}

/// A container like [`core::cell::OnceCell`] that is
/// set once and read multiple times.
///
/// # Safety
/// The reason this type has the prefix `Unsafe` in its name
/// is because it is not thread safe. To use it safely
/// the user must uphold the following invariants:
/// - The [`set`] method must not be called concurrently with itself or the
///   [`get`] method.
/// - The `get` method must not be called concurrently with the `set` method.
///
/// The typical pattern is to call `set` first from
/// a single thread and initialize the value, and then
/// call `get` from multiple threads as needed.
///
/// `UnsafeOnceCell` implements `Sync` but only to allow it to
/// be used in certain static variables in this module. In
/// reality it is not `Sync` under all conditions. It is `Sync`
/// only when the above-mentioned invariants are maintained.
/// Therefore **do not use it in contexts which require that
/// it is always `Sync`**.
///
/// More broadly speaking `UnsafeOnceCell` is NOT a
/// general-purpose type. It is meant to be used only in the way it is used
/// right now wherein instances of it are placed in static variables,
/// the driver entry function -- and only the driver entry
/// function -- calls `set` and other methods in this module
/// call `get` only after driver entry is finished. Therefore
/// **please do not use it for any other purpose and be careful
/// when changing it or any of the code that uses it**.
///
/// We could have made it thread-safe and avoid all of
/// the above constraints but that would have meant that every
/// access to it requires an atomic operation which is bad for
/// performance because values stored in it are meant to be
/// accessed very frequently such as from tracing calls.
struct UnsafeOnceCell<T> {
    val: UnsafeCell<Option<T>>,
}

impl<T> UnsafeOnceCell<T> {
    /// Creates a new `UnsafeOnceCell` instance
    pub const fn new() -> Self {
        Self {
            val: UnsafeCell::new(None),
        }
    }

    /// Returns a reference to the value.
    ///
    /// # Safety
    /// This method will causes data races if
    /// called concurrently with the [`set`]
    /// method. It is safe to be called
    /// concurrently with itself however.
    pub unsafe fn get(&self) -> Option<&T> {
        // SAFETY: Safe because we assume that the call to this method
        // is not concurrent with the `set` method. This is true
        let val_ref = unsafe { &*self.val.get() };
        val_ref.as_ref()
    }

    /// Sets the value if it has not been already set
    ///
    /// # Returns
    /// Returns `Ok(())` if the value was set successfully,
    /// or an `Err(NtError)` if it was already set.
    ///
    /// # Safety
    /// This method will cause data races if called
    /// concurrently with itself or the [`get`] method.
    pub unsafe fn set(&self, val: T) -> NtResult<()> {
        // SAFETY: Safe because we assume that the call to this method
        // is not concurrent with itself or the `get` method.
        unsafe {
            let val_ptr = self.val.get();
            if (*val_ptr).is_some() {
                return Err(status_codes::STATUS_UNSUCCESSFUL.into());
            }
            *val_ptr = Some(val);
        };
        Ok(())
    }
}

/// This type is `Sync` if `T` is `Sync` AND if the
/// safety invariants stated on the `UnsafeOnceCell` type
/// are upheld. Ideally we should not have implemented `Sync`
/// for it, but we had to to make it usable in static variables
unsafe impl<T> Sync for UnsafeOnceCell<T> where T: Sync {}

fn clean_up_tracing() {
    if let Some(trace_writer) =
        // SAFETY: This is safe because this call to `get`
        // is not concurrent with any call to `set`. `set` is
        // called only once in the beginning in the driver entry
        // function
        unsafe { TRACE_WRITER.get() }
    {
        trace_writer.stop();
    }
}

/// Calls the safe driver entry function
///
/// It is meant to be called by the driver entry function generated
/// in the user's driver code by the `driver_entry` procedural
/// macro attribute
#[doc(hidden)]
pub fn call_safe_driver_entry(
    driver_object: &mut DRIVER_OBJECT,
    reg_path: PCUNICODE_STRING,
    safe_entry: fn(&mut DriverObject, &UnicodeString) -> NtResult<()>,
    tracing_control_guid: Option<Guid>,
) -> NTSTATUS {
    driver_object.DriverUnload = Some(wdm_driver_unload);

    // SAFETY: `DriverObject` is `#[repr(transparent)]` over `DRIVER_OBJECT`,
    // so this cast is sound.
    let driver_object: &mut DriverObject =
        unsafe { &mut *(driver_object as *mut DRIVER_OBJECT as *mut DriverObject) };

    // SAFETY: `UnicodeString` is `#[repr(transparent)]` over `UNICODE_STRING`,
    // so casting `PCUNICODE_STRING` to `&UnicodeString` preserves pointer identity.
    let registry_path: &UnicodeString =
        unsafe { &*(reg_path.cast::<UnicodeString>()) };

    if let Some(control_guid) = tracing_control_guid {
        let trace_writer = unsafe {
            TraceWriter::init(control_guid, &mut driver_object.0, reg_path)
        };

        trace_writer.start();

        // SAFETY: We are upholding the invariants of `UnsafeOnceCell.set`
        // because:
        // 1. This is the only call to `TRACE_WRITER.set` and it runs only
        // on one thread. Therefore, there is no question of `set` running
        // concurrently with itself
        // 2. This is the driver entry and it is guaranteed to run before
        // any other driver code. Therefore `TRACE_WRITER.get` cannot run
        // concurrently with `TRACE_WRITER.set`
        unsafe {
            TRACE_WRITER
                .set(trace_writer)
                .expect("trace writer should not be already set");
        }
    }

    match safe_entry(driver_object, registry_path) {
        Ok(()) => 0,
        Err(e) => {
            clean_up_tracing();
            e.code()
        }
    }
}

#[unsafe(link_section = "PAGE")]
extern "C" fn evt_driver_device_add(
    driver: WDFDRIVER,
    device_init: *mut WDFDEVICE_INIT,
) -> NTSTATUS {
    // SAFETY: `Driver` is a ZST handle type, so casting `WDFDRIVER` to
    // `&Driver` is sound.
    let driver: &Driver = unsafe { &*(driver.cast()) };
    let ctxt = DriverContext::get(driver);

    let mut device_init = unsafe { DeviceInit::from(device_init) };
    match (ctxt.evt_device_add)(&mut device_init) {
        Ok(_) => 0,
        Err(e) => e.code(),
    }
}

extern "C" fn wdm_driver_unload(_driver: *mut DRIVER_OBJECT) {
    println!("Driver unload");

    clean_up_tracing();

    println!("Driver unload done");
}

pub fn trace(message: &str) {
    // SAFETY: This is safe because this call to `get`
    // is not concurrent with any call to `set`. `set` is
    // called only once in the beginning in the user's
    // driver entry function
    unsafe {
        if let Some(trace_writer) = TRACE_WRITER.get() {
            trace_writer.write(message);
        }
    }
}
