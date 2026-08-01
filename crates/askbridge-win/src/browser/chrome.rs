use std::{
    env,
    ffi::OsStr,
    fs,
    io::ErrorKind,
    os::windows::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    ptr,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use askbridge_core::{AppError, Result};
use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY,
        RRF_SUBKEY_WOW6464KEY, RegGetValueW,
    },
};

use super::{DevToolsEndpoint, ManagedProfile};

const APP_PATHS_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe";
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeSource {
    Configured,
    Registry,
    CommonLocation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChromeInstallation {
    path: PathBuf,
    source: ChromeSource,
}

impl ChromeInstallation {
    pub fn discover(configured: Option<&str>) -> Result<Self> {
        if let Some(configured) = configured.filter(|path| !path.trim().is_empty()) {
            let path = PathBuf::from(configured.trim());
            if is_chrome_executable(&path) {
                return Ok(Self {
                    path,
                    source: ChromeSource::Configured,
                });
            }
            return Err(AppError::ChromeNotFound);
        }

        for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for view in [RRF_SUBKEY_WOW6464KEY, RRF_SUBKEY_WOW6432KEY] {
                if let Some(path) = read_app_path(root, view)
                    && is_chrome_executable(&path)
                {
                    return Ok(Self {
                        path,
                        source: ChromeSource::Registry,
                    });
                }
            }
        }

        for path in common_locations() {
            if is_chrome_executable(&path) {
                return Ok(Self {
                    path,
                    source: ChromeSource::CommonLocation,
                });
            }
        }

        Err(AppError::ChromeNotFound)
    }

    #[cfg(test)]
    fn configured(path: PathBuf) -> Self {
        Self {
            path,
            source: ChromeSource::Configured,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(test)]
    pub const fn source(&self) -> ChromeSource {
        self.source
    }
}

pub struct ChromeManager {
    installation: ChromeInstallation,
    profile: ManagedProfile,
    child: Option<Child>,
}

impl ChromeManager {
    pub fn new(installation: ChromeInstallation, profile: ManagedProfile) -> Self {
        Self {
            installation,
            profile,
            child: None,
        }
    }

    pub fn launch_and_wait(
        &mut self,
        timeout: Duration,
        cancelled: &AtomicBool,
    ) -> Result<DevToolsEndpoint> {
        self.clear_finished_child()?;
        if self.child.is_none() {
            clear_stale_endpoint(&self.profile)?;
            let args = launch_args(self.profile.path());
            let child = Command::new(self.installation.path())
                .args(args)
                .creation_flags(CREATE_NEW_PROCESS_GROUP)
                .spawn()
                .map_err(|_| AppError::BrowserLaunchFailed)?;
            self.child = Some(child);
        }
        self.wait_for_endpoint(timeout, cancelled)
    }

    pub fn managed_process_id(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    pub fn wait_for_managed_exit(&mut self, timeout: Duration) -> Result<bool> {
        if self.child.is_none() {
            return Ok(true);
        }
        let deadline = Instant::now() + timeout;
        loop {
            if self.child_status()?.is_some() {
                self.child = None;
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn wait_for_endpoint(
        &mut self,
        timeout: Duration,
        cancelled: &AtomicBool,
    ) -> Result<DevToolsEndpoint> {
        let deadline = Instant::now() + timeout;
        let endpoint_path = self.profile.endpoint_file();
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(AppError::BrowserCancelled);
            }
            if let Ok(endpoint) = DevToolsEndpoint::read(&endpoint_path) {
                return Ok(endpoint);
            }
            if let Some(status) = self.child_status()? {
                self.child = None;
                if !status.success() {
                    return Err(AppError::BrowserLaunchFailed);
                }
                // Chrome can forward this invocation to an already-running
                // browser that owns the same dedicated profile. In that case
                // the launcher exits successfully while the profile endpoint
                // remains authoritative.
            }
            if Instant::now() >= deadline {
                return Err(AppError::BrowserEndpointUnavailable);
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    fn clear_finished_child(&mut self) -> Result<()> {
        if self.child_status()?.is_some() {
            self.child = None;
        }
        Ok(())
    }

    fn child_status(&mut self) -> Result<Option<ExitStatus>> {
        self.child
            .as_mut()
            .map(Child::try_wait)
            .transpose()
            .map_err(|_| AppError::BrowserLaunchFailed)
            .map(Option::flatten)
    }
}

fn clear_stale_endpoint(profile: &ManagedProfile) -> Result<()> {
    let endpoint = profile.endpoint_file();
    match fs::remove_file(&endpoint) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(source) => Err(AppError::io(
            "clearing stale browser debugging endpoint",
            endpoint,
            source,
        )),
    }
}

fn launch_args(profile: &Path) -> Vec<std::ffi::OsString> {
    vec![
        format!("--user-data-dir={}", profile.display()).into(),
        "--remote-debugging-port=0".into(),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
    ]
}

fn is_chrome_executable(path: &Path) -> bool {
    path.is_file()
        && path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.eq_ignore_ascii_case("chrome.exe"))
}

fn common_locations() -> Vec<PathBuf> {
    let mut locations = Vec::new();
    for variable in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Some(base) = env::var_os(variable) {
            locations.push(PathBuf::from(base).join(r"Google\Chrome\Application\chrome.exe"));
        }
    }
    locations
}

fn read_app_path(root: HKEY, view: u32) -> Option<PathBuf> {
    let subkey = wide(APP_PATHS_KEY);
    let flags = RRF_RT_REG_SZ | view;
    let mut byte_len = 0u32;
    // SAFETY: `subkey` is NUL-terminated and lives for the call. The first
    // query passes no output buffer and asks Windows for the required size.
    let status = unsafe {
        RegGetValueW(
            root,
            subkey.as_ptr(),
            ptr::null(),
            flags,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut byte_len,
        )
    };
    if status != ERROR_SUCCESS || byte_len < 2 || byte_len % 2 != 0 {
        return None;
    }

    let mut buffer = vec![0u16; byte_len as usize / 2];
    // SAFETY: `buffer` has exactly the byte capacity reported by Windows and
    // all pointers remain valid for the duration of the call.
    let status = unsafe {
        RegGetValueW(
            root,
            subkey.as_ptr(),
            ptr::null(),
            flags,
            ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut byte_len,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }

    let length = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    let value = String::from_utf16(&buffer[..length]).ok()?;
    Some(PathBuf::from(value.trim_matches('"')))
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_chrome_must_exist_and_be_named_chrome() {
        let temp = env::temp_dir().join(format!("askbridge-chrome-{}", std::process::id()));
        std::fs::create_dir_all(&temp).expect("temp");
        let chrome = temp.join("chrome.exe");
        std::fs::write(&chrome, b"fixture").expect("fixture");

        let installation =
            ChromeInstallation::discover(Some(&chrome.to_string_lossy())).expect("discover");
        assert_eq!(installation.path(), chrome);
        assert_eq!(installation.source(), ChromeSource::Configured);
        assert!(matches!(
            ChromeInstallation::discover(Some(&temp.join("other.exe").to_string_lossy())),
            Err(AppError::ChromeNotFound)
        ));

        std::fs::remove_dir_all(temp).expect("cleanup");
    }

    #[test]
    fn launch_arguments_are_centralized_and_use_dynamic_port() {
        let profile = Path::new(r"C:\Users\Test\AppData\Local\AskBridge\BrowserProfile");
        let args = launch_args(profile);
        let args: Vec<String> = args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect();

        assert_eq!(
            args,
            vec![
                r"--user-data-dir=C:\Users\Test\AppData\Local\AskBridge\BrowserProfile",
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
            ]
        );
    }

    #[test]
    fn test_constructor_does_not_change_discovery_source() {
        let installation = ChromeInstallation::configured(PathBuf::from("chrome.exe"));
        assert_eq!(installation.source(), ChromeSource::Configured);
    }

    #[test]
    fn stale_endpoint_is_removed_before_a_new_managed_process() {
        let path = env::temp_dir().join(format!("askbridge-stale-endpoint-{}", std::process::id()));
        let profile = ManagedProfile::open(&path.to_string_lossy(), &path).expect("profile");
        fs::write(profile.endpoint_file(), b"9222\n/devtools/browser/stale\n")
            .expect("stale endpoint");

        clear_stale_endpoint(&profile).expect("clear endpoint");
        assert!(!profile.endpoint_file().exists());

        fs::remove_dir_all(path).expect("cleanup");
    }
}
