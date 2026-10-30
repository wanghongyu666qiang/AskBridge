//! The editable settings draft exchanged with the settings page.
//!
//! The page owns no persistence logic: it posts a draft, and [`SettingsDraft`]
//! turns that draft into a validated [`AppConfig`] on top of the config that is
//! currently in effect. Everything here is pure data, so the conversion rules
//! are unit-tested without a window.

use std::collections::HashSet;

use askbridge_core::{
    AppCommand, AppConfig, AppError, BrowserLifecycle, BrowserTargetPreference, HotkeyBinding,
    HotkeyConfig, ProviderConfig, ProviderOverride, Result, provider::built_in_providers,
};
use serde::Deserialize;

/// One hotkey row: the binding text plus its enabled flag.
#[derive(Debug, Clone, Deserialize)]
pub struct HotkeyDraft {
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    pub text: String,
}

const fn enabled_by_default() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct HotkeyDrafts {
    pub capture_with_prompt: HotkeyDraft,
    pub capture_quick_dispatch: HotkeyDraft,
    pub text_only_prompt: HotkeyDraft,
}

impl HotkeyDrafts {
    fn binding(&self, command: AppCommand) -> &HotkeyDraft {
        match command {
            AppCommand::CaptureWithPrompt => &self.capture_with_prompt,
            AppCommand::CaptureQuickDispatch => &self.capture_quick_dispatch,
            AppCommand::TextOnlyPrompt => &self.text_only_prompt,
        }
    }
}

/// A built-in provider row. The page always sends every built-in, so a missing
/// row is rejected instead of silently keeping a stale override.
#[derive(Debug, Clone, Deserialize)]
pub struct ProviderDraft {
    pub id: String,
    pub enabled: bool,
    pub start_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomProviderDraft {
    pub id: String,
    pub display_name: String,
    pub start_url: String,
    /// Comma-separated match prefixes, as typed by the user.
    #[serde(default)]
    pub url_patterns: String,
}

/// Everything the settings page can change.
#[derive(Debug, Clone, Deserialize)]
pub struct SettingsDraft {
    pub hotkeys: HotkeyDrafts,
    pub default_provider_id: String,
    #[serde(default)]
    pub quick_prompt: String,
    #[serde(default)]
    pub start_on_login: bool,
    #[serde(default)]
    pub debug_logging: bool,
    #[serde(default)]
    pub chrome_path: String,
    pub lifecycle: BrowserLifecycle,
    pub chatgpt_target: BrowserTargetPreference,
    pub providers: Vec<ProviderDraft>,
    #[serde(default)]
    pub custom_providers: Vec<CustomProviderDraft>,
}

impl SettingsDraft {
    /// Applies the draft on top of `base` and validates the result.
    ///
    /// `base` is the configuration currently in effect, which supplies every
    /// field the settings page does not expose (profile directory, timeouts,
    /// desktop shortcuts, other providers' target preferences).
    pub fn to_config(&self, base: &AppConfig) -> Result<AppConfig> {
        let mut candidate = base.clone();
        candidate.hotkeys = self.hotkeys()?;
        candidate.default_provider_id = self.default_provider_id.clone();
        candidate.quick_prompt = self.quick_prompt.trim().to_owned();
        candidate.general.start_on_login = self.start_on_login;
        candidate.general.debug_logging = self.debug_logging;
        // Permanent product boundary: AskBridge never submits for the user.
        candidate.general.auto_submit = false;

        let chrome_path = self.chrome_path.trim();
        candidate.browser.chrome_path = (!chrome_path.is_empty()).then(|| chrome_path.to_owned());
        candidate.browser.lifecycle = self.lifecycle;
        candidate
            .browser
            .target_preferences
            .insert("chatgpt".to_owned(), self.chatgpt_target);
        candidate.provider_overrides = self.provider_overrides(base)?;
        candidate.custom_providers = self.custom_providers()?;
        candidate.validate()?;
        Ok(candidate)
    }

