use std::{mem::zeroed, ptr, sync::atomic::Ordering};

use askbridge_core::{AppError, Result};
use tracing::{error, info};
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{COLOR_WINDOW, GetSysColorBrush},
    UI::WindowsAndMessaging::{
        CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
        DispatchMessageW, GetMessageW, IDC_ARROW, IsDialogMessageW, LoadCursorW, MSG, PostMessageW,
        PostQuitMessage, RegisterClassW, TranslateMessage, WM_CLOSE, WM_COMMAND, WM_HOTKEY,
        WNDCLASSW, WNDPROC, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
    },
};

use crate::{
    browser::WM_BROWSER_EVENT,
    capture::WM_CAPTURE_BUSY,
    single_instance::{ACTIVATE_MESSAGE, MAIN_WINDOW_CLASS, MAIN_WINDOW_TITLE},
    tray::{TrayEvent, WM_TRAY_CALLBACK, WM_TRAY_DISPATCH, decode_tray_callback},
    util::{last_error, wide},
};

use super::{controller::REQUEST_SEQUENCE, controller::Runtime};

impl Runtime {
    pub(super) fn message_loop(&mut self) -> Result<()> {
        // SAFETY: Zero is the documented initial state for MSG.
        let mut message: MSG = unsafe { zeroed() };
        loop {
            // SAFETY: message points to writable storage and null HWND selects this thread queue.
            let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
            if result == -1 {
                return Err(AppError::Windows {
                    operation: "GetMessageW",
                    win32_code: last_error(),
                });
            }
            if result == 0 {
                break;
            }
            if self.handle_message(&message)? {
                continue;
            }
            if self.settings.is_visible() && self.settings.contains(message.hwnd) {
                // SAFETY: The settings window is live and the message belongs to it or a child.
                if unsafe { IsDialogMessageW(self.settings.hwnd(), &message) } != 0 {
                    continue;
                }
            }
            // SAFETY: message was populated successfully by GetMessageW.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    }

    fn handle_message(&mut self, message: &MSG) -> Result<bool> {
        if message.message == ACTIVATE_MESSAGE {
            info!("activation message received; showing settings");
            self.settings.show();
            return Ok(true);
        }
        match message.message {
            WM_BROWSER_EVENT => {
                self.handle_browser_events();
                Ok(true)
            }
            WM_CAPTURE_BUSY => {
                self.tray
                    .notify("AskBridge 正在框选", "框选期间触发的其他快捷键已忽略。");
                Ok(true)
            }
            WM_HOTKEY => {
                if !self.paused
                    && let Some(command) = self.hotkeys.command_for_id(message.wParam as i32)
                {
                    self.route_command(command);
                }
                Ok(true)
            }
            WM_TRAY_DISPATCH => {
                match decode_tray_callback(message.lParam) {
                    TrayEvent::ContextMenu => {
                        if let Some(command) = self.tray.show_menu(self.paused)? {
                            self.handle_command(command)?;
                        }
                    }
                    TrayEvent::ActivateSettings => self.settings.show(),
                    TrayEvent::Ignore => {}
                }
                Ok(true)
            }
            WM_COMMAND => {
                let command = (message.wParam & 0xffff) as u16;
                self.handle_command(command)?;
                Ok(true)
            }
            WM_CLOSE if message.hwnd == self.settings.hwnd() => {
                self.settings.hide();
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

pub(super) struct MainWindow(HWND);

impl MainWindow {
    pub(super) fn create(instance: HINSTANCE) -> Result<Self> {
        let class = wide(MAIN_WINDOW_CLASS);
        let title = wide(MAIN_WINDOW_TITLE);
        // SAFETY: Class is registered and all pointers remain valid for the call.
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        if window.is_null() {
            return Err(AppError::Windows {
                operation: "CreateWindowExW(main)",
                win32_code: last_error(),
            });
        }
        Ok(Self(window))
    }

    pub(super) const fn hwnd(&self) -> HWND {
        self.0
    }
}

impl Drop for MainWindow {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: This guard owns the hidden top-level window.
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
}

pub(super) fn register_window_class(
    name: &str,
    instance: HINSTANCE,
    window_proc: WNDPROC,
) -> Result<()> {
    let name = wide(name);
    // SAFETY: Loading the shared arrow cursor with a null module handle is supported.
    let cursor = unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) };
    if cursor.is_null() {
        return Err(AppError::Windows {
            operation: "LoadCursorW",
            win32_code: last_error(),
        });
    }
    // SAFETY: GetSysColorBrush returns a shared system brush.
    let background = unsafe { GetSysColorBrush(COLOR_WINDOW) };
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: window_proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: ptr::null_mut(),
        hCursor: cursor,
        hbrBackground: background,
        lpszMenuName: ptr::null(),
        lpszClassName: name.as_ptr(),
    };
    // SAFETY: WNDCLASSW fields remain valid for the synchronous registration call.
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(AppError::Windows {
            operation: "RegisterClassW",
            win32_code: last_error(),
        });
    }
    Ok(())
}

pub(super) unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_TRAY_CALLBACK {
        // Shell notification callbacks may be nonqueued. Relay them so Runtime can process them
        // in order from its ordinary GetMessage loop without storing a raw Runtime pointer.
        // SAFETY: The message carries only integer values and targets this live owner window.
        if unsafe { PostMessageW(window, WM_TRAY_DISPATCH, wparam, lparam) } == 0 {
            error!(
                stage = "tray_callback",
                completed = false,
                "failed to queue tray callback for runtime dispatch"
            );
        }
        return 0;
    }
    if message == WM_CLOSE {
        // SAFETY: This callback runs on the UI thread that owns the message loop.
        unsafe {
            PostQuitMessage(0);
        }
        return 0;
    }
    // SAFETY: Unhandled messages are forwarded exactly as received to DefWindowProcW.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

pub(super) fn next_request_id(created_at_ms: u64) -> String {
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("askbridge-{created_at_ms:013x}-{sequence:08x}")
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use windows_sys::Win32::{
        Foundation::{HINSTANCE, LPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, MSG, PM_REMOVE, PeekMessageW, SendMessageW,
            WM_CONTEXTMENU, WM_QUIT,
        },
    };

    use super::*;

    #[test]
    fn private_window_messages_do_not_overlap() {
        assert_ne!(WM_TRAY_CALLBACK, ACTIVATE_MESSAGE);
        assert_ne!(WM_TRAY_CALLBACK, WM_CAPTURE_BUSY);
        assert_ne!(WM_TRAY_CALLBACK, WM_TRAY_DISPATCH);
        assert_ne!(WM_TRAY_CALLBACK, WM_BROWSER_EVENT);
        assert_ne!(WM_TRAY_DISPATCH, ACTIVATE_MESSAGE);
        assert_ne!(WM_TRAY_DISPATCH, WM_CAPTURE_BUSY);
        assert_ne!(WM_TRAY_DISPATCH, WM_BROWSER_EVENT);
        assert_ne!(ACTIVATE_MESSAGE, WM_CAPTURE_BUSY);
        assert_ne!(ACTIVATE_MESSAGE, WM_BROWSER_EVENT);
        assert_ne!(WM_CAPTURE_BUSY, WM_BROWSER_EVENT);
    }

    #[test]
    fn window_proc_relays_nonqueued_tray_callback() {
        // SAFETY: A null module name requests the current test process module.
        let module = unsafe { GetModuleHandleW(ptr::null()) };
        assert!(!module.is_null());
        let instance = module as HINSTANCE;
        let class_name = "AskBridge.Test.TrayRelayWindow.v1";
        register_window_class(class_name, instance, Some(window_proc))
            .expect("test window class should register");
        let class = wide(class_name);
        let title = wide("AskBridge tray relay test");
        // SAFETY: The test class is registered and all pointers remain valid for the call.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        assert!(!window.is_null());
        let packed_event = ((1_u32 << 16) | WM_CONTEXTMENU) as LPARAM;
        // SAFETY: No pointer-bearing parameters are sent.
        unsafe {
            SendMessageW(window, WM_TRAY_CALLBACK, 23, packed_event);
        }
        // SAFETY: Zero is a valid initial message state.
        let mut queued: MSG = unsafe { zeroed() };
        // SAFETY: The test owns the window and removes only its private dispatch message.
        let found = unsafe {
            PeekMessageW(
                &mut queued,
                window,
                WM_TRAY_DISPATCH,
                WM_TRAY_DISPATCH,
                PM_REMOVE,
            )
        };
        // SAFETY: The test owns this window and no longer needs it.
        unsafe {
            DestroyWindow(window);
        }
        assert_ne!(found, 0);
        assert_eq!(queued.message, WM_TRAY_DISPATCH);
        assert_eq!(queued.wParam, 23);
        assert_eq!(queued.lParam, packed_event);
    }

    #[test]
    fn main_window_close_requests_a_clean_message_loop_exit() {
        // SAFETY: A null module name requests the current test process module.
        let module = unsafe { GetModuleHandleW(ptr::null()) };
        assert!(!module.is_null());
        let instance = module as HINSTANCE;
        let class_name = "AskBridge.Test.MainCloseWindow.v1";
        register_window_class(class_name, instance, Some(window_proc))
            .expect("test window class should register");
        let class = wide(class_name);
        let title = wide("AskBridge close test");
        // SAFETY: The test class is registered and arguments are valid.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        assert!(!window.is_null());
        // SAFETY: The test owns this window and synchronously exercises normal close handling.
        unsafe {
            SendMessageW(window, WM_CLOSE, 0, 0);
        }
        // SAFETY: Zero is a valid initial message state.
        let mut queued: MSG = unsafe { zeroed() };
        let mut found_quit = false;
        // SAFETY: This test thread owns the queue. Drain pending messages until WM_QUIT appears.
        while unsafe { PeekMessageW(&mut queued, ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if queued.message == WM_QUIT {
                found_quit = true;
                break;
            }
        }
        // SAFETY: The test still owns the window because the custom handler did not destroy it.
        unsafe {
            DestroyWindow(window);
        }
        assert!(found_quit);
    }
}
