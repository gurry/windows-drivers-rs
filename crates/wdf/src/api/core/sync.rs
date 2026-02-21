use alloc::string::String;
use core::{
    cell::UnsafeCell,
    ffi::c_void,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    ptr::NonNull,
    sync::atomic::{AtomicIsize, AtomicUsize, Ordering, fence},
};

use wdk::println;
use wdk_sys::{WDFOBJECT, WDFSPINLOCK, WDFWAITLOCK, call_unsafe_wdf_function_binding};

use super::{
    Timeout,
    object::{Handle, RefCountedHandle, bug_check, init_attributes},
    result::{NtResult, StatusCodeExt},
};

/// WDF Spin Lock
pub struct SpinLock<T> {
    wdf_spin_lock: WDFSPINLOCK,
    data: UnsafeCell<T>,
}

/// `SpinLock` requires `T` to be `Send` because non-`Send`
/// types can lead to situations where a thread NOT holding
/// the lock can also access the data. An example of this
/// is `Rc` wherein the lock will protect only one clone of
/// `Rc` and another thread can still access the data through
/// another clone without taking the lock.
unsafe impl<T> Sync for SpinLock<T> where T: Send {}

impl<T> SpinLock<T> {
    /// Construct a WDF Spin Lock object with data
    pub fn create(data: T) -> NtResult<Self> {
        let mut spin_lock = Self {
            wdf_spin_lock: core::ptr::null_mut(),
            data: UnsafeCell::new(data),
        };

        let mut attributes = init_attributes();

        // SAFETY: The resulting ffi object is stored in a private member and not
        // accessible outside of this module, and this module guarantees that it is
        // always in a valid state.
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfSpinLockCreate,
                &mut attributes,
                &mut spin_lock.wdf_spin_lock,
            )
        }
        .map(|| spin_lock)
    }

    /// Acquire the spinlock and return a guard that will release the spinlock
    /// when dropped
    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        // SAFETY: `wdf_spin_lock` is a private member of `SpinLock`, originally created
        // by WDF, and this module guarantees that it is always in a valid state.
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockAcquire, self.wdf_spin_lock);
        }
        SpinLockGuard {
            spin_lock: self,
        }
    }
}

impl<T> Drop for SpinLock<T> {
    fn drop(&mut self) {
        // SAFETY: `wdf_spin_lock` is a private member of `SpinLock`, originally created
        // by WDF, and this module guarantees that it is always in a valid state.
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, self.wdf_spin_lock.cast());
        }
    }
}

/// RAII guard for `SpinLock`.
///
/// The lock is acquired when the guard is created and released when the guard
/// is dropped.
pub struct SpinLockGuard<'a, T> {
    spin_lock: &'a SpinLock<T>,
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        // SAFETY: `wdf_spin_lock` is a private member of `SpinLock`, originally created
        // by WDF, and this module guarantees that it is always in a valid state.
        unsafe {
            call_unsafe_wdf_function_binding!(WdfSpinLockRelease, self.spin_lock.wdf_spin_lock);
        }
    }
}

impl<'a, T> Deref for SpinLockGuard<'a, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.spin_lock.data.get() }
    }
}

impl<'a, T> DerefMut for SpinLockGuard<'a, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.spin_lock.data.get() }
    }
}

/// WDF Wait Lock
pub struct WaitLock<T> {
    wdf_wait_lock: WDFWAITLOCK,
    data: UnsafeCell<T>,
}

/// `WaitLock` requires `T` to be `Send` because non-`Send`
/// types can lead to situations where a thread NOT holding
/// the lock can also access the data. An example of this
/// is `Rc` wherein the lock will protect only one clone of
/// `Rc` and another thread can still access the data through
/// another clone without taking the lock.
unsafe impl<T> Sync for WaitLock<T> where T: Send {}