    fn hotkeys(&self) -> Result<HotkeyConfig> {
        let mut config = HotkeyConfig::default();
        for command in AppCommand::ALL {
            let row = self.hotkeys.binding(command);
            let text = row.text.trim();
            let mut binding = text
                .parse::<HotkeyBinding>()
                .map_err(|error| AppError::InvalidHotkey(format!("{text}: {error}")))?;
            binding.enabled = row.enabled;
            *config.binding_mut(command) = binding;
        }
        config.validate()?;
        Ok(config)
    }

    fn custom_providers(&self) -> Result<Vec<ProviderConfig>> {
        let mut seen = HashSet::new();
        let mut providers = Vec::with_capacity(self.custom_providers.len());
        for row in &self.custom_providers {
            let id = row.id.trim().to_owned();
            // An added-but-unfilled row is an empty template, not a provider.
            // Dropping it here means "add a row, then change your mind" needs
            // no delete click.
            if id.is_empty()
                && row.display_name.trim().is_empty()
                && row.start_url.trim().is_empty()
                && row.url_patterns.trim().is_empty()
            {
                continue;
            }
            if !seen.insert(id.clone()) {
                return Err(AppError::ConfigurationInvalid(format!(
                    "custom provider '{id}' is duplicated"
                )));
            }
            let provider = ProviderConfig {
                id,
                display_name: row.display_name.trim().to_owned(),
                enabled: true,
                start_url: row.start_url.trim().to_owned(),
                url_patterns: row
                    .url_patterns
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect(),
                is_custom: true,
                adapter_override: None,
            };
            provider.validate()?;
            providers.push(provider);
        }
        Ok(providers)
    }

