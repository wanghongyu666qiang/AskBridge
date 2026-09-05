mod download;
mod http;
mod release;
mod verify;
mod version;

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use askbridge_core::{AppError, Result};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

use download::{cleanup_stale_updates, download_release};
use release::check_latest;
use verify::{
    hash_file_streaming, hold_exclusive_read, is_safe_file_name, read_hash_record,
    remove_cached_setup, validate_downloaded_setup, verify_cached_hash,
};
use version::ReleaseVersion;

pub const WM_UPDATE_EVENT: u32 = WM_APP + 7;

const UPDATE_DIRECTORY: &str = "Updates";
const DISABLE_AUTO_CHECK_ENV: &str = "ASKBRIDGE_DISABLE_UPDATE_CHECK";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
pub(super) const MAX_RELEASE_BYTES: usize = 1024 * 1024;
pub(super) const MAX_SETUP_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    version: String,
    notes: String,
    release_url: String,
    setup_name: String,
    setup_url: String,
    setup_size: u64,
    checksum_url: String,
    signature_url: String,
}

impl AvailableUpdate {
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn notes(&self) -> &str {
        &self.notes
    }

    pub fn release_url(&self) -> &str {
        &self.release_url
    }
}

#[derive(Debug)]
pub enum UpdateEvent {
    Checked {
        available: Option<AvailableUpdate>,
        manual: bool,
    },
    DownloadProgress {
        version: String,
        received: u64,
        total: u64,
    },
    Downloaded {
        release: AvailableUpdate,
        setup_path: PathBuf,
    },
    Failed {
        action: UpdateAction,
        manual: bool,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    Check,
    Download,
}

enum UpdateCommand {
    Check { manual: bool },
    Download(AvailableUpdate),
    Shutdown,
}

pub struct UpdateService {
    commands: Sender<UpdateCommand>,
    events: Arc<Mutex<VecDeque<UpdateEvent>>>,
    worker: Option<JoinHandle<()>>,
    update_root: PathBuf,
}

impl UpdateService {
    pub fn start(owner: HWND, data_root: &Path, current_version: &str) -> Result<Self> {
        if !data_root.is_absolute() {
            return Err(update_error("更新缓存根目录必须是绝对路径"));
        }
        let current_version = ReleaseVersion::parse(current_version)?;
        let update_root = data_root.join(UPDATE_DIRECTORY);
        cleanup_stale_updates(&update_root);
        let automatic_checks_enabled = std::env::var(DISABLE_AUTO_CHECK_ENV).as_deref() != Ok("1");
        let (commands, receiver) = mpsc::channel();
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let worker_events = Arc::clone(&events);
        let worker_root = update_root.clone();
        let owner = owner as usize;
        let worker = thread::spawn(move || {
            worker_loop(
                owner,
                receiver,
                worker_events,
                worker_root,
                current_version,
                automatic_checks_enabled,
            );
        });
        if automatic_checks_enabled {
            commands
                .send(UpdateCommand::Check { manual: false })
                .map_err(|_| update_error("更新检查线程不可用"))?;
        }
        Ok(Self {
            commands,
            events,
            worker: Some(worker),
            update_root,
        })
    }

    pub fn check_now(&self) -> Result<()> {
        self.commands
            .send(UpdateCommand::Check { manual: true })
            .map_err(|_| update_error("更新检查线程不可用"))
    }

    pub fn download(&self, release: AvailableUpdate) -> Result<()> {
        self.commands
            .send(UpdateCommand::Download(release))
            .map_err(|_| update_error("更新下载线程不可用"))
    }

    /// Returns the previously downloaded setup for `version` if it still sits
    /// in the update cache as a verified direct child. Lets the tray retry an
    /// install without downloading again after a failed installer launch.
    pub fn cached_verified_setup(&self, version: &str) -> Option<PathBuf> {
        let name = format!("AskBridge-{version}-Setup.exe");
        if !is_safe_file_name(&name) {
            return None;
        }
        let candidate = self.update_root.join(name);
        validate_downloaded_setup(&self.update_root, &candidate)
            .and_then(|()| verify_cached_hash(&candidate))
            .map(|_| candidate)
            .ok()
    }

    pub fn drain_events(&self) -> Vec<UpdateEvent> {
        let Ok(mut events) = self.events.lock() else {
            return vec![UpdateEvent::Failed {
                action: UpdateAction::Check,
                manual: false,
                message: "更新事件队列不可用".to_owned(),
            }];
        };
        events.drain(..).collect()
    }

    pub fn supports_in_place_update(&self) -> bool {
        std::env::current_exe()
            .ok()
            .and_then(|executable| executable.parent().map(Path::to_path_buf))
            .is_some_and(|install_root| install_root.join("install-manifest.json").is_file())
    }

