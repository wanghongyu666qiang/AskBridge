use serde::{Deserialize, Serialize};

use crate::{AppError, Result};

pub const BUILT_IN_ADAPTER_IDS: [&str; 4] = ["chatgpt", "gemini", "claude", "doubao"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    #[serde(alias = "new_chat_url")]
    pub start_url: String,
    #[serde(default, alias = "url_match_patterns")]
    pub url_patterns: Vec<String>,
    #[serde(default)]
    pub is_custom: bool,
    #[serde(default)]
    pub adapter_override: Option<String>,
}

impl ProviderConfig {
    pub fn validate(&self) -> Result<()> {
        if !is_safe_provider_id(&self.id) {
            return Err(AppError::InvalidProvider(
                "provider id must contain 1-64 lowercase ASCII letters, digits, or hyphens"
                    .to_owned(),
            ));
        }
        if self.display_name.trim().is_empty() {
            return Err(AppError::InvalidProvider(format!(
                "provider '{}' has an empty display name",
                self.id
            )));
        }
        validate_https_url(&self.start_url)?;
        if self.url_patterns.is_empty() {
            return Err(AppError::InvalidProvider(format!(
                "provider '{}' has no URL patterns",
                self.id
            )));
        }
        for pattern in &self.url_patterns {
            validate_https_url(pattern)?;
        }
        if self
            .adapter_override
            .as_deref()
            .is_some_and(|value| value.trim().is_empty() || !BUILT_IN_ADAPTER_IDS.contains(&value))
        {
            return Err(AppError::InvalidProvider(format!(
                "provider '{}' has an unknown adapter override",
                self.id,
            )));
        }
        Ok(())
    }