impl<T> WaitLock<T> {
    /// Construct a WDF Wait Lock object with data.
    pub fn create(data: T) -> NtResult<Self> {
        let mut wait_lock = Self {
            wdf_wait_lock: core::ptr::null_mut(),
            data: UnsafeCell::new(data),
        };

        let mut attributes = init_attributes();

        // SAFETY: The resulting ffi object is stored in a private member and not
        // accessible outside of this module, and this module guarantees that it is
        // always in a valid state.
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfWaitLockCreate,
                &mut attributes,
                &mut wait_lock.wdf_wait_lock,
            )
        }
        .map(|| wait_lock)
    }

    /// Acquire the wait lock, blocking indefinitely until it
    /// becomes available.
    ///
    /// Returns a guard that releases the lock when dropped.
    pub fn lock(&self) -> WaitLockGuard<'_, T> {
        // SAFETY: `wdf_wait_lock` is a private member, originally created
        // by WDF, and this module guarantees that it is always in a valid state.
        // Passing null for the timeout means wait indefinitely, which always
        // succeeds.
        let _ = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfWaitLockAcquire,
                self.wdf_wait_lock,
                core::ptr::null_mut(),
            )
        };
        WaitLockGuard { wait_lock: self }
    }

    /// Try to acquire the wait lock with a timeout.
    ///
    /// Returns `Some(guard)` if the lock was acquired, or `None`
    /// if the timeout elapsed.
    pub fn try_lock(&self, timeout: Timeout) -> Option<WaitLockGuard<'_, T>> {
        let mut timeout_value = timeout.as_wdf_timeout();

        // SAFETY: `wdf_wait_lock` is a private member, originally created
        // by WDF, and this module guarantees that it is always in a valid state.
        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfWaitLockAcquire,
                self.wdf_wait_lock,
                &mut timeout_value,
            )
        };

        if status.is_success() {
            Some(WaitLockGuard { wait_lock: self })
        } else {
            None
        }
    }
}

impl<T> Drop for WaitLock<T> {
    fn drop(&mut self) {
        // SAFETY: `wdf_wait_lock` is a private member, originally created
        // by WDF, and this module guarantees that it is always in a valid state.
        unsafe {
            call_unsafe_wdf_function_binding!(WdfObjectDelete, self.wdf_wait_lock.cast());
        }
    }
}

/// RAII guard for [`WaitLock`].
///
/// The lock is acquired when the guard is created and released when the guard
/// is dropped.
pub struct WaitLockGuard<'a, T> {
    wait_lock: &'a WaitLock<T>,
}

impl<'a, T> Drop for WaitLockGuard<'a, T> {
    fn drop(&mut self) {
        // SAFETY: `wdf_wait_lock` is a private member, originally created
        // by WDF, and this module guarantees that it is always in a valid state.
        unsafe {
            call_unsafe_wdf_function_binding!(WdfWaitLockRelease, self.wait_lock.wdf_wait_lock);
        }
    }
}

impl<'a, T> Deref for WaitLockGuard<'a, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.wait_lock.data.get() }
    }
}

impl<'a, T> DerefMut for WaitLockGuard<'a, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.wait_lock.data.get() }
    }
}

/// Arc for WDF object handles
pub struct Arc<T: RefCountedHandle> {
    // NonNull enables certain compiler optimizations
    // such as making Option<Arc<T>> have the same size
    // as *mut c_void
    ptr: NonNull<c_void>,
    _marker: PhantomData<T>,
}

impl<T: RefCountedHandle> Arc<T> {
    /// Creates a new `Arc` from a raw WDF object pointer and
    /// increments the ref count by 1.
    ///
    /// # Safety
    ///
    /// The following requirements must be met:
    /// - `ptr` must be non-null
    /// - `ptr` must be a valid WDF object pointer that implements
    /// `RefCountedHandle`.
    /// - The ref count of the object pointed to by `ptr` must be 0
    /// or greater (`from_raw` will increment it by 1)
    pub(crate) unsafe fn from_raw(ptr: WDFOBJECT) -> Self {
        let obj = unsafe { &*ptr.cast::<T>() };
        let ref_count = obj.get_ref_count();

        // Relaxed ordering is fine here since we do not care if
        // operations on ptr (i.e. the WDF pointer we are carrying)
        // get reordered with respect to fetch_add.
        // It is totally okay for an access to ptr to occur after
        // the fetch_add call because the object is guaranteed to be
        // alive thanks to this very ref count increment.
        // Here we also prevent the ref count from overflowing by bugchecking
        // early because an overflow would lead to all kinds of unsafety.
        if ref_count.fetch_add(1, Ordering::Relaxed) > isize::MAX / 2 {
            let ref_count = ref_count.load(Ordering::Relaxed);
            bug_check(0xDEADDEAD, ptr, Some(ref_count as usize));
        }

        unsafe { Self::from_raw_no_inc(ptr) }
    }

