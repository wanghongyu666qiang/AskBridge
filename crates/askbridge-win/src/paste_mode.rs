// Clipboard-paste dispatch target: locate an AI website or desktop-client window, bring it to
// the foreground, and synthesize exactly one Ctrl+V. Nothing is ever typed
// beyond that shortcut, no page content is read, and sending stays with the
// user. A provider-neutral UI Automation receipt verifies that the page added
// attachment structure after the paste.

use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use askbridge_core::{AppError, Result};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS, HANDLE, HWND, LPARAM, RECT,
    },
    Storage::Packaging::Appx::GetPackageFamilyName,
    System::{
        Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        Threading::{
            AttachThreadInput, GetCurrentThreadId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
    },
    UI::{
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_CONTROL,
            VK_MENU, keybd_event,
        },
        Shell::ShellExecuteW,
        WindowsAndMessaging::{
            BringWindowToTop, EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowRect,
            GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
            IsWindowVisible, SW_RESTORE, SW_SHOWNORMAL, SetForegroundWindow, ShowWindow,
            SwitchToThisWindow,
        },
    },
};

use crate::util::wide;

const S_OK: i32 = 0;
const S_FALSE: i32 = 1;
const RPC_E_CHANGED_MODE: i32 = 0x8001_0106u32 as i32;

/// Top-level window classes of the mainstream browsers AskBridge pastes into.
const BROWSER_WINDOW_CLASSES: [&str; 2] = ["Chrome_WidgetWin_1", "MozillaWindowClass"];
/// Virtual key for the letter V (windows-sys does not export letter VKs).
const VK_V: u16 = 0x56;
/// Time allowed for the target window to actually take the foreground.
const FOCUS_SETTLE: Duration = Duration::from_millis(200);
/// Time allowed for UI Automation focus state to settle after SetFocus.
const EDITOR_FOCUS_SETTLE: Duration = Duration::from_millis(100);
const PASTE_KEY_INTERVAL: Duration = Duration::from_millis(10);
const PASTE_RECEIPT_PROBE_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PasteReceiptScope {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PasteAttachmentReceipt {
    image_count: u32,
    group_count: u32,
    scope: PasteReceiptScope,
}

fn has_new_paste_attachment(
    baseline: PasteAttachmentReceipt,
    current: PasteAttachmentReceipt,
) -> bool {
    current.scope == baseline.scope
        && current.image_count > baseline.image_count
        && current.group_count > baseline.group_count
}

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

/// Restores and focuses the window, refusing to continue unless it really
/// owns the foreground so Ctrl+V can never land somewhere unexpected.
///
/// Windows denies foreground access to background processes. The escalation
/// ladder: plain SetForegroundWindow -> Alt held across the call (classic
/// permission workaround) -> AttachThreadInput to the current foreground and
/// target threads -> SwitchToThisWindow. Every step is verified; if none
/// takes the foreground the paste is refused rather than sent to a wrong
/// window.
pub(crate) fn activate(window: HWND) -> Result<()> {
    // SAFETY: window is a live top-level window on the calling thread's desktop.
    unsafe {
        if IsIconic(window) != 0 {
            ShowWindow(window, SW_RESTORE);
        }
    }
    try_focus(window);
    if focus_verified(window).is_ok() {
        return Ok(());
    }
    // Hold Alt down across SetForegroundWindow: the injected Alt makes the
    // process satisfy the foreground permission check.
    let alt_down = [keyboard_input(VK_MENU, 0)];
    let alt_up = [keyboard_input(VK_MENU, KEYEVENTF_KEYUP)];
    // SAFETY: Plain key arrays valid for these synchronous SendInput calls.
    unsafe {
        SendInput(1, alt_down.as_ptr(), size_of::<INPUT>() as i32);
    }
    try_focus(window);
    // SAFETY: See above; releases the held Alt key.
    unsafe {
        SendInput(1, alt_up.as_ptr(), size_of::<INPUT>() as i32);
    }
    focus_verified(window).or_else(|_| {
        force_focus(window);
        focus_verified(window).or_else(|_| {
            switch_to_this_window(window);
            focus_verified(window)
        })
    })
}

/// Makes a top-level target safe for a clipboard paste. Foreground ownership
/// alone is insufficient: browser chrome or the page body can own keyboard
/// focus, in which case Ctrl+V is accepted by SendInput but never reaches the
/// composer. This second step uses Windows UI Automation to focus the lowest
/// visible, enabled, non-password edit control in the content area.
///
/// The selector is provider-neutral and never reads control names, values, or
/// page text. If an editable control cannot be located and focus cannot be
/// verified, the paste is refused before any key is synthesized.
pub(crate) fn prepare_paste_target(window: HWND) -> Result<()> {
    activate(window)?;
    // A newly launched Chromium app can report itself as the foreground
    // window before its renderer accepts keyboard input. Reassert the app
    // switch, verify it again, and only then focus the unique editor.
    switch_to_this_window(window);
    focus_verified(window)?;
    focus_visible_editor(window)
}

pub(crate) fn paste_attachment_baseline(window: HWND) -> Result<PasteAttachmentReceipt> {
    read_paste_attachment_receipt(window, None)
}

pub(crate) fn wait_for_paste_attachment(
    window: HWND,
    baseline: PasteAttachmentReceipt,
    timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<bool> {
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        AppError::InvalidPreparation("paste receipt timeout is too large".to_owned())
    })?;
    let mut consecutive_receipts = 0u8;
    let mut transient_probe_errors = 0u32;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        let current = match read_paste_attachment_receipt(window, Some(baseline.scope)) {
            Ok(receipt) => receipt,
            Err(_) => {
                transient_probe_errors = transient_probe_errors.saturating_add(1);
                if Instant::now() >= deadline {
                    tracing::warn!(
                        stage = "paste_attachment_receipt",
                        completed = false,
                        transient_probe_errors,
                        "clipboard paste receipt remained unavailable until timeout"
                    );
                    return Ok(false);
                }
                thread::sleep(PASTE_RECEIPT_PROBE_INTERVAL);
                continue;
            }
        };
        if has_new_paste_attachment(baseline, current) {
            consecutive_receipts = consecutive_receipts.saturating_add(1);
            if consecutive_receipts >= 2 {
                tracing::info!(
                    stage = "paste_attachment_receipt",
                    completed = true,
                    "clipboard paste produced new attachment structure"
                );
                return Ok(true);
            }
        } else {
            consecutive_receipts = 0;
        }
        if Instant::now() >= deadline {
            tracing::warn!(
                stage = "paste_attachment_receipt",
                completed = false,
                "clipboard paste produced no verifiable attachment structure"
            );
            return Ok(false);
        }
        thread::sleep(PASTE_RECEIPT_PROBE_INTERVAL);
    }
}

