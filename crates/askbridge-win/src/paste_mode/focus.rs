//! Brings the chosen window to the foreground and focuses exactly one safe
//! edit control, refusing to continue when ownership cannot be proven.

use std::{thread, time::Duration};

use askbridge_core::{AppError, Result};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    System::{
        Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        Threading::{AttachThreadInput, GetCurrentThreadId},
    },
    UI::{
        Input::KeyboardAndMouse::{INPUT, KEYEVENTF_KEYUP, SendInput, VK_MENU},
        WindowsAndMessaging::{
            BringWindowToTop, GetClassNameW, GetForegroundWindow, GetWindowRect,
            GetWindowThreadProcessId, IsIconic, SW_RESTORE, SW_SHOWNORMAL, SetForegroundWindow,
            ShowWindow, SwitchToThisWindow,
        },
    },
};

use super::keystroke::keyboard_input;
use super::{S_FALSE, S_OK, RPC_E_CHANGED_MODE};

/// Time allowed for the target window to actually take the foreground.
const FOCUS_SETTLE: Duration = Duration::from_millis(200);
/// Time allowed for UI Automation focus state to settle after SetFocus.
const EDITOR_FOCUS_SETTLE: Duration = Duration::from_millis(100);

/// Restores and focuses the window, refusing to continue unless it really
/// owns the foreground so Ctrl+V can never land somewhere unexpected.
///
/// Windows denies foreground access to background processes. The escalation
/// ladder: plain SetForegroundWindow -> Alt held across the call (classic
/// permission workaround) -> AttachThreadInput to the current foreground and
/// target threads -> SwitchToThisWindow. Every step is verified; if none
/// takes the foreground the paste is refused rather than sent to a wrong
/// window.
fn activate(window: HWND) -> Result<()> {
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
