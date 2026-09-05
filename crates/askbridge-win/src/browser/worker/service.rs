//! Browser worker service: command channel, cancellation state, event queue,
//! and the single worker thread loop.

use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use askbridge_core::{AppError, BrowserLifecycle, Result};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

use crate::adapter::{cleanup_stale_temp_images, refresh_rules_from_environment};
use crate::util::last_error;

use super::jobs::{BrowserJob, BrowserStage, BrowserWarmupJob};
use super::prepare::{
    check_provider_batch, close_idle_managed_browser, close_managed_browser, prepare_browser_job,
    warmup_browser,
};

pub const WM_BROWSER_EVENT: u32 = WM_APP + 5;

#[derive(Debug)]
pub enum BrowserEvent {
    Stage {
        request_id: String,
        stage: BrowserStage,
    },
    Prepared {
        request_id: String,
        surface: super::jobs::BrowserSurface,
        outcome: askbridge_core::PreparationOutcome,
    },
    FallbackStarted {
        request_id: String,
        from: super::jobs::BrowserSurface,
        to: super::jobs::BrowserSurface,
    },
    WarmupReady,
    WarmupFailed {
        error: AppError,
    },
    ProviderHealthCompleted {
        reports: Vec<crate::adapter::ProviderHealthReport>,
    },
    Failed {
        request_id: String,
        error: AppError,
    },
}

enum BrowserCommand {
    Prepare(Box<BrowserJob>, Arc<AtomicBool>),
    Warmup(BrowserWarmupJob, Arc<AtomicBool>),
    ProviderHealth(super::jobs::ProviderHealthJob, Arc<AtomicBool>),
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
    pub fn check_providers(&self, job: super::jobs::ProviderHealthJob) -> Result<()> {
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
    let mut manager: Option<crate::browser::ChromeManager> = None;
    let mut connected_client: Option<crate::browser::CdpClient> = None;
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
                        .map(crate::adapter::ProviderHealthReport::network_error)
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

pub(super) fn send_stage(
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

pub(super) fn send_event(
    owner: usize,
    events: &Arc<Mutex<VecDeque<BrowserEvent>>>,
    event: BrowserEvent,
) {
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
}
