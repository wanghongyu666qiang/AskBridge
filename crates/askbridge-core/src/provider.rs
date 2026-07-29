use serde::{Deserialize, Serialize};

use crate::{AppError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub display_name: String,
    pub enabled: bool,
    pub new_chat_url: String,
    #[serde(default)]
    pub url_match_patterns: Vec<String>,
    #[serde(default)]
    pub browser_profile: Option<String>,
    #[serde(default)]
    pub is_custom: bool,
}

impl ProviderConfig {
    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(AppError::InvalidProvider("provider id is empty".to_owned()));
        }
        if self.display_name.trim().is_empty() {
            return Err(AppError::InvalidProvider(format!(
                "provider '{}' has an empty display name",
                self.id
            )));
        }
        validate_https_url(&self.new_chat_url)?;
        for pattern in &self.url_match_patterns {
            validate_https_url(pattern)?;
        }
        Ok(())
    }
}

fn validate_https_url(value: &str) -> Result<()> {
    let Some(authority_and_path) = value.strip_prefix("https://") else {
        return Err(AppError::InvalidProviderUrl(value.to_owned()));
    };
    let authority = authority_and_path.split('/').next().unwrap_or_default();
    if authority.is_empty()
        || authority.starts_with('.')
        || authority.ends_with('.')
        || !authority.contains('.')
        || value.chars().any(char::is_whitespace)
    {
        return Err(AppError::InvalidProviderUrl(value.to_owned()));
    }
    Ok(())
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
        ] {
            provider.new_chat_url = url.to_owned();
            assert!(matches!(
                provider.validate(),
                Err(AppError::InvalidProviderUrl(_))
            ));
        }
    }

    #[test]
    fn accepts_https_provider_urls() {
        assert!(built_in_provider().validate().is_ok());
    }

    fn built_in_provider() -> ProviderConfig {
        ProviderConfig {
            id: "example".to_owned(),
            display_name: "Example".to_owned(),
            enabled: true,
            new_chat_url: "https://example.com/chat".to_owned(),
            url_match_patterns: vec!["https://example.com/".to_owned()],
            browser_profile: None,
            is_custom: false,
        }
    }
}
