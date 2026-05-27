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

/// A wrapper for `UNICODE_STRING` that owns
/// the buffer that `UNICODE_STRING` points to.
pub struct UnicodeStringBuf {
    _buf: Box<[u16]>, // `_buf` exists only to keep the buffer alive
    unicode_str: UNICODE_STRING,
}

impl UnicodeStringBuf {
    pub fn from_rust_str(rust_str: &str) -> NtResult<Self> {
        let buf = Self::to_utf16_buf(rust_str)?;
        let unicode_str = Self::create_raw_unicode_string_from(&buf)?;
        Ok(Self {
            _buf: buf,
            unicode_str,
        })
    }

    /// Creates a `UnicodeStringBuf` from a raw `UNICODE_STRING`.
    /// 
    /// Allocates its own buffer and copies the contents of `unicode_str` into it.
    /// 
    /// # Safety
    /// 
    /// The caller must ensure that `unicode_str` is a valid `UNICODE_STRING`.
    pub unsafe fn from_raw(unicode_str: UNICODE_STRING) -> NtResult<Self> {
        // This implementation ensures we don't panick on OOM
        // by precomputing the required capacity and using
        // `try_reserve_exact`.

        // SAFETY: As per the contract of this function,
        // the caller must ensure that `unicode_str` is valid
        let slice = unsafe {
            core::slice::from_raw_parts(
                unicode_str.Buffer,
                (unicode_str.MaximumLength / 2) as usize,
            )
        };

        let mut vec = Vec::new();
        vec.try_reserve_exact(slice.len())
            .map_err(|_| NtStatusError::from(status_codes::STATUS_INSUFFICIENT_RESOURCES))?;
        vec.extend_from_slice(slice);

        // len == capacity, so into_boxed_slice() won't reallocate
        // and panic on allocation failure
        debug_assert_eq!(vec.len(), vec.capacity());
        let buf = vec.into_boxed_slice();

        let buf_ptr = buf.as_ptr().cast_mut().cast();

        Ok(Self {
            _buf: buf,
            unicode_str: UNICODE_STRING {
                Buffer: buf_ptr,
                ..unicode_str
            },
        })
    }

    pub fn to_string_lossy(&self) -> NtResult<String> {
        // SAFETY: The way this type is constructed ensures
        // that `self.unicode_str` is a valid `UNICODE_STRING`.
        unsafe { to_string_lossy(self.unicode_str) }
    }

    pub fn as_raw(&self) -> &UNICODE_STRING {
        &self.unicode_str
    }

    fn create_raw_unicode_string_from(buf: &[u16]) -> NtResult<UNICODE_STRING> {
        let byte_len = buf.len() * 2;

        if byte_len > u16::MAX as usize {
            return Err(NtStatusError::from(status_codes::STATUS_INVALID_PARAMETER));
        }

        let byte_len = byte_len as u16;
        Ok(UNICODE_STRING {
            Length: byte_len,
            MaximumLength: byte_len,
            Buffer: buf.as_ptr().cast_mut().cast(),
        })
    }

    /// Converts a `&str` to a UTF-16 encoded buffer.
    /// 
    /// # Errors
    /// Returns an `NtStatusError` if memory allocation fails.
    fn to_utf16_buf(rust_str: &str) -> NtResult<Box<[u16]>> {
        // This implementation ensures we don't panick on OOM
        // by precomputing the required capacity and using `try_reserve_exact`.
        // Unfortunately it means we iterate over the string twice.

        // Compute the exact number of UTF-16 code units
        let utf16_len = rust_str.chars().map(|c| c.len_utf16()).sum::<usize>();

        let mut utf16_vec = Vec::new();
        utf16_vec
            .try_reserve_exact(utf16_len)
            .map_err(|_| NtStatusError::from(status_codes::STATUS_INSUFFICIENT_RESOURCES))?;

        utf16_vec.extend(rust_str.encode_utf16());

        // len == capacity, so into_boxed_slice() won't reallocate
        // and panic on allocation failure
        debug_assert_eq!(utf16_vec.len(), utf16_vec.capacity());
        Ok(utf16_vec.into_boxed_slice())
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
unsafe fn to_string_lossy(unicode_str: UNICODE_STRING) -> NtResult<String> {
    // This implementation ensures we don't panick on OOM
    // by precomputing the required capacity and using `try_reserve_exact`.
    // Unfortunately it means we iterate over the string twice.

    let slice =
        unsafe { core::slice::from_raw_parts(unicode_str.Buffer, unicode_str.Length as usize / 2) };

    // Compute the UTF-8 byte length needed
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
        unsafe { to_string_lossy(self.unicode_str) }
    }

    pub fn as_raw(&self) -> &UNICODE_STRING {
        &self.unicode_str
    }
}
