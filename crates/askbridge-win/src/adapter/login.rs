use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

use askbridge_core::{
    AppError, PreparationFailureStage, PreparationRecovery, Result, matches_any_pattern,
};
use serde_json::Value;

use crate::browser::{CdpClient, CdpTarget};

use super::{javascript::login_detection_expression, rules::ProviderRule};

pub(super) fn current_target(
    client: &CdpClient,
    target_id: &str,
    timeout: Duration,
) -> Result<CdpTarget> {
    client
        .list_targets_with_timeout(timeout)?
        .into_iter()
        .find(|candidate| candidate.id == target_id && candidate.kind == "page")
        .ok_or(AppError::TargetNotFound)
}

pub(super) fn verify_page_and_login(
    client: &CdpClient,
    target_id: &str,
    rule: Option<&ProviderRule>,
    url_patterns: &[String],
    cancelled: &AtomicBool,
    timeout: Duration,
) -> Result<CdpTarget> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| AppError::InvalidPreparation("login timeout is too large".to_owned()))?;
    let remaining = || {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(AppError::TargetTimeout)
        } else {
            Ok(remaining)
        }
    };
    let current = current_target(client, target_id, remaining()?)?;
    if rule.is_some_and(|rule| rule.matches_login_url(&current.url)) {
        return Err(page_readiness_failure());
    }
    if !matches_any_pattern(&current.url, url_patterns) {
        return Err(navigation_failure());
    }

    let Some(rule) = rule.filter(|rule| !rule.login_selectors().is_empty()) else {
        return Ok(current);
    };
    let expression = login_detection_expression(rule.login_selectors())?;
    let result = client.evaluate_in_target(&current, &expression, cancelled, remaining()?)?;
    let value = result
        .pointer("/result/value")
        .ok_or_else(|| AppError::BrowserProtocol("login detection returned no value".to_owned()))?;
    let target_url = value
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or(&current.url);
    if rule.matches_login_url(target_url)
        || value
            .get("loginDetected")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return Err(page_readiness_failure());
    }
    if !matches_any_pattern(target_url, url_patterns) {
        return Err(navigation_failure());
    }
    Ok(current)
}

fn page_readiness_failure() -> AppError {
    AppError::PreparationFailed {
        stage: PreparationFailureStage::PageReadiness,
        recovery: PreparationRecovery::LoginInBrowser,
        text_inserted: false,
        attachment_prepared: false,
    }
}

fn navigation_failure() -> AppError {
    AppError::PreparationFailed {
        stage: PreparationFailureStage::NavigationChanged,
        recovery: PreparationRecovery::ReopenProviderPage,
        text_inserted: false,
        attachment_prepared: false,
    }
}

#[cfg(test)]
mod tests {
    use askbridge_core::{PreparationFailureStage, PreparationRecovery};

    use super::*;

    #[test]
    fn login_failure_is_explicit_and_never_claims_mutation() {
        assert!(matches!(
            page_readiness_failure(),
            AppError::PreparationFailed {
                stage: PreparationFailureStage::PageReadiness,
                recovery: PreparationRecovery::LoginInBrowser,
                text_inserted: false,
                attachment_prepared: false,
            }
        ));
    }
}
