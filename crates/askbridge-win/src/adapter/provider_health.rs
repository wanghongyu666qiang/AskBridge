use std::{sync::atomic::AtomicBool, time::Duration};

use askbridge_core::{AppError, Result, matches_any_pattern};
use serde_json::Value;

use crate::browser::{CdpClient, CdpTarget};

use super::rules::{ProviderRule, load_rule};

/// Overall result of a provider capability self-test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderHealth {
    /// The page is reachable, logged in, focusable, and exposes PNG attachment input.
    Healthy,
    /// The provider page requires the user to sign in.
    LoginRequired,
    /// No unique focusable composer could be found.
    ComposerMissing,
    /// Text composition works, but no PNG-capable file input is available.
    AttachmentUnsupported,
    /// The page or managed browser could not be reached.
    NetworkError,
}

/// Declarative inputs required to test one provider in managed Chrome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHealthCheck {
    /// Stable provider identifier used for rules and UI status updates.
    pub provider_id: String,
    /// Provider page to open when no reusable target exists.
    pub start_url: String,
    /// Allowed page URL patterns for this provider.
    pub url_patterns: Vec<String>,
    /// Optional built-in provider rule override.
    pub adapter_override: Option<String>,
}

/// Observable capabilities returned by a no-send provider self-test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHealthReport {
    /// Stable provider identifier associated with this result.
    pub provider_id: String,
    /// Overall provider health classification.
    pub health: ProviderHealth,
    /// Whether the provider page was reachable and inspectable.
    pub page_accessible: bool,
    /// Whether the inspected page appeared to be authenticated.
    pub logged_in: bool,
    /// Whether a unique composer was found.
    pub composer_found: bool,
    /// Whether a PNG-capable file input was found.
    pub attachment_supported: bool,
    /// Whether the discovered composer accepted focus.
    pub focus_supported: bool,
}

impl ProviderHealthReport {
    /// Creates a report for a page that could not be opened or inspected.
    pub fn network_error(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            health: ProviderHealth::NetworkError,
            page_accessible: false,
            logged_in: false,
            composer_found: false,
            attachment_supported: false,
            focus_supported: false,
        }
    }
}

/// Detects provider capabilities without inserting text, selecting a file, or submitting.
pub fn check_provider_health(
    client: &CdpClient,
    target: &CdpTarget,
    check: &ProviderHealthCheck,
    cancelled: &AtomicBool,
    timeout: Duration,
) -> Result<ProviderHealthReport> {
    let rule = load_rule(check.adapter_override.as_deref())?;
    if rule
        .as_ref()
        .is_some_and(|rule| rule.matches_login_url(&target.url))
    {
        return Ok(login_report(&check.provider_id));
    }
    if !matches_any_pattern(&target.url, &check.url_patterns) {
        return Ok(ProviderHealthReport::network_error(&check.provider_id));
    }
    let expression = health_expression(rule.as_ref(), &target.url)?;
    let result = client.evaluate_in_target(target, &expression, cancelled, timeout)?;
    report_from_evaluation(
        &check.provider_id,
        &target.url,
        &check.url_patterns,
        &result,
    )
}

