use core::{default::Default, sync::atomic::AtomicIsize};

use wdf_macros::object_context_with_ref_count_check;
use wdk_sys::{
    BOOLEAN,
    DEVICE_POWER_STATE,
    DEVICE_RELATION_TYPE,
    DEVPROPKEY,
    DEVPROPTYPE,
    NTSTATUS,
    WDF_DEVICE_FAILED_ACTION,
    WDF_DEVICE_IO_TYPE,
    WDF_DEVICE_PNP_CAPABILITIES,
    WDF_DEVICE_POWER_POLICY_IDLE_SETTINGS,
    WDF_DEVICE_POWER_POLICY_WAKE_SETTINGS,
    WDF_DEVICE_PROPERTY_DATA,
    WDF_IO_TYPE_CONFIG,
    WDF_NO_HANDLE,
    WDF_NO_OBJECT_ATTRIBUTES,
    WDF_PNPPOWER_EVENT_CALLBACKS,
    WDF_POWER_DEVICE_STATE,
    WDF_POWER_POLICY_IDLE_TIMEOUT_TYPE,
    WDF_POWER_POLICY_S0_IDLE_CAPABILITIES,
    WDF_POWER_POLICY_S0_IDLE_USER_CONTROL,
    WDF_POWER_POLICY_SX_WAKE_USER_CONTROL,
    WDF_SPECIAL_FILE_TYPE,
    WDFCMRESLIST,
    WDFDEVICE,
    WDFDEVICE_INIT,
    call_unsafe_wdf_function_binding,
};

use super::{
    TriState,
    enum_mapping,
    guid::Guid,
    init_wdf_struct,
    io_queue::IoQueue,
    io_target::IoTarget,
    object::{Handle, impl_ref_counted_handle},
    registry_key::{RegistryAccessRights, RegistryKey},
    request::RequestType,
    resource::CmResList,
    result::{NtResult, NtStatusError, StatusCodeExt, status_codes, to_status_code},
    string::{UnicodeStringBuf, WString},
};

impl_ref_counted_handle!(Device, DeviceContext);

