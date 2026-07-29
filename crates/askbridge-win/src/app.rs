use std::{env, mem::zeroed, path::PathBuf, ptr};

use askbridge_core::{AppCommand, AppConfig, AppError, ConfigStore, HotkeyConfig, Result};
use tracing::{error, info, warn};
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{COLOR_WINDOW, GetSysColorBrush},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
        WindowsAndMessaging::{
            CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
            DispatchMessageW, GetMessageW, IDC_ARROW, LoadCursorW, MSG, PostQuitMessage,
            RegisterClassW, TranslateMessage, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU, WM_HOTKEY,
            WM_LBUTTONDBLCLK, WM_RBUTTONUP, WNDCLASSW, WNDPROC, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
        },
    },
};

use crate::{
    capture::{CaptureOutcome, CaptureService},
    hotkey_manager::HotkeyManager,
    settings::{
        CONTROL_APPLY, CONTROL_CLOSE, CONTROL_RESTORE_DEFAULTS, SETTINGS_CLASS, SettingsWindow,
        settings_window_proc,
    },
    single_instance::{ACTIVATE_MESSAGE, MAIN_WINDOW_CLASS, MAIN_WINDOW_TITLE, SingleInstance},
    tray::{
        MENU_CAPTURE_QUICK, MENU_CAPTURE_WITH_PROMPT, MENU_EXIT, MENU_PAUSE, MENU_SETTINGS,
        MENU_TEXT_ONLY, TrayIcon, WM_TRAY_CALLBACK,
    },
    util::{last_error, wide},
};

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

    let config_path = config_path()?;
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

    let main_window = MainWindow::create(instance)?;
    let capture = CaptureService::new(instance, main_window.hwnd())?;
    let mut hotkeys = HotkeyManager::new(main_window.hwnd());
    let registration_errors = hotkeys.register_initial(&loaded.config.hotkeys);
    let tray = TrayIcon::create(main_window.hwnd())?;
    let settings = SettingsWindow::create(ptr::null_mut(), instance, &loaded.config.hotkeys)?;

    let mut runtime = Runtime {
        capture,
        hotkeys,
        tray,
        settings,
        config: loaded.config,
        store,
        paused: false,
        _main_window: main_window,
    };

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
        AppError::ClipboardUnavailable => "剪贴板正被其他程序占用，请稍后重试。".to_owned(),
        AppError::ClipboardWriteFailed => "无法将截图写入剪贴板。".to_owned(),
        _ => error.to_string(),
    }
}

struct Runtime {
    // Handle-backed fields are ordered before main_window so they release their resources first.
    capture: CaptureService,
    hotkeys: HotkeyManager,
    tray: TrayIcon,
    settings: SettingsWindow,
    config: AppConfig,
    store: ConfigStore,
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
            if self.handle_message(&message)? {
                continue;
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
            info!("activation message received; showing settings");
            self.settings.show();
            return Ok(true);
        }
        match message.message {
            WM_HOTKEY => {
                if !self.paused {
                    if let Some(command) = self.hotkeys.command_for_id(message.wParam as i32) {
                        self.route_command(command);
                    }
                }
                Ok(true)
            }
            WM_TRAY_CALLBACK => {
                match message.lParam as u32 {
                    WM_RBUTTONUP | WM_CONTEXTMENU => {
                        if let Some(command) = self.tray.show_menu(self.paused)? {
                            self.handle_command(command)?;
                        }
                    }
                    WM_LBUTTONDBLCLK => self.settings.show(),
                    _ => {}
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
            _ => Ok(false),
        }
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
                // SAFETY: Called on the UI thread to end its message loop.
                unsafe {
                    PostQuitMessage(0);
                }
            }
            CONTROL_APPLY => self.apply_settings(),
            CONTROL_RESTORE_DEFAULTS => self.restore_default_hotkeys(),
            CONTROL_CLOSE => self.settings.hide(),
            _ => {}
        }
        Ok(())
    }

    fn route_command(&mut self, command: AppCommand) {
        match command {
            AppCommand::CaptureWithPrompt | AppCommand::CaptureQuickDispatch => {
                self.capture_command(command);
            }
            AppCommand::TextOnlyPrompt => {
                info!(
                    command = command.event_name(),
                    phase = 2_u8,
                    "text-only command routed"
                );
                self.tray.notify(
                    "AskBridge 事件已触发",
                    "直接文字提问将在 Phase 3 接入轻量输入框。",
                );
            }
        }
    }

    fn capture_command(&mut self, command: AppCommand) {
        info!(
            command = command.event_name(),
            phase = 2_u8,
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
                self.tray.notify(
                    "AskBridge 截图已捕获",
                    &format!("{} × {}，已保存在内存中。", image.width, image.height),
                );
            }
            Ok(CaptureOutcome::Cancelled) => {
                info!(command = command.event_name(), "capture cancelled");
            }
            Err(error) => {
                error!(
                    command = command.event_name(),
                    error = %error,
                    "capture failed"
                );
                self.tray
                    .notify("AskBridge 截图失败", &user_facing_error(&error));
            }
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
        self.persist_hotkeys(requested, "快捷键已保存并立即生效。");
    }

    fn restore_default_hotkeys(&mut self) {
        self.persist_hotkeys(HotkeyConfig::default(), "已恢复默认快捷键并立即生效。");
    }

    fn persist_hotkeys(&mut self, requested: HotkeyConfig, success_message: &str) {
        let mut candidate = self.config.clone();
        candidate.hotkeys = requested.clone();
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
                self.settings.refresh(&self.config.hotkeys);
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
    // SAFETY: Unhandled messages are forwarded exactly as received to DefWindowProcW.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn config_path() -> Result<PathBuf> {
    let local_app_data = env::var_os("LOCALAPPDATA").ok_or_else(|| {
        AppError::ConfigurationInvalid("LOCALAPPDATA is not available".to_owned())
    })?;
    Ok(PathBuf::from(local_app_data)
        .join("AskBridge")
        .join("config.json"))
}

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .try_init();
}
