use std::collections::HashSet;

use askbridge_core::{AppError, BUILT_IN_ADAPTER_IDS, Result, matches_any_pattern};
use serde::Deserialize;

pub(super) const RULE_SCHEMA_VERSION: u32 = 2;
pub(super) const BUILTIN_RULES: &str = include_str!("builtin_rules.json");
pub(super) const MAX_RULE_SOURCE_BYTES: usize = 64 * 1024;
const MAX_PROVIDER_ID_BYTES: usize = 64;
const MAX_LOGIN_PATTERNS: usize = 8;
const MAX_SELECTORS_PER_FIELD: usize = 16;
const MAX_RULE_VALUE_BYTES: usize = 512;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ProviderRule {
    id: String,
    login_url_patterns: Vec<String>,
    #[serde(default)]
    login_selectors: Vec<String>,
    composer_selectors: Vec<String>,
    file_input_selectors: Vec<String>,
}

impl ProviderRule {
    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn matches_login_url(&self, url: &str) -> bool {
        matches_any_pattern(url, &self.login_url_patterns)
    }

    pub(super) fn composer_selectors(&self) -> &[String] {
        &self.composer_selectors
    }

    pub(super) fn login_selectors(&self) -> &[String] {
        &self.login_selectors
    }

    pub(super) fn file_input_selectors(&self) -> &[String] {
        &self.file_input_selectors
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ProviderRuleSet {
    pub(super) schema_version: u32,
    pub(super) providers: Vec<ProviderRule>,
}

pub(super) fn load_rule(adapter_override: Option<&str>) -> Result<Option<ProviderRule>> {
    let Some(adapter_id) = adapter_override else {
        return Ok(None);
    };
    if let Some(rule) = super::rules_update::active_rule(adapter_id)? {
        return Ok(Some(rule));
    }
    let rules = parse_and_validate(BUILTIN_RULES)?;
    rules
        .providers
        .into_iter()
        .find(|rule| rule.id == adapter_id)
        .map(Some)
        .ok_or_else(|| {
            AppError::InvalidProvider(format!(
                "adapter override '{adapter_id}' is not a built-in adapter"
            ))
        })
}

pub(super) fn validate_builtin_rules() -> Result<()> {
    parse_and_validate(BUILTIN_RULES).map(|_| ())
}

pub(super) fn parse_and_validate(source: &str) -> Result<ProviderRuleSet> {
    if source.len() > MAX_RULE_SOURCE_BYTES {
        return Err(AppError::InvalidPreparation(
            "built-in provider rules exceed the size limit".to_owned(),
        ));
    }
    let rules: ProviderRuleSet = serde_json::from_str(source).map_err(|error| {
        AppError::InvalidPreparation(format!("built-in provider rules are invalid: {error}"))
    })?;
    if rules.schema_version != RULE_SCHEMA_VERSION {
        return Err(AppError::InvalidPreparation(format!(
            "built-in provider rule schema {} is unsupported",
            rules.schema_version
        )));
    }
    let mut ids = HashSet::new();
    for rule in &rules.providers {
        if rule.id.trim().is_empty()
            || rule.id.len() > MAX_PROVIDER_ID_BYTES
            || !ids.insert(rule.id.as_str())
        {
            return Err(AppError::InvalidPreparation(
                "built-in provider rule ids must be non-empty and unique".to_owned(),
            ));
        }
        if rule.login_url_patterns.is_empty()
            || rule.composer_selectors.is_empty()
            || rule.file_input_selectors.is_empty()
        {
            return Err(AppError::InvalidPreparation(format!(
                "built-in provider rule '{}' is incomplete",
                rule.id
            )));
        }
        if rule.login_url_patterns.len() > MAX_LOGIN_PATTERNS
            || rule.login_selectors.len() > MAX_SELECTORS_PER_FIELD
            || rule.composer_selectors.len() > MAX_SELECTORS_PER_FIELD
            || rule.file_input_selectors.len() > MAX_SELECTORS_PER_FIELD
            || rule
                .login_url_patterns
                .iter()
                .chain(&rule.login_selectors)
                .chain(&rule.composer_selectors)
                .chain(&rule.file_input_selectors)
                .any(|value| value.len() > MAX_RULE_VALUE_BYTES)
        {
            return Err(AppError::InvalidPreparation(format!(
                "built-in provider rule '{}' exceeds a count or size limit",
                rule.id
            )));
        }
        if rule
            .login_url_patterns
            .iter()
            .any(|pattern| !pattern.starts_with("https://"))
            || rule
                .login_selectors
                .iter()
                .chain(&rule.composer_selectors)
                .chain(&rule.file_input_selectors)
                .any(|selector| {
                    selector.trim().is_empty()
                        || selector
                            .chars()
                            .any(|character| matches!(character, '{' | '}' | ';' | '\n' | '\r'))
                })
        {
            return Err(AppError::InvalidPreparation(format!(
                "built-in provider rule '{}' contains an unsafe value",
                rule.id
            )));
        }
    }
    if ids.len() != BUILT_IN_ADAPTER_IDS.len()
        || BUILT_IN_ADAPTER_IDS.iter().any(|id| !ids.contains(id))
    {
        return Err(AppError::InvalidPreparation(
            "built-in provider rules do not cover the required providers".to_owned(),
        ));
    }
    Ok(rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn provider_rule(id: &str) -> Value {
        json!({
            "id": id,
            "login_url_patterns": ["https://example.test/login"],
            "login_selectors": [],
            "composer_selectors": ["textarea"],
            "file_input_selectors": ["input[type=\"file\"]"]
        })
    }

    fn rule_document(schema_version: u32, providers: Vec<Value>) -> String {
        json!({
            "schema_version": schema_version,
            "providers": providers
        })
        .to_string()
    }

    fn assert_invalid_preparation(result: Result<ProviderRuleSet>, expected: &str) {
        match result {
            Err(AppError::InvalidPreparation(message)) => {
                assert!(
                    message.contains(expected),
                    "expected error containing {expected:?}, got {message:?}"
                );
            }
            other => panic!("expected InvalidPreparation containing {expected:?}, got {other:?}"),
        }
    }

    #[test]
    fn embedded_rules_are_versioned_complete_and_unique() {
        let rules = parse_and_validate(BUILTIN_RULES).expect("rules");
        assert_eq!(rules.providers.len(), 4);
        assert_eq!(
            rules
                .providers
                .iter()
                .map(|rule| rule.id.as_str())
                .collect::<Vec<_>>(),
            ["chatgpt", "gemini", "claude", "doubao"]
        );
    }

    #[test]
    fn missing_override_selects_generic_and_unknown_override_is_rejected() {
        assert!(load_rule(None).expect("generic").is_none());
        assert!(load_rule(Some("chatgpt")).expect("chatgpt").is_some());
        assert!(matches!(
            load_rule(Some("unknown")),
            Err(AppError::InvalidProvider(_))
        ));
    }

    #[test]
    fn login_patterns_are_provider_specific() {
        let cases = [
            ("chatgpt", "https://chatgpt.com/auth/login"),
            (
                "gemini",
                "https://accounts.google.com/v3/signin/identifier?continue=https://gemini.google.com/app",
            ),
            ("claude", "https://claude.ai/login"),
            ("doubao", "https://www.doubao.com/login"),
        ];

        for (adapter_id, login_url) in cases {
            let rule = load_rule(Some(adapter_id))
                .expect("rule")
                .unwrap_or_else(|| panic!("missing rule for {adapter_id}"));
            assert!(
                rule.matches_login_url(login_url),
                "{adapter_id} did not recognize its login URL"
            );
            for (other_adapter_id, other_login_url) in cases {
                if adapter_id != other_adapter_id {
                    assert!(
                        !rule.matches_login_url(other_login_url),
                        "{adapter_id} matched {other_adapter_id}'s login URL"
                    );
                }
            }
        }

        let chatgpt = load_rule(Some("chatgpt")).expect("rule").expect("chatgpt");
        assert!(chatgpt.matches_login_url("https://auth.openai.com/authorize"));
        let claude = load_rule(Some("claude")).expect("rule").expect("claude");
        assert!(claude.matches_login_url("https://claude.ai/logout"));
    }

    #[test]
    fn providers_with_observed_unique_file_inputs_use_exact_selectors() {
        let chatgpt = load_rule(Some("chatgpt")).expect("rule").expect("chatgpt");
        let claude = load_rule(Some("claude")).expect("rule").expect("claude");

        assert_eq!(chatgpt.file_input_selectors(), ["#upload-files"]);
        assert_eq!(
            claude.file_input_selectors(),
            ["#chat-input-file-upload-onpage"]
        );
    }

    #[test]
    fn rule_document_cannot_embed_executable_programs() {
        let rules = parse_and_validate(BUILTIN_RULES).expect("rules");
        for selector in rules.providers.iter().flat_map(|rule| {
            rule.composer_selectors
                .iter()
                .chain(&rule.file_input_selectors)
        }) {
            assert!(
                !selector
                    .chars()
                    .any(|character| matches!(character, '{' | '}' | ';' | '\n' | '\r'))
            );
        }
    }

    #[test]
    fn malformed_json_is_rejected_before_rule_use() {
        assert_invalid_preparation(
            parse_and_validate(r#"{"schema_version":1,"providers":["#),
            "built-in provider rules are invalid",
        );
    }

    #[test]
    fn unsupported_schema_version_is_rejected() {
        assert_invalid_preparation(
            parse_and_validate(&rule_document(99, Vec::new())),
            "schema 99 is unsupported",
        );
    }

    #[test]
    fn duplicate_provider_ids_are_rejected() {
        assert_invalid_preparation(
            parse_and_validate(&rule_document(
                RULE_SCHEMA_VERSION,
                vec![provider_rule("chatgpt"), provider_rule("chatgpt")],
            )),
            "ids must be non-empty and unique",
        );
    }

    #[test]
    fn incomplete_provider_rules_are_rejected() {
        let mut incomplete = provider_rule("chatgpt");
        incomplete["file_input_selectors"] = json!([]);
        assert_invalid_preparation(
            parse_and_validate(&rule_document(RULE_SCHEMA_VERSION, vec![incomplete])),
            "rule 'chatgpt' is incomplete",
        );
    }

    #[test]
    fn excessive_selector_counts_and_lengths_are_rejected() {
        let mut excessive_count = provider_rule("chatgpt");
        excessive_count["composer_selectors"] = json!(
            (0..=MAX_SELECTORS_PER_FIELD)
                .map(|index| format!("textarea[data-index='{index}']"))
                .collect::<Vec<_>>()
        );
        assert_invalid_preparation(
            parse_and_validate(&rule_document(RULE_SCHEMA_VERSION, vec![excessive_count])),
            "exceeds a count or size limit",
        );

        let mut excessive_length = provider_rule("chatgpt");
        excessive_length["composer_selectors"] = json!(["x".repeat(MAX_RULE_VALUE_BYTES + 1)]);
        assert_invalid_preparation(
            parse_and_validate(&rule_document(RULE_SCHEMA_VERSION, vec![excessive_length])),
            "exceeds a count or size limit",
        );
    }

    #[test]
    fn required_provider_coverage_is_enforced() {
        let providers = ["chatgpt", "gemini", "claude"]
            .into_iter()
            .map(provider_rule)
            .collect();
        assert_invalid_preparation(
            parse_and_validate(&rule_document(RULE_SCHEMA_VERSION, providers)),
            "do not cover the required providers",
        );
    }

    #[test]
    fn non_https_login_patterns_are_rejected() {
        for pattern in [
            "http://example.test/login",
            "javascript:alert(1)",
            "data:text/plain,login",
        ] {
            let mut rule = provider_rule("chatgpt");
            rule["login_url_patterns"] = json!([pattern]);
            assert_invalid_preparation(
                parse_and_validate(&rule_document(RULE_SCHEMA_VERSION, vec![rule])),
                "contains an unsafe value",
            );
        }
    }

    #[test]
    fn empty_or_executable_selector_values_are_rejected() {
        for selector in [
            "   ",
            "textarea;document.body.innerHTML='x'",
            "textarea{color:red}",
            "textarea\ninput",
        ] {
            let mut rule = provider_rule("chatgpt");
            rule["composer_selectors"] = json!([selector]);
            assert_invalid_preparation(
                parse_and_validate(&rule_document(RULE_SCHEMA_VERSION, vec![rule])),
                "contains an unsafe value",
            );
        }
    }

    #[test]
    fn executable_login_selector_values_are_rejected() {
        let mut rule = provider_rule("chatgpt");
        rule["login_selectors"] = json!(["a[href*=login];document.body.remove()"]);
        assert_invalid_preparation(
            parse_and_validate(&rule_document(RULE_SCHEMA_VERSION, vec![rule])),
            "contains an unsafe value",
        );
    }

    #[test]
    fn every_builtin_provider_has_complete_safe_rule_fields() {
        let rules = parse_and_validate(BUILTIN_RULES).expect("rules");
        assert_eq!(rules.providers.len(), BUILT_IN_ADAPTER_IDS.len());
        for adapter_id in BUILT_IN_ADAPTER_IDS {
            let rule = rules
                .providers
                .iter()
                .find(|rule| rule.id == adapter_id)
                .unwrap_or_else(|| panic!("missing rule for {adapter_id}"));
            assert!(!rule.login_url_patterns.is_empty());
            assert!(
                rule.login_url_patterns
                    .iter()
                    .all(|pattern| pattern.starts_with("https://"))
            );
            assert!(!rule.composer_selectors.is_empty());
            assert!(!rule.file_input_selectors.is_empty());
            assert!(
                rule.login_selectors
                    .iter()
                    .chain(&rule.composer_selectors)
                    .chain(&rule.file_input_selectors)
                    .all(|selector| {
                        !selector.trim().is_empty()
                            && !selector
                                .chars()
                                .any(|character| matches!(character, '{' | '}' | ';' | '\n' | '\r'))
                    })
            );
        }
    }
}
