pub mod capture;
pub mod command;
pub mod config;
pub mod config_store;
pub mod error;
pub mod hotkey;
pub mod provider;
pub mod state;

pub use capture::{CapturedImage, ScreenRect};
pub use command::AppCommand;
pub use config::{AppConfig, BrowserConfig, GeneralConfig, HotkeyConfig};
pub use config_store::{ConfigLoad, ConfigStore};
pub use error::{AppError, Result};
pub use hotkey::{HotkeyBinding, HotkeyValidationError, ModifierKey, VirtualKey};
pub use provider::{ProviderConfig, ProviderOverride};
pub use state::AppState;
