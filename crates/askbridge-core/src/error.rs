use std::fmt;

use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparationFailureStage {
    PageReadiness,
    ComposerDiscovery,
    AttachmentPreparation,
    Verification,
    NavigationChanged,
}

impl fmt::Display for PreparationFailureStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::PageReadiness => "page-readiness",
            Self::ComposerDiscovery => "composer-discovery",
            Self::AttachmentPreparation => "attachment-preparation",
            Self::Verification => "verification",
            Self::NavigationChanged => "navigation-changed",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparationRecovery {
    Retry,
    ReopenProviderPage,
    LoginInBrowser,
    ProviderPageChanged,
    UseDedicatedChrome,
}

impl fmt::Display for PreparationRecovery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Retry => "retry",
            Self::ReopenProviderPage => "reopen-provider-page",
            Self::LoginInBrowser => "login-in-browser",
            Self::ProviderPageChanged => "provider-page-changed",
            Self::UseDedicatedChrome => "use-dedicated-chrome",
        };
        formatter.write_str(name)
    }
}

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

    #[error("capture failed: {0}")]
    CaptureFailed(String),

    #[error("clipboard is unavailable")]
    ClipboardUnavailable,

    #[error("clipboard write failed")]
    ClipboardWriteFailed,

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

    #[error("desktop shortcut was not found for provider '{0}'")]
    DesktopShortcutNotFound(String),

    #[error("desktop shortcut is unsafe: {0}")]
    DesktopShortcutRejected(String),

    #[error("desktop application launch failed (Shell error {0})")]
    DesktopLaunchFailed(isize),

    #[error("Google Chrome was not found")]
    ChromeNotFound,

    #[error("browser profile is unsafe: {0}")]
    BrowserProfileRejected(String),

    #[error("browser profile is already in use by another AskBridge Chrome process")]
    BrowserProfileInUse,

    #[error("browser debugging endpoint is unavailable")]
    BrowserEndpointUnavailable,

    #[error("browser connection failed: {0}")]
    BrowserConnectionFailed(String),

    #[error("browser protocol failed: {0}")]
    BrowserProtocol(String),

    #[error("browser operation was cancelled")]
    BrowserCancelled,

    #[error("application update failed: {0}")]
    UpdateFailed(String),

    #[error("clipboard paste target window was not found")]
    PasteTargetUnavailable,

    #[error("target not found")]
    TargetNotFound,

    #[error("target timed out")]
    TargetTimeout,

    #[error("page preparation is invalid: {0}")]
    InvalidPreparation(String),

    #[error(
        "page preparation failed at {stage}; recovery={recovery}; text_inserted={text_inserted}; attachment_prepared={attachment_prepared}"
    )]
    PreparationFailed {
        stage: PreparationFailureStage,
        recovery: PreparationRecovery,
        text_inserted: bool,
        attachment_prepared: bool,
    },
}

impl AppError {
    pub fn io(context: &'static str, path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            context,
            path: path.into(),
            source,
        }
    }

    /// Stable snake_case category for structured logging. Unlike `Display`,
    /// this never carries user content, file paths, or operating-system
    /// message text, so it is always safe to write into log fields.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::HotkeyRegistrationFailed { .. } => "hotkey_registration_failed",
            Self::HotkeyConflict(_) => "hotkey_conflict",
            Self::InvalidHotkey(_) => "invalid_hotkey",
            Self::HotkeyTransactionFailed(_) => "hotkey_transaction_failed",
            Self::ConfigurationInvalid(_) => "configuration_invalid",
            Self::UnsupportedConfigurationSchema { .. } => "unsupported_configuration_schema",
            Self::ConfigurationParse { .. } => "configuration_parse",
            Self::Io { .. } => "io",
            Self::SingleInstance { .. } => "single_instance",
            Self::Windows { .. } => "windows",
            Self::AlreadyRunning => "already_running",
            Self::CaptureFailed(_) => "capture_failed",
            Self::ClipboardUnavailable => "clipboard_unavailable",
            Self::ClipboardWriteFailed => "clipboard_write_failed",
            Self::InvalidProvider(_) => "invalid_provider",
            Self::InvalidProviderUrl(_) => "invalid_provider_url",
            Self::InvalidDispatchRequest(_) => "invalid_dispatch_request",
            Self::WorkflowBusy(_) => "workflow_busy",
            Self::InvalidWorkflowTransition { .. } => "invalid_workflow_transition",
            Self::BrowserLaunchFailed => "browser_launch_failed",
            Self::DesktopShortcutNotFound(_) => "desktop_shortcut_not_found",
            Self::DesktopShortcutRejected(_) => "desktop_shortcut_rejected",
            Self::DesktopLaunchFailed(_) => "desktop_launch_failed",
            Self::ChromeNotFound => "chrome_not_found",
            Self::BrowserProfileRejected(_) => "browser_profile_rejected",
            Self::BrowserProfileInUse => "browser_profile_in_use",
            Self::BrowserEndpointUnavailable => "browser_endpoint_unavailable",
            Self::BrowserConnectionFailed(_) => "browser_connection_failed",
            Self::BrowserProtocol(_) => "browser_protocol",
            Self::BrowserCancelled => "browser_cancelled",
            Self::UpdateFailed(_) => "update_failed",
            Self::PasteTargetUnavailable => "paste_target_unavailable",
            Self::TargetNotFound => "target_not_found",
            Self::TargetTimeout => "target_timeout",
            Self::InvalidPreparation(_) => "invalid_preparation",
            Self::PreparationFailed { .. } => "preparation_failed",
        }
    }
}