impl Device {
    pub fn create<'a>(
        device_init: &'a mut DeviceInit,
        pnp_power_callbacks: Option<PnpPowerEventCallbacks>,
    ) -> NtResult<&'a mut Self> {
        if let Some(ref pnp_power_callbacks) = pnp_power_callbacks {
            let mut pnp_power_callbacks = pnp_power_callbacks.into();

            unsafe {
                call_unsafe_wdf_function_binding!(
                    WdfDeviceInitSetPnpPowerEventCallbacks,
                    device_init.as_ptr_mut(),
                    &mut pnp_power_callbacks
                );
            }
        }

        let mut device: WDFDEVICE = WDF_NO_HANDLE.cast();
        let mut device_init_ptr: *mut WDFDEVICE_INIT = device_init.as_ptr_mut();

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceCreate,
                &mut device_init_ptr,
                WDF_NO_OBJECT_ATTRIBUTES,
                &mut device,
            )
        }
        .and_then(|| {
            let device: &mut Device = unsafe { &mut *(device.cast()) };

            DeviceContext::attach(
                device,
                DeviceContext {
                    ref_count: AtomicIsize::new(0),
                    pnp_power_callbacks,
                },
            )?;
            Ok(device)
        })
    }

    pub fn create_device_interface(
        &self,
        interaface_class_guid: &Guid,
        reference_string: Option<&UnicodeStringBuf>,
    ) -> NtResult<()> {
        let reference_string_ptr =
            reference_string.map_or(core::ptr::null(), |s| s.as_raw() as *const _);

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceCreateDeviceInterface,
                self.as_ptr().cast(),
                interaface_class_guid.as_lpcguid(),
                reference_string_ptr
            )
        }
        .ok()
    }

    pub fn get_default_queue(&self) -> Option<&IoQueue> {
        let queue = unsafe {
            call_unsafe_wdf_function_binding!(WdfDeviceGetDefaultQueue, self.as_ptr().cast())
        };

        if !queue.is_null() {
            Some(unsafe { &*(queue.cast::<IoQueue>()) })
        } else {
            None
        }
    }

    /// Returns a reference to the default I/O target for the device.
    pub fn get_io_target(&self) -> &IoTarget {
        let io_target = unsafe {
            call_unsafe_wdf_function_binding!(WdfDeviceGetIoTarget, self.as_ptr().cast())
        };

        unsafe { &*(io_target.cast::<IoTarget>()) }
    }

    pub fn configure_request_dispatching(
        &self,
        queue: &IoQueue,
        request_type: RequestType,
    ) -> NtResult<()> {
        // TODO: is this function safe to call from anywhere?
        // Is it thread safe? If not we may have to do some design
        // to make it safe.
        let request_type = request_type.into();

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceConfigureRequestDispatching,
                self.as_ptr().cast(),
                queue.as_ptr().cast(),
                request_type
            )
        }
        .ok()
    }

    pub fn set_pnp_capabilities(&mut self, capabilities: &DevicePnpCapabilities) {
        let mut caps = capabilities.into();
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceSetPnpCapabilities,
                self.as_ptr().cast(),
                &mut caps
            );
        }
    }

    pub fn assign_s0_idle_settings(
        &self,
        settings: &DevicePowerPolicyIdleSettings,
    ) -> NtResult<()> {
        let mut settings = settings.into();

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceAssignS0IdleSettings,
                self.as_ptr().cast(),
                &mut settings
            )
        }
        .ok()
    }

    pub fn assign_sx_wake_settings(
        &self,
        settings: &DevicePowerPolicyWakeSettings,
    ) -> NtResult<()> {
        let mut settings = settings.into();

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceAssignSxWakeSettings,
                self.as_ptr().cast(),
                &mut settings
            )
        }
        .ok()
    }

    pub fn retrieve_device_interface_string(
        &self,
        interface_guid: &Guid,
        reference_string: Option<&UnicodeStringBuf>,
    ) -> NtResult<WString> {
        let wdf_string = WString::create()?;
        let reference_string_ptr =
            reference_string.map_or(core::ptr::null(), |s| s.as_raw() as *const _);

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceRetrieveDeviceInterfaceString,
                self.as_ptr().cast(),
                interface_guid.as_lpcguid(),
                reference_string_ptr,
                wdf_string.as_ptr().cast(),
            )
        }
        .map(|| wdf_string)
    }

    /// Opens a registry key for the device.
    pub fn open_registry_key(
        &self,
        key_type: DeviceInstanceKeyType,
        access: RegistryAccessRights,
    ) -> NtResult<RegistryKey> {
        let mut key: wdk_sys::WDFKEY = core::ptr::null_mut();

        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceOpenRegistryKey,
                self.as_ptr().cast(),
                key_type.into(),
                access.into(),
                WDF_NO_OBJECT_ATTRIBUTES,
                &mut key,
            )
        }
        .map(|| unsafe { RegistryKey::from_raw(key) })
    }

    /// Indicates that the device has encountered a hardware or
    /// software error, allowing the framework to either attempt
    /// a restart or leave the device disabled.
    pub fn set_failed(&self, failed_action: DeviceFailedAction) {
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceSetFailed,
                self.as_ptr().cast(),
                failed_action.into(),
            );
        }
    }

    /// Queries a device property.
    ///
    /// The caller supplies the buffer. On success, returns the
    /// property type. If the buffer is too small, returns
    /// [`QueryPropertyError::BufferTooSmall`] containing the
    /// required size in bytes.
    ///
    /// # Example (two-pass pattern)
    /// ```ignore
    /// let prop_data = DevicePropertyData::new(my_key);
    ///
    /// // First call: get required size.
    /// let required_size = match device.query_property_ex(
    ///     &prop_data, &mut [],
    /// ) {
    ///     Err(QueryPropertyError::BufferTooSmall(size)) => size,
    ///     other => panic!("Some other error"),
    /// };
    ///
    /// // Second call: retrieve the data.
    /// let mut buffer = vec![0u8; required_size as usize];
    /// let property_type = device.query_property_ex(
    ///     &prop_data, &mut buffer,
    /// )?;
    /// ```
    pub fn query_property_ex(
        &self,
        property_data: &DevicePropertyData,
        buffer: &mut [u8],
    ) -> Result<(DevicePropertyType, u32), QueryPropertyError> {
        let raw_key: DEVPROPKEY = property_data.property_key.into();
        let mut raw_property_data = init_wdf_struct!(WDF_DEVICE_PROPERTY_DATA);
        raw_property_data.PropertyKey = &raw_key as *const DEVPROPKEY;
        raw_property_data.Lcid = property_data.lcid;
        raw_property_data.Flags = property_data.flags;

        let buffer_len = buffer.len() as u32;
        let buffer_ptr = buffer.as_mut_ptr().cast();

        let mut required_size: u32 = 0;
        let mut property_type: DEVPROPTYPE = 0;

        let status = unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceQueryPropertyEx,
                self.as_ptr().cast(),
                &mut raw_property_data,
                buffer_len,
                buffer_ptr,
                &mut required_size,
                &mut property_type,
            )
        };

        if status == status_codes::STATUS_BUFFER_TOO_SMALL {
            return Err(QueryPropertyError::BufferTooSmall(required_size));
        }

        if status.is_success() {
            Ok((DevicePropertyType(property_type), required_size))
        } else {
            Err(QueryPropertyError::NtStatus(NtStatusError::from(status)))
        }
    }
}

/// The type of device registry key
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DeviceInstanceKeyType {
    /// The device's hardware key (PLUGPLAY_REGKEY_DEVICE).
    Device,
    /// The device's software key (PLUGPLAY_REGKEY_DRIVER).
    Driver,
    /// The current hardware profile key (PLUGPLAY_REGKEY_CURRENT_HWPROFILE).
    CurrentHwProfile,
}

impl From<DeviceInstanceKeyType> for u32 {
    fn from(value: DeviceInstanceKeyType) -> Self {
        match value {
            DeviceInstanceKeyType::Device => wdk_sys::PLUGPLAY_REGKEY_DEVICE,
            DeviceInstanceKeyType::Driver => wdk_sys::PLUGPLAY_REGKEY_DRIVER,
            DeviceInstanceKeyType::CurrentHwProfile => wdk_sys::PLUGPLAY_REGKEY_CURRENT_HWPROFILE,
        }
    }
}

