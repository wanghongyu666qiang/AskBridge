use std::{
    io::Write,
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

mod attachment;
mod connection;
mod http;
mod protocol;
mod validation;

pub(crate) use attachment::FileInputResult;
use attachment::{attachment_receipt, has_new_attachment_receipt, query_acceptable_file_inputs};
use connection::{BrowserConnection, TargetSession};
use http::{parse_http_response, read_http_response};
use protocol::ProtocolSocket;
use validation::{
    exact_url_check_expression, is_interactive_ready_state, percent_encode,
    split_loopback_websocket, validate_page_url, validate_target_id,
};

#[cfg(test)]
use attachment::{AttachmentReceipt, file_input_accepts_png};
#[cfg(test)]
use http::response_is_complete;

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

#[cfg(test)]
mod tests;
