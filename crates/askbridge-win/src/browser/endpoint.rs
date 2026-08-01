use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
};

use askbridge_core::{AppError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevToolsEndpoint {
    port: u16,
    browser_path: String,
}

impl DevToolsEndpoint {
    pub fn read(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .map_err(|source| AppError::io("reading browser debugging endpoint", path, source))?;
        Self::parse(&contents)
    }

    pub fn parse(contents: &str) -> Result<Self> {
        let mut lines = contents.lines();
        let port = lines
            .next()
            .and_then(|value| value.trim().parse::<u16>().ok())
            .filter(|port| *port != 0)
            .ok_or(AppError::BrowserEndpointUnavailable)?;
        let browser_path = lines
            .next()
            .map(str::trim)
            .filter(|path| path.starts_with("/devtools/browser/"))
            .filter(|path| !path.contains(['\r', '\n', '?', '#']))
            .ok_or(AppError::BrowserEndpointUnavailable)?;
        if lines.any(|line| !line.trim().is_empty()) {
            return Err(AppError::BrowserEndpointUnavailable);
        }

        Ok(Self {
            port,
            browser_path: browser_path.to_owned(),
        })
    }

    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port)
    }

    pub fn browser_websocket_url(&self) -> String {
        format!("ws://127.0.0.1:{}{}", self.port, self.browser_path)
    }

    pub fn http_path(&self, path: &str) -> Result<String> {
        if !path.starts_with('/') || path.contains(['\r', '\n']) {
            return Err(AppError::BrowserProtocol(
                "invalid local debugging request path".to_owned(),
            ));
        }
        Ok(path.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dynamic_loopback_endpoint() {
        let endpoint = DevToolsEndpoint::parse("49321\n/devtools/browser/01234567-89ab-cdef\n")
            .expect("parse");

        assert_eq!(endpoint.socket_addr(), "127.0.0.1:49321".parse().unwrap());
        assert_eq!(
            endpoint.browser_websocket_url(),
            "ws://127.0.0.1:49321/devtools/browser/01234567-89ab-cdef"
        );
    }

    #[test]
    fn rejects_missing_fixed_or_injected_endpoint_data() {
        for invalid in [
            "",
            "0\n/devtools/browser/id\n",
            "9222\n",
            "9222\n/http/not-browser\n",
            "9222\n/devtools/browser/id?redirect=evil\n",
            "9222\n/devtools/browser/id\nunexpected\n",
        ] {
            assert!(matches!(
                DevToolsEndpoint::parse(invalid),
                Err(AppError::BrowserEndpointUnavailable)
            ));
        }
    }

    #[test]
    fn rejects_request_line_injection() {
        let endpoint = DevToolsEndpoint::parse("49321\n/devtools/browser/id\n").expect("endpoint");
        assert!(endpoint.http_path("/json/list").is_ok());
        assert!(endpoint.http_path("/json/list\r\nHost: evil").is_err());
        assert!(endpoint.http_path("http://evil.test").is_err());
    }
}
