use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppCommand {
    CaptureWithPrompt,
    CaptureQuickDispatch,
    TextOnlyPrompt,
}

impl AppCommand {
    pub const ALL: [Self; 3] = [
        Self::CaptureWithPrompt,
        Self::CaptureQuickDispatch,
        Self::TextOnlyPrompt,
    ];

    pub const fn event_name(self) -> &'static str {
        match self {
            Self::CaptureWithPrompt => "capture_with_prompt",
            Self::CaptureQuickDispatch => "capture_quick_dispatch",
            Self::TextOnlyPrompt => "text_only_prompt",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::CaptureWithPrompt => "截图并提问",
            Self::CaptureQuickDispatch => "截图快速投递",
            Self::TextOnlyPrompt => "直接文字提问",
        }
    }
}