    /// Creates a new `Arc` from a raw WDF object pointer.
    /// Does not increment the ref count.
    ///
    /// # Safety
    ///
    /// The following requirements must be met:
    /// - `ptr` must be non-null
    /// - `ptr` must be a valid WDF object pointer
    unsafe fn from_raw_no_inc(ptr: WDFOBJECT) -> Self {
        // SAFETY: the incoming `ptr` is required to be non-null
        // by the safety contract of `from_raw`
        let ptr = unsafe { NonNull::new_unchecked(ptr) };

        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Returns a mutable reference to the inner value
    pub fn get_mut(&mut self) -> Option<RefMut<'_, T>> {
        let ref_count = self.get_ref_count();

        // Attempt 1 → -1 transition
        if ref_count
            .compare_exchange(1, -1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(RefMut {
                value: unsafe { &mut *self.as_ptr().cast::<T>() },
            })
        } else {
            None
        }
    }

    #[inline(always)]
    fn get_ref_count(&self) -> &AtomicIsize {
        let obj = unsafe { &*self.as_ptr().cast::<T>() };
        obj.get_ref_count()
    }
}

impl<T: RefCountedHandle> Clone for Arc<T> {
    fn clone(&self) -> Self {
        unsafe { Self::from_raw(self.as_ptr()) }
    }
}

impl<T: RefCountedHandle> Drop for Arc<T> {
    fn drop(&mut self) {
        let ref_count = self.get_ref_count();

        println!(
            "Drop {}: Ref count {}",
            Self::type_name(),
            ref_count.load(Ordering::Relaxed)
        );

        // We need to ensure here that if we are the thread doing
        // the final delete (i.e calling WdfObjectDelete) then
        // all other threads are done accessing ptr or we will get
        // a use-after-free. Hence we must form a happens-before
        // relationship with all the other threads calling drop.
        // We could have achieved that by using the AcqRel ordering
        // in fetch_sub. But WdfObjectDelete is called only when the
        // ref count reaches zero. Therefore as an optimization we
        // use only the Release ordering in fetch_sub and have a
        // separate Acquire fence inside the if block.
        let prev = ref_count.fetch_sub(1, Ordering::Release);
        if prev == 1 {
            fence(Ordering::Acquire);

            println!("Drop {}: Ref count 0. Deleting obj", Self::type_name());

            // SAFETY: The object is guaranteed to be valid here
            // because it is deleted only here and no place else
            unsafe {
                call_unsafe_wdf_function_binding!(WdfObjectDelete, self.as_ptr());
            }
        } else if prev < 1 {
            // Ref count went negative, which is a bug
            bug_check(0xDEADDEAD, self.as_ptr(), Some(prev as usize - 1));
        }
    }
}

impl<T: RefCountedHandle> Handle for Arc<T> {
    #[inline(always)]
    fn as_ptr(&self) -> WDFOBJECT {
        self.ptr.as_ptr()
    }

    fn type_name() -> String {
        let type_name = T::type_name();
        alloc::format!("Arc<{}>", type_name)
    }
}

impl<T: RefCountedHandle> Deref for Arc<T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.as_ptr().cast::<Self::Target>() }
    }
}

/// `Arc<T>` being `Sync` requires `T` to be `Sync`
/// because sharing `Arc<T>` effectively shares `T`. It
/// also requires `T` to be `Send` because any thread that
/// has `&Arc<T>` can call `clone` on it and get an `Arc<T>`.
/// Later that `Arc<T>` could drop `T` if it is the last
/// reference, implying that `T` is effectively being moved.
unsafe impl<T: RefCountedHandle + Sync + Send> Sync for Arc<T> {}

/// `Arc<T>` being `Send` requires `T` to be both `Send`
/// and `Sync` for the same reason as above.
unsafe impl<T: RefCountedHandle + Sync + Send> Send for Arc<T> {}

/// A mutable reference guard returned by `Arc::get_mut`
pub struct RefMut<'a, T: RefCountedHandle> {
    value: &'a mut T,
}

