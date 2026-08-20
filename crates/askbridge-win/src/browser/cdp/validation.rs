use askbridge_core::{AppError, Result};
use serde_json::Value;

pub(super) fn exact_url_check_expression(expected_url: &str) -> Result<String> {
    let expected_url = serde_json::to_string(expected_url).map_err(|_| {
        AppError::InvalidPreparation("expected target URL could not be encoded".to_owned())
    })?;
    Ok(format!("location.href === {expected_url}"))
}

pub(super) fn split_loopback_websocket(url: &str) -> Result<(u16, &str)> {
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

pub(super) fn is_interactive_ready_state(result: &Value) -> bool {
    let Some(value) = result.pointer("/result/value") else {
        return false;
    };
    let ready = value.get("ready").and_then(Value::as_str);
    let url = value.get("url").and_then(Value::as_str);
    ready.is_some_and(|state| matches!(state, "interactive" | "complete"))
        && url.is_some_and(|url| validate_page_url(url).is_ok())
}

pub(super) fn validate_page_url(url: &str) -> Result<()> {
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

pub(super) fn validate_target_id(target_id: &str) -> Result<()> {
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

pub(super) fn percent_encode(value: &str) -> String {
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
