use std::{ptr, sync::atomic::AtomicU64, time::Duration};

use askbridge_core::{
    AppConfig, AppError, CapturedImage, ConfigStore, DispatchRequest, HotkeyConfig, Result,
    WorkflowController,
};
use tracing::{error, info, warn};
use windows_sys::Win32::{
    Foundation::{ERROR_ACCESS_DENIED, HINSTANCE},
    System::LibraryLoader::GetModuleHandleW,
    UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
};

use crate::{
    adapter::validate_builtin_rules,
    browser::{BrowserService, BrowserWarmupJob, ChromeInstallation},
    capture::CaptureService,
    data_dir,
    hotkey_manager::HotkeyManager,
    logging,
    settings_v2::{SETTINGS_CLASS, SettingsWindow, settings_window_proc},
    single_instance::{MAIN_WINDOW_CLASS, SingleInstance},
    startup,
    tray::TrayIcon,
    update::{AvailableUpdate, UpdateService},
    util::last_error,
};

use super::{
    error_handler::user_facing_error,
    events::{MainWindow, register_window_class, window_proc},
};

pub(super) static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Starts the Windows application and owns its UI-thread lifecycle.
pub fn run() -> Result<()> {
    validate_builtin_rules()?;
    let _instance_guard = match SingleInstance::acquire() {
        Ok(instance) => instance,
        Err(AppError::AlreadyRunning) => return Ok(()),
        Err(error) => return Err(error),
    };

    let data_root = data_dir::resolve()?;
    let config_path = data_root.join("config.json");
    let store = ConfigStore::new(config_path);
    let loaded = store.load_or_create()?;
    let _log_path = logging::init(&data_root, loaded.config.general.debug_logging)?;
    info!(
        version = env!("CARGO_PKG_VERSION"),
        stage = "startup",
        completed = false,
        "AskBridge startup began"
    );
    startup::apply(loaded.config.general.start_on_login)?;
    if loaded.config.general.start_on_login && !startup::is_current_executable_registered()? {
        return Err(AppError::ConfigurationInvalid(
            "startup registration could not be verified".to_owned(),
        ));
    }

    // SAFETY: Process DPI awareness must be selected before any windows are created.
    // The embedded manifest also declares Per-Monitor V2, in which case this call
    // fails with ERROR_ACCESS_DENIED ("already set") and the manifest governs.
    if unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } == 0 {
        let win32_code = last_error();
        if win32_code != ERROR_ACCESS_DENIED {
            warn!(
                stage = "startup",
                completed = false,
                win32_code,
                "per-monitor V2 DPI awareness could not be enabled"
            );
        }
    }

    // SAFETY: A null module name requests the current process module.
    let module = unsafe { GetModuleHandleW(ptr::null()) };
    if module.is_null() {
        return Err(AppError::Windows {
            operation: "GetModuleHandleW",
            win32_code: last_error(),
        });
    }
    let instance = module as HINSTANCE;
    register_window_class(MAIN_WINDOW_CLASS, instance, Some(window_proc))?;
    register_window_class(SETTINGS_CLASS, instance, Some(settings_window_proc))?;
    let main_window = MainWindow::create(instance)?;
    let capture = CaptureService::new(instance, main_window.hwnd())?;
    let mut hotkeys = HotkeyManager::new(main_window.hwnd());
    let registration_errors = hotkeys.register_initial(&loaded.config.hotkeys);
    let tray = TrayIcon::create(main_window.hwnd())?;
    let settings = SettingsWindow::create(ptr::null_mut(), instance, &loaded.config, &data_root)?;
    let updater = UpdateService::start(main_window.hwnd(), &data_root, env!("CARGO_PKG_VERSION"))?;
    let browser = BrowserService::start(main_window.hwnd(), data_root);

    let mut runtime = Runtime {
        capture,
        hotkeys,
        tray,
        settings,
        browser,
        updater,
        available_update: None,
        update_busy: false,
        config: loaded.config,
        store,
        workflow: WorkflowController::default(),
        pending_dispatch: None,
        last_capture: None,
        paused: false,
        _main_window: main_window,
    };
    runtime.warmup_browser_if_configured();

    if let Some(_backup) = loaded.recovered_from {
        warn!(
            stage = "configuration_recovery",
            completed = false,
            "invalid configuration was backed up and replaced with defaults"
        );
        runtime.tray.notify(
            "AskBridge 配置已恢复",
            "原配置无效，已备份并恢复为默认配置。",
        );
    } else if loaded.migrated {
        info!("configuration migrated to schema v3");
    }
    if !registration_errors.is_empty() {
        let summary = registration_errors
            .iter()
            .map(user_facing_error)
            .collect::<Vec<_>>()
            .join("; ");
        warn!(
            stage = "hotkey_registration",
            completed = false,
            "one or more hotkeys could not be registered"
        );
        runtime
            .settings
            .set_status(&format!("快捷键冲突：{summary}"));
        runtime
            .tray
            .notify("AskBridge 快捷键冲突", "请打开设置修改冲突的快捷键。");
    }

    runtime.message_loop()
}

pub(super) struct Runtime {
    // Handle-backed fields are ordered before main_window so they release their resources first.
    pub(super) capture: CaptureService,
    pub(super) hotkeys: HotkeyManager,
    pub(super) tray: TrayIcon,
    pub(super) settings: SettingsWindow,
    pub(super) browser: BrowserService,
    pub(super) updater: UpdateService,
    pub(super) available_update: Option<AvailableUpdate>,
    pub(super) update_busy: bool,
    pub(super) config: AppConfig,
    pub(super) store: ConfigStore,
    pub(super) workflow: WorkflowController,
    pub(super) pending_dispatch: Option<DispatchRequest>,
    pub(super) last_capture: Option<CapturedImage>,
    pub(super) paused: bool,
    pub(super) _main_window: MainWindow,
}

