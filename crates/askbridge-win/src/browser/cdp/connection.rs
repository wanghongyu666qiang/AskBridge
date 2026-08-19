use std::{
    collections::{HashMap, VecDeque},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

use askbridge_core::{AppError, Result};
use serde_json::{Value, json};

use super::{CdpTarget, MAX_PROTOCOL_MESSAGES, ProtocolSocket, exact_url_check_expression};

pub(super) struct BrowserConnection {
    socket: ProtocolSocket,
    pub(super) sessions: HashMap<String, String>,
    pub(super) events: VecDeque<Value>,
}

impl BrowserConnection {
    pub(super) fn new(socket: ProtocolSocket) -> Self {
        Self {
            socket,
            sessions: HashMap::new(),
            events: VecDeque::new(),
        }
    }

    pub(super) fn browser_command(
        &mut self,
        method: &str,
        params: Option<Value>,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value> {
        let result = self.socket.command(method, params, cancelled, timeout);
        self.collect_events();
        result
    }

    pub(super) fn target_command(
        &mut self,
        target: &CdpTarget,
        method: &str,
        params: Option<Value>,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value> {
        let session_id = self.ensure_target_session(target, cancelled, timeout)?;
        self.command_in_session(&session_id, method, params, cancelled, timeout)
    }

    pub(super) fn ensure_target_session(
        &mut self,
        target: &CdpTarget,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<String> {
        if let Some(session_id) = self.sessions.get(&target.id) {
            return Ok(session_id.clone());
        }
        let result = self.browser_command(
            "Target.attachToTarget",
            Some(json!({"targetId": target.id, "flatten": true})),
            cancelled,
            timeout,
        )?;
        let session_id = result
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|session_id| !session_id.is_empty())
            .ok_or_else(|| {
                AppError::BrowserProtocol("Target.attachToTarget returned no session".to_owned())
            })?
            .to_owned();
        self.sessions.insert(target.id.clone(), session_id.clone());
        Ok(session_id)
    }

    fn command_in_session(
        &mut self,
        session_id: &str,
        method: &str,
        params: Option<Value>,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value> {
        let result = self
            .socket
            .command_in_session(session_id, method, params, cancelled, timeout);
        self.collect_events();
        result
    }

    fn collect_events(&mut self) {
        while let Some(event) = self.socket.events.pop_front() {
            if event.get("method").and_then(Value::as_str) == Some("Target.detachedFromTarget")
                && let Some(session_id) = event.pointer("/params/sessionId").and_then(Value::as_str)
            {
                self.sessions
                    .retain(|_, attached_session| attached_session != session_id);
            }
            if self.events.len() == MAX_PROTOCOL_MESSAGES {
                self.events.pop_front();
            }
            self.events.push_back(event);
        }
    }
}

pub(super) struct TargetSession<'a> {
    connection: &'a mut BrowserConnection,
    session_id: String,
    deadline: Instant,
}

impl<'a> TargetSession<'a> {
    pub(super) fn new(
        connection: &'a mut BrowserConnection,
        session_id: String,
        timeout: Duration,
    ) -> Self {
        Self {
            connection,
            session_id,
            deadline: Instant::now() + timeout,
        }
    }

    pub(super) fn command(
        &mut self,
        method: &str,
        params: Option<Value>,
        cancelled: &AtomicBool,
    ) -> Result<Value> {
        let timeout = self.deadline.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            return Err(AppError::BrowserConnectionFailed(
                "CDP command timed out".to_owned(),
            ));
        }
        self.connection
            .command_in_session(&self.session_id, method, params, cancelled, timeout)
    }

    pub(super) fn target_url_matches(
        &mut self,
        expected_url: &str,
        cancelled: &AtomicBool,
    ) -> Result<bool> {
        let expression = exact_url_check_expression(expected_url)?;
        let result = self.command(
            "Runtime.evaluate",
            Some(json!({
                "expression": expression,
                "returnByValue": true,
                "awaitPromise": false
            })),
            cancelled,
        )?;
        result
            .pointer("/result/value")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                AppError::BrowserProtocol("target URL verification returned no value".to_owned())
            })
    }
}
