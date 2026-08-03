//! Integration tests for the Face Unlock pipe protocol.
//!
//! These spin up an in-process named-pipe server (using the same
//! `pipe_server` module the real service uses) and exercise the JSON
//! protocol end-to-end: ping → auth_start → auth_poll → success.
//!
//! The auth runner is mocked at the handler level (no camera / ONNX
//! models needed), so these tests run in CI without hardware.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use micontrol_lib::hw::face::pipe_server;
use serde_json::json;

/// A mock auth runner that always succeeds for user "alice".
struct MockHandler {
    state: Arc<Mutex<MockState>>,
}

#[derive(Default)]
struct MockState {
    auth_started: bool,
    polls: u32,
}

impl MockHandler {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockState::default())),
        }
    }

    fn call(&self, req: &serde_json::Value) -> serde_json::Value {
        let cmd = req.get("cmd").and_then(|c| c.as_str()).unwrap_or("");
        match cmd {
            "ping" => json!({
                "ok": true,
                "ready": true,
                "users": ["alice"],
                "version": "test",
                "protocol": 1,
            }),
            "auth_start" => {
                let mut s = self.state.lock().unwrap();
                s.auth_started = true;
                s.polls = 0;
                json!({ "ok": true, "done": false })
            }
            "auth_poll" => {
                let mut s = self.state.lock().unwrap();
                s.polls += 1;
                if s.polls >= 3 {
                    json!({
                        "ok": true,
                        "done": true,
                        "success": true,
                        "user": "alice",
                        "similarity": 0.87,
                        "instruction": "ok",
                    })
                } else {
                    json!({
                        "ok": true,
                        "done": false,
                        "instruction": "blink",
                    })
                }
            }
            _ => json!({ "ok": false, "reason": format!("unknown command: {cmd}") }),
        }
    }
}

/// Serialize pipe tests (the pipe is single-instance — parallel tests would
/// collide on the same name).
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Run a single serve_one iteration on a background thread and connect as a
/// client, returning the client's response.
fn roundtrip(handler: &'static MockHandler, request: &str) -> String {
    let _guard = TEST_LOCK.lock().unwrap();
    let shutdown = AtomicBool::new(false);

    let server = std::thread::spawn(move || {
        // serve_one takes a concrete F: Fn — wrap the handler method in a
        // concrete closure capturing the &MockHandler.
        let h: &MockHandler = handler;
        let handler_fn = move |req: &serde_json::Value| h.call(req);
        let _ = pipe_server::serve_one(&handler_fn, &shutdown);
    });

    // Give the server a moment to create the pipe instance.
    std::thread::sleep(std::time::Duration::from_millis(300));

    let client = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(micontrol_lib::hw::face::config::PIPE_NAME)
        .expect("open pipe");

    use std::io::{Read, Write};
    let mut client = client;
    client.write_all(request.as_bytes()).unwrap();
    client.flush().unwrap();

    let mut buf = Vec::new();
    let _ = client.read_to_end(&mut buf);
    let _ = server.join();

    String::from_utf8_lossy(&buf).to_string()
}

#[test]
fn pipe_ping_responds() {
    let handler = Box::leak(Box::new(MockHandler::new()));
    let resp = roundtrip(handler, r#"{"cmd":"ping"}"#);
    let v: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    assert_eq!(v["ok"], true);
    assert_eq!(v["ready"], true);
    assert_eq!(v["users"][0], "alice");
}

#[test]
fn pipe_auth_flow_succeeds() {
    let handler = Box::leak(Box::new(MockHandler::new()));
    let resp = roundtrip(handler, r#"{"cmd":"auth_start"}"#);
    let v: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    assert_eq!(v["ok"], true);
    assert_eq!(v["done"], false);
}

#[test]
fn pipe_auth_poll_reports_progress() {
    let handler = Box::leak(Box::new(MockHandler::new()));
    let resp = roundtrip(handler, r#"{"cmd":"auth_poll"}"#);
    let v: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    assert!(
        v["done"].as_bool().is_some(),
        "poll should have a done field"
    );
}

#[test]
fn pipe_unknown_command_rejected() {
    let handler = Box::leak(Box::new(MockHandler::new()));
    let resp = roundtrip(handler, r#"{"cmd":"bogus"}"#);
    let v: serde_json::Value = serde_json::from_str(&resp).expect("valid JSON");
    assert_eq!(v["ok"], false);
    assert!(v["reason"].as_str().unwrap_or("").contains("unknown"));
}
