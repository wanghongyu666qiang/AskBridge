//! Streams the setup download into a hidden partial file and publishes it
//! only after size and hash match the signed SHA256SUMS record.

use std::{fs, io::Write, path::Path, path::PathBuf};

use askbridge_core::{AppError, RELEASE_SIGNING_PUBLIC_KEY, Result, Sha256Stream};

use super::MAX_SETUP_BYTES;
use super::http::{get_https, get_https_chunks};
use super::verify::{expected_checksum, verify_checksum_signature};
use super::{AvailableUpdate, update_error};

// Progress notifications are throttled to at most one per step so the UI thread
// is not flooded by PostMessageW during a large download.
const PROGRESS_STEP_FRACTION: u64 = 50;
const PROGRESS_STEP_MIN_BYTES: u64 = 256 * 1024;
const MAX_CHECKSUM_BYTES: usize = 64 * 1024;
const MAX_SIG_BYTES: usize = 1024;

pub(super) fn download_release(
    update_root: &Path,
    release: &AvailableUpdate,
    mut report_progress: impl FnMut(u64, u64),
) -> Result<PathBuf> {
    fs::create_dir_all(update_root)
        .map_err(|source| AppError::io("creating update cache", update_root, source))?;
    let signature = get_https(&release.signature_url, MAX_SIG_BYTES)?;
    let signature_text =
        std::str::from_utf8(&signature).map_err(|_| update_error("更新签名不是 UTF-8 文本"))?;
    let checksum_source = get_https(&release.checksum_url, MAX_CHECKSUM_BYTES)?;
    // The signature is verified before the checksum file is trusted at all.
    verify_checksum_signature(
        &checksum_source,
        signature_text,
        RELEASE_SIGNING_PUBLIC_KEY,
    )?;
    let checksum_text = std::str::from_utf8(&checksum_source)
        .map_err(|_| update_error("SHA256SUMS 不是 UTF-8 文本"))?;
    let expected_hash = expected_checksum(checksum_text, &release.setup_name)?;

    let mut spool = SetupSpool::create(update_root, &release.setup_name)?;
    let mut last_reported: u64 = 0;
    let result = get_https_chunks(&release.setup_url, MAX_SETUP_BYTES, |chunk| {
        spool.write_chunk(chunk, release.setup_size)?;
        if should_report_progress(spool.received(), last_reported, release.setup_size) {
            last_reported = spool.received();
            report_progress(last_reported, release.setup_size);
        }
        Ok(())
    });
    result?;
    spool.finish(&expected_hash, release.setup_size)
}

/// Spools the downloaded setup into a hidden `.partial` file while hashing,
/// then publishes it under its final name only after the size and hash match
/// the signed SHA256SUMS record. Aborted downloads are removed on drop.
struct SetupSpool {
    file: fs::File,
    hasher: Sha256Stream,
    temporary: PathBuf,
    destination: PathBuf,
    hash_record: PathBuf,
    received: u64,
}

