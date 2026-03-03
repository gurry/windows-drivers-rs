use alloc::{string::String, vec::Vec};
use core::{ptr, slice};

use bitflags::bitflags;
use wdf_macros::object_context;
use wdk_sys::{
    IO_STATUS_BLOCK,
    WDF_REQUEST_COMPLETION_PARAMS,
    WDF_REQUEST_PARAMETERS,
    WDF_REQUEST_REUSE_PARAMS,
    WDF_REQUEST_TYPE,
    WDFCONTEXT,
    WDFIOTARGET,
    WDFMEMORY,
    WDFOBJECT,
    WDFREQUEST,
    call_unsafe_wdf_function_binding,
};

use super::{
    enum_mapping,
    init_wdf_struct,
    io_queue::IoQueue,
    io_target::IoTarget,
    memory::{Memory, OwnedMemory},
    object::{Handle, init_attributes},
    result::{NtResult, NtStatus, NtStatusError, StatusCodeExt, status_codes},
    sync::Opaque,
};
use crate::usb::UsbRequestCompletionParams;

#[derive(Debug)]
#[repr(transparent)]
pub struct Request(WDFREQUEST);

// Removed the generic trait and its functions; the macro below generates
// per-buffer context structs and the Request methods, so the trait is not
// needed. (Remove the entire `trait UserMemoryContextLike { ... }` block here)

impl Request {
    pub(crate) unsafe fn from_raw(inner: WDFREQUEST) -> Self {
        Self(inner)
    }

