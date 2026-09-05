use windows_sys::Win32::{
    Graphics::Gdi::{DEFAULT_GUI_FONT, GetStockObject},
    UI::WindowsAndMessaging::GetWindowTextW,
};

use super::*;
use super::theme::UiScale;

#[allow(clippy::too_many_arguments)]
pub(super) fn create_label(
    parent: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: u16,
) -> Result<HWND> {
    create_control(
        parent,
        instance,
        scale,
        "STATIC",
        text,
        WS_CHILD | WS_VISIBLE,
        x,
        y,
        width,
        height,
        0,
        id,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_check(
    parent: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    fonts: &UiFonts,
    text: &str,
    x: i32,
    y: i32,
    id: u16,
) -> Result<HWND> {
    let check = create_control(
        parent,
        instance,
        scale,
        "BUTTON",
        text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        x,
        y,
        710,
        32,
        0,
        id,
    )?;
    set_font(check, fonts.body.handle());
    Ok(check)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_button(
    parent: HWND,
    instance: HINSTANCE,
    scale: UiScale,
    fonts: &UiFonts,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    id: u16,
) -> Result<HWND> {
    let button = create_control(
        parent,
        instance,
        scale,
        "BUTTON",
        text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        x,
        y,
        width,
        36,
        0,
        id,
    )?;
    set_font(button, fonts.label.handle());
    Ok(button)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn create_control(
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
    extended_style: u32,
    id: u16,
) -> Result<HWND> {
    let is_edit = class == "EDIT";
    let class = wide(class);
    let text = wide(text);
    // SAFETY: Class/text buffers are valid for the call, and parent/instance are live.
    let control = unsafe {
        CreateWindowExW(
            extended_style,
            class.as_ptr(),
            text.as_ptr(),
            style,
            scale.px(x),
            scale.px(y),
            scale.px(width),
            scale.px(height),
            parent,
            id as usize as _,
            instance,
            ptr::null(),
        )
    };
    if control.is_null() {
        return Err(AppError::Windows {
            operation: "CreateWindowExW(settings control)",
            win32_code: last_error(),
        });
    }
    // SAFETY: DEFAULT_GUI_FONT is a process-lifetime stock object and control is live.
    unsafe {
        SendMessageW(
            control,
            WM_SETFONT,
            GetStockObject(DEFAULT_GUI_FONT) as WPARAM,
            1,
        );
        if is_edit {
            SendMessageW(
                control,
                EM_SETMARGINS,
                (EC_LEFTMARGIN | EC_RIGHTMARGIN) as WPARAM,
                make_lparam(10, 10),
            );
        }
    }
    Ok(control)
}

pub(super) fn set_font(window: HWND, font: *mut c_void) {
    // SAFETY: font is owned by SettingsWindow and outlives the child window.
    unsafe {
        SendMessageW(window, WM_SETFONT, font as WPARAM, 1);
    }
}

pub(super) fn set_limit(edit: HWND, limit: WPARAM) {
    // SAFETY: edit is a live EDIT control.
    unsafe {
        SendMessageW(edit, EM_SETLIMITTEXT, limit, 0);
    }
}

pub(super) fn set_text(window: HWND, value: &str) -> Result<()> {
    let value = wide(value);
    // SAFETY: window is live and value is a nul-terminated UTF-16 string.
    if unsafe { SetWindowTextW(window, value.as_ptr()) } == 0 {
        return Err(AppError::Windows {
            operation: "SetWindowTextW(settings)",
            win32_code: last_error(),
        });
    }
    Ok(())
}

pub(super) fn get_text(window: HWND) -> Result<String> {
    // SAFETY: window is a live control and the query is read-only.
    let length = unsafe { GetWindowTextLengthW(window) };
    if length < 0 {
        return Err(AppError::Windows {
            operation: "GetWindowTextLengthW(settings)",
            win32_code: last_error(),
        });
    }
    let mut buffer = vec![0u16; length as usize + 1];
    // SAFETY: buffer has room for the reported text and terminating nul.
    let copied = unsafe { GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32) };
    if copied < 0 {
        return Err(AppError::Windows {
            operation: "GetWindowTextW(settings)",
            win32_code: last_error(),
        });
    }
    buffer.truncate(copied as usize);
    Ok(String::from_utf16_lossy(&buffer))
}

pub(super) fn set_checked(control: HWND, checked: bool) {
    // SAFETY: control is a live checkbox or radio button.
    unsafe {
        SendMessageW(
            control,
            BM_SETCHECK,
            if checked {
                BST_CHECKED as WPARAM
            } else {
                BST_UNCHECKED as WPARAM
            },
            0,
        );
    }
}

pub(super) fn is_checked(control: HWND) -> bool {
    // SAFETY: control is a live checkbox.
    unsafe { SendMessageW(control, BM_GETCHECK, 0, 0) == BST_CHECKED as isize }
}

pub(super) fn combo_reset(combo: HWND) {
    // SAFETY: combo is a live COMBOBOX.
    unsafe {
        SendMessageW(combo, CB_RESETCONTENT, 0, 0);
    }
}

pub(super) fn combo_add(combo: HWND, value: &str) -> Result<usize> {
    let value = wide(value);
    // SAFETY: combo is live and value remains valid for the synchronous call.
    let index = unsafe { SendMessageW(combo, CB_ADDSTRING, 0, value.as_ptr() as LPARAM) };
    if index < 0 {
        return Err(AppError::Windows {
            operation: "CB_ADDSTRING(settings)",
            win32_code: last_error(),
        });
    }
    Ok(index as usize)
}

pub(super) fn combo_select(combo: HWND, index: usize) {
    // SAFETY: combo is live; invalid indices are detected later by read_config.
    unsafe {
        SendMessageW(combo, CB_SETCURSEL, index, 0);
    }
}

pub(super) fn combo_selection(combo: HWND) -> Result<usize> {
    // SAFETY: combo is a live COMBOBOX.
    let selected = unsafe { SendMessageW(combo, CB_GETCURSEL, 0, 0) };
    if selected < 0 {
        Err(AppError::ConfigurationInvalid(
            "a required setting has no selection".to_owned(),
        ))
    } else {
        Ok(selected as usize)
    }
}