impl SetupSpool {
    fn create(update_root: &Path, setup_name: &str) -> Result<Self> {
        if !super::verify::is_safe_file_name(setup_name) {
            return Err(update_error("更新安装包文件名不安全"));
        }
        let temporary = update_root.join(format!(".{setup_name}.{}.partial", std::process::id()));
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|source| AppError::io("creating update download", &temporary, source))?;
        Ok(Self {
            file,
            hasher: Sha256Stream::new(),
            temporary,
            destination: update_root.join(setup_name),
            // Persisted next to the published setup so the tray retry path can
            // re-verify the file instead of trusting whatever sits there.
            hash_record: update_root.join(format!("{setup_name}.sha256")),
            received: 0,
        })
    }

    fn write_chunk(&mut self, chunk: &[u8], limit: u64) -> Result<()> {
        self.received = self
            .received
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| update_error("更新安装包大小超出安全限制"))?;
        if self.received > limit {
            return Err(update_error("更新安装包大小与 GitHub Release 不一致"));
        }
        self.hasher.update(chunk);
        self.file
            .write_all(chunk)
            .map_err(|source| AppError::io("writing update download", &self.temporary, source))?;
        Ok(())
    }

    fn received(&self) -> u64 {
        self.received
    }

    fn finish(mut self, expected_hash: &str, expected_size: u64) -> Result<PathBuf> {
        let outcome = self.publish(expected_hash, expected_size);
        if outcome.is_err() {
            let _ = fs::remove_file(&self.temporary);
        }
        outcome
    }

    fn publish(&mut self, expected_hash: &str, expected_size: u64) -> Result<PathBuf> {
        if self.received != expected_size {
            return Err(update_error("更新安装包大小与 GitHub Release 不一致"));
        }
        let actual_hash = self.hasher.finish_hex();
        if !actual_hash.eq_ignore_ascii_case(expected_hash) {
            return Err(update_error("更新安装包 SHA-256 校验失败"));
        }
        self.file
            .sync_all()
            .map_err(|source| AppError::io("writing update download", &self.temporary, source))?;
        if self.destination.exists() {
            fs::remove_file(&self.destination).map_err(|source| {
                AppError::io("replacing cached update", &self.destination, source)
            })?;
        }
        fs::rename(&self.temporary, &self.destination).map_err(|source| {
            AppError::io("installing cached update", &self.destination, source)
        })?;
        if let Err(source) = fs::write(&self.hash_record, format!("{actual_hash}\n")) {
            // Without the record the retry path cannot re-verify the file, so
            // keep the cache consistent by dropping the published setup too.
            let _ = fs::remove_file(&self.destination);
            return Err(AppError::io(
                "recording update checksum",
                &self.hash_record,
                source,
            ));
        }
        Ok(self.destination.clone())
    }
}

impl Drop for SetupSpool {
    fn drop(&mut self) {
        // After a successful rename the temporary path no longer exists.
        let _ = fs::remove_file(&self.temporary);
    }
}

pub(super) fn cleanup_stale_updates(update_root: &Path) {
    let Ok(entries) = fs::read_dir(update_root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let is_setup = name.starts_with("AskBridge-") && name.ends_with("-Setup.exe");
        let is_partial = name.starts_with(".AskBridge-") && name.ends_with(".partial");
        let is_checksum_record = name.starts_with("AskBridge-") && name.ends_with(".sha256");
        if path.is_file() && (is_setup || is_partial || is_checksum_record) {
            let _ = fs::remove_file(path);
        }
    }
}

fn progress_step(total: u64) -> u64 {
    (total / PROGRESS_STEP_FRACTION).max(PROGRESS_STEP_MIN_BYTES)
}

