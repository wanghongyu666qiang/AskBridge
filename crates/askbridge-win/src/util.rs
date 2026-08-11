use std::iter;

use windows_sys::Win32::{
    Foundation::{GetLastError, HWND},
    UI::WindowsAndMessaging::{
        IDYES, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MB_YESNO, MessageBoxW,
    },
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

pub fn confirm_close_managed_browser(owner: HWND) -> bool {
    let title = wide("AskBridge 专用 Chrome 生命周期");
    let message = wide(
        "网页内容已准备完成，但 AskBridge 不会自动发送。请先切换到专用 Chrome 检查并手动发送；完成后选择“是”正常关闭该专用 Chrome，选择“否”保持运行。",
    );
    // SAFETY: Both strings are valid NUL-terminated UTF-16 buffers and owner is the live
    // AskBridge hidden window. The dialog is synchronous and carries no pointers elsewhere.
    unsafe {
        MessageBoxW(
            owner,
            message.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONINFORMATION | MB_DEFBUTTON2,
        ) == IDYES
    }
}