    /// Creates a new WDF request object.
    ///
    /// The optional `io_target` parameter specifies the default
    /// I/O target for the request. If `None`, the request is not
    /// associated with any I/O target.
    pub fn create(io_target: Option<&IoTarget>) -> NtResult<Self> {
        let mut request: WDFREQUEST = ptr::null_mut();
        let io_target_ptr = io_target.map_or(ptr::null_mut(), |t| t.as_ptr().cast());
        let mut attributes = init_attributes();

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestCreate,
                &mut attributes,
                io_target_ptr,
                &mut request,
            )
        }
        .map(|| unsafe { Self::from_raw(request) })
    }

    pub fn id(&self) -> RequestId {
        RequestId(self.0 as usize)
    }

    pub fn complete(self, status: NtStatus) {
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestComplete,
                self.as_ptr().cast(),
                status.code()
            )
        };
    }

    pub fn complete_with_information(self, status: NtStatus, information: usize) {
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestCompleteWithInformation,
                self.as_ptr().cast(),
                status.code(),
                information as core::ffi::c_ulonglong
            )
        };
    }

    pub fn set_information(&self, information: usize) {
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestSetInformation,
                self.as_ptr().cast(),
                information as core::ffi::c_ulonglong
            )
        };
    }

    /// Returns the I/O status information value for the request.
    pub fn get_information(&self) -> usize {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfRequestGetInformation, self.as_ptr().cast())
                as usize
        }
    }

    /// Retrieves the parameters associated with the request.
    pub fn get_parameters(&self) -> RequestParameters {
        let mut params = WDF_REQUEST_PARAMETERS {
            Size: core::mem::size_of::<WDF_REQUEST_PARAMETERS>() as u16,
            ..WDF_REQUEST_PARAMETERS::default()
        };

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestGetParameters,
                self.as_ptr().cast(),
                &mut params,
            );
        }

        RequestParameters(params)
    }

    pub fn mark_cancellable(
        mut self,
        cancel_fn: fn(&RequestCancellationToken),
    ) -> Result<CancellableRequest, (NtStatusError, Request)> {
        if let Err(e) = self.set_cancel_callback_in_context(Some(cancel_fn)) {
            return Err((e, self));
        }

        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestMarkCancelableEx,
                self.as_ptr() as WDFREQUEST,
                Some(__evt_request_cancel)
            )
        };

        if !status.is_success() {
            let _ = self.set_cancel_callback_in_context(None); // Will not fail this time
            Err((NtStatusError::from(status), self))
        } else {
            Ok(CancellableRequest(self))
        }
    }

    fn set_cancel_callback_in_context(
        &mut self,
        cancel_fn: Option<fn(&RequestCancellationToken)>,
    ) -> NtResult<()> {
        let context = self.get_context_mut_or_attach_new()?;
        context.evt_request_cancel = cancel_fn;
        Ok(())
    }

    pub fn get_io_queue(&self) -> Option<&Opaque<IoQueue>> {
        unsafe { Self::get_io_queue_from_raw(self.0) }
    }

    pub fn retrieve_input_memory(&self) -> NtResult<&Memory> {
        let mut raw_memory: WDFMEMORY = ptr::null_mut();

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestRetrieveInputMemory,
                self.as_ptr().cast(),
                &mut raw_memory
            )
        }
        .map(|| unsafe { &*(raw_memory.cast::<Memory>()) })
    }

    pub fn retrieve_output_memory(&mut self) -> NtResult<&mut Memory> {
        let mut raw_memory: WDFMEMORY = ptr::null_mut();

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestRetrieveOutputMemory,
                self.as_ptr().cast(),
                &mut raw_memory
            )
        }
        .map(|| unsafe { &mut *(raw_memory.cast::<Memory>()) })
    }

    pub fn retrieve_input_buffer(&self, minimum_required_size: usize) -> NtResult<&[u8]> {
        let mut buffer_ptr: *mut core::ffi::c_void = ptr::null_mut();
        let mut buffer_size: usize = 0;

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestRetrieveInputBuffer,
                self.as_ptr().cast(),
                minimum_required_size,
                &mut buffer_ptr,
                &mut buffer_size
            )
            .and_then(|| Ok(slice::from_raw_parts(buffer_ptr.cast::<u8>(), buffer_size)))
        }
    }

    pub fn retrieve_output_buffer(&mut self, minimum_required_size: usize) -> NtResult<&mut [u8]> {
        let mut buffer_ptr: *mut core::ffi::c_void = ptr::null_mut();
        let mut buffer_size: usize = 0;

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestRetrieveOutputBuffer,
                self.as_ptr().cast(),
                minimum_required_size,
                &mut buffer_ptr,
                &mut buffer_size
            )
            .and_then(|| {
                Ok(slice::from_raw_parts_mut(
                    buffer_ptr.cast::<u8>(),
                    buffer_size,
                ))
            })
        }
    }

    pub fn set_completion_routine(
        &mut self,
        completion_routine: fn(RequestCompletionToken, &Opaque<IoTarget>),
    ) -> NtResult<()> {
        let context = self.get_context_mut_or_attach_new()?;

        context.evt_request_completion_routine = Some(completion_routine);
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestSetCompletionRoutine,
                self.as_ptr().cast(),
                Some(__evt_request_completion_routine),
                // Currently not supporting context.
                // For now it is enough to just add context to
                // the request using the usuaual mechanism
                // and rely on that
                ptr::null_mut()
            );
        }

        Ok(())
    }

    pub fn send_asynchronously(self, io_target: &IoTarget) -> Result<SentRequest, Request> {
        let res = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestSend,
                self.as_ptr().cast(),
                io_target.as_ptr().cast(),
                ptr::null_mut(), // Null options means asynchronous send
            )
        };

        if res != 0 {
            Ok(SentRequest(self))
        } else {
            Err(self)
        }
    }

    pub fn forward_to_io_queue(&self, queue: &IoQueue) -> NtResult<()> {
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestForwardToIoQueue,
                self.as_ptr().cast(),
                queue.as_ptr().cast()
            )
        }
        .ok()
    }

    pub fn get_completion_params<'a>(&'a self) -> RequestCompletionParams<'a> {
        let mut raw_params = init_wdf_struct!(WDF_REQUEST_COMPLETION_PARAMS);
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestGetCompletionParams,
                self.as_ptr().cast(),
                &mut raw_params
            )
        };

        (&raw_params).into()
    }

    pub fn get_status(&self) -> NtStatus {
        let status = unsafe {
            call_unsafe_wdf_function_binding!(WdfRequestGetStatus, self.as_ptr().cast(),)
        };

        status.into()
    }

    pub fn stop_acknowledge_requeue(self) {
        unsafe {
            call_unsafe_wdf_function_binding!(WdfRequestStopAcknowledge, self.as_ptr().cast(), 1);
        }
    }

    pub fn stop_acknowledge_no_requeue(request_id: RequestId) {
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestStopAcknowledge,
                request_id.0 as WDFREQUEST,
                0
            );
        }
    }

    pub fn cancel_sent_request(token: &SentRequestCancellationToken) -> bool {
        let res =
            unsafe { call_unsafe_wdf_function_binding!(WdfRequestCancelSentRequest, token.0) };

        res != 0
    }

    /// Reuses a previously created request object so it can be
    /// sent to an I/O target again.
    ///
    /// The `status` parameter specifies the NTSTATUS value to set
    /// in the reused request's IRP.
    ///
    /// # Returns
    ///
    /// A tuple containing the input and output user memory associated
    /// with the request from earlier, if any.
    pub fn reuse(
        &mut self,
        status: NtStatus,
    ) -> NtResult<(Option<OwnedMemory>, Option<OwnedMemory>)> {
        let mut reuse_params = init_wdf_struct!(WDF_REQUEST_REUSE_PARAMS);
        reuse_params.Flags = 0; // WDF_REQUEST_REUSE_NO_FLAGS
        reuse_params.Status = status.code();
        reuse_params.NewIrp = ptr::null_mut();

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfRequestReuse,
                self.as_ptr().cast(),
                &mut reuse_params,
            )
        }
        .map(|| {
            (unsafe { self.retrieve_user_input_memory() }, unsafe {
                self.retrieve_user_output_memory()
            })
        })
    }

    unsafe fn get_io_queue_from_raw<'a>(raw_request: WDFREQUEST) -> Option<&'a Opaque<IoQueue>> {
        unsafe {
            let queue = call_unsafe_wdf_function_binding!(WdfRequestGetIoQueue, raw_request);
            if queue.is_null() {
                None
            } else {
                Some(&*queue.cast::<Opaque<IoQueue>>())
            }
        }
    }

    fn get_context_mut_or_attach_new(&mut self) -> NtResult<&mut RequestContext> {
        if RequestContext::try_get_mut(self).is_none() {
            RequestContext::attach(
                self,
                RequestContext {
                    evt_request_cancel: None,
                    evt_request_completion_routine: None,
                },
            )?;
        }

        Ok(RequestContext::get_mut(self))
    }

    /// Returns a pointer to the WDM IRP associated with this
    /// request.
    /// TODO: callers can do real damage through the raw IRP
    /// pointer. We would want to wrap it in safe abstractions
    /// or get rid of this function entirely.
    pub fn wdm_get_irp(&self) -> wdk_sys::PIRP {
        unsafe { call_unsafe_wdf_function_binding!(WdfRequestWdmGetIrp, self.as_ptr().cast(),) }
    }
}

