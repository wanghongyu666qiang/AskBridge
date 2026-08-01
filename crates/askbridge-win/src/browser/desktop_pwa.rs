use std::{
    env,
    path::{Path, PathBuf},
    ptr,
};

use askbridge_core::{AppError, Result};
use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

use crate::util::wide;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopPwaTarget {
    provider_id: String,
}

impl DesktopPwaTarget {
    pub fn id(&self) -> String {
        format!("desktop-pwa:{}", self.provider_id)
    }
}

pub struct DesktopPwaLauncher;

impl DesktopPwaLauncher {
    pub fn open(provider_id: &str, configured_shortcut: Option<&str>) -> Result<DesktopPwaTarget> {
        let shortcut = discover(provider_id, configured_shortcut)?;
        launch_shortcut(&shortcut)?;
        Ok(DesktopPwaTarget {
            provider_id: provider_id.to_owned(),
        })
    }
}

fn discover(provider_id: &str, configured_shortcut: Option<&str>) -> Result<PathBuf> {
    let desktop_directories = desktop_directories();
    discover_in(provider_id, configured_shortcut, &desktop_directories)
}

fn discover_in(
    provider_id: &str,
    configured_shortcut: Option<&str>,
    desktop_directories: &[PathBuf],
) -> Result<PathBuf> {
    if let Some(configured) = configured_shortcut {
        let path = PathBuf::from(configured.trim());
        validate_shortcut(&path)?;
        return Ok(path);
    }

    let shortcut_name = match provider_id {
        "chatgpt" => "ChatGPT.lnk",
        _ => return Err(AppError::DesktopShortcutNotFound(provider_id.to_owned())),
    };
    for desktop in desktop_directories {
        let candidate = desktop.join(shortcut_name);
        if candidate.is_file() {
            validate_shortcut(&candidate)?;
            return Ok(candidate);
        }
    }
    Err(AppError::DesktopShortcutNotFound(provider_id.to_owned()))
}

fn desktop_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(user_profile) = env::var_os("USERPROFILE") {
        directories.push(PathBuf::from(user_profile).join("Desktop"));
    }
    if let Some(one_drive) = env::var_os("OneDrive") {
        let one_drive_desktop = PathBuf::from(one_drive).join("Desktop");
        if !directories.contains(&one_drive_desktop) {
            directories.push(one_drive_desktop);
        }
    }
    directories
}

fn validate_shortcut(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(AppError::DesktopShortcutRejected(
            "the path must be absolute".to_owned(),
        ));
    }
    if !path.is_file() {
        return Err(AppError::DesktopShortcutNotFound(
            path.file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("unknown")
                .to_owned(),
        ));
    }
    let is_link = path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("lnk"));
    if !is_link {
        return Err(AppError::DesktopShortcutRejected(
            "only Windows .lnk files are accepted".to_owned(),
        ));
    }
    Ok(())
}

fn launch_shortcut(path: &Path) -> Result<()> {
    let operation = wide("open");
    let file = wide(&path.to_string_lossy());
    // SAFETY: The strings are live, NUL-terminated UTF-16 buffers for the duration of the call.
    // No parameters or working directory are supplied; the user-selected shortcut owns them.
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        return Err(AppError::DesktopLaunchFailed(result));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_dir(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "askbridge-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn configured_shortcut_wins_over_discovery() {
        let root = unique_temp_dir("configured-pwa");
        fs::create_dir_all(&root).expect("root");
        let configured = root.join("Configured ChatGPT.lnk");
        fs::write(&configured, b"shortcut fixture").expect("shortcut");

        let discovered = discover_in("chatgpt", Some(&configured.to_string_lossy()), &[])
            .expect("configured shortcut");
        assert_eq!(discovered, configured);

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn discovers_chatgpt_on_desktop() {
        let root = unique_temp_dir("desktop-pwa");
        fs::create_dir_all(&root).expect("root");
        let shortcut = root.join("ChatGPT.lnk");
        fs::write(&shortcut, b"shortcut fixture").expect("shortcut");

        assert_eq!(
            discover_in("chatgpt", None, std::slice::from_ref(&root)).expect("discovery"),
            shortcut
        );

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rejects_non_link_targets() {
        let root = unique_temp_dir("unsafe-pwa");
        fs::create_dir_all(&root).expect("root");
        let executable = root.join("chatgpt.exe");
        fs::write(&executable, b"fixture").expect("fixture");

        assert!(matches!(
            discover_in("chatgpt", Some(&executable.to_string_lossy()), &[]),
            Err(AppError::DesktopShortcutRejected(_))
        ));

        fs::remove_dir_all(root).expect("cleanup");
    }
}
