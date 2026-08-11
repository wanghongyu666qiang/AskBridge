use std::ffi::c_void;

use askbridge_core::{AppError, ProviderConfig, Result};
use tracing::error;
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{
        CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
        DeleteObject, FF_DONTCARE, FW_NORMAL, FW_SEMIBOLD, OUT_DEFAULT_PRECIS,
    },
    UI::{
        Controls::EM_SETLIMITTEXT,
        HiDpi::GetDpiForSystem,
        Input::KeyboardAndMouse::{EnableWindow, SetFocus},
        WindowsAndMessaging::{
            BS_DEFPUSHBUTTON, CB_ADDSTRING, CB_ERR, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL,
            CBS_DROPDOWNLIST, CreateWindowExW, DefWindowProcW, DestroyWindow, ES_AUTOVSCROLL,
            ES_MULTILINE, ES_WANTRETURN, FindWindowW, GetSystemMetrics, GetWindowTextLengthW,
            GetWindowTextW, HMENU, IsChild, IsWindowVisible, PostMessageW, SM_CXSCREEN,
            SM_CYSCREEN, SW_HIDE, SW_SHOW, SendMessageW, SetForegroundWindow, SetWindowTextW,
            ShowWindow, WM_CLOSE, WM_COMMAND, WM_SETFONT, WS_BORDER, WS_CAPTION, WS_CHILD,
            WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_EX_TOOLWINDOW, WS_MINIMIZEBOX, WS_OVERLAPPED,
            WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
        },
    },
};

use crate::{
    single_instance::{MAIN_WINDOW_CLASS, MAIN_WINDOW_TITLE},
    util::{last_error, wide},
};

pub const PROMPT_CLASS: &str = "AskBridge.Prompt.Window.v1";
pub const CONTROL_PROMPT_SUBMIT: u16 = 3001;
pub const CONTROL_PROMPT_CANCEL: u16 = 3002;

const PROVIDER_COMBO: u16 = 3101;
const PROMPT_EDIT: u16 = 3102;
const STATUS_LABEL: u16 = 3103;
const WINDOW_WIDTH: i32 = 640;
const WINDOW_HEIGHT: i32 = 450;
const MAX_PROMPT_UTF16_UNITS: usize = 32_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptInput {
    pub provider_id: String,
    pub prompt: String,
}

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
                operation: "CreateFontW(prompt)",
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
            // SAFETY: The font was created by CreateFontW and the window is destroyed first.
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
}

struct PromptFonts {
    title: OwnedFont,
    body: OwnedFont,
    label: OwnedFont,
    small: OwnedFont,
}

impl PromptFonts {
    fn create(scale: UiScale) -> Result<Self> {
        Ok(Self {
            title: OwnedFont::create(23, FW_SEMIBOLD as i32, scale)?,
            body: OwnedFont::create(16, FW_NORMAL as i32, scale)?,
            label: OwnedFont::create(15, FW_SEMIBOLD as i32, scale)?,
            small: OwnedFont::create(13, FW_NORMAL as i32, scale)?,
        })
    }
}

pub struct PromptWindow {
    window: HWND,
    provider: HWND,
    editor: HWND,
    submit: HWND,
    cancel: HWND,
    status: HWND,
    provider_ids: Vec<String>,
    _fonts: PromptFonts,
}

