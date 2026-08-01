use std::{
    io::{Read, Write},
    net::TcpStream,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use askbridge_core::{AppError, Result};
use serde::Deserialize;
use serde_json::{Value, json};
use tungstenite::{Message, client};

use super::DevToolsEndpoint;

const MAX_HTTP_RESPONSE_BYTES: u64 = 1024 * 1024;
const MAX_PROTOCOL_MESSAGES: usize = 256;

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
        let client = Self { endpoint, timeout };
        let version: VersionResponse = client.request_json("GET", "/json/version")?;
        client.verify_browser_websocket(&version.web_socket_debugger_url)?;
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
            let result = self.websocket_command(
                websocket_url,
                "Runtime.evaluate",
                Some(json!({
                    "expression": "document.readyState",
                    "returnByValue": true
                })),
                cancelled,
                attempt_timeout,
            );
            if result.as_ref().is_ok_and(is_interactive_ready_state) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn close_browser(&self, cancelled: &AtomicBool) -> Result<()> {
        self.browser_command("Browser.close", None, cancelled)
            .map(|_| ())
    }

    #[cfg(test)]
    pub fn close_managed_endpoint(
        endpoint: DevToolsEndpoint,
        timeout: Duration,
        cancelled: &AtomicBool,
    ) -> Result<()> {
        Self { endpoint, timeout }.close_browser(cancelled)
    }

    fn browser_command(
        &self,
        method: &str,
        params: Option<Value>,
        cancelled: &AtomicBool,
    ) -> Result<Value> {
        let url = self.endpoint.browser_websocket_url();
        self.websocket_command(&url, method, params, cancelled, self.timeout)
    }

    fn websocket_command(
        &self,
        url: &str,
        method: &str,
        params: Option<Value>,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value> {
        let stream = TcpStream::connect_timeout(&self.endpoint.socket_addr(), timeout)
            .map_err(|_| AppError::BrowserConnectionFailed("CDP handshake failed".to_owned()))?;
        configure_socket_timeout(&stream, timeout)?;
        let (mut socket, _) = client(url, stream)
            .map_err(|_| AppError::BrowserConnectionFailed("CDP handshake failed".to_owned()))?;

        let mut command = json!({"id": 1, "method": method});
        if let Some(params) = params {
            command["params"] = params;
        }
        socket
            .send(Message::Text(command.to_string().into()))
            .map_err(|_| AppError::BrowserProtocol("CDP command send failed".to_owned()))?;

        let deadline = Instant::now() + timeout;
        for _ in 0..MAX_PROTOCOL_MESSAGES {
            if cancelled.load(Ordering::Acquire) {
                return Err(AppError::BrowserCancelled);
            }
            if Instant::now() >= deadline {
                return Err(AppError::BrowserConnectionFailed(
                    "CDP command timed out".to_owned(),
                ));
            }
            let message = socket
                .read()
                .map_err(|_| AppError::BrowserProtocol("CDP response read failed".to_owned()))?;
            let Message::Text(text) = message else {
                continue;
            };
            let response: Value = serde_json::from_str(text.as_ref())
                .map_err(|_| AppError::BrowserProtocol("invalid CDP response".to_owned()))?;
            if response.get("id").and_then(Value::as_u64) != Some(1) {
                continue;
            }
            if response.get("error").is_some() {
                return Err(AppError::BrowserProtocol(format!(
                    "CDP method {method} was rejected"
                )));
            }
            return Ok(response.get("result").cloned().unwrap_or(Value::Null));
        }
        Err(AppError::BrowserProtocol(
            "CDP response limit exceeded".to_owned(),
        ))
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
}

fn configure_socket_timeout(stream: &TcpStream, timeout: Duration) -> Result<()> {
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|_| stream.set_write_timeout(Some(timeout)))
        .map_err(|_| AppError::BrowserConnectionFailed("CDP timeout setup failed".to_owned()))
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
    result
        .pointer("/result/value")
        .and_then(Value::as_str)
        .is_some_and(|state| matches!(state, "interactive" | "complete"))
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
mod tests {
    use super::*;

    #[test]
    fn parses_successful_http_response_without_logging_body() {
        let body = parse_http_response(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"ok\":true}",
        )
        .expect("response");
        assert_eq!(body, br#"{"ok":true}"#);
    }

    #[test]
    fn rejects_non_success_or_malformed_http_response() {
        assert!(parse_http_response(b"HTTP/1.1 500 Error\r\n\r\n").is_err());
        assert!(parse_http_response(b"not http").is_err());
    }

    #[test]
    fn honors_content_length_without_waiting_for_connection_close() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nhello worldignored";
        assert!(response_is_complete(response).expect("complete"));
        assert_eq!(
            parse_http_response(response).expect("response"),
            b"hello world"
        );
    }

    #[test]
    fn decodes_chunked_http_response() {
        let response =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert!(response_is_complete(response).expect("complete"));
        assert_eq!(
            parse_http_response(response).expect("response"),
            b"hello world"
        );
    }

    #[test]
    fn target_url_and_id_are_strictly_validated() {
        assert!(validate_page_url("http://127.0.0.1:1234/test").is_ok());
        assert!(validate_page_url("https://example.test/chat").is_ok());
        assert!(validate_page_url("file:///C:/secret").is_err());
        assert!(validate_page_url("https://example.test/\r\nHost: evil").is_err());
        assert!(validate_target_id("ABC_def-123").is_ok());
        assert!(validate_target_id("../../target").is_err());
    }

    #[test]
    fn percent_encoding_is_unambiguous() {
        assert_eq!(
            percent_encode("http://127.0.0.1:1234/a b?x=1"),
            "http%3A%2F%2F127.0.0.1%3A1234%2Fa%20b%3Fx%3D1"
        );
    }

    #[test]
    fn only_interactive_and_complete_ready_states_are_accepted() {
        assert!(!is_interactive_ready_state(&json!({
            "result": {"value": "loading"}
        })));
        assert!(is_interactive_ready_state(&json!({
            "result": {"value": "interactive"}
        })));
        assert!(is_interactive_ready_state(&json!({
            "result": {"value": "complete"}
        })));
    }

    #[test]
    fn websocket_validation_accepts_only_loopback_host_forms() {
        for url in [
            "ws://127.0.0.1:9222/devtools/browser/id",
            "ws://localhost:9222/devtools/browser/id",
            "ws://[::1]:9222/devtools/browser/id",
        ] {
            assert_eq!(
                split_loopback_websocket(url).expect("loopback"),
                (9222, "/devtools/browser/id")
            );
        }
        assert!(split_loopback_websocket("ws://192.0.2.1:9222/devtools/browser/id").is_err());
        assert!(split_loopback_websocket("wss://localhost:9222/devtools/browser/id").is_err());
    }
}
