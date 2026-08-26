use std::mem::{size_of, zeroed};

use askbridge_core::{AppError, Result};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, POINT},
    UI::{
        Shell::{
            NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
            NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
        },
        WindowsAndMessaging::{
            AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HICON, MF_CHECKED, MF_GRAYED,
            MF_SEPARATOR, MF_STRING, MF_UNCHECKED, SetForegroundWindow, TPM_BOTTOMALIGN,
            TPM_LEFTALIGN, TPM_RETURNCMD, TrackPopupMenu, WM_APP, WM_CONTEXTMENU, WM_LBUTTONDBLCLK,
            WM_RBUTTONUP,
        },
    },
};

use crate::util::{last_error, wide};

pub const WM_TRAY_CALLBACK: u32 = WM_APP + 1;
pub const WM_TRAY_DISPATCH: u32 = WM_APP + 4;
pub const MENU_CAPTURE_WITH_PROMPT: u16 = 1001;
pub const MENU_CAPTURE_QUICK: u16 = 1002;
pub const MENU_TEXT_ONLY: u16 = 1003;
pub const MENU_PAUSE: u16 = 1004;
pub const MENU_SETTINGS: u16 = 1005;
pub const MENU_EXIT: u16 = 1006;
pub const MENU_CHECK_UPDATES: u16 = 1007;
pub const MENU_INSTALL_UPDATE: u16 = 1008;

const TRAY_ICON_ID: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayEvent {
    ContextMenu,
    ActivateSettings,
    Ignore,
}

pub const fn decode_tray_callback(lparam: LPARAM) -> TrayEvent {
    match (lparam as u32) & 0xffff {
        WM_RBUTTONUP | WM_CONTEXTMENU => TrayEvent::ContextMenu,
        WM_LBUTTONDBLCLK => TrayEvent::ActivateSettings,
        _ => TrayEvent::Ignore,
    }
}

pub struct TrayIcon {
    window: HWND,
    _icon: HICON,
    active: bool,
}

