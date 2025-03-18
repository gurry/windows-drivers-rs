extern crate windows;
use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;
use windows::{
    core::{GUID, PCWSTR},
    Win32::{
        Devices::DeviceAndDriverInstallation::{
            CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CONFIGRET,
        },
        Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE},
        Storage::FileSystem::{
            CreateFileW, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_WRITE, FILE_SHARE_MODE,
            OPEN_EXISTING,
        },
        System::IO::OVERLAPPED,
    },
};
use std::env;

const MAX_DEVPATH_LENGTH: usize = 256;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <interface_guid>", args[0]);
        std::process::exit(1);
    }

    let Some(interface_guid) = parse_guid(&args[1]) else {
        eprintln!("Failed to parse GUID");
        return;
    };

    let mut device_path = [0u16; MAX_DEVPATH_LENGTH];

    if get_device_path(&interface_guid, &mut device_path) {
        let device_path_str = String::from_utf16_lossy(&device_path);
        println!("Device Path: {}", device_path_str);

        // Send a write request to the device
        if send_write_request(&device_path_str, "Hello, Device!") {
            println!("Write request sent successfully.");
        } else {
            eprintln!("Failed to send write request.");
        }
    } else {
        eprintln!("Failed to retrieve device path.");
    }
}

fn get_device_path(interface_guid: &GUID, device_path: &mut [u16]) -> bool {
    let mut device_interface_list_length: u32 = 0;

    // Get the size of the device interface list
    let cr = unsafe {
        CM_Get_Device_Interface_List_SizeW(
            &mut device_interface_list_length,
            interface_guid,
            core::ptr::null(),
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    };

    if cr != CONFIGRET(ERROR_SUCCESS.0) {
        eprintln!("Error retrieving device interface list size: 0x{:x}", cr.0);
        return false;
    }

    if device_interface_list_length <= 1 {
        eprintln!("No active device interfaces found. Is the driver loaded?");
        return false;
    }

    // Allocate memory for the device interface list
    let mut device_interface_list = vec![0u16; device_interface_list_length as usize];
    // unsafe {
    //     core::ptr::write_bytes(device_interface_list.as_mut_ptr(), 0, 1);
    // }

    // unsafe {
    //     ZeroMemory(
    //         device_interface_list.as_mut_ptr() as *mut _,
    //         (device_interface_list_length as usize * std::mem::size_of::<u16>()) as _,
    //     );
    // }

    // Get the device interface list
    let cr = unsafe {
        CM_Get_Device_Interface_ListW(
            interface_guid,
            core::ptr::null(),
            &mut device_interface_list,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    };

    if cr != CONFIGRET(ERROR_SUCCESS.0) {
        eprintln!("Error retrieving device interface list: 0x{:x}", cr.0);
        return false;
    }

    // Copy the first device interface path to the output buffer
    let first_interface = device_interface_list
        .split(|&c| c == 0)
        .next()
        .unwrap_or(&[]);
    if first_interface.is_empty() {
        eprintln!("No valid device interfaces found.");
        return false;
    }

    device_path[..first_interface.len()].copy_from_slice(first_interface);
    true
}

fn send_write_request(device_path: &str, data: &str) -> bool {
    // Convert the device path to a wide string
    let device_path_wide: Vec<u16> = OsString::from(device_path)
        .encode_wide()
        .chain(Some(0))
        .collect();

    // Open the device
    let handle = match unsafe {
        CreateFileW(
            PCWSTR(device_path_wide.as_ptr()),
            FILE_GENERIC_WRITE,
            FILE_SHARE_MODE(0),
            null_mut(),
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    } {
        Ok(handle) => handle,
        Err(e) => {
            eprintln!("Failed to open device: {e}");
            return false;
        }
    };

    if handle == HANDLE::default() {
        eprintln!("Failed to open device.");
        return false;
    }

    // Data to write to the device
    let data = data.as_bytes();
    let mut bytes_written = 0;

    // Send the write request
    let result = unsafe {
        WriteFile(
            handle,
            data.as_ptr() as *const _,
            data.len() as u32,
            &mut bytes_written,
            null_mut() as *mut OVERLAPPED,
        )
    };

    // Close the device handle
    unsafe {
        CloseHandle(handle);
    }

    if result.as_bool() {
        println!("Successfully wrote {} bytes to the device.", bytes_written);
        true
    } else {
        eprintln!("Failed to write to the device.");
        false
    }
}

fn parse_guid(guid_str: &str) -> Option<GUID> {
    let parsed_guid = uuid::Uuid::parse_str(guid_str).ok()?;
    let fields = parsed_guid.as_fields();
    Some(GUID::from_values(
        fields.0 as u32,
        fields.1 as u16,
        fields.2 as u16,
        [
            fields.3[0], fields.3[1], fields.3[2], fields.3[3],
            fields.3[4], fields.3[5], fields.3[6], fields.3[7],
        ],
    ))
}
