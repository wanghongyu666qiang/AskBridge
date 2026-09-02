use std::time::{Duration, SystemTime, UNIX_EPOCH};

use askbridge_core::{
    AppCommand, AppError, BrowserLifecycle, BrowserTargetPreference, CapturedImage, DispatchMode,
    DispatchRequest, PreparationPolicy,
};
use tracing::{error, info};

use crate::adapter::ProviderHealthCheck;
use crate::browser::{
    BrowserEvent, BrowserJob, BrowserLaunch, BrowserLaunchPlan, BrowserStage, BrowserSurface,
    BrowserWarmupJob, ClipboardPasteJob, ClipboardPasteOpenTarget, DedicatedChromeJob,
    DesktopPwaJob, ProviderHealthJob,
};

use super::{controller::Runtime, error_handler::user_facing_error};

impl Runtime {
    pub(super) fn prepare_request(
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
            super::events::next_request_id(created_at_ms),
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
            provider_id = %request.provider_id,
            stage = "request_prepared",
            has_image = request.image.is_some(),
            auto_submit = request.auto_submit,
            "dispatch request prepared for browser surface"
        );
        self.pending_dispatch = Some(request.clone());
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
        let adapter_override = provider.adapter_override.clone();
        let dedicated_job = || DedicatedChromeJob {
            configured_chrome_path: self.config.browser.chrome_path.clone(),
            profile_dir: self.config.browser.profile_dir.clone(),
            connect_timeout: Duration::from_millis(self.config.browser.connect_timeout_ms),
            page_timeout: Duration::from_millis(self.config.browser.page_timeout_ms),
            lifecycle: self.config.browser.lifecycle,
            start_url: provider.start_url.clone(),
            url_patterns: provider.url_patterns.clone(),
        };
        let desktop_job = || DesktopPwaJob {
            provider_id: provider.id.clone(),
            configured_shortcut: self
                .config
                .browser
                .desktop_shortcut(&provider.id)
                .map(str::to_owned),
            start_url: provider.start_url.clone(),
            url_patterns: provider.url_patterns.clone(),
        };
        let clipboard_job = |open_target| ClipboardPasteJob {
            start_url: provider.start_url.clone(),
            title_keywords: crate::paste_mode::provider_title_keywords(
                &provider.id,
                &provider.display_name,
                &provider.start_url,
            ),
            locate_timeout: clipboard_locate_timeout(
                self.config.browser.connect_timeout_ms,
                self.config.browser.page_timeout_ms,
                &open_target,
            ),
            open_target,
        };
        let plan = match self.config.browser.target_preference(&provider.id) {
            BrowserTargetPreference::DesktopPwa => {
                if request.image.is_some() {
                    desktop_pwa_screenshot_plan(clipboard_job(
                        ClipboardPasteOpenTarget::DesktopPwa {
                            provider_id: provider.id.clone(),
                            configured_shortcut: self
                                .config
                                .browser
                                .desktop_shortcut(&provider.id)
                                .map(str::to_owned),
                        },
                    ))
                } else {
                    BrowserLaunchPlan::single(BrowserLaunch::DesktopPwa(desktop_job()))
                }
            }
            BrowserTargetPreference::DedicatedChrome => {
                BrowserLaunchPlan::single(BrowserLaunch::DedicatedChrome(dedicated_job()))
            }
            BrowserTargetPreference::DedicatedChromeThenClipboardPaste => {
                if request.image.is_some() {
                    BrowserLaunchPlan::dedicated_then_clipboard(
                        dedicated_job(),
                        clipboard_job(ClipboardPasteOpenTarget::DefaultBrowser),
                    )
                } else {
                    info!(
                        request_id = %request.id,
                        provider_id = %request.provider_id,
                        stage = "browser_fallback_skipped",
                        completed = true,
                        reason = "text_only_request",
                        "clipboard fallback is unavailable without a screenshot"
                    );
                    BrowserLaunchPlan::single(BrowserLaunch::DedicatedChrome(dedicated_job()))
                }
            }
            BrowserTargetPreference::ClipboardPaste => {
                BrowserLaunchPlan::single(BrowserLaunch::ClipboardPaste(clipboard_job(
                    ClipboardPasteOpenTarget::DefaultBrowser,
                )))
            }
        };
        let opens_desktop_pwa = plan.primary_surface() == BrowserSurface::DesktopPwa;
        let pastes_clipboard = plan.primary_surface() == BrowserSurface::ClipboardPaste;
        let has_clipboard_fallback =
            plan.fallback_surface() == Some(BrowserSurface::ClipboardPaste);
        let job = BrowserJob {
            request: request.clone(),
            policy: match PreparationPolicy::new(self.config.browser.page_timeout_ms) {
                Ok(policy) => policy,
                Err(error) => {
                    self.browser_workflow_failed(error);
                    return;
                }
            },
            adapter_override,
            plan,
        };
        if let Err(error) = self.browser.prepare(job) {
            self.browser_workflow_failed(error);
            return;
        }
        if opens_desktop_pwa {
            self.tray.notify(
                "AskBridge 正在打开桌面网页端",
                "将复用现有桌面快捷方式和其中的登录状态。",
            );
        } else if pastes_clipboard {
            self.tray.notify(
                "AskBridge 正在通过剪贴板粘贴",
                "将聚焦目标网页并模拟一次 Ctrl+V；请保持目标窗口可见。",
            );
        } else if has_clipboard_fallback {
            self.tray.notify(
                "AskBridge 正在打开专用 Chrome",
                "如果浏览器控制在写入前失败，将自动改用一次 Ctrl+V 粘贴截图。",
            );
        } else {
            self.tray.notify(
                "AskBridge 正在打开专用 Chrome",
                "首次使用时，请在这个独立浏览器中自行登录目标网站。",
            );
        }
    }

    pub(super) fn handle_browser_events(&mut self) {
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
                        provider_id = self
                            .pending_dispatch
                            .as_ref()
                            .map_or("", |request| request.provider_id.as_str()),
                        stage = ?stage,
                        completed = true,
                        "dedicated browser workflow advanced"
                    );
                }
                BrowserEvent::Prepared {
                    request_id,
                    surface,
                    outcome,
                } => {
                    if !self.is_current_request(&request_id) {
                        continue;
                    }
                    let ready = match surface {
                        BrowserSurface::DedicatedChrome => self.workflow.page_ready(),
                        BrowserSurface::DesktopPwa => self.workflow.desktop_surface_ready(),
                        BrowserSurface::ClipboardPaste => self.workflow.clipboard_surface_ready(),
                    };
                    if let Err(error) = ready {
                        self.browser_workflow_failed(error);
                        return;
                    }
                    if let Err(error) = self.workflow.page_prepared() {
                        self.browser_workflow_failed(error);
                        return;
                    }
                    info!(
                        request_id = %request_id,
                        provider_id = self
                            .pending_dispatch
                            .as_ref()
                            .map_or("", |request| request.provider_id.as_str()),
                        stage = "page_preparation",
                        completed = true,
                        text_inserted = outcome.text_inserted,
                        attachment_prepared = outcome.attachment_prepared,
                        "page preparation completed"
                    );
                    let prepared_message = if surface == BrowserSurface::ClipboardPaste {
                        "已向目标窗口发送 Ctrl+V；请确认截图已出现，再手动输入问题并发送。"
                    } else {
                        self.pending_dispatch.as_ref().map_or(
                            "网页已打开；请在输入框中输入问题并手动发送。",
                            |request| {
                                if request.expects_text() && request.image.is_some() {
                                    "文字和附件已验证就绪；请在网页中确认后手动发送。若附件随后报错，可从托盘复制上次截图。"
                                } else if request.image.is_some() {
                                    "截图已放入网页输入区；若网页随后报错，可从托盘复制上次截图后手动 Ctrl+V。"
                                } else if request.expects_text() {
                                    "文字已放入网页输入区；请确认后手动发送。"
                                } else {
                                    "网页已打开；请在输入框中输入问题并手动发送。"
                                }
                            },
                        )
                    };
                    self.pending_dispatch = None;
                    if let Err(error) = self.workflow.finish_delivery() {
                        self.browser_workflow_failed(error);
                        return;
                    }
                    self.tray
                        .notify("AskBridge 已准备网页内容", prepared_message);
                    if surface == BrowserSurface::DedicatedChrome
                        && self.config.browser.lifecycle == BrowserLifecycle::CloseAfterDispatch
                        && crate::util::confirm_close_managed_browser(self._main_window.hwnd())
                        && let Err(error) = self.browser.close_managed()
                    {
                        self.tray
                            .notify("AskBridge 无法关闭专用 Chrome", &user_facing_error(&error));
                    }
                }
                BrowserEvent::FallbackStarted {
                    request_id,
                    from,
                    to,
                } => {
                    if !self.is_current_request(&request_id) {
                        continue;
                    }
                    if let Err(error) = self.workflow.restart_browser_attempt() {
                        self.browser_workflow_failed(error);
                        return;
                    }
                    info!(
                        request_id = %request_id,
                        provider_id = self
                            .pending_dispatch
                            .as_ref()
                            .map_or("", |request| request.provider_id.as_str()),
                        stage = "browser_fallback",
                        completed = true,
                        from = ?from,
                        to = ?to,
                        "browser preparation switched to the configured fallback"
                    );
                    self.tray.notify(
                        "AskBridge 已切换到通用粘贴",
                        "专用浏览器在写入前失败；现在将聚焦目标窗口并模拟一次 Ctrl+V。",
                    );
                }
                BrowserEvent::WarmupReady => {
                    info!(
                        stage = "browser_warmup",
                        completed = true,
                        "dedicated browser is connected"
                    );
                    self.settings
                        .set_status("AskBridge 专用 Chrome 已启动，CDP 回环连接正常。");
                    self.tray.notify(
                        "AskBridge 专用 Chrome 已就绪",
                        "浏览器已启动并通过本机回环连接验证。",
                    );
                }
                BrowserEvent::WarmupFailed { error } => {
                    error!(
                        stage = "browser_warmup",
                        completed = false,
                        "dedicated browser startup warmup failed"
                    );
                    self.settings.set_status(&format!(
                        "AskBridge 专用 Chrome 操作失败：{}",
                        user_facing_error(&error)
                    ));
                    self.tray
                        .notify("AskBridge 专用 Chrome 启动失败", &user_facing_error(&error));
                }
                BrowserEvent::ProviderHealthCompleted { reports } => {
                    self.settings.set_provider_health(&reports);
                    self.settings
                        .set_status("供应商能力检测完成；未写入文本、未上传图片、未发送消息。");
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

    pub(super) fn warmup_browser_if_configured(&self) {
        if self.config.browser.lifecycle != BrowserLifecycle::OnStartup {
            return;
        }
        let job = BrowserWarmupJob {
            configured_chrome_path: self.config.browser.chrome_path.clone(),
            profile_dir: self.config.browser.profile_dir.clone(),
            connect_timeout: Duration::from_millis(self.config.browser.connect_timeout_ms),
            page_timeout: Duration::from_millis(self.config.browser.page_timeout_ms),
            open_url: None,
        };
        if self.browser.warmup(job).is_err() {
            error!(
                stage = "browser_warmup_queue",
                completed = false,
                "failed to queue dedicated browser startup"
            );
        }
    }

    pub(super) fn start_provider_health_check(&mut self) {
        if !self.workflow.is_idle() {
            self.settings
                .set_status("当前问答流程尚未结束，完成后再检测供应商能力。");
            return;
        }
        let providers = match self.config.merged_providers() {
            Ok(providers) => providers
                .into_iter()
                .filter(|provider| provider.enabled && !provider.is_custom)
                .map(|provider| ProviderHealthCheck {
                    provider_id: provider.id,
                    start_url: provider.start_url,
                    url_patterns: provider.url_patterns,
                    adapter_override: provider.adapter_override,
                })
                .collect::<Vec<_>>(),
            Err(error) => {
                self.settings.set_status(&format!(
                    "供应商检测无法开始：{}",
                    user_facing_error(&error)
                ));
                return;
            }
        };
        self.settings
            .set_status("正在检测供应商页面、登录、输入框、图片与焦点能力……");
        let job = ProviderHealthJob {
            configured_chrome_path: self.config.browser.chrome_path.clone(),
            profile_dir: self.config.browser.profile_dir.clone(),
            connect_timeout: Duration::from_millis(self.config.browser.connect_timeout_ms),
            page_timeout: Duration::from_millis(self.config.browser.page_timeout_ms),
            providers,
        };
        if let Err(error) = self.browser.check_providers(job) {
            self.settings.set_status(&format!(
                "供应商检测无法开始：{}",
                user_facing_error(&error)
            ));
        }
    }

    fn is_current_request(&self, request_id: &str) -> bool {
        self.pending_dispatch
            .as_ref()
            .is_some_and(|request| request.id == request_id)
    }

    pub(super) fn cancel_workflow(&mut self) {
        self.pending_dispatch = None;
        self.browser.cancel();
        if self.workflow.is_idle() {
            return;
        }
        if self.workflow.state() != askbridge_core::AppState::Cancelling {
            let _ = self.workflow.begin_cancelling();
        }
        if self.workflow.state() == askbridge_core::AppState::Cancelling {
            let _ = self.workflow.finish_cancelling();
        }
    }
}

