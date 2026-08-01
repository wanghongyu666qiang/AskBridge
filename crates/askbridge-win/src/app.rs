use std::{
    mem::zeroed,
    ptr,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use askbridge_core::{
    AppCommand, AppConfig, AppError, AppState, BrowserTargetPreference, CapturedImage, ConfigStore,
    DispatchMode, DispatchRequest, HotkeyConfig, Result, WorkflowController,
};
use tracing::{error, info, warn};
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{COLOR_WINDOW, GetSysColorBrush},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
        Input::KeyboardAndMouse::{GetKeyState, VK_ESCAPE, VK_RETURN, VK_SHIFT},
        WindowsAndMessaging::{
            CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
            DispatchMessageW, GetMessageW, IDC_ARROW, IsDialogMessageW, LoadCursorW, MSG,
            PostMessageW, PostQuitMessage, RegisterClassW, TranslateMessage, WM_CLOSE, WM_COMMAND,
            WM_HOTKEY, WM_KEYDOWN, WNDCLASSW, WNDPROC, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
        },
    },
};

use crate::{
    browser::{
        BrowserEvent, BrowserJob, BrowserLaunch, BrowserService, BrowserStage, BrowserSurface,
        BrowserWarmupJob, ChromeInstallation, DedicatedChromeJob, DesktopPwaJob, WM_BROWSER_EVENT,
    },
    capture::{CaptureOutcome, CaptureService, WM_CAPTURE_BUSY},
    data_dir,
    hotkey_manager::HotkeyManager,
    prompt::{
        CONTROL_PROMPT_CANCEL, CONTROL_PROMPT_SUBMIT, PROMPT_CLASS, PromptWindow,
        prompt_window_proc,
    },
    settings::{
        CONTROL_APPLY, CONTROL_CLOSE, CONTROL_RESTORE_DEFAULTS, SETTINGS_CLASS, SettingsWindow,
        settings_window_proc,
    },
    single_instance::{ACTIVATE_MESSAGE, MAIN_WINDOW_CLASS, MAIN_WINDOW_TITLE, SingleInstance},
    tray::{
        MENU_CAPTURE_QUICK, MENU_CAPTURE_WITH_PROMPT, MENU_EXIT, MENU_PAUSE, MENU_SETTINGS,
        MENU_TEXT_ONLY, TrayEvent, TrayIcon, WM_TRAY_CALLBACK, WM_TRAY_DISPATCH,
        decode_tray_callback,
    },
    util::{last_error, wide},
};

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub fn run() -> Result<()> {
    init_logging();
    // SAFETY: Process DPI awareness must be selected before any windows are created.
    if unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } == 0 {
        warn!(
            win32_code = last_error(),
            "per-monitor V2 DPI awareness could not be enabled"
        );
    }
    let _instance_guard = match SingleInstance::acquire() {
        Ok(instance) => instance,
        Err(AppError::AlreadyRunning) => return Ok(()),
        Err(error) => return Err(error),
    };

    let data_root = data_dir::resolve()?;
    let config_path = data_root.join("config.json");
    let store = ConfigStore::new(config_path);
    let loaded = store.load_or_create()?;

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
    register_window_class(PROMPT_CLASS, instance, Some(prompt_window_proc))?;

    let main_window = MainWindow::create(instance)?;
    let capture = CaptureService::new(instance, main_window.hwnd())?;
    let mut hotkeys = HotkeyManager::new(main_window.hwnd());
    let registration_errors = hotkeys.register_initial(&loaded.config.hotkeys);
    let tray = TrayIcon::create(main_window.hwnd())?;
    let settings = SettingsWindow::create(
        ptr::null_mut(),
        instance,
        &loaded.config.hotkeys,
        &loaded.config.browser,
    )?;
    let prompt = PromptWindow::create(instance)?;
    let browser = BrowserService::start(main_window.hwnd(), data_root);

    let mut runtime = Runtime {
        capture,
        hotkeys,
        tray,
        settings,
        prompt,
        browser,
        config: loaded.config,
        store,
        workflow: WorkflowController::default(),
        pending_prompt: None,
        pending_dispatch: None,
        paused: false,
        _main_window: main_window,
    };
    runtime.warmup_browser_if_configured();

    if let Some(backup) = loaded.recovered_from {
        warn!(
            backup = %backup.display(),
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
        warn!(errors = %summary, "one or more hotkeys could not be registered");
        runtime
            .settings
            .set_status(&format!("快捷键冲突：{summary}"));
        runtime
            .tray
            .notify("AskBridge 快捷键冲突", "请打开设置修改冲突的快捷键。");
    }

    runtime.message_loop()
}

