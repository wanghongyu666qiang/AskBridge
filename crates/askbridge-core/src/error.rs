use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("hotkey registration failed for {binding} (Win32 error {win32_code})")]
    HotkeyRegistrationFailed { binding: String, win32_code: u32 },

    #[error("hotkey conflict: {0}")]
    HotkeyConflict(String),

    #[error("invalid hotkey: {0}")]
    InvalidHotkey(String),

    #[error("hotkey transaction failed: {0}")]
    HotkeyTransactionFailed(String),

    #[error("configuration is invalid: {0}")]
    ConfigurationInvalid(String),

    #[error("configuration schema {found} is newer than supported schema {supported}")]
    UnsupportedConfigurationSchema { found: u32, supported: u32 },

    #[error("failed to parse configuration at {path}: {source}")]
    ConfigurationParse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("I/O error while {context} at {path}: {source}")]
    Io {
        context: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("single-instance operation failed (Win32 error {win32_code})")]
    SingleInstance { win32_code: u32 },

    #[error("Windows operation '{operation}' failed (Win32 error {win32_code})")]
    Windows {
        operation: &'static str,
        win32_code: u32,
    },

    #[error("application is already running")]
    AlreadyRunning,

    #[error("capture was cancelled")]
    CaptureCancelled,

    #[error("capture failed: {0}")]
    CaptureFailed(String),

    #[error("clipboard is unavailable")]
    ClipboardUnavailable,

    #[error("clipboard write failed")]
    ClipboardWriteFailed,

    #[error("clipboard restore failed")]
    ClipboardRestoreFailed,

    #[error("invalid provider: {0}")]
    InvalidProvider(String),

    #[error("invalid provider URL: {0}")]
    InvalidProviderUrl(String),

    #[error("invalid dispatch request: {0}")]
    InvalidDispatchRequest(String),

    #[error("workflow is busy in state {0}")]
    WorkflowBusy(String),

    #[error("workflow event '{event}' is invalid in state {state}")]
    InvalidWorkflowTransition { state: String, event: String },

    #[error("browser launch failed")]
    BrowserLaunchFailed,

    #[error("target not found")]
    TargetNotFound,

    #[error("target timed out")]
    TargetTimeout,

    #[error("web composer not found")]
    ComposerNotFound,
}

impl AppError {
    pub fn io(context: &'static str, path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            context,
            path: path.into(),
            source,
        }
    }
}
