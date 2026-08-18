use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppState {
    #[default]
    Idle,
    SelectingRegion,
    PreparingDispatch,
    StartingBrowser,
    ConnectingBrowser,
    ResolvingTarget,
    WaitingForPage,
    PreparingPage,
    PreparedForUser,
    Cancelling,
    Error,
}
