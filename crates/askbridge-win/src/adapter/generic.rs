use std::{
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

use askbridge_core::{
    AppError, DispatchRequest, PreparationFailureStage, PreparationOutcome, PreparationPolicy,
    PreparationRecovery, Result, matches_any_pattern,
};
use serde_json::Value;

use crate::{
    browser::{CdpClient, CdpTarget, FileInputResult},
    capture::encoder::encode_png,
};

use super::{
    attachment::poll_file_input_preparation,
    composer::poll_composer_preparation,
    javascript::composer_insertion_expression,
    login::{current_target, verify_page_and_login},
    rules::{ProviderRule, load_rule},
    session::PageSession,
    temp_image::create_retained_page_upload,
    r#trait::ProviderAdapter,
};

#[cfg(test)]
use super::{
    attachment::poll_file_input_preparation_with_interval,
    composer::poll_composer_preparation_with_interval, javascript::login_detection_expression,
    temp_image::cleanup_temp_images_older_than,
};

pub struct GenericProviderAdapter {
    provider_id: String,
    url_patterns: Vec<String>,
    rule: Option<ProviderRule>,
}

#[derive(Debug, Clone, Copy)]
struct PreparationDeadline {
    deadline: Instant,
}

impl PreparationDeadline {
    fn new(timeout: Duration) -> Result<Self> {
        if timeout.is_zero() {
            return Err(AppError::TargetTimeout);
        }
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            AppError::InvalidPreparation("preparation timeout is too large".to_owned())
        })?;
        Ok(Self { deadline })
    }

    fn remaining(self) -> Result<Duration> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            Err(AppError::TargetTimeout)
        } else {
            Ok(remaining)
        }
    }

    fn bounded(self, upper_bound: Duration) -> Result<Duration> {
        self.remaining().map(|remaining| remaining.min(upper_bound))
    }
}

impl GenericProviderAdapter {
    pub fn for_provider(
        provider_id: impl Into<String>,
        adapter_override: Option<&str>,
        url_patterns: Vec<String>,
    ) -> Result<Self> {
        let provider_id = provider_id.into();
        let rule = load_rule(adapter_override)?;
        Ok(Self {
            provider_id,
            url_patterns,
            rule,
        })
    }