enum_mapping! {
    infallible;
    pub enum DeviceFailedAction: WDF_DEVICE_FAILED_ACTION {
        AttemptRestart = WdfDeviceFailedAttemptRestart,
        NoRestart = WdfDeviceFailedNoRestart,
    }
}

/// Error type returned by [`Device::query_property_ex`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryPropertyError {
    /// The supplied buffer was too small. Contains the required
    /// size in bytes.
    BufferTooSmall(u32),
    /// An NT status error other than buffer-too-small.
    NtStatus(NtStatusError),
}

impl From<NtStatusError> for QueryPropertyError {
    fn from(e: NtStatusError) -> Self {
        QueryPropertyError::NtStatus(e)
    }
}

/// Safe wrapper around `DEVPROPTYPE`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DevicePropertyType(DEVPROPTYPE);

impl DevicePropertyType {
    pub const BINARY: Self = Self(wdk_sys::DEVPROP_TYPE_BINARY);
    pub const BOOLEAN: Self = Self(wdk_sys::DEVPROP_TYPE_BOOLEAN);
    pub const BYTE: Self = Self(wdk_sys::DEVPROP_TYPE_BYTE);
    pub const CURRENCY: Self = Self(wdk_sys::DEVPROP_TYPE_CURRENCY);
    pub const DATE: Self = Self(wdk_sys::DEVPROP_TYPE_DATE);
    pub const DECIMAL: Self = Self(wdk_sys::DEVPROP_TYPE_DECIMAL);
    pub const DEVPROPKEY: Self = Self(wdk_sys::DEVPROP_TYPE_DEVPROPKEY);
    pub const DEVPROPTYPE: Self = Self(wdk_sys::DEVPROP_TYPE_DEVPROPTYPE);
    pub const DOUBLE: Self = Self(wdk_sys::DEVPROP_TYPE_DOUBLE);
    pub const EMPTY: Self = Self(wdk_sys::DEVPROP_TYPE_EMPTY);
    pub const ERROR: Self = Self(wdk_sys::DEVPROP_TYPE_ERROR);
    pub const FILETIME: Self = Self(wdk_sys::DEVPROP_TYPE_FILETIME);
    pub const FLOAT: Self = Self(wdk_sys::DEVPROP_TYPE_FLOAT);
    pub const GUID: Self = Self(wdk_sys::DEVPROP_TYPE_GUID);
    pub const INT16: Self = Self(wdk_sys::DEVPROP_TYPE_INT16);
    pub const INT32: Self = Self(wdk_sys::DEVPROP_TYPE_INT32);
    pub const INT64: Self = Self(wdk_sys::DEVPROP_TYPE_INT64);
    pub const NTSTATUS: Self = Self(wdk_sys::DEVPROP_TYPE_NTSTATUS);
    pub const NULL: Self = Self(wdk_sys::DEVPROP_TYPE_NULL);
    pub const SBYTE: Self = Self(wdk_sys::DEVPROP_TYPE_SBYTE);
    pub const SECURITY_DESCRIPTOR: Self = Self(wdk_sys::DEVPROP_TYPE_SECURITY_DESCRIPTOR);
    pub const SECURITY_DESCRIPTOR_STRING: Self =
        Self(wdk_sys::DEVPROP_TYPE_SECURITY_DESCRIPTOR_STRING);
    pub const STRING: Self = Self(wdk_sys::DEVPROP_TYPE_STRING);
    pub const STRING_INDIRECT: Self = Self(wdk_sys::DEVPROP_TYPE_STRING_INDIRECT);
    pub const STRING_LIST: Self = Self(wdk_sys::DEVPROP_TYPE_STRING_LIST);
    pub const UINT16: Self = Self(wdk_sys::DEVPROP_TYPE_UINT16);
    pub const UINT32: Self = Self(wdk_sys::DEVPROP_TYPE_UINT32);
    pub const UINT64: Self = Self(wdk_sys::DEVPROP_TYPE_UINT64);

    /// Returns the raw `DEVPROPTYPE` value.
    pub fn raw(&self) -> DEVPROPTYPE {
        self.0
    }

    /// Returns the base type, stripping any array/list modifier.
    pub fn base_type(&self) -> Self {
        Self(self.0 & wdk_sys::DEVPROP_MASK_TYPE)
    }

    /// Returns `true` if this is an array type.
    pub fn is_array(&self) -> bool {
        self.0 & wdk_sys::DEVPROP_TYPEMOD_ARRAY != 0
    }

    /// Returns `true` if this is a list type.
    pub fn is_list(&self) -> bool {
        self.0 & wdk_sys::DEVPROP_TYPEMOD_LIST != 0
    }
}

/// Safe wrapper around `DEVPROPKEY`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct DevicePropertyKey {
    /// The property category GUID.
    pub fmtid: Guid,
    /// The property identifier within the category.
    pub pid: u32,
}

impl DevicePropertyKey {
    /// Creates a new `DevicePropertyKey` from a GUID and property
    /// identifier.
    pub const fn new(fmtid: Guid, pid: u32) -> Self {
        Self { fmtid, pid }
    }
}

impl From<DevicePropertyKey> for DEVPROPKEY {
    fn from(key: DevicePropertyKey) -> Self {
        DEVPROPKEY {
            fmtid: key.fmtid.to_raw(),
            pid: key.pid,
        }
    }
}