fn health_expression(rule: Option<&ProviderRule>, expected_url: &str) -> Result<String> {
    let composer_selectors = rule.map_or(&[][..], ProviderRule::composer_selectors);
    let file_selectors = rule.map_or(&[][..], ProviderRule::file_input_selectors);
    let login_selectors = rule.map_or(&[][..], ProviderRule::login_selectors);
    let composer_selectors = serde_json::to_string(composer_selectors).map_err(|_| {
        AppError::InvalidPreparation("health composer selectors could not be encoded".to_owned())
    })?;
    let file_selectors = serde_json::to_string(file_selectors).map_err(|_| {
        AppError::InvalidPreparation("health file selectors could not be encoded".to_owned())
    })?;
    let login_selectors = serde_json::to_string(login_selectors).map_err(|_| {
        AppError::InvalidPreparation("health login selectors could not be encoded".to_owned())
    })?;
    let expected_url = serde_json::to_string(expected_url).map_err(|_| {
        AppError::InvalidPreparation("health expected URL could not be encoded".to_owned())
    })?;
    Ok(format!(
        r#"(() => {{
  const expectedUrl = {expected_url};
  const preferredComposer = {composer_selectors};
  const preferredFiles = {file_selectors};
  const loginSelectors = {login_selectors};
  if (location.href !== expectedUrl) return {{ status: 'navigation_changed', url: location.href }};
  const visible = (el) => {{
    const rect = el.getBoundingClientRect();
    const style = getComputedStyle(el);
    return rect.width > 0 && rect.height > 0 && style.display !== 'none' &&
      style.visibility !== 'hidden' && Number(style.opacity || '1') > 0;
  }};
  const editable = (el) => !el.disabled && !el.readOnly &&
    (el.matches('textarea,input') || el.isContentEditable || el.getAttribute('role') === 'textbox');
  const login = [...new Set(loginSelectors.flatMap(
    (selector) => [...document.querySelectorAll(selector)]
  ))].some(visible);
  if (login) return {{ status: 'login_required', url: location.href }};
  const preferred = [...new Set(preferredComposer.flatMap(
    (selector) => [...document.querySelectorAll(selector)]
  ))].filter((el) => visible(el) && editable(el));
  const source = preferred.length ? preferred : [...new Set(
    document.querySelectorAll('textarea,[contenteditable="true"],[role="textbox"]')
  )].filter((el) => visible(el) && editable(el));
  const scored = source.map((el) => {{
    const rect = el.getBoundingClientRect();
    const label = [el.getAttribute('aria-label'), el.getAttribute('placeholder'),
      el.getAttribute('name'), el.id, el.className].filter(Boolean).join(' ').toLowerCase();
    let score = el.matches('textarea') ? 45 : (el.isContentEditable ? 40 : 35);
    if (preferred.includes(el)) score += 100;
    if (/message|prompt|ask|chat|send|提问|消息|输入/.test(label)) score += 30;
    if (/search|feedback|account|login|搜索|反馈|账号|登录/.test(label)) score -= 80;
    if (rect.top > innerHeight * 0.45) score += 20;
    if (rect.width > Math.min(360, innerWidth * 0.35)) score += 15;
    return {{ el, score }};
  }}).sort((a, b) => b.score - a.score);
  const unique = scored.length > 0 && scored[0].score >= 60 &&
    !(scored.length > 1 && scored[1].score >= scored[0].score - 10);
  let focused = false;
  if (unique) {{
    scored[0].el.focus();
    focused = document.activeElement === scored[0].el;
  }}
  const fileCandidates = [...new Set((preferredFiles.length ? preferredFiles : ['input[type=file]']).flatMap(
    (selector) => [...document.querySelectorAll(selector)]
  ))].filter((el) => {{
    if (el.disabled || !el.matches('input[type=file]')) return false;
    const accept = String(el.getAttribute('accept') || '').trim().toLowerCase();
    return !accept || accept.split(',').some((item) =>
      ['image/*', 'image/png', '.png'].includes(item.trim()));
  }});
  return {{ status: 'checked', url: location.href, composerFound: unique,
    focusSupported: focused, attachmentSupported: fileCandidates.length === 1 }};
}})()"#
    ))
}

