mod http;

use std::{
    collections::VecDeque,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use askbridge_core::{AppError, RELEASE_SIGNING_PUBLIC_KEY, Result, Sha256Stream, hex_to_array};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

use self::http::{get_https, get_https_chunks};

pub const WM_UPDATE_EVENT: u32 = WM_APP + 7;

const RELEASE_API_URL: &str =
    "https://api.github.com/repos/wanghongyu666qiang/AskBridge/releases/latest";
const RELEASE_DOWNLOAD_PREFIX: &str =
    "https://github.com/wanghongyu666qiang/AskBridge/releases/download/";
const RELEASE_PAGE_PREFIX: &str = "https://github.com/wanghongyu666qiang/AskBridge/releases/tag/";
const UPDATE_DIRECTORY: &str = "Updates";
const DISABLE_AUTO_CHECK_ENV: &str = "ASKBRIDGE_DISABLE_UPDATE_CHECK";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_RELEASE_BYTES: usize = 1024 * 1024;
const MAX_CHECKSUM_BYTES: usize = 64 * 1024;
const MAX_SIG_BYTES: usize = 1024;
const MAX_SETUP_BYTES: usize = 128 * 1024 * 1024;
const MAX_RELEASE_NOTES_CHARS: usize = 2_000;
// Progress notifications are throttled to at most one per step so the UI thread
// is not flooded by PostMessageW during a large download.
const PROGRESS_STEP_FRACTION: u64 = 50;
const PROGRESS_STEP_MIN_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    version: String,
    notes: String,
    release_url: String,
    setup_name: String,
    setup_url: String,
    setup_size: u64,
    checksum_url: String,
    signature_url: String,
}

impl AvailableUpdate {
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn notes(&self) -> &str {
        &self.notes
    }

    pub fn release_url(&self) -> &str {
        &self.release_url
    }
}