impl PromptWindow {
    pub fn create(instance: HINSTANCE) -> Result<Self> {
        let scale = UiScale::system();
        let fonts = PromptFonts::create(scale)?;
        let width = scale.px(WINDOW_WIDTH);
        let height = scale.px(WINDOW_HEIGHT);
        // SAFETY: System metrics are read-only and always available for the desktop.
        let x = unsafe { (GetSystemMetrics(SM_CXSCREEN) - width) / 2 };
        // SAFETY: System metrics are read-only and always available for the desktop.
        let y = unsafe { (GetSystemMetrics(SM_CYSCREEN) - height) / 2 };
        let class_name = wide(PROMPT_CLASS);
        let title = wide("AskBridge 提问");
        // SAFETY: The class is registered and all string buffers live through this call.
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_CONTROLPARENT,
                class_name.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
                x,
                y,
                width,
                height,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            )
        };
        if window.is_null() {
            return Err(AppError::Windows {
                operation: "CreateWindowExW(prompt)",
                win32_code: last_error(),
            });
        }

        let title_label = create_control(
            window,
            instance,
            scale,
            "STATIC",
            "准备问题",
            WS_CHILD | WS_VISIBLE,
            28,
            22,
            560,
            34,
            0,
            0,
        )?;
        set_font(title_label, fonts.title.handle());

        let subtitle = create_control(
            window,
            instance,
            scale,
            "STATIC",
            "选择供应商并输入问题。AskBridge 1.0 只准备内容，不会自动发送。",
            WS_CHILD | WS_VISIBLE,
            28,
            58,
            570,
            24,
            0,
            0,
        )?;
        set_font(subtitle, fonts.small.handle());

        let provider_label = create_control(
            window,
            instance,
            scale,
            "STATIC",
            "供应商",
            WS_CHILD | WS_VISIBLE,
            28,
            94,
            120,
            22,
            0,
            0,
        )?;
        set_font(provider_label, fonts.label.handle());

        let provider = create_control(
            window,
            instance,
            scale,
            "COMBOBOX",
            "",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_VSCROLL
                | u32::try_from(CBS_DROPDOWNLIST).unwrap_or_default(),
            28,
            119,
            570,
            180,
            WS_EX_CLIENTEDGE,
            PROVIDER_COMBO,
        )?;
        set_font(provider, fonts.body.handle());

        let prompt_label = create_control(
            window,
            instance,
            scale,
            "STATIC",
            "问题",
            WS_CHILD | WS_VISIBLE,
            28,
            161,
            120,
            22,
            0,
            0,
        )?;
        set_font(prompt_label, fonts.label.handle());

        let editor = create_control(
            window,
            instance,
            scale,
            "EDIT",
            "",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | WS_BORDER
                | WS_VSCROLL
                | u32::try_from(ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN).unwrap_or_default(),
            28,
            186,
            570,
            142,
            WS_EX_CLIENTEDGE,
            PROMPT_EDIT,
        )?;
        set_font(editor, fonts.body.handle());
        // SAFETY: editor is an EDIT control; this sets a bounded UTF-16 input size.
        unsafe {
            SendMessageW(editor, EM_SETLIMITTEXT, MAX_PROMPT_UTF16_UNITS, 0);
        }

        let status = create_control(
            window,
            instance,
            scale,
            "STATIC",
            "",
            WS_CHILD | WS_VISIBLE,
            28,
            335,
            360,
            24,
            0,
            STATUS_LABEL,
        )?;
        set_font(status, fonts.small.handle());

        let cancel = create_control(
            window,
            instance,
            scale,
            "BUTTON",
            "取消",
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            400,
            349,
            92,
            36,
            0,
            CONTROL_PROMPT_CANCEL,
        )?;
        set_font(cancel, fonts.body.handle());

        let submit = create_control(
            window,
            instance,
            scale,
            "BUTTON",
            "继续",
            WS_CHILD
                | WS_VISIBLE
                | WS_TABSTOP
                | u32::try_from(BS_DEFPUSHBUTTON).unwrap_or_default(),
            506,
            349,
            92,
            36,
            0,
            CONTROL_PROMPT_SUBMIT,
        )?;
        set_font(submit, fonts.body.handle());

        Ok(Self {
            window,
            provider,
            editor,
            submit,
            cancel,
            status,
            provider_ids: Vec::new(),
            _fonts: fonts,
        })
    }

    pub fn show(&mut self, providers: &[ProviderConfig], default_provider_id: &str) -> Result<()> {
        let enabled = providers
            .iter()
            .filter(|provider| provider.enabled)
            .collect::<Vec<_>>();
        if enabled.is_empty() {
            return Err(AppError::InvalidProvider(
                "no enabled provider is available".to_owned(),
            ));
        }

        // SAFETY: provider is a live COMBOBOX control.
        unsafe {
            SendMessageW(self.provider, CB_RESETCONTENT, 0, 0);
        }
        self.provider_ids.clear();
        let mut selected = 0;
        for (index, provider) in enabled.into_iter().enumerate() {
            let display_name = wide(&provider.display_name);
            // SAFETY: provider is a COMBOBOX and the string remains valid for the call.
            let result = unsafe {
                SendMessageW(
                    self.provider,
                    CB_ADDSTRING,
                    0,
                    display_name.as_ptr() as LPARAM,
                )
            };
            if result < 0 {
                return Err(AppError::Windows {
                    operation: "CB_ADDSTRING(prompt provider)",
                    win32_code: last_error(),
                });
            }
            if provider.id == default_provider_id {
                selected = index;
            }
            self.provider_ids.push(provider.id.clone());
        }
        // SAFETY: selected is a valid index into the just-populated COMBOBOX.
        unsafe {
            SendMessageW(self.provider, CB_SETCURSEL, selected, 0);
        }
        set_text(self.editor, "");
        self.set_status("");
        self.set_busy(false);
        self.focus_existing();
        Ok(())
    }

    pub fn focus_existing(&self) {
        // SAFETY: The window and editor are owned by this object and live on this UI thread.
        unsafe {
            ShowWindow(self.window, SW_SHOW);
            ShowWindow(self.window, SW_SHOW);
            SetForegroundWindow(self.window);
            SetFocus(self.editor);
        }
    }

    pub fn hide_and_clear(&self) {
        self.set_busy(false);
        set_text(self.editor, "");
        self.set_status("");
        // SAFETY: window is owned by this object.
        unsafe {
            ShowWindow(self.window, SW_HIDE);
        }
    }

    pub fn read_input(&self) -> Result<PromptInput> {
        // SAFETY: provider is a live COMBOBOX control.
        let selected = unsafe { SendMessageW(self.provider, CB_GETCURSEL, 0, 0) };
        if selected == CB_ERR as LRESULT {
            return Err(AppError::InvalidProvider(
                "no provider is selected".to_owned(),
            ));
        }
        let provider_id = self
            .provider_ids
            .get(selected as usize)
            .cloned()
            .ok_or_else(|| {
                AppError::InvalidProvider("selected provider index is invalid".to_owned())
            })?;
        Ok(PromptInput {
            provider_id,
            prompt: get_text(self.editor)?,
        })
    }

    pub fn set_status(&self, message: &str) {
        set_text(self.status, message);
    }

    pub fn set_busy(&self, busy: bool) {
        // SAFETY: These controls are owned by this window and live on the UI thread.
        unsafe {
            EnableWindow(self.provider, i32::from(!busy));
            EnableWindow(self.editor, i32::from(!busy));
            EnableWindow(self.submit, i32::from(!busy));
            EnableWindow(self.cancel, 1);
        }
    }

    pub const fn hwnd(&self) -> HWND {
        self.window
    }

    pub const fn editor_hwnd(&self) -> HWND {
        self.editor
    }

    pub fn is_visible(&self) -> bool {
        // SAFETY: window is owned by this object.
        unsafe { IsWindowVisible(self.window) != 0 }
    }

    pub fn contains(&self, window: HWND) -> bool {
        window == self.window || {
            // SAFETY: both handles are UI handles on the same thread.
            unsafe { IsChild(self.window, window) != 0 }
        }
    }
}

