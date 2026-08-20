use std::{
    collections::HashSet,
    io::{Read, Write},
    net::TcpStream,
    path::Path,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use askbridge_core::{AppError, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use super::DevToolsEndpoint;

mod connection;
mod protocol;

use connection::{BrowserConnection, TargetSession};
use protocol::ProtocolSocket;

const MAX_HTTP_RESPONSE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CdpTarget {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    pub web_socket_debugger_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionResponse {
    web_socket_debugger_url: String,
}

pub struct CdpClient {
    endpoint: DevToolsEndpoint,
    timeout: Duration,
    connection: Mutex<Option<BrowserConnection>>,
}

impl CdpClient {
    pub fn connect(
        endpoint: DevToolsEndpoint,
        timeout: Duration,
        cancelled: &AtomicBool,
    ) -> Result<Self> {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        let client = Self {
            endpoint,
            timeout,
            connection: Mutex::new(None),
        };
        let version: VersionResponse = client.request_json("GET", "/json/version")?;
        client.verify_browser_websocket(&version.web_socket_debugger_url)?;
        let socket = ProtocolSocket::connect(
            &client.endpoint,
            &version.web_socket_debugger_url,
            cancelled,
            timeout,
        )?;
        *client.lock_connection()? = Some(BrowserConnection::new(socket));
        client.browser_command("Browser.getVersion", None, cancelled)?;
        Ok(client)
    }

    pub fn list_targets(&self) -> Result<Vec<CdpTarget>> {
        self.request_json("GET", "/json/list")
    }

    pub fn create_target(&self, url: &str) -> Result<CdpTarget> {
        validate_page_url(url)?;
        let path = format!("/json/new?{}", percent_encode(url));
        self.request_json("PUT", &path)
    }

    pub fn activate_target(&self, target_id: &str) -> Result<()> {
        validate_target_id(target_id)?;
        let path = format!("/json/activate/{}", percent_encode(target_id));
        let _ = self.request("GET", &path)?;
        Ok(())
    }

    pub fn wait_until_ready(
        &self,
        target: &CdpTarget,
        timeout: Duration,
        cancelled: &AtomicBool,
    ) -> Result<()> {
        let websocket_url = target
            .web_socket_debugger_url
            .as_deref()
            .ok_or(AppError::TargetNotFound)?;
        self.verify_target_websocket(target, websocket_url)?;
        let deadline = Instant::now() + timeout;
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(AppError::BrowserCancelled);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(AppError::TargetTimeout);
            }
            let attempt_timeout = (deadline - now).min(Duration::from_millis(750));
            let result = self.target_command(
                target,
                "Runtime.evaluate",
                Some(json!({
                    "expression": "({ ready: document.readyState, url: location.href })",
                    "returnByValue": true
                })),
                cancelled,
                attempt_timeout,
            )?;
            if is_interactive_ready_state(&result) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn close_browser(&self, cancelled: &AtomicBool) -> Result<()> {
        self.browser_command("Browser.close", None, cancelled)
            .map(|_| ())
    }

    pub(crate) fn evaluate_in_target(
        &self,
        target: &CdpTarget,
        expression: &str,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value> {
        self.target_command(
            target,
            "Runtime.evaluate",
            Some(json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": false
            })),
            cancelled,
            timeout,
        )
    }

    pub(crate) fn target_url_matches(
        &self,
        target: &CdpTarget,
        expected_url: &str,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<bool> {
        let expression = exact_url_check_expression(expected_url)?;
        let result = self.target_command(
            target,
            "Runtime.evaluate",
            Some(json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": false
            })),
            cancelled,
            timeout,
        )?;
        result
            .pointer("/result/value")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                AppError::BrowserProtocol("target URL verification returned no value".to_owned())
            })
    }

    pub(crate) fn set_file_input(
        &self,
        target: &CdpTarget,
        expected_url: &str,
        file_path: &Path,
        preferred_selectors: &[String],
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<FileInputResult> {
        self.validate_target(target)?;
        let mut connection = self.lock_active_connection()?;
        let session_id = connection.ensure_target_session(target, cancelled, timeout)?;
        let mut session =
            TargetSession::new(&mut connection, target.id.clone(), session_id, timeout);
        if !session.target_url_matches(expected_url, cancelled)? {
            return Ok(FileInputResult::NavigationChanged);
        }
        let document = session.command(
            "DOM.getDocument",
            Some(json!({"depth": 0, "pierce": true})),
            cancelled,
        )?;
        let root_id = document
            .pointer("/root/nodeId")
            .and_then(Value::as_i64)
            .ok_or_else(|| AppError::BrowserProtocol("DOM root has no node id".to_owned()))?;
        let mut candidates =
            query_acceptable_file_inputs(&mut session, root_id, preferred_selectors, cancelled)?;
        if candidates.is_empty() {
            candidates = query_acceptable_file_inputs(
                &mut session,
                root_id,
                &["input[type=file]".to_owned()],
                cancelled,
            )?;
        }
        let node_id = match candidates.as_slice() {
            [] => return Ok(FileInputResult::NotFound),
            [node_id] => *node_id,
            [_, _, ..] => return Ok(FileInputResult::Ambiguous),
        };
        if !session.target_url_matches(expected_url, cancelled)? {
            return Ok(FileInputResult::NavigationChanged);
        }
        let file_path = file_path.to_str().ok_or_else(|| {
            AppError::InvalidPreparation("temporary image path is not Unicode".to_owned())
        })?;
        let resolved = session.command(
            "DOM.resolveNode",
            Some(json!({"nodeId": node_id})),
            cancelled,
        )?;
        let object_id = resolved
            .pointer("/object/objectId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AppError::BrowserProtocol("file input could not be resolved".to_owned())
            })?
            .to_owned();
        let file_name = file_path
            .rsplit(['\\', '/'])
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::InvalidPreparation("temporary image has no file name".to_owned())
            })?;
        let baseline = attachment_receipt(&mut session, &object_id, file_name, cancelled)?;
        session.command(
            "DOM.setFileInputFiles",
            Some(json!({"files": [file_path], "nodeId": node_id})),
            cancelled,
        )?;
        session.command(
            "Runtime.callFunctionOn",
            Some(json!({
                "objectId": object_id,
                "functionDeclaration": "function() { this.dispatchEvent(new Event('input', { bubbles: true })); this.dispatchEvent(new Event('change', { bubbles: true })); }"
            })),
            cancelled,
        )?;

        let deadline = Instant::now() + timeout;
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(AppError::BrowserCancelled);
            }
            if !session.target_url_matches(expected_url, cancelled)? {
                return Ok(FileInputResult::NavigationChanged);
            }
            let receipt = attachment_receipt(&mut session, &object_id, file_name, cancelled)?;
            if has_new_attachment_receipt(baseline, receipt) {
                return Ok(FileInputResult::Prepared);
            }
            if Instant::now() >= deadline {
                return Ok(FileInputResult::VerificationFailed);
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn target_command(
        &self,
        target: &CdpTarget,
        method: &str,
        params: Option<Value>,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value> {
        self.validate_target(target)?;
        let mut connection = self.lock_active_connection()?;
        connection.target_command(target, method, params, cancelled, timeout)
    }

    #[cfg(test)]
    pub fn close_managed_endpoint(
        endpoint: DevToolsEndpoint,
        timeout: Duration,
        cancelled: &AtomicBool,
    ) -> Result<()> {
        Self::connect(endpoint, timeout, cancelled)?.close_browser(cancelled)
    }

    fn browser_command(
        &self,
        method: &str,
        params: Option<Value>,
        cancelled: &AtomicBool,
    ) -> Result<Value> {
        let url = self.endpoint.browser_websocket_url();
        self.verify_browser_websocket(&url)?;
        self.lock_active_connection()?
            .browser_command(method, params, cancelled, self.timeout)
    }

    fn request_json<T: for<'de> Deserialize<'de>>(&self, method: &str, path: &str) -> Result<T> {
        let body = self.request(method, path)?;
        serde_json::from_slice(&body)
            .map_err(|_| AppError::BrowserProtocol("invalid debugging JSON".to_owned()))
    }

    fn request(&self, method: &str, path: &str) -> Result<Vec<u8>> {
        if !matches!(method, "GET" | "PUT") {
            return Err(AppError::BrowserProtocol(
                "unsupported debugging request method".to_owned(),
            ));
        }
        let path = self.endpoint.http_path(path)?;
        let mut stream = TcpStream::connect_timeout(&self.endpoint.socket_addr(), self.timeout)
            .map_err(|_| {
                AppError::BrowserConnectionFailed(
                    "local debugging endpoint did not accept a connection".to_owned(),
                )
            })?;
        stream
            .set_read_timeout(Some(self.timeout))
            .and_then(|_| stream.set_write_timeout(Some(self.timeout)))
            .map_err(|_| {
                AppError::BrowserConnectionFailed("local debugging timeout setup failed".to_owned())
            })?;

        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            self.endpoint.socket_addr().port()
        );
        stream.write_all(request.as_bytes()).map_err(|_| {
            AppError::BrowserConnectionFailed("local debugging request failed".to_owned())
        })?;
        let response = read_http_response(&mut stream)?;
        parse_http_response(&response)
    }

    fn verify_browser_websocket(&self, reported: &str) -> Result<()> {
        let managed_websocket_url = self.endpoint.browser_websocket_url();
        let expected_suffix = managed_websocket_url
            .split_once("/devtools/browser/")
            .map(|(_, suffix)| suffix)
            .ok_or_else(|| {
                AppError::BrowserProtocol("invalid managed browser endpoint".to_owned())
            })?;
        let (port, path) = split_loopback_websocket(reported)?;
        let suffix = path
            .strip_prefix("/devtools/browser/")
            .ok_or_else(|| AppError::BrowserProtocol("invalid browser websocket".to_owned()))?;
        if port != self.endpoint.socket_addr().port() || suffix != expected_suffix {
            return Err(AppError::BrowserConnectionFailed(
                "debugging endpoint did not match the managed profile".to_owned(),
            ));
        }
        Ok(())
    }

    fn verify_target_websocket(&self, target: &CdpTarget, reported: &str) -> Result<()> {
        validate_target_id(&target.id)?;
        let (port, path) = split_loopback_websocket(reported)?;
        if port != self.endpoint.socket_addr().port()
            || path != format!("/devtools/page/{}", target.id)
        {
            return Err(AppError::BrowserConnectionFailed(
                "target endpoint did not match the managed browser".to_owned(),
            ));
        }
        Ok(())
    }

    fn validate_target(&self, target: &CdpTarget) -> Result<()> {
        let websocket_url = target
            .web_socket_debugger_url
            .as_deref()
            .ok_or(AppError::TargetNotFound)?;
        self.verify_target_websocket(target, websocket_url)
    }

    fn lock_connection(&self) -> Result<MutexGuard<'_, Option<BrowserConnection>>> {
        self.connection.lock().map_err(|_| {
            AppError::BrowserProtocol("persistent CDP connection lock was poisoned".to_owned())
        })
    }

    fn lock_active_connection(&self) -> Result<ActiveConnectionGuard<'_>> {
        let guard = self.lock_connection()?;
        if guard.is_none() {
            return Err(AppError::BrowserConnectionFailed(
                "persistent CDP connection is not available".to_owned(),
            ));
        }
        Ok(ActiveConnectionGuard { guard })
    }
}

