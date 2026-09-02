use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use askbridge_core::{AppError, Result};

pub(super) const PAGE_UPLOAD_RETENTION: Duration = Duration::from_secs(10 * 60);
static TEMP_IMAGE_CLEANUP: OnceLock<Option<Sender<(Instant, PathBuf)>>> = OnceLock::new();

pub(crate) fn cleanup_stale_temp_images(data_root: &Path) -> Result<()> {
    cleanup_temp_images_older_than(data_root, PAGE_UPLOAD_RETENTION)
}

pub(super) fn create_retained_page_upload(
    temp_root: &Path,
    request_id: &str,
    bytes: &[u8],
) -> Result<PathBuf> {
    create_retained_page_upload_for(temp_root, request_id, bytes, PAGE_UPLOAD_RETENTION)
}

fn create_retained_page_upload_for(
    temp_root: &Path,
    request_id: &str,
    bytes: &[u8],
    retention: Duration,
) -> Result<PathBuf> {
    let image = TempImage::create(temp_root, request_id, bytes)?;
    let path = image.path().to_owned();
    image.retain_for_page_read(retention);
    Ok(path)
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
    remove_on_drop: bool,
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
        Ok(Self {
            path,
            remove_on_drop: true,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Keeps the file available for a page that may read its `File` object
    /// asynchronously after rendering a local attachment preview.
    pub(super) fn retain_for_page_read(mut self, retention: Duration) {
        self.remove_on_drop = !schedule_cleanup(self.path.clone(), retention);
    }
}

fn schedule_cleanup(path: PathBuf, retention: Duration) -> bool {
    let Some(deadline) = Instant::now().checked_add(retention) else {
        return false;
    };
    TEMP_IMAGE_CLEANUP
        .get_or_init(|| {
            let (sender, receiver) = mpsc::channel();
            thread::Builder::new()
                .name("askbridge-temp-image-cleanup".to_owned())
                .spawn(move || cleanup_worker(receiver))
                .ok()
                .map(|_| sender)
        })
        .as_ref()
        .is_some_and(|sender| sender.send((deadline, path)).is_ok())
}

fn cleanup_worker(receiver: Receiver<(Instant, PathBuf)>) {
    let mut pending: BinaryHeap<Reverse<(Instant, PathBuf)>> = BinaryHeap::new();
    loop {
        let now = Instant::now();
        while pending
            .peek()
            .is_some_and(|Reverse((deadline, _))| *deadline <= now)
        {
            if let Some(Reverse((_, path))) = pending.pop() {
                let _ = fs::remove_file(path);
            }
        }

        let received = match pending.peek() {
            Some(Reverse((deadline, _))) => {
                match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Ok(item) => Some(item),
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            None => match receiver.recv() {
                Ok(item) => Some(item),
                Err(_) => break,
            },
        };
        if let Some(item) = received {
            pending.push(Reverse(item));
        }
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
        if self.remove_on_drop {
            let _ = fs::remove_file(&self.path);
        }
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

    #[test]
    fn retained_image_survives_scope_then_expires() {
        let root = tempdir().expect("temp root");
        let path = create_retained_page_upload_for(
            root.path(),
            "request",
            b"png",
            Duration::from_millis(100),
        )
        .expect("retained page upload");
        assert!(path.exists(), "retained image was deleted immediately");

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while path.exists() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!path.exists(), "retained image did not expire");
    }

    #[test]
    fn page_upload_retention_is_ten_minutes() {
        assert_eq!(PAGE_UPLOAD_RETENTION, Duration::from_secs(10 * 60));
    }
}
