use std::collections::HashMap;

use askbridge_core::{AppCommand, AppError, HotkeyBinding, HotkeyConfig, ModifierKey, Result};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
    },
};

use crate::util::last_error;

#[derive(Debug, Clone)]
struct Registration {
    id: i32,
    binding: HotkeyBinding,
}

#[derive(Debug)]
struct Change {
    command: AppCommand,
    old: Option<Registration>,
    candidate: Option<Registration>,
}

pub(crate) trait HotkeyBackend {
    fn register(&mut self, id: i32, binding: &HotkeyBinding) -> Result<()>;
    fn unregister(&mut self, id: i32) -> Result<()>;
}

pub(crate) struct Win32HotkeyBackend {
    window: HWND,
}

impl HotkeyBackend for Win32HotkeyBackend {
    fn register(&mut self, id: i32, binding: &HotkeyBinding) -> Result<()> {
        let modifiers = modifier_flags(binding) | MOD_NOREPEAT;
        // SAFETY: window belongs to the UI thread; id is unique and key values are validated.
        let registered =
            unsafe { RegisterHotKey(self.window, id, modifiers, binding.key.win32_code()) };
        if registered == 0 {
            return Err(AppError::HotkeyRegistrationFailed {
                binding: binding.to_string(),
                win32_code: last_error(),
            });
        }
        Ok(())
    }

    fn unregister(&mut self, id: i32) -> Result<()> {
        // SAFETY: The manager only passes ids that it successfully registered for this window.
        if unsafe { UnregisterHotKey(self.window, id) } == 0 {
            return Err(AppError::Windows {
                operation: "UnregisterHotKey",
                win32_code: last_error(),
            });
        }
        Ok(())
    }
}

pub(crate) struct HotkeyManager<B: HotkeyBackend = Win32HotkeyBackend> {
    backend: B,
    by_command: HashMap<AppCommand, Registration>,
    by_id: HashMap<i32, AppCommand>,
    free_ids: Vec<i32>,
    next_id: i32,
}

impl HotkeyManager<Win32HotkeyBackend> {
    pub fn new(window: HWND) -> Self {
        Self::with_backend(Win32HotkeyBackend { window })
    }
}

impl<B: HotkeyBackend> HotkeyManager<B> {
    fn with_backend(backend: B) -> Self {
        Self {
            backend,
            by_command: HashMap::new(),
            by_id: HashMap::new(),
            free_ids: Vec::new(),
            next_id: 0x100,
        }
    }

    pub fn register_initial(&mut self, config: &HotkeyConfig) -> Vec<AppError> {
        let mut errors = Vec::new();
        if let Err(error) = config.validate() {
            errors.push(error);
            return errors;
        }
        for command in AppCommand::ALL {
            let binding = config.binding(command);
            if !binding.enabled {
                continue;
            }
            if self
                .by_command
                .get(&command)
                .is_some_and(|current| current.binding == *binding)
            {
                continue;
            }
            match self.register_new(binding) {
                Ok(registration) => self.insert_registration(command, registration),
                Err(error) => errors.push(error),
            }
        }
        errors
    }

    pub fn apply_transaction<F>(&mut self, requested: &HotkeyConfig, persist: F) -> Result<()>
    where
        F: FnOnce() -> Result<()>,
    {
        requested.validate()?;
        let mut changes = Vec::new();

        for command in AppCommand::ALL {
            let desired = requested.binding(command);
            let current = self.by_command.get(&command).cloned();
            let unchanged = match &current {
                Some(registration) => registration.binding == *desired,
                None => !desired.enabled,
            };
            if unchanged {
                continue;
            }

            let candidate = if desired.enabled {
                match self.register_new(desired) {
                    Ok(registration) => Some(registration),
                    Err(error) => {
                        let cleanup_errors = self.unregister_candidates(&changes);
                        return combine_transaction_error(error, cleanup_errors);
                    }
                }
            } else {
                None
            };
            changes.push(Change {
                command,
                old: current,
                candidate,
            });
        }

        if changes.is_empty() {
            return persist();
        }

        let mut unregistered_old = Vec::new();
        for (index, change) in changes.iter().enumerate() {
            if let Some(old) = &change.old {
                if let Err(error) = self.backend.unregister(old.id) {
                    let mut rollback_errors = self.reregister_old(&changes, &unregistered_old);
                    rollback_errors.extend(self.unregister_candidates(&changes));
                    return combine_transaction_error(error, rollback_errors);
                }
                unregistered_old.push(index);
            }
        }

        self.promote_candidates(&changes);
        if let Err(error) = persist() {
            let rollback_errors = self.rollback_promoted(&changes);
            return combine_transaction_error(error, rollback_errors);
        }

        for change in &changes {
            if let Some(old) = &change.old {
                self.release_id(old.id);
            }
        }
        Ok(())
    }