struct ActiveConnectionGuard<'a> {
    guard: MutexGuard<'a, Option<BrowserConnection>>,
}

impl std::ops::Deref for ActiveConnectionGuard<'_> {
    type Target = BrowserConnection;

    fn deref(&self) -> &Self::Target {
        self.guard.as_ref().expect("active connection")
    }
}

impl std::ops::DerefMut for ActiveConnectionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard.as_mut().expect("active connection")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AttachmentReceipt {
    file_count: u64,
    named_count: u64,
    preview_count: u64,
    busy_count: u64,
}

fn has_new_attachment_receipt(baseline: AttachmentReceipt, current: AttachmentReceipt) -> bool {
    current.file_count > 0
        && current.busy_count == 0
        && (current.named_count > baseline.named_count
            || current.preview_count > baseline.preview_count)
}

fn attachment_receipt(
    socket: &mut TargetSession<'_>,
    object_id: &str,
    file_name: &str,
    cancelled: &AtomicBool,
) -> Result<AttachmentReceipt> {
    let value = socket.command(
        "Runtime.callFunctionOn",
        Some(json!({
            "objectId": object_id,
            "functionDeclaration": r#"function(expectedName) {
                const visible = (element) => {
                    const style = getComputedStyle(element);
                    const rect = element.getBoundingClientRect();
                    return style.display !== 'none' && style.visibility !== 'hidden' && rect.width > 0 && rect.height > 0;
                };
                const root = this.closest('form') || this.parentElement || document.body;
                const expected = String(expectedName || '').toLowerCase();
                let named = 0;
                let previews = 0;
                let busy = 0;
                for (const element of root.querySelectorAll('*')) {
                    if (!visible(element)) continue;
                    const attributes = [
                        element.getAttribute('aria-label'),
                        element.getAttribute('title'),
                        element.getAttribute('data-testid')
                    ].filter(Boolean).join(' ').toLowerCase();
                    const text = (element.textContent || '').trim().toLowerCase();
                    if (expected && (attributes.includes(expected) || text.includes(expected))) named++;
                    if (element.tagName === 'IMG') {
                        const source = String(element.getAttribute('src') || '');
                        if (source.startsWith('blob:') || source.startsWith('data:image/')) previews++;
                    }
                    const state = String(element.getAttribute('data-state') || '').toLowerCase();
                    if (element.getAttribute('aria-busy') === 'true' || element.getAttribute('role') === 'progressbar' || state === 'uploading' || state === 'pending') busy++;
                }
                return {
                    fileCount: this.files ? this.files.length : 0,
                    namedCount: named,
                    previewCount: previews,
                    busyCount: busy
                };
            }"#,
            "arguments": [{"value": file_name}],
            "returnByValue": true
        })),
        cancelled,
    )?;
    let result = value
        .pointer("/result/value")
        .ok_or_else(|| AppError::BrowserProtocol("attachment receipt is missing".to_owned()))?;
    let count = |name: &str| {
        result
            .get(name)
            .and_then(Value::as_u64)
            .ok_or_else(|| AppError::BrowserProtocol(format!("attachment receipt has no {name}")))
    };
    Ok(AttachmentReceipt {
        file_count: count("fileCount")?,
        named_count: count("namedCount")?,
        preview_count: count("previewCount")?,
        busy_count: count("busyCount")?,
    })
}