#[derive(Debug)]
pub enum UpdateEvent {
    Checked {
        available: Option<AvailableUpdate>,
        manual: bool,
    },
    DownloadProgress {
        version: String,
        received: u64,
        total: u64,
    },
    Downloaded {
        release: AvailableUpdate,
        setup_path: PathBuf,
    },
    Failed {
        action: UpdateAction,
        manual: bool,
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAction {
    Check,
    Download,
}

enum UpdateCommand {
    Check { manual: bool },
    Download(AvailableUpdate),
    Shutdown,
}

pub struct UpdateService {
    commands: Sender<UpdateCommand>,
    events: Arc<Mutex<VecDeque<UpdateEvent>>>,
    worker: Option<JoinHandle<()>>,
    update_root: PathBuf,
}

impl UpdateService {
    pub fn start(owner: HWND, data_root: &Path, current_version: &str) -> Result<Self> {
        if !data_root.is_absolute() {
            return Err(update_error("更新缓存根目录必须是绝对路径"));
        }
        let current_version = ReleaseVersion::parse(current_version)?;
        let update_root = data_root.join(UPDATE_DIRECTORY);
        cleanup_stale_updates(&update_root);
        let automatic_checks_enabled = std::env::var(DISABLE_AUTO_CHECK_ENV).as_deref() != Ok("1");
        let (commands, receiver) = mpsc::channel();
        let events = Arc::new(Mutex::new(VecDeque::new()));
        let worker_events = Arc::clone(&events);
        let worker_root = update_root.clone();
        let owner = owner as usize;
        let worker = thread::spawn(move || {
            worker_loop(
                owner,
                receiver,
                worker_events,
                worker_root,
                current_version,
                automatic_checks_enabled,
            );
        });
        if automatic_checks_enabled {
            commands
                .send(UpdateCommand::Check { manual: false })
                .map_err(|_| update_error("更新检查线程不可用"))?;
        }
        Ok(Self {
            commands,
            events,
            worker: Some(worker),
            update_root,
        })
    }

    pub fn check_now(&self) -> Result<()> {
        self.commands
            .send(UpdateCommand::Check { manual: true })
            .map_err(|_| update_error("更新检查线程不可用"))
    }

    pub fn download(&self, release: AvailableUpdate) -> Result<()> {
        self.commands
            .send(UpdateCommand::Download(release))
            .map_err(|_| update_error("更新下载线程不可用"))
    }

    /// Returns the previously downloaded setup for `version` if it still sits
    /// in the update cache as a verified direct child. Lets the tray retry an
    /// install without downloading again after a failed installer launch.
    pub fn cached_verified_setup(&self, version: &str) -> Option<PathBuf> {
        let name = format!("AskBridge-{version}-Setup.exe");
        if !is_safe_file_name(&name) {
            return None;
        }
        let candidate = self.update_root.join(name);
        validate_downloaded_setup(&self.update_root, &candidate)
            .and_then(|()| verify_cached_hash(&candidate))
            .map(|_| candidate)
            .ok()
    }

    pub fn drain_events(&self) -> Vec<UpdateEvent> {
        let Ok(mut events) = self.events.lock() else {
            return vec![UpdateEvent::Failed {
                action: UpdateAction::Check,
                manual: false,
                message: "更新事件队列不可用".to_owned(),
            }];
        };
        events.drain(..).collect()
    }

    pub fn supports_in_place_update(&self) -> bool {
        std::env::current_exe()
            .ok()
            .and_then(|executable| executable.parent().map(Path::to_path_buf))
            .is_some_and(|install_root| install_root.join("install-manifest.json").is_file())
    }

    pub fn launch_installer(&self, setup_path: &Path) -> Result<()> {
        validate_downloaded_setup(&self.update_root, setup_path)?;
        verify_cached_hash(setup_path)?;
        let executable = std::env::current_exe().map_err(|source| {
            update_error(format!("无法定位当前运行的 AskBridge 程序（{source}）"))
        })?;
        let install_root = executable
            .parent()
            .ok_or_else(|| update_error("无法确定 AskBridge 安装目录"))?;
        if !install_root.join("install-manifest.json").is_file() {
            return Err(update_error(
                "当前程序不是通过 AskBridge 安装器安装，不能执行应用内覆盖升级",
            ));
        }
        Command::new(setup_path)
            .env("ASKBRIDGE_INSTALL_ROOT", install_root)
            .env(
                "ASKBRIDGE_UPDATE_PARENT_PID",
                std::process::id().to_string(),
            )
            .env("ASKBRIDGE_RESTART_AFTER_INSTALL", "1")
            .spawn()
            .map_err(|source| AppError::io("launching AskBridge updater", setup_path, source))?;
        Ok(())
    }
}

impl Drop for UpdateService {
    fn drop(&mut self) {
        let _ = self.commands.send(UpdateCommand::Shutdown);
        // Dropping the handle detaches the worker. A synchronous WinHTTP request may still be
        // inside its bounded timeout; application exit and self-update must not wait for it.
        let _ = self.worker.take();
    }
}

fn worker_loop(
    owner: usize,
    commands: Receiver<UpdateCommand>,
    events: Arc<Mutex<VecDeque<UpdateEvent>>>,
    update_root: PathBuf,
    current_version: ReleaseVersion,
    automatic_checks_enabled: bool,
) {
    // The automatic cadence is a wall-clock deadline independent of command
    // traffic: manual checks and downloads must not postpone it indefinitely.
    let mut next_automatic_check = Instant::now() + CHECK_INTERVAL;
    loop {
        let timeout = if automatic_checks_enabled {
            next_automatic_check.saturating_duration_since(Instant::now())
        } else {
            CHECK_INTERVAL
        };
        match commands.recv_timeout(timeout) {
            Ok(UpdateCommand::Check { manual }) => {
                let event = match check_latest(&current_version) {
                    Ok(available) => UpdateEvent::Checked { available, manual },
                    Err(error) => UpdateEvent::Failed {
                        action: UpdateAction::Check,
                        manual,
                        message: error.to_string(),
                    },
                };
                push_event(owner, &events, event);
            }
            Ok(UpdateCommand::Download(release)) => {
                let version = release.version.clone();
                let progress_events = Arc::clone(&events);
                let event = match download_release(&update_root, &release, |received, total| {
                    push_event(
                        owner,
                        &progress_events,
                        UpdateEvent::DownloadProgress {
                            version: version.clone(),
                            received,
                            total,
                        },
                    );
                }) {
                    Ok(setup_path) => UpdateEvent::Downloaded {
                        release,
                        setup_path,
                    },
                    Err(error) => UpdateEvent::Failed {
                        action: UpdateAction::Download,
                        manual: true,
                        message: error.to_string(),
                    },
                };
                push_event(owner, &events, event);
            }
            Ok(UpdateCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if !automatic_checks_enabled {
                    continue;
                }
                next_automatic_check = Instant::now() + CHECK_INTERVAL;
                let event = match check_latest(&current_version) {
                    Ok(available) => UpdateEvent::Checked {
                        available,
                        manual: false,
                    },
                    Err(error) => UpdateEvent::Failed {
                        action: UpdateAction::Check,
                        manual: false,
                        message: error.to_string(),
                    },
                };
                push_event(owner, &events, event);
            }
        }
    }
}

fn push_event(owner: usize, events: &Arc<Mutex<VecDeque<UpdateEvent>>>, event: UpdateEvent) {
    let Ok(mut queue) = events.lock() else {
        return;
    };
    queue.push_back(event);
    drop(queue);
    // SAFETY: The owner is the live hidden AskBridge window. This private message carries no
    // pointer-bearing parameters; event data remains owned by the synchronized queue.
    unsafe {
        PostMessageW(owner as HWND, WM_UPDATE_EVENT, 0, 0);
    }
}

fn check_latest(current_version: &ReleaseVersion) -> Result<Option<AvailableUpdate>> {
    let source = get_https(RELEASE_API_URL, MAX_RELEASE_BYTES)?;
    let release: GithubRelease = serde_json::from_slice(&source)
        .map_err(|_| update_error("GitHub 返回的更新信息不是有效 JSON"))?;
    parse_release(release, current_version)
}

fn parse_release(
    release: GithubRelease,
    current_version: &ReleaseVersion,
) -> Result<Option<AvailableUpdate>> {
    let version_text = release
        .tag_name
        .strip_prefix('v')
        .ok_or_else(|| update_error("发布标签必须以 v 开头"))?;
    let version = ReleaseVersion::parse(version_text)?;
    if version <= *current_version {
        return Ok(None);
    }
    let expected_setup = format!("AskBridge-{version_text}-Setup.exe");
    let expected_checksums = format!("AskBridge-{version_text}-SHA256SUMS.txt");
    let expected_signature = format!("{expected_checksums}.sig");
    let setup = single_asset(&release.assets, &expected_setup)?;
    let checksums = single_asset(&release.assets, &expected_checksums)?;
    let signature = single_asset(&release.assets, &expected_signature)?;
    validate_release_asset(
        &setup.browser_download_url,
        &release.tag_name,
        &expected_setup,
    )?;
    validate_release_asset(
        &checksums.browser_download_url,
        &release.tag_name,
        &expected_checksums,
    )?;
    validate_release_asset(
        &signature.browser_download_url,
        &release.tag_name,
        &expected_signature,
    )?;
    let expected_page = format!("{RELEASE_PAGE_PREFIX}{}", release.tag_name);
    if release.html_url != expected_page {
        return Err(update_error("发布页面地址不属于 AskBridge 官方仓库"));
    }
    if setup.size == 0 || setup.size > MAX_SETUP_BYTES as u64 {
        return Err(update_error("更新安装包大小超出安全限制"));
    }
    Ok(Some(AvailableUpdate {
        version: version_text.to_owned(),
        notes: truncate_chars(release.body.unwrap_or_default(), MAX_RELEASE_NOTES_CHARS),
        release_url: release.html_url,
        setup_name: expected_setup,
        setup_url: setup.browser_download_url.clone(),
        setup_size: setup.size,
        checksum_url: checksums.browser_download_url.clone(),
        signature_url: signature.browser_download_url.clone(),
    }))
}

fn single_asset<'a>(assets: &'a [GithubAsset], expected_name: &str) -> Result<&'a GithubAsset> {
    let mut matches = assets.iter().filter(|asset| asset.name == expected_name);
    let asset = matches
        .next()
        .ok_or_else(|| update_error(format!("发布缺少 {expected_name}")))?;
    if matches.next().is_some() {
        return Err(update_error(format!("发布包含重复的 {expected_name}")));
    }
    Ok(asset)
}

