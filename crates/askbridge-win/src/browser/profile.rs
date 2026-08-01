use std::{
    env,
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use askbridge_core::{AppError, Result};

const PROFILE_MARKER_NAME: &str = ".askbridge-profile";
const PROFILE_MARKER: &[u8] = b"AskBridge managed browser profile v1\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedProfile {
    path: PathBuf,
}

impl ManagedProfile {
    pub fn open(configured: &str, data_root: &Path) -> Result<Self> {
        let expanded = expand_profile_path(configured, data_root)?;
        reject_default_profile(&expanded)?;

        fs::create_dir_all(&expanded)
            .map_err(|error| AppError::io("creating browser profile", &expanded, error))?;
        let path = expanded
            .canonicalize()
            .map_err(|error| AppError::io("resolving browser profile", &expanded, error))?;
        reject_default_profile(&path)?;

        let marker = path.join(PROFILE_MARKER_NAME);
        match fs::read(&marker) {
            Ok(contents) if contents == PROFILE_MARKER => {}
            Ok(_) => {
                return Err(AppError::BrowserProfileRejected(
                    "the profile ownership marker is invalid".to_owned(),
                ));
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                ensure_directory_is_empty(&path)?;
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&marker)
                    .map_err(|source| {
                        AppError::io("creating browser profile marker", &marker, source)
                    })?;
                file.write_all(PROFILE_MARKER).map_err(|source| {
                    AppError::io("writing browser profile marker", &marker, source)
                })?;
                file.sync_all().map_err(|source| {
                    AppError::io("syncing browser profile marker", &marker, source)
                })?;
            }
            Err(source) => {
                return Err(AppError::io(
                    "reading browser profile marker",
                    &marker,
                    source,
                ));
            }
        }

        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn endpoint_file(&self) -> PathBuf {
        self.path.join("DevToolsActivePort")
    }
}

fn expand_profile_path(configured: &str, data_root: &Path) -> Result<PathBuf> {
    let configured = configured.trim();
    if configured.is_empty() {
        return Err(AppError::BrowserProfileRejected(
            "the configured path is empty".to_owned(),
        ));
    }

    let placeholder = "%ASKBRIDGE_DATA_DIR%";
    let expanded = if configured
        .get(..placeholder.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(placeholder))
    {
        let suffix = configured[placeholder.len()..].trim_start_matches(['\\', '/']);
        data_root.join(suffix)
    } else {
        if configured.contains('%') {
            return Err(AppError::BrowserProfileRejected(
                "only %ASKBRIDGE_DATA_DIR% expansion is supported".to_owned(),
            ));
        }
        let path = PathBuf::from(configured);
        if path.is_absolute() {
            path
        } else {
            data_root.join(path)
        }
    };

    std::path::absolute(&expanded)
        .map_err(|source| AppError::io("resolving browser profile path", expanded, source))
}

fn reject_default_profile(path: &Path) -> Result<()> {
    let Some(local_app_data) = env::var_os("LOCALAPPDATA") else {
        return Ok(());
    };
    let local_app_data = PathBuf::from(local_app_data);
    let defaults = [
        local_app_data.join(r"Google\Chrome\User Data"),
        local_app_data.join(r"Google\Chrome Beta\User Data"),
        local_app_data.join(r"Google\Chrome SxS\User Data"),
    ];
    let candidate = comparable_path(path);

    if defaults
        .iter()
        .any(|default| candidate == comparable_path(default))
    {
        return Err(AppError::BrowserProfileRejected(
            "the default Chrome user data directory cannot be used".to_owned(),
        ));
    }
    Ok(())
}

fn comparable_path(path: &Path) -> String {
    path.to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .replace('/', "\\")
        .to_lowercase()
}

fn ensure_directory_is_empty(path: &Path) -> Result<()> {
    let mut entries = fs::read_dir(path)
        .map_err(|source| AppError::io("checking browser profile", path, source))?;
    if entries
        .next()
        .transpose()
        .map_err(|source| AppError::io("checking browser profile contents", path, source))?
        .is_some()
    {
        return Err(AppError::BrowserProfileRejected(
            "an existing unmarked directory cannot be adopted".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "askbridge-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn creates_marker_and_reopens_managed_profile() {
        let path = unique_temp_dir("profile");
        let text = path.to_string_lossy().into_owned();

        let profile = ManagedProfile::open(&text, &path).expect("create profile");
        assert_eq!(profile.path(), path.canonicalize().expect("canonical"));
        assert_eq!(
            fs::read(path.join(PROFILE_MARKER_NAME)).expect("marker"),
            PROFILE_MARKER
        );
        ManagedProfile::open(&text, &path).expect("reopen profile");

        fs::remove_dir_all(path).expect("cleanup");
    }

    #[test]
    fn refuses_to_adopt_nonempty_unmarked_directory() {
        let path = unique_temp_dir("unmarked");
        fs::create_dir_all(&path).expect("directory");
        fs::write(path.join("Preferences"), b"existing").expect("fixture");

        assert!(matches!(
            ManagedProfile::open(&path.to_string_lossy(), &path),
            Err(AppError::BrowserProfileRejected(_))
        ));

        fs::remove_dir_all(path).expect("cleanup");
    }

    #[test]
    fn rejects_unknown_environment_expansion() {
        assert!(matches!(
            ManagedProfile::open(r"%USERPROFILE%\Chrome", Path::new(r"D:\AskBridge\data")),
            Err(AppError::BrowserProfileRejected(_))
        ));
    }

    #[test]
    fn resolves_relative_profile_beneath_data_root() {
        let root = unique_temp_dir("relative-root");
        fs::create_dir_all(&root).expect("data root");

        let profile = ManagedProfile::open("BrowserProfile", &root).expect("relative profile");
        assert_eq!(
            profile.path(),
            root.join("BrowserProfile")
                .canonicalize()
                .expect("canonical")
        );

        fs::remove_dir_all(root).expect("cleanup");
    }
}
