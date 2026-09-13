use std::{
    env,
    path::{Path, PathBuf},
};

use askbridge_core::{AppError, Result};

const DATA_DIR_ENV: &str = "ASKBRIDGE_DATA_DIR";
#[cfg(not(feature = "store"))]
const DATA_DIR_NAME: &str = "data";

pub fn resolve() -> Result<PathBuf> {
    let configured = env::var_os(DATA_DIR_ENV);
    let executable = env::current_exe()
        .map_err(|source| AppError::io("locating AskBridge executable", Path::new("."), source))?;
    resolve_from(&executable, configured.as_deref())
}

fn resolve_from(executable: &Path, configured: Option<&std::ffi::OsStr>) -> Result<PathBuf> {
    let candidate = match configured {
        Some(value) if !value.is_empty() => {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err(AppError::ConfigurationInvalid(format!(
                    "{DATA_DIR_ENV} must be an absolute path"
                )));
            }
            path
        }
        _ => default_for_executable(executable)?,
    };

    std::path::absolute(&candidate)
        .map_err(|source| AppError::io("resolving AskBridge data directory", candidate, source))
}

fn default_for_executable(executable: &Path) -> Result<PathBuf> {
    #[cfg(feature = "store")]
    {
        let _ = executable;
        store_local_state()
    }

    #[cfg(not(feature = "store"))]
    {
        let directory = executable.parent().ok_or_else(|| {
            AppError::ConfigurationInvalid(
                "AskBridge executable has no parent directory".to_owned(),
            )
        })?;

        for ancestor in directory.ancestors() {
            if ancestor.join("Cargo.toml").is_file()
                && ancestor
                    .join("crates")
                    .join("askbridge-win")
                    .join("Cargo.toml")
                    .is_file()
            {
                return Ok(ancestor.join(DATA_DIR_NAME));
            }
        }

        Ok(directory.join(DATA_DIR_NAME))
    }
}

#[cfg(feature = "store")]
fn store_local_state() -> Result<PathBuf> {
    let local_app_data = env::var_os("LOCALAPPDATA").ok_or_else(|| {
        AppError::ConfigurationInvalid(
            "LOCALAPPDATA is unavailable for the Microsoft Store package".to_owned(),
        )
    })?;
    let package_family_name = current_package_family_name()?;
    store_local_state_from(Path::new(&local_app_data), &package_family_name)
}

#[cfg(feature = "store")]
fn current_package_family_name() -> Result<String> {
    use windows_sys::Win32::{
        Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS},
        Storage::Packaging::Appx::GetCurrentPackageFamilyName,
    };

    let mut length = 0u32;
    // SAFETY: The first call intentionally supplies a null output buffer so
    // Windows returns the required UTF-16 buffer length.
    let first = unsafe { GetCurrentPackageFamilyName(&mut length, std::ptr::null_mut()) };
    if first != ERROR_INSUFFICIENT_BUFFER || length < 2 {
        return Err(AppError::ConfigurationInvalid(format!(
            "reading Microsoft Store package family length failed: Win32 error {first}"
        )));
    }

    let mut buffer = vec![0u16; length as usize];
    // SAFETY: The buffer contains length writable UTF-16 code units, exactly
    // matching the size reported by the first call.
    let second = unsafe { GetCurrentPackageFamilyName(&mut length, buffer.as_mut_ptr()) };
    if second != ERROR_SUCCESS {
        return Err(AppError::ConfigurationInvalid(format!(
            "reading Microsoft Store package family name failed: Win32 error {second}"
        )));
    }
    if buffer.last() == Some(&0) {
        buffer.pop();
    }
    String::from_utf16(&buffer).map_err(|_| {
        AppError::ConfigurationInvalid(
            "Microsoft Store package family name is not valid UTF-16".to_owned(),
        )
    })
}

#[cfg(feature = "store")]
fn store_local_state_from(local_app_data: &Path, package_family_name: &str) -> Result<PathBuf> {
    if !local_app_data.is_absolute() {
        return Err(AppError::ConfigurationInvalid(
            "LOCALAPPDATA must be absolute for the Microsoft Store package".to_owned(),
        ));
    }
    if package_family_name.is_empty()
        || !package_family_name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(AppError::ConfigurationInvalid(
            "Microsoft Store package family name contains invalid characters".to_owned(),
        ));
    }
    Ok(local_app_data
        .join("Packages")
        .join(package_family_name)
        .join("LocalState"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_absolute_directory_wins() {
        let resolved = resolve_from(
            Path::new(r"D:\AskBridge\target\debug\askbridge.exe"),
            Some(std::ffi::OsStr::new(r"D:\AskBridge\data")),
        )
        .expect("absolute data directory");
        assert_eq!(resolved, PathBuf::from(r"D:\AskBridge\data"));
    }

    #[test]
    fn relative_override_is_rejected() {
        assert!(matches!(
            resolve_from(
                Path::new(r"D:\AskBridge\target\debug\askbridge.exe"),
                Some(std::ffi::OsStr::new("data")),
            ),
            Err(AppError::ConfigurationInvalid(_))
        ));
    }

    #[cfg(not(feature = "store"))]
    #[test]
    fn portable_build_uses_directory_next_to_executable() {
        let resolved = default_for_executable(Path::new(r"D:\Apps\AskBridge\askbridge.exe"))
            .expect("portable data directory");
        assert_eq!(resolved, PathBuf::from(r"D:\Apps\AskBridge\data"));
    }

    #[cfg(feature = "store")]
    #[test]
    fn store_data_path_is_derived_without_winrt_activation() {
        let resolved = store_local_state_from(
            Path::new(r"C:\Users\StoreTest\AppData\Local"),
            "55AD4ABA.AskBridge_3kthnvq439ewe",
        )
        .expect("derive packaged LocalState path without activating WinRT");

        assert_eq!(
            resolved,
            PathBuf::from(
                r"C:\Users\StoreTest\AppData\Local\Packages\55AD4ABA.AskBridge_3kthnvq439ewe\LocalState"
            )
        );
    }
}