fn user_facing_error(error: &AppError) -> String {
    match error {
        AppError::HotkeyRegistrationFailed {
            binding,
            win32_code,
        } => format!("快捷键 {binding} 已被其他程序占用（Windows 错误 {win32_code}）"),
        AppError::HotkeyConflict(details) => format!("快捷键设置互相冲突：{details}"),
        AppError::InvalidHotkey(details) => format!("快捷键格式无效：{details}"),
        AppError::ConfigurationInvalid(details) => format!("配置无效：{details}"),
        AppError::CaptureCancelled => "截图已取消。".to_owned(),
        AppError::CaptureFailed(details) => format!("截图失败：{details}"),
        AppError::InvalidDispatchRequest(details) => format!("问题无法继续：{details}"),
        AppError::InvalidProvider(details) => format!("供应商无效：{details}"),
        AppError::WorkflowBusy(_) => "另一个操作尚未完成。".to_owned(),
        AppError::ClipboardUnavailable => "剪贴板正被其他程序占用，请稍后重试。".to_owned(),
        AppError::ClipboardWriteFailed => "无法将截图写入剪贴板。".to_owned(),
        AppError::ChromeNotFound => "未找到 Google Chrome，请在设置中选择 chrome.exe。".to_owned(),
        AppError::BrowserProfileRejected(details) => {
            format!("专用浏览器目录不安全：{details}")
        }
        AppError::BrowserLaunchFailed => "专用 Chrome 启动失败。".to_owned(),
        AppError::DesktopShortcutNotFound(_) => {
            "未找到桌面 ChatGPT 快捷方式，请确认 ChatGPT.lnk 仍在桌面。".to_owned()
        }
        AppError::DesktopShortcutRejected(details) => {
            format!("桌面快捷方式不安全：{details}")
        }
        AppError::DesktopLaunchFailed(_) => "ChatGPT 桌面网页端启动失败。".to_owned(),
        AppError::BrowserEndpointUnavailable => "专用 Chrome 未能提供调试端点，请重试。".to_owned(),
        AppError::BrowserConnectionFailed(_) | AppError::BrowserProtocol(_) => {
            "无法连接 AskBridge 专用 Chrome，请重试。".to_owned()
        }
        AppError::BrowserCancelled => "浏览器操作已取消。".to_owned(),
        AppError::TargetNotFound => "未找到可用的目标页面。".to_owned(),
        AppError::TargetTimeout => "目标页面加载超时，请检查浏览器后重试。".to_owned(),
        _ => error.to_string(),
    }
}

struct PendingPrompt {
    command: AppCommand,
    image: Option<CapturedImage>,
}

struct Runtime {
    // Handle-backed fields are ordered before main_window so they release their resources first.
    capture: CaptureService,
    hotkeys: HotkeyManager,
    tray: TrayIcon,
    settings: SettingsWindow,
    prompt: PromptWindow,
    browser: BrowserService,
    config: AppConfig,
    store: ConfigStore,
    workflow: WorkflowController,
    pending_prompt: Option<PendingPrompt>,
    pending_dispatch: Option<DispatchRequest>,
    paused: bool,
    _main_window: MainWindow,
}

