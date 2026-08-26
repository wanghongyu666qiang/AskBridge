use std::{
    net::TcpListener,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use super::*;
use tungstenite::{Message, accept};

fn connected_test_client(
    endpoint: DevToolsEndpoint,
    websocket_url: &str,
    cancelled: &AtomicBool,
) -> CdpClient {
    let timeout = Duration::from_secs(2);
    let socket =
        ProtocolSocket::connect(&endpoint, websocket_url, cancelled, timeout).expect("test socket");
    let mut connection = BrowserConnection::new(socket);
    connection
        .sessions
        .insert("target".to_owned(), "test-session".to_owned());
    CdpClient {
        endpoint,
        timeout,
        connection: Mutex::new(Some(connection)),
    }
}

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
fn rejects_absurd_chunk_sizes_without_panicking() {
    // Declared chunk size of usize::MAX - 18 makes `end + 2` wrap around;
    // the decoder must reject the body instead of slicing out of range.
    let response =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nFFFFFFFFFFFFFFED\r\nhello";
    assert!(!response_is_complete(response).expect("completion check"));
    assert!(parse_http_response(response).is_err());

    let truncated =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\nA\r\nshort";
    assert!(!response_is_complete(truncated).expect("completion check"));
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
fn only_interactive_http_pages_are_accepted() {
    assert!(!is_interactive_ready_state(&json!({
        "result": {"value": {"ready": "loading", "url": "https://example.test/"}}
    })));
    assert!(!is_interactive_ready_state(&json!({
        "result": {"value": {"ready": "complete", "url": "about:blank"}}
    })));
    assert!(is_interactive_ready_state(&json!({
        "result": {"value": {"ready": "interactive", "url": "https://example.test/"}}
    })));
    assert!(is_interactive_ready_state(&json!({
        "result": {"value": {"ready": "complete", "url": "http://127.0.0.1:1234/test"}}
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

#[test]
fn file_inputs_must_be_enabled_and_accept_png() {
    assert!(file_input_accepts_png(&json!({
        "attributes": ["type", "file", "accept", "image/*"]
    })));
    assert!(file_input_accepts_png(&json!({
        "attributes": ["type", "file"]
    })));
    assert!(!file_input_accepts_png(&json!({
        "attributes": ["type", "file", "accept", "application/pdf"]
    })));
    assert!(!file_input_accepts_png(&json!({
        "attributes": ["type", "file", "accept", "image/png", "disabled", ""]
    })));
}

#[test]
fn exact_url_guard_is_a_read_only_runtime_check() {
    let expression = exact_url_check_expression("https://example.test/chat").expect("expression");
    assert_eq!(
        expression,
        r#"location.href === "https://example.test/chat""#
    );
    assert!(!expression.contains("querySelector"));
    assert!(!expression.contains("setFileInputFiles"));
}

#[test]
fn stalled_protocol_read_preserves_cancellation() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    let port = listener.local_addr().expect("address").port();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("connection");
        let mut socket = accept(stream).expect("websocket");
        let _ = socket.read().expect("command");
        let _ = socket.read();
    });

    let endpoint = DevToolsEndpoint::parse(&format!("{port}\n/devtools/browser/cancel-read\n"))
        .expect("endpoint");
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut socket = ProtocolSocket::connect(
        &endpoint,
        &format!("ws://127.0.0.1:{port}/devtools/page/target"),
        &cancelled,
        Duration::from_millis(250),
    )
    .expect("socket");
    let cancellation = Arc::clone(&cancelled);
    let cancel_thread = thread::spawn(move || {
        thread::sleep(Duration::from_millis(25));
        cancellation.store(true, Ordering::Release);
    });

    let started = Instant::now();
    let result = socket.command(
        "Runtime.evaluate",
        None,
        &cancelled,
        Duration::from_millis(250),
    );
    let elapsed = started.elapsed();

    assert!(matches!(result, Err(AppError::BrowserCancelled)));
    assert!(
        elapsed < Duration::from_millis(200),
        "cancellation took {elapsed:?}"
    );
    cancel_thread.join().expect("cancellation thread");
    drop(socket);
    server.join().expect("server");
}

#[test]
fn cancelled_command_is_not_sent() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    let port = listener.local_addr().expect("address").port();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("connection");
        let mut socket = accept(stream).expect("websocket");
        socket
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(250)))
            .expect("read timeout");
        if let Ok(Message::Text(message)) = socket.read() {
            panic!("unexpected command: {message}");
        }
    });

    let endpoint =
        DevToolsEndpoint::parse(&format!("{port}\n/devtools/browser/cancel-before-send\n"))
            .expect("endpoint");
    let cancelled = AtomicBool::new(false);
    let mut socket = ProtocolSocket::connect(
        &endpoint,
        &format!("ws://127.0.0.1:{port}/devtools/page/target"),
        &cancelled,
        Duration::from_secs(2),
    )
    .expect("socket");
    cancelled.store(true, Ordering::Release);

    let result = socket.command(
        "DOM.setFileInputFiles",
        Some(json!({"files": ["fixture.png"], "nodeId": 42})),
        &cancelled,
        Duration::from_secs(2),
    );
    assert!(matches!(result, Err(AppError::BrowserCancelled)));
    drop(socket);
    server.join().expect("server");
}

