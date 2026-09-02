use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use askbridge_core::{AppError, Result};
use serde_json::{Value, json};

use super::protocol::{MAX_PROTOCOL_MESSAGES, ProtocolSocket};
use super::{CdpTarget, exact_url_check_expression};

pub(super) struct BrowserConnection {
    socket: ProtocolSocket,
    pub(super) sessions: HashMap<String, String>,
    pub(super) events: VecDeque<Value>,
    detached_targets: HashSet<String>,
    protocol_state_uncertain: bool,
}

impl BrowserConnection {
    pub(super) fn new(socket: ProtocolSocket) -> Self {
        Self {
            socket,
            sessions: HashMap::new(),
            events: VecDeque::new(),
            detached_targets: HashSet::new(),
            protocol_state_uncertain: false,
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
        let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
            AppError::BrowserConnectionFailed("CDP target timeout is too large".to_owned())
        })?;
        let remaining = || {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                Err(AppError::TargetTimeout)
            } else {
                Ok(remaining)
            }
        };
        let session_id = self.ensure_target_session(target, cancelled, remaining()?)?;
        self.command_in_session(
            &target.id,
            &session_id,
            method,
            params,
            cancelled,
            remaining()?,
        )
    }

    pub(super) fn ensure_target_session(
        &mut self,
        target: &CdpTarget,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<String> {
        if self.protocol_state_uncertain {
            return Err(AppError::BrowserProtocol(
                "CDP event overflow made target session state uncertain".to_owned(),
            ));
        }
        if self.detached_targets.contains(&target.id) {
            return Err(AppError::TargetNotFound);
        }
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
        if self.detached_targets.contains(&target.id)
            || self
                .events
                .iter()
                .any(|event| detached_event_matches(event, &target.id, &session_id))
        {
            self.invalidate_target(&target.id);
            return Err(AppError::BrowserProtocol(
                "target detached during session attach".to_owned(),
            ));
        }
        self.sessions.insert(target.id.clone(), session_id.clone());
        Ok(session_id)
    }

    fn command_in_session(
        &mut self,
        target_id: &str,
        session_id: &str,
        method: &str,
        params: Option<Value>,
        cancelled: &AtomicBool,
        timeout: Duration,
    ) -> Result<Value> {
        if !self.session_is_active(target_id, session_id) {
            return Err(AppError::BrowserProtocol(
                "CDP target session is no longer active".to_owned(),
            ));
        }
        let result = self
            .socket
            .command_in_session(session_id, method, params, cancelled, timeout);
        self.collect_events();
        if result.as_ref().is_err_and(is_session_invalid_error) {
            self.invalidate_target(target_id);
        }
        result
    }

    fn collect_events(&mut self) {
        let event_overflowed = self.socket.take_event_overflowed();
        while let Some(event) = self.socket.events.pop_front() {
            self.collect_detached_event(&event);
            if self.events.len() == MAX_PROTOCOL_MESSAGES {
                self.events.pop_front();
            }
            self.events.push_back(event);
        }
        if event_overflowed {
            self.invalidate_all_targets();
            self.protocol_state_uncertain = true;
        }
    }

    fn collect_detached_event(&mut self, event: &Value) {
        if event.get("method").and_then(Value::as_str) != Some("Target.detachedFromTarget") {
            return;
        }
        let Some(session_id) = event.pointer("/params/sessionId").and_then(Value::as_str) else {
            return;
        };
        if let Some(target_id) = event.pointer("/params/targetId").and_then(Value::as_str) {
            self.invalidate_target(target_id);
        } else {
            self.invalidate_session(session_id);
        }
    }

    fn invalidate_target(&mut self, target_id: &str) {
        self.sessions.remove(target_id);
        if self.detached_targets.len() == MAX_PROTOCOL_MESSAGES {
            self.detached_targets.clear();
            self.protocol_state_uncertain = true;
        }
        self.detached_targets.insert(target_id.to_owned());
    }

    fn invalidate_session(&mut self, session_id: &str) {
        let target_ids: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, attached_session)| attached_session.as_str() == session_id)
            .map(|(target_id, _)| target_id.clone())
            .collect();
        for target_id in target_ids {
            self.invalidate_target(&target_id);
        }
    }

    fn invalidate_all_targets(&mut self) {
        let target_ids: Vec<String> = self.sessions.keys().cloned().collect();
        for target_id in target_ids {
            self.invalidate_target(&target_id);
        }
    }

    fn session_is_active(&self, target_id: &str, session_id: &str) -> bool {
        !self.detached_targets.contains(target_id)
            && self
                .sessions
                .get(target_id)
                .is_some_and(|attached_session| attached_session == session_id)
    }
}

pub(super) struct TargetSession<'a> {
    connection: &'a mut BrowserConnection,
    target_id: String,
    session_id: String,
    deadline: Option<Instant>,
}

impl<'a> TargetSession<'a> {
    pub(super) fn new(
        connection: &'a mut BrowserConnection,
        target_id: String,
        session_id: String,
        timeout: Duration,
    ) -> Self {
        Self {
            connection,
            target_id,
            session_id,
            deadline: Instant::now().checked_add(timeout),
        }
    }

    pub(super) fn command(
        &mut self,
        method: &str,
        params: Option<Value>,
        cancelled: &AtomicBool,
    ) -> Result<Value> {
        if cancelled.load(Ordering::Acquire) {
            return Err(AppError::BrowserCancelled);
        }
        if !self
            .connection
            .session_is_active(&self.target_id, &self.session_id)
        {
            return Err(AppError::BrowserProtocol(
                "CDP target session is no longer active".to_owned(),
            ));
        }
        let timeout = self
            .deadline
            .ok_or_else(|| {
                AppError::BrowserConnectionFailed("CDP command timeout is too large".to_owned())
            })?
            .saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            return Err(AppError::BrowserConnectionFailed(
                "CDP command timed out".to_owned(),
            ));
        }
        self.connection.command_in_session(
            &self.target_id,
            &self.session_id,
            method,
            params,
            cancelled,
            timeout,
        )
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

fn detached_event_matches(event: &Value, target_id: &str, session_id: &str) -> bool {
    event.get("method").and_then(Value::as_str) == Some("Target.detachedFromTarget")
        && event.pointer("/params/sessionId").and_then(Value::as_str) == Some(session_id)
        && event
            .pointer("/params/targetId")
            .and_then(Value::as_str)
            .is_none_or(|detached_target_id| detached_target_id == target_id)
}

fn is_session_invalid_error(error: &AppError) -> bool {
    let AppError::BrowserProtocol(message) = error else {
        return false;
    };
    let message = message.to_ascii_lowercase();
    message.contains("detached")
        || (message.contains("session") && message.contains("not found"))
        || (message.contains("target") && message.contains("not found"))
}
