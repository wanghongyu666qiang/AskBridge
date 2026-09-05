use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    ptr,
    sync::{OnceLock, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use askbridge_core::{AppError, Result};
use tracing::warn;
use windows_sys::Win32::{
    Foundation::GetLastError,
    Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
        WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
        WinHttpSendRequest, WinHttpSetTimeouts,
    },
};

#[cfg(test)]
use super::rules::RULE_SCHEMA_VERSION;
use super::rules::{MAX_RULE_SOURCE_BYTES, ProviderRule, ProviderRuleSet, parse_and_validate};

const RULES_URL_ENV: &str = "ASKBRIDGE_PROVIDER_RULES_URL";
const RULES_DIRECTORY: &str = "ProviderRules";
const RULES_CACHE: &str = "rules-v2.json";
const REQUEST_TIMEOUT_MS: i32 = 5_000;

static ACTIVE_RULES: OnceLock<RwLock<Option<ProviderRuleSet>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuleUpdateSource {
    BuiltIn,
    Cache,
    Remote,
}

pub(super) fn active_rule(adapter_id: &str) -> Result<Option<ProviderRule>> {
    let Some(active) = ACTIVE_RULES.get() else {
        return Ok(None);
    };
    let rules = active.read().map_err(|_| {
        AppError::InvalidPreparation("provider rule state lock was poisoned".to_owned())
    })?;
    Ok(rules
        .as_ref()
        .and_then(|rules| rules.providers.iter().find(|rule| rule.id() == adapter_id))
        .cloned())
}

/// Refreshes declarative provider rules and always falls back to cache or built-ins.
pub(crate) fn refresh_rules_from_environment(data_root: &Path) -> Result<RuleUpdateSource> {
    let cache_path = rule_cache_path(data_root)?;
    let remote_url = env::var(RULES_URL_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty());

    if let Some(remote_url) = remote_url {
        let fetched = fetch_https_json(&remote_url).and_then(|source| {
            let rules = parse_source(&source)?;
            Ok((source, rules))
        });
        match fetched {
            Ok((source, rules)) => match persist_cache(&cache_path, &source) {
                Ok(()) => {
                    install_rules(Some(rules))?;
                    return Ok(RuleUpdateSource::Remote);
                }
                Err(error) => warn!(
                    stage = "provider_rules",
                    completed = false,
                    error_kind = error.kind(),
                    "fetched provider rules could not be cached; falling back"
                ),
            },
            Err(error) => warn!(
                stage = "provider_rules",
                completed = false,
                error_kind = error.kind(),
                "remote provider rules were unusable; falling back to cache or built-ins"
            ),
        }
    }

    if let Ok(source) = fs::read(&cache_path)
        && let Ok(rules) = parse_source(&source)
    {
        install_rules(Some(rules))?;
        return Ok(RuleUpdateSource::Cache);
    }

    install_rules(None)?;
    Ok(RuleUpdateSource::BuiltIn)
}

fn install_rules(rules: Option<ProviderRuleSet>) -> Result<()> {
    let active = ACTIVE_RULES.get_or_init(|| RwLock::new(None));
    *active.write().map_err(|_| {
        AppError::InvalidPreparation("provider rule state lock was poisoned".to_owned())
    })? = rules;
    Ok(())
}

fn parse_source(source: &[u8]) -> Result<ProviderRuleSet> {
    if source.len() > MAX_RULE_SOURCE_BYTES {
        return Err(AppError::InvalidPreparation(
            "provider rule update exceeds the size limit".to_owned(),
        ));
    }
    let source = std::str::from_utf8(source).map_err(|_| {
        AppError::InvalidPreparation("provider rule update is not UTF-8 JSON".to_owned())
    })?;
    parse_and_validate(source)
}

fn rule_cache_path(data_root: &Path) -> Result<PathBuf> {
    if !data_root.is_absolute() {
        return Err(AppError::InvalidPreparation(
            "provider rule cache root must be absolute".to_owned(),
        ));
    }
    Ok(data_root.join(RULES_DIRECTORY).join(RULES_CACHE))
}

