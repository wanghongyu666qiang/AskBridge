use std::iter;

use windows_sys::Win32::{
    Foundation::{GetLastError, HWND},
    UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
};

pub fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(iter::once(0)).collect()
}

pub fn last_error() -> u32 {
    // SAFETY: GetLastError has no preconditions.
    unsafe { GetLastError() }
}

pub fn show_error(title: &str, message: &str) {
    let title = wide(title);
    let message = wide(message);
    // SAFETY: Both strings are valid, nul-terminated UTF-16 buffers for the duration of the call.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut::<std::ffi::c_void>() as HWND,
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
