mod http;
mod sha256;

use std::{
    collections::VecDeque,
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use askbridge_core::{AppError, Result};
use serde::Deserialize;
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{PostMessageW, WM_APP},
};

use self::{http::get_https, sha256::sha256_reader};

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
const MAX_SETUP_BYTES: usize = 128 * 1024 * 1024;
const MAX_RELEASE_NOTES_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    version: String,
    notes: String,
    release_url: String,
    setup_name: String,
    setup_url: String,
    setup_size: u64,
    checksum_url: String,
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
        let executable = std::env::current_exe().map_err(|source| {
            AppError::io(
                "locating AskBridge executable for update",
                setup_path,
                source,
            )
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
    loop {
        match commands.recv_timeout(CHECK_INTERVAL) {
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
                let event = match download_release(&update_root, &release) {
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
    let setup = single_asset(&release.assets, &expected_setup)?;
    let checksums = single_asset(&release.assets, &expected_checksums)?;
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

fn download_release(update_root: &Path, release: &AvailableUpdate) -> Result<PathBuf> {
    fs::create_dir_all(update_root)
        .map_err(|source| AppError::io("creating update cache", update_root, source))?;
    let checksum_source = get_https(&release.checksum_url, MAX_CHECKSUM_BYTES)?;
    let checksum_text = std::str::from_utf8(&checksum_source)
        .map_err(|_| update_error("SHA256SUMS 不是 UTF-8 文本"))?;
    let expected_hash = expected_checksum(checksum_text, &release.setup_name)?;
    let setup = get_https(&release.setup_url, MAX_SETUP_BYTES)?;
    if setup.len() as u64 != release.setup_size {
        return Err(update_error("更新安装包大小与 GitHub Release 不一致"));
    }
    let actual_hash = sha256_reader(Cursor::new(&setup))
        .map_err(|source| AppError::io("hashing downloaded update", update_root, source))?;
    if !actual_hash.eq_ignore_ascii_case(&expected_hash) {
        return Err(update_error("更新安装包 SHA-256 校验失败"));
    }
    persist_download(update_root, &release.setup_name, &setup)
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
        if path.is_file() && (is_setup || is_partial) {
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

fn persist_download(update_root: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf> {
    if !is_safe_file_name(name) {
        return Err(update_error("更新安装包文件名不安全"));
    }
    let destination = update_root.join(name);
    let temporary = update_root.join(format!(".{name}.{}.partial", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|source| AppError::io("creating update download", &temporary, source))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|source| AppError::io("writing update download", &temporary, source))?;
        if destination.exists() {
            fs::remove_file(&destination)
                .map_err(|source| AppError::io("replacing cached update", &destination, source))?;
        }
        fs::rename(&temporary, &destination)
            .map_err(|source| AppError::io("installing cached update", &destination, source))
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    Ok(destination)
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

    #[test]
    fn semantic_versions_order_stable_after_prerelease() {
        let prerelease = ReleaseVersion::parse("1.2.3-rc.1").expect("prerelease");
        let stable = ReleaseVersion::parse("1.2.3").expect("stable");
        let next = ReleaseVersion::parse("1.2.4").expect("next");
        assert!(prerelease < stable);
        assert!(stable < next);
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
        let path =
            persist_download(&updates, "AskBridge-1.2.3-Setup.exe", b"setup").expect("persist");
        assert_eq!(fs::read(&path).expect("read"), b"setup");
        assert!(validate_downloaded_setup(&updates, &path).is_ok());
        assert!(persist_download(&updates, "..\\outside.exe", b"bad").is_err());
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
            ],
        };
        let current = ReleaseVersion::parse("1.2.2").expect("current");
        let available = parse_release(release, &current)
            .expect("release")
            .expect("newer release");
        assert_eq!(available.version(), "1.2.3");
        assert_eq!(available.notes(), "notes");
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
        let setup = download_release(&root, &release).expect("verified setup download");
        assert!(setup.is_file());
        assert_eq!(setup.parent(), Some(root.as_path()));
        cleanup_stale_updates(&root);
        let _ = fs::remove_dir(&root);
    }
}