fn persist_cache(path: &Path, source: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        AppError::InvalidPreparation("provider rule cache has no parent".to_owned())
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| AppError::io("creating provider rule cache directory", parent, error))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".{RULES_CACHE}.{nonce}.tmp"));
    let previous = parent.join(format!(".{RULES_CACHE}.previous"));
    let write_result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| AppError::io("creating provider rule cache", &temporary, error))?;
        file.write_all(source)
            .and_then(|_| file.sync_all())
            .map_err(|error| AppError::io("writing provider rule cache", &temporary, error))?;
        if previous.exists() {
            fs::remove_file(&previous).map_err(|error| {
                AppError::io("removing previous provider rule cache", &previous, error)
            })?;
        }
        if path.exists() {
            fs::rename(path, &previous).map_err(|error| {
                AppError::io("preserving previous provider rule cache", path, error)
            })?;
        }
        if let Err(error) = fs::rename(&temporary, path) {
            if previous.exists() {
                let _ = fs::rename(&previous, path);
            }
            return Err(AppError::io("installing provider rule cache", path, error));
        }
        if previous.exists() {
            let _ = fs::remove_file(&previous);
        }
        Ok(())
    })();
    if temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn fetch_https_json(url: &str) -> Result<Vec<u8>> {
    let parsed = ParsedHttpsUrl::parse(url)?;
    let agent = wide("AskBridge Provider Rules/1.0");
    let host = wide(&parsed.host);
    let method = wide("GET");
    let path = wide(&parsed.path);

    // SAFETY: All strings are live, nul-terminated UTF-16 buffers for the synchronous calls.
    let session = unsafe {
        WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        )
    };
    let session = InternetHandle::new(session, "WinHttpOpen")?;
    // SAFETY: session and host are valid for this synchronous connection call.
    let connection = unsafe { WinHttpConnect(session.0, host.as_ptr(), parsed.port, 0) };
    let connection = InternetHandle::new(connection, "WinHttpConnect")?;
    // SAFETY: connection is valid and the request uses HTTPS with no optional headers/body.
    let request = unsafe {
        WinHttpOpenRequest(
            connection.0,
            method.as_ptr(),
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            WINHTTP_FLAG_SECURE,
        )
    };
    let request = InternetHandle::new(request, "WinHttpOpenRequest")?;
    // SAFETY: request is a valid WinHTTP handle; all timeout values are bounded milliseconds.
    if unsafe {
        WinHttpSetTimeouts(
            request.0,
            REQUEST_TIMEOUT_MS,
            REQUEST_TIMEOUT_MS,
            REQUEST_TIMEOUT_MS,
            REQUEST_TIMEOUT_MS,
        )
    } == 0
    {
        return Err(last_windows_error("WinHttpSetTimeouts"));
    }
    // SAFETY: No headers or request body are supplied.
    if unsafe { WinHttpSendRequest(request.0, ptr::null(), 0, ptr::null(), 0, 0, 0) } == 0 {
        return Err(last_windows_error("WinHttpSendRequest"));
    }
    // SAFETY: The synchronous request handle remains live.
    if unsafe { WinHttpReceiveResponse(request.0, ptr::null_mut()) } == 0 {
        return Err(last_windows_error("WinHttpReceiveResponse"));
    }
    let mut status = 0_u32;
    let mut status_size = u32::try_from(std::mem::size_of_val(&status)).unwrap_or(u32::MAX);
    // SAFETY: status points to writable u32 storage described by status_size.
    if unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            ptr::null(),
            (&mut status as *mut u32).cast(),
            &mut status_size,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(last_windows_error("WinHttpQueryHeaders"));
    }
    if !(200..300).contains(&status) {
        return Err(AppError::BrowserConnectionFailed(format!(
            "provider rule endpoint returned status {status}"
        )));
    }

    let mut source = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let mut read = 0_u32;
        // SAFETY: buffer is writable for its declared length and read receives the byte count.
        if unsafe {
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut read,
            )
        } == 0
        {
            return Err(last_windows_error("WinHttpReadData"));
        }
        if read == 0 {
            break;
        }
        let read = usize::try_from(read).map_err(|_| {
            AppError::BrowserConnectionFailed("provider rule response length overflow".to_owned())
        })?;
        source.extend_from_slice(&buffer[..read]);
        if source.len() > MAX_RULE_SOURCE_BYTES {
            return Err(AppError::InvalidPreparation(
                "provider rule update exceeds the size limit".to_owned(),
            ));
        }
    }
    Ok(source)
}

