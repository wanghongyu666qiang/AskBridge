pub mod browser;
pub mod capture;
pub mod command;
pub mod config;
pub mod config_store;
pub mod error;
pub mod hotkey;
pub mod preparation;
pub mod provider;
pub mod request;
pub mod state;
pub mod update_signing;
pub mod workflow;

pub use browser::{
    BrowserTarget, FocusEvidence, TargetDecision, TargetResolver, matches_any_pattern,
};
pub use capture::{CapturedImage, ScreenRect};
pub use command::AppCommand;
pub use config::{
    AppConfig, BrowserConfig, BrowserLifecycle, BrowserTargetPreference, GeneralConfig,
    HotkeyConfig,
};
pub use config_store::{ConfigLoad, ConfigStore};
pub use error::{AppError, PreparationFailureStage, PreparationRecovery, Result};
pub use hotkey::{HotkeyBinding, HotkeyValidationError, ModifierKey, VirtualKey};
pub use preparation::{PreparationOutcome, PreparationPolicy, SubmissionMode};
pub use provider::{BUILT_IN_ADAPTER_IDS, ProviderConfig, ProviderOverride};
pub use request::{DispatchMode, DispatchRequest};
pub use state::AppState;
pub use update_signing::RELEASE_SIGNING_PUBLIC_KEY;
pub use workflow::WorkflowController;
