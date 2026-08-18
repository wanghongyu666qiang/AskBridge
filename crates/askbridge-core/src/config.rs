use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    AppCommand, AppError, HotkeyBinding, ModifierKey, ProviderConfig, ProviderOverride, Result,
    VirtualKey, provider::built_in_providers,
};

pub const CURRENT_SCHEMA_VERSION: u32 = 3;
pub const DEFAULT_QUICK_PROMPT: &str = "请分析这张截图，并解释其中的内容。";
const MAX_BROWSER_TIMEOUT_MS: u64 = 120_000;

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
    #[serde(default)]
    pub general: GeneralConfig,
    #[serde(default)]
    pub browser: BrowserConfig,
    #[serde(default)]
    pub provider_overrides: Vec<ProviderOverride>,
    #[serde(default)]
    pub custom_providers: Vec<ProviderConfig>,
    #[serde(default, rename = "providers", skip_serializing)]
    legacy_providers: Vec<ProviderConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            default_provider_id: default_provider_id(),
            hotkeys: HotkeyConfig::default(),
            quick_prompt: default_quick_prompt(),
            general: GeneralConfig::default(),
            browser: BrowserConfig::default(),
            provider_overrides: Vec::new(),
            custom_providers: Vec::new(),
            legacy_providers: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn migrate(&mut self) -> Result<bool> {
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

        let mut changed = self.schema_version != CURRENT_SCHEMA_VERSION;
        if !self.legacy_providers.is_empty() {
            if !self.provider_overrides.is_empty() || !self.custom_providers.is_empty() {
                return Err(AppError::ConfigurationInvalid(
                    "legacy and current provider configuration cannot be mixed".to_owned(),
                ));
            }
            self.import_legacy_providers()?;
            changed = true;
        }
        if self.general.auto_submit {
            self.general.auto_submit = false;
            changed = true;
        }
        self.schema_version = CURRENT_SCHEMA_VERSION;

        let providers = self.merged_providers()?;
        if !providers
            .iter()
            .any(|provider| provider.enabled && provider.id == self.default_provider_id)
        {
            let fallback = providers
                .iter()
                .find(|provider| provider.enabled && !provider.is_custom)
                .ok_or_else(|| {
                    AppError::ConfigurationInvalid(
                        "at least one built-in provider must remain enabled".to_owned(),
                    )
                })?;
            self.default_provider_id.clone_from(&fallback.id);
            changed = true;
        }

        self.validate()?;
        Ok(changed)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(AppError::ConfigurationInvalid(
                "configuration must be migrated before validation".to_owned(),
            ));
        }
        self.hotkeys.validate()?;
        if self.quick_prompt.trim().is_empty() {
            return Err(AppError::ConfigurationInvalid(
                "quick_prompt must not be empty".to_owned(),
            ));
        }
        if self.general.auto_submit {
            return Err(AppError::ConfigurationInvalid(
                "auto_submit must remain false in AskBridge 1.0".to_owned(),
            ));
        }
        self.browser.validate()?;

        let providers = self.merged_providers()?;
        if !providers
            .iter()
            .any(|provider| provider.enabled && provider.id == self.default_provider_id)
        {
            return Err(AppError::ConfigurationInvalid(format!(
                "default provider '{}' is missing or disabled",
                self.default_provider_id
            )));
        }
        Ok(())
    }

    pub fn merged_providers(&self) -> Result<Vec<ProviderConfig>> {
        let mut providers = built_in_providers();
        let mut built_in_indices = providers
            .iter()
            .enumerate()
            .map(|(index, provider)| (provider.id.clone(), index))
            .collect::<HashMap<_, _>>();
        let mut override_ids = HashSet::new();

        for provider_override in &self.provider_overrides {
            provider_override.validate()?;
            if !override_ids.insert(provider_override.id.as_str()) {
                return Err(AppError::ConfigurationInvalid(format!(
                    "provider override '{}' is duplicated",
                    provider_override.id
                )));
            }
            let Some(index) = built_in_indices.get(&provider_override.id).copied() else {
                return Err(AppError::InvalidProvider(format!(
                    "provider override '{}' does not match a built-in provider",
                    provider_override.id
                )));
            };
            providers[index].apply_override(provider_override);
            providers[index].validate()?;
        }

        for provider in &self.custom_providers {
            provider.validate()?;
            if built_in_indices.contains_key(&provider.id) {
                return Err(AppError::ConfigurationInvalid(format!(
                    "custom provider id '{}' conflicts with a built-in provider",
                    provider.id
                )));
            }
            if !provider.is_custom {
                return Err(AppError::ConfigurationInvalid(format!(
                    "custom provider '{}' must set is_custom to true",
                    provider.id
                )));
            }
            let index = providers.len();
            if built_in_indices
                .insert(provider.id.clone(), index)
                .is_some()
            {
                return Err(AppError::ConfigurationInvalid(format!(
                    "custom provider id '{}' is duplicated",
                    provider.id
                )));
            }
            providers.push(provider.clone());
        }
        Ok(providers)
    }

    fn import_legacy_providers(&mut self) -> Result<()> {
        let built_ins = built_in_providers()
            .into_iter()
            .map(|provider| (provider.id.clone(), provider))
            .collect::<HashMap<_, _>>();
        let legacy_providers = std::mem::take(&mut self.legacy_providers);

        for mut provider in legacy_providers {
            provider.validate()?;
            if let Some(built_in) = built_ins.get(&provider.id) {
                let provider_override = ProviderOverride {
                    id: provider.id,
                    display_name: (provider.display_name != built_in.display_name)
                        .then_some(provider.display_name),
                    enabled: (provider.enabled != built_in.enabled).then_some(provider.enabled),
                    start_url: (provider.start_url != built_in.start_url)
                        .then_some(provider.start_url),
                    url_patterns: (provider.url_patterns != built_in.url_patterns)
                        .then_some(provider.url_patterns),
                    adapter_override: None,
                };
                if provider_override.display_name.is_some()
                    || provider_override.enabled.is_some()
                    || provider_override.start_url.is_some()
                    || provider_override.url_patterns.is_some()
                {
                    self.provider_overrides.push(provider_override);
                }
            } else {
                provider.is_custom = true;
                provider.adapter_override = None;
                self.custom_providers.push(provider);
            }
        }
        Ok(())
    }
}

