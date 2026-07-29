use std::ffi::c_void;

use askbridge_core::{AppCommand, AppError, HotkeyBinding, HotkeyConfig, Result};
use windows_sys::Win32::{
    Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_WINDOW, CreateFontW, CreateSolidBrush,
        DEFAULT_CHARSET, DEFAULT_PITCH, DeleteObject, DrawTextW, FF_DONTCARE, FW_NORMAL,
        FW_SEMIBOLD, FillRect, FrameRect, GetSysColorBrush, OUT_DEFAULT_PRECIS, SelectObject,
        SetBkMode, SetTextColor, TRANSPARENT,
    },
    UI::{
        Controls::{
            BST_CHECKED, BST_UNCHECKED, DRAWITEMSTRUCT, EM_SETMARGINS, ODS_DISABLED, ODS_SELECTED,
        },
        HiDpi::GetDpiForSystem,
        WindowsAndMessaging::{
            BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_OWNERDRAW, CreateWindowExW,
            DefWindowProcW, DestroyWindow, EC_LEFTMARGIN, EC_RIGHTMARGIN, ES_AUTOHSCROLL,
            FindWindowW, GetDlgCtrlID, GetWindowTextLengthW, GetWindowTextW, HMENU, PostMessageW,
            SW_HIDE, SW_SHOW, SendMessageW, SetForegroundWindow, SetWindowTextW, ShowWindow,
            WM_CLOSE, WM_COMMAND, WM_CTLCOLORSTATIC, WM_DRAWITEM, WM_GETFONT, WM_SETFONT,
            WS_BORDER, WS_CAPTION, WS_CHILD, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP,
            WS_VISIBLE,
        },
    },
};

use crate::{
    single_instance::{MAIN_WINDOW_CLASS, MAIN_WINDOW_TITLE},
    util::{last_error, wide},
};

pub const SETTINGS_CLASS: &str = "AskBridge.Settings.Window.v1";
pub const CONTROL_APPLY: u16 = 2001;
pub const CONTROL_RESTORE_DEFAULTS: u16 = 2002;
pub const CONTROL_CLOSE: u16 = 2003;

const EDIT_CAPTURE: u16 = 2101;
const CHECK_CAPTURE: u16 = 2102;
const EDIT_QUICK: u16 = 2103;
const CHECK_QUICK: u16 = 2104;
const EDIT_TEXT: u16 = 2105;
const CHECK_TEXT: u16 = 2106;
const STATUS_LABEL: u16 = 2107;
const TITLE_LABEL: u16 = 2108;
const SUBTITLE_LABEL: u16 = 2109;
const DESCRIPTION_CAPTURE: u16 = 2110;
const DESCRIPTION_QUICK: u16 = 2111;
const DESCRIPTION_TEXT: u16 = 2112;

const WINDOW_WIDTH: i32 = 720;
const WINDOW_HEIGHT: i32 = 460;

const COLOR_TEXT: COLORREF = rgb(30, 41, 59);
const COLOR_MUTED: COLORREF = rgb(100, 116, 139);
const COLOR_ACCENT: COLORREF = rgb(37, 99, 235);
const COLOR_ACCENT_PRESSED: COLORREF = rgb(29, 78, 216);
const COLOR_SECONDARY: COLORREF = rgb(241, 245, 249);
const COLOR_SECONDARY_PRESSED: COLORREF = rgb(226, 232, 240);
const COLOR_BORDER: COLORREF = rgb(203, 213, 225);
const COLOR_DISABLED: COLORREF = rgb(148, 163, 184);
const COLOR_WHITE: COLORREF = rgb(255, 255, 255);

#[derive(Clone, Copy)]
struct UiScale {
    dpi: u32,
}

impl UiScale {
    fn system() -> Self {
        // SAFETY: This read-only query is valid after process DPI awareness is initialized.
        let dpi = unsafe { GetDpiForSystem() };
        Self {
            dpi: if dpi == 0 { 96 } else { dpi },
        }
    }

    fn px(self, value: i32) -> i32 {
        ((i64::from(value) * i64::from(self.dpi) + 48) / 96) as i32
    }
}

