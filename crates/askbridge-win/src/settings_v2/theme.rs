use std::{
    ffi::c_void,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use askbridge_core::{AppError, Result};
use windows_sys::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreatePen, CreateSolidBrush,
        DEFAULT_CHARSET, DEFAULT_PITCH, DeleteObject, DrawTextW, FF_DONTCARE, FW_NORMAL,
        FW_SEMIBOLD, FillRect, GetStockObject, InvalidateRect, NULL_BRUSH, OUT_DEFAULT_PRECIS,
        PS_SOLID, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
    UI::{
        Controls::{DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_SELECTED},
        HiDpi::GetDpiForSystem,
        Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent},
        WindowsAndMessaging::{
            CallWindowProcW, GWLP_USERDATA, GWLP_WNDPROC, GetDlgCtrlID, GetDlgItem, GetParent,
            GetWindowLongPtrW, GetWindowTextW, IsWindowVisible, SendMessageW, SetWindowLongPtrW,
            WM_GETFONT, WM_MOUSEMOVE, WM_NCDESTROY, WNDPROC,
        },
    },
};

use crate::util::{last_error, wide};

use super::{
    CONTROL_APPLY, CONTROL_DECORATION, CONTROL_OPEN_BROWSER, CONTROL_OPEN_LOGIN, CONTROL_SEPARATOR,
    DESC_LABEL, PAGE_BROWSER, PAGE_GENERAL, PAGE_HOTKEYS, PAGE_PROVIDERS, STATUS_LABEL,
    SUBTITLE_LABEL, TAB_BROWSER, TAB_GENERAL, TAB_HOTKEYS, TAB_PROVIDERS,
};

// windows-sys omits this message; the value is stable since Windows 95.
const WM_MOUSELEAVE: u32 = 0x02A3;

// Fluent light palette; the burnt-orange accent stays as the brand identity
// shared with the capture toolbar (see capture/toolbar_html.rs).
const COLOR_TEXT: COLORREF = rgb(27, 27, 27);
const COLOR_MUTED: COLORREF = rgb(96, 96, 96);
const COLOR_ACCENT: COLORREF = rgb(153, 60, 29);
const COLOR_ACCENT_HOVER: COLORREF = rgb(173, 73, 40);
const COLOR_ACCENT_PRESSED: COLORREF = rgb(140, 52, 25);
const COLOR_SECONDARY: COLORREF = rgb(251, 251, 251);
const COLOR_SECONDARY_HOVER: COLORREF = rgb(243, 243, 243);
const COLOR_SECONDARY_PRESSED: COLORREF = rgb(238, 238, 238);
const COLOR_BORDER: COLORREF = rgb(208, 208, 208);
const COLOR_FRAME: COLORREF = rgb(212, 212, 216);
const COLOR_SEPARATOR: COLORREF = rgb(226, 226, 230);
const COLOR_DISABLED: COLORREF = rgb(150, 150, 150);
const COLOR_WHITE: COLORREF = rgb(255, 255, 255);
const COLOR_TAB_HOVER: COLORREF = rgb(240, 240, 240);
pub(super) const COLOR_WINDOW_BG: COLORREF = rgb(250, 250, 250);

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
}

/// Process-lifetime brushes, created once like the system brushes they stand
/// in for. Returning one per WM_CTLCOLORSTATIC matches the system's contract.
fn cached_brush(slot: &OnceLock<usize>, color: COLORREF) -> *mut c_void {
    // SAFETY: the brush is a plain GDI object kept alive for the process.
    *slot.get_or_init(|| unsafe { CreateSolidBrush(color) } as usize) as *mut c_void
}

pub(super) fn window_background_brush() -> *mut c_void {
    static BRUSH: OnceLock<usize> = OnceLock::new();
    cached_brush(&BRUSH, COLOR_WINDOW_BG)
}

fn tab_hover_brush() -> *mut c_void {
    static BRUSH: OnceLock<usize> = OnceLock::new();
    cached_brush(&BRUSH, COLOR_TAB_HOVER)
}

fn frame_brush() -> *mut c_void {
    static BRUSH: OnceLock<usize> = OnceLock::new();
    cached_brush(&BRUSH, COLOR_FRAME)
}