/// Device property data used as a parameter for querying
/// device properties (e.g. with [`Device::query_property_ex`]).
pub struct DevicePropertyData {
    /// The property key to query.
    pub property_key: DevicePropertyKey,
    /// The locale identifier. Use `0` for default.
    pub lcid: u32,
    /// Flags. Use `0` for default.
    pub flags: u32,
}

impl DevicePropertyData {
    /// Creates a new `DevicePropertyData` from a property key
    /// with default locale and flags.
    pub fn new(property_key: DevicePropertyKey) -> Self {
        Self {
            property_key,
            lcid: 0,
            flags: 0,
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

    pub fn set_io_type(&mut self, io_type_config: &IoTypeConfig) {
        let mut io_type_config = io_type_config.into();
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceInitSetIoTypeEx,
                self.as_ptr_mut(),
                &mut io_type_config
            );
        }
    }

    pub fn set_power_policy_ownership(&mut self, is_power_policy_owner: bool) {
        unsafe {
            call_unsafe_wdf_function_binding!(
                WdfDeviceInitSetPowerPolicyOwnership,
                self.as_ptr_mut(),
                is_power_policy_owner as BOOLEAN
            );
        }
    }
}

pub struct IoTypeConfig {
    pub read_write_io_type: DeviceIoType,
    pub device_control_io_type: DeviceIoType,
    pub direct_transfer_threshold: u32,
}

impl From<&IoTypeConfig> for WDF_IO_TYPE_CONFIG {
    fn from(config: &IoTypeConfig) -> Self {
        let mut raw_config = init_wdf_struct!(WDF_IO_TYPE_CONFIG);
        raw_config.ReadWriteIoType = config.read_write_io_type.into();
        raw_config.DeviceControlIoType = config.device_control_io_type.into();
        raw_config.DirectTransferThreshold = config.direct_transfer_threshold;

        raw_config
    }
}

impl Default for IoTypeConfig {
    fn default() -> Self {
        Self {
            read_write_io_type: DeviceIoType::Buffered,
            device_control_io_type: DeviceIoType::Buffered,
            direct_transfer_threshold: 0,
        }
    }
}

enum_mapping! {
    pub enum DeviceIoType: WDF_DEVICE_IO_TYPE {
        Neither = WdfDeviceIoNeither,
        Buffered = WdfDeviceIoBuffered,
        Direct = WdfDeviceIoDirect,
        BufferedOrDirect = WdfDeviceIoBufferedOrDirect,
    }
}

pub struct DevicePnpCapabilities {
    pub lock_supported: TriState,
    pub eject_supported: TriState,
    pub removable: TriState,
    pub dock_device: TriState,
    pub unique_id: TriState,
    pub silent_install: TriState,
    pub surprise_removal_ok: TriState,
    pub hardware_disabled: TriState,
    pub no_display_in_ui: TriState,
    pub address: u32,
    pub ui_number: u32,
}

impl From<&DevicePnpCapabilities> for WDF_DEVICE_PNP_CAPABILITIES {
    fn from(caps: &DevicePnpCapabilities) -> Self {
        let mut raw_caps = init_wdf_struct!(WDF_DEVICE_PNP_CAPABILITIES);
        raw_caps.LockSupported = caps.lock_supported.into();
        raw_caps.EjectSupported = caps.eject_supported.into();
        raw_caps.Removable = caps.removable.into();
        raw_caps.DockDevice = caps.dock_device.into();
        raw_caps.UniqueID = caps.unique_id.into();
        raw_caps.SilentInstall = caps.silent_install.into();
        raw_caps.SurpriseRemovalOK = caps.surprise_removal_ok.into();
        raw_caps.HardwareDisabled = caps.hardware_disabled.into();
        raw_caps.NoDisplayInUI = caps.no_display_in_ui.into();
        raw_caps.Address = caps.address;
        raw_caps.UINumber = caps.ui_number;

        raw_caps
    }
}

impl Default for DevicePnpCapabilities {
    fn default() -> Self {
        Self {
            lock_supported: TriState::default(),
            eject_supported: TriState::default(),
            removable: TriState::default(),
            dock_device: TriState::default(),
            unique_id: TriState::default(),
            silent_install: TriState::default(),
            surprise_removal_ok: TriState::default(),
            hardware_disabled: TriState::default(),
            no_display_in_ui: TriState::default(),
            address: -1_i32 as u32,
            ui_number: -1_i32 as u32,
        }
    }
}

#[object_context_with_ref_count_check(Device)]
struct DeviceContext {
    ref_count: AtomicIsize,
    pnp_power_callbacks: Option<PnpPowerEventCallbacks>,
}