    fn prepare_dedicated_chrome(
        &self,
        client: &CdpClient,
        target: &CdpTarget,
        temp_root: &Path,
        cancelled: &AtomicBool,
        request: &DispatchRequest,
        policy: &PreparationPolicy,
    ) -> Result<PreparationOutcome> {
        let timeout = Duration::from_millis(policy.timeout_ms);
        let deadline = PreparationDeadline::new(timeout)?;
        let login_timeout = deadline.remaining().map_err(|error| {
            read_only_preparation_failure(error, PreparationFailureStage::PageReadiness)
        })?;
        let current = verify_page_and_login(
            client,
            &target.id,
            self.rule.as_ref(),
            &self.url_patterns,
            cancelled,
            login_timeout,
        )
        .map_err(|error| {
            read_only_preparation_failure(error, PreparationFailureStage::PageReadiness)
        })?;

        let attachment_prepared = if let Some(image) = &request.image {
            let navigation_check_timeout = deadline.remaining().map_err(|error| {
                read_only_preparation_failure(error, PreparationFailureStage::NavigationChanged)
            })?;
            let target_matches = client
                .target_url_matches(&current, &current.url, cancelled, navigation_check_timeout)
                .map_err(|error| {
                    read_only_preparation_failure(error, PreparationFailureStage::NavigationChanged)
                })?;
            if !target_matches {
                return Err(preparation_failed(
                    PreparationFailureStage::NavigationChanged,
                    PreparationRecovery::ReopenProviderPage,
                    false,
                    false,
                ));
            }
            // A page-visible preview does not prove that the provider has
            // finished reading or uploading the file. Keep the backing PNG
            // alive for a bounded grace period even if later verification is
            // ambiguous or fails.
            let temp_image_path =
                create_retained_page_upload(temp_root, &request.id, &encode_png(image)?)?;
            let preferred_selectors = self
                .rule
                .as_ref()
                .map_or(&[][..], |rule| rule.file_input_selectors());
            let prepare_file_input = |target: &CdpTarget| {
                let poll_timeout = deadline.remaining().map_err(|error| {
                    read_only_preparation_failure(
                        error,
                        PreparationFailureStage::AttachmentPreparation,
                    )
                })?;
                poll_file_input_preparation(cancelled, poll_timeout, |attempt_timeout| {
                    let attempt_timeout = deadline.bounded(attempt_timeout).map_err(|error| {
                        read_only_preparation_failure(
                            error,
                            PreparationFailureStage::AttachmentPreparation,
                        )
                    })?;
                    client.set_file_input(
                        target,
                        &target.url,
                        &temp_image_path,
                        preferred_selectors,
                        cancelled,
                        attempt_timeout,
                    )
                })
            };
            let mut file_input_result = prepare_file_input(&current)?;
            if matches!(file_input_result, FileInputResult::NavigationChanged) {
                let refreshed = current_target(client, &current.id, deadline.remaining()?)?;
                if self
                    .rule
                    .as_ref()
                    .is_some_and(|rule| rule.matches_login_url(&refreshed.url))
                {
                    return Err(preparation_failed(
                        PreparationFailureStage::PageReadiness,
                        PreparationRecovery::LoginInBrowser,
                        false,
                        false,
                    ));
                }
                if !self.matches_url(&refreshed.url) {
                    return Err(preparation_failed(
                        PreparationFailureStage::NavigationChanged,
                        PreparationRecovery::ReopenProviderPage,
                        false,
                        false,
                    ));
                }
                file_input_result = prepare_file_input(&refreshed)?;
            }
            attachment_prepared_or_error(file_input_result)?
        } else {
            false
        };

        let composer_target = current_target(client, &current.id, deadline.remaining()?)?;
        if self
            .rule
            .as_ref()
            .is_some_and(|rule| rule.matches_login_url(&composer_target.url))
        {
            return Err(preparation_failed(
                PreparationFailureStage::PageReadiness,
                PreparationRecovery::LoginInBrowser,
                false,
                attachment_prepared,
            ));
        }
        if !self.matches_url(&composer_target.url) {
            return Err(preparation_failed(
                PreparationFailureStage::NavigationChanged,
                PreparationRecovery::ReopenProviderPage,
                false,
                attachment_prepared,
            ));
        }
        let expected_url = composer_target.url.clone();
        let preferred_selectors = self
            .rule
            .as_ref()
            .map_or(&[][..], |rule| rule.composer_selectors());
        let login_selectors = self
            .rule
            .as_ref()
            .map_or(&[][..], |rule| rule.login_selectors());
        let expression = composer_insertion_expression(
            &request.prompt,
            preferred_selectors,
            login_selectors,
            &expected_url,
        )?;
        let result =
            poll_composer_preparation(cancelled, deadline.remaining()?, |attempt_timeout| {
                client.evaluate_in_target(
                    &composer_target,
                    &expression,
                    cancelled,
                    deadline.bounded(attempt_timeout)?,
                )
            })?;
        let value = result.pointer("/result/value").ok_or_else(|| {
            AppError::BrowserProtocol("composer preparation returned no value".to_owned())
        })?;
        let target_url = value
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or(&expected_url);
        let verified_target = current_target(client, &composer_target.id, deadline.remaining()?)?;
        if self.rule.as_ref().is_some_and(|rule| {
            rule.matches_login_url(target_url) || rule.matches_login_url(&verified_target.url)
        }) {
            return Err(preparation_failed(
                PreparationFailureStage::PageReadiness,
                PreparationRecovery::LoginInBrowser,
                false,
                attachment_prepared,
            ));
        }
        if value.get("status").and_then(Value::as_str) == Some("navigation_changed") {
            return Err(preparation_failed(
                PreparationFailureStage::NavigationChanged,
                PreparationRecovery::ReopenProviderPage,
                false,
                attachment_prepared,
            ));
        }
        if !self.matches_url(target_url) || !self.matches_url(&verified_target.url) {
            return Err(preparation_failed(
                PreparationFailureStage::NavigationChanged,
                PreparationRecovery::ReopenProviderPage,
                false,
                attachment_prepared,
            ));
        }
        match value.get("status").and_then(Value::as_str) {
            Some("inserted") => Ok(PreparationOutcome::prepared(
                target_url,
                true,
                attachment_prepared,
            )),
            Some("focused") => Ok(PreparationOutcome::prepared(
                target_url,
                false,
                attachment_prepared,
            )),
            Some("login_detected") => Err(preparation_failed(
                PreparationFailureStage::PageReadiness,
                PreparationRecovery::LoginInBrowser,
                false,
                attachment_prepared,
            )),
            Some("missing") => Err(preparation_failed(
                PreparationFailureStage::ComposerDiscovery,
                if value
                    .get("providerRuleMiss")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    PreparationRecovery::ProviderPageChanged
                } else {
                    PreparationRecovery::Retry
                },
                false,
                attachment_prepared,
            )),
            Some("ambiguous") => Err(preparation_failed(
                PreparationFailureStage::ComposerDiscovery,
                PreparationRecovery::Retry,
                false,
                attachment_prepared,
            )),
            Some("verification_failed") => Err(preparation_failed(
                PreparationFailureStage::Verification,
                PreparationRecovery::Retry,
                false,
                attachment_prepared,
            )),
            _ => Err(AppError::BrowserProtocol(
                "composer preparation returned an invalid status".to_owned(),
            )),
        }
    }
}

