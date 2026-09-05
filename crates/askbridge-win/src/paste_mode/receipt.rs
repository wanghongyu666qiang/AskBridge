//! Reads the provider-neutral UI Automation attachment receipt that verifies
//! whether a synthesized paste produced new attachment structure.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use askbridge_core::{AppError, Result};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
    UI::WindowsAndMessaging::GetWindowRect,
};

use super::{RPC_E_CHANGED_MODE, S_FALSE, S_OK};

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

#[cfg(test)]
mod tests {
    use super::*;

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
