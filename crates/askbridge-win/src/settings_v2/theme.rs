use std::ffi::c_void;

use askbridge_core::{AppError, Result};
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND, LRESULT, RECT},
    Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_WINDOW, CreateFontW, CreateSolidBrush,
        DEFAULT_CHARSET, DEFAULT_PITCH, DeleteObject, DrawTextW, FF_DONTCARE, FW_NORMAL,
        FW_SEMIBOLD, FillRect, FrameRect, GetSysColorBrush, OUT_DEFAULT_PRECIS, SelectObject,
        SetBkMode, SetTextColor, TRANSPARENT,
    },
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_SELECTED},
        HiDpi::GetDpiForSystem,
        WindowsAndMessaging::{GetDlgCtrlID, GetWindowTextW, SendMessageW, WM_GETFONT},
    },
};

use crate::util::{last_error, wide};

use super::{CONTROL_APPLY, CONTROL_OPEN_BROWSER, CONTROL_OPEN_LOGIN, SUBTITLE_LABEL, STATUS_LABEL};

const COLOR_TEXT: COLORREF = rgb(30, 41, 59);
const COLOR_MUTED: COLORREF = rgb(100, 116, 139);
const COLOR_ACCENT: COLORREF = rgb(37, 99, 235);
const COLOR_ACCENT_PRESSED: COLORREF = rgb(29, 78, 216);
const COLOR_SECONDARY: COLORREF = rgb(241, 245, 249);
const COLOR_SECONDARY_PRESSED: COLORREF = rgb(226, 232, 240);
const COLOR_BORDER: COLORREF = rgb(203, 213, 225);
const COLOR_DISABLED: COLORREF = rgb(148, 163, 184);
const COLOR_WHITE: COLORREF = rgb(255, 255, 255);

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
}

#[derive(Clone, Copy)]
pub(super) struct UiScale {
    dpi: u32,
}

impl UiScale {
    pub(super) fn system() -> Self {
        // SAFETY: The process selects Per-Monitor V2 awareness before creating settings UI.
        let dpi = unsafe { GetDpiForSystem() };
        Self {
            dpi: if dpi == 0 { 96 } else { dpi },
        }
    }

    pub(super) fn px(self, value: i32) -> i32 {
        ((i64::from(value) * i64::from(self.dpi) + 48) / 96) as i32
    }
}

pub(super) struct OwnedFont(*mut c_void);

impl OwnedFont {
    fn create(height: i32, weight: i32, scale: UiScale) -> Result<Self> {
        let family = wide("Microsoft YaHei UI");
        // SAFETY: All metrics are ordinary font attributes and family is nul-terminated.
        let font = unsafe {
            CreateFontW(
                -scale.px(height),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH | FF_DONTCARE) as u32,
                family.as_ptr(),
            )
        };
        if font.is_null() {
            return Err(AppError::Windows {
                operation: "CreateFontW(settings)",
                win32_code: last_error(),
            });
        }
        Ok(Self(font))
    }

    pub(super) const fn handle(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for OwnedFont {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: The top-level settings window is destroyed before the fonts are dropped.
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
}

pub(super) struct UiFonts {
    pub(super) title: OwnedFont,
    pub(super) body: OwnedFont,
    pub(super) label: OwnedFont,
    pub(super) small: OwnedFont,
}

impl UiFonts {
    pub(super) fn create(scale: UiScale) -> Result<Self> {
        Ok(Self {
            title: OwnedFont::create(24, FW_SEMIBOLD as i32, scale)?,
            body: OwnedFont::create(16, FW_NORMAL as i32, scale)?,
            label: OwnedFont::create(16, FW_SEMIBOLD as i32, scale)?,
            small: OwnedFont::create(14, FW_NORMAL as i32, scale)?,
        })
    }
}

pub(super) fn draw_owner_button(item: &DRAWITEMSTRUCT) -> LRESULT {
    let primary = item.CtlID == CONTROL_APPLY as u32
        || item.CtlID == CONTROL_OPEN_BROWSER as u32
        || item.CtlID == CONTROL_OPEN_LOGIN as u32;
    let disabled = item.itemState & ODS_DISABLED != 0;
    let pressed = item.itemState & ODS_SELECTED != 0;
    let fill_color = if primary {
        if pressed {
            COLOR_ACCENT_PRESSED
        } else {
            COLOR_ACCENT
        }
    } else if pressed {
        COLOR_SECONDARY_PRESSED
    } else {
        COLOR_SECONDARY
    };
    let border_color = if primary { fill_color } else { COLOR_BORDER };
    let text_color = if disabled {
        COLOR_DISABLED
    } else if primary {
        COLOR_WHITE
    } else {
        COLOR_TEXT
    };

    // SAFETY: item contains the valid HDC and RECT supplied by WM_DRAWITEM.
    unsafe {
        let fill = CreateSolidBrush(fill_color);
        if !fill.is_null() {
            FillRect(item.hDC, &item.rcItem, fill);
            DeleteObject(fill);
        }

        let border = CreateSolidBrush(border_color);
        if !border.is_null() {
            FrameRect(item.hDC, &item.rcItem, border);
            DeleteObject(border);
        }

        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, text_color);

        let font = SendMessageW(item.hwndItem, WM_GETFONT, 0, 0) as *mut c_void;
        let previous_font = if font.is_null() {
            std::ptr::null_mut()
        } else {
            SelectObject(item.hDC, font)
        };

        let mut text = [0_u16; 96];
        let copied = GetWindowTextW(item.hwndItem, text.as_mut_ptr(), text.len() as i32);
        let mut text_rect: RECT = item.rcItem;
        DrawTextW(
            item.hDC,
            text.as_ptr(),
            copied,
            &mut text_rect,
            0x0000_0001 | 0x0000_0004 | 0x0000_0020,
        );

        if !previous_font.is_null() {
            SelectObject(item.hDC, previous_font);
        }
    }
    1
}

pub(super) fn static_control_color(window: HWND, device_context: *mut c_void) -> LRESULT {
    // SAFETY: window and device_context are supplied by WM_CTLCOLORSTATIC for a live control.
    unsafe {
        SetBkMode(device_context, TRANSPARENT as i32);
        let id = GetDlgCtrlID(window);
        let color = match id as u16 {
            SUBTITLE_LABEL | STATUS_LABEL => COLOR_MUTED,
            _ => COLOR_TEXT,
        };
        SetTextColor(device_context, color);
        GetSysColorBrush(COLOR_WINDOW) as LRESULT
    }
}