    pub fn apply_override(&mut self, provider_override: &ProviderOverride) {
        if let Some(display_name) = &provider_override.display_name {
            self.display_name.clone_from(display_name);
        }
        if let Some(enabled) = provider_override.enabled {
            self.enabled = enabled;
        }
        if let Some(start_url) = &provider_override.start_url {
            self.start_url.clone_from(start_url);
        }
        if let Some(url_patterns) = &provider_override.url_patterns {
            self.url_patterns.clone_from(url_patterns);
        }
        if let Some(adapter_override) = &provider_override.adapter_override {
            self.adapter_override.clone_from(adapter_override);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderOverride {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub start_url: Option<String>,
    #[serde(default)]
    pub url_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub adapter_override: Option<Option<String>>,
}

impl ProviderOverride {
    pub fn validate(&self) -> Result<()> {
        if !is_safe_provider_id(&self.id) {
            return Err(AppError::InvalidProvider(
                "provider override id must contain 1-64 lowercase ASCII letters, digits, or hyphens"
                    .to_owned(),
            ));
        }
        if self
            .display_name
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(AppError::InvalidProvider(format!(
                "provider override '{}' has an empty display name",
                self.id
            )));
        }
        if let Some(start_url) = &self.start_url {
            validate_https_url(start_url)?;
        }
        if let Some(url_patterns) = &self.url_patterns {
            if url_patterns.is_empty() {
                return Err(AppError::InvalidProvider(format!(
                    "provider override '{}' has no URL patterns",
                    self.id
                )));
            }
            for pattern in url_patterns {
                validate_https_url(pattern)?;
            }
        }
        if matches!(
            self.adapter_override.as_ref(),
            Some(Some(value))
                if value.trim().is_empty() || !BUILT_IN_ADAPTER_IDS.contains(&value.as_str())
        ) {
            return Err(AppError::InvalidProvider(format!(
                "provider override '{}' has an unknown adapter override",
                self.id
            )));
        }
        Ok(())
    }
}

pub fn built_in_providers() -> Vec<ProviderConfig> {
    vec![
        ProviderConfig {
            id: "chatgpt".to_owned(),
            display_name: "ChatGPT".to_owned(),
            enabled: true,
            start_url: "https://chatgpt.com/".to_owned(),
            url_patterns: vec!["https://chatgpt.com/".to_owned()],
            is_custom: false,
            adapter_override: Some("chatgpt".to_owned()),
        },
        ProviderConfig {
            id: "gemini".to_owned(),
            display_name: "Gemini".to_owned(),
            enabled: true,
            start_url: "https://gemini.google.com/app".to_owned(),
            url_patterns: vec!["https://gemini.google.com/".to_owned()],
            is_custom: false,
            adapter_override: Some("gemini".to_owned()),
        },
        ProviderConfig {
            id: "claude".to_owned(),
            display_name: "Claude".to_owned(),
            enabled: true,
            start_url: "https://claude.ai/new".to_owned(),
            url_patterns: vec!["https://claude.ai/".to_owned()],
            is_custom: false,
            adapter_override: Some("claude".to_owned()),
        },
        ProviderConfig {
            id: "doubao".to_owned(),
            display_name: "豆包".to_owned(),
            enabled: true,
            start_url: "https://www.doubao.com/chat/".to_owned(),
            url_patterns: vec!["https://www.doubao.com/".to_owned()],
            is_custom: false,
            adapter_override: Some("doubao".to_owned()),
        },
        ProviderConfig {
            id: "deepseek".to_owned(),
            display_name: "DeepSeek".to_owned(),
            enabled: true,
            start_url: "https://chat.deepseek.com/".to_owned(),
            url_patterns: vec!["https://chat.deepseek.com/".to_owned()],
            is_custom: false,
            adapter_override: None,
        },
        ProviderConfig {
            id: "glm".to_owned(),
            display_name: "智谱清言".to_owned(),
            enabled: true,
            start_url: "https://chatglm.cn/".to_owned(),
            url_patterns: vec!["https://chatglm.cn/".to_owned()],
            is_custom: false,
            adapter_override: None,
        },
        ProviderConfig {
            id: "kimi".to_owned(),
            display_name: "Kimi".to_owned(),
            enabled: true,
            start_url: "https://www.kimi.com/".to_owned(),
            url_patterns: vec!["https://www.kimi.com/".to_owned()],
            is_custom: false,
            adapter_override: None,
        },
    ]
}

pub(crate) fn validate_https_url(value: &str) -> Result<()> {
    let Some(authority_and_path) = value.strip_prefix("https://") else {
        return Err(AppError::InvalidProviderUrl(value.to_owned()));
    };
    let authority = authority_and_path
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    if authority.is_empty()
        || authority.starts_with('.')
        || authority.ends_with('.')
        || !authority.contains('.')
        || authority.contains('@')
        || value.contains('*')
        || value.contains('\\')
        || value.chars().any(char::is_control)
        || value.chars().any(char::is_whitespace)
    {
        return Err(AppError::InvalidProviderUrl(value.to_owned()));
    }
    Ok(())
}

fn is_safe_provider_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_provider_urls() {
        let mut provider = built_in_provider();
        for url in [
            "javascript:alert(1)",
            "file:///tmp/a",
            "data:text/plain,a",
            "http://localhost:3000",
            "https://user:password@example.com/chat",
            "https://*.example.com/",
            "https://example.com\\chat",
        ] {
            provider.start_url = url.to_owned();
            assert!(matches!(
                provider.validate(),
                Err(AppError::InvalidProviderUrl(_))
            ));
        }
    }

    #[test]
    fn provider_ids_are_safe_for_structured_logs_and_configuration_keys() {
        let mut provider = built_in_provider();
        for id in ["", "HasUpper", "contains space", "带中文", "../escape"] {
            provider.id = id.to_owned();
            assert!(provider.validate().is_err(), "unsafe id {id} was accepted");
        }
        provider.id = "custom-provider-1".to_owned();
        assert!(provider.validate().is_ok());
    }

    #[test]
    fn accepts_https_provider_urls() {
        assert!(built_in_provider().validate().is_ok());
    }

    #[test]
    fn built_in_provider_entry_urls_match_verified_web_apps() {
        let providers = built_in_providers();
        assert_eq!(
            providers
                .iter()
                .map(|provider| (provider.id.as_str(), provider.start_url.as_str()))
                .collect::<Vec<_>>(),
            [
                ("chatgpt", "https://chatgpt.com/"),
                ("gemini", "https://gemini.google.com/app"),
                ("claude", "https://claude.ai/new"),
                ("doubao", "https://www.doubao.com/chat/"),
                ("deepseek", "https://chat.deepseek.com/"),
                ("glm", "https://chatglm.cn/"),
                ("kimi", "https://www.kimi.com/"),
            ]
        );
        assert!(providers.iter().all(|provider| provider.validate().is_ok()));
    }

    #[test]
    fn rejects_unknown_adapter_overrides_before_browser_launch() {
        let mut provider = built_in_provider();
        provider.adapter_override = Some("downloaded-script".to_owned());
        assert!(matches!(
            provider.validate(),
            Err(AppError::InvalidProvider(_))
        ));
    }

    #[test]
    fn apply_override_updates_present_fields_and_clears_adapter_on_none() {
        // Outer `Some` with inner `None` is the explicit "clear the adapter"
        // signal; it gates which automation script may run.
        let mut provider = built_in_provider();
        provider.apply_override(&ProviderOverride {
            id: "example".to_owned(),
            display_name: None,
            enabled: Some(false),
            start_url: None,
            url_patterns: None,
            adapter_override: Some(Some("gemini".to_owned())),
        });
        assert!(!provider.enabled);
        assert_eq!(provider.adapter_override.as_deref(), Some("gemini"));

        provider.apply_override(&ProviderOverride {
            id: "example".to_owned(),
            display_name: Some("Renamed".to_owned()),
            enabled: None,
            start_url: Some("https://renamed.example.com/".to_owned()),
            url_patterns: Some(vec!["https://renamed.example.com/".to_owned()]),
            adapter_override: Some(None),
        });
        assert_eq!(provider.display_name, "Renamed");
        assert_eq!(provider.start_url, "https://renamed.example.com/");
        assert_eq!(provider.url_patterns, vec!["https://renamed.example.com/"]);
        assert_eq!(provider.adapter_override, None);
    }

    fn built_in_provider() -> ProviderConfig {
        ProviderConfig {
            id: "example".to_owned(),
            display_name: "Example".to_owned(),
            enabled: true,
            start_url: "https://example.com/chat".to_owned(),
            url_patterns: vec!["https://example.com/".to_owned()],
            is_custom: false,
            adapter_override: None,
        }
    }
}
