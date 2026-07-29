use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{AppCommand, AppError, HotkeyBinding, ModifierKey, ProviderConfig, Result, VirtualKey};

pub const CURRENT_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_QUICK_PROMPT: &str = "请分析这张截图，并解释其中的内容。";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "current_schema_version")]
    pub schema_version: u32,
    #[serde(default = "default_provider_id")]
    pub default_provider_id: String,
    #[serde(default)]
    pub hotkeys: HotkeyConfig,
    #[serde(default = "default_quick_prompt")]
    pub quick_prompt: String,
    #[serde(default = "default_providers")]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub browser: BrowserConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            default_provider_id: default_provider_id(),
            hotkeys: HotkeyConfig::default(),
            quick_prompt: default_quick_prompt(),
            providers: default_providers(),
            general: GeneralConfig::default(),
            browser: BrowserConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version > CURRENT_SCHEMA_VERSION {
            return Err(AppError::UnsupportedConfigurationSchema {
                found: self.schema_version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
        if self.schema_version == 0 {
            return Err(AppError::ConfigurationInvalid(
                "schema_version must be at least 1".to_owned(),
            ));
        }
        self.hotkeys.validate()?;
        if self.quick_prompt.trim().is_empty() {
            return Err(AppError::ConfigurationInvalid(
                "quick_prompt must not be empty".to_owned(),
            ));
        }
        let mut ids = HashSet::new();
        for provider in &self.providers {
            provider.validate()?;
            if !ids.insert(provider.id.as_str()) {
                return Err(AppError::ConfigurationInvalid(format!(
                    "provider id '{}' is duplicated",
                    provider.id
                )));
            }
        }
        if !self
            .providers
            .iter()
            .any(|provider| provider.enabled && provider.id == self.default_provider_id)
        {
            return Err(AppError::ConfigurationInvalid(format!(
                "default provider '{}' is missing or disabled",
                self.default_provider_id
            )));
        }
        if self.browser.target_timeout_ms == 0 {
            return Err(AppError::ConfigurationInvalid(
                "target_timeout_ms must be greater than zero".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyConfig {
    #[serde(default = "default_capture_with_prompt")]
    pub capture_with_prompt: HotkeyBinding,
    #[serde(default = "default_capture_quick_dispatch")]
    pub capture_quick_dispatch: HotkeyBinding,
    #[serde(default = "default_text_only_prompt")]
    pub text_only_prompt: HotkeyBinding,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            capture_with_prompt: default_capture_with_prompt(),
            capture_quick_dispatch: default_capture_quick_dispatch(),
            text_only_prompt: default_text_only_prompt(),
        }
    }
}

impl HotkeyConfig {
    pub fn binding(&self, command: AppCommand) -> &HotkeyBinding {
        match command {
            AppCommand::CaptureWithPrompt => &self.capture_with_prompt,
            AppCommand::CaptureQuickDispatch => &self.capture_quick_dispatch,
            AppCommand::TextOnlyPrompt => &self.text_only_prompt,
        }
    }

    pub fn binding_mut(&mut self, command: AppCommand) -> &mut HotkeyBinding {
        match command {
            AppCommand::CaptureWithPrompt => &mut self.capture_with_prompt,
            AppCommand::CaptureQuickDispatch => &mut self.capture_quick_dispatch,
            AppCommand::TextOnlyPrompt => &mut self.text_only_prompt,
        }
    }

    pub fn validate(&self) -> Result<()> {
        let mut active = HashSet::new();
        for command in AppCommand::ALL {
            let binding = self.binding(command);
            if !binding.enabled {
                continue;
            }
            binding
                .validate()
                .map_err(|error| AppError::InvalidHotkey(error.to_string()))?;
            if !active.insert(binding.clone()) {
                return Err(AppError::HotkeyConflict(format!(
                    "{} duplicates another AskBridge command",
                    binding
                )));
            }
        }
        Ok(())
    }

    pub fn restore_defaults(&mut self) {
        *self = Self::default();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default)]
    pub start_on_login: bool,
    #[serde(default = "default_true")]
    pub reuse_open_provider_tab: bool,
    #[serde(default)]
    pub auto_submit: bool,
    #[serde(default = "default_true")]
    pub restore_clipboard: bool,
    #[serde(default)]
    pub debug_logging: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            start_on_login: false,
            reuse_open_provider_tab: true,
            auto_submit: false,
            restore_clipboard: true,
            debug_logging: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserConfig {
    #[serde(default = "default_preferred_browser")]
    pub preferred_browser: String,
    #[serde(default = "default_target_timeout_ms")]
    pub target_timeout_ms: u32,
    #[serde(default = "default_paste_retry_count")]
    pub paste_retry_count: u8,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            preferred_browser: default_preferred_browser(),
            target_timeout_ms: default_target_timeout_ms(),
            paste_retry_count: default_paste_retry_count(),
        }
    }
}

const fn current_schema_version() -> u32 {
    CURRENT_SCHEMA_VERSION
}

fn default_provider_id() -> String {
    "chatgpt".to_owned()
}

fn default_quick_prompt() -> String {
    DEFAULT_QUICK_PROMPT.to_owned()
}

fn default_capture_with_prompt() -> HotkeyBinding {
    HotkeyBinding::new(true, vec![ModifierKey::Alt], VirtualKey::Letter('Q'))
}

fn default_capture_quick_dispatch() -> HotkeyBinding {
    HotkeyBinding::new(
        true,
        vec![ModifierKey::Alt, ModifierKey::Shift],
        VirtualKey::Letter('Q'),
    )
}

fn default_text_only_prompt() -> HotkeyBinding {
    HotkeyBinding::new(true, vec![ModifierKey::Alt], VirtualKey::Letter('A'))
}

fn default_true() -> bool {
    true
}

fn default_preferred_browser() -> String {
    "system_default".to_owned()
}

const fn default_target_timeout_ms() -> u32 {
    10_000
}

const fn default_paste_retry_count() -> u8 {
    3
}

fn default_providers() -> Vec<ProviderConfig> {
    vec![
        ProviderConfig {
            id: "chatgpt".to_owned(),
            display_name: "ChatGPT".to_owned(),
            enabled: true,
            new_chat_url: "https://chatgpt.com/".to_owned(),
            url_match_patterns: vec!["https://chatgpt.com/".to_owned()],
            browser_profile: None,
            is_custom: false,
        },
        ProviderConfig {
            id: "gemini".to_owned(),
            display_name: "Gemini".to_owned(),
            enabled: true,
            new_chat_url: "https://gemini.google.com/app".to_owned(),
            url_match_patterns: vec!["https://gemini.google.com/".to_owned()],
            browser_profile: None,
            is_custom: false,
        },
        ProviderConfig {
            id: "claude".to_owned(),
            display_name: "Claude".to_owned(),
            enabled: true,
            new_chat_url: "https://claude.ai/new".to_owned(),
            url_match_patterns: vec!["https://claude.ai/".to_owned()],
            browser_profile: None,
            is_custom: false,
        },
        ProviderConfig {
            id: "doubao".to_owned(),
            display_name: "豆包".to_owned(),
            enabled: true,
            new_chat_url: "https://www.doubao.com/chat/".to_owned(),
            url_match_patterns: vec!["https://www.doubao.com/".to_owned()],
            browser_profile: None,
            is_custom: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid_and_has_required_hotkeys() {
        let config = AppConfig::default();
        config.validate().expect("default config should be valid");
        assert_eq!(config.hotkeys.capture_with_prompt.to_string(), "Alt+Q");
        assert_eq!(
            config.hotkeys.capture_quick_dispatch.to_string(),
            "Alt+Shift+Q"
        );
        assert_eq!(config.hotkeys.text_only_prompt.to_string(), "Alt+A");
    }

    #[test]
    fn detects_internal_hotkey_conflict() {
        let mut config = AppConfig::default();
        config.hotkeys.text_only_prompt = config.hotkeys.capture_with_prompt.clone();
        assert!(matches!(
            config.validate(),
            Err(AppError::HotkeyConflict(_))
        ));
    }

    #[test]
    fn disabled_hotkeys_do_not_conflict() {
        let mut config = AppConfig::default();
        config.hotkeys.text_only_prompt = config.hotkeys.capture_with_prompt.clone();
        config.hotkeys.text_only_prompt.enabled = false;
        config.validate().expect("disabled duplicate is allowed");
    }

    #[test]
    fn missing_fields_receive_defaults() {
        let config: AppConfig =
            serde_json::from_str(r#"{"schema_version":1}"#).expect("parse partial config");
        assert_eq!(config.default_provider_id, "chatgpt");
        assert_eq!(config.hotkeys, HotkeyConfig::default());
        assert_eq!(config.providers.len(), 4);
    }

    #[test]
    fn rejects_newer_schema() {
        let config = AppConfig {
            schema_version: CURRENT_SCHEMA_VERSION + 1,
            ..AppConfig::default()
        };
        assert!(matches!(
            config.validate(),
            Err(AppError::UnsupportedConfigurationSchema { .. })
        ));
    }

    #[test]
    fn restores_default_hotkeys() {
        let mut hotkeys = HotkeyConfig::default();
        hotkeys.capture_with_prompt.enabled = false;
        hotkeys.restore_defaults();
        assert_eq!(hotkeys, HotkeyConfig::default());
    }
}