impl Handle for Request {
    fn as_ptr(&self) -> WDFOBJECT {
        self.0.cast()
    }

    fn type_name() -> String {
        String::from("Request")
    }
}

/// Although `Request` carries a raw pointer type, `WDFREQUEST`,
/// it is still `Sync` because all the C methods on `WDFREQUEST`
/// are thread-safe and therefore all the `Request` methods which
/// call these C methods are also thread-safe
unsafe impl Sync for Request {}

/// Although `Request` carries a raw pointer type, `WDFREQUEST`,
/// it is still `Send` because it uniquely owns the `WDFREQUEST`
/// pointer
unsafe impl Send for Request {}

pub trait CancellableRequestStore {
    fn add(&mut self, request: CancellableRequest);
    fn take(&mut self, id: RequestId) -> Option<CancellableRequest>;
}

impl CancellableRequestStore for Option<CancellableRequest> {
    fn add(&mut self, request: CancellableRequest) {
        *self = Some(request);
    }

    fn take(&mut self, id: RequestId) -> Option<CancellableRequest> {
        if let Some(request) = self.take() {
            if request.id() == id {
                return Some(request);
            } else {
                *self = Some(request);
            }
        }
        None
    }
}

impl CancellableRequestStore for Vec<CancellableRequest> {
    fn add(&mut self, request: CancellableRequest) {
        self.push(request);
    }