impl BrowserConfig {
    fn validate(&self) -> Result<()> {
        if self.profile_dir.trim().is_empty() {
            return Err(AppError::ConfigurationInvalid(
                "browser profile_dir must not be empty".to_owned(),
            ));
        }
        if self.connect_timeout_ms == 0
            || self.page_timeout_ms == 0
            || self.connect_timeout_ms > MAX_BROWSER_TIMEOUT_MS
            || self.page_timeout_ms > MAX_BROWSER_TIMEOUT_MS
        {
            return Err(AppError::ConfigurationInvalid(
                "browser timeouts must be between 1 and 120000 milliseconds".to_owned(),
            ));
        }
        for (provider_id, shortcut) in &self.desktop_shortcuts {
            if provider_id.trim().is_empty() || shortcut.trim().is_empty() {
                return Err(AppError::ConfigurationInvalid(
                    "desktop shortcut mappings require non-empty provider ids and paths".to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub fn target_preference(&self, provider_id: &str) -> BrowserTargetPreference {
        self.target_preferences
            .get(provider_id)
            .copied()
            .unwrap_or_default()
    }

    pub fn desktop_shortcut(&self, provider_id: &str) -> Option<&str> {
        self.desktop_shortcuts.get(provider_id).map(String::as_str)
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

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default)]
    pub start_on_login: bool,
    #[serde(default)]
    pub auto_submit: bool,
    #[serde(default)]
    pub debug_logging: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserConfig {
    #[serde(default)]
    pub chrome_path: Option<String>,
    #[serde(default = "default_profile_dir")]
    pub profile_dir: String,
    #[serde(default)]
    pub lifecycle: BrowserLifecycle,
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
    #[serde(default = "default_page_timeout_ms", alias = "target_timeout_ms")]
    pub page_timeout_ms: u64,
    #[serde(default = "default_target_preferences")]
    pub target_preferences: HashMap<String, BrowserTargetPreference>,
    #[serde(default)]
    pub desktop_shortcuts: HashMap<String, String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            chrome_path: None,
            profile_dir: default_profile_dir(),
            lifecycle: BrowserLifecycle::default(),
            connect_timeout_ms: default_connect_timeout_ms(),
            page_timeout_ms: default_page_timeout_ms(),
            target_preferences: default_target_preferences(),
            desktop_shortcuts: HashMap::new(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserTargetPreference {
    #[default]
    DedicatedChrome,
    DesktopPwa,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserLifecycle {
    #[default]
    OnDemandKeepRunning,
    OnDemandIdleClose,
    CloseAfterDispatch,
    OnStartup,
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
    HotkeyBinding::new(true, vec![ModifierKey::Alt], VirtualKey::Letter('W'))
}

fn default_profile_dir() -> String {
    "BrowserProfile".to_owned()
}

fn default_target_preferences() -> HashMap<String, BrowserTargetPreference> {
    HashMap::from([("chatgpt".to_owned(), BrowserTargetPreference::DesktopPwa)])
}

const fn default_connect_timeout_ms() -> u64 {
    10_000
}

const fn default_page_timeout_ms() -> u64 {
    15_000
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
        assert_eq!(config.hotkeys.text_only_prompt.to_string(), "Alt+W");
        assert_eq!(
            config.browser.target_preference("chatgpt"),
            BrowserTargetPreference::DesktopPwa
        );
        assert_eq!(
            config.browser.target_preference("gemini"),
            BrowserTargetPreference::DedicatedChrome
        );
        assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(config.merged_providers().expect("providers").len(), 4);
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
        let mut config: AppConfig =
            serde_json::from_str(r#"{"schema_version":1}"#).expect("parse partial config");
        assert!(config.migrate().expect("migrate old config"));
        assert_eq!(config.default_provider_id, "chatgpt");
        assert_eq!(config.hotkeys, HotkeyConfig::default());
        assert_eq!(
            config.browser.target_preference("chatgpt"),
            BrowserTargetPreference::DesktopPwa
        );
        assert_eq!(config.merged_providers().expect("providers").len(), 4);
        assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn browser_lifecycle_is_typed_but_keeps_stable_configuration_strings() {
        let mut config: AppConfig = serde_json::from_str(
            r#"{"schema_version":3,"browser":{"lifecycle":"close_after_dispatch"}}"#,
        )
        .expect("known lifecycle");
        config.migrate().expect("current config");
        assert_eq!(
            config.browser.lifecycle,
            BrowserLifecycle::CloseAfterDispatch
        );
        assert!(
            serde_json::to_string(&config)
                .expect("serialize config")
                .contains(r#""lifecycle":"close_after_dispatch""#)
        );
        assert!(
            serde_json::from_str::<AppConfig>(
                r#"{"schema_version":3,"browser":{"lifecycle":"unknown"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_newer_schema() {
        let mut config = AppConfig {
            schema_version: CURRENT_SCHEMA_VERSION + 1,
            ..AppConfig::default()
        };
        assert!(matches!(
            config.migrate(),
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

    #[test]
    fn migrates_legacy_provider_list_to_overrides_and_custom_providers() {
        let mut config: AppConfig = serde_json::from_str(
            r#"{
                "schema_version": 1,
                "default_provider_id": "example",
                "providers": [
                    {
                        "id": "chatgpt",
                        "display_name": "ChatGPT",
                        "enabled": false,
                        "new_chat_url": "https://chatgpt.com/",
                        "url_match_patterns": ["https://chatgpt.com/"],
                        "is_custom": false
                    },
                    {
                        "id": "example",
                        "display_name": "Example",
                        "enabled": true,
                        "new_chat_url": "https://example.com/chat",
                        "url_match_patterns": ["https://example.com/"],
                        "is_custom": true
                    }
                ],
                "general": { "auto_submit": true }
            }"#,
        )
        .expect("parse legacy config");

        assert!(config.migrate().expect("migrate legacy config"));
        assert!(!config.general.auto_submit);
        assert_eq!(config.provider_overrides.len(), 1);
        assert_eq!(config.custom_providers.len(), 1);
        assert_eq!(config.default_provider_id, "example");
        assert_eq!(config.merged_providers().expect("providers").len(), 5);
    }

    #[test]
    fn serialized_config_keeps_built_ins_out_of_user_configuration() {
        let value = serde_json::to_value(AppConfig::default()).expect("serialize config");

        assert!(value.get("providers").is_none());
        assert_eq!(value["provider_overrides"], serde_json::json!([]));
        assert_eq!(value["custom_providers"], serde_json::json!([]));
    }

    #[test]
    fn rejects_auto_submit_in_current_schema() {
        let mut config = AppConfig::default();
        config.general.auto_submit = true;

        assert!(config.validate().is_err());
        assert!(config.migrate().expect("normalize compatibility field"));
        assert!(!config.general.auto_submit);
    }

    #[test]
    fn rejects_empty_desktop_shortcut_mapping() {
        let mut config = AppConfig::default();
        config
            .browser
            .desktop_shortcuts
            .insert("chatgpt".to_owned(), " ".to_owned());

        assert!(matches!(
            config.validate(),
            Err(AppError::ConfigurationInvalid(_))
        ));
    }

    #[test]
    fn rejects_zero_or_unbounded_browser_timeouts() {
        for timeout in [0, MAX_BROWSER_TIMEOUT_MS + 1, u64::MAX] {
            let mut config = AppConfig::default();
            config.browser.page_timeout_ms = timeout;
            assert!(matches!(
                config.validate(),
                Err(AppError::ConfigurationInvalid(_))
            ));
        }
    }
}