fn read_paste_attachment_receipt(
    window: HWND,
    scope: Option<PasteReceiptScope>,
) -> Result<PasteAttachmentReceipt> {
    // SAFETY: This initializes COM only for the current browser worker/test
    // thread. A different existing apartment remains usable by UI Automation.
    let com_status =
        unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED as u32) };
    let should_uninitialize = com_status == S_OK || com_status == S_FALSE;
    if !should_uninitialize && com_status != RPC_E_CHANGED_MODE {
        return Err(AppError::PasteTargetUnavailable);
    }
    let result = read_paste_attachment_receipt_with_uia(window, scope);
    if should_uninitialize {
        // SAFETY: Balances the successful CoInitializeEx call above.
        unsafe { CoUninitialize() };
    }
    result
}

fn read_paste_attachment_receipt_with_uia(
    window: HWND,
    scope: Option<PasteReceiptScope>,
) -> Result<PasteAttachmentReceipt> {
    use windows::Win32::{
        Foundation::HWND as AutomationHwnd,
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        UI::Accessibility::{
            CUIAutomation, IUIAutomation, TreeScope_Descendants, UIA_EditControlTypeId,
            UIA_GroupControlTypeId, UIA_ImageControlTypeId,
        },
    };

    let mut window_rect = RECT::default();
    // SAFETY: window is a live top-level window and window_rect is writable.
    if unsafe { GetWindowRect(window, &mut window_rect) } == 0 {
        return Err(AppError::PasteTargetUnavailable);
    }
    let window_height = window_rect.bottom.saturating_sub(window_rect.top);
    if window_height <= 0 {
        return Err(AppError::PasteTargetUnavailable);
    }
    let result = (|| -> windows::core::Result<PasteAttachmentReceipt> {
        // SAFETY: CUIAutomation is an in-process COM server and COM is
        // initialized on this thread by the caller.
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? };
        let root = unsafe { automation.ElementFromHandle(AutomationHwnd(window))? };
        let scope = match scope {
            Some(scope) => scope,
            None => {
                // prepare_paste_target focused exactly one eligible editor.
                // Anchor the receipt to that editor's local composer band so
                // unrelated page rendering cannot satisfy the attachment check.
                let editor = unsafe { automation.GetFocusedElement()? };
                if unsafe { editor.CurrentControlType()? } != UIA_EditControlTypeId
                    || !unsafe { editor.CurrentIsKeyboardFocusable()? }.as_bool()
                    || !unsafe { editor.CurrentIsEnabled()? }.as_bool()
                    || unsafe { editor.CurrentIsPassword()? }.as_bool()
                    || unsafe { editor.CurrentIsOffscreen()? }.as_bool()
                {
                    return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                        0x8000_4005u32 as i32,
                    )));
                }
                let rect = unsafe { editor.CurrentBoundingRectangle()? };
                let width = rect.right.saturating_sub(rect.left);
                let height = rect.bottom.saturating_sub(rect.top);
                if width < 20 || height < 10 {
                    return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                        0x8000_4005u32 as i32,
                    )));
                }
                PasteReceiptScope {
                    left: rect.left.saturating_sub(64).max(window_rect.left),
                    top: rect.top.saturating_sub(320).max(window_rect.top),
                    right: rect.right.saturating_add(64).min(window_rect.right),
                    bottom: rect.bottom.saturating_add(96).min(window_rect.bottom),
                }
            }
        };
        let condition = unsafe { automation.CreateTrueCondition()? };
        let elements = unsafe { root.FindAll(TreeScope_Descendants, &condition)? };
        let length = unsafe { elements.Length()? };
        let mut receipt = PasteAttachmentReceipt {
            image_count: 0,
            group_count: 0,
            scope,
        };
        for index in 0..length {
            let element = unsafe { elements.GetElement(index)? };
            if unsafe { element.CurrentIsOffscreen()? }.as_bool() {
                continue;
            }
            let rect = unsafe { element.CurrentBoundingRectangle()? };
            let intersects_content = rect.right > scope.left
                && rect.left < scope.right
                && rect.bottom > scope.top
                && rect.top < scope.bottom
                && rect.right > rect.left
                && rect.bottom > rect.top;
            if !intersects_content {
                continue;
            }
            let control_type = unsafe { element.CurrentControlType()? };
            if control_type == UIA_ImageControlTypeId {
                receipt.image_count = receipt.image_count.saturating_add(1);
            } else if control_type == UIA_GroupControlTypeId {
                receipt.group_count = receipt.group_count.saturating_add(1);
            }
        }
        Ok(receipt)
    })();
    result.map_err(|_| AppError::PasteTargetUnavailable)
}