fn query_acceptable_file_inputs(
    socket: &mut TargetSession<'_>,
    root_id: i64,
    selectors: &[String],
    cancelled: &AtomicBool,
) -> Result<Vec<i64>> {
    let mut node_ids = HashSet::new();
    for selector in selectors {
        let query = socket.command(
            "DOM.querySelectorAll",
            Some(json!({"nodeId": root_id, "selector": selector})),
            cancelled,
        )?;
        let queried = query
            .get("nodeIds")
            .and_then(Value::as_array)
            .ok_or_else(|| AppError::BrowserProtocol("file input query failed".to_owned()))?;
        for node_id in queried.iter().filter_map(Value::as_i64) {
            node_ids.insert(node_id);
        }
    }
    let mut candidates = Vec::new();
    for node_id in node_ids {
        let attributes = socket.command(
            "DOM.getAttributes",
            Some(json!({"nodeId": node_id})),
            cancelled,
        )?;
        if file_input_accepts_png(&attributes) {
            candidates.push(node_id);
        }
    }
    candidates.sort_unstable();
    Ok(candidates)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileInputResult {
    Prepared,
    NotFound,
    Ambiguous,
    VerificationFailed,
    NavigationChanged,
}

fn file_input_accepts_png(result: &Value) -> bool {
    let Some(attributes) = result.get("attributes").and_then(Value::as_array) else {
        return false;
    };
    let pairs: Vec<(&str, &str)> = attributes
        .chunks_exact(2)
        .filter_map(|pair| Some((pair[0].as_str()?, pair[1].as_str()?)))
        .collect();
    if pairs
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("disabled"))
    {
        return false;
    }
    pairs
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("accept"))
        .is_none_or(|(_, accept)| {
            accept.trim().is_empty()
                || accept.split(',').any(|value| {
                    matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "image/*" | "image/png" | ".png"
                    )
                })
        })
}

