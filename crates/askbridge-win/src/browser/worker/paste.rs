//! Clipboard-paste delivery: screenshot to clipboard, locate and focus the
//! provider target, then synthesize exactly one Ctrl+V and verify the
//! provider-neutral attachment receipt.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, atomic::AtomicBool, atomic::Ordering},
    thread,
    time::{Duration, Instant},
};

use askbridge_core::{
    AppError, PreparationFailureStage, PreparationOutcome, PreparationRecovery, Result,
};
use windows_sys::Win32::Foundation::HWND;

use super::jobs::{BrowserSurface, ClipboardPasteJob, ClipboardPasteOpenTarget};
use super::service::send_event;

/// Time budget for a cold-opened provider page to expose a stable editor.
const COLD_OPEN_EDITOR_STABILITY_TIMEOUT: Duration = Duration::from_secs(7);
const COLD_OPEN_EDITOR_MIN_SETTLE: Duration = Duration::from_secs(2);
const COLD_OPEN_EDITOR_STABILITY_INTERVAL: Duration = Duration::from_millis(100);
const COLD_OPEN_EDITOR_STABILITY_SAMPLES: u8 = 2;

/// Clipboard-paste target: no CDP at all. Writes the screenshot to the
/// clipboard, then locates a matching provider page or supported desktop
/// client (opening a page if needed) and synthesizes a single Ctrl+V into it.
pub(super) fn prepare_clipboard_paste_job(
    owner: usize,
    events: &Arc<Mutex<VecDeque<super::service::BrowserEvent>>>,
    cancelled: &AtomicBool,
    dispatch: &super::jobs::BrowserJob,
    job: &ClipboardPasteJob,
) -> Result<()> {
    let request = &dispatch.request;
    let Some(image) = request.image.as_ref() else {
        return Err(AppError::InvalidDispatchRequest(
            "clipboard paste mode requires a captured screenshot".to_owned(),
        ));
    };
    crate::clipboard_image::copy_image_to_clipboard(owner as HWND, image)?;

    let deadline = Instant::now() + job.locate_timeout;
    let mut page_opened = false;
    if let ClipboardPasteOpenTarget::DesktopPwa {
        provider_id,
        configured_shortcut,
    } = &job.open_target
    {
        // An explicit PWA preference must bring that app to the front before
        // enumerating title matches. Otherwise an already-open provider tab
        // in a normal browser can win the Z-order search and receive Ctrl+V.
        crate::browser::DesktopPwaLauncher::open(provider_id, configured_shortcut.as_deref())?;
        page_opened = true;
    }
    let mut cold_open_editor_settled = false;
    let mut activation_error = None;
    'locate: loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        for window in crate::paste_mode::find_provider_windows(&job.title_keywords) {
            tracing::info!(
                stage = "paste_window_found",
                completed = true,
                window_class = %window.class,
                process = %window.process,
                keyword_index = window.keyword_index,
                "provider window located for clipboard paste"
            );
            let mut preparation = crate::paste_mode::prepare_paste_target(window.hwnd);
            if preparation.is_ok() && page_opened && !cold_open_editor_settled {
                wait_for_cold_open_editor_settle(window.hwnd, cancelled)?;
                cold_open_editor_settled = true;
                preparation = crate::paste_mode::prepare_paste_target(window.hwnd);
            }
            match preparation
                .and_then(|()| crate::paste_mode::paste_attachment_baseline(window.hwnd))
            {
                Ok(baseline) => {
                    // A cold provider page can re-render the composer while
                    // UI Automation walks the tree for the baseline. Re-focus
                    // and re-verify the exact target immediately before the
                    // only input injection; failure here is still pre-write.
                    if let Err(error) = crate::paste_mode::prepare_paste_target(window.hwnd) {
                        tracing::warn!(
                            stage = "paste_target_focus_changed",
                            completed = false,
                            window_class = %window.class,
                            process = %window.process,
                            "paste target changed while capturing the attachment baseline"
                        );
                        activation_error = Some(error);
                        continue;
                    }
                    // Do not retry a partial SendInput on another target: the
                    // first target may already have received part of Ctrl+V.
                    crate::paste_mode::send_paste()?;
                    let verified = crate::paste_mode::wait_for_paste_attachment(
                        window.hwnd,
                        baseline,
                        Duration::from_secs(5),
                        cancelled,
                    )
                    .map_err(|error| match error {
                        AppError::BrowserCancelled => error,
                        _ => clipboard_paste_verification_failed(),
                    })?;
                    if !verified {
                        return Err(clipboard_paste_verification_failed());
                    }
                    break 'locate;
                }
                // The located window may be on another virtual desktop or
                // otherwise unactivatable. Try every matching candidate
                // before falling through to a fresh page on this desktop.
                Err(error) => {
                    tracing::warn!(
                        stage = "paste_target_not_ready",
                        completed = false,
                        window_class = %window.class,
                        process = %window.process,
                        "located window could not expose a focused editor; trying another candidate"
                    );
                    activation_error = Some(error);
                }
            }
        }
        if !page_opened {
            page_opened = true;
            tracing::info!(
                stage = "paste_window_not_found",
                completed = false,
                "no paste-ready provider window yet; opening the configured target"
            );
            match &job.open_target {
                ClipboardPasteOpenTarget::DefaultBrowser => {
                    crate::paste_mode::open_default_browser(&job.start_url)?;
                }
                ClipboardPasteOpenTarget::DesktopPwa {
                    provider_id,
                    configured_shortcut,
                } => {
                    crate::browser::DesktopPwaLauncher::open(
                        provider_id,
                        configured_shortcut.as_deref(),
                    )?;
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(activation_error.unwrap_or(AppError::PasteTargetUnavailable));
        }
        thread::sleep(Duration::from_millis(250));
    }

    // validate_for is intentionally skipped: paste mode delivers only the
    // screenshot, so quick-dispatch prompts are deliberately not filled in.
    let outcome = PreparationOutcome::prepared(&job.start_url, false, true);
    send_event(
        owner,
        events,
        super::service::BrowserEvent::Prepared {
            request_id: request.id.clone(),
            surface: BrowserSurface::ClipboardPaste,
            outcome,
        },
    );
    Ok(())
}