impl TrayIcon {
    pub fn create(window: HWND) -> Result<Self> {
        let icon = crate::app_icon::load_app_icon(true);
        if icon.is_null() {
            return Err(AppError::Windows {
                operation: "load tray icon",
                win32_code: last_error(),
            });
        }
        let mut tray = Self {
            window,
            _icon: icon,
            active: false,
        };
        let mut data = tray.base_data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = WM_TRAY_CALLBACK;
        data.hIcon = icon;
        copy_wide(&mut data.szTip, "AskBridge");
        // SAFETY: data is fully initialized and references a live owner window.
        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
            return Err(AppError::Windows {
                operation: "Shell_NotifyIconW(NIM_ADD)",
                win32_code: last_error(),
            });
        }
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: data identifies the tray item just added by this process.
        if unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) } == 0 {
            let win32_code = last_error();
            // SAFETY: Best-effort cleanup of the item added immediately above.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &data);
            }
            return Err(AppError::Windows {
                operation: "Shell_NotifyIconW(NIM_SETVERSION)",
                win32_code,
            });
        }
        tray.active = true;
        Ok(tray)
    }

    pub fn show_menu(
        &self,
        paused: bool,
        available_update: Option<&str>,
        update_busy: bool,
    ) -> Result<Option<u16>> {
        // SAFETY: CreatePopupMenu has no preconditions.
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return Err(AppError::Windows {
                operation: "CreatePopupMenu",
                win32_code: last_error(),
            });
        }

        let result = (|| {
            append_item(menu, MENU_CAPTURE_WITH_PROMPT, "截图并提问", MF_STRING)?;
            append_item(menu, MENU_CAPTURE_QUICK, "截图快速投递", MF_STRING)?;
            append_item(menu, MENU_TEXT_ONLY, "直接文字提问", MF_STRING)?;
            append_item(menu, 0, "", MF_SEPARATOR)?;
            append_item(
                menu,
                MENU_PAUSE,
                "暂停快捷键",
                MF_STRING | if paused { MF_CHECKED } else { MF_UNCHECKED },
            )?;
            append_item(menu, MENU_SETTINGS, "设置…", MF_STRING)?;
            append_item(menu, 0, "", MF_SEPARATOR)?;
            append_item(
                menu,
                MENU_CHECK_UPDATES,
                if update_busy {
                    "正在检查或下载更新…"
                } else {
                    "检查更新…"
                },
                MF_STRING | if update_busy { MF_GRAYED } else { 0 },
            )?;
            if let Some(version) = available_update {
                append_item(
                    menu,
                    MENU_INSTALL_UPDATE,
                    &format!("安装 AskBridge {version}…"),
                    MF_STRING | if update_busy { MF_GRAYED } else { 0 },
                )?;
            }
            append_item(menu, 0, "", MF_SEPARATOR)?;
            append_item(menu, MENU_EXIT, "退出", MF_STRING)?;

            let mut point = POINT { x: 0, y: 0 };
            // SAFETY: point is valid writable storage.
            if unsafe { GetCursorPos(&mut point) } == 0 {
                return Err(AppError::Windows {
                    operation: "GetCursorPos",
                    win32_code: last_error(),
                });
            }
            // SAFETY: The owner window and menu are valid for the duration of this call.
            unsafe {
                SetForegroundWindow(self.window);
            }
            // SAFETY: The menu and owner window are live, and TPM_RETURNCMD requests a command id.
            let command = unsafe {
                TrackPopupMenu(
                    menu,
                    TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD,
                    point.x,
                    point.y,
                    0,
                    self.window,
                    std::ptr::null(),
                )
            };
            Ok((command != 0).then_some(command as u16))
        })();

        // SAFETY: menu was created by CreatePopupMenu and is no longer in use.
        unsafe {
            DestroyMenu(menu);
        }
        result
    }

    pub fn notify(&self, title: &str, body: &str) {
        let mut data = self.base_data();
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = NIIF_INFO;
        copy_wide(&mut data.szInfoTitle, title);
        copy_wide(&mut data.szInfo, body);
        // SAFETY: data references the live tray item; notification failure is non-fatal.
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    fn base_data(&self) -> NOTIFYICONDATAW {
        // SAFETY: Zero is a valid initial state for NOTIFYICONDATAW before required fields are set.
        let mut data: NOTIFYICONDATAW = unsafe { zeroed() };
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.window;
        data.uID = TRAY_ICON_ID;
        data
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        if self.active {
            let data = self.base_data();
            // SAFETY: data identifies the tray item created by this guard.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &data);
            }
            self.active = false;
        }
    }
}

fn append_item(
    menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
    id: u16,
    label: &str,
    flags: u32,
) -> Result<()> {
    let label = wide(label);
    let text = if flags & MF_SEPARATOR != 0 {
        std::ptr::null()
    } else {
        label.as_ptr()
    };
    // SAFETY: menu is live and the text buffer remains valid for the synchronous call.
    if unsafe { AppendMenuW(menu, flags, usize::from(id), text) } == 0 {
        return Err(AppError::Windows {
            operation: "AppendMenuW",
            win32_code: last_error(),
        });
    }
    Ok(())
}

fn copy_wide<const N: usize>(destination: &mut [u16; N], value: &str) {
    let encoded = value.encode_utf16().take(N.saturating_sub(1));
    for (slot, character) in destination.iter_mut().zip(encoded) {
        *slot = character;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_legacy_context_menu_callback() {
        assert_eq!(
            decode_tray_callback(WM_RBUTTONUP as LPARAM),
            TrayEvent::ContextMenu
        );
    }

    #[test]
    fn decodes_version_4_context_menu_callback() {
        let packed = ((TRAY_ICON_ID << 16) | WM_CONTEXTMENU) as LPARAM;

        assert_eq!(decode_tray_callback(packed), TrayEvent::ContextMenu);
    }

    #[test]
    fn decodes_version_4_double_click_callback() {
        let packed = ((TRAY_ICON_ID << 16) | WM_LBUTTONDBLCLK) as LPARAM;

        assert_eq!(decode_tray_callback(packed), TrayEvent::ActivateSettings);
    }
}