fn exact_url_check_expression(expected_url: &str) -> Result<String> {
    let expected_url = serde_json::to_string(expected_url).map_err(|_| {
        AppError::InvalidPreparation("expected target URL could not be encoded".to_owned())
    })?;
    Ok(format!("location.href === {expected_url}"))
}

fn split_loopback_websocket(url: &str) -> Result<(u16, &str)> {
    let remainder = url.strip_prefix("ws://").ok_or_else(|| {
        AppError::BrowserConnectionFailed("debugging endpoint is not loopback".to_owned())
    })?;
    let (authority, path) = remainder
        .split_once('/')
        .ok_or_else(|| AppError::BrowserProtocol("debugging websocket has no path".to_owned()))?;
    let (host, port) = if let Some(ipv6) = authority.strip_prefix('[') {
        let (host, port) = ipv6.split_once("]:").ok_or_else(|| {
            AppError::BrowserProtocol(
                "debugging websocket has an invalid IPv6 authority".to_owned(),
            )
        })?;
        (host, port)
    } else {
        authority.rsplit_once(':').ok_or_else(|| {
            AppError::BrowserProtocol("debugging websocket has no port".to_owned())
        })?
    };
    if !matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    ) {
        return Err(AppError::BrowserConnectionFailed(
            "debugging endpoint is not loopback".to_owned(),
        ));
    }
    let port = port
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| {
            AppError::BrowserProtocol("debugging websocket has an invalid port".to_owned())
        })?;
    Ok((port, &url[url.len() - path.len() - 1..]))
}