fn wait_for_cold_open_editor_settle(window: HWND, cancelled: &AtomicBool) -> Result<()> {
    let started = Instant::now();
    let deadline = Instant::now() + COLD_OPEN_EDITOR_STABILITY_TIMEOUT;
    let mut previous = None;
    let mut consecutive_stable_samples = 0u8;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }

        let sample = crate::paste_mode::prepare_paste_target(window)
            .and_then(|()| crate::paste_mode::paste_attachment_baseline(window));
        if let Ok(current) = sample {
            if previous == Some(current) {
                consecutive_stable_samples = consecutive_stable_samples.saturating_add(1);
            } else {
                consecutive_stable_samples = 1;
                previous = Some(current);
            }
        } else {
            // Cold Chromium pages can replace the accessibility subtree while
            // they hydrate. This is still pre-write, so discard the sample and
            // keep probing within the bounded readiness budget.
            previous = None;
            consecutive_stable_samples = 0;
        }
        if consecutive_stable_samples >= COLD_OPEN_EDITOR_STABILITY_SAMPLES
            && started.elapsed() >= COLD_OPEN_EDITOR_MIN_SETTLE
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(AppError::PasteTargetUnavailable);
        }
        thread::sleep(COLD_OPEN_EDITOR_STABILITY_INTERVAL);
    }
}

fn clipboard_paste_verification_failed() -> AppError {
    // Ctrl+V was already synthesized. Treat attachment state as potentially
    // prepared so no caller can safely paste the screenshot a second time.
    AppError::PreparationFailed {
        stage: PreparationFailureStage::Verification,
        recovery: PreparationRecovery::Retry,
        text_inserted: false,
        attachment_prepared: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertain_clipboard_receipt_is_marked_as_a_possible_attachment_write() {
        assert!(matches!(
            clipboard_paste_verification_failed(),
            AppError::PreparationFailed {
                stage: PreparationFailureStage::Verification,
                recovery: PreparationRecovery::Retry,
                text_inserted: false,
                attachment_prepared: true,
            }
        ));
    }

    #[test]
    fn cold_open_editor_settle_honours_cancellation_before_waiting() {
        let cancelled = AtomicBool::new(true);
        assert!(matches!(
            wait_for_cold_open_editor_settle(0 as HWND, &cancelled),
            Err(AppError::BrowserCancelled)
        ));
    }
}
