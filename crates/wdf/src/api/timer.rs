use crate::api::{
    error::NtResult,
    object::{wdf_struct_size, FrameworkObject, FrameworkObjectType},
};
use core::{mem::MaybeUninit, ptr::null_mut};
use wdf_macros::object_context;
use wdk::nt_success;
use wdk_sys::{
    call_unsafe_wdf_function_binding, NT_SUCCESS, WDFOBJECT, WDFTIMER, WDF_OBJECT_ATTRIBUTES,
    WDF_TIMER_CONFIG,
};

// TODO: Make timer more ergonomic and safer. It's
// not fully safe yet. For example it lets you pass
// a negative value for due time to start when
// use_high_resolution_timer is set to true which would
// crash the system.

/// A WDF timer
pub struct Timer(WDFTIMER);

impl Timer {
    pub fn create<'a, P: FrameworkObject>(config: &TimerConfig<'a, P>) -> NtResult<Self> {
        let context = TimerContext {
            evt_timer_func: config.evt_timer_func,
            parent_type: P::object_type(),
        };

        let mut timer: WDFTIMER = null_mut();

        let mut attributes = WDF_OBJECT_ATTRIBUTES::default();
        attributes.ParentObject = config.parent.as_ptr();

        let mut config: WDF_TIMER_CONFIG = config.into();

        // SAFETY: The resulting ffi object is stored in a private member and not
        // accessible outside of this module, and this module guarantees that it is
        // always in a valid state.
        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfTimerCreate,
                &mut config as *mut _,
                &mut attributes as *mut _,
                &mut timer as *mut _,
            )
        };

        if NT_SUCCESS(status) {
            let mut timer = unsafe { Timer::from_ptr(timer as *mut _) };

            TimerContext::attach(&mut timer, context)?;

            Ok(timer)
        } else {
            Err(status.into())
        }
    }

    // TODO: takes &mut self instead of &self because right now
    // we don't have a good design for representation thread safey
    // of WDF objects to the driver code. So we're using &self for
    // the moment as it lets us put the object in the object context.
    // When we have a good design for thread safe reprensetation we
    // will change it back to &mut self
    pub fn start(&self, due_time: i64) -> bool {
        // TODO: use something like duration instead of i64 for due_time
        unsafe { call_unsafe_wdf_function_binding!(WdfTimerStart, self.0, due_time) != 0 }
    }

    // TODO: Change to &mut self. See comment on start() method
    pub fn stop(&self, wait: bool) -> bool {
        unsafe { call_unsafe_wdf_function_binding!(WdfTimerStop, self.0, wait as u8) != 0 }
    }

    pub fn get_parent_object<P: FrameworkObject>(&self) -> Option<P> {
        let parent = unsafe { call_unsafe_wdf_function_binding!(WdfTimerGetParentObject, self.0) };

        if !parent.is_null() {
            TimerContext::get(&self).and_then(|context| {
                if context.parent_type == P::object_type() {
                    Some(unsafe { P::from_ptr(parent) })
                } else {
                    None
                }
            })
        } else {
            None
        }
    }
}

impl FrameworkObject for Timer {
    unsafe fn from_ptr(inner: WDFOBJECT) -> Self {
        Self(inner as WDFTIMER)
    }

    fn as_ptr(&self) -> *mut core::ffi::c_void {
        self.0 as *mut _
    }

    fn object_type() -> FrameworkObjectType {
        FrameworkObjectType::Timer
    }
}

/// SAFETY: This is safe because all the WDF functions
/// that operate on WDFTIMER do so in a thread-safe manner.
/// As a result, all the Rust methods on this struct are
/// also thread-safe.
unsafe impl Send for Timer {}
unsafe impl Sync for Timer {}

pub struct TimerConfig<'a, P: FrameworkObject> {
    pub evt_timer_func: fn(&mut Timer),
    pub period: u32,
    pub tolerable_delay: u32,
    pub use_high_resolution_timer: bool,
    pub parent: &'a P,
}

impl<'a, P: FrameworkObject> TimerConfig<'a, P> {
    pub fn new_non_periodic(parent: &'a P, evt_timer_func: fn(&mut Timer)) -> Self {
        Self {
            evt_timer_func,
            period: 0,
            tolerable_delay: 0,
            use_high_resolution_timer: false,
            parent: &parent,
        }
    }

    pub fn new_periodic(
        parent: &'a P,
        evt_timer_func: fn(&mut Timer),
        period: u32,
        tolerable_delay: u32,
        use_high_resolution_timer: bool,
    ) -> Self {
        Self {
            evt_timer_func,
            period,
            tolerable_delay,
            use_high_resolution_timer,
            parent: &parent,
        }
    }
}

impl<'a, P: FrameworkObject> From<&TimerConfig<'a, P>> for WDF_TIMER_CONFIG {
    fn from(config: &TimerConfig<'a, P>) -> Self {
        let mut wdf_config: WDF_TIMER_CONFIG = unsafe { MaybeUninit::zeroed().assume_init() };

        wdf_config.Size = wdf_struct_size!(WDF_TIMER_CONFIG);
        wdf_config.Period = config.period;
        wdf_config.AutomaticSerialization = 0;
        wdf_config.TolerableDelay = config.tolerable_delay;
        wdf_config.UseHighResolutionTimer = config.use_high_resolution_timer as u8;
        wdf_config.EvtTimerFunc = Some(__evt_timer_func);

        wdf_config
    }
}

#[object_context(Timer)]
struct TimerContext {
    evt_timer_func: fn(&mut Timer),
    parent_type: FrameworkObjectType,
}

pub extern "C" fn __evt_timer_func(timer: WDFTIMER) {
    let mut timer = unsafe { Timer::from_ptr(timer as WDFOBJECT) };
    if let Some(timer_state) = TimerContext::get(&timer) {
        (timer_state.evt_timer_func)(&mut timer);
    }
}