#[test]
fn file_input_navigation_guard_precedes_dom_queries() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    let port = listener.local_addr().expect("address").port();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("connection");
        let mut socket = accept(stream).expect("websocket");

        let Message::Text(message) = socket.read().expect("command") else {
            panic!("expected text command");
        };
        let command: Value = serde_json::from_str(message.as_ref()).expect("JSON command");
        assert_eq!(command["method"], "Runtime.evaluate");
        assert!(
            command["params"]["expression"]
                .as_str()
                .expect("expression")
                .contains("location.href ===")
        );
        socket
            .send(Message::Text(
                json!({
                    "id": command["id"],
                    "result": {"result": {"value": false}}
                })
                .to_string()
                .into(),
            ))
            .expect("response");
    });

    let endpoint =
        DevToolsEndpoint::parse(&format!("{port}\n/devtools/browser/guard-before-query\n"))
            .expect("endpoint");
    let cancelled = AtomicBool::new(false);
    let client = connected_test_client(
        endpoint,
        &format!("ws://127.0.0.1:{port}/devtools/page/target"),
        &cancelled,
    );
    let target = CdpTarget {
        id: "target".to_owned(),
        kind: "page".to_owned(),
        url: "https://example.test/chat".to_owned(),
        web_socket_debugger_url: Some(format!("ws://127.0.0.1:{port}/devtools/page/target")),
    };
    let result = client
        .set_file_input(
            &target,
            "https://example.test/chat",
            Path::new("fixture.png"),
            &[],
            &cancelled,
            Duration::from_secs(2),
        )
        .expect("guard result");
    assert_eq!(result, FileInputResult::NavigationChanged);
    server.join().expect("server");
}

#[test]
fn file_input_rechecks_navigation_before_set_file_input_files() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    let port = listener.local_addr().expect("address").port();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("connection");
        let mut socket = accept(stream).expect("websocket");
        let mut methods = Vec::new();
        let mut runtime_evaluations = 0;
        loop {
            let Ok(Message::Text(message)) = socket.read() else {
                break;
            };
            let command: Value = serde_json::from_str(message.as_ref()).expect("JSON command");
            let id = command["id"].clone();
            let method = command["method"].as_str().expect("method").to_owned();
            methods.push(method.clone());
            let result = match method.as_str() {
                "Runtime.evaluate" => {
                    runtime_evaluations += 1;
                    json!({"result": {"value": runtime_evaluations == 1}})
                }
                "DOM.getDocument" => json!({"root": {"nodeId": 1}}),
                "DOM.querySelectorAll" => json!({"nodeIds": [42]}),
                "DOM.getAttributes" => {
                    json!({"attributes": ["type", "file", "accept", "image/png"]})
                }
                "DOM.setFileInputFiles" => panic!("file mutation bypassed navigation guard"),
                other => panic!("unexpected CDP method {other}"),
            };
            socket
                .send(Message::Text(
                    json!({"id": id, "result": result}).to_string().into(),
                ))
                .expect("response");
        }
        methods
    });

    let endpoint =
        DevToolsEndpoint::parse(&format!("{port}\n/devtools/browser/guard-before-set\n"))
            .expect("endpoint");
    let cancelled = AtomicBool::new(false);
    let client = connected_test_client(
        endpoint,
        &format!("ws://127.0.0.1:{port}/devtools/page/target"),
        &cancelled,
    );
    let target = CdpTarget {
        id: "target".to_owned(),
        kind: "page".to_owned(),
        url: "https://example.test/chat".to_owned(),
        web_socket_debugger_url: Some(format!("ws://127.0.0.1:{port}/devtools/page/target")),
    };
    let result = client
        .set_file_input(
            &target,
            "https://example.test/chat",
            Path::new("fixture.png"),
            &[],
            &cancelled,
            Duration::from_secs(2),
        )
        .expect("guard result");
    assert_eq!(result, FileInputResult::NavigationChanged);
    drop(client);
    let methods = server.join().expect("server");
    assert_eq!(
        methods,
        [
            "Runtime.evaluate",
            "DOM.getDocument",
            "DOM.querySelectorAll",
            "DOM.getAttributes",
            "Runtime.evaluate"
        ]
    );
}