fn validate_release_asset(url: &str, tag: &str, name: &str) -> Result<()> {
    let expected = format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{name}");
    if url != expected {
        return Err(update_error("更新资产地址不属于 AskBridge 官方仓库"));
    }
    Ok(())
}

fn download_release(
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
    verify_checksum_signature(&checksum_source, signature_text, RELEASE_SIGNING_PUBLIC_KEY)?;
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
        if !is_safe_file_name(setup_name) {
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

fn cleanup_stale_updates(update_root: &Path) {
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

fn expected_checksum(source: &str, expected_name: &str) -> Result<String> {
    let mut found = None;
    for line in source.lines().filter(|line| !line.trim().is_empty()) {
        let Some((hash, name)) = line.split_once("  ") else {
            return Err(update_error("SHA256SUMS 格式无效"));
        };
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(update_error("SHA256SUMS 包含无效哈希"));
        }
        if name == expected_name && found.replace(hash.to_owned()).is_some() {
            return Err(update_error("SHA256SUMS 包含重复安装包记录"));
        }
    }
    found.ok_or_else(|| update_error("SHA256SUMS 未包含更新安装包"))
}

/// Verifies the release maintainer's offline Ed25519 signature over the exact
/// SHA256SUMS bytes. The public key is embedded at compile time, so tampering
/// with the GitHub release cannot produce accepted checksums.
fn verify_checksum_signature(
    message: &[u8],
    signature_text: &str,
    public_key_hex: &str,
) -> Result<()> {
    let public_key_bytes =
        hex_to_array(public_key_hex.trim()).ok_or_else(|| update_error("内嵌更新公钥格式无效"))?;
    let signature_bytes =
        hex_to_array(signature_text.trim()).ok_or_else(|| update_error("更新签名格式无效"))?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_bytes)
        .map_err(|_| update_error("内嵌更新公钥无效"))?;
    let signature = Signature::from_bytes(&signature_bytes);
    verifying_key
        .verify(message, &signature)
        .map_err(|_| update_error("更新签名校验失败，安装包来源不可信"))
}

fn progress_step(total: u64) -> u64 {
    (total / PROGRESS_STEP_FRACTION).max(PROGRESS_STEP_MIN_BYTES)
}

fn should_report_progress(received: u64, last_reported: u64, total: u64) -> bool {
    received.saturating_sub(last_reported) >= progress_step(total)
}

fn validate_downloaded_setup(update_root: &Path, setup_path: &Path) -> Result<()> {
    if !update_root.is_absolute()
        || !setup_path.is_absolute()
        || setup_path.parent() != Some(update_root)
        || !setup_path.is_file()
        || !setup_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| is_safe_file_name(name) && name.ends_with("-Setup.exe"))
    {
        return Err(update_error("更新安装包路径不安全"));
    }
    Ok(())
}