impl ProviderAdapter for GenericProviderAdapter {
    fn id(&self) -> &str {
        &self.provider_id
    }

    fn matches_url(&self, url: &str) -> bool {
        matches_any_pattern(url, &self.url_patterns)
    }

    fn prepare(
        &self,
        page: &mut PageSession<'_>,
        request: &DispatchRequest,
        policy: &PreparationPolicy,
    ) -> Result<PreparationOutcome> {
        request.validate()?;
        let outcome = match page {
            PageSession::DedicatedChrome {
                client,
                target,
                temp_root,
                cancelled,
            } => {
                self.prepare_dedicated_chrome(client, target, temp_root, cancelled, request, policy)
            }
            PageSession::DesktopPwa { target_url }
                if !request.expects_text() && request.image.is_none() =>
            {
                Ok(PreparationOutcome::prepared(*target_url, false, false))
            }
            PageSession::DesktopPwa { .. } => Err(preparation_failed(
                PreparationFailureStage::ComposerDiscovery,
                PreparationRecovery::UseDedicatedChrome,
                false,
                false,
            )),
        }?;
        Ok(outcome)
    }
}

fn preparation_failed(
    stage: PreparationFailureStage,
    recovery: PreparationRecovery,
    text_inserted: bool,
    attachment_prepared: bool,
) -> AppError {
    AppError::PreparationFailed {
        stage,
        recovery,
        text_inserted,
        attachment_prepared,
    }
}

fn read_only_preparation_failure(error: AppError, stage: PreparationFailureStage) -> AppError {
    match error {
        AppError::BrowserCancelled | AppError::PreparationFailed { .. } => error,
        _ => preparation_failed(stage, PreparationRecovery::Retry, false, false),
    }
}