struct OwnedFont(*mut c_void);

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
                operation: "CreateFontW",
                win32_code: last_error(),
            });
        }
        Ok(Self(font))
    }

    const fn handle(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for OwnedFont {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: The font was created by CreateFontW and is no longer used after window
            // destruction.
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
}

struct UiFonts {
    title: OwnedFont,
    body: OwnedFont,
    label: OwnedFont,
    small: OwnedFont,
}

impl UiFonts {
    fn create(scale: UiScale) -> Result<Self> {
        Ok(Self {
            title: OwnedFont::create(24, FW_SEMIBOLD as i32, scale)?,
            body: OwnedFont::create(16, FW_NORMAL as i32, scale)?,
            label: OwnedFont::create(16, FW_SEMIBOLD as i32, scale)?,
            small: OwnedFont::create(14, FW_NORMAL as i32, scale)?,
        })
    }
}

struct HotkeyRow {
    command: AppCommand,
    edit: HWND,
    enabled: HWND,
}

pub struct SettingsWindow {
    window: HWND,
    rows: Vec<HotkeyRow>,
    status: HWND,
    _fonts: UiFonts,
}

impl SettingsWindow {
    pub fn create(parent: HWND, instance: HINSTANCE, config: &HotkeyConfig) -> Result<Self> {
        let scale = UiScale::system();
        let fonts = UiFonts::create(scale)?;
        let class_name = wide(SETTINGS_CLASS);
        let title = wide("AskBridge 设置");
        // SAFETY: The registered class, parent and instance are valid; no lpParam is required.
        let window = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
                scale.px(200),
                scale.px(200),
                scale.px(WINDOW_WIDTH),
                scale.px(WINDOW_HEIGHT),
                parent,
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if window.is_null() {
            return Err(AppError::Windows {
                operation: "CreateWindowExW(settings)",
                win32_code: last_error(),
            });
        }

        let title_label = create_control(
            window,
            instance,
            scale,
            "STATIC",
            "全局快捷键",
            WS_CHILD | WS_VISIBLE,
            32,
            24,
            630,
            36,
            0,
            TITLE_LABEL,
        )?;
        set_font(title_label, fonts.title.handle());

        let subtitle = create_control(
            window,
            instance,
            scale,
            "STATIC",
            "自定义三个常用入口。修改后立即生效，发生冲突时会保留原设置。",
            WS_CHILD | WS_VISIBLE,
            32,
            62,
            630,
            24,
            0,
            SUBTITLE_LABEL,
        )?;
        set_font(subtitle, fonts.small.handle());

        let rows_data = [
            (
                AppCommand::CaptureWithPrompt,
                "截图并提问",
                "截取区域后进入提问流程",
                EDIT_CAPTURE,
                CHECK_CAPTURE,
                DESCRIPTION_CAPTURE,
            ),
            (
                AppCommand::CaptureQuickDispatch,
                "截图快速投递",
                "截取区域后直接投递到默认目标",
                EDIT_QUICK,
                CHECK_QUICK,
                DESCRIPTION_QUICK,
            ),
            (
                AppCommand::TextOnlyPrompt,
                "直接文字提问",
                "不截图，直接打开文字提问入口",
                EDIT_TEXT,
                CHECK_TEXT,
                DESCRIPTION_TEXT,
            ),
        ];
        let mut rows = Vec::with_capacity(rows_data.len());
        for (index, (command, label, description, edit_id, check_id, description_id)) in
            rows_data.into_iter().enumerate()
        {
            let y = 108 + index as i32 * 72;
            let label_control = create_control(
                window,
                instance,
                scale,
                "STATIC",
                label,
                WS_CHILD | WS_VISIBLE,
                32,
                y,
                270,
                26,
                0,
                0,
            )?;
            set_font(label_control, fonts.label.handle());

            let description_control = create_control(
                window,
                instance,
                scale,
                "STATIC",
                description,
                WS_CHILD | WS_VISIBLE,
                32,
                y + 29,
                280,
                22,
                0,
                description_id,
            )?;
            set_font(description_control, fonts.small.handle());

            let edit = create_control(
                window,
                instance,
                scale,
                "EDIT",
                "",
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
                338,
                y + 4,
                210,
                34,
                0,
                edit_id,
            )?;
            set_font(edit, fonts.body.handle());
            // SAFETY: edit is a live EDIT control; the LPARAM packs 10px left/right margins.
            unsafe {
                SendMessageW(
                    edit,
                    EM_SETMARGINS,
                    (EC_LEFTMARGIN | EC_RIGHTMARGIN) as WPARAM,
                    make_lparam(scale.px(10) as u16, scale.px(10) as u16),
                );
            }

            let enabled = create_control(
                window,
                instance,
                scale,
                "BUTTON",
                "启用",
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                576,
                y + 6,
                82,
                30,
                0,
                check_id,
            )?;
            set_font(enabled, fonts.body.handle());
            rows.push(HotkeyRow {
                command,
                edit,
                enabled,
            });
        }

        let apply = create_control(
            window,
            instance,
            scale,
            "BUTTON",
            "应用更改",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
            338,
            333,
            112,
            38,
            0,
            CONTROL_APPLY,
        )?;
        let defaults = create_control(
            window,
            instance,
            scale,
            "BUTTON",
            "恢复默认",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
            462,
            333,
            96,
            38,
            0,
            CONTROL_RESTORE_DEFAULTS,
        )?;
        let close = create_control(
            window,
            instance,
            scale,
            "BUTTON",
            "关闭",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
            570,
            333,
            88,
            38,
            0,
            CONTROL_CLOSE,
        )?;
        let status = create_control(
            window,
            instance,
            scale,
            "STATIC",
            "准备就绪。修改快捷键后点击“应用更改”。",
            WS_CHILD | WS_VISIBLE,
            32,
            392,
            626,
            24,
            0,
            STATUS_LABEL,
        )?;

        for control in [apply, defaults, close] {
            set_font(control, fonts.body.handle());
        }
        set_font(status, fonts.small.handle());

        let settings = Self {
            window,
            rows,
            status,
            _fonts: fonts,
        };
        settings.refresh(config);
        Ok(settings)
    }

    pub const fn hwnd(&self) -> HWND {
        self.window
    }

    pub fn show(&self) {
        // SAFETY: window is owned by this object and remains valid.
        unsafe {
            // The first ShowWindow call may consume STARTUPINFO.wShowWindow from launchers.
            ShowWindow(self.window, SW_SHOW);
            ShowWindow(self.window, SW_SHOW);
            SetForegroundWindow(self.window);
        }
    }

    pub fn hide(&self) {
        // SAFETY: window is owned by this object and remains valid.
        unsafe {
            ShowWindow(self.window, SW_HIDE);
        }
    }

    pub fn refresh(&self, config: &HotkeyConfig) {
        for row in &self.rows {
            let binding = config.binding(row.command);
            set_text(row.edit, &binding.to_string());
            // SAFETY: enabled is a checkbox control and BM_SETCHECK accepts these values.
            unsafe {
                SendMessageW(
                    row.enabled,
                    BM_SETCHECK,
                    if binding.enabled {
                        BST_CHECKED as WPARAM
                    } else {
                        BST_UNCHECKED as WPARAM
                    },
                    0,
                );
            }
        }
    }

    pub fn read_hotkeys(&self) -> Result<HotkeyConfig> {
        let mut config = HotkeyConfig::default();
        for row in &self.rows {
            let text = get_text(row.edit)?;
            let mut binding = text
                .parse::<HotkeyBinding>()
                .map_err(|error| AppError::InvalidHotkey(format!("{text}: {error}")))?;
            // SAFETY: enabled is a checkbox control and BM_GETCHECK returns its check state.
            binding.enabled =
                unsafe { SendMessageW(row.enabled, BM_GETCHECK, 0, 0) } == BST_CHECKED as isize;
            *config.binding_mut(row.command) = binding;
        }
        config.validate()?;
        Ok(config)
    }

    pub fn set_status(&self, message: &str) {
        set_text(self.status, message);
    }
}

impl Drop for SettingsWindow {
    fn drop(&mut self) {
        if !self.window.is_null() {
            // SAFETY: This object owns the top-level window and drops it on the creating UI
            // thread. Win32 destroys all child controls with their parent.
            unsafe {
                DestroyWindow(self.window);
            }
            self.window = std::ptr::null_mut();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn create_control(
    parent: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    ex_style: u32,
    id: u16,
) -> Result<HWND> {
    let class = wide(class);
    let text = wide(text);
    // SAFETY: Parent and module instance are valid; class names are Win32 standard controls.
    let control = unsafe {
        CreateWindowExW(
            ex_style,
            class.as_ptr(),
            text.as_ptr(),
            style,
            scale.px(x),
            scale.px(y),
            scale.px(width),
            scale.px(height),
            parent,
            id as usize as HMENU,
            instance,
            std::ptr::null(),
        )
    };
    if control.is_null() {
        return Err(AppError::Windows {
            operation: "CreateWindowExW(control)",
            win32_code: last_error(),
        });
    }
    Ok(control)
}

fn set_font(control: HWND, font: *mut c_void) {
    // SAFETY: control and font remain valid for the control's lifetime; ownership stays local.
    unsafe {
        SendMessageW(control, WM_SETFONT, font as WPARAM, 1);
    }
}

fn set_text(window: HWND, value: &str) {
    let value = wide(value);
    // SAFETY: window is valid and value is nul-terminated for the duration of the call.
    unsafe {
        SetWindowTextW(window, value.as_ptr());
    }
}

fn get_text(window: HWND) -> Result<String> {
    // SAFETY: window is a live edit control.
    let length = unsafe { GetWindowTextLengthW(window) };
    let mut buffer = vec![0_u16; length as usize + 1];
    // SAFETY: buffer has capacity for the requested text plus terminator.
    let copied = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
    if copied == 0 && length > 0 {
        return Err(AppError::Windows {
            operation: "GetWindowTextW",
            win32_code: last_error(),
        });
    }
    Ok(String::from_utf16_lossy(&buffer[..copied as usize]))
}

fn draw_owner_button(item: &DRAWITEMSTRUCT) -> LRESULT {
    let primary = item.CtlID == CONTROL_APPLY as u32;
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

        let mut text = [0_u16; 64];
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

fn static_control_color(window: HWND, device_context: *mut c_void) -> LRESULT {
    // SAFETY: window and device_context are supplied by WM_CTLCOLORSTATIC for a live control.
    unsafe {
        SetBkMode(device_context, TRANSPARENT as i32);
        let id = GetDlgCtrlID(window);
        let color = match id as u16 {
            SUBTITLE_LABEL | DESCRIPTION_CAPTURE | DESCRIPTION_QUICK | DESCRIPTION_TEXT
            | STATUS_LABEL => COLOR_MUTED,
            _ => COLOR_TEXT,
        };
        SetTextColor(device_context, color);
        GetSysColorBrush(COLOR_WINDOW) as LRESULT
    }
}

const fn rgb(red: u8, green: u8, blue: u8) -> COLORREF {
    red as COLORREF | ((green as COLORREF) << 8) | ((blue as COLORREF) << 16)
}

const fn make_lparam(low: u16, high: u16) -> LPARAM {
    (low as usize | ((high as usize) << 16)) as LPARAM
}

pub unsafe extern "system" fn settings_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_COMMAND => {
            let class = wide(MAIN_WINDOW_CLASS);
            let title = wide(MAIN_WINDOW_TITLE);
            // SAFETY: Both search strings are valid nul-terminated UTF-16 buffers.
            let main_window = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
            if !main_window.is_null() {
                // SAFETY: WM_COMMAND carries control identifiers and targets our own UI thread.
                unsafe {
                    PostMessageW(main_window, message, wparam, lparam);
                }
            }
            0
        }
        WM_DRAWITEM => {
            if lparam == 0 {
                return 0;
            }
            // SAFETY: For WM_DRAWITEM, lparam points to a DRAWITEMSTRUCT valid for this call.
            let item = unsafe { &*(lparam as *const DRAWITEMSTRUCT) };
            draw_owner_button(item)
        }
        WM_CTLCOLORSTATIC => static_control_color(lparam as HWND, wparam as *mut c_void),
        WM_CLOSE => {
            // SAFETY: window is the live settings window receiving this close request.
            unsafe {
                ShowWindow(window, SW_HIDE);
            }
            0
        }
        _ => {
            // SAFETY: Unhandled messages are forwarded exactly as received.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
    }
}
