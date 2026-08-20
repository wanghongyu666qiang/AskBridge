use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use askbridge_core::{AppError, Result};

pub(crate) fn cleanup_stale_temp_images(data_root: &Path) -> Result<()> {
    cleanup_temp_images_older_than(data_root, Duration::from_secs(24 * 60 * 60))
}

pub(super) fn cleanup_temp_images_older_than(
    data_root: &Path,
    minimum_age: Duration,
) -> Result<()> {
    let temp_root = data_root.join("Temp");
    let entries = match fs::read_dir(&temp_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(AppError::io(
                "reading temporary image directory",
                &temp_root,
                error,
            ));
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_askbridge_png = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("askbridge-") && name.ends_with(".png"));
        if !is_askbridge_png {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= minimum_age);
        if stale {
            fs::remove_file(&path)
                .map_err(|error| AppError::io("removing stale temporary image", path, error))?;
        }
    }
    Ok(())
}

pub(super) struct TempImage {
    path: PathBuf,
}

impl TempImage {
    pub(super) fn create(temp_root: &Path, request_id: &str, bytes: &[u8]) -> Result<Self> {
        Self::create_with_writer(temp_root, request_id, bytes, |file, bytes| {
            file.write_all(bytes).and_then(|_| file.sync_all())
        })
    }

    fn create_with_writer<F>(
        temp_root: &Path,
        request_id: &str,
        bytes: &[u8],
        write: F,
    ) -> Result<Self>
    where
        F: FnOnce(&mut fs::File, &[u8]) -> std::io::Result<()>,
    {
        fs::create_dir_all(temp_root).map_err(|error| {
            AppError::io("creating temporary image directory", temp_root, error)
        })?;
        let safe_id: String = request_id
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
            .take(48)
            .collect();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = temp_root.join(format!(
            "askbridge-{}-{nonce}.png",
            if safe_id.is_empty() {
                "request"
            } else {
                &safe_id
            }
        ));
        let mut cleanup = TempImageCleanup::new(path.clone());
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| AppError::io("creating temporary image", &path, error))?;
        cleanup.mark_created();
        write(&mut file, bytes)
            .map_err(|error| AppError::io("writing temporary image", &path, error))?;
        cleanup.disarm();
        Ok(Self { path })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

struct TempImageCleanup {
    path: PathBuf,
    armed: bool,
}

impl TempImageCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: false }
    }

    fn mark_created(&mut self) {
        self.armed = true;
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempImageCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl Drop for TempImage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn failed_write_removes_partial_temp_image() {
        let root = tempdir().expect("temp root");
        let error = TempImage::create_with_writer(root.path(), "request", b"png", |_, _| {
            Err(std::io::Error::other("injected write failure"))
        })
        .err()
        .expect("write must fail");
        assert!(matches!(error, AppError::Io { .. }));
        assert_eq!(fs::read_dir(root.path()).expect("read temp").count(), 0);
    }

    #[test]
    fn cleanup_removes_only_owned_stale_names() {
        let root = tempdir().expect("temp root");
        let temp = root.path().join("Temp");
        fs::create_dir(&temp).expect("Temp");
        fs::write(temp.join("askbridge-old.png"), b"old").expect("owned");
        fs::write(temp.join("other.png"), b"keep").expect("foreign");

        cleanup_temp_images_older_than(root.path(), Duration::ZERO).expect("cleanup");

        assert!(!temp.join("askbridge-old.png").exists());
        assert!(temp.join("other.png").exists());
    }
}