impl<T: RefCountedHandle> Drop for RefMut<'_, T> {
    fn drop(&mut self) {
        let ref_count = self.value.get_ref_count();

        // Release ordering for the next Arc::get_mut()
        // and Opaque::upgrade() to synchronize with
        let prev = ref_count.swap(1, Ordering::Release);

        if prev != -1 {
            bug_check(0xDEADDEAD, self.value.as_ptr(), Some(prev as usize));
        }
    }
}

impl<T: RefCountedHandle> Deref for RefMut<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.value
    }
}

impl<T: RefCountedHandle> DerefMut for RefMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value
    }
}

/// Opaque pointer to WDF object handle
///
/// It is only ever used as a reference `&Opaque<T>`. Can be
/// upgraded to `Arc<T>` if the underlying object is not
/// exclusively borrowed or deleted.
#[repr(C)]
pub struct Opaque<T> {
    _private: [u8; 0], // Prevents instantiation of the struct from driver code
    _marker: PhantomData<*mut T>, // *mut T disables Send and Sync. Just being conseravtive
}

impl<T: RefCountedHandle> Opaque<T> {
    pub fn upgrade(&self) -> Option<Arc<T>> {
        let inner_ptr = (self as *const Self).cast::<T>();
        let inner = unsafe { &*inner_ptr };

        let ref_count = inner.get_ref_count();

        let mut cur = ref_count.load(Ordering::Relaxed);

        loop {
            // Values < -1 are invalid for ref count
            if cur < -1 {
                bug_check(
                    0xDEADDEAD,
                    (self as *const Self).cast_mut().cast::<_>(),
                    Some(cur as usize),
                );
            }

            // Exclusively borrowed or deleted
            if cur == -1 || cur == 0 {
                // It is not safe to upgrade to Arc
                // under these conditions
                return None;
            }

            match ref_count.compare_exchange_weak(
                cur,
                cur + 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let arc = unsafe { Arc::from_raw_no_inc(inner_ptr as WDFOBJECT) };
                    return Some(arc);
                }
                Err(next) => cur = next,
            }
        }
    }
}

impl<T: RefCountedHandle> core::fmt::Debug for Opaque<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let ptr = self as *const Self;
        write!(f, "{ptr:p}")
    }
}

/// Thread-safe version of `OnceCell`
///
/// Like `OnceCell` it allows initialization only
/// once and getting access to `&T` after that.
/// All operations, including initialization, are
/// thread-safe.
///
/// Compared to `OnceLock` it uses no locks
/// and relies purely on atomic operations.
pub struct AtomicOnceCell<T> {
    init_state: AtomicUsize,
    inner: UnsafeCell<Option<T>>,
}

const UNINITIALIZED: usize = 0;
const INITIALIZING: usize = 1;
const INITIALIZED: usize = 2;

impl<T> AtomicOnceCell<T> {
    /// Creates a new `AtomicOnceCell` instance
    pub const fn new() -> Self {
        Self {
            init_state: AtomicUsize::new(UNINITIALIZED),
            inner: UnsafeCell::new(None),
        }
    }

    /// Initializes the cell with the given value.
    ///
    /// # Returns
    /// Returns `Ok(())` if the cell was successfully initialized,
    /// or `Err(value)` if it was already initialized.
    pub fn set(&self, value: T) -> Result<(), T> {
        if self
            .init_state
            .compare_exchange(
                UNINITIALIZED,
                INITIALIZING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            unsafe { (*self.inner.get()) = Some(value) };
            self.init_state.store(INITIALIZED, Ordering::Release);
            Ok(())
        } else {
            Err(value)
        }
    }

    /// Returns a reference to the inner value if the cell
    /// is initialized.
    ///
    /// # Returns
    /// `Some(&T)` if the cell is initialized, `None` otherwise.
    pub fn get(&self) -> Option<&T> {
        if self.init_state.load(Ordering::Acquire) == INITIALIZED {
            unsafe { (*self.inner.get()).as_ref() }
        } else {
            None
        }
    }
}

/// `AtomicOnceCell` contains two pieces of data:
/// the initialization state and the inner value `T`.
/// The initialization state being atomic is automatically
/// `Sync`. Therefore `AtomicOnceCell` is `Sync` if and
/// only if `T` is `Sync`.
unsafe impl<T> Sync for AtomicOnceCell<T> where T: Sync {}

/// For the same reason as above, `AtomicOnceCell<T>` is
// `Send` if and only if `T` is `Send`.
unsafe impl<T> Send for AtomicOnceCell<T> where T: Send {}

