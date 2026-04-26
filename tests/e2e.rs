//! End-to-end integration tests.
//!
//! Each test spawns a sangha daemon as a subprocess, exercises it via HTTP
//! using the MCP streamable-HTTP / JSON-RPC 2.0 protocol, and tears down
//! the process on drop.

use std::net::TcpStream;
use std::process::{Child, Command};
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

struct TestDaemon {
    child: Child,
    port: u16,
    _dir: TempDir,
}

impl TestDaemon {
    fn start() -> Self {
        let dir = TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("test.db");

        // Grab an available port, then release it so the daemon can bind.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let child = Command::new(env!("CARGO_BIN_EXE_sangha"))
            .arg("serve")
            .env("SANGHA_DB_PATH", db_path.to_str().unwrap())
            .env("SANGHA_PORT", port.to_string())
            .env("SANGHA_HOST", "127.0.0.1")
            .env("SANGHA_LOG_LEVEL", "error")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("failed to start sangha");

        // Poll until the server accepts connections (up to 5 s).
        let addr = format!("127.0.0.1:{port}");
        for _ in 0..50 {
            if TcpStream::connect(&addr).is_ok() {
                return Self {
                    child,
                    port,
                    _dir: dir,
                };
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        // Server never became ready — clean up and fail.
        let mut child = child;
        let _ = child.kill();
        let _ = child.wait();
        panic!("sangha did not become ready on port {port} within 5 s");
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// MCP / JSON-RPC helpers
// ---------------------------------------------------------------------------

/// Send a JSON-RPC request and return the parsed response.
///
/// The MCP streamable-HTTP transport may respond with either
/// `application/json` or `text/event-stream`. This helper handles both.
async fn send_jsonrpc(
    client: &reqwest::Client,
    url: &str,
    session_id: Option<&str>,
    request: Value,
) -> Value {
    let mut builder = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&request);

    if let Some(sid) = session_id {
        builder = builder.header("Mcp-Session-Id", sid);
    }

    let resp = builder.send().await.expect("send request");
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body = resp.text().await.expect("read body");

    if content_type.contains("text/event-stream") {
        // Parse SSE: find the last `data:` line that contains a JSON-RPC response.
        for line in body.lines() {
            if let Some(data) = line.strip_prefix("data: ").or_else(|| line.strip_prefix("data:")) {
                let data = data.trim();
                if let Ok(v) = serde_json::from_str::<Value>(data)
                    && v.get("id").is_some()
                {
                    return v;
                }
            }
        }
        panic!("no JSON-RPC response found in SSE stream:\n{body}");
    } else {
        serde_json::from_str(&body).unwrap_or_else(|e| {
            panic!("failed to parse JSON response ({e}):\n{body}");
        })
    }
}

/// Send a JSON-RPC notification (no response expected).
async fn send_notification(
    client: &reqwest::Client,
    url: &str,
    session_id: &str,
    notification: Value,
) {
    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("Mcp-Session-Id", session_id)
        .json(&notification)
        .send()
        .await
        .expect("send notification");

    // Consume the body regardless of status.
    let _ = resp.text().await;
}

/// Perform the MCP `initialize` + `notifications/initialized` handshake.
/// Returns the `Mcp-Session-Id` header value.
async fn init_session(client: &reqwest::Client, url: &str) -> String {
    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "e2e-test", "version": "0.1.0" }
        }
    });

    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .json(&init_req)
        .send()
        .await
        .expect("send initialize");

    let session_id = resp
        .headers()
        .get("mcp-session-id")
        .expect("initialize response should contain Mcp-Session-Id header")
        .to_str()
        .expect("Mcp-Session-Id should be valid UTF-8")
        .to_string();

    // Consume the body.
    let _ = resp.text().await;

    // Send the initialized notification.
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    send_notification(client, url, &session_id, notif).await;

    session_id
}

/// Call a sangha tool over MCP and return the parsed tool-result content.
///
/// The JSON-RPC response looks like:
/// ```json
/// { "jsonrpc": "2.0", "id": N, "result": { "content": [{ "type": "text", "text": "..." }] } }
/// ```
///
/// This helper extracts the `text` field of the first content item and parses
/// it as JSON (since all sangha tools return `serde_json::to_string_pretty`).
async fn call_tool(
    client: &reqwest::Client,
    url: &str,
    session_id: &str,
    tool_name: &str,
    args: Value,
    id: u64,
) -> Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": args
        }
    });

    let resp = send_jsonrpc(client, url, Some(session_id), req).await;

    // Check for a JSON-RPC-level error.
    if let Some(err) = resp.get("error") {
        return json!({ "_error": err });
    }

    // Extract the tool's text content and parse it.
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("unexpected tools/call response shape:\n{resp:#}");
        });

    serde_json::from_str(text).unwrap_or_else(|e| {
        panic!("tool response is not JSON ({e}):\n{text}");
    })
}