/// Re-hashes a cached setup against the `<name>.sha256` record written when it
/// was published. Anything that modified or replaced the file after download —
/// corruption, another process — is rejected and removed instead of launched.
fn verify_cached_hash(setup_path: &Path) -> Result<()> {
    let Some(name) = setup_path.file_name().and_then(|name| name.to_str()) else {
        return Err(update_error("更新安装包路径不安全"));
    };
    let hash_record = setup_path.with_file_name(format!("{name}.sha256"));
    let recorded = fs::read_to_string(&hash_record)
        .map_err(|_| update_error("缓存更新安装包缺少校验记录，请重新下载"))?;
    let expected = recorded.trim();
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(update_error("缓存更新校验记录无效，请重新下载"));
    }
    let actual = hash_file_streaming(setup_path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        let _ = fs::remove_file(setup_path);
        let _ = fs::remove_file(&hash_record);
        return Err(update_error("缓存更新安装包与校验记录不一致，请重新下载"));
    }
    Ok(())
}

fn hash_file_streaming(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .map_err(|source| AppError::io("reading cached update", path, source))?;
    let mut hasher = Sha256Stream::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| AppError::io("reading cached update", path, source))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finish_hex())
}

fn is_safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn truncate_chars(mut text: String, max_chars: usize) -> String {
    if let Some((index, _)) = text.char_indices().nth(max_chars) {
        text.truncate(index);
    }
    text
}

