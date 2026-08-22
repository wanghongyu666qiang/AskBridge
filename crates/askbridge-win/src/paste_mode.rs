// Clipboard-paste dispatch target: locate an AI website window, bring it to
// the foreground, and synthesize exactly one Ctrl+V. Nothing is ever typed
// beyond that shortcut, no page content is read, and sending stays with the
// user. The result cannot be verified, so callers must say so honestly in
// their notifications.

use std::{thread, time::Duration};

use askbridge_core::{AppError, Result};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM},
    UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_CONTROL,
            VK_MENU,
        },
        Shell::ShellExecuteW,
        WindowsAndMessaging::{
            EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowTextW, IsIconic,
            IsWindowVisible, SW_RESTORE, SetForegroundWindow, ShowWindow,
        },
    },
};

use crate::util::{last_error, wide};

/// Top-level window classes of the mainstream browsers AskBridge pastes into.
const BROWSER_WINDOW_CLASSES: [&str; 2] = ["Chrome_WidgetWin_1", "MozillaWindowClass"];
/// Virtual key for the letter V (windows-sys does not export letter VKs).
const VK_V: u16 = 0x56;
/// Time allowed for the target window to actually take the foreground.
const FOCUS_SETTLE: Duration = Duration::from_millis(200);

/// Title keywords used to recognize a provider website window. Built-in
/// providers match their product names; custom providers fall back to their
/// configured display name, then to the host of the start URL.
pub(crate) fn provider_title_keywords(
    provider_id: &str,
    display_name: &str,
    start_url: &str,
) -> Vec<String> {
    let mut keywords = match provider_id {
        "chatgpt" => vec!["ChatGPT".to_owned()],
        "gemini" => vec!["Gemini".to_owned()],
        "claude" => vec!["Claude".to_owned()],
        "doubao" => vec!["豆包".to_owned()],
        _ => vec![display_name.trim().to_owned()],
    };
    if keywords.iter().all(String::is_empty) {
        keywords = vec![url_host(start_url)];
    }
    keywords.retain(|keyword| !keyword.is_empty());
    keywords
}

/// Case-insensitive containment check against the lowercased window title.
fn title_matches(title_lower: &str, keywords: &[String]) -> bool {
    keywords
        .iter()
        .any(|keyword| title_lower.contains(&keyword.to_lowercase()))
}

fn url_host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .to_owned()
}

struct WindowSearch {
    keywords: Vec<String>,
    found: Option<HWND>,
}

/// Finds the first visible top-level browser window whose title contains one
/// of the provider keywords.
pub(crate) fn find_provider_window(keywords: &[String]) -> Option<HWND> {
    if keywords.is_empty() {
        return None;
    }
    let mut search = WindowSearch {
        keywords: keywords.to_vec(),
        found: None,
    };
    // SAFETY: The callback is synchronous and receives a valid pointer to search.
    unsafe {
        EnumWindows(
            Some(enum_windows_callback),
            (&mut search as *mut WindowSearch) as LPARAM,
        );
    }
    search.found
}

unsafe extern "system" fn enum_windows_callback(window: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: lparam points to the live WindowSearch for this enumeration.
    let search = unsafe { &mut *(lparam as *mut WindowSearch) };
    // SAFETY: window is a top-level window handed out by EnumWindows.
    let visible = unsafe { IsWindowVisible(window) };
    if visible == 0 {
        return 1;
    }
    // SAFETY: window is valid and class_buffer receives at most its length.
    let mut class_buffer = [0u16; 256];
    let class_length =
        unsafe { GetClassNameW(window, class_buffer.as_mut_ptr(), class_buffer.len() as i32) };
    let class = String::from_utf16_lossy(
        &class_buffer[..class_length.clamp(0, class_buffer.len() as i32) as usize],
    );
    if !BROWSER_WINDOW_CLASSES
        .iter()
        .any(|candidate| class.eq_ignore_ascii_case(candidate))
    {
        return 1;
    }
    // SAFETY: window is valid and title_buffer receives at most its length.
    let mut title_buffer = [0u16; 512];
    let title_length =
        unsafe { GetWindowTextW(window, title_buffer.as_mut_ptr(), title_buffer.len() as i32) };
    let title = String::from_utf16_lossy(
        &title_buffer[..title_length.clamp(0, title_buffer.len() as i32) as usize],
    );
    if title_matches(&title.to_lowercase(), &search.keywords) {
        search.found = Some(window);
        return 0;
    }
    1
}