/// A thread-safe, `Option`-like container
/// for a values that implement `Clone`.
///
/// Provides thread-safe access to `T`
/// by internally using a `SpinLock`.
pub struct Slot<T: Clone> {
    val: SpinLock<Option<T>>,
}

impl<T: Clone> Slot<T> {
    /// Creates a new `Slot` with the given inner value
    ///
    /// # Errors
    /// Returns an error if it fails to create the `SpinLock`
    pub fn try_new(val: Option<T>) -> NtResult<Self> {
        Ok(Self {
            val: SpinLock::create(val)?,
        })
    }

    /// Returns a clone of the inner value if it exists.
    ///
    /// To do it in a thread-safe way, it acquires the
    /// `SpinLock` first, clones the inner value,
    /// releases the lock and then returns the value.
    ///
    /// # Returns
    /// `Some(T)` if the inner value exists, `None` otherwise.
    pub fn get(&self) -> Option<T> {
        self.val.lock().as_ref().cloned()
    }

    /// Sets the inner value to `val`
    pub fn set(&self, val: Option<T>) {
        *self.val.lock() = val;
    }

    pub fn is_some(&self) -> bool {
        self.val.lock().is_some()
    }

    pub fn is_none(&self) -> bool {
        self.val.lock().is_none()
    }
}

/// Thread-safe version of `RefCell`
///
/// Behaves like a reader-writer lock except it never
/// blocks any thread. If a borrow cannot be obtained,
/// `None` is returned instead.
///
/// Uses an `AtomicIsize` borrow counter:
/// - `0` means unborrowed
/// - `> 0` means shared (number of active readers)
/// - `-1` means exclusively borrowed (writer active)
///
/// All operations are lock-free and work at any IRQL.
pub struct AtomicRefCell<T> {
    borrow_state: AtomicIsize,
    data: UnsafeCell<T>,
}

/// `AtomicRefCell` requires `T` to be `Send` because
/// non-`Send` types can lead to situations where a thread
/// NOT holding the borrow can also access the data. An
/// example of this is `Rc` wherein the borrow will protect
/// only one clone of `Rc` and another thread can still
/// access the data through another clone without borrowing.
///
/// `AtomicRefCell` also requires `T` to be `Sync` because
/// `borrow()` hands out shared references `&T` to multiple
/// threads concurrently, which is only valid if `T` supports
/// shared references across threads.
unsafe impl<T> Sync for AtomicRefCell<T> where T: Send + Sync {}

/// `AtomicRefCell<T>` is `Send` if `T` is `Send` because
/// moving the cell to another thread moves the owned `T`.
unsafe impl<T> Send for AtomicRefCell<T> where T: Send {}

impl<T> AtomicRefCell<T> {
    /// Creates a new `AtomicRefCell` with the given value
    pub const fn new(data: T) -> Self {
        Self {
            borrow_state: AtomicIsize::new(0),
            data: UnsafeCell::new(data),
        }
    }

    /// Attempts to immutably borrow the value.
    ///
    /// Returns `Some(AtomicRef)` if no exclusive borrow
    /// is active, or `None` if the value is currently
    /// exclusively borrowed.
    ///
    /// Multiple shared borrows can be held simultaneously.
    pub fn borrow(&self) -> Option<AtomicRef<'_, T>> {
        // `Relaxed` is sufficient for the initial load because this
        // is just a hint for the CAS loop. If the value is stale,
        // compare_exchange_weak will fail and give us the fresh
        // value — no synchronization is needed here.
        let mut cur = self.borrow_state.load(Ordering::Relaxed);

