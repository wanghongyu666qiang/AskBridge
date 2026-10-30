//! The read-only payload rendered by the settings page.
//!
//! The page never reads configuration files: it receives one snapshot of the
//! configuration in effect, plus the live status line and provider health
//! results that the runtime pushes while the window is open.

use std::path::Path;

use askbridge_core::{
    AppCommand, AppConfig, BrowserLifecycle, BrowserTargetPreference, ProviderConfig, Result,
};
use serde::Serialize;

use crate::adapter::{ProviderHealth, ProviderHealthReport};

/// Sidebar navigation entries. The order here is the order the page renders.
#[derive(Serialize)]
pub(super) struct NavItem {
    pub(super) id: &'static str,
    pub(super) label: &'static str,
}

pub(super) const NAV_ITEMS: [NavItem; 4] = [
    NavItem {
        id: "hotkeys",
        label: "快捷键",
    },
    NavItem {
        id: "providers",
        label: "供应商",
    },
    NavItem {
        id: "browser",
        label: "浏览器",
    },
    NavItem {
        id: "general",
        label: "常规",
    },
];

#[derive(Serialize)]
pub(super) struct HotkeyViewModel {
    pub(super) id: &'static str,
    pub(super) label: &'static str,
    pub(super) description: &'static str,
    pub(super) text: String,
    pub(super) enabled: bool,
}

#[derive(Serialize)]
pub(super) struct ProviderViewModel {
    pub(super) id: String,
    pub(super) display_name: String,
    pub(super) enabled: bool,
    pub(super) start_url: String,
    /// One of `unknown`, `healthy`, `login_required`, `composer_missing`,
    /// `attachment_unsupported`, `network_error`.
    pub(super) health: &'static str,
    pub(super) health_label: &'static str,
}

#[derive(Serialize)]
pub(super) struct CustomProviderViewModel {
    pub(super) id: String,
    pub(super) display_name: String,
    pub(super) start_url: String,
    pub(super) url_patterns: String,
}

#[derive(Serialize)]
pub(super) struct LifecycleOption {
    pub(super) value: &'static str,
    pub(super) label: &'static str,
}

#[derive(Serialize)]
pub(super) struct TargetOption {
    pub(super) value: &'static str,
    pub(super) label: &'static str,
    pub(super) description: &'static str,
}

/// Serialized once per push and handed to the page as JSON.
#[derive(Serialize)]
pub(super) struct SettingsViewModel {
    pub(super) version: &'static str,
    pub(super) nav: &'static [NavItem],
    pub(super) hotkeys: Vec<HotkeyViewModel>,
    pub(super) default_provider_id: String,
    pub(super) providers: Vec<ProviderViewModel>,
    pub(super) custom_providers: Vec<CustomProviderViewModel>,
    pub(super) quick_prompt: String,
    pub(super) start_on_login: bool,
    pub(super) debug_logging: bool,
    pub(super) chrome_path: String,
    pub(super) lifecycle: &'static str,
    pub(super) lifecycles: Vec<LifecycleOption>,
    pub(super) chatgpt_target: &'static str,
    pub(super) targets: Vec<TargetOption>,
    pub(super) data_root: String,
    pub(super) status: String,
}

const LIFECYCLES: [(&str, &str); 4] = [
    ("on_demand_keep_running", "按需启动，保持运行"),
    ("on_demand_idle_close", "按需启动，空闲十分钟关闭"),
    ("close_after_dispatch", "每次准备后询问关闭"),
    ("on_startup", "随 AskBridge 启动"),
];

const TARGETS: [(&str, &str, &str); 4] = [
    (
        "desktop_pwa",
        "桌面网页端",
        "复用现有登录，但截图需要手动上传",
    ),
    (
        "dedicated_chrome",
        "AskBridge 专用 Chrome",
        "支持自动上传图片，需要单独登录",
    ),
    (
        "clipboard_paste",
        "通用粘贴",
        "支持浏览器或 AI 桌面端，仅模拟 Ctrl+V（不验证结果）",
    ),
    (
        "dedicated_chrome_then_clipboard_paste",
        "专用 Chrome 优先",
        "安全失败后自动用 Ctrl+V 粘贴截图",
    ),
];

