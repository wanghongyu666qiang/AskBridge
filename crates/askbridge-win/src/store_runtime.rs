//! Small RAII guard for Windows Runtime calls used only by the Store build.

use askbridge_core::{AppError, Result};
use windows::Win32::System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize};

pub struct Apartment;

pub fn initialize_sta(operation: &'static str) -> Result<Apartment> {
    // SAFETY: The Store APIs are called on the application's UI thread. Each
    // successful initialization is balanced by this guard's Drop.
    unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }
        .map(|()| Apartment)
        .map_err(|error| AppError::ConfigurationInvalid(format!("{operation}: {error}")))
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: This guard is created only after a successful RoInitialize.
        unsafe { RoUninitialize() };
    }
}