pub struct PnpPowerEventCallbacks {
    pub evt_device_d0_entry: Option<fn(&Device, PowerDeviceState) -> NtResult<()>>,
    pub evt_device_d0_entry_post_interrupts_enabled:
        Option<fn(&Device, PowerDeviceState) -> NtResult<()>>,
    pub evt_device_d0_exit: Option<fn(&Device, PowerDeviceState) -> NtResult<()>>,
    pub evt_device_d0_exit_pre_interrupts_disabled:
        Option<fn(&Device, PowerDeviceState) -> NtResult<()>>,
    pub evt_device_prepare_hardware: Option<fn(&Device, &CmResList, &CmResList) -> NtResult<()>>,
    pub evt_device_release_hardware: Option<fn(&Device, &CmResList) -> NtResult<()>>,
    pub evt_device_self_managed_io_cleanup: Option<fn(&Device)>,
    pub evt_device_self_managed_io_flush: Option<fn(&Device)>,
    pub evt_device_self_managed_io_init: Option<fn(&Device) -> NtResult<()>>,
    pub evt_device_self_managed_io_suspend: Option<fn(&Device) -> NtResult<()>>,
    pub evt_device_self_managed_io_restart: Option<fn(&Device) -> NtResult<()>>,
    pub evt_device_surprise_removal: Option<fn(&Device)>,
    pub evt_device_query_remove: Option<fn(&Device) -> NtResult<()>>,
    pub evt_device_query_stop: Option<fn(&Device) -> NtResult<()>>,
    pub evt_device_usage_notification: Option<fn(&Device, SpecialFileType, bool)>,
    pub evt_device_relations_query: Option<fn(&Device, DeviceRelationType)>,
    pub evt_device_usage_notification_ex:
        Option<fn(&Device, SpecialFileType, bool) -> NtResult<()>>,
}

impl Default for PnpPowerEventCallbacks {
    fn default() -> Self {
        Self {
            evt_device_d0_entry: None,
            evt_device_d0_entry_post_interrupts_enabled: None,
            evt_device_d0_exit: None,
            evt_device_d0_exit_pre_interrupts_disabled: None,
            evt_device_prepare_hardware: None,
            evt_device_release_hardware: None,
            evt_device_self_managed_io_cleanup: None,
            evt_device_self_managed_io_flush: None,
            evt_device_self_managed_io_init: None,
            evt_device_self_managed_io_suspend: None,
            evt_device_self_managed_io_restart: None,
            evt_device_surprise_removal: None,
            evt_device_query_remove: None,
            evt_device_query_stop: None,
            evt_device_usage_notification: None,
            evt_device_relations_query: None,
            evt_device_usage_notification_ex: None,
        }
    }
}

enum_mapping! {
    pub enum PowerDeviceState: WDF_POWER_DEVICE_STATE {
        Invalid = WdfPowerDeviceInvalid,
        D0 = WdfPowerDeviceD0,
        D1 = WdfPowerDeviceD1,
        D2 = WdfPowerDeviceD2,
        D3 = WdfPowerDeviceD3,
        D3Final = WdfPowerDeviceD3Final,
        PrepareForHibernation = WdfPowerDevicePrepareForHibernation,
    }
}

enum_mapping! {
    pub enum SpecialFileType: WDF_SPECIAL_FILE_TYPE {
        Paging = WdfSpecialFilePaging,
        Hibernation = WdfSpecialFileHibernation,
        Dump = WdfSpecialFileDump,
        Boot = WdfSpecialFileBoot,
        PostDisplay = WdfSpecialFilePostDisplay,
        GuestAssigned = WdfSpecialFileGuestAssigned,
        // InlineCryptoEngine = WdfSpecialFileInlineCryptoEngine,
    }
}

enum_mapping! {
    pub enum DeviceRelationType: DEVICE_RELATION_TYPE {
        Bus = BusRelations,
        Ejection = EjectionRelations,
        Power = PowerRelations,
        Removal = RemovalRelations,
        TargetDevice = TargetDeviceRelation,
        SingleBus = SingleBusRelations,
        Transport = TransportRelations,
    }
}

