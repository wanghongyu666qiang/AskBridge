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

pub struct HotkeyManager {
    window: HWND,
    by_command: HashMap<AppCommand, Registration>,
    by_id: HashMap<i32, AppCommand>,
    next_id: i32,
}

impl HotkeyManager {
    pub fn new(window: HWND) -> Self {
        Self {
            window,
            by_command: HashMap::new(),
            by_id: HashMap::new(),
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
        let mut staged = Vec::new();

        for command in AppCommand::ALL {
            let desired = requested.binding(command);
            let unchanged = self
                .by_command
                .get(&command)
                .is_some_and(|current| current.binding == *desired);
            if desired.enabled && !unchanged {
                match self.register_new(desired) {
                    Ok(registration) => staged.push((command, registration)),
                    Err(error) => {
                        self.unregister_staged(&staged);
                        return Err(error);
                    }
                }
            }
        }

        if let Err(error) = persist() {
            self.unregister_staged(&staged);
            return Err(error);
        }

        for command in AppCommand::ALL {
            let desired = requested.binding(command);
            let unchanged = self
                .by_command
                .get(&command)
                .is_some_and(|current| current.binding == *desired);
            if !unchanged {
                self.remove_registration(command);
            }
        }
        for (command, registration) in staged {
            self.insert_registration(command, registration);
        }
        Ok(())
    }

    pub fn pause(&mut self) {
        for command in AppCommand::ALL {
            self.remove_registration(command);
        }
    }

    pub fn command_for_id(&self, id: i32) -> Option<AppCommand> {
        self.by_id.get(&id).copied()
    }

    fn register_new(&mut self, binding: &HotkeyBinding) -> Result<Registration> {
        let id = self.allocate_id();
        let modifiers = modifier_flags(binding) | MOD_NOREPEAT;
        // SAFETY: window belongs to this UI thread; id is unique and the key values are validated.
        let registered =
            unsafe { RegisterHotKey(self.window, id, modifiers, binding.key.win32_code()) };
        if registered == 0 {
            return Err(AppError::HotkeyRegistrationFailed {
                binding: binding.to_string(),
                win32_code: last_error(),
            });
        }
        Ok(Registration {
            id,
            binding: binding.clone(),
        })
    }

    fn allocate_id(&mut self) -> i32 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    fn insert_registration(&mut self, command: AppCommand, registration: Registration) {
        self.by_id.insert(registration.id, command);
        self.by_command.insert(command, registration);
    }

    fn remove_registration(&mut self, command: AppCommand) {
        if let Some(registration) = self.by_command.remove(&command) {
            // SAFETY: This id was successfully registered by this manager for this window.
            unsafe {
                UnregisterHotKey(self.window, registration.id);
            }
            self.by_id.remove(&registration.id);
        }
    }

    fn unregister_staged(&self, staged: &[(AppCommand, Registration)]) {
        for (_, registration) in staged {
            // SAFETY: Staged registrations were successfully registered and are not yet committed.
            unsafe {
                UnregisterHotKey(self.window, registration.id);
            }
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.pause();
    }
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