    pub fn launch_installer(&self, setup_path: &Path) -> Result<()> {
        validate_downloaded_setup(&self.update_root, setup_path)?;
        // Pin the file through verification and spawn so the bytes CreateProcess
        // executes are the bytes the hash check verified (see hold_exclusive_read).
        let pinned = hold_exclusive_read(setup_path)
            .map_err(|source| AppError::io("pinning cached update", setup_path, source))?;
        let expected = read_hash_record(setup_path)?;
        let actual = hash_file_streaming(setup_path)?;
        if !actual.eq_ignore_ascii_case(&expected) {
            // The pin blocks deletion, so release it before cleanup.
            drop(pinned);
            remove_cached_setup(setup_path);
            return Err(update_error("缓存更新安装包与校验记录不一致，请重新下载"));
        }
        let executable = std::env::current_exe().map_err(|source| {
            update_error(format!("无法定位当前运行的 AskBridge 程序（{source}）"))
        })?;
        let install_root = executable
            .parent()
            .ok_or_else(|| update_error("无法确定 AskBridge 安装目录"))?;
        if !install_root.join("install-manifest.json").is_file() {
            return Err(update_error(
                "当前程序不是通过 AskBridge 安装器安装，不能执行应用内覆盖升级",
            ));
        }
        Command::new(setup_path)
            .env("ASKBRIDGE_INSTALL_ROOT", install_root)
            .env(
                "ASKBRIDGE_UPDATE_PARENT_PID",
                std::process::id().to_string(),
            )
            .env("ASKBRIDGE_RESTART_AFTER_INSTALL", "1")
            .spawn()
            .map_err(|source| AppError::io("launching AskBridge updater", setup_path, source))?;
        Ok(())
    }
}

impl Drop for UpdateService {
    fn drop(&mut self) {
        let _ = self.commands.send(UpdateCommand::Shutdown);
        // Dropping the handle detaches the worker. A synchronous WinHTTP request may still be
        // inside its bounded timeout; application exit and self-update must not wait for it.
        let _ = self.worker.take();
    }
}

fn worker_loop(
    owner: usize,
    commands: Receiver<UpdateCommand>,
    events: Arc<Mutex<VecDeque<UpdateEvent>>>,
    update_root: PathBuf,
    current_version: ReleaseVersion,
    automatic_checks_enabled: bool,
) {
    // The automatic cadence is a wall-clock deadline independent of command
    // traffic: manual checks and downloads must not postpone it indefinitely.
    let mut next_automatic_check = Instant::now() + CHECK_INTERVAL;
    loop {
        let timeout = if automatic_checks_enabled {
            next_automatic_check.saturating_duration_since(Instant::now())
        } else {
            CHECK_INTERVAL
        };
        match commands.recv_timeout(timeout) {
            Ok(UpdateCommand::Check { manual }) => {
                let event = match check_latest(&current_version) {
                    Ok(available) => UpdateEvent::Checked { available, manual },
                    Err(error) => UpdateEvent::Failed {
                        action: UpdateAction::Check,
                        manual,
                        message: error.to_string(),
                    },
                };
                push_event(owner, &events, event);
            }
            Ok(UpdateCommand::Download(release)) => {
                let version = release.version.clone();
                let progress_events = Arc::clone(&events);
                let event = match download_release(&update_root, &release, |received, total| {
                    push_event(
                        owner,
                        &progress_events,
                        UpdateEvent::DownloadProgress {
                            version: version.clone(),
                            received,
                            total,
                        },
                    );
                }) {
                    Ok(setup_path) => UpdateEvent::Downloaded {
                        release,
                        setup_path,
                    },
                    Err(error) => UpdateEvent::Failed {
                        action: UpdateAction::Download,
                        manual: true,
                        message: error.to_string(),
                    },
                };
                push_event(owner, &events, event);
            }
            Ok(UpdateCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if !automatic_checks_enabled {
                    continue;
                }
                next_automatic_check = Instant::now() + CHECK_INTERVAL;
                let event = match check_latest(&current_version) {
                    Ok(available) => UpdateEvent::Checked {
                        available,
                        manual: false,
                    },
                    Err(error) => UpdateEvent::Failed {
                        action: UpdateAction::Check,
                        manual: false,
                        message: error.to_string(),
                    },
                };
                push_event(owner, &events, event);
            }
        }
    }
}

fn push_event(owner: usize, events: &Arc<Mutex<VecDeque<UpdateEvent>>>, event: UpdateEvent) {
    let Ok(mut queue) = events.lock() else {
        return;
    };
    queue.push_back(event);
    drop(queue);
    // SAFETY: The owner is the live hidden AskBridge window. This private message carries no
    // pointer-bearing parameters; event data remains owned by the synchronized queue.
    unsafe {
        PostMessageW(owner as HWND, WM_UPDATE_EVENT, 0, 0);
    }
}

pub(super) fn update_error(message: impl Into<String>) -> AppError {
    AppError::UpdateFailed(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_errors_keep_update_context() {
        let error = update_error("测试");
        assert!(error.to_string().contains("application update failed"));
    }
}
