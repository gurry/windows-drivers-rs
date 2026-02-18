use core::{ptr, sync::atomic::AtomicIsize};

use wdf_macros::object_context_with_ref_count_check;
use wdk_sys::{
    NT_SUCCESS,
    PWDFMEMORY_OFFSET,
    WDF_IO_TARGET_SENT_IO_ACTION,
    WDF_MEMORY_DESCRIPTOR,
    WDF_NO_OBJECT_ATTRIBUTES,
    WDF_REQUEST_SEND_OPTIONS,
    WDFIOTARGET,
    WDFMEMORY,
    WDFMEMORY_OFFSET,
    _WDF_REQUEST_SEND_OPTIONS_FLAGS,
    call_unsafe_wdf_function_binding,
};

use super::{
    Timeout,
    device::Device,
    enum_mapping,
    init_wdf_struct,
    memory::{Memory, MemoryDescriptor, MemoryDescriptorMut, MemoryOffset, OwnedMemory},
    object::{Handle, impl_ref_counted_handle},
    request::Request,
    result::{NtResult, StatusCodeExt, status_codes},
    sync::Arc,
};

impl_ref_counted_handle!(IoTarget, IoTargetContext);

impl IoTarget {
    /// Create an `IoTarget`
    pub fn create(device: &Device) -> NtResult<Arc<Self>> {
        let mut io_target: WDFIOTARGET = core::ptr::null_mut();
        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfIoTargetCreate,
                device.as_ptr().cast(),
                WDF_NO_OBJECT_ATTRIBUTES,
                &mut io_target,
            )
        };

        if NT_SUCCESS(status) {
            let ctxt = IoTargetContext {
                ref_count: AtomicIsize::new(0),
            };

            IoTargetContext::attach(unsafe { &*(io_target.cast()) }, ctxt)?;

            let io_target = unsafe { Arc::from_raw(io_target.cast()) };

            Ok(io_target)
        } else {
            Err(status.into())
        }
    }

    // TODO: start and stop are not thread-safe. They
    // cannot be called concurrently with each other. Fix that!
    pub fn start(&self) -> NtResult<()> {
        unsafe { call_unsafe_wdf_function_binding!(WdfIoTargetStart, self.as_ptr().cast()) }.ok()
    }

    // TODO: start and stop are not thread-safe. They
    // cannot be called concurrently with each other. Fix that!
    pub fn stop(&self, action: IoTargetSentIoAction) {
        let action_val: WDF_IO_TARGET_SENT_IO_ACTION = action.into();
        unsafe {
            call_unsafe_wdf_function_binding!(WdfIoTargetStop, self.as_ptr().cast(), action_val)
        }
    }

    pub fn get_device(&self) -> &Device {
        let device_ptr = unsafe {
            call_unsafe_wdf_function_binding!(WdfIoTargetGetDevice, self.as_ptr().cast())
        };

        unsafe { &*(device_ptr.cast::<Device>()) }
    }

    pub fn format_request_for_read(
        &self,
        request: &mut Request,
        output_memory: RequestFormatMemory,
        device_offset: Option<i64>,
    ) -> NtResult<()> {
        let mut memory_offset = WDFMEMORY_OFFSET::default();
        let (memory_ptr, memory_offset_ptr) =
            to_memory_ptrs(request, output_memory, &mut memory_offset, false)?;

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfIoTargetFormatRequestForRead,
                self.as_ptr().cast(),
                request.as_ptr().cast(),
                memory_ptr.cast(),
                memory_offset_ptr,
                to_device_offset_ptr(device_offset)
            )
        }
        .ok()
    }

    pub fn format_request_for_write(
        &self,
        request: &mut Request,
        input_memory: RequestFormatMemory,
        device_offset: Option<i64>,
    ) -> NtResult<()> {
        let mut memory_offset = WDFMEMORY_OFFSET::default();
        let (memory_ptr, memory_offset_ptr) =
            to_memory_ptrs(request, input_memory, &mut memory_offset, true)?;

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfIoTargetFormatRequestForWrite,
                self.as_ptr().cast(),
                request.as_ptr().cast(),
                memory_ptr.cast(),
                memory_offset_ptr,
                to_device_offset_ptr(device_offset)
            )
        }
        .ok()
    }

    /// Formats a request for a device I/O control operation.
    ///
    /// # Arguments
    /// * `request` - The request to format
    /// * `ioctl_code` - The I/O control code (IOCTL)
    /// * `input_memory` - Optional input memory for the IOCTL
    /// * `output_memory` - Optional output memory for the IOCTL
    pub fn format_request_for_ioctl(
        &self,
        request: &mut Request,
        ioctl_code: u32,
        input_memory: RequestFormatMemory,
        output_memory: RequestFormatMemory,
    ) -> NtResult<()> {
        let mut input_offset = WDFMEMORY_OFFSET::default();
        let (input_ptr, input_offset_ptr) =
            to_memory_ptrs(request, input_memory, &mut input_offset, true)?;

        let mut output_offset = WDFMEMORY_OFFSET::default();
        let (output_ptr, output_offset_ptr) =
            to_memory_ptrs(request, output_memory, &mut output_offset, false)?;

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfIoTargetFormatRequestForIoctl,
                self.as_ptr().cast(),
                request.as_ptr().cast(),
                ioctl_code,
                input_ptr.cast(),
                input_offset_ptr,
                output_ptr.cast(),
                output_offset_ptr,
            )
        }
        .ok()
    }

    /// Sends a device I/O control request synchronously.
    ///
    /// # Arguments
    /// * `request` - Optional request object. If `None`, the framework
    ///   uses an internal request object.
    /// * `ioctl_code` - The I/O control code (IOCTL)
    /// * `input_buffer` - Optional input buffer descriptor
    /// * `output_buffer` - Optional output buffer descriptor
    /// * `timeout` - Timeout for the request
    ///
    /// Returns the number of bytes returned by the target on success.
    pub fn send_ioctl_synchronously(
        &self,
        request: Option<&Request>,
        ioctl_code: u32,
        input_buffer: Option<&MemoryDescriptor<'_>>,
        output_buffer: Option<&mut MemoryDescriptorMut<'_>>,
        timeout: Timeout,
    ) -> NtResult<usize> {
        let input_descriptor: Option<WDF_MEMORY_DESCRIPTOR> =
            input_buffer.map(|b| b.into());
        let input_descriptor_ptr = input_descriptor
            .as_ref()
            .map_or(ptr::null_mut(), |desc| {
                (desc as *const WDF_MEMORY_DESCRIPTOR).cast_mut()
            });

        let output_descriptor: Option<WDF_MEMORY_DESCRIPTOR> =
            output_buffer.map(|b| (&*b).into());
        let output_descriptor_ptr = output_descriptor
            .as_ref()
            .map_or(ptr::null_mut(), |desc| {
                (desc as *const WDF_MEMORY_DESCRIPTOR).cast_mut()
            });

        let mut send_options = init_wdf_struct!(WDF_REQUEST_SEND_OPTIONS);
        send_options.Flags |=
            _WDF_REQUEST_SEND_OPTIONS_FLAGS::WDF_REQUEST_SEND_OPTION_TIMEOUT as u32;
        send_options.Timeout = timeout.as_wdf_timeout();

        let mut bytes_returned: u64 = 0;

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfIoTargetSendIoctlSynchronously,
                self.as_ptr().cast(),
                request.map_or(ptr::null_mut(), |r| r.as_ptr().cast()),
                ioctl_code,
                input_descriptor_ptr,
                output_descriptor_ptr,
                &mut send_options,
                &mut bytes_returned,
            )
        }
        .map(|| bytes_returned as usize)
    }
}