const HOTKEYS: [(&str, AppCommand, &str); 3] = [
    (
        "capture_with_prompt",
        AppCommand::CaptureWithPrompt,
        "截取区域后输入问题",
    ),
    (
        "capture_quick_dispatch",
        AppCommand::CaptureQuickDispatch,
        "使用默认供应商和快速提示词",
    ),
    (
        "text_only_prompt",
        AppCommand::TextOnlyPrompt,
        "不截图，直接打开输入框",
    ),
];

/// `BrowserLifecycle` has no string form of its own, so the page-facing value
/// is derived here and must stay in sync with [`LIFECYCLES`].
pub(super) const fn lifecycle_value(lifecycle: BrowserLifecycle) -> &'static str {
    match lifecycle {
        BrowserLifecycle::OnDemandKeepRunning => "on_demand_keep_running",
        BrowserLifecycle::OnDemandIdleClose => "on_demand_idle_close",
        BrowserLifecycle::CloseAfterDispatch => "close_after_dispatch",
        BrowserLifecycle::OnStartup => "on_startup",
    }
}

pub(super) const fn target_value(target: BrowserTargetPreference) -> &'static str {
    match target {
        BrowserTargetPreference::DesktopPwa => "desktop_pwa",
        BrowserTargetPreference::DedicatedChrome => "dedicated_chrome",
        BrowserTargetPreference::ClipboardPaste => "clipboard_paste",
        BrowserTargetPreference::DedicatedChromeThenClipboardPaste => {
            "dedicated_chrome_then_clipboard_paste"
        }
    }
}

/// Maps a health report to the page's status vocabulary. Kept next to the
/// view model so the labels and the CSS classes cannot drift apart.
pub(super) const fn health_status(health: ProviderHealth) -> (&'static str, &'static str) {
    match health {
        ProviderHealth::Healthy => ("healthy", "页面/输入框/图片均可用"),
        ProviderHealth::LoginRequired => ("login_required", "需要登录"),
        ProviderHealth::ComposerMissing => ("composer_missing", "输入框检测失败"),
        ProviderHealth::AttachmentUnsupported => ("attachment_unsupported", "输入框正常；无图片"),
        ProviderHealth::NetworkError => ("network_error", "页面访问失败"),
    }
}

impl SettingsViewModel {
    /// Builds the page payload from the configuration in effect.
    pub(super) fn build(
        config: &AppConfig,
        data_root: &Path,
        status: &str,
        health: &[ProviderHealthReport],
    ) -> Result<Self> {
        let providers = config.merged_providers()?;
        let hotkeys = HOTKEYS
            .iter()
            .map(|(id, command, description)| HotkeyViewModel {
                id,
                label: command.label(),
                description,
                text: config.hotkeys.binding(*command).to_string(),
                enabled: config.hotkeys.binding(*command).enabled,
            })
            .collect();
        let chatgpt_target = config.browser.target_preference("chatgpt");

        Ok(Self {
            version: env!("CARGO_PKG_VERSION"),
            nav: &NAV_ITEMS,
            hotkeys,
            default_provider_id: config.default_provider_id.clone(),
            providers: providers
                .iter()
                .map(|provider| provider_view_model(provider, health))
                .collect(),
            custom_providers: config
                .custom_providers
                .iter()
                .map(custom_provider_view_model)
                .collect(),
            quick_prompt: config.quick_prompt.clone(),
            start_on_login: config.general.start_on_login,
            debug_logging: config.general.debug_logging,
            chrome_path: config.browser.chrome_path.clone().unwrap_or_default(),
            lifecycle: lifecycle_value(config.browser.lifecycle),
            lifecycles: LIFECYCLES
                .iter()
                .map(|(value, label)| LifecycleOption { value, label })
                .collect(),
            chatgpt_target: target_value(chatgpt_target),
            targets: TARGETS
                .iter()
                .map(|(value, label, description)| TargetOption {
                    value,
                    label,
                    description,
                })
                .collect(),
            data_root: data_root.to_string_lossy().into_owned(),
            status: status.to_owned(),
        })
    }

    pub(super) fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|error| {
            askbridge_core::AppError::ConfigurationInvalid(format!(
                "settings view model failed to serialize: {error}"
            ))
        })
    }
}

