use askbridge_core::{AppCommand, AppError, Result};
use tracing::{error, info};

use crate::capture::{CaptureOutcome, CaptureProviderChoice};

use super::controller::Runtime;

impl Runtime {
    pub(super) fn capture_command(&mut self, command: AppCommand) {
        info!(stage = "capture", completed = false, "capture requested");
        let capture = match command {
            AppCommand::CaptureWithPrompt => self
                .capture_toolbar_providers()
                .and_then(|providers| self.capture.capture_with_toolbar(providers)),
            AppCommand::CaptureQuickDispatch => self.capture.capture(),
            AppCommand::TextOnlyPrompt => unreachable!("text command does not capture"),
        };
        match capture {
            Ok(CaptureOutcome::Captured(image)) => {
                info!(
                    stage = "capture",
                    completed = true,
                    has_image = true,
                    "capture completed in memory"
                );
                if let Err(error) = self.workflow.capture_completed(command) {
                    self.workflow_failed(command, error);
                    return;
                }
                self.last_capture = Some(image.clone());
                match command {
                    AppCommand::CaptureWithPrompt => {
                        let provider_id = self.config.default_provider_id.clone();
                        self.prepare_request(command, provider_id, String::new(), Some(image));
                    }
                    AppCommand::CaptureQuickDispatch => {
                        let provider_id = self.config.default_provider_id.clone();
                        let prompt = self.config.quick_prompt.clone();
                        self.prepare_request(command, provider_id, prompt, Some(image));
                    }
                    AppCommand::TextOnlyPrompt => unreachable!("text command does not capture"),
                }
            }
            Ok(CaptureOutcome::CapturedForProvider { image, provider_id }) => {
                info!(
                    stage = "capture",
                    completed = true,
                    has_image = true,
                    "capture completed from toolbar action"
                );
                if let Err(error) = self.workflow.capture_completed(command) {
                    self.workflow_failed(command, error);
                    return;
                }
                self.last_capture = Some(image.clone());
                if let Err(error) = self.remember_default_provider(&provider_id) {
                    self.workflow_failed(command, error);
                    return;
                }
                self.prepare_request(command, provider_id, String::new(), Some(image));
            }
            Ok(CaptureOutcome::CopiedToClipboard) => {
                info!(
                    stage = "capture",
                    completed = true,
                    copied = true,
                    "capture copied to clipboard from toolbar action"
                );
                self.tray
                    .notify("AskBridge 已复制截图", "截图已复制到剪贴板。");
                self.cancel_workflow();
            }
            Ok(CaptureOutcome::Cancelled) => {
                info!(
                    stage = "capture",
                    completed = false,
                    cancelled = true,
                    "capture cancelled"
                );
                self.cancel_workflow();
            }
            Err(error) => {
                error!(stage = "capture", completed = false, "capture failed");
                self.workflow_failed(command, error);
            }
        }
    }

    fn capture_toolbar_providers(&self) -> Result<Vec<CaptureProviderChoice>> {
        let providers = self
            .config
            .merged_providers()?
            .into_iter()
            .filter(|provider| provider.enabled)
            .map(|provider| CaptureProviderChoice {
                selected: provider.id == self.config.default_provider_id,
                id: provider.id,
                display_name: provider.display_name,
            })
            .collect::<Vec<_>>();
        if providers.is_empty() {
            return Err(AppError::InvalidProvider(
                "no enabled providers are available".to_owned(),
            ));
        }
        Ok(providers)
    }

    fn remember_default_provider(&mut self, provider_id: &str) -> Result<()> {
        if self.config.default_provider_id == provider_id {
            return Ok(());
        }
        let mut candidate = self.config.clone();
        candidate.default_provider_id = provider_id.to_owned();
        self.store.save(&candidate)?;
        self.config = candidate;
        self.settings.refresh(&self.config)?;
        info!(
            provider_id = %provider_id,
            stage = "capture_toolbar",
            completed = true,
            "default provider updated from capture toolbar"
        );
        Ok(())
    }

    pub(super) fn copy_last_capture_to_clipboard(&mut self) {
        let Some(image) = self.last_capture.as_ref() else {
            self.tray
                .notify("AskBridge 没有可复制的截图", "请先完成一次截图。");
            return;
        };
        match crate::clipboard_image::copy_image_to_clipboard(self._main_window.hwnd(), image) {
            Ok(()) => {
                info!(
                    stage = "last_capture_clipboard",
                    completed = true,
                    "last capture copied to clipboard by explicit user action"
                );
                self.tray.notify(
                    "AskBridge 已复制上次截图",
                    "现在可以在目标网页输入区按 Ctrl+V。",
                );
            }
            Err(error) => {
                error!(
                    stage = "last_capture_clipboard",
                    completed = false,
                    "last capture could not be copied to clipboard"
                );
                self.tray.notify(
                    "AskBridge 无法复制上次截图",
                    &super::error_handler::user_facing_error(&error),
                );
            }
        }
    }
}
