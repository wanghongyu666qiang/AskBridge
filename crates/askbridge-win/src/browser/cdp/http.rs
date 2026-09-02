use std::{
    io::Read,
    net::TcpStream,
    time::{Duration, Instant},
};

use askbridge_core::{AppError, Result};

use super::MAX_HTTP_RESPONSE_BYTES;

pub(super) fn parse_http_response(response: &[u8]) -> Result<Vec<u8>> {
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

pub(super) fn read_http_response(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>> {
    let mut response = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AppError::TargetTimeout);
        }
        stream
            .set_read_timeout(Some(remaining.min(Duration::from_millis(250))))
            .map_err(|_| {
                AppError::BrowserConnectionFailed("local debugging timeout setup failed".to_owned())
            })?;
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
                ) =>
            {
                if response_is_complete(&response)? {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(AppError::TargetTimeout);
                }
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

pub(super) fn response_is_complete(response: &[u8]) -> Result<bool> {
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

pub(super) fn content_length(headers: &str) -> Result<Option<usize>> {
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

pub(super) fn decode_chunked_body(body: &[u8]) -> Result<Vec<u8>> {
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
        let Some(tail) = body.get(end..) else {
            return Err(AppError::BrowserProtocol(
                "invalid chunked debugging body".to_owned(),
            ));
        };
        if tail.len() < 2 || &tail[..2] != b"\r\n" {
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

pub(super) fn chunked_body_length(body: &[u8]) -> Result<Option<usize>> {
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
            if body
                .get(cursor..)
                .is_some_and(|tail| tail.starts_with(b"\r\n"))
            {
                return Ok(Some(cursor + 2));
            }
            return Ok(None);
        }
        let end = cursor
            .checked_add(size)
            .ok_or_else(|| AppError::BrowserProtocol("debugging chunk size overflow".to_owned()))?;
        let Some(tail) = body.get(end..) else {
            return Ok(None);
        };
        if tail.len() < 2 {
            return Ok(None);
        }
        if &tail[..2] != b"\r\n" {
            return Err(AppError::BrowserProtocol(
                "invalid chunked debugging body".to_owned(),
            ));
        }
        cursor = end + 2;
    }
}

pub(super) fn find_crlf(bytes: &[u8], start: usize) -> Option<usize> {
    bytes
        .get(start..)?
        .windows(2)
        .position(|window| window == b"\r\n")
        .map(|position| start + position)
}
