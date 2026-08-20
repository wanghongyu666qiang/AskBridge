use std::{
    io::ErrorKind,
    net::TcpStream,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use askbridge_core::{AppError, Result};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket, client};

use super::DevToolsEndpoint;

pub(super) const MAX_PROTOCOL_MESSAGES: usize = 256;
const PROTOCOL_READ_SLICE: Duration = Duration::from_millis(50);

pub(super) struct ProtocolSocket {
    socket: WebSocket<TcpStream>,
    next_id: u64,
    pub(super) events: std::collections::VecDeque<Value>,
    events_overflowed: bool,
}

impl ProtocolSocket {
    pub(super) fn connect(
        endpoint: &DevToolsEndpoint,
        url: &str,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<Self> {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        let stream = TcpStream::connect_timeout(&endpoint.socket_addr(), timeout)
            .map_err(|_| AppError::BrowserConnectionFailed("CDP handshake failed".to_owned()))?;
        configure_socket_timeout(&stream, timeout)?;
        let (socket, _) = client(url, stream)
            .map_err(|_| AppError::BrowserConnectionFailed("CDP handshake failed".to_owned()))?;
        Ok(Self {
            socket,
            next_id: 1,
            events: std::collections::VecDeque::new(),
            events_overflowed: false,
        })
    }

    pub(super) fn command(
        &mut self,
        method: &str,
        params: Option<Value>,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value> {
        self.command_with_session(method, params, None, cancelled, timeout)
    }

    pub(super) fn command_in_session(
        &mut self,
        session_id: &str,
        method: &str,
        params: Option<Value>,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value> {
        self.command_with_session(method, params, Some(session_id), cancelled, timeout)
    }

    pub(super) fn take_event_overflowed(&mut self) -> bool {
        std::mem::take(&mut self.events_overflowed)
    }

    fn command_with_session(
        &mut self,
        method: &str,
        params: Option<Value>,
        session_id: Option<&str>,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value> {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        if timeout.is_zero() {
            return Err(AppError::BrowserConnectionFailed(
                "CDP command timed out".to_owned(),
            ));
        }
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            AppError::BrowserConnectionFailed("CDP command timeout is too large".to_owned())
        })?;
        let command_id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or_else(|| {
            AppError::BrowserConnectionFailed("CDP command id exhausted".to_owned())
        })?;
        let mut command = json!({"id": command_id, "method": method});
        if let Some(params) = params {
            command["params"] = params;
        }
        if let Some(session_id) = session_id {
            command["sessionId"] = Value::String(session_id.to_owned());
        }
        configure_socket_write_timeout(self.socket.get_ref(), timeout)?;
        self.socket
            .send(Message::Text(command.to_string().into()))
            .map_err(|_| AppError::BrowserProtocol("CDP command send failed".to_owned()))?;

        let mut messages_read = 0;
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(AppError::BrowserCancelled);
            }
            if messages_read == MAX_PROTOCOL_MESSAGES {
                self.events_overflowed = true;
                return Err(AppError::BrowserProtocol(
                    "CDP response limit exceeded".to_owned(),
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AppError::BrowserConnectionFailed(
                    "CDP command timed out".to_owned(),
                ));
            }
            let read_timeout = remaining.min(PROTOCOL_READ_SLICE);
            configure_socket_read_timeout(self.socket.get_ref(), read_timeout)?;
            let message = match self.socket.read() {
                Ok(message) => {
                    messages_read += 1;
                    message
                }
                Err(tungstenite::Error::Io(error))
                    if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) =>
                {
                    continue;
                }
                Err(_) if cancelled.load(Ordering::Acquire) => {
                    return Err(AppError::BrowserCancelled);
                }
                Err(_) if Instant::now() >= deadline => {
                    return Err(AppError::BrowserConnectionFailed(
                        "CDP command timed out".to_owned(),
                    ));
                }
                Err(_) => {
                    return Err(AppError::BrowserProtocol(
                        "CDP response read failed".to_owned(),
                    ));
                }
            };
            let Message::Text(text) = message else {
                continue;
            };
            let response: Value = serde_json::from_str(text.as_ref())
                .map_err(|_| AppError::BrowserProtocol("invalid CDP response".to_owned()))?;
            if response.get("id").and_then(Value::as_u64) != Some(command_id) {
                if response.get("method").and_then(Value::as_str).is_some() {
                    if self.events.len() == MAX_PROTOCOL_MESSAGES {
                        self.events.pop_front();
                        self.events_overflowed = true;
                    }
                    self.events.push_back(response);
                }
                continue;
            }
            if let Some(error) = response.get("error") {
                let detail = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown protocol error");
                return Err(AppError::BrowserProtocol(format!(
                    "CDP method {method} was rejected: {detail}"
                )));
            }
            return Ok(response.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

fn configure_socket_timeout(stream: &TcpStream, timeout: Duration) -> Result<()> {
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|_| stream.set_write_timeout(Some(timeout)))
        .map_err(|_| AppError::BrowserConnectionFailed("CDP timeout setup failed".to_owned()))
}

fn configure_socket_read_timeout(stream: &TcpStream, timeout: Duration) -> Result<()> {
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|_| AppError::BrowserConnectionFailed("CDP timeout setup failed".to_owned()))
}

fn configure_socket_write_timeout(stream: &TcpStream, timeout: Duration) -> Result<()> {
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|_| AppError::BrowserConnectionFailed("CDP timeout setup failed".to_owned()))
}
