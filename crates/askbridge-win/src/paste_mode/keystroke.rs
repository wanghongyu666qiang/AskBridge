//! Synthesizes the single trusted Ctrl+V keystroke and opens plain HTTPS
//! URLs through the shell.

use std::{thread, time::Duration};

use askbridge_core::{AppError, Result};
use windows_sys::Win32::{
    System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
    UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, keybd_event, VK_CONTROL,
        },
        Shell::ShellExecuteW,
    },
};

use crate::util::wide;

use super::{S_FALSE, S_OK};

/// Virtual key for the letter V (windows-sys does not export letter VKs).
const VK_V: u16 = 0x56;
const PASTE_KEY_INTERVAL: Duration = Duration::from_millis(10);

/// Synthesizes one Ctrl+V keystroke sequence.
pub(crate) fn send_paste() -> Result<()> {
    let inputs = paste_inputs();
    for (index, input) in inputs.iter().enumerate() {
        // Chromium PWAs can acknowledge SendInput while dropping its batched
        // shortcut during renderer startup. keybd_event queues the same trusted
        // system key transitions one by one; the attachment receipt remains the
        // authority for whether the paste actually reached the page.
        let keyboard = unsafe { input.Anonymous.ki };
        unsafe { keybd_event(keyboard.wVk as u8, 0, keyboard.dwFlags, 0) };
        if index + 1 < inputs.len() {
            thread::sleep(PASTE_KEY_INTERVAL);
        }
    }
    Ok(())
}

/// Builds the exact key sequence sent by [`send_paste`]: Ctrl down, V down,
/// V up, Ctrl up.
fn paste_inputs() -> [INPUT; 4] {
    [
        keyboard_input(VK_CONTROL, 0),
        keyboard_input(VK_V, 0),
        keyboard_input(VK_V, KEYEVENTF_KEYUP),
        keyboard_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ]
}

pub(super) fn keyboard_input(virtual_key: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Opens a plain HTTPS URL in the user's default browser.
///
/// ShellExecuteW resolves protocol handlers through COM, so the calling
/// thread must have an apartment initialized; the browser worker thread has
/// none, which made this call fail outright until the explicit init below.
pub(crate) fn open_default_browser(url: &str) -> Result<()> {
    // SAFETY: CoInitializeEx on a worker thread with no apartment yet is the
    // documented use; S_FALSE just means an apartment already existed.
    let com_initialized =
        unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED as u32) };
    if com_initialized != S_OK && com_initialized != S_FALSE {
        return Err(AppError::Windows {
            operation: "CoInitializeEx(clipboard paste)",
            win32_code: com_initialized as u32,
        });
    }
    let result = open_url_via_shell(url);
    // SAFETY: Every successful CoInitializeEx call, including S_FALSE, must
    // be balanced on this thread.
    unsafe {
        CoUninitialize();
    }
    result
}

fn open_url_via_shell(url: &str) -> Result<()> {
    let operation = wide("open");
    let target = wide(url);
    // SAFETY: The strings are live, NUL-terminated UTF-16 buffers for the
    // duration of the call. No parameters or working directory are supplied.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // SW_SHOWNORMAL
        )
    } as isize;
    if result <= 32 {
        // Numeric shell result only: keeps the log within the privacy
        // boundary while pinpointing why protocol launch was rejected.
        tracing::warn!(
            stage = "paste_open_default_browser",
            completed = false,
            shell_result = result,
            "default browser launch was rejected"
        );
        return Err(AppError::DesktopLaunchFailed(result));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_sequence_is_ctrl_v_with_balanced_release() {
        let inputs = paste_inputs();
        assert_eq!(inputs.len(), 4);
        let virtual_keys: Vec<u16> = inputs
            .iter()
            .map(|input| unsafe { input.Anonymous.ki.wVk })
            .collect();
        assert_eq!(virtual_keys, [VK_CONTROL, VK_V, VK_V, VK_CONTROL]);
        assert_eq!(unsafe { inputs[0].Anonymous.ki.dwFlags }, 0);
        assert_eq!(unsafe { inputs[3].Anonymous.ki.dwFlags }, KEYEVENTF_KEYUP);
    }
}
