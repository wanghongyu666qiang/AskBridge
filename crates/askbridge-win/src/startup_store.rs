//! Microsoft Store startup registration backed by the packaged StartupTask.

use askbridge_core::{AppError, Result};
use windows::{
    ApplicationModel::{StartupTask, StartupTaskState},
    core::HSTRING,
};

const TASK_ID: &str = "AskBridgeStartupTask";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupSnapshot(bool);

/// Reconciles the packaged startup task during ordinary application launch.
///
/// This helper is separated from the WinRT calls so the launch policy can be
/// tested without requiring an installed package.
pub fn reconcile_on_launch(start_on_login: bool) -> Result<()> {
    reconcile_on_launch_with(start_on_login, apply, is_current_executable_registered)
}

fn reconcile_on_launch_with(
    _start_on_login: bool,
    _apply_setting: impl FnMut(bool) -> Result<()>,
    _verify_setting: impl FnMut() -> Result<bool>,
) -> Result<()> {
    // A packaged StartupTask persists its own state after the user changes it
    // in AskBridge or Windows Settings. Reopening that WinRT object during
    // every process launch is unnecessary and, on certification build
    // 10.0.26100.9168, faulted inside combase.dll before the UI was ready.
    Ok(())
}

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
    use std::cell::Cell;

    #[test]
    fn enabled_states_include_policy_state() {
        assert!(is_enabled(StartupTaskState::Enabled));
        assert!(is_enabled(StartupTaskState::EnabledByPolicy));
        assert!(!is_enabled(StartupTaskState::Disabled));
        assert!(!is_enabled(StartupTaskState::DisabledByUser));
        assert!(!is_enabled(StartupTaskState::DisabledByPolicy));
    }

    #[test]
    fn disabled_default_does_not_touch_winrt_during_launch() {
        let apply_calls = Cell::new(0);
        let verify_calls = Cell::new(0);

        reconcile_on_launch_with(
            false,
            |_| {
                apply_calls.set(apply_calls.get() + 1);
                Ok(())
            },
            || {
                verify_calls.set(verify_calls.get() + 1);
                Ok(false)
            },
        )
        .expect("disabled Store startup should not require WinRT during launch");

        assert_eq!(apply_calls.get(), 0);
        assert_eq!(verify_calls.get(), 0);
    }

    #[test]
    fn enabled_setting_does_not_reopen_winrt_during_launch() {
        let apply_calls = Cell::new(0);
        let verify_calls = Cell::new(0);

        reconcile_on_launch_with(
            true,
            |_| {
                apply_calls.set(apply_calls.get() + 1);
                Ok(())
            },
            || {
                verify_calls.set(verify_calls.get() + 1);
                Ok(true)
            },
        )
        .expect("persisted Store startup state should be left to Windows during launch");

        assert_eq!(apply_calls.get(), 0);
        assert_eq!(verify_calls.get(), 0);
    }
}
