//! Prepare pipeline: warmup, provider health batch, dedicated-Chrome CDP
//! attempts, and the fail-closed decision to fall back to clipboard paste.

use std::{
    collections::VecDeque,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use askbridge_core::{
    AppError, BrowserTarget, FocusEvidence, Result, TargetDecision, TargetResolver,
};

use super::jobs::{
    AttemptDeadline, BrowserJob, BrowserLaunch, BrowserStage, BrowserSurface, BrowserWarmupJob,
    DedicatedChromeJob, ProviderHealthJob,
};
use super::paste::prepare_clipboard_paste_job;
use super::service::{BrowserEvent, send_event, send_stage};
use crate::adapter::{
    GenericProviderAdapter, PageSession, ProviderAdapter, ProviderHealthReport,
    check_provider_health,
};
use crate::browser::{
    CdpClient, CdpTarget, ChromeInstallation, ChromeManager, DesktopPwaLauncher, DevToolsEndpoint,
    ManagedProfile,
};

pub(super) fn warmup_browser(
    cancelled: &AtomicBool,
    manager: &mut Option<ChromeManager>,
    data_root: &Path,
    job: &BrowserWarmupJob,
) -> Result<CdpClient> {
    if manager.is_none() {
        let installation = ChromeInstallation::discover(job.configured_chrome_path.as_deref())?;
        let profile = ManagedProfile::open(&job.profile_dir, data_root)?;
        *manager = Some(ChromeManager::new(installation, profile));
    }
    let manager = manager.as_mut().ok_or_else(|| {
        AppError::BrowserConnectionFailed("browser manager initialization failed".to_owned())
    })?;
    let endpoint = manager.launch_and_wait(job.connect_timeout, cancelled)?;
    if manager.managed_process_id().is_none() {
        return Err(AppError::BrowserConnectionFailed(
            "browser process ownership could not be confirmed".to_owned(),
        ));
    }
    let client = connect_with_one_retry(endpoint, job.connect_timeout, cancelled)?;
    if let Some(url) = &job.open_url {
        let target = client.create_target(url)?;
        client.activate_target(&target.id)?;
        client.wait_until_ready(&target, job.page_timeout, cancelled)?;
    }
    Ok(client)
}

pub(super) fn check_provider_batch(
    cancelled: &AtomicBool,
    manager: &mut Option<ChromeManager>,
    data_root: &Path,
    job: &ProviderHealthJob,
) -> Result<(CdpClient, Vec<ProviderHealthReport>)> {
    let warmup = BrowserWarmupJob {
        configured_chrome_path: job.configured_chrome_path.clone(),
        profile_dir: job.profile_dir.clone(),
        connect_timeout: job.connect_timeout,
        page_timeout: job.page_timeout,
        open_url: None,
    };
    let client = warmup_browser(cancelled, manager, data_root, &warmup)?;
    let mut reports = Vec::with_capacity(job.providers.len());
    for check in &job.providers {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        let report = (|| -> Result<ProviderHealthReport> {
            let target = client
                .list_targets()?
                .into_iter()
                .find(|target| {
                    target.kind == "page"
                        && askbridge_core::matches_any_pattern(&target.url, &check.url_patterns)
                })
                .map_or_else(|| client.create_target(&check.start_url), Ok)?;
            client.activate_target(&target.id)?;
            client.wait_until_ready(&target, job.page_timeout, cancelled)?;
            let target = client
                .list_targets()?
                .into_iter()
                .find(|candidate| candidate.id == target.id)
                .ok_or_else(|| {
                    AppError::BrowserProtocol(
                        "provider page disappeared during the health check".to_owned(),
                    )
                })?;
            check_provider_health(&client, &target, check, cancelled, job.page_timeout)
        })()
        .unwrap_or_else(|_| ProviderHealthReport::network_error(&check.provider_id));
        reports.push(report);
    }
    Ok((client, reports))
}

pub(super) fn prepare_browser_job(
    owner: usize,
    events: &Arc<Mutex<VecDeque<BrowserEvent>>>,
    cancelled: &AtomicBool,
    manager: &mut Option<ChromeManager>,
    data_root: &Path,
    job: &BrowserJob,
) -> Result<Option<CdpClient>> {
    match prepare_browser_attempt(
        owner,
        events,
        cancelled,
        manager,
        data_root,
        job,
        job.plan.primary(),
    ) {
        Ok(client) => Ok(client),
        Err(primary_failure) => {
            let Some(fallback) = job.plan.fallback() else {
                return Err(primary_failure.error);
            };
            if !automatic_clipboard_fallback_allowed(&primary_failure) {
                tracing::warn!(
                    stage = "browser_fallback_suppressed",
                    completed = false,
                    error_kind = primary_failure.error.kind(),
                    provider_preparation_started = primary_failure.provider_preparation_started,
                    "automatic clipboard fallback was suppressed to avoid duplicate insertion"
                );
                return Err(primary_failure.error);
            }
            tracing::warn!(
                stage = "browser_fallback_started",
                completed = false,
                error_kind = primary_failure.error.kind(),
                "managed browser preparation failed safely; starting clipboard fallback"
            );
            send_event(
                owner,
                events,
                BrowserEvent::FallbackStarted {
                    request_id: job.request.id.clone(),
                    from: BrowserSurface::DedicatedChrome,
                    to: BrowserSurface::ClipboardPaste,
                },
            );
            prepare_browser_attempt(owner, events, cancelled, manager, data_root, job, fallback)
                .map_err(|failure| failure.error)
        }
    }
}

pub(super) struct BrowserAttemptFailure {
    pub error: AppError,
    pub provider_preparation_started: bool,
}

fn prepare_browser_attempt(
    owner: usize,
    events: &Arc<Mutex<VecDeque<BrowserEvent>>>,
    cancelled: &AtomicBool,
    manager: &mut Option<ChromeManager>,
    data_root: &Path,
    job: &BrowserJob,
    launch: &BrowserLaunch,
) -> std::result::Result<Option<CdpClient>, BrowserAttemptFailure> {
    if let BrowserLaunch::DedicatedChrome(dedicated) = launch {
        return prepare_dedicated_browser_job(
            owner, events, cancelled, manager, data_root, job, dedicated,
        )
        .map(Some);
    }

    let mut provider_preparation_started = false;
    let result = (|| -> Result<Option<CdpClient>> {
        match launch {
            BrowserLaunch::DedicatedChrome(_) => unreachable!("handled above"),
            BrowserLaunch::DesktopPwa(desktop) => {
                if cancelled.load(Ordering::Acquire) {
                    return Err(AppError::BrowserCancelled);
                }
                DesktopPwaLauncher::open(
                    &desktop.provider_id,
                    desktop.configured_shortcut.as_deref(),
                )?;
                let adapter = GenericProviderAdapter::for_provider(
                    &desktop.provider_id,
                    job.adapter_override.as_deref(),
                    desktop.url_patterns.clone(),
                )?;
                if adapter.id() != job.request.provider_id {
                    return Err(AppError::InvalidPreparation(
                        "adapter provider did not match dispatch request".to_owned(),
                    ));
                }
                let mut page = PageSession::DesktopPwa {
                    target_url: &desktop.start_url,
                };
                provider_preparation_started = true;
                let outcome = adapter.prepare(&mut page, &job.request, &job.policy)?;
                outcome.validate_for(&job.request)?;
                send_event(
                    owner,
                    events,
                    BrowserEvent::Prepared {
                        request_id: job.request.id.clone(),
                        surface: BrowserSurface::DesktopPwa,
                        outcome,
                    },
                );
                Ok(None)
            }
            BrowserLaunch::ClipboardPaste(paste) => {
                provider_preparation_started = true;
                prepare_clipboard_paste_job(owner, events, cancelled, job, paste)?;
                Ok(None)
            }
        }
    })();
    result.map_err(|error| BrowserAttemptFailure {
        error,
        provider_preparation_started,
    })
}

fn automatic_clipboard_fallback_allowed(failure: &BrowserAttemptFailure) -> bool {
    match &failure.error {
        AppError::BrowserCancelled => false,
        AppError::PreparationFailed {
            text_inserted: false,
            attachment_prepared: false,
            ..
        } => true,
        AppError::BrowserLaunchFailed
        | AppError::ChromeNotFound
        | AppError::BrowserProfileRejected(_)
        | AppError::BrowserProfileInUse
        | AppError::BrowserEndpointUnavailable
        | AppError::BrowserConnectionFailed(_)
        | AppError::BrowserProtocol(_)
        | AppError::TargetNotFound
        | AppError::TargetTimeout => !failure.provider_preparation_started,
        _ => false,
    }
}

fn prepare_dedicated_browser_job(
    owner: usize,
    events: &Arc<Mutex<VecDeque<BrowserEvent>>>,
    cancelled: &AtomicBool,
    manager: &mut Option<ChromeManager>,
    data_root: &Path,
    dispatch: &BrowserJob,
    job: &DedicatedChromeJob,
) -> std::result::Result<CdpClient, BrowserAttemptFailure> {
    let request = &dispatch.request;
    let request_id = &request.id;
    let deadline =
        AttemptDeadline::new(job.attempt_timeout).map_err(|error| BrowserAttemptFailure {
            error,
            provider_preparation_started: false,
        })?;
    let before_preparation = (|| -> Result<(CdpClient, CdpTarget, GenericProviderAdapter)> {
        if manager.is_none() {
            let installation = ChromeInstallation::discover(job.configured_chrome_path.as_deref())?;
            let profile = ManagedProfile::open(&job.profile_dir, data_root)?;
            *manager = Some(ChromeManager::new(installation, profile));
        }
        let manager = manager.as_mut().ok_or_else(|| {
            AppError::BrowserConnectionFailed("browser manager initialization failed".to_owned())
        })?;
        let endpoint =
            manager.launch_and_wait(deadline.remaining(job.connect_timeout)?, cancelled)?;
        if manager.managed_process_id().is_none() {
            return Err(AppError::BrowserConnectionFailed(
                "browser process ownership could not be confirmed".to_owned(),
            ));
        }
        send_stage(owner, events, request_id, BrowserStage::Started);

        let client = connect_with_one_retry(
            endpoint,
            deadline.remaining(job.connect_timeout)?,
            cancelled,
        )?;
        send_stage(owner, events, request_id, BrowserStage::Connected);

        let targets = client.list_targets_with_timeout(deadline.remaining(job.page_timeout)?)?;
        let page_targets: Vec<CdpTarget> = targets
            .into_iter()
            .filter(|target| target.kind == "page")
            .collect();
        let core_targets: Vec<BrowserTarget> = page_targets
            .iter()
            .map(|target| BrowserTarget::new(&target.id, &target.url))
            .collect();
        let target = match TargetResolver::resolve(
            &core_targets,
            &job.url_patterns,
            &FocusEvidence::Unknown,
        ) {
            TargetDecision::UseExisting(target_id) => page_targets
                .into_iter()
                .find(|target| target.id == target_id)
                .ok_or(AppError::TargetNotFound)?,
            TargetDecision::CreateNew => client.create_target_with_timeout(
                &job.start_url,
                deadline.remaining(job.page_timeout)?,
            )?,
        };
        client.activate_target_with_timeout(&target.id, deadline.remaining(job.page_timeout)?)?;
        send_stage(owner, events, request_id, BrowserStage::TargetResolved);
        client.wait_until_ready(&target, deadline.remaining(job.page_timeout)?, cancelled)?;
        let adapter = GenericProviderAdapter::for_provider(
            &request.provider_id,
            dispatch.adapter_override.as_deref(),
            job.url_patterns.clone(),
        )?;
        Ok((client, target, adapter))
    })()
    .map_err(|error| BrowserAttemptFailure {
        error,
        provider_preparation_started: false,
    })?;

    let (client, target, adapter) = before_preparation;
    let policy = deadline
        .preparation_policy(&dispatch.policy)
        .map_err(|error| BrowserAttemptFailure {
            error,
            provider_preparation_started: false,
        })?;
    let temp_root = data_root.join("Temp");
    (|| -> Result<CdpClient> {
        let mut page = PageSession::DedicatedChrome {
            client: &client,
            target: &target,
            temp_root: &temp_root,
            cancelled,
        };
        let outcome = adapter.prepare(&mut page, request, &policy)?;
        outcome.validate_for(request)?;
        send_event(
            owner,
            events,
            BrowserEvent::Prepared {
                request_id: request_id.to_owned(),
                surface: BrowserSurface::DedicatedChrome,
                outcome,
            },
        );
        Ok(client)
    })()
    .map_err(|error| BrowserAttemptFailure {
        error,
        provider_preparation_started: true,
    })
}

pub(super) fn close_idle_managed_browser(
    manager: &mut Option<ChromeManager>,
    client: &mut Option<CdpClient>,
) {
    close_managed_browser(manager, client);
}

pub(super) fn close_managed_browser(
    manager: &mut Option<ChromeManager>,
    client: &mut Option<CdpClient>,
) {
    // Bounded so application exit (which joins this worker after Shutdown)
    // never waits tens of seconds on an unresponsive Chrome. Whatever the
    // graceful path does not finish in time falls through to terminate.
    const SHUTDOWN_BUDGET: Duration = Duration::from_secs(7);
    const MIN_STEP: Duration = Duration::from_secs(1);
    let started = Instant::now();
    let remaining = || SHUTDOWN_BUDGET.saturating_sub(started.elapsed());
    let Some(manager) = manager.as_mut() else {
        return;
    };
    if manager.managed_process_id().is_none() {
        return;
    }
    let close_cancelled = AtomicBool::new(false);
    let mut closed = client
        .take()
        .is_some_and(|client| client.close_browser(&close_cancelled).is_ok());
    if !closed
        && remaining() >= MIN_STEP
        && let Some(endpoint) = manager.read_managed_endpoint().ok()
    {
        closed = CdpClient::connect(
            endpoint,
            remaining().min(Duration::from_secs(5)),
            &close_cancelled,
        )
        .and_then(|client| client.close_browser(&close_cancelled))
        .is_ok();
    }
    let exited = closed
        && remaining() >= MIN_STEP
        && manager
            .wait_for_managed_exit(remaining().min(Duration::from_secs(5)))
            .unwrap_or(false);
    if !exited && manager.terminate_managed().is_err() {
        tracing::warn!(
            stage = "managed_browser_shutdown",
            completed = false,
            "managed Chrome did not exit cleanly"
        );
    }
}

fn connect_with_one_retry(
    endpoint: DevToolsEndpoint,
    timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<CdpClient> {
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        AppError::BrowserConnectionFailed("CDP retry timeout is too large".to_owned())
    })?;
    match CdpClient::connect(
        endpoint.clone(),
        deadline.saturating_duration_since(Instant::now()),
        cancelled,
    ) {
        Ok(client) => Ok(client),
        Err(first_error) => {
            if cancelled.load(Ordering::Acquire) {
                return Err(AppError::BrowserCancelled);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(first_error);
            }
            thread::sleep(remaining.min(Duration::from_millis(50)));
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(first_error);
            }
            CdpClient::connect(endpoint, remaining, cancelled).map_err(|_| first_error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::jobs::{BrowserLaunchPlan, ClipboardPasteJob, ClipboardPasteOpenTarget};
    use super::*;

    #[test]
    fn clipboard_fallback_requires_proof_that_nothing_was_inserted() {
        let before_preparation = |error| BrowserAttemptFailure {
            error,
            provider_preparation_started: false,
        };
        let during_preparation = |error| BrowserAttemptFailure {
            error,
            provider_preparation_started: true,
        };

        assert!(automatic_clipboard_fallback_allowed(&before_preparation(
            AppError::ChromeNotFound
        )));
        assert!(automatic_clipboard_fallback_allowed(&before_preparation(
            AppError::TargetTimeout
        )));
        assert!(automatic_clipboard_fallback_allowed(&during_preparation(
            AppError::PreparationFailed {
                stage: askbridge_core::PreparationFailureStage::ComposerDiscovery,
                recovery: askbridge_core::PreparationRecovery::Retry,
                text_inserted: false,
                attachment_prepared: false,
            }
        )));
        assert!(!automatic_clipboard_fallback_allowed(&during_preparation(
            AppError::PreparationFailed {
                stage: askbridge_core::PreparationFailureStage::Verification,
                recovery: askbridge_core::PreparationRecovery::Retry,
                text_inserted: true,
                attachment_prepared: false,
            }
        )));
        assert!(!automatic_clipboard_fallback_allowed(&during_preparation(
            AppError::PreparationFailed {
                stage: askbridge_core::PreparationFailureStage::Verification,
                recovery: askbridge_core::PreparationRecovery::Retry,
                text_inserted: false,
                attachment_prepared: true,
            }
        )));
        assert!(automatic_clipboard_fallback_allowed(&before_preparation(
            AppError::BrowserConnectionFailed("before adapter".to_owned())
        )));
        assert!(!automatic_clipboard_fallback_allowed(&during_preparation(
            AppError::BrowserConnectionFailed("ambiguous mutation state".to_owned())
        )));
        assert!(!automatic_clipboard_fallback_allowed(&before_preparation(
            AppError::BrowserCancelled
        )));
    }

    #[test]
    #[ignore = "writes an in-memory test image to a real ChatGPT window but never sends"]
    fn dedicated_failure_falls_back_to_real_clipboard_paste_without_sending() {
        let width = 24;
        let height = 24;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let accent = if (x / 6 + y / 6) % 2 == 0 { 230 } else { 80 };
                rgba.extend_from_slice(&[accent, 40, 180, 255]);
            }
        }
        let image = askbridge_core::CapturedImage::new(
            width,
            height,
            rgba,
            askbridge_core::ScreenRect::new(0, 0, width, height),
        )
        .expect("test image");
        let request = askbridge_core::DispatchRequest::new(
            "manual-fallback-acceptance".to_owned(),
            askbridge_core::DispatchMode::CaptureWithPrompt,
            "chatgpt".to_owned(),
            String::new(),
            Some(image),
            1,
        )
        .expect("dispatch request");
        let plan = BrowserLaunchPlan::dedicated_then_clipboard(
            DedicatedChromeJob {
                configured_chrome_path: Some(r"missing-for-fallback-test\chrome.exe".to_owned()),
                profile_dir: "BrowserProfile".to_owned(),
                connect_timeout: Duration::from_secs(1),
                page_timeout: Duration::from_secs(1),
                attempt_timeout: Some(Duration::from_secs(3)),
                lifecycle: askbridge_core::BrowserLifecycle::OnDemandKeepRunning,
                start_url: "https://chatgpt.com/".to_owned(),
                url_patterns: vec!["https://chatgpt.com/".to_owned()],
            },
            ClipboardPasteJob {
                start_url: "https://chatgpt.com/".to_owned(),
                title_keywords: vec!["ChatGPT".to_owned()],
                open_target: ClipboardPasteOpenTarget::DesktopPwa {
                    provider_id: "chatgpt".to_owned(),
                    configured_shortcut: None,
                },
                locate_timeout: Duration::from_secs(45),
            },
        );
        let job = BrowserJob {
            request,
            policy: askbridge_core::PreparationPolicy::new(1_000).expect("policy"),
            adapter_override: None,
            plan,
        };
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let cancelled = AtomicBool::new(false);
        let mut manager = None;

        let result =
            prepare_browser_job(0, &events, &cancelled, &mut manager, Path::new("."), &job)
                .expect("safe fallback should paste without a managed browser");
        assert!(result.is_none());

        let events = events.lock().expect("events");
        assert!(events.iter().any(|event| matches!(
            event,
            BrowserEvent::FallbackStarted {
                from: BrowserSurface::DedicatedChrome,
                to: BrowserSurface::ClipboardPaste,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            BrowserEvent::Prepared {
                surface: BrowserSurface::ClipboardPaste,
                ..
            }
        )));
    }
}