fn focus_visible_editor(window: HWND) -> Result<()> {
    // SAFETY: This initializes COM only for the current browser worker/test
    // thread. An existing apartment with a different model is still usable by
    // UI Automation and must not be uninitialized here.
    let com_status =
        unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED as u32) };
    let should_uninitialize = com_status == S_OK || com_status == S_FALSE;
    if !should_uninitialize && com_status != RPC_E_CHANGED_MODE {
        tracing::warn!(
            stage = "paste_editor_focus",
            completed = false,
            hresult = format_args!("0x{:08X}", com_status as u32),
            "COM initialization failed before editor discovery"
        );
        return Err(AppError::PasteTargetUnavailable);
    }

    let result = focus_visible_editor_with_uia(window);
    if should_uninitialize {
        // SAFETY: Balances the successful CoInitializeEx call above.
        unsafe { CoUninitialize() };
    }
    result
}

fn focus_visible_editor_with_uia(window: HWND) -> Result<()> {
    use windows::Win32::{
        Foundation::HWND as AutomationHwnd,
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance},
        UI::Accessibility::{
            CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Descendants,
            UIA_EditControlTypeId,
        },
    };

    let mut window_rect = RECT::default();
    // SAFETY: window is a live top-level window and window_rect is writable.
    if unsafe { GetWindowRect(window, &mut window_rect) } == 0 {
        return Err(AppError::PasteTargetUnavailable);
    }
    let window_height = window_rect.bottom.saturating_sub(window_rect.top);
    if window_height <= 0 {
        return Err(AppError::PasteTargetUnavailable);
    }
    // Ignore browser chrome (address/search bars) without relying on a
    // provider, title, DOM selector, locale, or executable-specific rule.
    let content_top = window_rect.top.saturating_add(window_height / 4);

    let result = (|| -> windows::core::Result<()> {
        // SAFETY: CUIAutomation is an in-process COM server and COM has been
        // initialized on this thread by the caller.
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? };
        let root = unsafe { automation.ElementFromHandle(AutomationHwnd(window))? };
        let condition = unsafe { automation.CreateTrueCondition()? };
        let elements = unsafe { root.FindAll(TreeScope_Descendants, &condition)? };
        let length = unsafe { elements.Length()? };

        let mut selected: Option<IUIAutomationElement> = None;
        let mut eligible_count = 0u32;
        for index in 0..length {
            let element = unsafe { elements.GetElement(index)? };
            if unsafe { element.CurrentControlType()? } != UIA_EditControlTypeId
                || !unsafe { element.CurrentIsKeyboardFocusable()? }.as_bool()
                || !unsafe { element.CurrentIsEnabled()? }.as_bool()
                || unsafe { element.CurrentIsPassword()? }.as_bool()
                || unsafe { element.CurrentIsOffscreen()? }.as_bool()
            {
                continue;
            }
            let rect = unsafe { element.CurrentBoundingRectangle()? };
            let width = rect.right.saturating_sub(rect.left);
            let height = rect.bottom.saturating_sub(rect.top);
            let center_y = rect.top.saturating_add(height / 2);
            let intersects_window = rect.right > window_rect.left
                && rect.left < window_rect.right
                && rect.bottom > window_rect.top
                && rect.top < window_rect.bottom;
            if width < 20 || height < 10 || !intersects_window || center_y < content_top {
                continue;
            }
            eligible_count = eligible_count.saturating_add(1);
            selected = Some(element);
        }

        if eligible_count != 1 {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x8000_4005u32 as i32,
            )));
        }
        let Some(editor) = selected else {
            unreachable!("exactly one eligible editor must have been selected")
        };
        unsafe { editor.SetFocus()? };
        thread::sleep(EDITOR_FOCUS_SETTLE);
        let focused = unsafe { automation.GetFocusedElement()? };
        if !unsafe { automation.CompareElements(&editor, &focused)? }.as_bool()
            && !unsafe { editor.CurrentHasKeyboardFocus()? }.as_bool()
        {
            return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
                0x8000_4005u32 as i32,
            )));
        }
        tracing::info!(
            stage = "paste_editor_focus",
            completed = true,
            "visible editable control focused for clipboard paste"
        );
        Ok(())
    })();

    result.map_err(|error| {
        tracing::warn!(
            stage = "paste_editor_focus",
            completed = false,
            hresult = format_args!("0x{:08X}", error.code().0 as u32),
            "no safe editable control could be focused; clipboard paste refused"
        );
        AppError::PasteTargetUnavailable
    })
}

