use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use askbridge_core::{
    AppError, BrowserLifecycle, BrowserTarget, DispatchRequest, FocusEvidence,
    PreparationFailureStage, PreparationOutcome, PreparationPolicy, PreparationRecovery, Result,
    TargetDecision, TargetResolver,
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

use super::{
    CdpClient, CdpTarget, ChromeInstallation, ChromeManager, DesktopPwaLauncher, ManagedProfile,
};
use crate::adapter::{
    GenericProviderAdapter, PageSession, ProviderAdapter, ProviderHealthCheck,
    ProviderHealthReport, check_provider_health, cleanup_stale_temp_images,
    refresh_rules_from_environment,
};
use crate::util::last_error;

pub const WM_BROWSER_EVENT: u32 = WM_APP + 5;
const COLD_OPEN_EDITOR_STABILITY_TIMEOUT: Duration = Duration::from_secs(7);
const COLD_OPEN_EDITOR_MIN_SETTLE: Duration = Duration::from_secs(2);
const COLD_OPEN_EDITOR_STABILITY_INTERVAL: Duration = Duration::from_millis(100);
const COLD_OPEN_EDITOR_STABILITY_SAMPLES: u8 = 2;

#[derive(Debug, Clone)]
pub struct BrowserJob {
    pub request: DispatchRequest,
    pub policy: PreparationPolicy,
    pub adapter_override: Option<String>,
    pub plan: BrowserLaunchPlan,
}

#[derive(Debug, Clone)]
pub enum BrowserLaunch {
    DedicatedChrome(DedicatedChromeJob),
    DesktopPwa(DesktopPwaJob),
    ClipboardPaste(ClipboardPasteJob),
}

/// A deliberately small launch policy. The only automatic fallback we expose
/// is managed Chrome to clipboard paste; arbitrary chains would make it
/// impossible to reason about duplicate insertion safely.
#[derive(Debug, Clone)]
pub struct BrowserLaunchPlan {
    primary: BrowserLaunch,
    fallback: Option<BrowserLaunch>,
}

impl BrowserLaunchPlan {
    pub const fn single(primary: BrowserLaunch) -> Self {
        Self {
            primary,
            fallback: None,
        }
    }

    pub const fn dedicated_then_clipboard(
        dedicated: DedicatedChromeJob,
        clipboard: ClipboardPasteJob,
    ) -> Self {
        Self {
            primary: BrowserLaunch::DedicatedChrome(dedicated),
            fallback: Some(BrowserLaunch::ClipboardPaste(clipboard)),
        }
    }

    pub(crate) const fn primary(&self) -> &BrowserLaunch {
        &self.primary
    }

    const fn fallback(&self) -> Option<&BrowserLaunch> {
        self.fallback.as_ref()
    }

    pub const fn primary_surface(&self) -> BrowserSurface {
        launch_surface(&self.primary)
    }

    pub const fn fallback_surface(&self) -> Option<BrowserSurface> {
        match self.fallback.as_ref() {
            Some(launch) => Some(launch_surface(launch)),
            None => None,
        }
    }

    const fn dedicated_lifecycle(&self) -> Option<BrowserLifecycle> {
        match &self.primary {
            BrowserLaunch::DedicatedChrome(job) => Some(job.lifecycle),
            BrowserLaunch::DesktopPwa(_) | BrowserLaunch::ClipboardPaste(_) => None,
        }
    }
}

const fn launch_surface(launch: &BrowserLaunch) -> BrowserSurface {
    match launch {
        BrowserLaunch::DedicatedChrome(_) => BrowserSurface::DedicatedChrome,
        BrowserLaunch::DesktopPwa(_) => BrowserSurface::DesktopPwa,
        BrowserLaunch::ClipboardPaste(_) => BrowserSurface::ClipboardPaste,
    }
}

#[derive(Debug, Clone)]
pub struct DedicatedChromeJob {
    pub configured_chrome_path: Option<String>,
    pub profile_dir: String,
    pub connect_timeout: Duration,
    pub page_timeout: Duration,
    pub lifecycle: BrowserLifecycle,
    pub start_url: String,
    pub url_patterns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DesktopPwaJob {
    pub provider_id: String,
    pub configured_shortcut: Option<String>,
    pub start_url: String,
    pub url_patterns: Vec<String>,
}

/// Inputs for the clipboard-paste target: screenshot to clipboard, focus a
/// matching provider page or supported desktop client (opening a page if
/// needed), and synthesize one Ctrl+V.
#[derive(Debug, Clone)]
pub struct ClipboardPasteJob {
    pub start_url: String,
    pub title_keywords: Vec<String>,
    /// How to cold-open the user-selected target when no matching window is
    /// already available. Delivery stays clipboard-based either way.
    pub open_target: ClipboardPasteOpenTarget,
    /// Total budget for locating the target window, including the one-time
    /// cold-open wait when no matching window exists yet.
    pub locate_timeout: Duration,
}

#[derive(Debug, Clone)]
pub enum ClipboardPasteOpenTarget {
    DefaultBrowser,
    DesktopPwa {
        provider_id: String,
        configured_shortcut: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct BrowserWarmupJob {
    pub configured_chrome_path: Option<String>,
    pub profile_dir: String,
    pub connect_timeout: Duration,
    pub page_timeout: Duration,
    pub open_url: Option<String>,
}

/// Managed-Chrome inputs for a batch of no-send provider capability checks.
#[derive(Debug, Clone)]
pub struct ProviderHealthJob {
    /// Optional configured Chrome executable path.
    pub configured_chrome_path: Option<String>,
    /// Managed Chrome profile directory name.
    pub profile_dir: String,
    /// Maximum time allowed to connect to Chrome DevTools.
    pub connect_timeout: Duration,
    /// Maximum time allowed for each provider page inspection.
    pub page_timeout: Duration,
    /// Providers to inspect without mutating their pages.
    pub providers: Vec<ProviderHealthCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserStage {
    Started,
    Connected,
    TargetResolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserSurface {
    DedicatedChrome,
    DesktopPwa,
    ClipboardPaste,
}

#[derive(Debug)]
pub enum BrowserEvent {
    Stage {
        request_id: String,
        stage: BrowserStage,
    },
    Prepared {
        request_id: String,
        surface: BrowserSurface,
        outcome: PreparationOutcome,
    },
    FallbackStarted {
        request_id: String,
        from: BrowserSurface,
        to: BrowserSurface,
    },
    WarmupReady,
    WarmupFailed {
        error: AppError,
    },
    ProviderHealthCompleted {
        reports: Vec<ProviderHealthReport>,
    },
    Failed {
        request_id: String,
        error: AppError,
    },
}

enum BrowserCommand {
    Prepare(Box<BrowserJob>, Arc<AtomicBool>),
    Warmup(BrowserWarmupJob, Arc<AtomicBool>),
    ProviderHealth(ProviderHealthJob, Arc<AtomicBool>),
    Reconfigure,
    CloseManaged,
    Shutdown,
}

#[derive(Default)]
struct CancellationState {
    current: Mutex<Option<Arc<AtomicBool>>>,
}

impl CancellationState {
    fn begin(&self) -> Result<Arc<AtomicBool>> {
        let mut current = self.current.lock().map_err(|_| {
            AppError::BrowserConnectionFailed(
                "browser cancellation state is unavailable".to_owned(),
            )
        })?;
        if let Some(previous) = current.as_ref() {
            previous.store(true, Ordering::Release);
        }
        let token = Arc::new(AtomicBool::new(false));
        *current = Some(Arc::clone(&token));
        Ok(token)
    }

    fn cancel_current(&self) {
        if let Ok(current) = self.current.lock()
            && let Some(token) = current.as_ref()
        {
            token.store(true, Ordering::Release);
        }
    }
}

pub struct BrowserService {
    commands: Sender<BrowserCommand>,
    events: Arc<Mutex<VecDeque<BrowserEvent>>>,
    cancellation: CancellationState,
    worker: Option<JoinHandle<()>>,
}

impl BrowserService {
    pub fn start(owner: HWND, data_root: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel();
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let worker_events = Arc::clone(&events);
        let owner = owner as usize;
        let worker = thread::spawn(move || {
            worker_loop(owner, receiver, worker_events, data_root);
        });
        Self {
            commands: sender,
            events,
            cancellation: CancellationState::default(),
            worker: Some(worker),
        }
    }

    pub fn prepare(&self, job: BrowserJob) -> Result<()> {
        let cancelled = self.cancellation.begin()?;
        self.commands
            .send(BrowserCommand::Prepare(Box::new(job), cancelled))
            .map_err(|_| {
                AppError::BrowserConnectionFailed("browser worker is unavailable".to_owned())
            })
    }

    pub fn warmup(&self, job: BrowserWarmupJob) -> Result<()> {
        let cancelled = self.cancellation.begin()?;
        self.commands
            .send(BrowserCommand::Warmup(job, cancelled))
            .map_err(|_| {
                AppError::BrowserConnectionFailed("browser worker is unavailable".to_owned())
            })
    }

    /// Queues provider capability checks that never insert or submit user text.
    pub fn check_providers(&self, job: ProviderHealthJob) -> Result<()> {
        let cancelled = self.cancellation.begin()?;
        self.commands
            .send(BrowserCommand::ProviderHealth(job, cancelled))
            .map_err(|_| {
                AppError::BrowserConnectionFailed("browser worker is unavailable".to_owned())
            })
    }

    pub fn cancel(&self) {
        self.cancellation.cancel_current();
    }

    pub fn reconfigure(&self) -> Result<()> {
        self.commands
            .send(BrowserCommand::Reconfigure)
            .map_err(|_| {
                AppError::BrowserConnectionFailed("browser worker is unavailable".to_owned())
            })
    }

    pub fn close_managed(&self) -> Result<()> {
        self.commands
            .send(BrowserCommand::CloseManaged)
            .map_err(|_| {
                AppError::BrowserConnectionFailed("browser worker is unavailable".to_owned())
            })
    }

    pub fn drain_events(&self) -> Vec<BrowserEvent> {
        let Ok(mut events) = self.events.lock() else {
            return vec![BrowserEvent::Failed {
                request_id: String::new(),
                error: AppError::BrowserConnectionFailed(
                    "browser event queue is unavailable".to_owned(),
                ),
            }];
        };
        events.drain(..).collect()
    }
}

impl Drop for BrowserService {
    fn drop(&mut self) {
        self.cancel();
        let _ = self.commands.send(BrowserCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn worker_loop(
    owner: usize,
    commands: Receiver<BrowserCommand>,
    events: Arc<Mutex<VecDeque<BrowserEvent>>>,
    data_root: PathBuf,
) {
    match refresh_rules_from_environment(&data_root) {
        Ok(source) => tracing::info!(
            stage = "provider_rules",
            completed = true,
            source = ?source,
            "provider rules initialized"
        ),
        Err(_error) => tracing::warn!(
            stage = "provider_rules",
            completed = false,
            "provider rules fell back to built-in defaults"
        ),
    }
    if let Err(_error) = cleanup_stale_temp_images(&data_root) {
        tracing::warn!(
            stage = "temporary_image_cleanup",
            completed = false,
            "stale temporary image cleanup failed"
        );
    }
    let mut manager: Option<ChromeManager> = None;
    let mut connected_client: Option<CdpClient> = None;
    let mut idle_close_deadline: Option<Instant> = None;
    loop {
        match commands.recv_timeout(Duration::from_secs(1)) {
            Ok(BrowserCommand::Prepare(job, cancelled)) => {
                let request_id = job.request.id.clone();
                if job.plan.dedicated_lifecycle().is_some() {
                    idle_close_deadline = None;
                }
                match prepare_browser_job(
                    owner,
                    &events,
                    &cancelled,
                    &mut manager,
                    &data_root,
                    &job,
                ) {
                    Ok(Some(client)) => {
                        connected_client = Some(client);
                        if matches!(
                            job.plan.dedicated_lifecycle(),
                            Some(BrowserLifecycle::OnDemandIdleClose)
                        ) {
                            idle_close_deadline =
                                Some(Instant::now() + Duration::from_secs(10 * 60));
                        }
                    }
                    Ok(None) => {
                        if matches!(
                            job.plan.dedicated_lifecycle(),
                            Some(BrowserLifecycle::OnDemandIdleClose)
                        ) && manager
                            .as_ref()
                            .is_some_and(|manager| manager.managed_process_id().is_some())
                        {
                            idle_close_deadline =
                                Some(Instant::now() + Duration::from_secs(10 * 60));
                        }
                    }
                    Err(error) => {
                        send_event(owner, &events, BrowserEvent::Failed { request_id, error });
                    }
                }
            }
            Ok(BrowserCommand::Warmup(job, cancelled)) => {
                match warmup_browser(&cancelled, &mut manager, &data_root, &job) {
                    Ok(client) => {
                        connected_client = Some(client);
                        send_event(owner, &events, BrowserEvent::WarmupReady);
                    }
                    Err(error) => {
                        send_event(owner, &events, BrowserEvent::WarmupFailed { error });
                    }
                }
            }
            Ok(BrowserCommand::ProviderHealth(job, cancelled)) => {
                let provider_ids = job
                    .providers
                    .iter()
                    .map(|provider| provider.provider_id.clone())
                    .collect::<Vec<_>>();
                let reports = match check_provider_batch(&cancelled, &mut manager, &data_root, &job)
                {
                    Ok((client, reports)) => {
                        connected_client = Some(client);
                        reports
                    }
                    Err(_error) => provider_ids
                        .into_iter()
                        .map(ProviderHealthReport::network_error)
                        .collect(),
                };
                send_event(
                    owner,
                    &events,
                    BrowserEvent::ProviderHealthCompleted { reports },
                );
            }
            Ok(BrowserCommand::Reconfigure | BrowserCommand::CloseManaged) => {
                close_managed_browser(&mut manager, &mut connected_client);
                idle_close_deadline = None;
            }
            Ok(BrowserCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if idle_close_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    close_idle_managed_browser(&mut manager, &mut connected_client);
                    idle_close_deadline = None;
                }
            }
        }
    }
    close_managed_browser(&mut manager, &mut connected_client);
}

fn warmup_browser(
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

fn check_provider_batch(
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

fn prepare_browser_job(
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

struct BrowserAttemptFailure {
    error: AppError,
    provider_preparation_started: bool,
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

/// Clipboard-paste target: no CDP at all. Writes the screenshot to the
/// clipboard, then locates a matching provider page or supported desktop
/// client (opening a page if needed) and synthesizes a single Ctrl+V into it.
fn prepare_clipboard_paste_job(
    owner: usize,
    events: &Arc<Mutex<VecDeque<BrowserEvent>>>,
    cancelled: &AtomicBool,
    dispatch: &BrowserJob,
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
        DesktopPwaLauncher::open(provider_id, configured_shortcut.as_deref())?;
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
                    DesktopPwaLauncher::open(provider_id, configured_shortcut.as_deref())?;
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
        BrowserEvent::Prepared {
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
    let policy = &dispatch.policy;
    let request_id = &request.id;
    let before_preparation = (|| -> Result<(CdpClient, CdpTarget, GenericProviderAdapter)> {
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
        send_stage(owner, events, request_id, BrowserStage::Started);

        let client = connect_with_one_retry(endpoint, job.connect_timeout, cancelled)?;
        send_stage(owner, events, request_id, BrowserStage::Connected);

        let targets = client.list_targets()?;
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
            TargetDecision::CreateNew => client.create_target(&job.start_url)?,
        };
        client.activate_target(&target.id)?;
        send_stage(owner, events, request_id, BrowserStage::TargetResolved);
        client.wait_until_ready(&target, job.page_timeout, cancelled)?;
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
    let temp_root = data_root.join("Temp");
    (|| -> Result<CdpClient> {
        let mut page = PageSession::DedicatedChrome {
            client: &client,
            target: &target,
            temp_root: &temp_root,
            cancelled,
        };
        let outcome = adapter.prepare(&mut page, request, policy)?;
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

fn close_idle_managed_browser(manager: &mut Option<ChromeManager>, client: &mut Option<CdpClient>) {
    close_managed_browser(manager, client);
}

fn close_managed_browser(manager: &mut Option<ChromeManager>, client: &mut Option<CdpClient>) {
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
    endpoint: super::DevToolsEndpoint,
    timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<CdpClient> {
    match CdpClient::connect(endpoint.clone(), timeout, cancelled) {
        Ok(client) => Ok(client),
        Err(first_error) => {
            if cancelled.load(Ordering::Acquire) {
                return Err(AppError::BrowserCancelled);
            }
            thread::sleep(Duration::from_millis(50));
            CdpClient::connect(endpoint, timeout, cancelled).map_err(|_| first_error)
        }
    }
}

fn send_stage(
    owner: usize,
    events: &Arc<Mutex<VecDeque<BrowserEvent>>>,
    request_id: &str,
    stage: BrowserStage,
) {
    send_event(
        owner,
        events,
        BrowserEvent::Stage {
            request_id: request_id.to_owned(),
            stage,
        },
    );
}

fn send_event(owner: usize, events: &Arc<Mutex<VecDeque<BrowserEvent>>>, event: BrowserEvent) {
    if let Ok(mut queue) = events.lock() {
        queue.push_back(event);
    } else {
        return;
    }
    // SAFETY: The worker posts an integer-only private message to the hidden
    // owner window. Event data stays in the synchronized queue.
    if unsafe { PostMessageW(owner as HWND, WM_BROWSER_EVENT, 0, 0) } == 0 {
        tracing::error!(
            stage = "browser_event_dispatch",
            completed = false,
            win32_code = last_error(),
            "failed to wake the UI thread for a queued browser event"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_private_message_does_not_overlap_existing_messages() {
        use crate::{
            capture::WM_CAPTURE_BUSY,
            single_instance::ACTIVATE_MESSAGE,
            tray::{WM_TRAY_CALLBACK, WM_TRAY_DISPATCH},
        };

        assert_ne!(WM_BROWSER_EVENT, WM_TRAY_CALLBACK);
        assert_ne!(WM_BROWSER_EVENT, ACTIVATE_MESSAGE);
        assert_ne!(WM_BROWSER_EVENT, WM_CAPTURE_BUSY);
        assert_ne!(WM_BROWSER_EVENT, WM_TRAY_DISPATCH);
    }

    #[test]
    fn beginning_a_new_job_never_revives_the_cancelled_previous_job() {
        let cancellation = CancellationState::default();
        let first = cancellation.begin().expect("first token");
        cancellation.cancel_current();
        assert!(first.load(Ordering::Acquire));

        let second = cancellation.begin().expect("second token");
        assert!(first.load(Ordering::Acquire));
        assert!(!second.load(Ordering::Acquire));
    }

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
                lifecycle: BrowserLifecycle::OnDemandKeepRunning,
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
            policy: PreparationPolicy::new(1_000).expect("policy"),
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
