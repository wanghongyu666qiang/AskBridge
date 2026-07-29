use std::{thread, time::Duration};

use askbridge_core::{AppError, Result};
use tracing::{info, warn};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_APP},
};

use crate::util::{last_error, wide};

const MUTEX_NAME: &str = "Local\\AskBridge.Desktop.Singleton.v1";
pub const MAIN_WINDOW_CLASS: &str = "AskBridge.Desktop.HiddenWindow.v1";
pub const MAIN_WINDOW_TITLE: &str = "AskBridge";
pub const ACTIVATE_MESSAGE: u32 = WM_APP + 2;

pub struct SingleInstance {
    handle: HANDLE,
}

impl SingleInstance {
    pub fn acquire() -> Result<Self> {
        let mutex_name = wide(MUTEX_NAME);
        // SAFETY: No security descriptor is supplied and the name buffer remains valid for the call.
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
        if handle.is_null() {
            return Err(AppError::SingleInstance {
                win32_code: last_error(),
            });
        }

        if last_error() == ERROR_ALREADY_EXISTS {
            notify_existing_instance();
            // SAFETY: handle was returned by CreateMutexW in this process.
            unsafe {
                CloseHandle(handle);
            }
            return Err(AppError::AlreadyRunning);
        }

        Ok(Self { handle })
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            // SAFETY: handle was returned by CreateMutexW and is owned by this guard.
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

fn notify_existing_instance() {
    let class = wide(MAIN_WINDOW_CLASS);
    let title = wide(MAIN_WINDOW_TITLE);
    for _ in 0..10 {
        // SAFETY: Both search strings are valid nul-terminated UTF-16 buffers.
        let window = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
        if !window.is_null() {
            // SAFETY: The message carries no pointers and targets the exact AskBridge window.
            let posted = unsafe { PostMessageW(window, ACTIVATE_MESSAGE, 0, 0) };
            if posted == 0 {
                warn!(
                    win32_code = last_error(),
                    "failed to notify existing instance"
                );
            } else {
                info!("existing instance notified");
            }
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    warn!("existing instance window was not found");
}
