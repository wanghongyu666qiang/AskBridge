use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use askbridge_core::{AppError, Result};
use tracing_subscriber::{
    Registry, filter::LevelFilter, fmt::MakeWriter, layer::SubscriberExt, reload,
    util::SubscriberInitExt,
};

const LOG_DIRECTORY: &str = "logs";
const LOG_FILE: &str = "askbridge.log";
const ROTATED_LOG_FILE: &str = "askbridge.log.1";
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
type LogFilterHandle = reload::Handle<LevelFilter, Registry>;
static LOG_FILTER: OnceLock<LogFilterHandle> = OnceLock::new();

pub fn init(data_root: &Path, debug_logging: bool) -> Result<PathBuf> {
    let directory = data_root.join(LOG_DIRECTORY);
    fs::create_dir_all(&directory)
        .map_err(|source| AppError::io("creating log directory", &directory, source))?;
    let path = directory.join(LOG_FILE);
    rotate_if_needed(&path)?;
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|source| AppError::io("opening application log", &path, source))?;
    let writer = SharedWriter(Arc::new(Mutex::new(file)));
    let (filter, handle) = reload::Layer::new(level_filter(debug_logging));
    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_target(false)
                .with_writer(writer),
        )
        .try_init()
        .map_err(|error| {
            AppError::ConfigurationInvalid(format!("initializing structured logging: {error}"))
        })?;
    LOG_FILTER.set(handle).map_err(|_| {
        AppError::ConfigurationInvalid(
            "structured logging filter was already initialized".to_owned(),
        )
    })?;
    Ok(path)
}

pub fn set_debug_logging(enabled: bool) -> Result<()> {
    let handle = LOG_FILTER.get().ok_or_else(|| {
        AppError::ConfigurationInvalid("structured logging is not initialized".to_owned())
    })?;
    handle.reload(level_filter(enabled)).map_err(|error| {
        AppError::ConfigurationInvalid(format!("reloading structured logging filter: {error}"))
    })
}

const fn level_filter(debug_logging: bool) -> LevelFilter {
    if debug_logging {
        LevelFilter::DEBUG
    } else {
        LevelFilter::INFO
    }
}

fn rotate_if_needed(path: &Path) -> Result<()> {
    let size = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(AppError::io(
                "reading application log metadata",
                path,
                source,
            ));
        }
    };
    if size < MAX_LOG_BYTES {
        return Ok(());
    }
    let rotated = path.with_file_name(ROTATED_LOG_FILE);
    if rotated.exists() {
        fs::remove_file(&rotated)
            .map_err(|source| AppError::io("removing rotated application log", &rotated, source))?;
    }
    fs::rename(path, &rotated)
        .map_err(|source| AppError::io("rotating application log", &rotated, source))
}

#[derive(Clone)]
struct SharedWriter(Arc<Mutex<File>>);

impl<'a> MakeWriter<'a> for SharedWriter {
    type Writer = LockedWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LockedWriter(self.0.clone())
    }
}

struct LockedWriter(Arc<Mutex<File>>);

impl Write for LockedWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("application log lock was poisoned"))?
            .write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("application log lock was poisoned"))?
            .flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_log_is_preserved() {
        let directory = tempfile::tempdir().expect("temporary log directory");
        let path = directory.path().join(LOG_FILE);
        fs::write(&path, b"safe structured event\n").expect("write log");
        rotate_if_needed(&path).expect("rotate check");
        assert_eq!(
            fs::read(path).expect("read log"),
            b"safe structured event\n"
        );
    }

    #[test]
    fn privacy_forbidden_fields_are_absent_from_runtime_tracing_calls() {
        let sources = [
            ("app", include_str!("app.rs")),
            ("adapter", include_str!("adapter/mod.rs")),
            ("browser worker", include_str!("browser/worker.rs")),
            ("CDP", include_str!("browser/cdp.rs")),
            ("Chrome", include_str!("browser/chrome.rs")),
            ("clipboard", include_str!("clipboard.rs")),
            ("fallback", include_str!("fallback.rs")),
            ("prompt", include_str!("prompt.rs")),
            ("settings", include_str!("settings_v2.rs")),
            ("startup", include_str!("startup.rs")),
            ("tray", include_str!("tray.rs")),
        ];
        for (module, source) in sources {
            for forbidden in [
                "prompt = %",
                "clipboard = %",
                "html = %",
                "cookie = %",
                "target_url = %",
                "debug_port = %",
                "error = %error",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{module} logging contains forbidden field fragment {forbidden}"
                );
            }
        }
    }
}