fn is_interactive_ready_state(result: &Value) -> bool {
    let Some(value) = result.pointer("/result/value") else {
        return false;
    };
    let ready = value.get("ready").and_then(Value::as_str);
    let url = value.get("url").and_then(Value::as_str);
    ready.is_some_and(|state| matches!(state, "interactive" | "complete"))
        && url.is_some_and(|url| validate_page_url(url).is_ok())
}

fn parse_http_response(response: &[u8]) -> Result<Vec<u8>> {
    let delimiter = b"\r\n\r\n";
    let header_end = response
        .windows(delimiter.len())
        .position(|window| window == delimiter)
        .ok_or_else(|| AppError::BrowserProtocol("invalid debugging HTTP response".to_owned()))?;
    let headers = std::str::from_utf8(&response[..header_end])
        .map_err(|_| AppError::BrowserProtocol("invalid debugging HTTP headers".to_owned()))?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(|| AppError::BrowserProtocol("missing debugging HTTP status".to_owned()))?;
    if !(200..300).contains(&status) {
        return Err(AppError::BrowserProtocol(format!(
            "debugging HTTP request returned status {status}"
        )));
    }
    let body = &response[header_end + delimiter.len()..];
    if headers.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value
                    .split(',')
                    .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        })
    }) {
        return decode_chunked_body(body);
    }
    if let Some(content_length) = content_length(headers)? {
        if body.len() < content_length {
            return Err(AppError::BrowserProtocol(
                "incomplete debugging HTTP body".to_owned(),
            ));
        }
        return Ok(body[..content_length].to_vec());
    }
    Ok(body.to_vec())
}

fn read_http_response(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut response = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                response.extend_from_slice(&buffer[..read]);
                if response.len() as u64 > MAX_HTTP_RESPONSE_BYTES {
                    return Err(AppError::BrowserProtocol(
                        "debugging response exceeded size limit".to_owned(),
                    ));
                }
                if response_is_complete(&response)? {
                    break;
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) && response_is_complete(&response)? =>
            {
                break;
            }
            Err(_) => {
                return Err(AppError::BrowserConnectionFailed(
                    "local debugging response failed".to_owned(),
                ));
            }
        }
    }
    Ok(response)
}

fn response_is_complete(response: &[u8]) -> Result<bool> {
    let delimiter = b"\r\n\r\n";
    let Some(header_end) = response
        .windows(delimiter.len())
        .position(|window| window == delimiter)
    else {
        return Ok(false);
    };
    let headers = std::str::from_utf8(&response[..header_end])
        .map_err(|_| AppError::BrowserProtocol("invalid debugging HTTP headers".to_owned()))?;
    let body = &response[header_end + delimiter.len()..];
    if headers.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value
                    .split(',')
                    .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        })
    }) {
        return Ok(chunked_body_length(body)?.is_some());
    }
    Ok(content_length(headers)?.is_some_and(|length| body.len() >= length))
}