fn desktop_pwa_screenshot_plan(clipboard: ClipboardPasteJob) -> BrowserLaunchPlan {
    BrowserLaunchPlan::single(BrowserLaunch::ClipboardPaste(clipboard))
}

fn clipboard_locate_timeout(
    connect_timeout_ms: u64,
    page_timeout_ms: u64,
    open_target: &ClipboardPasteOpenTarget,
) -> Duration {
    let page_timeout = Duration::from_millis(page_timeout_ms.max(5_000));
    match open_target {
        ClipboardPasteOpenTarget::DesktopPwa { .. } => Duration::from_millis(connect_timeout_ms)
            .saturating_add(page_timeout)
            .min(Duration::from_secs(120)),
        ClipboardPasteOpenTarget::DefaultBrowser => page_timeout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_pwa_screenshot_keeps_pwa_target_and_adds_clipboard_delivery() {
        let clipboard = ClipboardPasteJob {
            start_url: "https://ai.example/".to_owned(),
            title_keywords: vec!["Custom AI".to_owned()],
            open_target: ClipboardPasteOpenTarget::DesktopPwa {
                provider_id: "custom-ai".to_owned(),
                configured_shortcut: Some(r"D:\AI\Custom AI.lnk".to_owned()),
            },
            locate_timeout: Duration::from_secs(5),
        };

        let plan = desktop_pwa_screenshot_plan(clipboard);
        assert_eq!(plan.primary_surface(), BrowserSurface::ClipboardPaste);
        assert!(matches!(
            plan.primary(),
            BrowserLaunch::ClipboardPaste(ClipboardPasteJob {
                open_target: ClipboardPasteOpenTarget::DesktopPwa { provider_id, .. },
                ..
            }) if provider_id == "custom-ai"
        ));
    }

    #[test]
    fn desktop_pwa_cold_open_gets_process_startup_plus_page_readiness_budget() {
        let desktop = ClipboardPasteOpenTarget::DesktopPwa {
            provider_id: "chatgpt".to_owned(),
            configured_shortcut: None,
        };
        assert_eq!(
            clipboard_locate_timeout(10_000, 10_000, &desktop),
            Duration::from_secs(20)
        );
        assert_eq!(
            clipboard_locate_timeout(10_000, 10_000, &ClipboardPasteOpenTarget::DefaultBrowser),
            Duration::from_secs(10)
        );
        assert_eq!(
            clipboard_locate_timeout(120_000, 120_000, &desktop),
            Duration::from_secs(120)
        );
    }
}