    /// Rebuilds the override list so it only carries the fields that actually
    /// differ from the built-in defaults. A provider whose start URL changed
    /// gets its match prefixes re-derived from the new origin, which keeps a
    /// stale pattern from widening where AskBridge may act.
    fn provider_overrides(&self, base: &AppConfig) -> Result<Vec<ProviderOverride>> {
        let defaults = built_in_providers();
        let current = base.merged_providers()?;
        let mut overrides = Vec::new();
        for original in &defaults {
            let row = self
                .providers
                .iter()
                .find(|row| row.id == original.id)
                .ok_or_else(|| AppError::InvalidProvider(original.id.clone()))?;
            let previous = current
                .iter()
                .find(|provider| provider.id == original.id)
                .ok_or_else(|| AppError::InvalidProvider(original.id.clone()))?;
            let existing = base
                .provider_overrides
                .iter()
                .find(|provider| provider.id == original.id);
            let enabled = row.enabled;
            let start_url = row.start_url.trim().to_owned();
            let url_patterns = if start_url == previous.start_url {
                previous.url_patterns.clone()
            } else {
                vec![origin_pattern(&start_url)?]
            };
            ProviderConfig {
                id: original.id.clone(),
                display_name: original.display_name.clone(),
                enabled,
                start_url: start_url.clone(),
                url_patterns: url_patterns.clone(),
                is_custom: false,
                adapter_override: original.adapter_override.clone(),
            }
            .validate()?;
            let provider_override = ProviderOverride {
                id: original.id.clone(),
                display_name: existing.and_then(|value| value.display_name.clone()),
                enabled: (enabled != original.enabled).then_some(enabled),
                start_url: (start_url != original.start_url).then_some(start_url),
                url_patterns: (url_patterns != original.url_patterns).then_some(url_patterns),
                adapter_override: existing.and_then(|value| value.adapter_override.clone()),
            };
            if provider_override.display_name.is_some()
                || provider_override.enabled.is_some()
                || provider_override.start_url.is_some()
                || provider_override.url_patterns.is_some()
                || provider_override.adapter_override.is_some()
            {
                overrides.push(provider_override);
            }
        }
        Ok(overrides)
    }
}

/// Collapses a provider start URL to its HTTPS origin, used as the single
/// match prefix for a provider whose URL the user just edited.
pub(super) fn origin_pattern(url: &str) -> Result<String> {
    let Some(remainder) = url.strip_prefix("https://") else {
        return Err(AppError::InvalidProviderUrl(url.to_owned()));
    };
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return Err(AppError::InvalidProviderUrl(url.to_owned()));
    }
    Ok(format!("https://{authority}/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use askbridge_core::ModifierKey;

    fn hotkey(text: &str) -> HotkeyDraft {
        HotkeyDraft {
            enabled: true,
            text: text.to_owned(),
        }
    }

    fn draft() -> SettingsDraft {
        SettingsDraft {
            hotkeys: HotkeyDrafts {
                capture_with_prompt: hotkey("Alt+Q"),
                capture_quick_dispatch: hotkey("Alt+Shift+Q"),
                text_only_prompt: hotkey("Alt+W"),
            },
            default_provider_id: "chatgpt".to_owned(),
            quick_prompt: "  summarize  ".to_owned(),
            start_on_login: false,
            debug_logging: false,
            chrome_path: "   ".to_owned(),
            lifecycle: BrowserLifecycle::OnDemandKeepRunning,
            chatgpt_target: BrowserTargetPreference::DedicatedChrome,
            providers: built_in_providers()
                .into_iter()
                .map(|provider| ProviderDraft {
                    id: provider.id,
                    enabled: true,
                    start_url: provider.start_url,
                })
                .collect(),
            custom_providers: Vec::new(),
        }
    }

    #[test]
    fn draft_round_trips_unchanged_defaults_without_overrides() {
        let base = AppConfig::default();
        let config = draft().to_config(&base).expect("draft applies");

        assert!(config.provider_overrides.is_empty());
        assert_eq!(config.quick_prompt, "summarize");
        assert_eq!(config.browser.chrome_path, None);
        assert!(!config.general.auto_submit);
    }

    #[test]
    fn disabled_built_in_provider_becomes_an_override() {
        let base = AppConfig::default();
        let mut draft = draft();
        draft
            .providers
            .iter_mut()
            .find(|row| row.id == "doubao")
            .expect("doubao row")
            .enabled = false;

        let config = draft.to_config(&base).expect("draft applies");
        let doubao = config
            .provider_overrides
            .iter()
            .find(|provider| provider.id == "doubao")
            .expect("doubao override");
        assert_eq!(doubao.enabled, Some(false));
    }

    #[test]
    fn edited_start_url_rederives_its_match_prefix() {
        let base = AppConfig::default();
        let mut draft = draft();
        draft
            .providers
            .iter_mut()
            .find(|row| row.id == "chatgpt")
            .expect("chatgpt row")
            .start_url = "https://chat.example.com/app?x=1".to_owned();

        let config = draft.to_config(&base).expect("draft applies");
        let chatgpt = config
            .provider_overrides
            .iter()
            .find(|provider| provider.id == "chatgpt")
            .expect("chatgpt override");
        assert_eq!(
            chatgpt.url_patterns.as_deref(),
            Some(&["https://chat.example.com/".to_owned()][..])
        );
        assert_eq!(
            chatgpt.start_url.as_deref(),
            Some("https://chat.example.com/app?x=1")
        );
    }

    #[test]
    fn editing_only_the_path_keeps_the_existing_match_prefix() {
        let base = AppConfig::default();
        let mut draft = draft();
        draft
            .providers
            .iter_mut()
            .find(|row| row.id == "chatgpt")
            .expect("chatgpt row")
            .start_url = "https://chatgpt.com/app/new?x=1".to_owned();

        let config = draft.to_config(&base).expect("draft applies");
        let chatgpt = config
            .provider_overrides
            .iter()
            .find(|provider| provider.id == "chatgpt")
            .expect("chatgpt override");
        // Same origin, so the match prefix is unchanged and is not re-emitted.
        assert_eq!(chatgpt.url_patterns, None);
        assert_eq!(
            chatgpt.start_url.as_deref(),
            Some("https://chatgpt.com/app/new?x=1")
        );
    }

    #[test]
    fn missing_built_in_row_is_rejected() {
        let base = AppConfig::default();
        let mut draft = draft();
        draft.providers.retain(|row| row.id != "kimi");

        assert!(matches!(
            draft.to_config(&base),
            Err(AppError::InvalidProvider(_))
        ));
    }

    #[test]
    fn custom_providers_require_safe_complete_rows() {
        let base = AppConfig::default();
        let mut draft = draft();
        draft.custom_providers = vec![CustomProviderDraft {
            id: "example".to_owned(),
            display_name: "Example".to_owned(),
            start_url: "https://example.com/chat".to_owned(),
            url_patterns: "https://example.com/, https://api.example.com/".to_owned(),
        }];
        let config = draft.to_config(&base).expect("custom provider applies");
        assert_eq!(config.custom_providers.len(), 1);
        assert_eq!(config.custom_providers[0].url_patterns.len(), 2);
        assert!(config.custom_providers[0].is_custom);

        draft.custom_providers[0].start_url = "javascript:alert(1)".to_owned();
        assert!(matches!(
            draft.to_config(&base),
            Err(AppError::InvalidProviderUrl(_))
        ));
    }

    #[test]
    fn empty_custom_rows_are_dropped_instead_of_rejected() {
        let base = AppConfig::default();
        let mut draft = draft();
        draft.custom_providers = vec![CustomProviderDraft {
            id: String::new(),
            display_name: String::new(),
            start_url: String::new(),
            url_patterns: String::new(),
        }];

        let config = draft.to_config(&base).expect("empty row is ignored");
        assert!(config.custom_providers.is_empty());
    }

    #[test]
    fn a_partially_filled_custom_row_is_still_rejected() {
        let base = AppConfig::default();
        let mut draft = draft();
        draft.custom_providers = vec![CustomProviderDraft {
            id: "example".to_owned(),
            display_name: String::new(),
            start_url: String::new(),
            url_patterns: String::new(),
        }];

        // Half a row is a mistake the user needs to see, not something to
        // silently discard.
        assert!(draft.to_config(&base).is_err());
    }

    #[test]
    fn duplicate_custom_provider_ids_are_rejected() {
        let base = AppConfig::default();
        let mut draft = draft();
        let row = CustomProviderDraft {
            id: "example".to_owned(),
            display_name: "Example".to_owned(),
            start_url: "https://example.com/chat".to_owned(),
            url_patterns: "https://example.com/".to_owned(),
        };
        draft.custom_providers = vec![row.clone(), row];

        assert!(matches!(
            draft.to_config(&base),
            Err(AppError::ConfigurationInvalid(_))
        ));
    }

    #[test]
    fn conflicting_hotkeys_are_rejected_before_persisting() {
        let base = AppConfig::default();
        let mut draft = draft();
        draft.hotkeys.capture_quick_dispatch = hotkey("Alt+Q");

        assert!(matches!(
            draft.to_config(&base),
            Err(AppError::HotkeyConflict(_))
        ));
    }

    #[test]
    fn hotkey_without_modifier_is_rejected() {
        let base = AppConfig::default();
        let mut draft = draft();
        draft.hotkeys.text_only_prompt = hotkey("W");

        assert!(matches!(
            draft.to_config(&base),
            Err(AppError::InvalidHotkey(_))
        ));
    }

    #[test]
    fn disabling_a_hotkey_keeps_its_parsed_binding() {
        let base = AppConfig::default();
        let mut draft = draft();
        draft.hotkeys.capture_with_prompt.enabled = false;

        let config = draft.to_config(&base).expect("draft applies");
        let binding = &config.hotkeys.capture_with_prompt;
        assert!(!binding.enabled);
        assert_eq!(binding.to_string(), "Alt+Q");
        assert!(binding.has_modifier(ModifierKey::Alt));
    }

    #[test]
    fn origin_pattern_requires_https_and_a_host() {
        assert_eq!(
            origin_pattern("https://example.com/app/new?x=1").expect("origin"),
            "https://example.com/"
        );
        assert!(origin_pattern("http://example.com/").is_err());
        assert!(origin_pattern("https:///path").is_err());
    }
}