/// Like `call_tool` but returns the raw JSON-RPC response (for error checks).
async fn call_tool_raw(
    client: &reqwest::Client,
    url: &str,
    session_id: &str,
    tool_name: &str,
    args: Value,
    id: u64,
) -> Value {
    let req = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": args
        }
    });

    send_jsonrpc(client, url, Some(session_id), req).await
}

/// Register a session on the given MCP connection and return the session_id
/// assigned by sangha.
async fn register_session(
    client: &reqwest::Client,
    url: &str,
    session_id: &str,
    project: &str,
    id: u64,
) -> String {
    let out = call_tool(
        client,
        url,
        session_id,
        "session_register",
        json!({ "project": project }),
        id,
    )
    .await;

    out["session_id"]
        .as_str()
        .expect("session_register should return session_id")
        .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 1. The daemon starts and responds to an MCP `initialize` handshake.
#[tokio::test]
async fn test_server_starts_and_responds() {
    let daemon = TestDaemon::start();
    let client = reqwest::Client::new();

    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "e2e-test", "version": "0.1.0" }
        }
    });

    let resp = send_jsonrpc(&client, &daemon.url(), None, init_req).await;

    // The response should be a valid JSON-RPC result with server info.
    assert!(resp.get("result").is_some(), "expected a result, got: {resp:#}");
    let server_info = &resp["result"]["serverInfo"];
    assert!(
        server_info["name"].is_string(),
        "serverInfo.name should be a string, got: {server_info:#}"
    );
    // The instructions should mention sangha.
    let instructions = resp["result"]["instructions"]
        .as_str()
        .unwrap_or("");
    assert!(
        instructions.contains("sangha"),
        "instructions should mention sangha, got: {instructions}"
    );
}

/// 2. After initialize, `tools/list` returns exactly 9 tools.
#[tokio::test]
async fn test_tool_count() {
    let daemon = TestDaemon::start();
    let client = reqwest::Client::new();
    let sid = init_session(&client, &daemon.url()).await;

    let req = json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/list",
        "params": {}
    });

    let resp = send_jsonrpc(&client, &daemon.url(), Some(&sid), req).await;

    let tools = resp["result"]["tools"]
        .as_array()
        .expect("tools/list should return an array of tools");

    let tool_names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();

    assert_eq!(
        tools.len(),
        9,
        "expected 9 tools, got {}: {tool_names:?}",
        tools.len()
    );
}

/// 3. Full session lifecycle: register -> heartbeat -> list -> unregister.
#[tokio::test]
async fn test_full_session_lifecycle() {
    let daemon = TestDaemon::start();
    let client = reqwest::Client::new();
    let sid = init_session(&client, &daemon.url()).await;

    let project = "/test/lifecycle";

    // Register.
    let reg = call_tool(
        &client,
        &daemon.url(),
        &sid,
        "session_register",
        json!({ "project": project, "intent": "lifecycle test" }),
        2,
    )
    .await;
    let sangha_sid = reg["session_id"]
        .as_str()
        .expect("session_register should return session_id");

    // Heartbeat.
    let hb = call_tool(
        &client,
        &daemon.url(),
        &sid,
        "session_heartbeat",
        json!({}),
        3,
    )
    .await;
    assert_eq!(
        hb["session_id"].as_str().unwrap(),
        sangha_sid,
        "heartbeat should echo the same session_id"
    );
    assert!(
        hb["ttl_remaining_sec"].as_i64().unwrap() > 0,
        "TTL remaining should be positive"
    );

    // List.
    let list = call_tool(
        &client,
        &daemon.url(),
        &sid,
        "session_list",
        json!({ "project": project }),
        4,
    )
    .await;
    let sessions = list["sessions"].as_array().expect("sessions array");
    assert!(
        sessions.iter().any(|s| s["session_id"].as_str() == Some(sangha_sid)),
        "our session should appear in session_list"
    );

    // Unregister.
    let unreg = call_tool(
        &client,
        &daemon.url(),
        &sid,
        "session_unregister",
        json!({}),
        5,
    )
    .await;
    assert_eq!(unreg["ok"].as_bool(), Some(true));

    // List again — should be gone.
    // Need a fresh MCP session to list since identity is now unbound on server side.
    // Actually, session_list doesn't require identity binding, so we can still call it.
    // But the identity is still bound (identity is connection-scoped, not cleared on unregister).
    // However, a new MCP session is cleaner.
    let sid2 = init_session(&client, &daemon.url()).await;
    let list2 = call_tool(
        &client,
        &daemon.url(),
        &sid2,
        "session_list",
        json!({ "project": project }),
        6,
    )
    .await;
    let sessions2 = list2["sessions"].as_array().expect("sessions array");
    assert!(
        !sessions2.iter().any(|s| s["session_id"].as_str() == Some(sangha_sid)),
        "unregistered session should no longer appear in session_list"
    );
}