pub(crate) fn to_memory_ptrs(
    request: &mut Request,
    memory: RequestFormatMemory,
    raw_memory_offset: &mut WDFMEMORY_OFFSET,
    is_input_memory: bool,
) -> NtResult<(WDFMEMORY, PWDFMEMORY_OFFSET)> {
    let (mem_ptr, buffer_len, offset) = match memory {
        RequestFormatMemory::None => (ptr::null_mut(), 0, None),
        RequestFormatMemory::RequestMemory(offset) => {
            let memory: &Memory = if is_input_memory {
                request.retrieve_input_memory()?
            } else {
                request.retrieve_output_memory()?
            };
            let (ptr, len) = get_memory_ptr_and_len(memory);

            (ptr, len, offset)
        }
        RequestFormatMemory::UserBuffer(memory, offset) => {
            let (ptr, len) = get_memory_ptr_and_len(&memory);

            // TODO: do we really have to save the buffer in the context?
            // Can't we just skip the drop of OwnedMemory and recover
            // it later when the buffer ptr is being given back to the driver?
            // This will eliminate the need to take &mut Request. We could just
            // take &Request instead.

            // IMPORTANT: Save the buffer in the request
            // so that it stays alive while the request
            // is being processed
            set_request_user_buffer(request, memory, is_input_memory)?;

            (ptr, len, offset)
        }
    };

    if !is_valid_offset(buffer_len, &offset) {
        return Err(status_codes::STATUS_INVALID_PARAMETER.into());
    }

    let raw_memory_offset_ptr = if let Some(ref offset) = offset {
        *raw_memory_offset = offset.into();
        raw_memory_offset as PWDFMEMORY_OFFSET
    } else {
        ptr::null_mut()
    };

    Ok((mem_ptr, raw_memory_offset_ptr))
}