        loop {
            // Exclusively borrowed — cannot obtain shared borrow
            if cur < 0 {
                return None;
            }

            // `Acquire` on success pairs with `Release` in both
            // AtomicRef::drop and AtomicRefMut::drop. This
            // ensures we see all writes a previous writer
            // performed before releasing its exclusive borrow.
            //
            // `Relaxed` on failure because we do not access the
            // data on the failure path. The returned error
            // value is used as the next expected value.
            //
            // compare_exchange_weak (rather than strong) is used
            // because we are already in a retry loop. Weak is
            // allowed to spuriously fail, which lets the compiler
            // emit more efficient LL/SC instructions on
            // architectures that do not have true hardware CAS.
            match self.borrow_state.compare_exchange_weak(
                cur,
                cur + 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(AtomicRef { cell: self }),
                Err(next) => cur = next,
            }
        }
    }

    /// Attempts to mutably borrow the value.
    ///
    /// Returns `Some(AtomicRefMut)` if there are no active
    /// borrows (shared or exclusive), or `None` otherwise.
    ///
    /// Only one exclusive borrow can be held at a time,
    /// and it cannot coexist with shared borrows.
    pub fn borrow_mut(&self) -> Option<AtomicRefMut<'_, T>> {
        // `Acquire` on success pairs with `Release` in both
        // AtomicRef::drop (readers releasing) and
        // AtomicRefMut::drop (previous writer releasing).
        // This ensures we see all data modifications made
        // by any prior borrow holder before we access the
        // UnsafeCell.
        //
        // `Relaxed` on failure because we return None
        // immediately — no data is accessed.
        //
        // Strong compare_exchange (not weak) is used because
        // there is no retry loop. A spurious failure would
        // incorrectly return None to the caller when the
        // borrow was actually available.
        if self
            .borrow_state
            .compare_exchange(0, -1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(AtomicRefMut { cell: self })
        } else {
            None
        }
    }

    /// Bugchecks with the address of this cell and the
    /// invalid borrow state value for debugging.
    fn bug_check_invalid_state(&self, state: isize) -> ! {
        bug_check(
            0xDEADDEAD,
            (self as *const Self).cast_mut().cast::<_>(),
            Some(state as usize),
        )
    }
}

/// RAII guard for a shared borrow from [`AtomicRefCell`].
///
/// The shared borrow is held for the lifetime of this guard
/// and released when it is dropped.
pub struct AtomicRef<'a, T> {
    cell: &'a AtomicRefCell<T>,
}

impl<T> Drop for AtomicRef<'_, T> {
    fn drop(&mut self) {
        // `Release` pairs with `Acquire` in borrow() and
        // borrow_mut(). This ensures that all reads
        // performed through this guard are completed
        // before the borrow count decreases.
        //
        // Every reader needs `Release` (not just the last
        // one going from 1 → 0) because unlike Arc::drop
        // where 1 → 0 means the object is being destroyed
        // and no further access will occur, here a writer
        // could be accessing T next and we don't want any
        // of the readers' accesses to be reordered such
        // that they occur concucurrently with the writer's
        // writes.
        let prev = self.cell.borrow_state.fetch_sub(1, Ordering::Release);

        // prev must be > 0 (at least one active reader).
        // A value <= 0 means the borrow state is corrupted.
        if prev <= 0 {
            self.cell.bug_check_invalid_state(prev);
        }
    }
}

impl<T> Deref for AtomicRef<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        // SAFETY: The borrow state guarantees that no exclusive
        // borrow is active while this guard exists, so reading
        // from the UnsafeCell is safe.
        unsafe { &*self.cell.data.get() }
    }
}

/// RAII guard for an exclusive borrow from [`AtomicRefCell`].
///
/// The exclusive borrow is held for the lifetime of this guard
/// and released when it is dropped.
pub struct AtomicRefMut<'a, T> {
    cell: &'a AtomicRefCell<T>,
}

impl<T> Drop for AtomicRefMut<'_, T> {
    fn drop(&mut self) {
        // `Release` pairs with `Acquire` in borrow() and
        // borrow_mut(). This ensures that all writes
        // performed through this guard via DerefMut are
        // visible to the next thread that successfully
        // acquires a borrow.
        //
        // A swap (not CAS) is used because we hold
        // the exclusive borrow — the state must be -1 and
        // no other thread can change it.
        let prev = self.cell.borrow_state.swap(0, Ordering::Release);

        // prev must be -1 (exclusively borrowed).
        // Any other value means the borrow state is corrupted.
        if prev != -1 {
            self.cell.bug_check_invalid_state(prev);
        }
    }
}

impl<T> Deref for AtomicRefMut<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        // SAFETY: The borrow state guarantees exclusive access
        // while this guard exists.
        unsafe { &*self.cell.data.get() }
    }
}

impl<T> DerefMut for AtomicRefMut<'_, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: The borrow state guarantees exclusive access
        // while this guard exists.
        unsafe { &mut *self.cell.data.get() }
    }
}
