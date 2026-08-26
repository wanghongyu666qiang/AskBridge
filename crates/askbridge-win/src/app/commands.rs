use askbridge_core::{AppCommand, Result};
use tracing::info;
use windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage;

use crate::{
    settings_v2::{
        CONTROL_APPLY, CONTROL_CHECK_BROWSER, CONTROL_CHECK_PROVIDERS, CONTROL_CLOSE,
        CONTROL_OPEN_BROWSER, CONTROL_OPEN_LOGIN, CONTROL_RESTORE_DEFAULTS,
    },
    tray::{
        MENU_CAPTURE_QUICK, MENU_CAPTURE_WITH_PROMPT, MENU_CHECK_UPDATES, MENU_EXIT,
        MENU_INSTALL_UPDATE, MENU_PAUSE, MENU_SETTINGS, MENU_TEXT_ONLY,
    },
};

use super::controller::Runtime;

impl Runtime {
    pub(super) fn handle_command(&mut self, command: u16) -> Result<()> {
        match command {
            MENU_CAPTURE_WITH_PROMPT => self.route_command(AppCommand::CaptureWithPrompt),
            MENU_CAPTURE_QUICK => self.route_command(AppCommand::CaptureQuickDispatch),
            MENU_TEXT_ONLY => self.route_command(AppCommand::TextOnlyPrompt),
            MENU_PAUSE => self.toggle_paused(),
            MENU_SETTINGS => self.settings.show(),
            MENU_CHECK_UPDATES => self.check_for_updates(),
            MENU_INSTALL_UPDATE => self.prompt_and_download_update(),
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
            CONTROL_OPEN_BROWSER | CONTROL_CHECK_BROWSER => self.open_browser_tool(false),
            CONTROL_OPEN_LOGIN => self.open_browser_tool(true),
            CONTROL_CHECK_PROVIDERS => self.start_provider_health_check(),
            _ => {}
        }
        Ok(())
    }

    pub(super) fn route_command(&mut self, command: AppCommand) {
        if !self.workflow.is_idle() {
            info!(
                stage = "workflow",
                completed = false,
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
                    stage = "browser_handoff",
                    completed = true,
                    has_image = false,
                    "text-only command opens provider composer directly"
                );
                let provider_id = self.config.default_provider_id.clone();
                self.prepare_request(command, provider_id, String::new(), None);
            }
        }
    }
}