fn update_error(message: impl Into<String>) -> AppError {
    AppError::UpdateFailed(message.into())
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
    prerelease: Option<Vec<PrereleaseIdentifier>>,
}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(left), Some(right)) => left.cmp(right),
            })
    }
}

impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrereleaseIdentifier {
    Numeric(u64),
    Text(String),
}

impl Ord for PrereleaseIdentifier {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Numeric(left), Self::Numeric(right)) => left.cmp(right),
            (Self::Numeric(_), Self::Text(_)) => std::cmp::Ordering::Less,
            (Self::Text(_), Self::Numeric(_)) => std::cmp::Ordering::Greater,
            (Self::Text(left), Self::Text(right)) => left.cmp(right),
        }
    }
}

impl PartialOrd for PrereleaseIdentifier {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl ReleaseVersion {
    fn parse(value: &str) -> Result<Self> {
        let value = value.split_once('+').map_or(value, |(core, _)| core);
        let (core, prerelease) = value
            .split_once('-')
            .map_or((value, None), |(core, suffix)| (core, Some(suffix)));
        let mut parts = core.split('.');
        let major = parse_version_number(parts.next(), value)?;
        let minor = parse_version_number(parts.next(), value)?;
        let patch = parse_version_number(parts.next(), value)?;
        if parts.next().is_some() {
            return Err(update_error(format!("版本号 {value} 格式无效")));
        }
        let prerelease = prerelease
            .map(parse_prerelease)
            .transpose()?
            .filter(|parts| !parts.is_empty());
        Ok(Self {
            major,
            minor,
            patch,
            prerelease,
        })
    }
}

fn parse_version_number(value: Option<&str>, complete: &str) -> Result<u64> {
    let value = value.ok_or_else(|| update_error(format!("版本号 {complete} 格式无效")))?;
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(update_error(format!("版本号 {complete} 格式无效")));
    }
    value
        .parse()
        .map_err(|_| update_error(format!("版本号 {complete} 超出范围")))
}

