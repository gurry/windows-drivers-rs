use alloc::string::String;

use wdk_sys::{WDFKEY, WDFOBJECT, call_unsafe_wdf_function_binding};

use super::{
    object::Handle,
    result::{NtResult, StatusCodeExt},
    string::UnicodeStringBuf,
};

/// Represents a framework registry key object corresponding to WDFKEY.
///
/// Implements RAII — the key is closed via `WdfRegistryClose` on drop.
#[derive(Debug)]
#[repr(transparent)]
pub struct RegistryKey(WDFKEY);

impl Handle for RegistryKey {
    #[inline(always)]
    fn as_ptr(&self) -> WDFOBJECT {
        self.0.cast()
    }

    fn type_name() -> String {
        String::from("RegistryKey")
    }
}

unsafe impl Send for RegistryKey {}
unsafe impl Sync for RegistryKey {}

impl RegistryKey {
    /// Creates a `RegistryKey` from a raw `WDFKEY` handle.
    ///
    /// # Safety
    ///
    /// The caller must ensure `key` is a valid, open `WDFKEY` handle.
    pub(crate) unsafe fn from_raw(key: WDFKEY) -> Self {
        Self(key)
    }

    /// Queries a `u32` value from the registry key.
    pub fn query_u32(&self, value_name: &UnicodeStringBuf) -> NtResult<u32> {
        let mut value: u32 = 0;

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRegistryQueryULong,
                self.0,
                value_name.as_raw(),
                &mut value,
            )
        }
        .map(|| value)
    }
}

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfRegistryClose, self.0);
        }
    }
}

/// The desired access rights for a registry key.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RegistryAccessRights {
    /// Read access (KEY_READ).
    Read,
    /// Write access (KEY_WRITE).
    Write,
    /// Full access (KEY_ALL_ACCESS).
    AllAccess,
}

impl From<RegistryAccessRights> for u32 {
    fn from(value: RegistryAccessRights) -> Self {
        match value {
            RegistryAccessRights::Read => wdk_sys::KEY_READ,
            RegistryAccessRights::Write => wdk_sys::KEY_WRITE,
            RegistryAccessRights::AllAccess => wdk_sys::KEY_ALL_ACCESS,
        }
    }
}