fn separator_brush() -> *mut c_void {
    static BRUSH: OnceLock<usize> = OnceLock::new();
    cached_brush(&BRUSH, COLOR_SEPARATOR)
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
            title: OwnedFont::create(20, FW_SEMIBOLD as i32, scale)?,
            body: OwnedFont::create(14, FW_NORMAL as i32, scale)?,
            label: OwnedFont::create(14, FW_SEMIBOLD as i32, scale)?,
            small: OwnedFont::create(12, FW_NORMAL as i32, scale)?,
        })
    }
}

// Owner-drawn buttons get no hover feedback from the system, so a per-button
// subclass tracks WM_MOUSEMOVE/WM_MOUSELEAVE and republishes the hovered
// control for the draw routine. All settings UI runs on one thread.
static HOVERED_CONTROL: AtomicUsize = AtomicUsize::new(0);

fn is_hovered(window: HWND) -> bool {
    HOVERED_CONTROL.load(Ordering::Relaxed) == window as usize
}

pub(super) fn install_hover_tracking(control: HWND) {
    // SAFETY: control is a live owner-drawn button created on the UI thread.
    unsafe {
        let previous = GetWindowLongPtrW(control, GWLP_WNDPROC);
        if previous == hover_button_proc as *const () as isize || previous == 0 {
            return;
        }
        SetWindowLongPtrW(control, GWLP_USERDATA, previous);
        SetWindowLongPtrW(
            control,
            GWLP_WNDPROC,
            hover_button_proc as *const () as isize,
        );
    }
}

unsafe extern "system" fn hover_button_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_MOUSEMOVE => {
            if !is_hovered(window) {
                HOVERED_CONTROL.store(window as usize, Ordering::Relaxed);
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: window,
                    dwHoverTime: 0,
                };
                // SAFETY: track describes this window for the synchronous call.
                unsafe {
                    TrackMouseEvent(&mut track);
                    InvalidateRect(window, std::ptr::null(), 0);
                }
            }
        }
        WM_MOUSELEAVE => {
            if is_hovered(window) {
                HOVERED_CONTROL.store(0, Ordering::Relaxed);
            }
            // SAFETY: window is live.
            unsafe {
                InvalidateRect(window, std::ptr::null(), 0);
            }
        }
        WM_NCDESTROY if is_hovered(window) => {
            HOVERED_CONTROL.store(0, Ordering::Relaxed);
        }
        _ => {}
    }
    // SAFETY: GWLP_USERDATA holds the original button procedure stored by
    // install_hover_tracking before this subclass was installed.
    let previous = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) };
    let previous: WNDPROC = unsafe { std::mem::transmute(previous) };
    // SAFETY: previous is the live button procedure; all parameters pass through.
    unsafe { CallWindowProcW(previous, window, message, wparam, lparam) }
}

