use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use askbridge_core::{AppError, Result};
use serde_json::Value;

pub(super) const PROBE_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const ATTEMPT_TIMEOUT: Duration = Duration::from_millis(750);

pub(super) fn poll_composer_preparation<F>(
    cancelled: &AtomicBool,
    timeout: Duration,
    evaluate: F,
) -> Result<Value>
where
    F: FnMut(Duration) -> Result<Value>,
{
    poll_composer_preparation_with_interval(cancelled, timeout, PROBE_INTERVAL, evaluate)
}

pub(super) fn poll_composer_preparation_with_interval<F>(
    cancelled: &AtomicBool,
    timeout: Duration,
    interval: Duration,
    mut evaluate: F,
) -> Result<Value>
where
    F: FnMut(Duration) -> Result<Value>,
{
    let deadline = polling_deadline(timeout, "composer")?;
    let mut last_missing = None;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        let now = Instant::now();
        if now >= deadline {
            return last_missing.ok_or(AppError::TargetTimeout);
        }
        let result = evaluate((deadline - now).min(ATTEMPT_TIMEOUT))?;
        let status = result
            .pointer("/result/value/status")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AppError::BrowserProtocol(
                    "composer preparation returned an invalid status".to_owned(),
                )
            })?;
        if status != "missing" {
            return Ok(result);
        }
        last_missing = Some(result);
        wait_for_next_probe(cancelled, deadline, interval)?;
    }
}

pub(super) fn polling_deadline(timeout: Duration, stage: &str) -> Result<Instant> {
    Instant::now().checked_add(timeout).ok_or_else(|| {
        AppError::InvalidPreparation(format!("{stage} polling timeout is too large"))
    })
}

pub(super) fn wait_for_next_probe(
    cancelled: &AtomicBool,
    deadline: Instant,
    interval: Duration,
) -> Result<()> {
    let now = Instant::now();
    if now >= deadline {
        return Ok(());
    }
    let wait_until = now + interval.min(deadline - now);
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        let remaining = wait_until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(25).min(remaining));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use serde_json::json;

    use super::*;

    #[test]
    fn missing_composer_is_retried_until_found() {
        let cancelled = AtomicBool::new(false);
        let mut attempts = 0;
        let result = poll_composer_preparation_with_interval(
            &cancelled,
            Duration::from_millis(100),
            Duration::ZERO,
            |_| {
                attempts += 1;
                Ok(if attempts == 1 {
                    json!({"result":{"value":{"status":"missing"}}})
                } else {
                    json!({"result":{"value":{"status":"focused"}}})
                })
            },
        )
        .expect("composer");
        assert_eq!(attempts, 2);
        assert_eq!(
            result.pointer("/result/value/status"),
            Some(&json!("focused"))
        );
    }
}
