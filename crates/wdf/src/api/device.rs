use core::sync::atomic::AtomicUsize;
use crate::api::{
    error::NtResult,
    guid::Guid,
    object::{Handle, impl_ref_counted_handle},
    string::{to_unicode_string, to_utf16_buf},
};
use wdf_macros::primary_object_context;
use wdk_sys::{
    call_unsafe_wdf_function_binding, NT_SUCCESS, WDFDEVICE, WDFDEVICE_INIT, WDF_NO_HANDLE,
    WDF_NO_OBJECT_ATTRIBUTES,
};

impl_ref_counted_handle!(
    Device,
    PrimaryDeviceContext
);

impl Device {
    pub fn create(device_init: &mut DeviceInit) -> NtResult<&Self> {
        let mut device: WDFDEVICE = WDF_NO_HANDLE.cast();
        let mut device_init_ptr: *mut WDFDEVICE_INIT = device_init.as_ptr_mut();

        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceCreate,
                &mut device_init_ptr,
                WDF_NO_OBJECT_ATTRIBUTES,
                &mut device,
            )
        };

        if NT_SUCCESS(status) {
            let device = unsafe { &*(device as *mut _) };
            PrimaryDeviceContext::attach(device, PrimaryDeviceContext { ref_count: AtomicUsize::new(0) })?;
            Ok(device)
        } else {
            Err(status.into())
        }
    }

    pub fn create_interface(
        &self,
        interaface_class_guid: &Guid,
        reference_string: Option<&str>,
    ) -> NtResult<()> {
        let ref_str_buf = reference_string.map(to_utf16_buf);
        let unicode_ref_str = ref_str_buf.map(|b| to_unicode_string(b.as_ref()));

        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceCreateDeviceInterface,
                self.as_ptr() as *mut _,
                interaface_class_guid.as_lpcguid(),
                unicode_ref_str.map_or(core::ptr::null(), |s| &s)
            )
        };

        if NT_SUCCESS(status) {
            Ok(())
        } else {
            Err(status.into())
        }
    }
}

pub struct DeviceInit(*mut WDFDEVICE_INIT);

impl DeviceInit {
    pub unsafe fn from(inner: *mut WDFDEVICE_INIT) -> Self {
        Self(inner)
    }

    pub fn as_ptr_mut(&self) -> *mut WDFDEVICE_INIT {
        self.0
    }
}

#[primary_object_context(Device)]
struct PrimaryDeviceContext {
    ref_count: AtomicUsize,
}
