//! Locates candidate paste targets: browser-class windows whose titles match
//! provider keywords, filtered by owning process identity.

use std::path::Path;

use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HANDLE, HWND, LPARAM},
    Storage::Packaging::Appx::GetPackageFamilyName,
    System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    },
    UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible,
    },
};

/// Top-level window classes of the mainstream browsers AskBridge pastes into.
const BROWSER_WINDOW_CLASSES: [&str; 2] = ["Chrome_WidgetWin_1", "MozillaWindowClass"];

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
#[cfg(test)]
fn title_matches(title_lower: &str, keywords: &[String]) -> bool {
    title_match_index(title_lower, keywords).is_some()
}

/// Index of the first keyword contained in the lowercased window title.
fn title_match_index(title_lower: &str, keywords: &[String]) -> Option<usize> {
    keywords
        .iter()
        .position(|keyword| title_lower.contains(&keyword.to_lowercase()))
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
    found: Vec<ProviderWindow>,
}

/// Finds visible top-level browser or supported AI desktop windows whose title
/// contains one of the provider keywords. EnumWindows yields them in Z order.
pub(crate) fn find_provider_windows(keywords: &[String]) -> Vec<ProviderWindow> {
    if keywords.is_empty() {
        return Vec::new();
    }
    let mut search = WindowSearch {
        keywords: keywords.to_vec(),
        found: Vec::new(),
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

/// A located provider window plus the evidence that identified it.
pub(crate) struct ProviderWindow {
    pub hwnd: HWND,
    pub class: String,
    pub process: String,
    pub keyword_index: usize,
}

/// Executable names accepted as real browser hosts. Chromium-based desktop
/// apps share the Chrome window class, so the owning process distinguishes
/// supported AI clients from unrelated Electron applications.
const BROWSER_PROCESSES: [&str; 6] = [
    "chrome.exe",
    "msedge.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "vivaldi.exe",
];
const AI_DESKTOP_PROCESSES: [&str; 3] = ["chatgpt.exe", "claude.exe", "doubao.exe"];

fn is_allowed_paste_process(process: &str, package_family: Option<&str>) -> bool {
    if BROWSER_PROCESSES.contains(&process) {
        return true;
    }
    AI_DESKTOP_PROCESSES.contains(&process)
        && !package_family.is_some_and(|family| family.contains("codex"))
}

struct ProcessIdentity {
    executable_name: String,
    package_family: Option<String>,
}

fn window_process_identity(window: HWND) -> Option<ProcessIdentity> {
    let mut pid = 0u32;
    // SAFETY: window is valid and pid receives the owning process id.
    unsafe {
        GetWindowThreadProcessId(window, &mut pid);
    }
    if pid == 0 {
        return None;
    }
    // SAFETY: A query-limited handle is the least privilege needed here and
    // is closed before returning.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut name_buffer = [0u16; 1024];
    let mut length = name_buffer.len() as u32;
    // SAFETY: process is live and name_buffer/length describe writable space.
    let written =
        unsafe { QueryFullProcessImageNameW(process, 0, name_buffer.as_mut_ptr(), &mut length) };
    let package_family = package_family_name(process);
    // SAFETY: The handle was opened by this function and is no longer needed.
    unsafe {
        CloseHandle(process);
    }
    if written == 0 || length as usize > name_buffer.len() {
        return None;
    }
    let full_path = String::from_utf16_lossy(&name_buffer[..length as usize]);
    let executable_name = Path::new(&full_path)
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())?;
    Some(ProcessIdentity {
        executable_name,
        package_family,
    })
}

fn package_family_name(process: HANDLE) -> Option<String> {
    let mut length = 0u32;
    // SAFETY: The first call intentionally supplies no buffer so Windows
    // reports the required package-family length for packaged processes.
    let status = unsafe { GetPackageFamilyName(process, &mut length, std::ptr::null_mut()) };
    if status != ERROR_INSUFFICIENT_BUFFER || length == 0 {
        return None;
    }
    let mut buffer = vec![0u16; length as usize];
    // SAFETY: buffer has exactly the size requested by the first call.
    let status = unsafe { GetPackageFamilyName(process, &mut length, buffer.as_mut_ptr()) };
    if status != ERROR_SUCCESS {
        return None;
    }
    if buffer.last() == Some(&0) {
        buffer.pop();
    }
    Some(String::from_utf16_lossy(&buffer).to_lowercase())
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
    // Desktop clients share the Chrome class; process identity prevents an
    // unrelated Electron application from receiving the synthetic paste.
    let Some(identity) = window_process_identity(window) else {
        tracing::warn!(
            stage = "paste_window_skipped",
            completed = false,
            window_class = %class,
            "browser-class window skipped; owning process could not be identified"
        );
        return 1;
    };
    if !is_allowed_paste_process(
        &identity.executable_name,
        identity.package_family.as_deref(),
    ) {
        tracing::info!(
            stage = "paste_window_skipped",
            completed = false,
            window_class = %class,
            process = %identity.executable_name,
            "unsupported host ignored for clipboard paste"
        );
        return 1;
    }
    // SAFETY: window is valid and title_buffer receives at most its length.
    let mut title_buffer = [0u16; 512];
    let mut title_length =
        unsafe { GetWindowTextW(window, title_buffer.as_mut_ptr(), title_buffer.len() as i32) };
    let mut title = String::from_utf16_lossy(
        &title_buffer[..title_length.clamp(0, title_buffer.len() as i32) as usize],
    );
    if title_length == title_buffer.len() as i32 {
        // The title filled the probe buffer: ask for the real length and
        // re-read once instead of silently truncating (a keyword past the cut
        // would never match). The cap keeps an absurd title from forcing a
        // large allocation inside this enumeration callback.
        const MAX_TITLE_CHARS: usize = 4096;
        // SAFETY: window is valid; the call only reports the title length.
        let reported = unsafe { GetWindowTextLengthW(window) };
        let capacity = usize::try_from(reported).map(|length| length.saturating_add(1));
        if let Ok(capacity) = capacity
            && capacity <= MAX_TITLE_CHARS
        {
            let mut full_buffer = vec![0u16; capacity];
            title_length = unsafe {
                GetWindowTextW(window, full_buffer.as_mut_ptr(), full_buffer.len() as i32)
            };
            title = String::from_utf16_lossy(
                &full_buffer[..title_length.clamp(0, full_buffer.len() as i32) as usize],
            );
        }
    }
    if let Some(keyword_index) = title_match_index(&title.to_lowercase(), &search.keywords) {
        search.found.push(ProviderWindow {
            hwnd: window,
            class,
            process: identity.executable_name,
            keyword_index,
        });
    }
    1
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
    fn paste_process_allowlist_accepts_browsers_and_ai_desktop_clients() {
        assert!(is_allowed_paste_process("chrome.exe", None));
        assert!(is_allowed_paste_process("firefox.exe", None));
        assert!(is_allowed_paste_process(
            "chatgpt.exe",
            Some("openai.chatgpt_123")
        ));
        assert!(is_allowed_paste_process("claude.exe", None));
        assert!(is_allowed_paste_process("doubao.exe", None));
        assert!(!is_allowed_paste_process(
            "chatgpt.exe",
            Some("openai.codex_2p2nqsd0c76g0")
        ));
        assert!(!is_allowed_paste_process("discord.exe", None));
        assert!(!is_allowed_paste_process("unknown.exe", None));
    }
}