fn parse_prerelease(value: &str) -> Result<Vec<PrereleaseIdentifier>> {
    value
        .split('.')
        .map(|part| {
            if part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            {
                return Err(update_error("预发布版本号格式无效"));
            }
            if part.bytes().all(|byte| byte.is_ascii_digit()) {
                if part.len() > 1 && part.starts_with('0') {
                    return Err(update_error("预发布数字版本不能有前导零"));
                }
                part.parse::<u64>()
                    .map(PrereleaseIdentifier::Numeric)
                    .map_err(|_| update_error("预发布版本号超出范围"))
            } else {
                Ok(PrereleaseIdentifier::Text(part.to_owned()))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn test_hash(bytes: &[u8]) -> String {
        let mut hasher = Sha256Stream::new();
        hasher.update(bytes);
        hasher.finish_hex()
    }

    fn encode_hex_upper(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02X}")).collect()
    }

    #[test]
    fn semantic_versions_order_stable_after_prerelease() {
        let prerelease = ReleaseVersion::parse("1.2.3-rc.1").expect("prerelease");
        let stable = ReleaseVersion::parse("1.2.3").expect("stable");
        let next = ReleaseVersion::parse("1.2.4").expect("next");
        assert!(prerelease < stable);
        assert!(stable < next);
    }

    #[test]
    fn cached_setup_hash_verification_accepts_match_and_rejects_tampering() {
        let root = tempdir().expect("root");
        let setup = root.path().join("AskBridge-1.2.3-Setup.exe");
        fs::write(&setup, b"installer bytes").expect("setup");

        // A missing record fails closed.
        assert!(verify_cached_hash(&setup).is_err());

        let record = root.path().join("AskBridge-1.2.3-Setup.exe.sha256");
        fs::write(&record, format!("{}\n", test_hash(b"installer bytes"))).expect("record");
        assert!(verify_cached_hash(&setup).is_ok());

        fs::write(&setup, b"tampered bytes").expect("tamper");
        assert!(verify_cached_hash(&setup).is_err());
        // The tampered pair is removed so a retry forces a fresh download.
        assert!(!setup.exists());
        assert!(!record.exists());
    }

    #[test]
    fn rejects_ambiguous_or_invalid_versions() {
        for value in ["v1.2.3", "1.2", "01.2.3", "1.2.3-01", "1.2.x"] {
            assert!(ReleaseVersion::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn parses_only_exact_setup_checksum() {
        let source = concat!(
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA  AskBridge-1.2.3-windows-x64.zip\n",
            "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB  AskBridge-1.2.3-Setup.exe\n",
        );
        assert_eq!(
            expected_checksum(source, "AskBridge-1.2.3-Setup.exe").expect("checksum"),
            "B".repeat(64)
        );
        assert!(expected_checksum(source, "AskBridge-9.9.9-Setup.exe").is_err());
    }

    #[test]
    fn rejects_duplicate_setup_checksum() {
        let line = format!("{}  AskBridge-1.2.3-Setup.exe\n", "A".repeat(64));
        assert!(expected_checksum(&(line.clone() + &line), "AskBridge-1.2.3-Setup.exe").is_err());
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
    fn checksums_require_a_valid_ed25519_signature() {
        use ed25519_dalek::{Signer, SigningKey};

        let secret = SigningKey::from_bytes(&[7_u8; 32]);
        let public_hex = encode_hex_upper(secret.verifying_key().as_bytes());
        let message =
            b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA  AskBridge-1.2.3-Setup.exe\n";
        let signature_hex = encode_hex_upper(&secret.sign(message).to_bytes());
        assert!(verify_checksum_signature(message, &signature_hex, &public_hex).is_ok());

        let tampered = b"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB  AskBridge-1.2.3-Setup.exe\n";
        assert!(verify_checksum_signature(tampered, &signature_hex, &public_hex).is_err());
        assert!(verify_checksum_signature(message, "not-hex", &public_hex).is_err());
        let invalid_key_hex: String = "00".repeat(32);
        assert!(verify_checksum_signature(message, &signature_hex, &invalid_key_hex).is_err());
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
    fn accepts_only_official_release_assets() {
        assert!(
            validate_release_asset(
                "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-Setup.exe",
                "v1.2.3",
                "AskBridge-1.2.3-Setup.exe"
            )
            .is_ok()
        );
        assert!(
            validate_release_asset(
                "https://example.test/AskBridge-1.2.3-Setup.exe",
                "v1.2.3",
                "AskBridge-1.2.3-Setup.exe"
            )
            .is_err()
        );
    }

    #[test]
    fn newer_release_requires_the_complete_official_asset_pair() {
        let release = GithubRelease {
            tag_name: "v1.2.3".to_owned(),
            html_url:
                "https://github.com/wanghongyu666qiang/AskBridge/releases/tag/v1.2.3"
                    .to_owned(),
            body: Some("notes".to_owned()),
            assets: vec![
                GithubAsset {
                    name: "AskBridge-1.2.3-Setup.exe".to_owned(),
                    browser_download_url: "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-Setup.exe".to_owned(),
                    size: 4096,
                },
                GithubAsset {
                    name: "AskBridge-1.2.3-SHA256SUMS.txt".to_owned(),
                    browser_download_url: "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-SHA256SUMS.txt".to_owned(),
                    size: 256,
                },
                GithubAsset {
                    name: "AskBridge-1.2.3-SHA256SUMS.txt.sig".to_owned(),
                    browser_download_url: "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-SHA256SUMS.txt.sig".to_owned(),
                    size: 128,
                },
            ],
        };
        let current = ReleaseVersion::parse("1.2.2").expect("current");
        let available = parse_release(release, &current)
            .expect("release")
            .expect("newer release");
        assert_eq!(available.version(), "1.2.3");
        assert_eq!(available.notes(), "notes");
        assert!(
            available
                .signature_url
                .ends_with("/AskBridge-1.2.3-SHA256SUMS.txt.sig")
        );
    }

    #[test]
    fn release_without_the_signature_asset_is_rejected() {
        let release = GithubRelease {
            tag_name: "v1.2.3".to_owned(),
            html_url:
                "https://github.com/wanghongyu666qiang/AskBridge/releases/tag/v1.2.3"
                    .to_owned(),
            body: None,
            assets: vec![
                GithubAsset {
                    name: "AskBridge-1.2.3-Setup.exe".to_owned(),
                    browser_download_url: "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-Setup.exe".to_owned(),
                    size: 4096,
                },
                GithubAsset {
                    name: "AskBridge-1.2.3-SHA256SUMS.txt".to_owned(),
                    browser_download_url: "https://github.com/wanghongyu666qiang/AskBridge/releases/download/v1.2.3/AskBridge-1.2.3-SHA256SUMS.txt".to_owned(),
                    size: 256,
                },
            ],
        };
        let current = ReleaseVersion::parse("1.2.2").expect("current");
        let error = parse_release(release, &current).expect_err("missing signature asset");
        assert!(
            error.to_string().contains("SHA256SUMS.txt.sig"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn truncates_release_notes_on_character_boundaries() {
        assert_eq!(truncate_chars("更新说明abc".to_owned(), 4), "更新说明");
    }

    #[test]
    fn downloaded_setup_must_be_an_existing_direct_child() {
        let root = tempdir().expect("root");
        let updates = root.path().join("Updates");
        fs::create_dir_all(&updates).expect("updates");
        let setup = updates.join("AskBridge-1.2.3-Setup.exe");
        fs::write(&setup, b"setup").expect("setup");
        let absolute_updates = std::path::absolute(&updates).expect("absolute updates");
        let absolute_setup = absolute_updates.join("AskBridge-1.2.3-Setup.exe");
        assert!(validate_downloaded_setup(&absolute_updates, &absolute_setup).is_ok());
        assert!(
            validate_downloaded_setup(&absolute_updates, &root.path().join("outside.exe")).is_err()
        );
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
    fn update_errors_keep_update_context() {
        let error = update_error("测试");
        assert!(error.to_string().contains("application update failed"));
    }

    #[test]
    #[ignore = "requires live GitHub access"]
    fn live_github_release_matches_the_update_contract() {
        let baseline = ReleaseVersion::parse("0.0.0").expect("baseline");
        let release = check_latest(&baseline)
            .expect("live release check")
            .expect("published release");
        assert!(release.version().split('.').count() == 3);
        assert!(release.release_url().starts_with(RELEASE_PAGE_PREFIX));
    }

    #[test]
    #[ignore = "requires live GitHub access and ASKBRIDGE_UPDATE_TEST_ROOT"]
    fn live_github_release_download_matches_sha256() {
        let root = std::env::var_os("ASKBRIDGE_UPDATE_TEST_ROOT")
            .map(PathBuf::from)
            .expect("ASKBRIDGE_UPDATE_TEST_ROOT");
        assert!(root.is_absolute());
        let baseline = ReleaseVersion::parse("0.0.0").expect("baseline");
        let release = check_latest(&baseline)
            .expect("live release check")
            .expect("published release");
        let setup = download_release(&root, &release, |_, _| {}).expect("verified setup download");
        assert!(setup.is_file());
        assert_eq!(setup.parent(), Some(root.as_path()));
        cleanup_stale_updates(&root);
        let _ = fs::remove_dir(&root);
    }
}