struct InternetHandle(*mut core::ffi::c_void);

impl InternetHandle {
    fn new(handle: *mut core::ffi::c_void, operation: &'static str) -> Result<Self> {
        if handle.is_null() {
            Err(last_windows_error(operation))
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for InternetHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: This guard owns one WinHTTP handle and closes it exactly once.
            unsafe {
                WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn last_windows_error(operation: &'static str) -> AppError {
    // SAFETY: GetLastError has no preconditions and is read immediately after a failed WinHTTP call.
    let win32_code = unsafe { GetLastError() };
    AppError::Windows {
        operation,
        win32_code,
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

struct ParsedHttpsUrl {
    host: String,
    port: u16,
    path: String,
}

impl ParsedHttpsUrl {
    fn parse(url: &str) -> Result<Self> {
        if url.len() > 4096 || url.contains(['\r', '\n', '#']) {
            return Err(AppError::InvalidPreparation(
                "provider rule URL is invalid".to_owned(),
            ));
        }
        let remainder = url.strip_prefix("https://").ok_or_else(|| {
            AppError::InvalidPreparation("provider rule URL must use HTTPS".to_owned())
        })?;
        let (authority, suffix) = remainder
            .split_once('/')
            .map_or((remainder, ""), |(authority, path)| (authority, path));
        if authority.is_empty() || authority.contains('@') || authority.starts_with('[') {
            return Err(AppError::InvalidPreparation(
                "provider rule URL authority is invalid".to_owned(),
            ));
        }
        let (host, port) =
            authority
                .rsplit_once(':')
                .map_or(Ok((authority, 443)), |(host, port)| {
                    port.parse::<u16>()
                        .ok()
                        .filter(|port| *port != 0)
                        .map(|port| (host, port))
                        .ok_or_else(|| {
                            AppError::InvalidPreparation(
                                "provider rule URL port is invalid".to_owned(),
                            )
                        })
                })?;
        if host.is_empty()
            || !host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        {
            return Err(AppError::InvalidPreparation(
                "provider rule URL host is invalid".to_owned(),
            ));
        }
        Ok(Self {
            host: host.to_owned(),
            port,
            path: format!("/{suffix}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::adapter::rules::BUILTIN_RULES;

    #[test]
    fn cache_requires_absolute_data_root_and_validated_json() {
        assert!(rule_cache_path(Path::new("relative")).is_err());
        assert!(parse_source(BUILTIN_RULES.as_bytes()).is_ok());
        assert!(parse_source(br#"{"schema_version":99,"providers":[]}"#).is_err());
    }

    #[test]
    fn invalid_update_never_replaces_last_valid_cache() {
        let root = tempdir().expect("root");
        let cache = rule_cache_path(root.path()).expect("cache path");
        let valid = BUILTIN_RULES.as_bytes();
        let rules = parse_source(valid).expect("valid rules");
        assert_eq!(rules.schema_version, RULE_SCHEMA_VERSION);
        persist_cache(&cache, valid).expect("initial cache");

        let invalid = br#"{"schema_version":99,"providers":[]}"#;
        assert!(parse_source(invalid).is_err());
        assert_eq!(fs::read(cache).expect("cache"), valid);
    }

    #[test]
    fn remote_url_accepts_only_bounded_https_authorities() {
        let parsed = ParsedHttpsUrl::parse("https://example.test/rules/v2.json?channel=stable")
            .expect("HTTPS URL");
        assert_eq!(parsed.host, "example.test");
        assert_eq!(parsed.port, 443);
        assert_eq!(parsed.path, "/rules/v2.json?channel=stable");
        assert!(ParsedHttpsUrl::parse("http://example.test/rules.json").is_err());
        assert!(ParsedHttpsUrl::parse("https://user@example.test/rules.json").is_err());
        assert!(ParsedHttpsUrl::parse("https://example.test/rules.json#fragment").is_err());
    }
}