fn report_from_evaluation(
    provider_id: &str,
    expected_url: &str,
    url_patterns: &[String],
    result: &Value,
) -> Result<ProviderHealthReport> {
    let value = result.pointer("/result/value").ok_or_else(|| {
        AppError::BrowserProtocol("provider health evaluation returned no value".to_owned())
    })?;
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or(expected_url);
    if !matches_any_pattern(url, url_patterns) {
        return Ok(ProviderHealthReport::network_error(provider_id));
    }
    match value.get("status").and_then(Value::as_str) {
        Some("login_required") => Ok(login_report(provider_id)),
        Some("navigation_changed") => Ok(ProviderHealthReport::network_error(provider_id)),
        Some("checked") => {
            let composer_found = value
                .get("composerFound")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let focus_supported = value
                .get("focusSupported")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let attachment_supported = value
                .get("attachmentSupported")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let health = if !composer_found || !focus_supported {
                ProviderHealth::ComposerMissing
            } else if !attachment_supported {
                ProviderHealth::AttachmentUnsupported
            } else {
                ProviderHealth::Healthy
            };
            Ok(ProviderHealthReport {
                provider_id: provider_id.to_owned(),
                health,
                page_accessible: true,
                logged_in: true,
                composer_found,
                attachment_supported,
                focus_supported,
            })
        }
        _ => Err(AppError::BrowserProtocol(
            "provider health evaluation returned an invalid status".to_owned(),
        )),
    }
}

fn login_report(provider_id: &str) -> ProviderHealthReport {
    ProviderHealthReport {
        provider_id: provider_id.to_owned(),
        health: ProviderHealth::LoginRequired,
        page_accessible: true,
        logged_in: false,
        composer_found: false,
        attachment_supported: false,
        focus_supported: false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn capability_probe_has_no_text_file_or_submit_mutation() {
        let expression = health_expression(None, "https://example.test/chat").expect("expression");
        for forbidden in [
            "DOM.setFileInputFiles",
            "execCommand('insertText'",
            ".click(",
            "requestSubmit",
            "InputEvent",
        ] {
            assert!(!expression.contains(forbidden), "found {forbidden}");
        }
        assert!(expression.contains(".focus()"));
    }

    #[test]
    fn missing_page_is_reported_without_claiming_any_capability() {
        let report = ProviderHealthReport::network_error("example");

        assert_eq!(report.health, ProviderHealth::NetworkError);
        assert!(!report.page_accessible);
        assert!(!report.logged_in);
        assert!(!report.composer_found);
        assert!(!report.attachment_supported);
        assert!(!report.focus_supported);
    }

    #[test]
    fn reports_composer_and_attachment_failures_separately() {
        let patterns = vec!["https://example.test/".to_owned()];
        let missing = report_from_evaluation(
            "example",
            "https://example.test/",
            &patterns,
            &json!({"result":{"value":{"status":"checked","url":"https://example.test/","composerFound":false,"focusSupported":false,"attachmentSupported":true}}}),
        )
        .expect("report");
        assert_eq!(missing.health, ProviderHealth::ComposerMissing);

        let attachment = report_from_evaluation(
            "example",
            "https://example.test/",
            &patterns,
            &json!({"result":{"value":{"status":"checked","url":"https://example.test/","composerFound":true,"focusSupported":true,"attachmentSupported":false}}}),
        )
        .expect("report");
        assert_eq!(attachment.health, ProviderHealth::AttachmentUnsupported);
    }

    #[test]
    fn login_navigation_and_healthy_pages_are_classified_without_sending() {
        let patterns = vec!["https://example.test/".to_owned()];
        let login = report_from_evaluation(
            "example",
            "https://example.test/",
            &patterns,
            &json!({"result":{"value":{"status":"login_required","url":"https://example.test/"}}}),
        )
        .expect("login report");
        assert_eq!(login.health, ProviderHealth::LoginRequired);
        assert!(!login.logged_in);

        let navigation = report_from_evaluation(
            "example",
            "https://example.test/",
            &patterns,
            &json!({"result":{"value":{"status":"navigation_changed","url":"https://other.test/"}}}),
        )
        .expect("navigation report");
        assert_eq!(navigation.health, ProviderHealth::NetworkError);

        let healthy = report_from_evaluation(
            "example",
            "https://example.test/",
            &patterns,
            &json!({"result":{"value":{"status":"checked","url":"https://example.test/","composerFound":true,"focusSupported":true,"attachmentSupported":true}}}),
        )
        .expect("healthy report");
        assert_eq!(healthy.health, ProviderHealth::Healthy);
        assert!(healthy.page_accessible && healthy.logged_in && healthy.composer_found);
    }
}
