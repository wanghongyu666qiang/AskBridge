use std::{mem::size_of, path::PathBuf, ptr};

use askbridge_core::{AppError, Result};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
        RegSetValueExW,
    },
};

use crate::util::wide;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "AskBridge";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupSnapshot(Option<String>);

pub fn snapshot() -> Result<StartupSnapshot> {
    read_command().map(StartupSnapshot)
}

pub fn apply(enabled: bool) -> Result<()> {
    if enabled {
        let command = current_executable_command()?;
        write_command(Some(&command))
    } else {
        write_command(None)
    }
}

pub fn restore(snapshot: &StartupSnapshot) -> Result<()> {
    write_command(snapshot.0.as_deref())
}

pub fn is_current_executable_registered() -> Result<bool> {
    let expected = current_executable_command()?;
    Ok(startup_command_matches(
        &expected,
        read_command()?.as_deref(),
    ))
}

fn startup_command_matches(expected: &str, registered: Option<&str>) -> bool {
    registered.is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn current_executable_command() -> Result<String> {
    let executable = std::env::current_exe().map_err(|source| {
        AppError::io(
            "locating executable for startup registration",
            PathBuf::from("askbridge.exe"),
            source,
        )
    })?;
    let text = executable.to_string_lossy();
    if text.contains('"') {
        return Err(AppError::ConfigurationInvalid(
            "AskBridge executable path contains an unsupported quote".to_owned(),
        ));
    }
    Ok(format!("\"{text}\""))
}

fn read_command() -> Result<Option<String>> {
    let Some(key) = open_run_key(KEY_QUERY_VALUE)? else {
        return Ok(None);
    };
    read_command_value(key.0)
}

fn open_run_key(access: u32) -> Result<Option<OwnedRegistryKey>> {
    let run_key = wide(RUN_KEY);
    let mut raw_key: HKEY = ptr::null_mut();
    // SAFETY: The key path is a valid nul-terminated UTF-16 string and raw_key is writable.
    let status =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, run_key.as_ptr(), 0, access, &mut raw_key) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check_status("opening startup registry key", status)?;
    Ok(Some(OwnedRegistryKey(raw_key)))
}

fn read_command_value(key: HKEY) -> Result<Option<String>> {
    let value_name = wide(VALUE_NAME);
    let mut value_type = 0;
    let mut bytes = 0;
    // SAFETY: Query uses a live key and writable size/type outputs; no data is requested yet.
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_name.as_ptr(),
            ptr::null_mut(),
            &mut value_type,
            ptr::null_mut(),
            &mut bytes,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check_status("querying startup registry value", status)?;
    if value_type != REG_SZ || bytes == 0 || !(bytes as usize).is_multiple_of(size_of::<u16>()) {
        return Err(AppError::ConfigurationInvalid(
            "AskBridge startup registry value is not a valid string".to_owned(),
        ));
    }
    let mut buffer = vec![0u16; bytes as usize / size_of::<u16>()];
    // SAFETY: buffer has exactly the byte capacity reported by the first query.
    let status = unsafe {
        RegQueryValueExW(
            key,
            value_name.as_ptr(),
            ptr::null_mut(),
            &mut value_type,
            buffer.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    check_status("reading startup registry value", status)?;
    while buffer.last() == Some(&0) {
        buffer.pop();
    }
    Ok(Some(String::from_utf16_lossy(&buffer)))
}

fn write_command(command: Option<&str>) -> Result<()> {
    let Some(command) = command else {
        return remove_current_command();
    };
    let run_key = wide(RUN_KEY);
    let mut raw_key: HKEY = ptr::null_mut();
    let mut disposition = 0;
    // SAFETY: The key path is valid and all output pointers are writable for this call.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            run_key.as_ptr(),
            0,
            ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_QUERY_VALUE | KEY_SET_VALUE,
            ptr::null(),
            &mut raw_key,
            &mut disposition,
        )
    };
    check_status("opening startup registry key for update", status)?;
    let key = OwnedRegistryKey(raw_key);
    let value_name = wide(VALUE_NAME);
    let encoded = wide(command);
    let byte_len = u32::try_from(encoded.len() * size_of::<u16>())
        .map_err(|_| AppError::ConfigurationInvalid("startup command is too long".to_owned()))?;
    // SAFETY: encoded is a nul-terminated UTF-16 string and byte_len covers it.
    let status = unsafe {
        RegSetValueExW(
            key.0,
            value_name.as_ptr(),
            0,
            REG_SZ,
            encoded.as_ptr().cast(),
            byte_len,
        )
    };
    check_status("writing startup registry value", status)
}

fn remove_current_command() -> Result<()> {
    let expected = current_executable_command()?;
    let Some(key) = open_run_key(KEY_QUERY_VALUE | KEY_SET_VALUE)? else {
        return Ok(());
    };
    if !startup_command_matches(&expected, read_command_value(key.0)?.as_deref()) {
        return Ok(());
    }
    let value_name = wide(VALUE_NAME);
    // SAFETY: key and value name are valid for this call.
    let status = unsafe { RegDeleteValueW(key.0, value_name.as_ptr()) };
    if status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        check_status("removing startup registry value", status)
    }
}

fn check_status(operation: &'static str, status: u32) -> Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(AppError::Windows {
            operation,
            win32_code: status,
        })
    }
}

struct OwnedRegistryKey(HKEY);

impl Drop for OwnedRegistryKey {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: The key is owned by this guard and is closed exactly once.
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_command_is_quoted_and_targets_current_executable() {
        let command = current_executable_command().expect("startup command");
        assert!(command.starts_with('"'));
        assert!(command.ends_with('"'));
        assert!(command.to_ascii_lowercase().contains("askbridge"));
    }

    #[test]
    fn startup_disable_requires_exact_current_command() {
        let expected = r#""C:\Program Files\AskBridge\askbridge.exe""#;
        assert!(startup_command_matches(expected, Some(expected)));
        assert!(startup_command_matches(
            expected,
            Some(r#""c:\program files\askbridge\askbridge.exe""#)
        ));
        assert!(!startup_command_matches(
            expected,
            Some(r#""C:\Program Files\Other\askbridge.exe""#)
        ));
        assert!(!startup_command_matches(
            expected,
            Some(r#""C:\Program Files\AskBridge\askbridge.exe" --argument"#)
        ));
        assert!(!startup_command_matches(expected, None));
    }
}