    fn take(&mut self, id: RequestId) -> Option<CancellableRequest> {
        if let Some(position) = self.iter().position(|r| r.id() == id) {
            Some(self.remove(position))
        } else {
            None
        }
    }
}

#[object_context(Request)]
struct RequestContext {
    evt_request_cancel: Option<fn(&RequestCancellationToken)>,
    evt_request_completion_routine: Option<fn(RequestCompletionToken, &Opaque<IoTarget>)>,
}

/// Macro that defines input and output memory contexts
/// and methods to retrieve and set them
macro_rules! define_user_memory_context {
    (input) => {
        define_user_memory_context!(@impl UserInputMemoryContext, retrieve_user_input_memory, set_user_input_memory);
    };
    (output) => {
        define_user_memory_context!(@impl UserOutputMemoryContext, retrieve_user_output_memory, set_user_output_memory);
    };

    // helper: contains the common implementation for any per-request-memory context
    (@impl $ctx_name:ident, $retrieve_fn:ident, $set_fn:ident) => {
        #[object_context(Request)]
        struct $ctx_name {
            memory: Option<OwnedMemory>,
        }

        impl Request {
            /// Extracts user memory if available
            ///
            /// # Safety
            ///
            /// It is possible the formatting info in WDFREQUEST is
            /// refrencing this memory. Make sure the request is not
            /// formatted before calling this method. Because of this
            /// the best place to call it is in from [`reuse`] method
            /// above because it clears the formatting info
            unsafe fn $retrieve_fn(&mut self) -> Option<OwnedMemory> {
                let context = $ctx_name::get_mut(self);
                context.memory.take()
            }

            /// Sets the user memory for this request. This is used in when request
            /// is sent to an I/O target and the driver needs to keep the user memory alive until
            /// the completion routine is called.
            pub(crate) fn $set_fn(&mut self, memory: OwnedMemory) -> NtResult<()> {
                match $ctx_name::try_get_mut(self) {
                    Some(context) => {
                        if context.memory.is_some() {
                            // We cannot allow overrwriting an already existing user memory
                            // as it may be being referenced to from formatting info within
                            // WDFREQUEST and that would cause a dangling reference.
                            Err(NtStatusError::from(status_codes::STATUS_INVALID_DEVICE_REQUEST))
                        } else {
                            context.memory = Some(memory);
                            Ok(())
                        }
                    }
                    None => $ctx_name::attach(self, $ctx_name { memory: Some(memory) }),
                }
            }
        }
    };
}

define_user_memory_context!(input);
define_user_memory_context!(output);

pub extern "C" fn __evt_request_cancel(request: WDFREQUEST) {
    let safe_request = unsafe { &Request::from_raw(request as _) };
    let context = RequestContext::get(safe_request);
    let Some(evt_request_cancel) = context.evt_request_cancel else {
        panic!("Request cancellation callback called but no user callback set");
    };
    let token = unsafe { RequestCancellationToken::new(request as _) };
    (evt_request_cancel)(&token);
}

pub extern "C" fn __evt_request_completion_routine(
    request: WDFREQUEST,
    target: WDFIOTARGET,
    _params: *mut WDF_REQUEST_COMPLETION_PARAMS,
    _context: WDFCONTEXT,
) {
    let safe_req = unsafe { Request::from_raw(request as _) };
    let context = RequestContext::get(&safe_req);
    let Some(callback) = context.evt_request_completion_routine else {
        panic!("Request's completion routine called but not user callback was set");
    };

    // Not passing on params to user callback because it
    // could complete the request and then try use the
    // param value which would be unsafe. Instead the user is
    // allowed to get access to params only by calling
    // `Request::get_completion_params` inside their callback.
    // That way params cannot outlive the request
    callback(unsafe { RequestCompletionToken::new(request) }, unsafe {
        &*(target.cast::<Opaque<IoTarget>>())
    });
}

