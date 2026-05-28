use alloc::{boxed::Box, string::String, vec::Vec};
use core::char;

use wdk_sys::{
    NT_SUCCESS,
    UNICODE_STRING,
    WDF_NO_OBJECT_ATTRIBUTES,
    WDFOBJECT,
    WDFSTRING,
    call_unsafe_wdf_function_binding,
};

use super::{object::Handle, result::{NtResult, NtStatusError, status_codes}};

// TODO: We assume that WDFSTRING always owns
// the underlying buffer. If that's not the case
// we need to change the implementation.
/// Represents a framework string object corresponding to WDFSTRING
///
/// Implements RAII to ensure proper resource management.
#[derive(Debug)]
#[repr(transparent)]
pub struct WString(WDFSTRING);

impl Handle for WString {
    #[inline(always)]
    fn as_ptr(&self) -> WDFOBJECT {
        self.0 as WDFOBJECT
    }
}

impl WString {
    pub fn create() -> NtResult<Self> {
        let mut raw_string: WDFSTRING = core::ptr::null_mut();
        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfStringCreate,
                core::ptr::null_mut(),
                WDF_NO_OBJECT_ATTRIBUTES,
                &mut raw_string
            )
        };

        if NT_SUCCESS(status) {
            Ok(Self(raw_string))
        } else {
            Err(status.into())
        }
    }

    pub fn get_unicode_string<'a>(&'a self) -> UnicodeString<'a> {
        let mut unicode_string = UNICODE_STRING::default();

        // SAFETY: The contract of the `Wstring` constructor
        // requires that the underlying pointer is a valid WDFOBJECT.
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfStringGetUnicodeString,
                self.0,
                &mut unicode_string
            )
        }

        unsafe { UnicodeString::from_raw(unicode_string) }
    }

    pub fn to_rust_string_lossy(&self) -> NtResult<String> {
        self.get_unicode_string().to_string_lossy()
    }
}

impl Drop for WString {
    fn drop(&mut self) {
        // SAFETY: The contract of the FwString type constructor
        // requires that the underlying pointer is a valid WDFOBJECT.
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, self.as_ptr());
        }
    }
}

/// A wrapper over `UNICODE_STRING` that owns its buffer
pub struct UnicodeStringBuf {
    // Owned buffer to which `unicode_str` points.
    // This field is not accessed. It exists just to
    // keep the buffer alive
    _buf: Option<Box<[u16]>>,
    unicode_str: UNICODE_STRING,
}

impl UnicodeStringBuf {
    /// Creates a `UnicodeStringBuf` from a Rust `&str`.
    /// 
    /// Converts `&str` to UTF-16 and stores the result
    /// into the owned buffer.
    /// 
    /// # Errors
    /// 
    /// Returns an `NtStatusError` if allocation of the owned
    /// buffer fails
    pub fn from_str(s: &str) -> NtResult<Self> {
        // This method carefully avoids panic on OOM by
        // using `try_reserve_exact`.
        // Unfortunately it means we iterate over the string twice.

        // Compute the exact number of UTF-16 code units
        let utf16_len = s.chars().map(|c| c.len_utf16()).sum::<usize>();
        let utf16_byte_len = utf16_len * 2;

        // `UNICODE_STRING` uses `u16` for length fields
        // so we must ensure byte length fits into it
        if utf16_byte_len > u16::MAX as usize {
            return Err(NtStatusError::from(status_codes::STATUS_INVALID_PARAMETER));
        }

        let mut buf = Vec::new();
        buf
            .try_reserve_exact(utf16_len)
            .map_err(|_| NtStatusError::from(status_codes::STATUS_INSUFFICIENT_RESOURCES))?;
        buf.extend(s.encode_utf16());

        // len must match capacity because otherwise into_boxed_slice would allocate
        // and thereby potentially cause OOM panic which we want to avoid
        debug_assert_eq!(buf.len(), buf.capacity());
        let buf = buf.into_boxed_slice();
        
        let buf_ptr = buf.as_ptr().cast_mut();

        Ok(Self {
            _buf: Some(buf),
            unicode_str: UNICODE_STRING {
                Buffer: buf_ptr,
                Length: utf16_byte_len as u16,
                MaximumLength: utf16_byte_len as u16,
            },
        })
    }

    pub fn to_string_lossy(&self) -> NtResult<String> {
        // SAFETY: The way this type is constructed ensures
        // that `self.unicode_str` is a valid `UNICODE_STRING`.
        unsafe { to_string_lossy(self.as_raw()) }
    }

    #[inline(always)]
    pub fn as_raw(&self) -> &UNICODE_STRING {
        &self.unicode_str
    }
}

/// Converts a `UNICODE_STRING` to a Rust `String`
/// replacing invalid UTF-16 sequences with the replacement character.
/// 
/// # Safety
/// 
/// The caller must ensure that `unicode_str` is a valid `UNICODE_STRING`.
/// 
/// # Errors
/// 
/// Returns an `NtStatusError` if memory allocation fails during string construction.
unsafe fn to_string_lossy(unicode_str: &UNICODE_STRING) -> NtResult<String> {
    // We avoid panic on OOM by carefully using
    // APIs like `try_reserve_exact`.

    // If length is 0 then the buffer can be null,
    // so we return an empty string without trying
    // to read from the buffer.
    if unicode_str.Length  == 0 {
        return Ok(String::new()); // No panic on OOM risk as String::new() does not allocate
    }

    // SAFETY: The caller must ensure `unicode_str` is valid.
    // We have checked that Length is non-zero above,
    // so Buffer must be non-null and valid for `len` elements.
    let slice =
        unsafe { core::slice::from_raw_parts(unicode_str.Buffer, unicode_str.Length as usize / 2) };

    // Compute the UTF-8 byte length needed for reserving.
    // Unfortunately it means we iterate over the string twice.
    let utf8_len: usize = char::decode_utf16(slice.iter().copied())
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER).len_utf8())
        .sum();

    let mut result = String::new();
    result
        .try_reserve_exact(utf8_len)
        .map_err(|_| NtStatusError::from(status_codes::STATUS_INSUFFICIENT_RESOURCES))?;

    for c in char::decode_utf16(slice.iter().copied()) {
        result.push(c.unwrap_or(char::REPLACEMENT_CHARACTER));
    }

    Ok(result)
}

/// A wrapper for `UNICODE_STRING`
/// `'a` represents the lifetime of the underlying buffer
///
/// This type has `repr(transparent)` to ensure `&UnicodeString`
/// can be safely cast to `PUNICODE_STRING`
#[repr(transparent)]
pub struct UnicodeString<'a> {
    unicode_str: UNICODE_STRING,
    _marker: core::marker::PhantomData<&'a ()>, // Marker to tie the lifetime to the buffer
}

impl<'a> UnicodeString<'a> {
    /// Creates a `UnicodeString` from a raw `UNICODE_STRING`.
    /// 
    /// # Safety
    /// 
    /// The caller must ensure that `unicode_str` is a valid `UNICODE_STRING`.
    pub(crate) unsafe fn from_raw(unicode_str: UNICODE_STRING) -> Self {
        Self {
            unicode_str,
            _marker: core::marker::PhantomData,
        }
    }

    pub fn to_string_lossy(&self) -> NtResult<String> {
        // SAFETY: As per the contract of this type
        // `self.unicode_str` is a valid `UNICODE_STRING`.
        unsafe { to_string_lossy(&self.unicode_str) }
    }

    pub fn as_raw(&self) -> &UNICODE_STRING {
        &self.unicode_str
    }
}