fn to_device_offset_ptr(device_offset: Option<i64>) -> *mut i64 {
    device_offset
        .map(|mut offset| &raw mut offset)
        .unwrap_or(ptr::null_mut())
}

fn set_request_user_buffer(
    request: &mut Request,
    buffer: OwnedMemory,
    is_input_buffer: bool,
) -> NtResult<()> {
    if is_input_buffer {
        request.set_user_input_memory(buffer)?;
    } else {
        request.set_user_output_memory(buffer)?;
    }
    Ok(())
}

/// Gets the raw pointer corresponding to the given
/// `Memory` reference along with the length of its
/// underlying buffer
fn get_memory_ptr_and_len(memory: &Memory) -> (WDFMEMORY, usize) {
    let buffer = memory.get_buffer();
    (memory.as_ptr() as WDFMEMORY, buffer.len())
}

fn is_valid_offset(buffer_len: usize, offset: &Option<MemoryOffset>) -> bool {
    let Some(offset) = offset else {
        return true;
    };

    let offset_end = offset.buffer_offset.checked_add(offset.buffer_length);
    match offset_end {
        Some(offset_end) if offset_end <= buffer_len => true,
        _ => false,
    }
}

/// Specifies the memory used while formatting
/// a request
#[derive(Debug)]
pub enum RequestFormatMemory {
    /// Do not use any memory
    None,

    /// The memory associated with the request
    RequestMemory(Option<MemoryOffset>),

    /// An independent memory provided by user
    UserBuffer(OwnedMemory, Option<MemoryOffset>),
}

#[object_context_with_ref_count_check(IoTarget)]
struct IoTargetContext {
    ref_count: AtomicIsize,
}

enum_mapping! {
    pub enum IoTargetSentIoAction: WDF_IO_TARGET_SENT_IO_ACTION {
        CancelSentIo = WdfIoTargetCancelSentIo,
        WaitForSentIoToComplete = WdfIoTargetWaitForSentIoToComplete,
        LeaveSentIoPending = WdfIoTargetLeaveSentIoPending,
    }
}