impl From<&PnpPowerEventCallbacks> for WDF_PNPPOWER_EVENT_CALLBACKS {
    fn from(callbacks: &PnpPowerEventCallbacks) -> Self {
        let mut raw_callbacks = init_wdf_struct!(WDF_PNPPOWER_EVENT_CALLBACKS);

        if callbacks.evt_device_d0_entry.is_some() {
            raw_callbacks.EvtDeviceD0Entry = Some(__evt_device_d0_entry);
        }

        if callbacks
            .evt_device_d0_entry_post_interrupts_enabled
            .is_some()
        {
            raw_callbacks.EvtDeviceD0EntryPostInterruptsEnabled =
                Some(__evt_device_d0_entry_post_interrupts_enabled);
        }

        if callbacks.evt_device_d0_exit.is_some() {
            raw_callbacks.EvtDeviceD0Exit = Some(__evt_device_d0_exit);
        }

        if callbacks
            .evt_device_d0_exit_pre_interrupts_disabled
            .is_some()
        {
            raw_callbacks.EvtDeviceD0ExitPreInterruptsDisabled =
                Some(__evt_device_d0_exit_pre_interrupts_disabled);
        }

        if callbacks.evt_device_prepare_hardware.is_some() {
            raw_callbacks.EvtDevicePrepareHardware = Some(__evt_device_prepare_hardware);
        }

        if callbacks.evt_device_release_hardware.is_some() {
            raw_callbacks.EvtDeviceReleaseHardware = Some(__evt_device_release_hardware);
        }

        if callbacks.evt_device_self_managed_io_cleanup.is_some() {
            raw_callbacks.EvtDeviceSelfManagedIoCleanup =
                Some(__evt_device_self_managed_io_cleanup);
        }

        if callbacks.evt_device_self_managed_io_flush.is_some() {
            raw_callbacks.EvtDeviceSelfManagedIoFlush = Some(__evt_device_self_managed_io_flush);
        }

        if callbacks.evt_device_self_managed_io_init.is_some() {
            raw_callbacks.EvtDeviceSelfManagedIoInit = Some(__evt_device_self_managed_io_init);
        }

        if callbacks.evt_device_self_managed_io_suspend.is_some() {
            raw_callbacks.EvtDeviceSelfManagedIoSuspend =
                Some(__evt_device_self_managed_io_suspend);
        }

        if callbacks.evt_device_self_managed_io_restart.is_some() {
            raw_callbacks.EvtDeviceSelfManagedIoRestart =
                Some(__evt_device_self_managed_io_restart);
        }

        if callbacks.evt_device_surprise_removal.is_some() {
            raw_callbacks.EvtDeviceSurpriseRemoval = Some(__evt_device_surprise_removal);
        }

        if callbacks.evt_device_query_remove.is_some() {
            raw_callbacks.EvtDeviceQueryRemove = Some(__evt_device_query_remove);
        }

        if callbacks.evt_device_query_stop.is_some() {
            raw_callbacks.EvtDeviceQueryStop = Some(__evt_device_query_stop);
        }

        if callbacks.evt_device_usage_notification.is_some() {
            raw_callbacks.EvtDeviceUsageNotification = Some(__evt_device_usage_notification);
        }

        if callbacks.evt_device_relations_query.is_some() {
            raw_callbacks.EvtDeviceRelationsQuery = Some(__evt_device_relations_query);
        }

        if callbacks.evt_device_usage_notification_ex.is_some() {
            raw_callbacks.EvtDeviceUsageNotificationEx = Some(__evt_device_usage_notification_ex);
        }

        raw_callbacks
    }
}

macro_rules! unsafe_pnp_power_callback {
    // Both public arms forward to the same internal implementation (@impl) to deduplicate
    (mut $callback_name:ident($($param_name:ident: $param_type:ty => $conversion:expr),*) $(-> $return_type:tt)?) => {
        unsafe_pnp_power_callback!(@impl $callback_name, ($($param_name: $param_type => $conversion),*), ($($return_type)?));
    };

    ($callback_name:ident($($param_name:ident: $param_type:ty => $conversion:expr),*) $(-> $return_type:tt)?) => {
        unsafe_pnp_power_callback!(@impl $callback_name, ($($param_name: $param_type => $conversion),*), ($($return_type)?));
    };

    // internal implementation
    (@impl $callback_name:ident, ($($param_name:ident: $param_type:ty => $conversion:expr),*), ($($return_type:tt)?)) => {
        paste::paste! {
            pub extern "C" fn [<__ $callback_name>](device: WDFDEVICE $(, $param_name: $param_type)*) -> unsafe_pnp_power_callback!(@ret_type $($return_type)*) {
                let (device, ctxt) = get_device_and_ctxt(device);

                if let Some(callbacks) = &ctxt.pnp_power_callbacks {
                    if let Some(callback) = callbacks.$callback_name {
                        return unsafe_pnp_power_callback_call_and_return!($($return_type)*, callback(device $(, $conversion)*));
                    }
                }

                panic!("User did not provide callback {} but we subscribed to it", stringify!($callback_name));
            }
        }
    };

    // Return type helpers
    (@ret_type) => { () };
    (@ret_type $return_type:tt) => { $return_type };
}

// Helper macro: convert Result-returning Rust callbacks into the C return
// value.
macro_rules! unsafe_pnp_power_callback_call_and_return {
    (NTSTATUS, $call:expr) => {
        match $call {
            Ok(_) => 0,
            Err(err) => err.code(),
        }
    };

    (, $call:expr) => {
        $call
    };

    ((), $call:expr) => {
        $call
    };
}

unsafe_pnp_power_callback!(evt_device_d0_entry_post_interrupts_enabled(previous_state: WDF_POWER_DEVICE_STATE => to_rust_power_state_enum(previous_state)) -> NTSTATUS);
unsafe_pnp_power_callback!(evt_device_d0_exit_pre_interrupts_disabled(target_state: WDF_POWER_DEVICE_STATE => to_rust_power_state_enum(target_state)) -> NTSTATUS);
unsafe_pnp_power_callback!(evt_device_self_managed_io_cleanup());
unsafe_pnp_power_callback!(evt_device_self_managed_io_flush());
unsafe_pnp_power_callback!(evt_device_self_managed_io_init() -> NTSTATUS);
unsafe_pnp_power_callback!(evt_device_self_managed_io_suspend() -> NTSTATUS);
unsafe_pnp_power_callback!(evt_device_self_managed_io_restart() -> NTSTATUS);
unsafe_pnp_power_callback!(evt_device_surprise_removal());
unsafe_pnp_power_callback!(evt_device_query_remove() -> NTSTATUS);
unsafe_pnp_power_callback!(evt_device_query_stop() -> NTSTATUS);
unsafe_pnp_power_callback!(evt_device_usage_notification(notification_type: WDF_SPECIAL_FILE_TYPE => to_rust_special_file_type_enum(notification_type), is_in_notification_path: BOOLEAN => is_in_notification_path == 1));
unsafe_pnp_power_callback!(evt_device_relations_query(relation_type: DEVICE_RELATION_TYPE => to_rust_device_relation_type_enum(relation_type)));
unsafe_pnp_power_callback!(evt_device_usage_notification_ex(notification_type: WDF_SPECIAL_FILE_TYPE => to_rust_special_file_type_enum(notification_type), is_in_notification_path: BOOLEAN => is_in_notification_path == 1) -> NTSTATUS);