pub(super) fn draw_owner_button(item: &DRAWITEMSTRUCT) -> LRESULT {
    if (TAB_HOTKEYS as u32..=TAB_GENERAL as u32).contains(&item.CtlID) {
        return draw_tab(item);
    }
    let primary = item.CtlID == CONTROL_APPLY as u32
        || item.CtlID == CONTROL_OPEN_BROWSER as u32
        || item.CtlID == CONTROL_OPEN_LOGIN as u32;
    let disabled = item.itemState & ODS_DISABLED != 0;
    let pressed = item.itemState & ODS_SELECTED != 0;
    let hovered = !disabled && is_hovered(item.hwndItem);
    let fill_color = if primary {
        if pressed {
            COLOR_ACCENT_PRESSED
        } else if hovered {
            COLOR_ACCENT_HOVER
        } else {
            COLOR_ACCENT
        }
    } else if pressed {
        COLOR_SECONDARY_PRESSED
    } else if hovered {
        COLOR_SECONDARY_HOVER
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
        // Blend the rounded corners into the page background.
        FillRect(item.hDC, &item.rcItem, window_background_brush());

        let height = item.rcItem.bottom - item.rcItem.top;
        let corner = ((height / 9).max(4)) * 2;
        let fill = CreateSolidBrush(fill_color);
        let border = CreatePen(PS_SOLID, 1, border_color);
        let previous_brush = SelectObject(item.hDC, fill);
        let previous_pen = SelectObject(item.hDC, border);
        RoundRect(
            item.hDC,
            item.rcItem.left,
            item.rcItem.top,
            item.rcItem.right,
            item.rcItem.bottom,
            corner,
            corner,
        );
        SelectObject(item.hDC, previous_brush);
        SelectObject(item.hDC, previous_pen);
        DeleteObject(fill);
        DeleteObject(border);

        if item.itemState & ODS_FOCUS != 0 && !disabled {
            // Fluent focus visual: a dark ring inset from the control edge.
            let ring = CreatePen(PS_SOLID, 1, COLOR_TEXT);
            let null_brush = GetStockObject(NULL_BRUSH);
            let previous_brush = SelectObject(item.hDC, null_brush);
            let previous_pen = SelectObject(item.hDC, ring);
            RoundRect(
                item.hDC,
                item.rcItem.left + 3,
                item.rcItem.top + 3,
                item.rcItem.right - 3,
                item.rcItem.bottom - 3,
                corner.saturating_sub(4).max(2),
                corner.saturating_sub(4).max(2),
            );
            SelectObject(item.hDC, previous_brush);
            SelectObject(item.hDC, previous_pen);
            DeleteObject(ring);
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

/// Page tabs are owner-drawn so the strip reads as tabs instead of native
/// radio buttons. Selection state is derived from the paired page's
/// visibility, which `switch_page` keeps in sync.
fn draw_tab(item: &DRAWITEMSTRUCT) -> LRESULT {
    let parent = unsafe { GetParent(item.hwndItem) };
    let page = if parent.is_null() {
        std::ptr::null_mut()
    } else {
        unsafe { GetDlgItem(parent, i32::from(page_for_tab(item.CtlID))) }
    };
    let selected = !page.is_null() && unsafe { IsWindowVisible(page) != 0 };
    let hovered = is_hovered(item.hwndItem);
    let text_color = if selected { COLOR_TEXT } else { COLOR_MUTED };

    // SAFETY: item contains the valid HDC and RECT supplied by WM_DRAWITEM.
    unsafe {
        let background = if !selected && hovered {
            tab_hover_brush()
        } else {
            window_background_brush()
        };
        FillRect(item.hDC, &item.rcItem, background);

        if selected {
            // Pivot-style indicator: a short rounded accent bar under the label.
            let indicator = RECT {
                left: item.rcItem.left + 10,
                top: item.rcItem.bottom - 4,
                right: item.rcItem.right - 10,
                bottom: item.rcItem.bottom - 1,
            };
            let fill = CreateSolidBrush(COLOR_ACCENT);
            let edge = CreatePen(PS_SOLID, 1, COLOR_ACCENT);
            let previous_brush = SelectObject(item.hDC, fill);
            let previous_pen = SelectObject(item.hDC, edge);
            RoundRect(
                item.hDC,
                indicator.left,
                indicator.top,
                indicator.right,
                indicator.bottom,
                3,
                3,
            );
            SelectObject(item.hDC, previous_brush);
            SelectObject(item.hDC, previous_pen);
            DeleteObject(fill);
            DeleteObject(edge);
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
        if selected {
            text_rect.bottom -= 4;
        }
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

fn page_for_tab(tab_id: u32) -> u16 {
    if tab_id == TAB_HOTKEYS as u32 {
        PAGE_HOTKEYS
    } else if tab_id == TAB_PROVIDERS as u32 {
        PAGE_PROVIDERS
    } else if tab_id == TAB_BROWSER as u32 {
        PAGE_BROWSER
    } else {
        PAGE_GENERAL
    }
}

pub(super) fn static_control_color(window: HWND, device_context: *mut c_void) -> LRESULT {
    // SAFETY: window and device_context are supplied by WM_CTLCOLORSTATIC for a live control.
    unsafe {
        SetBkMode(device_context, TRANSPARENT as i32);
        let id = GetDlgCtrlID(window);
        match id as u16 {
            SUBTITLE_LABEL | STATUS_LABEL | DESC_LABEL => {
                SetTextColor(device_context, COLOR_MUTED);
            }
            CONTROL_SEPARATOR => {
                return separator_brush() as LRESULT;
            }
            CONTROL_DECORATION => {
                // Decorative frames and separators are painted with the frame
                // color; the brush is process-cached like the system ones.
                return frame_brush() as LRESULT;
            }
            _ => {
                SetTextColor(device_context, COLOR_TEXT);
            }
        }
        window_background_brush() as LRESULT
    }
}