    pub fn pause(&mut self) -> Vec<AppError> {
        let commands = self.by_command.keys().copied().collect::<Vec<_>>();
        let mut errors = Vec::new();
        for command in commands {
            let Some(registration) = self.by_command.get(&command).cloned() else {
                continue;
            };
            match self.backend.unregister(registration.id) {
                Ok(()) => {
                    self.by_command.remove(&command);
                    self.by_id.remove(&registration.id);
                    self.release_id(registration.id);
                }
                Err(error) => errors.push(error),
            }
        }
        errors
    }

    pub fn command_for_id(&self, id: i32) -> Option<AppCommand> {
        self.by_id.get(&id).copied()
    }

    fn register_new(&mut self, binding: &HotkeyBinding) -> Result<Registration> {
        let id = self.allocate_id()?;
        if let Err(error) = self.backend.register(id, binding) {
            self.release_id(id);
            return Err(error);
        }
        Ok(Registration {
            id,
            binding: binding.clone(),
        })
    }

    fn allocate_id(&mut self) -> Result<i32> {
        if let Some(id) = self.free_ids.pop() {
            return Ok(id);
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or_else(|| {
            AppError::HotkeyTransactionFailed("hotkey id space exhausted".to_owned())
        })?;
        Ok(id)
    }

    fn release_id(&mut self, id: i32) {
        if !self.free_ids.contains(&id) {
            self.free_ids.push(id);
        }
    }

    fn insert_registration(&mut self, command: AppCommand, registration: Registration) {
        self.by_id.insert(registration.id, command);
        self.by_command.insert(command, registration);
    }

    fn promote_candidates(&mut self, changes: &[Change]) {
        for change in changes {
            if let Some(old) = &change.old {
                self.by_id.remove(&old.id);
                self.by_command.remove(&change.command);
            }
            if let Some(candidate) = &change.candidate {
                self.insert_registration(change.command, candidate.clone());
            }
        }
    }

    fn unregister_candidates(&mut self, changes: &[Change]) -> Vec<AppError> {
        let mut errors = Vec::new();
        for change in changes {
            let Some(candidate) = &change.candidate else {
                continue;
            };
            match self.backend.unregister(candidate.id) {
                Ok(()) => self.release_id(candidate.id),
                Err(error) => errors.push(error),
            }
        }
        errors
    }

    fn reregister_old(&mut self, changes: &[Change], indices: &[usize]) -> Vec<AppError> {
        let mut errors = Vec::new();
        for index in indices {
            let Some(old) = &changes[*index].old else {
                continue;
            };
            if let Err(error) = self.backend.register(old.id, &old.binding) {
                errors.push(error);
            }
        }
        errors
    }

    fn rollback_promoted(&mut self, changes: &[Change]) -> Vec<AppError> {
        let mut errors = Vec::new();
        let mut candidates_removed = true;

        for change in changes {
            if let Some(candidate) = &change.candidate {
                if let Err(error) = self.backend.unregister(candidate.id) {
                    errors.push(error);
                    candidates_removed = false;
                }
            }
        }
        for change in changes {
            if let Some(old) = &change.old
                && let Err(error) = self.backend.register(old.id, &old.binding)
            {
                errors.push(error);
            }
        }

        for change in changes {
            if let Some(candidate) = &change.candidate {
                self.by_id.remove(&candidate.id);
                self.by_command.remove(&change.command);
                if candidates_removed {
                    self.release_id(candidate.id);
                }
            }
            if let Some(old) = &change.old {
                self.insert_registration(change.command, old.clone());
            }
        }
        errors
    }
}

impl<B: HotkeyBackend> Drop for HotkeyManager<B> {
    fn drop(&mut self) {
        let _ = self.pause();
    }
}

fn combine_transaction_error(primary: AppError, rollback_errors: Vec<AppError>) -> Result<()> {
    if rollback_errors.is_empty() {
        return Err(primary);
    }
    let rollback = rollback_errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    Err(AppError::HotkeyTransactionFailed(format!(
        "{primary}; rollback errors: {rollback}"
    )))
}

fn modifier_flags(binding: &HotkeyBinding) -> u32 {
    binding.modifiers.iter().fold(0, |flags, modifier| {
        flags
            | match modifier {
                ModifierKey::Alt => MOD_ALT,
                ModifierKey::Control => MOD_CONTROL,
                ModifierKey::Shift => MOD_SHIFT,
                ModifierKey::Win => MOD_WIN,
            }
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use askbridge_core::VirtualKey;

    use super::*;

    #[derive(Default)]
    struct FakeBackend {
        active: HashMap<i32, HotkeyBinding>,
        fail_registration_for: Option<HotkeyBinding>,
        fail_unregistration_for: Option<i32>,
    }

    impl HotkeyBackend for FakeBackend {
        fn register(&mut self, id: i32, binding: &HotkeyBinding) -> Result<()> {
            if self
                .fail_registration_for
                .as_ref()
                .is_some_and(|failed| failed == binding)
            {
                return Err(AppError::HotkeyRegistrationFailed {
                    binding: binding.to_string(),
                    win32_code: 1409,
                });
            }
            if self.active.insert(id, binding.clone()).is_some() {
                return Err(AppError::HotkeyTransactionFailed(format!(
                    "test backend id {id} was already active"
                )));
            }
            Ok(())
        }

        fn unregister(&mut self, id: i32) -> Result<()> {
            if self.fail_unregistration_for == Some(id) {
                return Err(AppError::Windows {
                    operation: "UnregisterHotKey",
                    win32_code: 5,
                });
            }
            if self.active.remove(&id).is_none() {
                return Err(AppError::HotkeyTransactionFailed(format!(
                    "test backend id {id} was not active"
                )));
            }
            Ok(())
        }
    }

    fn manager_with_defaults() -> HotkeyManager<FakeBackend> {
        let mut manager = HotkeyManager::with_backend(FakeBackend::default());
        assert!(
            manager
                .register_initial(&HotkeyConfig::default())
                .is_empty()
        );
        manager
    }

    fn changed_capture_key(key: char) -> HotkeyConfig {
        let mut config = HotkeyConfig::default();
        config.capture_with_prompt.key = VirtualKey::Letter(key);
        config
    }

    #[test]
    fn candidate_registration_failure_keeps_old_binding_and_skips_persist() {
        let mut manager = manager_with_defaults();
        let old_id = manager
            .by_command
            .get(&AppCommand::CaptureWithPrompt)
            .expect("old registration")
            .id;
        let requested = changed_capture_key('E');
        manager.backend.fail_registration_for = Some(requested.capture_with_prompt.clone());
        let persisted = Cell::new(false);

        let result = manager.apply_transaction(&requested, || {
            persisted.set(true);
            Ok(())
        });

        assert!(result.is_err());
        assert!(!persisted.get());
        assert_eq!(
            manager.command_for_id(old_id),
            Some(AppCommand::CaptureWithPrompt)
        );
        assert_eq!(manager.backend.active.len(), 3);
    }

    #[test]
    fn candidate_id_is_promoted_without_reregistering_old_id() {
        let mut manager = manager_with_defaults();
        let old_id = manager
            .by_command
            .get(&AppCommand::CaptureWithPrompt)
            .expect("old registration")
            .id;

        manager
            .apply_transaction(&changed_capture_key('E'), || Ok(()))
            .expect("transaction succeeds");

        let new_id = manager
            .by_command
            .get(&AppCommand::CaptureWithPrompt)
            .expect("new registration")
            .id;
        assert_ne!(new_id, old_id);
        assert_eq!(manager.command_for_id(old_id), None);
        assert_eq!(
            manager.command_for_id(new_id),
            Some(AppCommand::CaptureWithPrompt)
        );
        assert!(manager.free_ids.contains(&old_id));
        assert_eq!(manager.backend.active.len(), 3);
    }

    #[test]
    fn persistence_failure_restores_old_registration_and_mapping() {
        let mut manager = manager_with_defaults();
        let old_id = manager
            .by_command
            .get(&AppCommand::CaptureWithPrompt)
            .expect("old registration")
            .id;

        let result = manager.apply_transaction(&changed_capture_key('E'), || {
            Err(AppError::ConfigurationInvalid(
                "simulated persistence failure".to_owned(),
            ))
        });

        assert!(result.is_err());
        assert_eq!(
            manager.command_for_id(old_id),
            Some(AppCommand::CaptureWithPrompt)
        );
        assert_eq!(manager.backend.active.len(), 3);
        assert_eq!(
            manager.backend.active[&old_id],
            HotkeyConfig::default().capture_with_prompt
        );
    }

    #[test]
    fn unregister_failure_removes_candidate_and_keeps_old_mapping() {
        let mut manager = manager_with_defaults();
        let old_id = manager
            .by_command
            .get(&AppCommand::CaptureWithPrompt)
            .expect("old registration")
            .id;
        manager.backend.fail_unregistration_for = Some(old_id);

        let result = manager.apply_transaction(&changed_capture_key('E'), || Ok(()));

        assert!(result.is_err());
        assert_eq!(
            manager.command_for_id(old_id),
            Some(AppCommand::CaptureWithPrompt)
        );
        assert_eq!(manager.backend.active.len(), 3);
    }

    #[test]
    fn repeated_changes_reuse_ids_without_leaking_registrations() {
        let mut manager = manager_with_defaults();

        for key in ['E', 'R', 'T', 'Y', 'U', 'I'] {
            manager
                .apply_transaction(&changed_capture_key(key), || Ok(()))
                .expect("transaction succeeds");
            assert_eq!(manager.backend.active.len(), 3);
            assert_eq!(manager.by_command.len(), 3);
            assert_eq!(manager.by_id.len(), 3);
        }

        assert_eq!(manager.next_id, 0x104);
        assert_eq!(manager.free_ids.len(), 1);
    }
}
