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
            });
        }

        match self.load() {
            Ok(config) => Ok(ConfigLoad {
                config,
                recovered_from: None,
                created_default: false,
            }),
            Err(AppError::ConfigurationParse { .. })
            | Err(AppError::ConfigurationInvalid(_))
            | Err(AppError::UnsupportedConfigurationSchema { .. })
            | Err(AppError::InvalidHotkey(_))
            | Err(AppError::HotkeyConflict(_))
            | Err(AppError::InvalidProvider(_))
            | Err(AppError::InvalidProviderUrl(_)) => {
                let backup_path = self.backup_corrupt_config()?;
                let config = AppConfig::default();
                self.save(&config)?;
                Ok(ConfigLoad {
                    config,
                    recovered_from: Some(backup_path),
                    created_default: true,
                })
            }
            Err(error) => Err(error),
        }
    }

    pub fn load(&self) -> Result<AppConfig> {
        let bytes = fs::read(&self.path)
            .map_err(|source| AppError::io("reading configuration", &self.path, source))?;
        let config = serde_json::from_slice::<AppConfig>(&bytes).map_err(|source| {
            AppError::ConfigurationParse {
                path: self.path.clone(),
                source,
            }
        })?;
        config.validate()?;
        Ok(config)
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
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let backup_path = self.path.with_extension(format!("corrupt-{suffix}.json"));
        fs::rename(&self.path, &backup_path).map_err(|source| {
            AppError::io("backing up corrupt configuration", &backup_path, source)
        })?;
        Ok(backup_path)
    }
}

fn replace_file(temporary_path: &Path, destination: &Path) -> Result<()> {
    if !destination.exists() {
        return fs::rename(temporary_path, destination)
            .map_err(|source| AppError::io("installing configuration", destination, source));
    }

    let previous_path = destination.with_extension("json.previous");
    if previous_path.exists() {
        fs::remove_file(&previous_path).map_err(|source| {
            AppError::io(
                "removing previous configuration backup",
                &previous_path,
                source,
            )
        })?;
    }
    fs::rename(destination, &previous_path)
        .map_err(|source| AppError::io("staging previous configuration", &previous_path, source))?;
    if let Err(source) = fs::rename(temporary_path, destination) {
        let _ = fs::rename(&previous_path, destination);
        return Err(AppError::io(
            "installing replacement configuration",
            destination,
            source,
        ));
    }
    fs::remove_file(&previous_path).map_err(|source| {
        AppError::io(
            "removing previous configuration backup",
            &previous_path,
            source,
        )
    })
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
    }

    #[test]
    fn creates_default_when_file_is_missing() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = ConfigStore::new(directory.path().join("config.json"));
        let loaded = store.load_or_create().expect("create default");
        assert!(loaded.created_default);
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
}