pub extern "C" fn __evt_device_d0_entry(
    device: WDFDEVICE,
    previous_state: WDF_POWER_DEVICE_STATE,
) -> NTSTATUS {
    let (device, ctxt) = get_device_and_ctxt(device);

    if let Some(callbacks) = &ctxt.pnp_power_callbacks {
        if let Some(callback) = callbacks.evt_device_d0_entry {
            let previous_state = to_rust_power_state_enum(previous_state);

            return to_status_code(&callback(device, previous_state));
        }
    }

    panic!(
        "User did not provide callback {} but we subscribed to it",
        stringify!(evt_device_d0_entry)
    );
}

pub extern "C" fn __evt_device_d0_exit(
    device: WDFDEVICE,
    target_state: WDF_POWER_DEVICE_STATE,
) -> NTSTATUS {
    let (device, ctxt) = get_device_and_ctxt(device);

    let mut user_callback_result = None;

    if let Some(callbacks) = &ctxt.pnp_power_callbacks {
        if let Some(callback) = callbacks.evt_device_d0_exit {
            let target_state = to_rust_power_state_enum(target_state);
            user_callback_result = Some(callback(device, target_state));
        }
    }

    if let Some(res) = user_callback_result {
        to_status_code(&res)
    } else {
        panic!(
            "User did not provide callback {} but we subscribed to it",
            stringify!(evt_device_d0_exit)
        );
    }
}

pub extern "C" fn __evt_device_prepare_hardware(
    device: WDFDEVICE,
    resources_raw: WDFCMRESLIST,
    resources_translated: WDFCMRESLIST,
) -> NTSTATUS {
    let device = unsafe { &mut *(device.cast()) };
    let ctxt = DeviceContext::get(device);

    if let Some(callbacks) = &ctxt.pnp_power_callbacks {
        if let Some(callback) = callbacks.evt_device_prepare_hardware {
            let resources = unsafe { &*(resources_raw.cast::<CmResList>()) };
            let resources_translated = unsafe { &*(resources_translated.cast::<CmResList>()) };

            return to_status_code(&callback(device, resources, resources_translated));
        }
    }

    panic!(
        "User did not provide callback {} but we subscribed to it",
        stringify!(evt_device_release_hardware)
    );
}

pub extern "C" fn __evt_device_release_hardware(
    device: WDFDEVICE,
    resources_translated: WDFCMRESLIST,
) -> NTSTATUS {
    let device = unsafe { &mut *(device.cast()) };
    let ctxt = DeviceContext::get(device);

    if let Some(callbacks) = &ctxt.pnp_power_callbacks {
        if let Some(callback) = callbacks.evt_device_release_hardware {
            let resources_translated = unsafe { &*(resources_translated.cast::<CmResList>()) };

            return to_status_code(&callback(device, resources_translated));
        }
    }

    panic!(
        "User did not provide callback {} but we subscribed to it",
        stringify!(evt_device_prepare_hardware)
    );
}

fn to_rust_power_state_enum(state: WDF_POWER_DEVICE_STATE) -> PowerDeviceState {
    PowerDeviceState::try_from(state)
        .expect("framework should not send invalid WDF_POWER_DEVICE_STATE")
}

fn to_rust_special_file_type_enum(file_type: WDF_SPECIAL_FILE_TYPE) -> SpecialFileType {
    SpecialFileType::try_from(file_type)
        .expect("framework should not send invalid WDF_SPECIAL_FILE_TYPE")
}

fn to_rust_device_relation_type_enum(relation_type: DEVICE_RELATION_TYPE) -> DeviceRelationType {
    DeviceRelationType::try_from(relation_type)
        .expect("framework should not send invalid DEVICE_RELATION_TYPE")
}

#[inline]
fn get_device_and_ctxt<'a>(device: WDFDEVICE) -> (&'a Device, &'a DeviceContext) {
    let device = unsafe { &*(device.cast()) };
    let ctxt = DeviceContext::get(device);
    (device, ctxt)
}

pub struct DevicePowerPolicyIdleSettings {
    pub idle_caps: PowerPolicyS0IdleCapabilities,
    pub dx_state: DevicePowerState,
    pub idle_timeout: u32,
    pub user_control_of_idle_settings: PowerPolicyS0IdleUserControl,
    pub enabled: TriState,
    pub power_up_idle_device_on_system_wake: TriState,
    pub idle_timeout_type: PowerPolicyIdleTimeoutType,
    pub exclude_d3_cold: TriState,
}

impl DevicePowerPolicyIdleSettings {
    pub fn from_caps(caps: PowerPolicyS0IdleCapabilities) -> Self {
        let mut obj = Self::default();
        obj.idle_caps = caps;

        obj.dx_state = match caps {
            PowerPolicyS0IdleCapabilities::CanWakeFromS0
            | PowerPolicyS0IdleCapabilities::UsbSelectiveSuspend => DevicePowerState::Maximum,
            PowerPolicyS0IdleCapabilities::CannotWakeFromS0 => DevicePowerState::D3,
        };
        obj
    }
}

