use std::ptr;

use askbridge_core::{AppError, Result};
use windows_sys::Win32::{
    Foundation::GetLastError,
    Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
        WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
        WinHttpSendRequest, WinHttpSetTimeouts,
    },
};

const REQUEST_TIMEOUT_MS: i32 = 15_000;

pub(super) fn get_https(url: &str, max_bytes: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    get_https_chunks(url, max_bytes, |chunk| {
        body.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(body)
}

/// Streams the response body to `sink` in bounded chunks. Returning an error
/// from `sink` aborts the transfer without reading further data.
pub(super) fn get_https_chunks(
    url: &str,
    max_bytes: usize,
    mut sink: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    if max_bytes == 0 {
        return Err(update_error("HTTP 响应大小限制不能为零"));
    }
    let parsed = ParsedHttpsUrl::parse(url)?;
    let agent = wide("AskBridge Update/1.0");
    let host = wide(&parsed.host);
    let method = wide("GET");
    let path = wide(&parsed.path);

    // SAFETY: All strings are live, NUL-terminated UTF-16 buffers for synchronous calls.
    let session = unsafe {
        WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        )
    };
    let session = InternetHandle::new(session, "WinHttpOpen(update)")?;
    // SAFETY: session and host remain valid for this synchronous connection call.
    let connection = unsafe { WinHttpConnect(session.0, host.as_ptr(), parsed.port, 0) };
    let connection = InternetHandle::new(connection, "WinHttpConnect(update)")?;
    // SAFETY: connection is valid and the request uses HTTPS without a body.
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
    let request = InternetHandle::new(request, "WinHttpOpenRequest(update)")?;
    // SAFETY: request is live and all bounded timeout values are milliseconds.
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
        return Err(last_windows_error("WinHttpSetTimeouts(update)"));
    }
    // SAFETY: No extra headers or request body are supplied.
    if unsafe { WinHttpSendRequest(request.0, ptr::null(), 0, ptr::null(), 0, 0, 0) } == 0 {
        return Err(last_windows_error("WinHttpSendRequest(update)"));
    }
    // SAFETY: request remains live for this synchronous response operation.
    if unsafe { WinHttpReceiveResponse(request.0, ptr::null_mut()) } == 0 {
        return Err(last_windows_error("WinHttpReceiveResponse(update)"));
    }
    let status = response_status(&request)?;
    if !(200..300).contains(&status) {
        return Err(update_error(format!("更新服务器返回 HTTP {status}")));
    }
    read_response(&request, max_bytes, &mut sink)
}

fn response_status(request: &InternetHandle) -> Result<u32> {
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
        return Err(last_windows_error("WinHttpQueryHeaders(update)"));
    }
    Ok(status)
}

fn read_response(
    request: &InternetHandle,
    max_bytes: usize,
    sink: &mut impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    let mut total: usize = 0;
    let mut buffer = [0_u8; 64 * 1024];
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
            return Err(last_windows_error("WinHttpReadData(update)"));
        }
        if read == 0 {
            break;
        }
        let read = usize::try_from(read).map_err(|_| update_error("HTTP 响应长度溢出"))?;
        total = total.saturating_add(read);
        if total > max_bytes {
            return Err(update_error("更新服务器响应超出大小限制"));
        }
        sink(&buffer[..read])?;
    }
    Ok(())
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
    // SAFETY: GetLastError is read immediately after a failed WinHTTP call.
    AppError::Windows {
        operation,
        win32_code: unsafe { GetLastError() },
    }
}

fn update_error(message: impl Into<String>) -> AppError {
    AppError::UpdateFailed(message.into())
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
            return Err(update_error("更新地址格式无效"));
        }
        let remainder = url
            .strip_prefix("https://")
            .ok_or_else(|| update_error("更新地址必须使用 HTTPS"))?;
        let (authority, suffix) = remainder
            .split_once('/')
            .map_or((remainder, ""), |(authority, path)| (authority, path));
        if authority.is_empty() || authority.contains('@') || authority.starts_with('[') {
            return Err(update_error("更新地址主机格式无效"));
        }
        let (host, port) = authority.rsplit_once(':').map_or_else(
            || Ok((authority, 443)),
            |(host, port)| {
                port.parse::<u16>()
                    .ok()
                    .filter(|port| *port != 0)
                    .map(|port| (host, port))
                    .ok_or_else(|| update_error("更新地址端口无效"))
            },
        )?;
        if host.is_empty()
            || !host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        {
            return Err(update_error("更新地址主机格式无效"));
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
    use super::*;

    #[test]
    fn accepts_bounded_https_urls() {
        let parsed = ParsedHttpsUrl::parse(
            "https://api.github.com/repos/wanghongyu666qiang/AskBridge/releases/latest",
        )
        .expect("GitHub URL");
        assert_eq!(parsed.host, "api.github.com");
        assert_eq!(parsed.port, 443);
        assert_eq!(
            parsed.path,
            "/repos/wanghongyu666qiang/AskBridge/releases/latest"
        );
    }

    #[test]
    fn rejects_non_https_credentials_and_fragments() {
        for url in [
            "http://github.com/file",
            "https://user@github.com/file",
            "https://github.com/file#fragment",
        ] {
            assert!(ParsedHttpsUrl::parse(url).is_err(), "{url}");
        }
    }
}