#[test]
fn target_session_reuses_one_persistent_websocket_and_buffers_events() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    let port = listener.local_addr().expect("address").port();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("connection");
        let mut socket = accept(stream).expect("websocket");
        let mut methods = Vec::new();
        for index in 0..3 {
            let Message::Text(message) = socket.read().expect("command") else {
                panic!("expected text command");
            };
            let command: Value = serde_json::from_str(message.as_ref()).expect("JSON command");
            let method = command["method"].as_str().expect("method").to_owned();
            methods.push(method.clone());
            match index {
                0 => {
                    assert_eq!(method, "Target.attachToTarget");
                    assert_eq!(command["params"]["targetId"], "target");
                    assert_eq!(command["params"]["flatten"], true);
                    assert!(command.get("sessionId").is_none());
                    socket
                        .send(Message::Text(
                            json!({
                                "id": command["id"],
                                "result": {"sessionId": "session-1"}
                            })
                            .to_string()
                            .into(),
                        ))
                        .expect("attach response");
                }
                1 => {
                    assert_eq!(method, "Runtime.evaluate");
                    assert_eq!(command["sessionId"], "session-1");
                    socket
                        .send(Message::Text(
                            json!({
                                "method": "Runtime.consoleAPICalled",
                                "sessionId": "session-1",
                                "params": {"type": "log"}
                            })
                            .to_string()
                            .into(),
                        ))
                        .expect("event");
                    socket
                        .send(Message::Text(
                            json!({"id": command["id"], "result": {"value": 1}})
                                .to_string()
                                .into(),
                        ))
                        .expect("first response");
                }
                2 => {
                    assert_eq!(method, "Runtime.evaluate");
                    assert_eq!(command["sessionId"], "session-1");
                    socket
                        .send(Message::Text(
                            json!({"id": command["id"], "result": {"value": 2}})
                                .to_string()
                                .into(),
                        ))
                        .expect("second response");
                }
                _ => unreachable!(),
            }
        }
        methods
    });

    let endpoint =
        DevToolsEndpoint::parse(&format!("{port}\n/devtools/browser/persistent-session\n"))
            .expect("endpoint");
    let cancelled = AtomicBool::new(false);
    let socket = ProtocolSocket::connect(
        &endpoint,
        &format!("ws://127.0.0.1:{port}/devtools/browser/persistent-session"),
        &cancelled,
        Duration::from_secs(2),
    )
    .expect("socket");
    let mut connection = BrowserConnection::new(socket);
    let target = CdpTarget {
        id: "target".to_owned(),
        kind: "page".to_owned(),
        url: "https://example.test/chat".to_owned(),
        web_socket_debugger_url: Some(format!("ws://127.0.0.1:{port}/devtools/page/target")),
    };
    let first = connection
        .target_command(
            &target,
            "Runtime.evaluate",
            None,
            &cancelled,
            Duration::from_secs(2),
        )
        .expect("first command");
    let second = connection
        .target_command(
            &target,
            "Runtime.evaluate",
            None,
            &cancelled,
            Duration::from_secs(2),
        )
        .expect("second command");
    assert_eq!(first["value"], 1);
    assert_eq!(second["value"], 2);
    assert_eq!(
        connection.sessions.get("target").map(String::as_str),
        Some("session-1")
    );
    assert_eq!(connection.events.len(), 1);
    assert_eq!(connection.events[0]["method"], "Runtime.consoleAPICalled");
    drop(connection);
    assert_eq!(
        server.join().expect("server"),
        [
            "Target.attachToTarget",
            "Runtime.evaluate",
            "Runtime.evaluate"
        ]
    );
}