fn content_length(headers: &str) -> Result<Option<usize>> {
    let values: Vec<&str> = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim())
        .collect();
    match values.as_slice() {
        [] => Ok(None),
        [value] => value
            .parse::<usize>()
            .ok()
            .filter(|length| *length as u64 <= MAX_HTTP_RESPONSE_BYTES)
            .map(Some)
            .ok_or_else(|| {
                AppError::BrowserProtocol("invalid debugging content length".to_owned())
            }),
        _ => Err(AppError::BrowserProtocol(
            "ambiguous debugging content length".to_owned(),
        )),
    }
}

fn decode_chunked_body(body: &[u8]) -> Result<Vec<u8>> {
    let Some(length) = chunked_body_length(body)? else {
        return Err(AppError::BrowserProtocol(
            "incomplete chunked debugging body".to_owned(),
        ));
    };
    let mut decoded = Vec::new();
    let mut cursor = 0usize;
    while cursor < length {
        let line_end = find_crlf(body, cursor).ok_or_else(|| {
            AppError::BrowserProtocol("invalid chunked debugging body".to_owned())
        })?;
        let size_text = std::str::from_utf8(&body[cursor..line_end])
            .ok()
            .and_then(|line| line.split(';').next())
            .ok_or_else(|| AppError::BrowserProtocol("invalid debugging chunk size".to_owned()))?;
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| AppError::BrowserProtocol("invalid debugging chunk size".to_owned()))?;
        cursor = line_end + 2;
        if size == 0 {
            return Ok(decoded);
        }
        let end = cursor
            .checked_add(size)
            .ok_or_else(|| AppError::BrowserProtocol("debugging chunk size overflow".to_owned()))?;
        if end + 2 > body.len() || &body[end..end + 2] != b"\r\n" {
            return Err(AppError::BrowserProtocol(
                "invalid chunked debugging body".to_owned(),
            ));
        }
        decoded.extend_from_slice(&body[cursor..end]);
        if decoded.len() as u64 > MAX_HTTP_RESPONSE_BYTES {
            return Err(AppError::BrowserProtocol(
                "debugging response exceeded size limit".to_owned(),
            ));
        }
        cursor = end + 2;
    }
    Err(AppError::BrowserProtocol(
        "invalid chunked debugging body".to_owned(),
    ))
}

fn chunked_body_length(body: &[u8]) -> Result<Option<usize>> {
    let mut cursor = 0usize;
    loop {
        let Some(line_end) = find_crlf(body, cursor) else {
            return Ok(None);
        };
        let size_text = std::str::from_utf8(&body[cursor..line_end])
            .ok()
            .and_then(|line| line.split(';').next())
            .ok_or_else(|| AppError::BrowserProtocol("invalid debugging chunk size".to_owned()))?;
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| AppError::BrowserProtocol("invalid debugging chunk size".to_owned()))?;
        cursor = line_end + 2;
        if size == 0 {
            if body.len() >= cursor + 2 && &body[cursor..cursor + 2] == b"\r\n" {
                return Ok(Some(cursor + 2));
            }
            return Ok(None);
        }
        let end = cursor
            .checked_add(size)
            .ok_or_else(|| AppError::BrowserProtocol("debugging chunk size overflow".to_owned()))?;
        if body.len() < end + 2 {
            return Ok(None);
        }
        if &body[end..end + 2] != b"\r\n" {
            return Err(AppError::BrowserProtocol(
                "invalid chunked debugging body".to_owned(),
            ));
        }
        cursor = end + 2;
    }
}

fn find_crlf(bytes: &[u8], start: usize) -> Option<usize> {
    bytes
        .get(start..)?
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|position| start + position)
}

fn validate_page_url(url: &str) -> Result<()> {
    if url.len() > 4096
        || url.contains(['\r', '\n'])
        || !(url.starts_with("http://127.0.0.1:")
            || url.starts_with("https://")
            || url.starts_with("http://localhost:"))
    {
        return Err(AppError::BrowserProtocol(
            "target URL is not allowed".to_owned(),
        ));
    }
    Ok(())
}

fn validate_target_id(target_id: &str) -> Result<()> {
    if target_id.is_empty()
        || target_id.len() > 256
        || !target_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(AppError::BrowserProtocol(
            "invalid target identifier".to_owned(),
        ));
    }
    Ok(())
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests;