#[derive(Debug)]
pub struct RequestCompletionParams<'a> {
    pub request_type: RequestType,
    pub io_status: IoStatusBlock,
    pub parameters: RequestCompletionParamDetails<'a>,
}

impl<'a> From<&WDF_REQUEST_COMPLETION_PARAMS> for RequestCompletionParams<'a> {
    fn from(raw: &WDF_REQUEST_COMPLETION_PARAMS) -> Self {
        let request_type = RequestType::from(raw.Type);
        let io_status = IoStatusBlock::from(raw.IoStatus);

        let parameters = match request_type {
            RequestType::Write => {
                let write = unsafe { &raw.Parameters.Write };
                RequestCompletionParamDetails::Write {
                    buffer: unsafe { &*(write.Buffer.cast::<Memory>()) },
                    length: write.Length as usize,
                    offset: write.Offset as usize,
                }
            }
            RequestType::Read => {
                let read = unsafe { &raw.Parameters.Read };
                RequestCompletionParamDetails::Read {
                    buffer: unsafe { &*(read.Buffer.cast::<Memory>()) },
                    length: read.Length as usize,
                    offset: read.Offset as usize,
                }
            }
            RequestType::DeviceControl | RequestType::DeviceControlInternal => {
                let ioctl = unsafe { &raw.Parameters.Ioctl };
                RequestCompletionParamDetails::Ioctl {
                    io_control_code: ioctl.IoControlCode,
                    input_buffer: unsafe { &*(ioctl.Input.Buffer.cast::<Memory>()) },
                    input_offset: ioctl.Input.Offset as usize,
                    output_buffer: unsafe { &*(ioctl.Output.Buffer.cast::<Memory>()) },
                    output_offset: ioctl.Output.Offset as usize,
                    output_length: ioctl.Output.Length as usize,
                }
            }
            RequestType::Usb => RequestCompletionParamDetails::Usb {
                completion: UsbRequestCompletionParams::from(unsafe {
                    &*raw.Parameters.Usb.Completion
                }),
            },
            _ => unsafe {
                let others = &raw.Parameters.Others;
                RequestCompletionParamDetails::Others {
                    argument1: others.Argument1.Value as usize,
                    argument2: others.Argument2.Value as usize,
                    argument3: others.Argument3.Value as usize,
                    argument4: others.Argument4.Value as usize,
                }
            },
        };

        Self {
            request_type,
            io_status,
            parameters,
        }
    }
}

#[derive(Debug)]
pub struct IoStatusBlock {
    pub status: NtStatus,
    pub information: usize,
}

impl From<IO_STATUS_BLOCK> for IoStatusBlock {
    fn from(raw: IO_STATUS_BLOCK) -> Self {
        Self {
            status: NtStatus::from(unsafe { raw.__bindgen_anon_1.Status }),
            information: raw.Information as usize,
        }
    }
}

#[derive(Debug)]
pub enum RequestCompletionParamDetails<'a> {
    Write {
        buffer: &'a Memory,
        length: usize,
        offset: usize,
    },
    Read {
        buffer: &'a Memory,
        length: usize,
        offset: usize,
    },
    Ioctl {
        io_control_code: u32,
        input_buffer: &'a Memory,
        input_offset: usize,
        output_buffer: &'a Memory,
        output_offset: usize,
        output_length: usize,
    },
    Others {
        argument1: usize,
        argument2: usize,
        argument3: usize,
        argument4: usize,
    },
    Usb {
        completion: UsbRequestCompletionParams<'a>,
    },
}

pub struct CancellableRequest(Request);

impl CancellableRequest {
    pub fn id(&self) -> RequestId {
        self.0.id()
    }