/// Legacy forced window switch used as the last activation resort.
fn switch_to_this_window(window: HWND) {
    // SAFETY: window is a live top-level window; the call is synchronous and
    // best-effort, and callers verify the outcome afterwards.
    unsafe {
        SwitchToThisWindow(window, 1);
    }
}

/// Last-resort foreground takeover by sharing input state with the threads
/// that currently own it.
fn force_focus(window: HWND) {
    // SAFETY: window is a live top-level window; all calls are synchronous
    // and every AttachThreadInput is balanced before returning.
    unsafe {
        let this_thread = GetCurrentThreadId();
        let foreground = GetForegroundWindow();
        let mut pid = 0u32;
        let fg_thread = if foreground.is_null() {
            0
        } else {
            GetWindowThreadProcessId(foreground, &mut pid)
        };
        let window_thread = GetWindowThreadProcessId(window, &mut pid);
        let attached_foreground = fg_thread != 0
            && fg_thread != this_thread
            && AttachThreadInput(this_thread, fg_thread, 1) != 0;
        let attached_window = window_thread != 0
            && window_thread != this_thread
            && AttachThreadInput(this_thread, window_thread, 1) != 0;
        BringWindowToTop(window);
        ShowWindow(window, SW_SHOWNORMAL);
        SetForegroundWindow(window);
        if attached_foreground {
            AttachThreadInput(this_thread, fg_thread, 0);
        }
        if attached_window {
            AttachThreadInput(this_thread, window_thread, 0);
        }
    }
}

