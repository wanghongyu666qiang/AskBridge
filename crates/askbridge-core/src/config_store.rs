use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{AppConfig, AppError, Result};

#[derive(Debug)]
pub struct ConfigStore {
    path: PathBuf,
}

#[derive(Debug)]
pub struct ConfigLoad {
    pub config: AppConfig,
    pub recovered_from: Option<PathBuf>,
    pub created_default: bool,
    pub migrated: bool,
    /// True when the file was written by a newer application version. The
    /// defaults above only apply to this session; the original file stays on
    /// disk untouched so a downgrade or re-upgrade loses nothing.
    pub kept_unsupported_schema: bool,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_or_create(&self) -> Result<ConfigLoad> {
        if !self.path.exists() {
            let config = AppConfig::default();
            self.save(&config)?;
            return Ok(ConfigLoad {
                config,
                recovered_from: None,
                created_default: true,
                migrated: false,
                kept_unsupported_schema: false,
            });
        }

        match self.load_with_migration() {
            Ok((config, migrated)) => {
                if migrated {
                    self.save(&config)?;
                }
                Ok(ConfigLoad {
                    config,
                    recovered_from: None,
                    created_default: false,
                    migrated,
                    kept_unsupported_schema: false,
                })
            }
            // A file from a newer app version is not corruption. Overwriting it
            // with defaults would silently wipe the user's configuration the
            // moment they roll back an update, so keep it on disk and run this
            // session with defaults instead.
            Err(AppError::UnsupportedConfigurationSchema { .. }) => Ok(ConfigLoad {
                config: AppConfig::default(),
                recovered_from: None,
                created_default: false,
                migrated: false,
                kept_unsupported_schema: true,
            }),
            // I/O failures are environmental, not recoverable by resetting.
            Err(error @ AppError::Io { .. }) => Err(error),
            // Anything else means the content itself is unusable; back it up
            // and start over from defaults rather than failing startup.
            Err(_) => {
                let backup_path = self.backup_corrupt_config()?;
                let config = AppConfig::default();
                self.save(&config)?;
                Ok(ConfigLoad {
                    config,
                    recovered_from: Some(backup_path),
                    created_default: true,
                    migrated: false,
                    kept_unsupported_schema: false,
                })
            }
        }
    }

    pub fn load(&self) -> Result<AppConfig> {
        self.load_with_migration().map(|(config, _)| config)
    }

    fn load_with_migration(&self) -> Result<(AppConfig, bool)> {
        let bytes = fs::read(&self.path)
            .map_err(|source| AppError::io("reading configuration", &self.path, source))?;
        let mut config = serde_json::from_slice::<AppConfig>(&bytes).map_err(|source| {
            AppError::ConfigurationParse {
                path: self.path.clone(),
                source,
            }
        })?;
        let migrated = config.migrate()?;
        Ok((config, migrated))
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        config.validate()?;
        let parent = self.path.parent().ok_or_else(|| {
            AppError::ConfigurationInvalid("configuration path has no parent".to_owned())
        })?;
        fs::create_dir_all(parent)
            .map_err(|source| AppError::io("creating configuration directory", parent, source))?;
        let bytes =
            serde_json::to_vec_pretty(config).map_err(|source| AppError::ConfigurationParse {
                path: self.path.clone(),
                source,
            })?;
        let temporary_path = self.path.with_extension("json.tmp");
        let mut temporary_file = fs::File::create(&temporary_path).map_err(|source| {
            AppError::io("creating temporary configuration", &temporary_path, source)
        })?;
        temporary_file.write_all(&bytes).map_err(|source| {
            AppError::io("writing temporary configuration", &temporary_path, source)
        })?;
        temporary_file.sync_all().map_err(|source| {
            AppError::io("syncing temporary configuration", &temporary_path, source)
        })?;
        drop(temporary_file);
        replace_file(&temporary_path, &self.path)
    }

    fn backup_corrupt_config(&self) -> Result<PathBuf> {
        let mut suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        // Advance past any suffix that lost the race, so two corruptions in the
        // same millisecond cannot make the rename fail and hard-error startup.
        loop {
            let backup_path = self.path.with_extension(format!("corrupt-{suffix}.json"));
            if !backup_path.exists() {
                fs::rename(&self.path, &backup_path).map_err(|source| {
                    AppError::io("backing up corrupt configuration", &backup_path, source)
                })?;
                return Ok(backup_path);
            }
            suffix += 1;
        }
    }
}

