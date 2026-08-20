use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use askbridge_core::{AppError, Result};

use crate::browser::FileInputResult;

use super::composer::{ATTEMPT_TIMEOUT, PROBE_INTERVAL, polling_deadline, wait_for_next_probe};

pub(super) fn poll_file_input_preparation<F>(
    cancelled: &AtomicBool,
    timeout: Duration,
    evaluate: F,
) -> Result<FileInputResult>
where
    F: FnMut(Duration) -> Result<FileInputResult>,
{
    poll_file_input_preparation_with_interval(cancelled, timeout, PROBE_INTERVAL, evaluate)
}

pub(super) fn poll_file_input_preparation_with_interval<F>(
    cancelled: &AtomicBool,
    timeout: Duration,
    interval: Duration,
    mut evaluate: F,
) -> Result<FileInputResult>
where
    F: FnMut(Duration) -> Result<FileInputResult>,
{
    let deadline = polling_deadline(timeout, "file input")?;
    let mut observed_not_found = false;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            return if observed_not_found {
                Ok(FileInputResult::NotFound)
            } else {
                Err(AppError::TargetTimeout)
            };
        }
        let result = evaluate((deadline - now).min(ATTEMPT_TIMEOUT))?;
        if !matches!(result, FileInputResult::NotFound) {
            return Ok(result);
        }
        observed_not_found = true;
        wait_for_next_probe(cancelled, deadline, interval)?;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;

    #[test]
    fn missing_file_input_is_retried_until_prepared() {
        let cancelled = AtomicBool::new(false);
        let mut attempts = 0;
        let result = poll_file_input_preparation_with_interval(
            &cancelled,
            Duration::from_millis(100),
            Duration::ZERO,
            |_| {
                attempts += 1;
                Ok(if attempts == 1 {
                    FileInputResult::NotFound
                } else {
                    FileInputResult::Prepared
                })
            },
        )
        .expect("file input");
        assert_eq!(attempts, 2);
        assert_eq!(result, FileInputResult::Prepared);
    }
}
