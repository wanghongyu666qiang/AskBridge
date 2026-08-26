use serde::{Deserialize, Serialize};

use crate::{AppCommand, AppError, CapturedImage, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchMode {
    CaptureWithPrompt,
    CaptureWithDefaultPrompt,
    TextOnlyPrompt,
}

impl From<AppCommand> for DispatchMode {
    fn from(command: AppCommand) -> Self {
        match command {
            AppCommand::CaptureWithPrompt => Self::CaptureWithPrompt,
            AppCommand::CaptureQuickDispatch => Self::CaptureWithDefaultPrompt,
            AppCommand::TextOnlyPrompt => Self::TextOnlyPrompt,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchRequest {
    pub id: String,
    pub mode: DispatchMode,
    pub provider_id: String,
    pub prompt: String,
    pub image: Option<CapturedImage>,
    /// Permanent compatibility tombstone: deserialization refuses to set it
    /// (`skip_deserializing`) and validation rejects `true`, so wire-level
    /// tampering cannot enable auto-submit. Do not resurrect this field as a
    /// feature.
    #[serde(default, skip_deserializing)]
    pub auto_submit: bool,
    pub created_at_ms: u64,
}

impl DispatchRequest {
    pub fn new(
        id: String,
        mode: DispatchMode,
        provider_id: String,
        prompt: String,
        image: Option<CapturedImage>,
        created_at_ms: u64,
    ) -> Result<Self> {
        let request = Self {
            id,
            mode,
            provider_id,
            prompt,
            image,
            auto_submit: false,
            created_at_ms,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.trim().is_empty() {
            return Err(AppError::InvalidDispatchRequest(
                "request id must not be empty".to_owned(),
            ));
        }
        if self.provider_id.trim().is_empty() {
            return Err(AppError::InvalidDispatchRequest(
                "provider id must not be empty".to_owned(),
            ));
        }
        if self.auto_submit {
            return Err(AppError::InvalidDispatchRequest(
                "auto_submit must remain false in AskBridge 1.0".to_owned(),
            ));
        }
        if self.mode == DispatchMode::CaptureWithDefaultPrompt && self.prompt.trim().is_empty() {
            return Err(AppError::InvalidDispatchRequest(
                "quick capture requests must contain the configured prompt".to_owned(),
            ));
        }
        match (self.mode, self.image.is_some()) {
            (DispatchMode::TextOnlyPrompt, true) => Err(AppError::InvalidDispatchRequest(
                "text-only requests must not contain an image".to_owned(),
            )),
            (DispatchMode::CaptureWithPrompt | DispatchMode::CaptureWithDefaultPrompt, false) => {
                Err(AppError::InvalidDispatchRequest(
                    "capture requests must contain an image".to_owned(),
                ))
            }
            _ => Ok(()),
        }
    }

    pub fn expects_text(&self) -> bool {
        !self.prompt.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ScreenRect;

    #[test]
    fn maps_all_commands_to_dispatch_modes() {
        assert_eq!(
            DispatchMode::from(AppCommand::CaptureWithPrompt),
            DispatchMode::CaptureWithPrompt
        );
        assert_eq!(
            DispatchMode::from(AppCommand::CaptureQuickDispatch),
            DispatchMode::CaptureWithDefaultPrompt
        );
        assert_eq!(
            DispatchMode::from(AppCommand::TextOnlyPrompt),
            DispatchMode::TextOnlyPrompt
        );
    }

    #[test]
    fn creates_capture_and_text_requests_with_auto_submit_disabled() {
        let capture = DispatchRequest::new(
            "capture-1".to_owned(),
            DispatchMode::CaptureWithPrompt,
            "chatgpt".to_owned(),
            "Explain this".to_owned(),
            Some(sample_image()),
            10,
        )
        .expect("capture request");
        let text = DispatchRequest::new(
            "text-1".to_owned(),
            DispatchMode::TextOnlyPrompt,
            "claude".to_owned(),
            "Review this idea".to_owned(),
            None,
            20,
        )
        .expect("text request");

        assert!(!capture.auto_submit);
        assert!(!text.auto_submit);
    }

    #[test]
    fn allows_web_composer_handoff_without_local_prompt_text() {
        let capture = DispatchRequest::new(
            "capture-1".to_owned(),
            DispatchMode::CaptureWithPrompt,
            "chatgpt".to_owned(),
            String::new(),
            Some(sample_image()),
            10,
        )
        .expect("capture request");
        let text = DispatchRequest::new(
            "text-1".to_owned(),
            DispatchMode::TextOnlyPrompt,
            "chatgpt".to_owned(),
            String::new(),
            None,
            20,
        )
        .expect("text handoff request");

        assert!(!capture.expects_text());
        assert!(!text.expects_text());
    }

    #[test]
    fn enforces_image_and_required_text_invariants() {
        assert!(
            DispatchRequest::new(
                "capture-1".to_owned(),
                DispatchMode::CaptureWithDefaultPrompt,
                "chatgpt".to_owned(),
                "Explain".to_owned(),
                None,
                10,
            )
            .is_err()
        );
        assert!(
            DispatchRequest::new(
                "text-1".to_owned(),
                DispatchMode::TextOnlyPrompt,
                "chatgpt".to_owned(),
                "Explain".to_owned(),
                Some(sample_image()),
                10,
            )
            .is_err()
        );
        assert!(
            DispatchRequest::new(
                "quick-1".to_owned(),
                DispatchMode::CaptureWithDefaultPrompt,
                "chatgpt".to_owned(),
                "   ".to_owned(),
                Some(sample_image()),
                10,
            )
            .is_err()
        );
    }

    #[test]
    fn deserialization_cannot_enable_auto_submit() {
        let request: DispatchRequest = serde_json::from_str(
            r#"{
                "id":"text-1",
                "mode":"text_only_prompt",
                "provider_id":"chatgpt",
                "prompt":"Explain",
                "image":null,
                "auto_submit":true,
                "created_at_ms":10
            }"#,
        )
        .expect("request JSON");

        assert!(!request.auto_submit);
        request.validate().expect("request remains valid");
    }

    fn sample_image() -> CapturedImage {
        CapturedImage::new(1, 1, vec![0, 0, 0, 255], ScreenRect::new(0, 0, 1, 1))
            .expect("sample image")
    }
}