impl From<&DevicePowerPolicyIdleSettings> for WDF_DEVICE_POWER_POLICY_IDLE_SETTINGS {
    fn from(settings: &DevicePowerPolicyIdleSettings) -> Self {
        let mut raw_settings = init_wdf_struct!(WDF_DEVICE_POWER_POLICY_IDLE_SETTINGS);
        raw_settings.IdleCaps = settings.idle_caps.into();
        raw_settings.DxState = settings.dx_state.into();
        raw_settings.IdleTimeout = settings.idle_timeout;
        raw_settings.UserControlOfIdleSettings = settings.user_control_of_idle_settings.into();
        raw_settings.Enabled = settings.enabled.into();
        raw_settings.PowerUpIdleDeviceOnSystemWake =
            settings.power_up_idle_device_on_system_wake.into();
        raw_settings.IdleTimeoutType = settings.idle_timeout_type.into();
        raw_settings.ExcludeD3Cold = settings.exclude_d3_cold.into();

        raw_settings
    }
}

impl Default for DevicePowerPolicyIdleSettings {
    fn default() -> Self {
        Self {
            idle_caps: PowerPolicyS0IdleCapabilities::CannotWakeFromS0,
            dx_state: DevicePowerState::Maximum,
            idle_timeout: 0,
            user_control_of_idle_settings: PowerPolicyS0IdleUserControl::AllowUserControl,
            enabled: TriState::default(),
            power_up_idle_device_on_system_wake: TriState::default(),
            idle_timeout_type: PowerPolicyIdleTimeoutType::DriverManagedIdleTimeout,
            exclude_d3_cold: TriState::default(),
        }
    }
}

enum_mapping! {
    pub enum PowerPolicyS0IdleCapabilities: WDF_POWER_POLICY_S0_IDLE_CAPABILITIES {
        CannotWakeFromS0 = IdleCannotWakeFromS0,
        CanWakeFromS0 = IdleCanWakeFromS0,
        UsbSelectiveSuspend = IdleUsbSelectiveSuspend
    }
}

enum_mapping! {
    pub enum DevicePowerState: DEVICE_POWER_STATE {
        Unspecified = PowerDeviceUnspecified,
        D0 = PowerDeviceD0,
        D1 = PowerDeviceD1,
        D2 = PowerDeviceD2,
        D3 = PowerDeviceD3,
        Maximum = PowerDeviceMaximum
    }
}

enum_mapping! {
    pub enum PowerPolicyS0IdleUserControl: WDF_POWER_POLICY_S0_IDLE_USER_CONTROL {
        Invalid = IdleUserControlInvalid,
        DoNotAllowUserControl = IdleDoNotAllowUserControl,
        AllowUserControl = IdleAllowUserControl
    }
}

enum_mapping! {
    pub enum PowerPolicyIdleTimeoutType: WDF_POWER_POLICY_IDLE_TIMEOUT_TYPE {
        DriverManagedIdleTimeout = DriverManagedIdleTimeout,
        SystemManagedIdleTimeout = SystemManagedIdleTimeout,
        SystemManagedIdleTimeoutWithHint = SystemManagedIdleTimeoutWithHint
    }
}

pub struct DevicePowerPolicyWakeSettings {
    pub dx_state: DevicePowerState,
    pub user_control_of_wake_settings: PowerPolicySxWakeUserControl,
    pub enabled: TriState,
    pub arm_for_wake_if_children_are_armed_for_wake: bool,
    pub indicate_child_wake_on_parent_wake: bool,
}

impl From<&DevicePowerPolicyWakeSettings> for WDF_DEVICE_POWER_POLICY_WAKE_SETTINGS {
    fn from(settings: &DevicePowerPolicyWakeSettings) -> Self {
        let mut raw_settings = init_wdf_struct!(WDF_DEVICE_POWER_POLICY_WAKE_SETTINGS);
        raw_settings.DxState = settings.dx_state.into();
        raw_settings.UserControlOfWakeSettings = settings.user_control_of_wake_settings.into();
        raw_settings.Enabled = settings.enabled.into();
        raw_settings.ArmForWakeIfChildrenAreArmedForWake =
            settings.arm_for_wake_if_children_are_armed_for_wake.into();
        raw_settings.IndicateChildWakeOnParentWake =
            settings.indicate_child_wake_on_parent_wake.into();

        raw_settings
    }
}

impl Default for DevicePowerPolicyWakeSettings {
    fn default() -> Self {
        Self {
            dx_state: DevicePowerState::Maximum,
            user_control_of_wake_settings: PowerPolicySxWakeUserControl::AllowUserControl,
            enabled: TriState::default(),
            arm_for_wake_if_children_are_armed_for_wake: false,
            indicate_child_wake_on_parent_wake: false,
        }
    }
}

enum_mapping! {
    pub enum PowerPolicySxWakeUserControl: WDF_POWER_POLICY_SX_WAKE_USER_CONTROL {
        DoNotAllowUserControl = WakeDoNotAllowUserControl,
        AllowUserControl = WakeAllowUserControl
    }
}
