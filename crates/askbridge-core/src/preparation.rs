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
}