/// 4. Lock conflict between two clients: A claims, B is denied, A releases, B succeeds.
#[tokio::test]
async fn test_lock_conflict_two_clients() {
    let daemon = TestDaemon::start();
    let client = reqwest::Client::new();
    let project = "/test/lock-conflict";
    let resource = "src/main.rs";

    // Client A.
    let sid_a = init_session(&client, &daemon.url()).await;
    let _sangha_a = register_session(&client, &daemon.url(), &sid_a, project, 2).await;

    // Client B.
    let sid_b = init_session(&client, &daemon.url()).await;
    let _sangha_b = register_session(&client, &daemon.url(), &sid_b, project, 2).await;

    // A claims the lock.
    let claim_a = call_tool(
        &client,
        &daemon.url(),
        &sid_a,
        "resource_claim",
        json!({ "resource": resource }),
        3,
    )
    .await;
    assert_eq!(claim_a["ok"].as_bool(), Some(true), "A should claim successfully");

    // B tries to claim the same lock — should fail.
    let claim_b = call_tool(
        &client,
        &daemon.url(),
        &sid_b,
        "resource_claim",
        json!({ "resource": resource }),
        3,
    )
    .await;
    assert_eq!(claim_b["ok"].as_bool(), Some(false), "B should be denied");
    assert!(
        claim_b.get("held_by").is_some(),
        "B should see held_by info: {claim_b:#}"
    );

    // A releases the lock.
    let release_a = call_tool(
        &client,
        &daemon.url(),
        &sid_a,
        "resource_release",
        json!({ "resource": resource }),
        4,
    )
    .await;
    assert_eq!(release_a["ok"].as_bool(), Some(true));

    // B retries — should succeed now.
    let claim_b2 = call_tool(
        &client,
        &daemon.url(),
        &sid_b,
        "resource_claim",
        json!({ "resource": resource }),
        4,
    )
    .await;
    assert_eq!(
        claim_b2["ok"].as_bool(),
        Some(true),
        "B should succeed after A released"
    );
}

/// 5. Inbox between clients: A broadcasts, B reads.
#[tokio::test]
async fn test_inbox_between_clients() {
    let daemon = TestDaemon::start();
    let client = reqwest::Client::new();
    let project = "/test/inbox";

    // Client A.
    let sid_a = init_session(&client, &daemon.url()).await;
    let _sangha_a = register_session(&client, &daemon.url(), &sid_a, project, 2).await;

    // Client B.
    let sid_b = init_session(&client, &daemon.url()).await;
    let _sangha_b = register_session(&client, &daemon.url(), &sid_b, project, 2).await;

    // A broadcasts a message.
    let bcast = call_tool(
        &client,
        &daemon.url(),
        &sid_a,
        "broadcast",
        json!({ "message": "hello from A", "tags": ["test"] }),
        3,
    )
    .await;
    assert_eq!(bcast["ok"].as_bool(), Some(true));
    assert!(bcast["message_id"].as_i64().is_some());

    // B reads inbox.
    let inbox = call_tool(
        &client,
        &daemon.url(),
        &sid_b,
        "read_inbox",
        json!({}),
        3,
    )
    .await;
    let messages = inbox["messages"].as_array().expect("messages array");
    assert!(
        messages.iter().any(|m| m["message"].as_str() == Some("hello from A")),
        "B should see A's broadcast: {inbox:#}"
    );
}

/// 6. Identity binding: calling session_heartbeat before session_register errors.
#[tokio::test]
async fn test_identity_binding() {
    let daemon = TestDaemon::start();
    let client = reqwest::Client::new();
    let sid = init_session(&client, &daemon.url()).await;

    // Call heartbeat without registering first — should produce an error.
    let resp = call_tool_raw(
        &client,
        &daemon.url(),
        &sid,
        "session_heartbeat",
        json!({}),
        2,
    )
    .await;

    // The response should carry a JSON-RPC error (identity not bound).
    assert!(
        resp.get("error").is_some(),
        "expected a JSON-RPC error for unbound identity, got: {resp:#}"
    );
}

/// 7. Multiple concurrent sessions: 3 clients register, session_list shows all 3.
#[tokio::test]
async fn test_multiple_concurrent_sessions() {
    let daemon = TestDaemon::start();
    let client = reqwest::Client::new();
    let project = "/test/multi";

    let mut sangha_ids = Vec::new();
    let mut mcp_sids = Vec::new();

    for _ in 0..3 {
        let sid = init_session(&client, &daemon.url()).await;
        let sangha_id = register_session(&client, &daemon.url(), &sid, project, 2).await;
        sangha_ids.push(sangha_id);
        mcp_sids.push(sid);
    }

    // Any session can list — use the first.
    let list = call_tool(
        &client,
        &daemon.url(),
        &mcp_sids[0],
        "session_list",
        json!({ "project": project }),
        10,
    )
    .await;

    let sessions = list["sessions"].as_array().expect("sessions array");
    let listed_ids: Vec<&str> = sessions
        .iter()
        .filter_map(|s| s["session_id"].as_str())
        .collect();

    for expected in &sangha_ids {
        assert!(
            listed_ids.contains(&expected.as_str()),
            "session {expected} should appear in list; got: {listed_ids:?}"
        );
    }
}