impl Drop for PromptWindow {
    fn drop(&mut self) {
        if !self.window.is_null() {
            // SAFETY: This object owns the top-level window and all child controls.
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
    // SAFETY: Parent and module instance are valid; class names are standard controls.
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
            usize::from(id) as HMENU,
            instance,
            std::ptr::null(),
        )
    };
    if control.is_null() {
        return Err(AppError::Windows {
            operation: "CreateWindowExW(prompt control)",
            win32_code: last_error(),
        });
    }
    Ok(control)
}

fn set_font(control: HWND, font: *mut c_void) {
    // SAFETY: control and font remain valid for the control's lifetime.
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
    // SAFETY: window is a live EDIT control.
    let length = unsafe { GetWindowTextLengthW(window) };
    let mut buffer = vec![0_u16; length as usize + 1];
    // SAFETY: buffer has capacity for the requested text plus terminator.
    let copied = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
    if copied == 0 && length > 0 {
        return Err(AppError::Windows {
            operation: "GetWindowTextW(prompt)",
            win32_code: last_error(),
        });
    }
    Ok(String::from_utf16_lossy(&buffer[..copied as usize]))
}

pub unsafe extern "system" fn prompt_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let command = (wparam & 0xffff) as u16;
    if (message == WM_COMMAND && matches!(command, CONTROL_PROMPT_SUBMIT | CONTROL_PROMPT_CANCEL))
        || message == WM_CLOSE
    {
        let class = wide(MAIN_WINDOW_CLASS);
        let title = wide(MAIN_WINDOW_TITLE);
        // SAFETY: Both search strings are valid nul-terminated UTF-16 buffers.
        let main_window = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
        if !main_window.is_null() {
            let forwarded = if message == WM_CLOSE {
                CONTROL_PROMPT_CANCEL as WPARAM
            } else {
                wparam
            };
            // SAFETY: The target is our hidden main window on the same UI thread.
            if unsafe { PostMessageW(main_window, WM_COMMAND, forwarded, lparam) } == 0 {
                error!(
                    stage = "prompt_command_dispatch",
                    completed = false,
                    win32_code = last_error(),
                    "failed to forward a prompt command to the runtime"
                );
            }
        }
        return 0;
    }
    // SAFETY: Unhandled messages are forwarded exactly as received.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}
