//! Microsoft Store startup registration backed by the packaged StartupTask.

use askbridge_core::{AppError, Result};
use windows::{
    ApplicationModel::{StartupTask, StartupTaskState},
    core::HSTRING,
};

const TASK_ID: &str = "AskBridgeStartupTask";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupSnapshot(bool);

pub fn snapshot() -> Result<StartupSnapshot> {
    let _apartment = crate::store_runtime::initialize_sta(
        "initializing Windows Runtime for startup settings failed",
    )?;
    Ok(StartupSnapshot(is_enabled(
        task_without_initialization()?
            .State()
            .map_err(|error| store_error("reading Microsoft Store startup state", error))?,
    )))
}

pub fn apply(enabled: bool) -> Result<()> {
    let _apartment = crate::store_runtime::initialize_sta(
        "initializing Windows Runtime for startup settings failed",
    )?;
    let task = task_without_initialization()?;
    let state = task
        .State()
        .map_err(|error| store_error("reading Microsoft Store startup state", error))?;

    if enabled {
        if is_enabled(state) {
            return Ok(());
        }
        let state = task
            .RequestEnableAsync()
            .and_then(|operation| operation.get())
            .map_err(|error| store_error("requesting Microsoft Store startup permission", error))?;
        if is_enabled(state) {
            Ok(())
        } else {
            Err(AppError::ConfigurationInvalid(
                "Windows did not enable AskBridge startup; enable it in Settings > Apps > Startup"
                    .to_owned(),
            ))
        }
    } else {
        if state == StartupTaskState::Enabled {
            task.Disable()
                .map_err(|error| store_error("disabling Microsoft Store startup task", error))?;
        }
        // An administrator policy can force startup on. Treat that state as
        // authoritative rather than failing application startup.
        Ok(())
    }
}

pub fn restore(snapshot: &StartupSnapshot) -> Result<()> {
    apply(snapshot.0)
}

pub fn is_current_executable_registered() -> Result<bool> {
    let _apartment = crate::store_runtime::initialize_sta(
        "initializing Windows Runtime for startup settings failed",
    )?;
    Ok(is_enabled(task_without_initialization()?.State().map_err(
        |error| store_error("reading Microsoft Store startup state", error),
    )?))
}

fn task_without_initialization() -> Result<StartupTask> {
    StartupTask::GetAsync(&HSTRING::from(TASK_ID))
        .and_then(|operation| operation.get())
        .map_err(|error| store_error("opening Microsoft Store startup task", error))
}

fn is_enabled(state: StartupTaskState) -> bool {
    state == StartupTaskState::Enabled || state == StartupTaskState::EnabledByPolicy
}

fn store_error(operation: &'static str, error: windows::core::Error) -> AppError {
    AppError::ConfigurationInvalid(format!("{operation}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_states_include_policy_state() {
        assert!(is_enabled(StartupTaskState::Enabled));
        assert!(is_enabled(StartupTaskState::EnabledByPolicy));
        assert!(!is_enabled(StartupTaskState::Disabled));
        assert!(!is_enabled(StartupTaskState::DisabledByUser));
        assert!(!is_enabled(StartupTaskState::DisabledByPolicy));
    }
}