fn should_report_progress(received: u64, last_reported: u64, total: u64) -> bool {
    received.saturating_sub(last_reported) >= progress_step(total)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::update::verify::validate_downloaded_setup;
    use super::*;

    fn test_hash(bytes: &[u8]) -> String {
        let mut hasher = Sha256Stream::new();
        hasher.update(bytes);
        hasher.finish_hex()
    }

    #[test]
    fn persists_updates_only_below_updates_directory() {
        let root = tempdir().expect("root");
        let updates = root.path().join("Updates");
        fs::create_dir_all(&updates).expect("updates");
        let mut spool = SetupSpool::create(&updates, "AskBridge-1.2.3-Setup.exe").expect("spool");
        spool.write_chunk(b"setup", 64).expect("chunk");
        let path = spool.finish(&test_hash(b"setup"), 5).expect("persist");
        assert_eq!(fs::read(&path).expect("read"), b"setup");
        assert!(validate_downloaded_setup(&updates, &path).is_ok());
        assert!(SetupSpool::create(&updates, "..\\outside.exe").is_err());
    }

    #[test]
    fn streamed_setup_publishes_only_after_hash_and_size_match() {
        let root = tempdir().expect("root");
        let updates = root.path().join("Updates");
        fs::create_dir_all(&updates).expect("updates");
        let name = "AskBridge-1.2.3-Setup.exe";

        let mut short = SetupSpool::create(&updates, name).expect("spool");
        short.write_chunk(b"setup", 64).expect("chunk");
        assert!(short.finish(&test_hash(b"setup"), 6).is_err());
        assert!(
            !updates.join(name).exists(),
            "size mismatch must not publish"
        );

        let mut wrong_hash = SetupSpool::create(&updates, name).expect("spool");
        wrong_hash.write_chunk(b"setup", 64).expect("chunk");
        assert!(wrong_hash.finish(&test_hash(b"other"), 5).is_err());
        assert!(
            !updates.join(name).exists(),
            "hash mismatch must not publish"
        );

        let mut oversized = SetupSpool::create(&updates, name).expect("spool");
        assert!(oversized.write_chunk(b"0123456789", 4).is_err());
        assert!(!updates.join(name).exists());

        let mut valid = SetupSpool::create(&updates, name).expect("spool");
        valid.write_chunk(b"set", 64).expect("chunk");
        valid.write_chunk(b"up", 64).expect("chunk");
        let path = valid.finish(&test_hash(b"setup"), 5).expect("publish");
        assert!(updates.join(name).exists());
        let leftovers: Vec<String> = fs::read_dir(&updates)
            .expect("dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".AskBridge-") && name.ends_with(".partial"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "leftover partial files: {leftovers:?}"
        );
        assert!(validate_downloaded_setup(&updates, &path).is_ok());
    }

    #[test]
    fn progress_reports_are_throttled_to_bounded_steps() {
        let total = 100 * 1024 * 1024;
        assert!(!should_report_progress(1, 0, total));
        let step = progress_step(total);
        assert_eq!(step, 2 * 1024 * 1024);
        assert!(should_report_progress(step, 0, total));
        assert!(!should_report_progress(step, step, total));

        // Small downloads fall back to a fixed minimum step.
        assert_eq!(progress_step(4096), PROGRESS_STEP_MIN_BYTES);
        assert!(should_report_progress(PROGRESS_STEP_MIN_BYTES, 0, 4096));
    }

    #[test]
    fn startup_cleanup_removes_only_owned_update_files() {
        let root = tempdir().expect("root");
        let updates = root.path().join("Updates");
        fs::create_dir_all(&updates).expect("updates");
        let setup = updates.join("AskBridge-1.2.3-Setup.exe");
        let partial = updates.join(".AskBridge-1.2.3-Setup.exe.42.partial");
        let unrelated = updates.join("keep.txt");
        fs::write(&setup, b"setup").expect("setup");
        fs::write(&partial, b"partial").expect("partial");
        fs::write(&unrelated, b"keep").expect("unrelated");
        cleanup_stale_updates(&updates);
        assert!(!setup.exists());
        assert!(!partial.exists());
        assert!(unrelated.exists());
    }

    #[test]
    #[ignore = "requires live GitHub access and ASKBRIDGE_UPDATE_TEST_ROOT"]
    fn live_github_release_download_matches_sha256() {
        let root = std::env::var_os("ASKBRIDGE_UPDATE_TEST_ROOT")
            .map(PathBuf::from)
            .expect("ASKBRIDGE_UPDATE_TEST_ROOT");
        assert!(root.is_absolute());
        let baseline = super::super::version::ReleaseVersion::parse("0.0.0").expect("baseline");
        let release = super::super::release::check_latest(&baseline)
            .expect("live release check")
            .expect("published release");
        let setup = download_release(&root, &release, |_, _| {}).expect("verified setup download");
        assert!(setup.is_file());
        assert_eq!(setup.parent(), Some(root.as_path()));
        cleanup_stale_updates(&root);
        let _ = fs::remove_dir(&root);
    }
}