/// Returns true only when `window` currently owns the foreground.
fn focus_verified(window: HWND) -> Result<()> {
    thread::sleep(FOCUS_SETTLE);
    // SAFETY: Reading the foreground window is side-effect free.
    let foreground = unsafe { GetForegroundWindow() };
    if foreground == window {
        Ok(())
    } else {
        if !foreground.is_null() {
            // SAFETY: foreground is a live window; class_buffer receives at
            // most its length. Only the window class is logged, not content.
            let mut class_buffer = [0u16; 256];
            let class_length = unsafe {
                GetClassNameW(
                    foreground,
                    class_buffer.as_mut_ptr(),
                    class_buffer.len() as i32,
                )
            };
            let class = String::from_utf16_lossy(
                &class_buffer[..class_length.clamp(0, class_buffer.len() as i32) as usize],
            );
            tracing::warn!(
                stage = "paste_foreground_denied",
                completed = false,
                foreground_class = %class,
                "target window did not take the foreground"
            );
        }
        Err(AppError::PasteTargetUnavailable)
    }
}

fn try_focus(window: HWND) {
    // SAFETY: window is a live top-level window; SetForegroundWindow may be
    // rejected by the foreground lock, which callers handle by retrying.
    unsafe {
        SetForegroundWindow(window);
    }
}

/// Synthesizes one Ctrl+V keystroke sequence.
pub(crate) fn send_paste() -> Result<()> {
    let inputs = paste_inputs();
    for (index, input) in inputs.iter().enumerate() {
        // Chromium PWAs can acknowledge SendInput while dropping its batched
        // shortcut during renderer startup. keybd_event queues the same trusted
        // system key transitions one by one; the attachment receipt remains the
        // authority for whether the paste actually reached the page.
        let keyboard = unsafe { input.Anonymous.ki };
        unsafe { keybd_event(keyboard.wVk as u8, 0, keyboard.dwFlags, 0) };
        if index + 1 < inputs.len() {
            thread::sleep(PASTE_KEY_INTERVAL);
        }
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
///
/// ShellExecuteW resolves protocol handlers through COM, so the calling
/// thread must have an apartment initialized; the browser worker thread has
/// none, which made this call fail outright until the explicit init below.
pub(crate) fn open_default_browser(url: &str) -> Result<()> {
    // SAFETY: CoInitializeEx on a worker thread with no apartment yet is the
    // documented use; S_FALSE just means an apartment already existed.
    let com_initialized =
        unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED as u32) };
    if com_initialized != S_OK && com_initialized != S_FALSE {
        return Err(AppError::Windows {
            operation: "CoInitializeEx(clipboard paste)",
            win32_code: com_initialized as u32,
        });
    }
    let result = open_url_via_shell(url);
    // SAFETY: Every successful CoInitializeEx call, including S_FALSE, must
    // be balanced on this thread.
    unsafe {
        CoUninitialize();
    }
    result
}

fn open_url_via_shell(url: &str) -> Result<()> {
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
        // Numeric shell result only: keeps the log within the privacy
        // boundary while pinpointing why protocol launch was rejected.
        tracing::warn!(
            stage = "paste_open_default_browser",
            completed = false,
            shell_result = result,
            "default browser launch was rejected"
        );
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

    #[test]
    fn paste_receipt_requires_an_image_and_group_in_the_same_scope() {
        let scope = PasteReceiptScope {
            left: 10,
            top: 20,
            right: 300,
            bottom: 400,
        };
        let baseline = PasteAttachmentReceipt {
            image_count: 2,
            group_count: 3,
            scope,
        };
        assert!(!has_new_paste_attachment(baseline, baseline));
        assert!(!has_new_paste_attachment(
            baseline,
            PasteAttachmentReceipt {
                image_count: 3,
                ..baseline
            }
        ));
        assert!(!has_new_paste_attachment(
            baseline,
            PasteAttachmentReceipt {
                group_count: 4,
                ..baseline
            }
        ));
        assert!(has_new_paste_attachment(
            baseline,
            PasteAttachmentReceipt {
                image_count: 3,
                group_count: 4,
                ..baseline
            }
        ));
        assert!(!has_new_paste_attachment(
            baseline,
            PasteAttachmentReceipt {
                image_count: 3,
                group_count: 4,
                scope: PasteReceiptScope { left: 11, ..scope },
            }
        ));
    }
}
