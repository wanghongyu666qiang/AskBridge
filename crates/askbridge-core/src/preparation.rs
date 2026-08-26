use serde::{Deserialize, Serialize};

use crate::{AppError, DispatchRequest, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionMode {
    UserConfirmationRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparationPolicy {
    pub timeout_ms: u64,
    pub submission_mode: SubmissionMode,
}

impl PreparationPolicy {
    pub fn new(timeout_ms: u64) -> Result<Self> {
        if timeout_ms == 0 {
            return Err(AppError::InvalidPreparation(
                "preparation timeout must be greater than zero".to_owned(),
            ));
        }
        // Mirror the browser-timeout cap so no single policy can outwait the
        // whole dispatch budget.
        let timeout_ms = timeout_ms.min(crate::config::MAX_BROWSER_TIMEOUT_MS);
        Ok(Self {
            timeout_ms,
            submission_mode: SubmissionMode::UserConfirmationRequired,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparationOutcome {
    pub target_url: String,
    pub text_inserted: bool,
    pub attachment_prepared: bool,
}

impl PreparationOutcome {
    pub fn prepared(
        target_url: impl Into<String>,
        text_inserted: bool,
        attachment_prepared: bool,
    ) -> Self {
        Self {
            target_url: target_url.into(),
            text_inserted,
            attachment_prepared,
        }
    }

    pub fn validate_for(&self, request: &DispatchRequest) -> Result<()> {
        if self.target_url.trim().is_empty() {
            return Err(AppError::InvalidPreparation(
                "target URL must not be empty".to_owned(),
            ));
        }
        if request.image.is_none() && self.attachment_prepared {
            return Err(AppError::InvalidPreparation(
                "text-only result unexpectedly reports an attachment".to_owned(),
            ));
        }
        if (request.expects_text() && !self.text_inserted)
            || (request.image.is_some() && !self.attachment_prepared)
        {
            return Err(AppError::InvalidPreparation(
                "prepared result has not verified all requested content".to_owned(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DispatchMode;

    fn text_request() -> DispatchRequest {
        DispatchRequest::new(
            "request-1".to_owned(),
            DispatchMode::TextOnlyPrompt,
            "chatgpt".to_owned(),
            "Explain this".to_owned(),
            None,
            1,
        )
        .expect("request")
    }

    #[test]
    fn prepared_results_require_verified_text_and_user_confirmation() {
        let request = text_request();
        let outcome = PreparationOutcome::prepared("https://example.test/chat", true, false);

        assert!(outcome.validate_for(&request).is_ok());
        assert!(
            PreparationOutcome::prepared("https://example.test/chat", false, false)
                .validate_for(&request)
                .is_err()
        );
    }

    #[test]
    fn prepared_results_can_only_focus_web_composer_when_no_text_is_requested() {
        let request = DispatchRequest::new(
            "request-1".to_owned(),
            DispatchMode::TextOnlyPrompt,
            "chatgpt".to_owned(),
            String::new(),
            None,
            1,
        )
        .expect("request");
        let outcome = PreparationOutcome::prepared("https://example.test/chat", false, false);

        assert!(outcome.validate_for(&request).is_ok());
    }

    #[test]
    fn policy_rejects_unbounded_zero_timeout() {
        assert!(PreparationPolicy::new(0).is_err());
        assert_eq!(
            PreparationPolicy::new(1_000)
                .expect("policy")
                .submission_mode,
            SubmissionMode::UserConfirmationRequired
        );
    }

    #[test]
    fn policy_caps_timeout_at_the_browser_limit() {
        let capped = PreparationPolicy::new(u64::MAX).expect("policy");
        assert_eq!(capped.timeout_ms, crate::config::MAX_BROWSER_TIMEOUT_MS);
    }

    #[test]
    fn image_results_without_attachment_evidence_are_rejected() {
        let request = DispatchRequest::new(
            "request-1".to_owned(),
            DispatchMode::CaptureWithPrompt,
            "chatgpt".to_owned(),
            "Explain this".to_owned(),
            Some(
                crate::CapturedImage::new(
                    1,
                    1,
                    vec![0, 0, 0, 255],
                    crate::ScreenRect::new(0, 0, 1, 1),
                )
                .expect("image"),
            ),
            1,
        )
        .expect("request");

        let outcome = PreparationOutcome::prepared("https://example.test/chat", true, false);
        assert!(matches!(
            outcome.validate_for(&request),
            Err(AppError::InvalidPreparation(_))
        ));
        let complete = PreparationOutcome::prepared("https://example.test/chat", true, true);
        complete.validate_for(&request).expect("complete outcome");
    }
}