impl Runtime {
    pub(super) fn apply_settings(&mut self) {
        let candidate = match self.settings.read_config(&self.config) {
            Ok(candidate) => candidate,
            Err(error) => {
                self.settings
                    .set_status(&format!("无法应用：{}", user_facing_error(&error)));
                return;
            }
        };
        if let Some(path) = candidate.browser.chrome_path.as_deref()
            && let Err(error) = ChromeInstallation::discover(Some(path))
        {
            self.settings
                .set_status(&format!("无法应用：{}", user_facing_error(&error)));
            return;
        }
        self.persist_settings(candidate, "设置已保存并立即生效。");
    }

    pub(super) fn restore_default_hotkeys(&mut self) {
        let mut candidate = self.config.clone();
        candidate.hotkeys = HotkeyConfig::default();
        self.persist_settings(candidate, "已恢复默认快捷键并立即生效。");
    }

    fn persist_settings(&mut self, candidate: AppConfig, success_message: &str) {
        let requested = candidate.hotkeys.clone();
        let browser_changed = self.config.browser != candidate.browser;
        let debug_logging_changed =
            self.config.general.debug_logging != candidate.general.debug_logging;
        let startup_snapshot = match startup::snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.settings
                    .set_status(&format!("无法应用：{}", user_facing_error(&error)));
                return;
            }
        };
        if let Err(error) = startup::apply(candidate.general.start_on_login) {
            self.settings
                .set_status(&format!("无法应用：{}", user_facing_error(&error)));
            return;
        }
        if candidate.general.start_on_login
            && !startup::is_current_executable_registered().unwrap_or(false)
        {
            if let Err(rollback_error) = startup::restore(&startup_snapshot) {
                warn!(
                    stage = "settings",
                    completed = false,
                    error_kind = rollback_error.kind(),
                    "startup registration rollback failed"
                );
            }
            self.settings
                .set_status("无法应用：开机启动项写入后未能通过校验。");
            return;
        }
        let store = &self.store;
        let result = self
            .hotkeys
            .apply_transaction(&requested, || store.save(&candidate));
        match result {
            Ok(()) => {
                self.config = candidate;
                let mut runtime_warnings = Vec::new();
                if debug_logging_changed
                    && let Err(error) =
                        logging::set_debug_logging(self.config.general.debug_logging)
                {
                    runtime_warnings.push(user_facing_error(&error));
                }
                if browser_changed {
                    if let Err(error) = self.browser.reconfigure() {
                        runtime_warnings.push(user_facing_error(&error));
                    } else {
                        self.warmup_browser_if_configured();
                    }
                }
                if self.paused {
                    let _ = self.hotkeys.pause();
                }
                if let Err(error) = self.settings.refresh(&self.config) {
                    self.settings.set_status(&format!(
                        "已保存，但界面刷新失败：{}",
                        user_facing_error(&error)
                    ));
                    return;
                }
                if runtime_warnings.is_empty() {
                    self.settings.set_status(success_message);
                } else {
                    self.settings.set_status(&format!(
                        "设置已保存；运行时刷新失败，下次启动会生效：{}",
                        runtime_warnings.join("；")
                    ));
                }
                info!(stage = "settings", completed = true, "settings updated");
            }
            Err(error) => {
                if let Err(rollback_error) = startup::restore(&startup_snapshot) {
                    warn!(
                        stage = "settings",
                        completed = false,
                        error_kind = rollback_error.kind(),
                        "startup registration rollback failed"
                    );
                }
                error!(
                    stage = "settings",
                    completed = false,
                    "hotkey configuration update failed"
                );
                self.settings
                    .set_status(&format!("无法应用：{}", user_facing_error(&error)));
            }
        }
    }

    pub(super) fn open_browser_tool(&mut self, open_login: bool) {
        let message = if open_login {
            "正在打开默认供应商页面；请只在 AskBridge 专用 Chrome 中自行登录。"
        } else {
            "正在启动并检查 AskBridge 专用 Chrome。"
        };
        self.settings.set_status(message);
        let open_url = if open_login {
            match self.config.merged_providers().and_then(|providers| {
                providers
                    .into_iter()
                    .find(|provider| {
                        provider.enabled && provider.id == self.config.default_provider_id
                    })
                    .map(|provider| provider.start_url)
                    .ok_or_else(|| {
                        AppError::InvalidProvider(
                            "default provider is missing or disabled".to_owned(),
                        )
                    })
            }) {
                Ok(url) => Some(url),
                Err(error) => {
                    self.settings
                        .set_status(&format!("浏览器操作失败：{}", user_facing_error(&error)));
                    return;
                }
            }
        } else {
            None
        };
        let job = BrowserWarmupJob {
            configured_chrome_path: self.config.browser.chrome_path.clone(),
            profile_dir: self.config.browser.profile_dir.clone(),
            connect_timeout: Duration::from_millis(self.config.browser.connect_timeout_ms),
            page_timeout: Duration::from_millis(self.config.browser.page_timeout_ms),
            open_url,
        };
        if let Err(error) = self.browser.warmup(job) {
            self.settings
                .set_status(&format!("浏览器操作失败：{}", user_facing_error(&error)));
        }
    }
}
