use std::{
    env,
    path::{Path, PathBuf},
};

use askbridge_core::{AppError, Result};

const DATA_DIR_ENV: &str = "ASKBRIDGE_DATA_DIR";
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
    let directory = executable.parent().ok_or_else(|| {
        AppError::ConfigurationInvalid("AskBridge executable has no parent directory".to_owned())
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

    #[test]
    fn portable_build_uses_directory_next_to_executable() {
        let resolved = default_for_executable(Path::new(r"D:\Apps\AskBridge\askbridge.exe"))
            .expect("portable data directory");
        assert_eq!(resolved, PathBuf::from(r"D:\Apps\AskBridge\data"));
    }
}
