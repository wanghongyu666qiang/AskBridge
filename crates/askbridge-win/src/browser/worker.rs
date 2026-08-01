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
    AppError, BrowserTarget, FocusEvidence, Result, TargetDecision, TargetResolver,
};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

use super::{
    CdpClient, CdpTarget, ChromeInstallation, ChromeManager, DesktopPwaLauncher, ManagedProfile,
};

pub const WM_BROWSER_EVENT: u32 = WM_APP + 5;

#[derive(Debug, Clone)]
pub struct BrowserJob {
    pub request_id: String,
    pub launch: BrowserLaunch,
}

#[derive(Debug, Clone)]
pub enum BrowserLaunch {
    DedicatedChrome(DedicatedChromeJob),
    DesktopPwa(DesktopPwaJob),
}

#[derive(Debug, Clone)]
pub struct DedicatedChromeJob {
    pub configured_chrome_path: Option<String>,
    pub profile_dir: String,
    pub connect_timeout: Duration,
    pub page_timeout: Duration,
    pub lifecycle: String,
    pub start_url: String,
    pub url_patterns: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DesktopPwaJob {
    pub provider_id: String,
    pub configured_shortcut: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BrowserWarmupJob {
    pub configured_chrome_path: Option<String>,
    pub profile_dir: String,
    pub connect_timeout: Duration,
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
}

#[derive(Debug)]
pub enum BrowserEvent {
    Stage {
        request_id: String,
        stage: BrowserStage,
    },
    Ready {
        request_id: String,
        surface: BrowserSurface,
        target_id: String,
        created: bool,
    },
    WarmupReady,
    WarmupFailed {
        error: AppError,
    },
    Failed {
        request_id: String,
        error: AppError,
    },
}

enum BrowserCommand {
    Prepare(BrowserJob),
    Warmup(BrowserWarmupJob),
    Shutdown,
}

pub struct BrowserService {
    commands: Sender<BrowserCommand>,
    events: Arc<Mutex<VecDeque<BrowserEvent>>>,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl BrowserService {
    pub fn start(owner: HWND, data_root: PathBuf) -> Self {
        let (sender, receiver) = mpsc::channel();
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_events = Arc::clone(&events);
        let worker_cancelled = Arc::clone(&cancelled);
        let owner = owner as usize;
        let worker = thread::spawn(move || {
            worker_loop(owner, receiver, worker_events, worker_cancelled, data_root);
        });
        Self {
            commands: sender,
            events,
            cancelled,
            worker: Some(worker),
        }
    }

    pub fn prepare(&self, job: BrowserJob) -> Result<()> {
        self.cancelled.store(false, Ordering::Release);
        self.commands
            .send(BrowserCommand::Prepare(job))
            .map_err(|_| {
                AppError::BrowserConnectionFailed("browser worker is unavailable".to_owned())
            })
    }

    pub fn warmup(&self, job: BrowserWarmupJob) -> Result<()> {
        self.cancelled.store(false, Ordering::Release);
        self.commands
            .send(BrowserCommand::Warmup(job))
            .map_err(|_| {
                AppError::BrowserConnectionFailed("browser worker is unavailable".to_owned())
            })
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
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
    cancelled: Arc<AtomicBool>,
    data_root: PathBuf,
) {
    let mut manager: Option<ChromeManager> = None;
    let mut connected_client: Option<CdpClient> = None;
    let mut idle_close_deadline: Option<Instant> = None;
    loop {
        match commands.recv_timeout(Duration::from_secs(1)) {
            Ok(BrowserCommand::Prepare(job)) => {
                let request_id = job.request_id.clone();
                if matches!(&job.launch, BrowserLaunch::DedicatedChrome(_)) {
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
                            &job.launch,
                            BrowserLaunch::DedicatedChrome(job)
                                if job.lifecycle == "on_demand_idle_close"
                        ) {
                            idle_close_deadline =
                                Some(Instant::now() + Duration::from_secs(10 * 60));
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        send_event(owner, &events, BrowserEvent::Failed { request_id, error });
                    }
                }
            }
            Ok(BrowserCommand::Warmup(job)) => {
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
            Ok(BrowserCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if idle_close_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    close_idle_managed_browser(&mut manager, &mut connected_client);
                    idle_close_deadline = None;
                }
            }
        }
    }
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
    connect_with_one_retry(endpoint, job.connect_timeout, cancelled)
}

fn prepare_browser_job(
    owner: usize,
    events: &Arc<Mutex<VecDeque<BrowserEvent>>>,
    cancelled: &AtomicBool,
    manager: &mut Option<ChromeManager>,
    data_root: &Path,
    job: &BrowserJob,
) -> Result<Option<CdpClient>> {
    match &job.launch {
        BrowserLaunch::DedicatedChrome(dedicated) => prepare_dedicated_browser_job(
            owner,
            events,
            cancelled,
            manager,
            data_root,
            &job.request_id,
            dedicated,
        )
        .map(Some),
        BrowserLaunch::DesktopPwa(desktop) => {
            if cancelled.load(Ordering::Acquire) {
                return Err(AppError::BrowserCancelled);
            }
            let target = DesktopPwaLauncher::open(
                &desktop.provider_id,
                desktop.configured_shortcut.as_deref(),
            )?;
            send_event(
                owner,
                events,
                BrowserEvent::Ready {
                    request_id: job.request_id.clone(),
                    surface: BrowserSurface::DesktopPwa,
                    target_id: target.id(),
                    created: false,
                },
            );
            Ok(None)
        }
    }
}

fn prepare_dedicated_browser_job(
    owner: usize,
    events: &Arc<Mutex<VecDeque<BrowserEvent>>>,
    cancelled: &AtomicBool,
    manager: &mut Option<ChromeManager>,
    data_root: &Path,
    request_id: &str,
    job: &DedicatedChromeJob,
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
    let (target, created) =
        match TargetResolver::resolve(&core_targets, &job.url_patterns, &FocusEvidence::Unknown) {
            TargetDecision::UseExisting(target_id) => {
                let target = page_targets
                    .into_iter()
                    .find(|target| target.id == target_id)
                    .ok_or(AppError::TargetNotFound)?;
                (target, false)
            }
            TargetDecision::CreateNew => (client.create_target(&job.start_url)?, true),
        };
    client.activate_target(&target.id)?;
    send_stage(owner, events, request_id, BrowserStage::TargetResolved);
    client.wait_until_ready(&target, job.page_timeout, cancelled)?;
    send_event(
        owner,
        events,
        BrowserEvent::Ready {
            request_id: request_id.to_owned(),
            surface: BrowserSurface::DedicatedChrome,
            target_id: target.id,
            created,
        },
    );
    Ok(client)
}

fn close_idle_managed_browser(manager: &mut Option<ChromeManager>, client: &mut Option<CdpClient>) {
    let Some(manager) = manager.as_mut() else {
        return;
    };
    if manager.managed_process_id().is_none() {
        return;
    }
    let Some(client) = client.take() else {
        return;
    };
    let close_cancelled = AtomicBool::new(false);
    if client.close_browser(&close_cancelled).is_ok() {
        let _ = manager.wait_for_managed_exit(Duration::from_secs(5));
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
    unsafe {
        PostMessageW(owner as HWND, WM_BROWSER_EVENT, 0, 0);
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
}