#[test]
fn detached_target_session_is_invalidated_before_another_command() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");

    let port = listener.local_addr().expect("address").port();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("connection");
        let mut socket = accept(stream).expect("websocket");

        let Message::Text(attach) = socket.read().expect("attach command") else {
            panic!("expected attach command");
        };
        let attach: Value = serde_json::from_str(attach.as_ref()).expect("attach JSON");
        assert_eq!(attach["method"], "Target.attachToTarget");
        socket
            .send(Message::Text(
                json!({"id": attach["id"], "result": {"sessionId": "session-1"}})
                    .to_string()
                    .into(),
            ))
            .expect("attach response");

        let Message::Text(command) = socket.read().expect("session command") else {
            panic!("expected session command");
        };
        let command: Value = serde_json::from_str(command.as_ref()).expect("command JSON");
        assert_eq!(command["method"], "Runtime.evaluate");
        socket
            .send(Message::Text(
                json!({
                    "method": "Target.detachedFromTarget",
                    "params": {"sessionId": "session-1", "targetId": "target"}
                })
                .to_string()
                .into(),
            ))
            .expect("detached event");
        socket
            .send(Message::Text(
                json!({"id": command["id"], "result": {"value": 1}})
                    .to_string()
                    .into(),
            ))
            .expect("command response");

        socket
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(250)))
            .expect("read timeout");
        if let Ok(Message::Text(message)) = socket.read() {
            panic!("detached session received another command: {message}");
        }
    });

    let endpoint =
        DevToolsEndpoint::parse(&format!("{port}\n/devtools/browser/detached-session\n"))
            .expect("endpoint");
    let cancelled = AtomicBool::new(false);
    let socket = ProtocolSocket::connect(
        &endpoint,
        &format!("ws://127.0.0.1:{port}/devtools/browser/detached-session"),
        &cancelled,
        Duration::from_secs(2),
    )
    .expect("socket");
    let mut connection = BrowserConnection::new(socket);
    let target = CdpTarget {
        id: "target".to_owned(),
        kind: "page".to_owned(),
        url: "https://example.test/chat".to_owned(),
        web_socket_debugger_url: Some(format!("ws://127.0.0.1:{port}/devtools/page/target")),
    };
    let first = connection
        .target_command(
            &target,
            "Runtime.evaluate",
            None,
            &cancelled,
            Duration::from_secs(2),
        )
        .expect("first command response");
    assert_eq!(first["value"], 1);
    assert!(!connection.sessions.contains_key("target"));

    let error = connection
        .target_command(
            &target,
            "Runtime.evaluate",
            None,
            &cancelled,
            Duration::from_secs(2),
        )
        .expect_err("detached target must fail closed");
    assert!(matches!(error, AppError::TargetNotFound));
    drop(connection);
    server.join().expect("server");
}

#[test]
fn detach_event_during_attach_never_publishes_the_stale_session() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("listener");
    let port = listener.local_addr().expect("address").port();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("connection");
        let mut socket = accept(stream).expect("websocket");
        let Message::Text(attach) = socket.read().expect("attach command") else {
            panic!("expected attach command");
        };
        let attach: Value = serde_json::from_str(attach.as_ref()).expect("attach JSON");
        socket
            .send(Message::Text(
                json!({
                    "method": "Target.detachedFromTarget",
                    "params": {"sessionId": "session-1"}
                })
                .to_string()
                .into(),
            ))
            .expect("detach event");
        socket
            .send(Message::Text(
                json!({"id": attach["id"], "result": {"sessionId": "session-1"}})
                    .to_string()
                    .into(),
            ))
            .expect("attach response");
        socket
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(250)))
            .expect("read timeout");
        if let Ok(Message::Text(message)) = socket.read() {
            panic!("stale attached session received a command: {message}");
        }
    });

    let endpoint =
        DevToolsEndpoint::parse(&format!("{port}\n/devtools/browser/detach-during-attach\n"))
            .expect("endpoint");
    let cancelled = AtomicBool::new(false);
    let socket = ProtocolSocket::connect(
        &endpoint,
        &format!("ws://127.0.0.1:{port}/devtools/browser/detach-during-attach"),
        &cancelled,
        Duration::from_secs(2),
    )
    .expect("socket");
    let mut connection = BrowserConnection::new(socket);
    let target = CdpTarget {
        id: "target".to_owned(),
        kind: "page".to_owned(),
        url: "https://example.test/chat".to_owned(),
        web_socket_debugger_url: Some(format!("ws://127.0.0.1:{port}/devtools/page/target")),
    };
    let error = connection
        .target_command(
            &target,
            "Runtime.evaluate",
            None,
            &cancelled,
            Duration::from_secs(2),
        )
        .expect_err("attach race must fail closed");
    assert!(matches!(error, AppError::BrowserProtocol(_)));
    assert!(!connection.sessions.contains_key("target"));
    drop(connection);
    server.join().expect("server");
}

#[test]
fn file_input_state_alone_never_claims_page_attachment_readiness() {
    let baseline = AttachmentReceipt {
        file_count: 0,
        named_count: 2,
        preview_count: 1,
        busy_count: 0,
    };
    assert!(!has_new_attachment_receipt(
        baseline,
        AttachmentReceipt {
            file_count: 1,
            ..baseline
        }
    ));
    assert!(!has_new_attachment_receipt(
        baseline,
        AttachmentReceipt {
            file_count: 1,
            named_count: 3,
            busy_count: 1,
            ..baseline
        }
    ));
    assert!(has_new_attachment_receipt(
        baseline,
        AttachmentReceipt {
            file_count: 1,
            named_count: 3,
            ..baseline
        }
    ));
    assert!(has_new_attachment_receipt(
        baseline,
        AttachmentReceipt {
            file_count: 1,
            preview_count: 2,
            ..baseline
        }
    ));
}