fn attachment_prepared_or_error(result: FileInputResult) -> Result<bool> {
    match result {
        FileInputResult::Prepared => Ok(true),
        FileInputResult::NavigationChanged => Err(preparation_failed(
            PreparationFailureStage::NavigationChanged,
            PreparationRecovery::ReopenProviderPage,
            false,
            false,
        )),
        FileInputResult::NotFound | FileInputResult::Ambiguous => Err(preparation_failed(
            PreparationFailureStage::AttachmentPreparation,
            PreparationRecovery::UseDedicatedChrome,
            false,
            false,
        )),
        FileInputResult::VerificationFailed => Err(preparation_failed(
            PreparationFailureStage::Verification,
            PreparationRecovery::Retry,
            false,
            true,
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn fixed_composer_program_never_contains_submit_actions() {
        let expression =
            composer_insertion_expression("hello\nworld", &[], &[], "https://example.test/chat")
                .expect("expression");
        assert!(expression.contains("InputEvent('input'"));
        assert!(!expression.contains(".click("));
        assert!(!expression.contains("dispatchKeyEvent"));
    }

    #[test]
    fn composer_navigation_guard_precedes_dom_queries_and_mutation() {
        let expression =
            composer_insertion_expression("hello", &[], &[], "https://example.test/chat")
                .expect("expression");
        let guard = expression
            .find("if (location.href !== expectedUrl)")
            .expect("navigation guard");
        assert!(
            guard
                < expression
                    .find("document.querySelectorAll")
                    .expect("DOM query")
        );
        assert!(guard < expression.find("el.focus()").expect("focus"));
        assert!(guard < expression.find("InputEvent('input'").expect("input event"));
        assert!(expression.contains("status: 'navigation_changed'"));
    }

    #[test]
    fn login_detection_program_only_checks_structural_selectors() {
        let expression =
            login_detection_expression(&["a[href*=\"//accounts.google.com/\"]".to_owned()])
                .expect("expression");
        assert!(expression.contains("//accounts.google.com/"));
        assert!(expression.contains("loginDetected"));
        assert!(!expression.contains(".click("));
        assert!(!expression.contains("textContent"));
        assert!(!expression.contains("innerText"));
    }

    #[test]
    fn built_in_adapter_embeds_preferred_selectors_but_keeps_generic_fallback() {
        let adapter = GenericProviderAdapter::for_provider(
            "chatgpt",
            Some("chatgpt"),
            vec!["https://chatgpt.com/".to_owned()],
        )
        .expect("adapter");
        let selectors = adapter.rule.as_ref().expect("rule").composer_selectors();
        let expression =
            composer_insertion_expression("hello", selectors, &[], "https://chatgpt.com/chat")
                .expect("expression");
        assert!(expression.contains("#prompt-textarea"));
        assert!(expression.contains("textarea,[contenteditable=\"true\"]"));
        assert!(!expression.contains(".click("));
    }

    #[test]
    fn composer_poll_retries_missing_result_until_inserted() {
        let cancelled = AtomicBool::new(false);
        let mut attempts = 0;
        let result = poll_composer_preparation_with_interval(
            &cancelled,
            Duration::from_secs(1),
            Duration::ZERO,
            |_| {
                attempts += 1;
                Ok(if attempts < 3 {
                    serde_json::json!({"result": {"value": {"status": "missing"}}})
                } else {
                    serde_json::json!({"result": {"value": {"status": "inserted"}}})
                })
            },
        )
        .expect("poll result");

        assert_eq!(attempts, 3);
        assert_eq!(
            result
                .pointer("/result/value/status")
                .and_then(Value::as_str),
            Some("inserted")
        );
    }

    #[test]
    fn file_input_poll_retries_not_found_until_prepared() {
        let cancelled = AtomicBool::new(false);
        let mut attempts = 0;
        let result = poll_file_input_preparation_with_interval(
            &cancelled,
            Duration::from_secs(1),
            Duration::ZERO,
            |_| {
                attempts += 1;
                Ok(if attempts < 3 {
                    FileInputResult::NotFound
                } else {
                    FileInputResult::Prepared
                })
            },
        )
        .expect("poll result");

        assert_eq!(attempts, 3);
        assert!(matches!(result, FileInputResult::Prepared));
    }

    #[test]
    fn file_input_poll_stops_on_ambiguity() {
        let cancelled = AtomicBool::new(false);
        let mut attempts = 0;
        let result = poll_file_input_preparation_with_interval(
            &cancelled,
            Duration::from_secs(1),
            Duration::ZERO,
            |_| {
                attempts += 1;
                Ok(FileInputResult::Ambiguous)
            },
        )
        .expect("poll result");

        assert_eq!(attempts, 1);
        assert!(matches!(result, FileInputResult::Ambiguous));
    }

    #[test]
    fn unverified_attachment_receipt_never_claims_no_write_for_fallback() {
        let error = attachment_prepared_or_error(FileInputResult::VerificationFailed)
            .expect_err("missing receipt must fail closed");
        assert!(matches!(
            error,
            AppError::PreparationFailed {
                stage: PreparationFailureStage::Verification,
                text_inserted: false,
                attachment_prepared: true,
                ..
            }
        ));
    }

    #[test]
    fn read_only_timeout_records_explicit_no_write_evidence() {
        let error = read_only_preparation_failure(
            AppError::TargetTimeout,
            PreparationFailureStage::PageReadiness,
        );
        assert!(matches!(
            error,
            AppError::PreparationFailed {
                stage: PreparationFailureStage::PageReadiness,
                recovery: PreparationRecovery::Retry,
                text_inserted: false,
                attachment_prepared: false,
            }
        ));
    }

    #[test]
    fn composer_poll_stops_for_visible_login_structure() {
        let cancelled = AtomicBool::new(false);
        let mut attempts = 0;
        let result = poll_composer_preparation_with_interval(
            &cancelled,
            Duration::from_secs(1),
            Duration::ZERO,
            |_| {
                attempts += 1;
                Ok(serde_json::json!({
                    "result": {"value": {"status": "login_detected"}}
                }))
            },
        )
        .expect("poll result");

        assert_eq!(attempts, 1);
        assert_eq!(
            result
                .pointer("/result/value/status")
                .and_then(Value::as_str),
            Some("login_detected")
        );
    }

    #[test]
    fn composer_poll_honours_cancellation_before_evaluation() {
        let cancelled = AtomicBool::new(true);
        let result = poll_composer_preparation_with_interval(
            &cancelled,
            Duration::from_secs(1),
            Duration::ZERO,
            |_| panic!("cancelled polling must not evaluate the page"),
        );

        assert!(matches!(result, Err(AppError::BrowserCancelled)));
    }

    #[test]
    fn startup_cleanup_removes_only_owned_temp_images() {
        let directory = tempfile::tempdir().expect("temporary data directory");
        let temp_root = directory.path().join("Temp");
        fs::create_dir_all(&temp_root).expect("create temp root");
        let owned = temp_root.join("askbridge-stale.png");
        let unrelated = temp_root.join("keep.png");
        fs::write(&owned, b"owned").expect("write owned temp image");
        fs::write(&unrelated, b"unrelated").expect("write unrelated image");

        cleanup_temp_images_older_than(directory.path(), Duration::ZERO)
            .expect("cleanup owned images");

        assert!(!owned.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn generic_adapter_matches_only_provider_url_boundaries() {
        let adapter = GenericProviderAdapter::for_provider(
            "example",
            None,
            vec!["https://example.test/chat".to_owned()],
        )
        .expect("adapter");
        assert_eq!(adapter.id(), "example");
        assert!(adapter.matches_url("https://example.test/chat/1"));
        assert!(!adapter.matches_url("https://example.test/chatter"));
    }

    #[test]
    fn desktop_surface_rejects_automatic_preparation() {
        let adapter = GenericProviderAdapter::for_provider(
            "chatgpt",
            None,
            vec!["https://chatgpt.com/".to_owned()],
        )
        .expect("adapter");
        let request = DispatchRequest::new(
            "text-1".to_owned(),
            askbridge_core::DispatchMode::TextOnlyPrompt,
            "chatgpt".to_owned(),
            "Explain".to_owned(),
            None,
            1,
        )
        .expect("request");
        let policy = PreparationPolicy::new(1_000).expect("policy");
        let mut page = PageSession::DesktopPwa {
            target_url: "desktop-pwa://chatgpt",
        };

        let error = adapter
            .prepare(&mut page, &request, &policy)
            .expect_err("desktop PWA preparation must stop");
        assert!(matches!(
            error,
            AppError::PreparationFailed {
                recovery: PreparationRecovery::UseDedicatedChrome,
                ..
            }
        ));
    }

    #[test]
    fn preparation_failure_preserves_navigation_recovery() {
        let error = preparation_failed(
            PreparationFailureStage::NavigationChanged,
            PreparationRecovery::ReopenProviderPage,
            false,
            false,
        );

        assert!(matches!(
            error,
            AppError::PreparationFailed {
                stage: PreparationFailureStage::NavigationChanged,
                recovery: PreparationRecovery::ReopenProviderPage,
                ..
            }
        ));
    }
}