    pub fn complete(self, status: NtStatus) {
        // Ignoring the return value because the call to this method can
        // come from both the request cancellation, event where unmarking
        // is not required, and from other places, where unmarking is
        // required, and we know that it will fail in the former case.
        // At this point know where the call came from so we just ignore
        // the return value.
        // TODO: Redesign this and make sure we can handle genuine errors
        let _ = unsafe {
            call_unsafe_wdf_function_binding!(WdfRequestUnmarkCancelable, self.as_ptr().cast())
        };

        self.0.complete(status);
    }

    pub fn complete_with_information(self, status: NtStatus, information: usize) {
        // Ignoring the return value for the same reason as in `complete`
        let _ = unsafe {
            call_unsafe_wdf_function_binding!(WdfRequestUnmarkCancelable, self.as_ptr().cast())
        };

        self.0.complete_with_information(status, information);
    }
}

impl Handle for CancellableRequest {
    fn as_ptr(&self) -> WDFOBJECT {
        self.0.as_ptr()
    }

    fn type_name() -> String {
        String::from("CancellableRequest")
    }
}

/// A request that has been sent to an I/O target.
#[derive(Debug)]
pub struct SentRequest(Request);

impl SentRequest {
    pub fn id(&self) -> RequestId {
        self.0.id()
    }

    pub fn into_request(self, _token: RequestCompletionToken) -> Request {
        // _token is required only to ensure that
        // caller is calling this from or evt_request_completion_routine
        self.0
    }

    pub fn get_cancellation_token(&self) -> SentRequestCancellationToken {
        unsafe { SentRequestCancellationToken::new(self.0.as_ptr().cast()) }
    }
}

impl Handle for SentRequest {
    fn as_ptr(&self) -> WDFOBJECT {
        self.0.as_ptr()
    }

    fn type_name() -> String {
        String::from("SentRequest")
    }
}

#[derive(Debug)]
pub struct RequestCancellationToken(Request);

impl RequestCancellationToken {
    unsafe fn new(inner: WDFREQUEST) -> Self {
        Self(unsafe { Request::from_raw(inner) })
    }

    pub fn request_id(&self) -> RequestId {
        self.0.id()
    }

    pub fn get_io_queue(&self) -> Option<&Opaque<IoQueue>> {
        self.0.get_io_queue()
    }
}

#[derive(Debug)]
pub struct RequestCompletionToken(Request);

impl RequestCompletionToken {
    unsafe fn new(inner: WDFREQUEST) -> Self {
        Self(unsafe { Request::from_raw(inner) })
    }

    pub fn request_id(&self) -> RequestId {
        self.0.id()
    }

    pub fn get_io_queue(&self) -> Option<&Opaque<IoQueue>> {
        self.0.get_io_queue()
    }
}

#[derive(Debug)]
pub struct SentRequestCancellationToken(WDFREQUEST);

impl SentRequestCancellationToken {
    unsafe fn new(inner: WDFREQUEST) -> Self {
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfObjectReferenceActual,
                inner.cast(),
                core::ptr::null_mut(),
                line!() as i32,
                c"request.rs".as_ptr(),
            );
        }
        Self(inner)
    }

    pub fn request_id(&self) -> RequestId {
        RequestId(self.0 as usize)
    }

    pub fn get_io_queue(&self) -> Option<&Opaque<IoQueue>> {
        unsafe { Request::get_io_queue_from_raw(self.0) }
    }
}

impl Drop for SentRequestCancellationToken {
    fn drop(&mut self) {
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfObjectDereferenceActual,
                self.0.cast(),
                core::ptr::null_mut(),
                line!() as i32,
                c"request.rs".as_ptr(),
            );
        }
    }
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequestId(usize);

/// Request parameters returned by [`Request::get_parameters`] method
#[derive(Copy, Clone)]
pub struct RequestParameters(WDF_REQUEST_PARAMETERS);

impl RequestParameters {
    /// Returns the request type.
    pub fn request_type(&self) -> RequestType {
        self.0.Type.into()
    }