/// Restores and focuses the window, refusing to continue unless it really
/// owns the foreground so Ctrl+V can never land somewhere unexpected.
///
/// Windows denies foreground access to background processes. When the plain
/// call is rejected, a bare Alt tap makes the process satisfy the foreground
/// permission check (the classic workaround); the verification below still
/// refuses to continue unless the target really took the foreground.
pub(crate) fn activate(window: HWND) -> Result<()> {
    // SAFETY: window is a live top-level window on the calling thread's desktop.
    unsafe {
        if IsIconic(window) != 0 {
            ShowWindow(window, SW_RESTORE);
        }
        if try_focus(window) {
            return focus_verified(window);
        }
    }
    let alt_tap = [
        keyboard_input(VK_MENU, 0),
        keyboard_input(VK_MENU, KEYEVENTF_KEYUP),
    ];
    // SAFETY: alt_tap is a plain array valid for this synchronous call.
    unsafe {
        SendInput(
            alt_tap.len() as u32,
            alt_tap.as_ptr(),
            size_of::<INPUT>() as i32,
        );
    }
    thread::sleep(Duration::from_millis(60));
    try_focus(window);
    focus_verified(window)
}

/// Returns true only when `window` currently owns the foreground.
fn focus_verified(window: HWND) -> Result<()> {
    thread::sleep(FOCUS_SETTLE);
    // SAFETY: Reading the foreground window is side-effect free.
    let foreground = unsafe { GetForegroundWindow() };
    if foreground == window {
        Ok(())
    } else {
        Err(AppError::PasteTargetUnavailable)
    }
}

fn try_focus(window: HWND) -> bool {
    // SAFETY: window is a live top-level window; SetForegroundWindow may be
    // rejected by the foreground lock, which callers handle by retrying.
    unsafe { SetForegroundWindow(window) != 0 || GetForegroundWindow() == window }
}

/// Synthesizes one Ctrl+V keystroke sequence.
pub(crate) fn send_paste() -> Result<()> {
    let inputs = paste_inputs();
    // SAFETY: inputs is a plain array valid for this synchronous call.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        return Err(AppError::Windows {
            operation: "SendInput(clipboard paste)",
            win32_code: last_error(),
        });
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

fn keyboard_input(virtual_key: u16, flags: u32) -> INPUT {
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
pub(crate) fn open_default_browser(url: &str) -> Result<()> {
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
        return Err(AppError::DesktopLaunchFailed(result));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_providers_use_product_title_keywords() {
        assert_eq!(
            provider_title_keywords("chatgpt", "ChatGPT", "https://chatgpt.com/"),
            vec!["ChatGPT"]
        );
        assert_eq!(
            provider_title_keywords("doubao", "豆包", "https://www.doubao.com/chat/"),
            vec!["豆包"]
        );
    }

    #[test]
    fn custom_providers_fall_back_to_display_name_then_host() {
        assert_eq!(
            provider_title_keywords("myai", "My AI 助手", "https://chat.example.com/app"),
            vec!["My AI 助手"]
        );
        assert_eq!(
            provider_title_keywords("myai", "  ", "https://chat.example.com/app"),
            vec!["chat.example.com"]
        );
    }

    #[test]
    fn title_matching_is_case_insensitive_containment() {
        assert!(title_matches(
            "chatgpt: chat, work",
            &["ChatGPT".to_owned()]
        ));
        assert!(title_matches("新聊天 | 豆包", &["豆包".to_owned()]));
        assert!(!title_matches("gmail - inbox", &["Gemini".to_owned()]));
        assert!(!title_matches("anything", &[]));
    }

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
