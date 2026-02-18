use core::{ptr::null_mut, sync::atomic::AtomicIsize};

use wdf_macros::object_context_with_ref_count_check;
use wdk_sys::{WDF_WORKITEM_CONFIG, WDFWORKITEM, call_unsafe_wdf_function_binding};

use super::{
    device::Device,
    init_wdf_struct,
    object::{Handle, impl_ref_counted_handle, init_attributes},
    result::{NtResult, StatusCodeExt},
    sync::{Arc, Opaque},
};

impl_ref_counted_handle!(WorkItem, WorkItemContext);

impl WorkItem {
    /// Creates a new WDF work item object.
    ///
    /// The work item callback `evt_work_item_func` in the config
    /// will be invoked when the work item is enqueued and
    /// subsequently picked up by a system worker thread.
    pub fn create(parent: &Device, config: &WorkItemConfig) -> NtResult<Arc<Self>> {
        let context = WorkItemContext {
            ref_count: AtomicIsize::new(0),
            evt_work_item_func: config.evt_work_item_func,
        };

        let mut work_item: WDFWORKITEM = null_mut();

        let mut attributes = init_attributes();
        attributes.ParentObject = parent.as_ptr();

        let mut wdf_config: WDF_WORKITEM_CONFIG = config.into();

        // SAFETY: The resulting ffi object is stored in a private member and not
        // accessible outside of this module, and this module guarantees that it is
        // always in a valid state.
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfWorkItemCreate,
                &mut wdf_config,
                &mut attributes,
                &mut work_item,
            )
        }
        .and_then(|| {
            WorkItemContext::attach(unsafe { &*work_item.cast() }, context)?;
            let work_item = unsafe { Arc::from_raw(work_item.cast()) };

            Ok(work_item)
        })
    }

    /// Enqueues the work item for execution.
    ///
    /// The work item's `evt_work_item_func` callback will be
    /// invoked by a system worker thread.
    ///
    /// Note: If the work item is already in the queue, it will
    /// not be added again. Each call to `enqueue` results in at
    /// most one execution of the callback.
    pub fn enqueue(&self) {
        // SAFETY: The work item handle is guaranteed to be valid
        // by the module's invariants.
        unsafe {
            call_unsafe_wdf_function_binding!(WdfWorkItemEnqueue, self.as_ptr().cast());
        }
    }

    /// Waits for the work item callback to complete if it is
    /// currently executing, and then removes the work item
    /// from the queue if it was queued.
    ///
    /// This must not be called from within the work item callback.
    pub fn flush(&self) {
        // SAFETY: The work item handle is guaranteed to be valid
        // by the module's invariants.
        unsafe {
            call_unsafe_wdf_function_binding!(WdfWorkItemFlush, self.as_ptr().cast());
        }
    }

    /// Returns a reference to the parent device of this work item.
    pub fn get_device(&self) -> &Device {
        let device_ptr = unsafe {
            call_unsafe_wdf_function_binding!(WdfWorkItemGetParentObject, self.as_ptr().cast())
        };

        unsafe { &*(device_ptr.cast::<Device>()) }
    }
}

/// Configuration for creating a [`WorkItem`].
pub struct WorkItemConfig {
    /// The callback function invoked when the work item executes.
    pub evt_work_item_func: fn(&Opaque<WorkItem>),
    /// Whether the framework synchronizes execution of the work
    /// item callback with callbacks from other objects in the
    /// parent's synchronization scope. Defaults to `false`.
    pub automatic_serialization: bool,
}

impl WorkItemConfig {
    /// Creates a new `WorkItemConfig` with the given callback.
    /// `automatic_serialization` defaults to `false`.
    pub fn new(evt_work_item_func: fn(&Opaque<WorkItem>)) -> Self {
        Self {
            evt_work_item_func,
            automatic_serialization: false,
        }
    }
}

impl From<&WorkItemConfig> for WDF_WORKITEM_CONFIG {
    fn from(config: &WorkItemConfig) -> Self {
        let mut wdf_config = init_wdf_struct!(WDF_WORKITEM_CONFIG);
        wdf_config.AutomaticSerialization = config.automatic_serialization as u8;
        wdf_config.EvtWorkItemFunc = Some(__evt_work_item_func);

        wdf_config
    }
}

#[object_context_with_ref_count_check(WorkItem)]
struct WorkItemContext {
    ref_count: AtomicIsize,
    evt_work_item_func: fn(&Opaque<WorkItem>),
}

pub extern "C" fn __evt_work_item_func(work_item: WDFWORKITEM) {
    let work_item_ref = unsafe { &*work_item.cast::<WorkItem>() };
    let work_item_state = WorkItemContext::get(work_item_ref);
    let work_item = unsafe { &*work_item.cast::<Opaque<WorkItem>>() };
    (work_item_state.evt_work_item_func)(work_item);
}
