extern crate windows;
use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use windows::{
    core::{GUID, PCWSTR, BOOL},
    Win32::{
        // Foundation::*,
        Devices::DeviceAndDriverInstallation::{
            CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CONFIGRET,
        },
        Foundation::{CloseHandle, ERROR_SUCCESS, ERROR_IO_PENDING, ERROR_IO_INCOMPLETE, ERROR_OPERATION_ABORTED, GetLastError, HANDLE},
        Storage::FileSystem::{
            CreateFileW, WriteFile, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_MODE,
            FILE_FLAG_OVERLAPPED, OPEN_EXISTING,
        },
        System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        System::Console::{CTRL_C_EVENT, SetConsoleCtrlHandler},
    },
};
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use anyhow::Context;

static CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);


fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <interface_guid>", args[0]);
        std::process::exit(1);
    }

    let Some(interface_guid) = parse_guid(&args[1]) else {
        eprintln!("Failed to parse GUID");
        return Ok(());
    };

    let device_path = match get_device_path(&interface_guid) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("Error: {}", e);
            return Ok(());
        }
    };

    println!("Device Path: {}", device_path);

    // Set the Ctrl+C handler
    unsafe {
        SetConsoleCtrlHandler(Some(ctrlc_handler), true)
            .context("Failed to set Ctrl+C handler")?;
    }

    // Send a write request to the device
    match send_write_request(&device_path, "Hello, Device!") {
        Ok(()) => {
            println!("Write request completed");
        }
        Err(RequestError::Cancelled) => {
            println!("Write request cancelled");
        },
        Err(RequestError::IoError(e)) => {
            eprintln!("Error sending write request: {}", e);
        }
    }

    Ok(())
}

unsafe extern "system" fn ctrlc_handler(ctrl_type: u32) -> BOOL {
    if ctrl_type == CTRL_C_EVENT {
        CANCEL_REQUESTED.store(true, Ordering::SeqCst);
        return true.into();
    }

    println!("You need to press Ctrl+C to cancel");
    false.into()
}

fn get_device_path(interface_guid: &GUID) -> Result<String, String> {
    let mut device_interface_list_length: u32 = 0;

    // Get the size of the device interface list
    let cr = unsafe {
        CM_Get_Device_Interface_List_SizeW(
            &mut device_interface_list_length,
            interface_guid,
            PCWSTR(core::ptr::null()),
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    };

    if cr != CONFIGRET(ERROR_SUCCESS.0) {
        return Err(format!(
            "Error retrieving device interface list size: 0x{:x}",
            cr.0
        ));
    }

    if device_interface_list_length <= 1 {
        return Err("No active device interfaces found. Is the driver loaded?".to_string());
    }

    // Allocate memory for the device interface list
    let mut device_interface_list = vec![0u16; device_interface_list_length as usize];

    // Get the device interface list
    let cr = unsafe {
        CM_Get_Device_Interface_ListW(
            interface_guid,
            PCWSTR(core::ptr::null()),// as PCWSTR,
            &mut device_interface_list,
            CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        )
    };

    if cr != CONFIGRET(ERROR_SUCCESS.0) {
        return Err(format!(
            "Error retrieving device interface list: 0x{:x}",
            cr.0
        ));
    }

    // Copy the first device interface path to the output buffer
    let first_interface = device_interface_list
        .split(|&c| c == 0)
        .next()
        .unwrap_or(&[]);
    if first_interface.is_empty() {
        return Err("No valid device interfaces found.".to_string());
    }

    let device_path = String::from_utf16_lossy(first_interface);

    Ok(device_path)
}

enum RequestError {
    IoError(String),
    Cancelled,
}

fn send_write_request(device_path: &str, data: &str) -> Result<(), RequestError> {
    send_request(device_path, |handle: HANDLE, overlapped: *mut OVERLAPPED| {
        unsafe {
            WriteFile(
                handle,
                Some(data.as_bytes()),
                None, // Bytes written will be retrieved via GetOverlappedResult
                Some(overlapped),
            )
        }
    })
}

fn send_request<F: Fn(HANDLE, *mut OVERLAPPED) -> windows::core::Result<()>>(device_path: &str, call_win32_api: F) -> Result<(), RequestError> {
    // Convert the device path to a wide string
    let device_path_wide: Vec<u16> = OsString::from(device_path)
        .encode_wide()
        .chain(Some(0))
        .collect();

    // Open the device with FILE_FLAG_OVERLAPPED for asynchronous I/O
    let handle = match unsafe {
        CreateFileW(
            PCWSTR(device_path_wide.as_ptr()),
            (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            None,
        )
    } {
        Ok(handle) => handle,
        Err(e) => {
            return Err(RequestError::IoError(format!("Failed to open device: {e}")));
        }
    };

    if handle == HANDLE::default() {
        return Err(RequestError::IoError("Failed to open device".to_string()));
    }

    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };

    // Call the actual Win32 API to send the request
    let result = call_win32_api(handle, &mut overlapped);

    if result.is_err() {
        let error_code = unsafe { GetLastError() };
        if error_code.0 != ERROR_IO_PENDING.0 {
            unsafe {
                CloseHandle(handle)
                    .map_err(|e| RequestError::IoError(format!("Failed to close handle: {e}")))?
            };
            return Err(RequestError::IoError(format!(
                "Failed to send request. Error code: {}",
                error_code.0
            )));
        }
    }

    println!("Request sent, waiting for completion...");
    // Wait for the asynchronous operation to complete in a loop
    let mut bytes_written = 0;
    let res = loop {
        let overlapped_result = unsafe {
            GetOverlappedResult(
                handle,
                &mut overlapped,
                &mut bytes_written,
                false, // Non-blocking call
            )
        };

        if overlapped_result.is_ok() {
            break Ok(())
        } else {
            let error_code = unsafe { GetLastError() };
            if error_code.0 == ERROR_IO_INCOMPLETE.0  {
                if CANCEL_REQUESTED.load(Ordering::SeqCst) {
                    unsafe {
                        CancelIoEx(handle, Some(&overlapped))
                            .map_err(|e| RequestError::IoError(format!("Failed to cancel I/O: {e}")))?
                    };
                    CANCEL_REQUESTED.store(false, Ordering::SeqCst);
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            } else if error_code.0 == ERROR_OPERATION_ABORTED.0 {
                unsafe {
                    CloseHandle(handle)
                        .map_err(|e| RequestError::IoError(format!("Failed to close handle: {e}")))?
                };
                break Err(RequestError::Cancelled);
            } else {
                break Err(RequestError::IoError(format!(
                    "Failed to send request. Error code: {}",
                    error_code.0
                )));
            }
        };
    };

    unsafe {
        CloseHandle(handle)
            .map_err(|e| RequestError::IoError(format!("Failed to close handle: {e}")))?
    };

    res
}

fn parse_guid(guid_str: &str) -> Option<GUID> {
    let parsed_guid = uuid::Uuid::parse_str(guid_str).ok()?;
    let fields = parsed_guid.as_fields();
    Some(GUID::from_values(
        fields.0 as u32,
        fields.1 as u16,
        fields.2 as u16,
        [
            fields.3[0],
            fields.3[1],
            fields.3[2],
            fields.3[3],
            fields.3[4],
            fields.3[5],
            fields.3[6],
            fields.3[7],
        ],
       ))
}