impl Runtime {
    fn message_loop(&mut self) -> Result<()> {
        // SAFETY: Zero is the documented initial state for MSG.
        let mut message: MSG = unsafe { zeroed() };
        loop {
            // SAFETY: message points to writable storage and null HWND selects this thread queue.
            let result = unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) };
            if result == -1 {
                return Err(AppError::Windows {
                    operation: "GetMessageW",
                    win32_code: last_error(),
                });
            }
            if result == 0 {
                break;
            }
            if self.handle_prompt_key(&message)? {
                continue;
            }
            if self.handle_message(&message)? {
                continue;
            }
            if self.prompt.is_visible() && self.prompt.contains(message.hwnd) {
                // SAFETY: The prompt window is live and message belongs to it or a child.
                if unsafe { IsDialogMessageW(self.prompt.hwnd(), &message) } != 0 {
                    continue;
                }
            }
            // SAFETY: message was populated successfully by GetMessageW.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        Ok(())
    }

    fn handle_message(&mut self, message: &MSG) -> Result<bool> {
        if message.message == ACTIVATE_MESSAGE {
            if self.prompt.is_visible() {
                info!("activation message received; focusing prompt");
                self.prompt.focus_existing();
            } else {
                info!("activation message received; showing settings");
                self.settings.show();
            }
            return Ok(true);
        }
        match message.message {
            WM_BROWSER_EVENT => {
                self.handle_browser_events();
                Ok(true)
            }
            WM_CAPTURE_BUSY => {
                self.tray
                    .notify("AskBridge 正在框选", "框选期间触发的其他快捷键已忽略。");
                Ok(true)
            }
            WM_HOTKEY => {
                if !self.paused {
                    if let Some(command) = self.hotkeys.command_for_id(message.wParam as i32) {
                        self.route_command(command);
                    }
                }
                Ok(true)
            }
            WM_TRAY_DISPATCH => {
                match decode_tray_callback(message.lParam) {
                    TrayEvent::ContextMenu => {
                        if let Some(command) = self.tray.show_menu(self.paused)? {
                            self.handle_command(command)?;
                        }
                    }
                    TrayEvent::ActivateSettings => self.settings.show(),
                    TrayEvent::Ignore => {}
                }
                Ok(true)
            }
            WM_COMMAND => {
                let command = (message.wParam & 0xffff) as u16;
                self.handle_command(command)?;
                Ok(true)
            }
            WM_CLOSE if message.hwnd == self.settings.hwnd() => {
                self.settings.hide();
                Ok(true)
            }
            WM_CLOSE if message.hwnd == self.prompt.hwnd() => {
                self.cancel_prompt();
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn handle_prompt_key(&mut self, message: &MSG) -> Result<bool> {
        if message.message != WM_KEYDOWN
            || !self.prompt.is_visible()
            || !self.prompt.contains(message.hwnd)
        {
            return Ok(false);
        }
        if message.wParam == usize::from(VK_ESCAPE) {
            self.cancel_prompt();
            return Ok(true);
        }
        if message.wParam == usize::from(VK_RETURN) && message.hwnd == self.prompt.editor_hwnd() {
            // SAFETY: GetKeyState is a read-only query for the current UI thread.
            let shift_pressed = unsafe { GetKeyState(i32::from(VK_SHIFT)) } < 0;
            if !shift_pressed {
                self.submit_prompt();
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn handle_command(&mut self, command: u16) -> Result<()> {
        match command {
            MENU_CAPTURE_WITH_PROMPT => self.route_command(AppCommand::CaptureWithPrompt),
            MENU_CAPTURE_QUICK => self.route_command(AppCommand::CaptureQuickDispatch),
            MENU_TEXT_ONLY => self.route_command(AppCommand::TextOnlyPrompt),
            MENU_PAUSE => self.toggle_paused(),
            MENU_SETTINGS => self.settings.show(),
            MENU_EXIT => {
                info!("application exit requested");
                self.browser.cancel();
                // SAFETY: Called on the UI thread to end its message loop.
                unsafe {
                    PostQuitMessage(0);
                }
            }
            CONTROL_APPLY => self.apply_settings(),
            CONTROL_RESTORE_DEFAULTS => self.restore_default_hotkeys(),
            CONTROL_CLOSE => self.settings.hide(),
            CONTROL_PROMPT_SUBMIT => self.submit_prompt(),
            CONTROL_PROMPT_CANCEL => self.cancel_prompt(),
            _ => {}
        }
        Ok(())
    }

    fn route_command(&mut self, command: AppCommand) {
        if self.workflow.state() == AppState::Prompting && self.prompt.is_visible() {
            info!(
                command = command.event_name(),
                "prompt already open; focusing existing workflow"
            );
            self.prompt.focus_existing();
            return;
        }
        if !self.workflow.is_idle() {
            info!(
                command = command.event_name(),
                state = ?self.workflow.state(),
                "workflow command ignored while busy"
            );
            self.tray
                .notify("AskBridge 正在处理", "当前操作尚未完成，请先完成或取消。");
            return;
        }
        if let Err(error) = self.workflow.start(command) {
            self.workflow_failed(command, error);
            return;
        }
        match command {
            AppCommand::CaptureWithPrompt | AppCommand::CaptureQuickDispatch => {
                self.capture_command(command);
            }
            AppCommand::TextOnlyPrompt => {
                info!(
                    command = command.event_name(),
                    phase = 3_u8,
                    "text-only command routed"
                );
                self.open_prompt(command, None);
            }
        }
    }

    fn capture_command(&mut self, command: AppCommand) {
        info!(
            command = command.event_name(),
            phase = 3_u8,
            "capture requested"
        );
        match self.capture.capture() {
            Ok(CaptureOutcome::Captured(image)) => {
                info!(
                    command = command.event_name(),
                    width = image.width,
                    height = image.height,
                    rgba_bytes = image.rgba_bytes.len(),
                    "capture completed in memory"
                );
                if let Err(error) = self.workflow.capture_completed(command) {
                    self.workflow_failed(command, error);
                    return;
                }
                match command {
                    AppCommand::CaptureWithPrompt => self.open_prompt(command, Some(image)),
                    AppCommand::CaptureQuickDispatch => {
                        let provider_id = self.config.default_provider_id.clone();
                        let prompt = self.config.quick_prompt.clone();
                        self.prepare_request(command, provider_id, prompt, Some(image));
                    }
                    AppCommand::TextOnlyPrompt => unreachable!("text command does not capture"),
                }
            }
            Ok(CaptureOutcome::Cancelled) => {
                info!(command = command.event_name(), "capture cancelled");
                self.cancel_workflow();
            }
            Err(error) => {
                error!(
                    command = command.event_name(),
                    error = %error,
                    "capture failed"
                );
                self.workflow_failed(command, error);
            }
        }
    }

    fn open_prompt(&mut self, command: AppCommand, image: Option<CapturedImage>) {
        let providers = match self.config.merged_providers() {
            Ok(providers) => providers,
            Err(error) => {
                self.workflow_failed(command, error);
                return;
            }
        };
        self.pending_prompt = Some(PendingPrompt { command, image });
        if let Err(error) = self
            .prompt
            .show(&providers, &self.config.default_provider_id)
        {
            self.workflow_failed(command, error);
        }
    }

    fn submit_prompt(&mut self) {
        if self.workflow.state() != AppState::Prompting {
            return;
        }
        let input = match self.prompt.read_input() {
            Ok(input) => input,
            Err(error) => {
                self.prompt
                    .set_status(&format!("无法继续：{}", user_facing_error(&error)));
                return;
            }
        };
        if input.prompt.trim().is_empty() {
            self.prompt.set_status("请输入问题后再继续。");
            self.prompt.focus_existing();
            return;
        }
        let Some(pending) = self.pending_prompt.take() else {
            self.workflow_failed(
                AppCommand::TextOnlyPrompt,
                AppError::InvalidDispatchRequest("prompt context is missing".to_owned()),
            );
            return;
        };
        if let Err(error) = self.workflow.prompt_submitted() {
            self.workflow_failed(pending.command, error);
            return;
        }
        self.prompt.hide_and_clear();
        self.prepare_request(
            pending.command,
            input.provider_id,
            input.prompt,
            pending.image,
        );
    }

    fn cancel_prompt(&mut self) {
        self.prompt.hide_and_clear();
        self.pending_prompt = None;
        if self.workflow.state() == AppState::Prompting {
            self.cancel_workflow();
        }
    }

    fn prepare_request(
        &mut self,
        command: AppCommand,
        provider_id: String,
        prompt: String,
        image: Option<CapturedImage>,
    ) {
        let created_at_ms = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(duration) => u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            Err(error) => {
                self.workflow_failed(
                    command,
                    AppError::InvalidDispatchRequest(format!(
                        "system clock is before the Unix epoch: {error}"
                    )),
                );
                return;
            }
        };
        let request = DispatchRequest::new(
            next_request_id(created_at_ms),
            DispatchMode::from(command),
            provider_id,
            prompt,
            image,
            created_at_ms,
        );
        match request {
            Ok(request) => self.handoff_browser_request(request),
            Err(error) => self.workflow_failed(command, error),
        }
    }

    fn handoff_browser_request(&mut self, request: DispatchRequest) {
        info!(
            request_id = %request.id,
            mode = ?request.mode,
            provider_id = %request.provider_id,
            has_image = request.image.is_some(),
            auto_submit = request.auto_submit,
            "dispatch request prepared for browser surface"
        );
        let provider = match self.config.merged_providers().and_then(|providers| {
            providers
                .into_iter()
                .find(|provider| provider.enabled && provider.id == request.provider_id)
                .ok_or_else(|| {
                    AppError::InvalidProvider(format!(
                        "provider '{}' is unavailable",
                        request.provider_id
                    ))
                })
        }) {
            Ok(provider) => provider,
            Err(error) => {
                self.browser_workflow_failed(error);
                return;
            }
        };
        if let Err(error) = self.workflow.begin_browser() {
            self.browser_workflow_failed(error);
            return;
        }
        let launch = match self.config.browser.target_preference(&provider.id) {
            BrowserTargetPreference::DesktopPwa => BrowserLaunch::DesktopPwa(DesktopPwaJob {
                provider_id: provider.id.clone(),
                configured_shortcut: self
                    .config
                    .browser
                    .desktop_shortcut(&provider.id)
                    .map(str::to_owned),
            }),
            BrowserTargetPreference::DedicatedChrome => {
                BrowserLaunch::DedicatedChrome(DedicatedChromeJob {
                    configured_chrome_path: self.config.browser.chrome_path.clone(),
                    profile_dir: self.config.browser.profile_dir.clone(),
                    connect_timeout: Duration::from_millis(self.config.browser.connect_timeout_ms),
                    page_timeout: Duration::from_millis(self.config.browser.page_timeout_ms),
                    lifecycle: self.config.browser.lifecycle.clone(),
                    start_url: provider.start_url,
                    url_patterns: provider.url_patterns,
                })
            }
        };
        let opens_desktop_pwa = matches!(&launch, BrowserLaunch::DesktopPwa(_));
        let job = BrowserJob {
            request_id: request.id.clone(),
            launch,
        };
        self.pending_dispatch = Some(request);
        if let Err(error) = self.browser.prepare(job) {
            self.browser_workflow_failed(error);
            return;
        }
        if opens_desktop_pwa {
            self.tray.notify(
                "AskBridge 正在打开桌面网页端",
                "将复用现有桌面快捷方式和其中的登录状态。",
            );
        } else {
            self.tray.notify(
                "AskBridge 正在打开专用 Chrome",
                "首次使用时，请在这个独立浏览器中自行登录目标网站。",
            );
        }
    }

    fn handle_browser_events(&mut self) {
        for event in self.browser.drain_events() {
            match event {
                BrowserEvent::Stage { request_id, stage } => {
                    if !self.is_current_request(&request_id) {
                        continue;
                    }
                    let transition = match stage {
                        BrowserStage::Started => self.workflow.browser_started(),
                        BrowserStage::Connected => self.workflow.browser_connected(),
                        BrowserStage::TargetResolved => self.workflow.target_resolved(),
                    };
                    if let Err(error) = transition {
                        self.browser_workflow_failed(error);
                        return;
                    }
                    info!(
                        request_id = %request_id,
                        stage = ?stage,
                        "dedicated browser workflow advanced"
                    );
                }
                BrowserEvent::Ready {
                    request_id,
                    surface,
                    target_id,
                    created,
                } => {
                    if !self.is_current_request(&request_id) {
                        continue;
                    }
                    let ready = match surface {
                        BrowserSurface::DedicatedChrome => self.workflow.page_ready(),
                        BrowserSurface::DesktopPwa => self.workflow.desktop_surface_ready(),
                    };
                    if let Err(error) = ready {
                        self.browser_workflow_failed(error);
                        return;
                    }
                    info!(
                        request_id = %request_id,
                        target_id = %target_id,
                        created,
                        surface = ?surface,
                        "browser surface is ready"
                    );
                    self.pending_dispatch = None;
                    if let Err(error) = self.workflow.defer_page_preparation() {
                        self.browser_workflow_failed(error);
                        return;
                    }
                    match surface {
                        BrowserSurface::DedicatedChrome => self.tray.notify(
                            "AskBridge 专用 Chrome 已就绪",
                            "目标页面已打开并置前；网页内容准备将在 Phase 5 接入。",
                        ),
                        BrowserSurface::DesktopPwa => self.tray.notify(
                            "ChatGPT 桌面网页端已打开",
                            "已复用现有登录会话；内容准备将在 Phase 5 接入。",
                        ),
                    }
                }
                BrowserEvent::WarmupReady => {
                    info!("dedicated browser started with AskBridge");
                    self.tray.notify(
                        "AskBridge 专用 Chrome 已启动",
                        "浏览器生命周期设置为“随 AskBridge 启动”。",
                    );
                }
                BrowserEvent::WarmupFailed { error } => {
                    error!(error = %error, "dedicated browser startup warmup failed");
                    self.tray
                        .notify("AskBridge 专用 Chrome 启动失败", &user_facing_error(&error));
                }
                BrowserEvent::Failed { request_id, error } => {
                    if request_id.is_empty() || self.is_current_request(&request_id) {
                        self.browser_workflow_failed(error);
                        return;
                    }
                }
            }
        }
    }

    fn warmup_browser_if_configured(&self) {
        if self.config.browser.lifecycle != "on_startup" {
            return;
        }
        let job = BrowserWarmupJob {
            configured_chrome_path: self.config.browser.chrome_path.clone(),
            profile_dir: self.config.browser.profile_dir.clone(),
            connect_timeout: Duration::from_millis(self.config.browser.connect_timeout_ms),
        };
        if let Err(error) = self.browser.warmup(job) {
            error!(error = %error, "failed to queue dedicated browser startup");
        }
    }

    fn is_current_request(&self, request_id: &str) -> bool {
        self.pending_dispatch
            .as_ref()
            .is_some_and(|request| request.id == request_id)
    }

    fn browser_workflow_failed(&mut self, error: AppError) {
        error!(
            state = ?self.workflow.state(),
            error = %error,
            "browser surface workflow failed"
        );
        self.pending_dispatch = None;
        self.tray
            .notify("AskBridge 无法打开目标页面", &user_facing_error(&error));
        if !self.workflow.is_idle() && self.workflow.state() != AppState::Error {
            let _ = self.workflow.fail();
        }
        if self.workflow.state() == AppState::Error {
            let _ = self.workflow.recover();
        }
    }

    fn cancel_workflow(&mut self) {
        self.pending_prompt = None;
        self.pending_dispatch = None;
        self.browser.cancel();
        if self.workflow.is_idle() {
            return;
        }
        if self.workflow.state() != AppState::Cancelling {
            let _ = self.workflow.begin_cancelling();
        }
        if self.workflow.state() == AppState::Cancelling {
            let _ = self.workflow.finish_cancelling();
        }
    }

    fn workflow_failed(&mut self, command: AppCommand, error: AppError) {
        error!(
            command = command.event_name(),
            state = ?self.workflow.state(),
            error = %error,
            "workflow failed"
        );
        self.prompt.hide_and_clear();
        self.pending_prompt = None;
        self.pending_dispatch = None;
        self.tray
            .notify("AskBridge 无法继续", &user_facing_error(&error));
        if !self.workflow.is_idle() && self.workflow.state() != AppState::Error {
            let _ = self.workflow.fail();
        }
        if self.workflow.state() == AppState::Error {
            let _ = self.workflow.recover();
        }
    }

    fn apply_settings(&mut self) {
        let requested = match self.settings.read_hotkeys() {
            Ok(hotkeys) => hotkeys,
            Err(error) => {
                self.settings
                    .set_status(&format!("无法应用：{}", user_facing_error(&error)));
                return;
            }
        };
        let chrome_path = match self.settings.read_chrome_path() {
            Ok(path) => path,
            Err(error) => {
                self.settings
                    .set_status(&format!("无法应用：{}", user_facing_error(&error)));
                return;
            }
        };
        if let Some(path) = chrome_path.as_deref()
            && let Err(error) = ChromeInstallation::discover(Some(path))
        {
            self.settings
                .set_status(&format!("无法应用：{}", user_facing_error(&error)));
            return;
        }
        self.persist_settings(
            requested,
            chrome_path,
            self.settings.use_chatgpt_desktop_pwa(),
            "设置已保存并立即生效。",
        );
    }

    fn restore_default_hotkeys(&mut self) {
        self.persist_settings(
            HotkeyConfig::default(),
            self.config.browser.chrome_path.clone(),
            self.config.browser.target_preference("chatgpt") == BrowserTargetPreference::DesktopPwa,
            "已恢复默认快捷键并立即生效。",
        );
    }

    fn persist_settings(
        &mut self,
        requested: HotkeyConfig,
        chrome_path: Option<String>,
        use_chatgpt_desktop_pwa: bool,
        success_message: &str,
    ) {
        let mut candidate = self.config.clone();
        candidate.hotkeys = requested.clone();
        candidate.browser.chrome_path = chrome_path;
        candidate.browser.target_preferences.insert(
            "chatgpt".to_owned(),
            if use_chatgpt_desktop_pwa {
                BrowserTargetPreference::DesktopPwa
            } else {
                BrowserTargetPreference::DedicatedChrome
            },
        );
        let store = &self.store;
        let result = self
            .hotkeys
            .apply_transaction(&requested, || store.save(&candidate));
        match result {
            Ok(()) => {
                self.config = candidate;
                if self.paused {
                    let _ = self.hotkeys.pause();
                }
                self.settings
                    .refresh(&self.config.hotkeys, &self.config.browser);
                self.settings.set_status(success_message);
                info!("hotkey configuration updated");
            }
            Err(error) => {
                error!(error = %error, "hotkey configuration update failed");
                self.settings
                    .set_status(&format!("无法应用：{}", user_facing_error(&error)));
            }
        }
    }

    fn toggle_paused(&mut self) {
        if self.paused {
            let errors = self.hotkeys.register_initial(&self.config.hotkeys);
            if errors.is_empty() {
                self.paused = false;
                self.tray.notify("AskBridge", "全局快捷键已恢复。");
                info!("global hotkeys resumed");
            } else {
                let _ = self.hotkeys.pause();
                let summary = errors
                    .iter()
                    .map(user_facing_error)
                    .collect::<Vec<_>>()
                    .join("; ");
                self.settings
                    .set_status(&format!("无法恢复快捷键：{summary}"));
                self.tray
                    .notify("AskBridge 快捷键冲突", "快捷键仍保持暂停，请检查设置。");
            }
        } else {
            let errors = self.hotkeys.pause();
            if !errors.is_empty() {
                let summary = errors
                    .iter()
                    .map(user_facing_error)
                    .collect::<Vec<_>>()
                    .join("; ");
                error!(errors = %summary, "one or more hotkeys could not be paused");
                self.tray
                    .notify("AskBridge 快捷键暂停失败", "部分快捷键仍然处于活动状态。");
                return;
            }
            self.paused = true;
            self.tray.notify("AskBridge", "全局快捷键已暂停。");
            info!("global hotkeys paused");
        }
    }
}

struct MainWindow(HWND);

impl MainWindow {
    fn create(instance: HINSTANCE) -> Result<Self> {
        let class = wide(MAIN_WINDOW_CLASS);
        let title = wide(MAIN_WINDOW_TITLE);
        // SAFETY: Class is registered and all pointers remain valid for the call.
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        if window.is_null() {
            return Err(AppError::Windows {
                operation: "CreateWindowExW(main)",
                win32_code: last_error(),
            });
        }
        Ok(Self(window))
    }

    const fn hwnd(&self) -> HWND {
        self.0
    }
}

impl Drop for MainWindow {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: This guard owns the hidden top-level window.
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
}

fn register_window_class(name: &str, instance: HINSTANCE, window_proc: WNDPROC) -> Result<()> {
    let name = wide(name);
    // SAFETY: Loading the shared arrow cursor with a null module handle is supported.
    let cursor = unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) };
    if cursor.is_null() {
        return Err(AppError::Windows {
            operation: "LoadCursorW",
            win32_code: last_error(),
        });
    }
    // SAFETY: GetSysColorBrush returns a shared system brush.
    let background = unsafe { GetSysColorBrush(COLOR_WINDOW) };
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: window_proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: ptr::null_mut(),
        hCursor: cursor,
        hbrBackground: background,
        lpszMenuName: ptr::null(),
        lpszClassName: name.as_ptr(),
    };
    // SAFETY: WNDCLASSW fields remain valid for the synchronous registration call.
    if unsafe { RegisterClassW(&class) } == 0 {
        return Err(AppError::Windows {
            operation: "RegisterClassW",
            win32_code: last_error(),
        });
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_TRAY_CALLBACK {
        // Shell notification callbacks may be nonqueued. Relay them so Runtime can process them
        // in order from its ordinary GetMessage loop without storing a raw Runtime pointer.
        // SAFETY: The message carries only integer values and targets this live owner window.
        if unsafe { PostMessageW(window, WM_TRAY_DISPATCH, wparam, lparam) } == 0 {
            error!(
                win32_code = last_error(),
                "failed to queue tray callback for runtime dispatch"
            );
        }
        return 0;
    }
    if message == WM_CLOSE {
        // The hidden owner window has no visible close affordance, but process managers send
        // WM_CLOSE for a normal shutdown. End the message loop so all RAII resources and the
        // browser worker are released in order.
        // SAFETY: This callback runs on the UI thread that owns the message loop.
        unsafe {
            PostQuitMessage(0);
        }
        return 0;
    }
    // SAFETY: Unhandled messages are forwarded exactly as received to DefWindowProcW.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn next_request_id(created_at_ms: u64) -> String {
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("askbridge-{created_at_ms:013x}-{sequence:08x}")
}

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_window_messages_do_not_overlap() {
        assert_ne!(WM_TRAY_CALLBACK, ACTIVATE_MESSAGE);
        assert_ne!(WM_TRAY_CALLBACK, WM_CAPTURE_BUSY);
        assert_ne!(WM_TRAY_CALLBACK, WM_TRAY_DISPATCH);
        assert_ne!(WM_TRAY_CALLBACK, WM_BROWSER_EVENT);
        assert_ne!(WM_TRAY_DISPATCH, ACTIVATE_MESSAGE);
        assert_ne!(WM_TRAY_DISPATCH, WM_CAPTURE_BUSY);
        assert_ne!(WM_TRAY_DISPATCH, WM_BROWSER_EVENT);
        assert_ne!(ACTIVATE_MESSAGE, WM_CAPTURE_BUSY);
        assert_ne!(ACTIVATE_MESSAGE, WM_BROWSER_EVENT);
        assert_ne!(WM_CAPTURE_BUSY, WM_BROWSER_EVENT);
    }

    #[test]
    fn window_proc_relays_nonqueued_tray_callback() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            PM_REMOVE, PeekMessageW, SendMessageW, WM_CONTEXTMENU,
        };

        // SAFETY: A null module name requests the current test process module.
        let module = unsafe { GetModuleHandleW(ptr::null()) };
        assert!(!module.is_null());
        let instance = module as HINSTANCE;
        let class_name = "AskBridge.Test.TrayRelayWindow.v1";
        register_window_class(class_name, instance, Some(window_proc))
            .expect("test window class should register");
        let class = wide(class_name);
        let title = wide("AskBridge tray relay test");
        // SAFETY: The test class is registered and all pointers remain valid for the call.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        assert!(!window.is_null());
        let packed_event = ((1_u32 << 16) | WM_CONTEXTMENU) as LPARAM;

        // SAFETY: This synchronously exercises the same nonqueued window-procedure entry used by
        // Shell callbacks. No pointer-bearing parameters are sent.
        unsafe {
            SendMessageW(window, WM_TRAY_CALLBACK, 23, packed_event);
        }
        // SAFETY: Zero is a valid initial message state.
        let mut queued: MSG = unsafe { zeroed() };
        // SAFETY: The test owns the window and removes only its private dispatch message.
        let found = unsafe {
            PeekMessageW(
                &mut queued,
                window,
                WM_TRAY_DISPATCH,
                WM_TRAY_DISPATCH,
                PM_REMOVE,
            )
        };
        // SAFETY: The test owns this window and no longer needs it.
        unsafe {
            DestroyWindow(window);
        }

        assert_ne!(found, 0);
        assert_eq!(queued.message, WM_TRAY_DISPATCH);
        assert_eq!(queued.wParam, 23);
        assert_eq!(queued.lParam, packed_event);
    }

    #[test]
    fn main_window_close_requests_a_clean_message_loop_exit() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            PM_REMOVE, PeekMessageW, SendMessageW, WM_QUIT,
        };

        // SAFETY: A null module name requests the current test process module.
        let module = unsafe { GetModuleHandleW(ptr::null()) };
        assert!(!module.is_null());
        let instance = module as HINSTANCE;
        let class_name = "AskBridge.Test.MainCloseWindow.v1";
        register_window_class(class_name, instance, Some(window_proc))
            .expect("test window class should register");
        let class = wide(class_name);
        let title = wide("AskBridge close test");
        // SAFETY: The test class is registered and arguments are valid.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                instance,
                ptr::null(),
            )
        };
        assert!(!window.is_null());

        // SAFETY: The test owns this window and synchronously exercises normal close handling.
        unsafe {
            SendMessageW(window, WM_CLOSE, 0, 0);
        }
        let mut queued: MSG = unsafe { zeroed() };
        let mut found_quit = false;
        // SAFETY: This test thread owns the queue. Drain pending messages until WM_QUIT appears.
        while unsafe { PeekMessageW(&mut queued, ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if queued.message == WM_QUIT {
                found_quit = true;
                break;
            }
        }
        // SAFETY: The test still owns the window because the custom handler did not destroy it.
        unsafe {
            DestroyWindow(window);
        }

        assert!(found_quit);
    }
}