fn provider_view_model(
    provider: &ProviderConfig,
    health: &[ProviderHealthReport],
) -> ProviderViewModel {
    let report = health
        .iter()
        .find(|report| report.provider_id == provider.id);
    let (status, label) = report.map_or(("unknown", "未检测"), |report| {
        health_status(report.health)
    });
    ProviderViewModel {
        id: provider.id.clone(),
        display_name: provider.display_name.clone(),
        enabled: provider.enabled,
        start_url: provider.start_url.clone(),
        health: status,
        health_label: label,
    }
}

fn custom_provider_view_model(provider: &ProviderConfig) -> CustomProviderViewModel {
    CustomProviderViewModel {
        id: provider.id.clone(),
        display_name: provider.display_name.clone(),
        start_url: provider.start_url.clone(),
        url_patterns: provider.url_patterns.join(", "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use askbridge_core::provider::built_in_providers;

    fn built_in_ids() -> Vec<String> {
        built_in_providers()
            .into_iter()
            .map(|provider| provider.id)
            .collect()
    }

    #[test]
    fn view_model_covers_every_built_in_provider_and_hotkey() {
        let config = AppConfig::default();
        let model = SettingsViewModel::build(&config, Path::new("D:/data"), "准备就绪。", &[])
            .expect("view model");

        assert_eq!(model.providers.len(), built_in_ids().len());
        assert_eq!(model.hotkeys.len(), AppCommand::ALL.len());
        assert_eq!(model.nav.len(), 4);
        assert_eq!(model.lifecycles.len(), 4);
        assert_eq!(model.targets.len(), 4);
        assert!(model.providers.iter().all(|row| row.health == "unknown"));
        assert_eq!(model.status, "准备就绪。");
    }

    #[test]
    fn view_model_reports_provider_health_by_id() {
        let config = AppConfig::default();
        let reports = vec![ProviderHealthReport {
            provider_id: "doubao".to_owned(),
            health: ProviderHealth::LoginRequired,
            page_accessible: true,
            logged_in: false,
            composer_found: true,
            attachment_supported: false,
            focus_supported: true,
        }];
        let model =
            SettingsViewModel::build(&config, Path::new("D:/data"), "", &reports).expect("model");
        let doubao = model
            .providers
            .iter()
            .find(|row| row.id == "doubao")
            .expect("doubao row");

        assert_eq!(doubao.health, "login_required");
        assert_eq!(doubao.health_label, "需要登录");
        assert!(
            model
                .providers
                .iter()
                .filter(|row| row.id != "doubao")
                .all(|row| row.health == "unknown")
        );
    }

    #[test]
    fn view_model_serializes_to_the_camel_free_contract_the_page_reads() {
        let config = AppConfig::default();
        let json = SettingsViewModel::build(&config, Path::new("D:/data"), "", &[])
            .expect("model")
            .to_json()
            .expect("json");

        assert!(json.contains("\"default_provider_id\""));
        assert!(json.contains("\"start_on_login\""));
        assert!(json.contains("\"health_label\""));
        assert!(json.contains("\"data_root\""));
        // The page must never receive the retired auto-submit tombstone.
        assert!(!json.contains("auto_submit"));
    }

    #[test]
    fn lifecycle_and_target_values_match_their_option_lists() {
        for (value, _) in LIFECYCLES {
            assert!(
                [
                    BrowserLifecycle::OnDemandKeepRunning,
                    BrowserLifecycle::OnDemandIdleClose,
                    BrowserLifecycle::CloseAfterDispatch,
                    BrowserLifecycle::OnStartup,
                ]
                .iter()
                .any(|lifecycle| lifecycle_value(*lifecycle) == value),
                "lifecycle option '{value}' has no matching enum value"
            );
        }
        for (value, _, _) in TARGETS {
            assert!(
                [
                    BrowserTargetPreference::DesktopPwa,
                    BrowserTargetPreference::DedicatedChrome,
                    BrowserTargetPreference::ClipboardPaste,
                    BrowserTargetPreference::DedicatedChromeThenClipboardPaste,
                ]
                .iter()
                .any(|target| target_value(*target) == value),
                "target option '{value}' has no matching enum value"
            );
        }
    }
}