#[cfg(windows)]
fn replace_file(temporary_path: &Path, destination: &Path) -> Result<()> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};

    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_WRITE_THROUGH, ReplaceFileW};

    if !destination.exists() {
        return fs::rename(temporary_path, destination)
            .map_err(|source| AppError::io("installing configuration", destination, source));
    }

    fn wide(path: &Path) -> Vec<u16> {
        OsStr::new(path).encode_wide().chain(Some(0)).collect()
    }

    let destination_wide = wide(destination);
    let temporary_wide = wide(temporary_path);
    // SAFETY: Both path buffers are valid NUL-terminated UTF-16 strings for the
    // duration of the call. No backup is requested and reserved pointers are null.
    let replaced = unsafe {
        ReplaceFileW(
            destination_wide.as_ptr(),
            temporary_wide.as_ptr(),
            ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            ptr::null(),
            ptr::null(),
        )
    };
    if replaced == 0 {
        return Err(AppError::io(
            "atomically replacing configuration",
            destination,
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(temporary_path: &Path, destination: &Path) -> Result<()> {
    fs::rename(temporary_path, destination)
        .map_err(|source| AppError::io("atomically replacing configuration", destination, source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_configuration() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = ConfigStore::new(directory.path().join("config.json"));
        let mut expected = AppConfig::default();
        expected.general.debug_logging = true;
        store.save(&expected).expect("save config");
        assert_eq!(store.load().expect("load config"), expected);
        assert!(!directory.path().join("config.json.previous").exists());
    }

    #[test]
    fn creates_default_when_file_is_missing() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = ConfigStore::new(directory.path().join("config.json"));
        let loaded = store.load_or_create().expect("create default");
        assert!(loaded.created_default);
        assert!(!loaded.migrated);
        assert!(store.path().exists());
        assert_eq!(loaded.config, AppConfig::default());
    }

    #[test]
    fn backs_up_corrupt_configuration() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("config.json");
        fs::write(&path, b"{ definitely not json").expect("write corrupt config");
        let store = ConfigStore::new(&path);
        let loaded = store.load_or_create().expect("recover config");
        let backup = loaded.recovered_from.expect("corrupt backup path");
        assert!(backup.exists());
        assert_eq!(
            fs::read(backup).expect("read backup"),
            b"{ definitely not json"
        );
        assert_eq!(
            store.load().expect("load recovered config"),
            AppConfig::default()
        );
    }

    #[test]
    fn migrates_and_persists_older_configuration() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("config.json");
        fs::write(
            &path,
            br#"{
                "schema_version": 1,
                "hotkeys": {},
                "general": { "auto_submit": true }
            }"#,
        )
        .expect("write old config");
        let store = ConfigStore::new(&path);

        let loaded = store.load_or_create().expect("migrate config");

        assert!(loaded.migrated);
        assert!(!loaded.config.general.auto_submit);
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(path).expect("read migrated config"))
                .expect("parse migrated config");
        assert_eq!(
            persisted["schema_version"],
            crate::config::CURRENT_SCHEMA_VERSION
        );
        assert_eq!(persisted["general"]["auto_submit"], false);
    }

    #[test]
    fn keeps_newer_schema_file_instead_of_resetting_it() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("config.json");
        let future_config = format!(
            r#"{{ "schema_version": {} }}"#,
            crate::config::CURRENT_SCHEMA_VERSION + 1
        );
        fs::write(&path, &future_config).expect("write future config");
        let store = ConfigStore::new(&path);

        let loaded = store.load_or_create().expect("load with session defaults");

        assert!(loaded.kept_unsupported_schema);
        assert!(!loaded.created_default);
        assert_eq!(loaded.config, AppConfig::default());
        // The file written by the newer version is still on disk untouched.
        assert_eq!(
            fs::read_to_string(&path).expect("read kept file"),
            future_config
        );
    }

    #[test]
    fn recovers_with_defaults_from_parseable_but_invalid_content() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("config.json");
        fs::write(&path, br#"{ "schema_version": 3, "quick_prompt": "   " }"#)
            .expect("write invalid config");
        let store = ConfigStore::new(&path);

        let loaded = store.load_or_create().expect("recover config");

        assert!(!loaded.kept_unsupported_schema);
        assert!(loaded.recovered_from.is_some());
        assert_eq!(
            store.load().expect("load recovered config"),
            AppConfig::default()
        );
    }
}