    /// Returns the minor function code, if any.
    pub fn minor_function(&self) -> u8 {
        self.0.MinorFunction
    }

    /// Returns the IOCTL code for a device I/O control request,
    /// or `None` if the request type is not
    /// [`RequestType::DeviceControl`] or
    /// [`RequestType::DeviceControlInternal`].
    pub fn ioctl_code(&self) -> Option<u32> {
        self.is_device_io_control()
            .then(|| unsafe { self.0.Parameters.DeviceIoControl.IoControlCode })
    }

    /// Returns the input buffer length for a device I/O control
    /// request, or `None` if the request type is not
    /// [`RequestType::DeviceControl`] or
    /// [`RequestType::DeviceControlInternal`].
    pub fn ioctl_input_buffer_length(&self) -> Option<usize> {
        self.is_device_io_control()
            .then(|| unsafe { self.0.Parameters.DeviceIoControl.InputBufferLength })
    }

    /// Returns the output buffer length for a device I/O control
    /// request, or `None` if the request type is not
    /// [`RequestType::DeviceControl`] or
    /// [`RequestType::DeviceControlInternal`].
    pub fn ioctl_output_buffer_length(&self) -> Option<usize> {
        self.is_device_io_control()
            .then(|| unsafe { self.0.Parameters.DeviceIoControl.OutputBufferLength })
    }

    /// Returns the raw `WDF_REQUEST_PARAMETERS` struct.
    pub fn as_raw(&self) -> &WDF_REQUEST_PARAMETERS {
        &self.0
    }

    /// Returns `true` if the request type is a device I/O control
    /// request.
    fn is_device_io_control(&self) -> bool {
        matches!(
            self.request_type(),
            RequestType::DeviceControl | RequestType::DeviceControlInternal
        )
    }
}

enum_mapping! {
    infallible;
    pub enum RequestType: WDF_REQUEST_TYPE {
        Create = WdfRequestTypeCreate,
        CreateNamedPipe = WdfRequestTypeCreateNamedPipe,
        Close = WdfRequestTypeClose,
        Read = WdfRequestTypeRead,
        Write = WdfRequestTypeWrite,
        QueryInformation = WdfRequestTypeQueryInformation,
        SetInformation = WdfRequestTypeSetInformation,
        QueryEA = WdfRequestTypeQueryEA,
        SetEA = WdfRequestTypeSetEA,
        FlushBuffers = WdfRequestTypeFlushBuffers,
        QueryVolumeInformation = WdfRequestTypeQueryVolumeInformation,
        SetVolumeInformation = WdfRequestTypeSetVolumeInformation,
        DirectoryControl = WdfRequestTypeDirectoryControl,
        FileSystemControl = WdfRequestTypeFileSystemControl,
        DeviceControl = WdfRequestTypeDeviceControl,
        DeviceControlInternal = WdfRequestTypeDeviceControlInternal,
        Shutdown = WdfRequestTypeShutdown,
        LockControl = WdfRequestTypeLockControl,
        Cleanup = WdfRequestTypeCleanup,
        CreateMailSlot = WdfRequestTypeCreateMailSlot,
        QuerySecurity = WdfRequestTypeQuerySecurity,
        SetSecurity = WdfRequestTypeSetSecurity,
        Power = WdfRequestTypePower,
        SystemControl = WdfRequestTypeSystemControl,
        DeviceChange = WdfRequestTypeDeviceChange,
        QueryQuota = WdfRequestTypeQueryQuota,
        SetQuota = WdfRequestTypeSetQuota,
        Pnp = WdfRequestTypePnp,
        Other = WdfRequestTypeOther,
        Usb = WdfRequestTypeUsb,
        NoFormat = WdfRequestTypeNoFormat,
        Max = WdfRequestTypeMax
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct RequestStopActionFlags: u32 {
        const SUSPEND = 0x00000001;
        const PURGE = 0x00000002;
        const CANCELABLE = 0x1000_0000;
    }
}
