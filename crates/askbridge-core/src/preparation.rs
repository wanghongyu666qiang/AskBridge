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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationFailureStage {
    PageReadiness,
    ComposerDiscovery,
    AttachmentPreparation,
    TextInsertion,
    Verification,
    NavigationChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryHint {
    Retry,
    ReopenProviderPage,
    LoginInBrowser,
    ProviderPageChanged,
    FocusComposerAndPaste,
    CopyImageThenText,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparationOutcome {
    pub target_url: String,
    pub text_inserted: bool,
    pub attachment_prepared: bool,
    pub manual_fallback_required: bool,
    pub submit_allowed: bool,
    pub failure_stage: Option<PreparationFailureStage>,
    pub recovery_hint: Option<RecoveryHint>,
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
            manual_fallback_required: false,
            submit_allowed: true,
            failure_stage: None,
            recovery_hint: None,
        }
    }

    pub fn manual_fallback(
        target_url: impl Into<String>,
        stage: PreparationFailureStage,
        hint: RecoveryHint,
        text_inserted: bool,
        attachment_prepared: bool,
    ) -> Self {
        Self {
            target_url: target_url.into(),
            text_inserted,
            attachment_prepared,
            manual_fallback_required: true,
            submit_allowed: false,
            failure_stage: Some(stage),
            recovery_hint: Some(hint),
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
        if self.manual_fallback_required {
            if self.submit_allowed || self.failure_stage.is_none() || self.recovery_hint.is_none() {
                return Err(AppError::InvalidPreparation(
                    "manual fallback result is internally inconsistent".to_owned(),
                ));
            }
            if self.text_inserted && (request.image.is_none() || self.attachment_prepared) {
                return Err(AppError::InvalidPreparation(
                    "manual fallback result already reports all content prepared".to_owned(),
                ));
            }
            return Ok(());
        }
        if !self.submit_allowed || self.failure_stage.is_some() || self.recovery_hint.is_some() {
            return Err(AppError::InvalidPreparation(
                "prepared result is internally inconsistent".to_owned(),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchOutcome {
    PreparedForUser(PreparationOutcome),
    ManualFallbackReady(PreparationOutcome),
    Cancelled,
}

impl DispatchOutcome {
    pub fn from_preparation(
        request: &DispatchRequest,
        outcome: PreparationOutcome,
    ) -> Result<Self> {
        outcome.validate_for(request)?;
        if outcome.manual_fallback_required {
            Ok(Self::ManualFallbackReady(outcome))
        } else {
            Ok(Self::PreparedForUser(outcome))
        }
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

        assert!(matches!(
            DispatchOutcome::from_preparation(&request, outcome),
            Ok(DispatchOutcome::PreparedForUser(_))
        ));
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

        assert!(matches!(
            DispatchOutcome::from_preparation(&request, outcome),
            Ok(DispatchOutcome::PreparedForUser(_))
        ));
    }

    #[test]
    fn fallback_results_require_stage_and_recovery_hint() {
        let request = text_request();
        let fallback = PreparationOutcome::manual_fallback(
            "https://example.test/chat",
            PreparationFailureStage::ComposerDiscovery,
            RecoveryHint::FocusComposerAndPaste,
            false,
            false,
        );

        assert!(matches!(
            DispatchOutcome::from_preparation(&request, fallback),
            Ok(DispatchOutcome::ManualFallbackReady(_))
        ));

        let contradictory = PreparationOutcome {
            target_url: "https://example.test/chat".to_owned(),
            text_inserted: false,
            attachment_prepared: false,
            manual_fallback_required: true,
            submit_allowed: true,
            failure_stage: None,
            recovery_hint: None,
        };
        assert!(contradictory.validate_for(&request).is_err());
        assert!(
            PreparationOutcome::manual_fallback(
                "https://example.test/chat",
                PreparationFailureStage::Verification,
                RecoveryHint::Retry,
                true,
                false,
            )
            .validate_for(&request)
            .is_err()
        );
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
