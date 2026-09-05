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
            ("app controller", include_str!("app/controller.rs")),
            ("app capture", include_str!("app/capture_flow.rs")),
            ("app commands", include_str!("app/commands.rs")),
            ("app dispatch", include_str!("app/dispatch_flow.rs")),
            ("app errors", include_str!("app/error_handler.rs")),
            ("app events", include_str!("app/events.rs")),
            ("app tray", include_str!("tray.rs")),
            ("app update flow", include_str!("app/update_flow.rs")),
            ("adapter generic", include_str!("adapter/generic.rs")),
            ("adapter JavaScript", include_str!("adapter/javascript.rs")),
            (
                "adapter provider health",
                include_str!("adapter/provider_health.rs"),
            ),
            (
                "adapter rules update",
                include_str!("adapter/rules_update.rs"),
            ),
            ("app icon", include_str!("app_icon.rs")),
            ("browser worker", include_str!("browser/worker/mod.rs")),
            (
                "browser worker jobs",
                include_str!("browser/worker/jobs.rs"),
            ),
            (
                "browser worker service",
                include_str!("browser/worker/service.rs"),
            ),
            (
                "browser worker prepare",
                include_str!("browser/worker/prepare.rs"),
            ),
            (
                "browser worker paste",
                include_str!("browser/worker/paste.rs"),
            ),
            ("CDP", include_str!("browser/cdp.rs")),
            ("Chrome", include_str!("browser/chrome.rs")),
            ("clipboard image", include_str!("clipboard_image.rs")),
            ("capture mod", include_str!("capture/mod.rs")),
            (
                "overlay session",
                include_str!("capture/overlay/session.rs"),
            ),
            ("main", include_str!("main.rs")),
            ("paste mode", include_str!("paste_mode/mod.rs")),
            (
                "paste mode discovery",
                include_str!("paste_mode/discover.rs"),
            ),
            ("paste mode focus", include_str!("paste_mode/focus.rs")),
            ("paste mode receipt", include_str!("paste_mode/receipt.rs")),
            (
                "paste mode keystroke",
                include_str!("paste_mode/keystroke.rs"),
            ),
            ("settings", include_str!("settings_v2/mod.rs")),
            ("single instance", include_str!("single_instance.rs")),
            ("startup", include_str!("startup.rs")),
            ("update service", include_str!("update/mod.rs")),
        ];
        let forbidden_fields = [
            "prompt",
            "clipboard",
            "html",
            "cookie",
            "target_url",
            "debug_port",
            "error",
        ];
        for (module, source) in sources {
            for field in forbidden_fields {
                assert!(
                    !contains_forbidden_logging_field(source, field),
                    "{module} logs a value into the privacy-sensitive field {field}"
                );
            }
        }
    }

    /// True when a tracing call writes any value into `field`, either by
    /// naming the field (`{field} = %anything`) or by formatting an expression
    /// whose last path segment is the field name (`%value.{field}`, bare
    /// `%{field}`, `?{field}`). Matching on expression shape instead of one
    /// literal keeps renames such as `primary_failure.error` from silently
    /// bypassing the guard.
    fn contains_forbidden_logging_field(source: &str, field: &str) -> bool {
        source
            .lines()
            .any(|line| contains_field_assignment(line, field))
    }

    fn contains_field_assignment(line: &str, field: &str) -> bool {
        if line.contains(&format!("{field} = %")) || line.contains(&format!("{field} = ?")) {
            return true;
        }
        let bytes = line.as_bytes();
        let mut index = 0_usize;
        while let Some(offset) = line[index..].find(['%', '?']) {
            let start = index + offset + 1;
            let mut end = start;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'.')
            {
                end += 1;
            }
            if line[start..end].rsplit('.').next() == Some(field)
                || (field == "error"
                    && line[start..end]
                        .rsplit('.')
                        .next()
                        .is_some_and(|name| name.ends_with("_error")))
            {
                return true;
            }
            index = end.max(start);
        }
        false
    }

    #[test]
    fn forbidden_field_scanner_catches_renamed_error_values() {
        assert!(contains_forbidden_logging_field(
            "warn!(error = %primary_failure.error, \"x\")",
            "error"
        ));
        assert!(contains_forbidden_logging_field(
            "warn!(%error, \"x\")",
            "error"
        ));
        assert!(contains_forbidden_logging_field(
            "warn!(detail = ?paste_error, \"x\")",
            "error"
        ));
        assert!(!contains_forbidden_logging_field(
            "warn!(error_kind = failure.kind(), \"x\")",
            "error"
        ));
        assert!(!contains_forbidden_logging_field(
            "info!(stage = \"started\", completed = true)",
            "error"
        ));
    }
}
