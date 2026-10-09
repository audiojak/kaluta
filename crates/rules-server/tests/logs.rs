//! What the server logs: one line per request naming the token's id, and
//! never a token, an address it was asked about or what a snapshot says.
//! Its own test binary, as it installs the process's subscriber.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use common::{MAILBOX, call, snapshot, start};
use serde_json::json;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_name_token_ids_and_nothing_secret() {
    let captured = Captured::default();
    let writer = captured.clone();
    // As `openagc-rules` sets it, with the server's own lines at debug.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn,rules_server=debug"))
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .init();

    let s = start(0, None).await;
    let publisher = s.registered().await;
    let (status, _) = s.publish(&publisher, None, snapshot(1).to_json().unwrap()).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let (id, agent) = s.mint(&publisher, "Routine").await;
    let r = s.get(Some(&agent), &format!("/v1/m/{MAILBOX}/guide?to=bea@globex.com")).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let client = s.mcp(&agent).await.unwrap();
    call(&client, "guide_rules", json!({ "to": ["ann@acme.com"] })).await;
    let _ = s.get(Some("oagc_agt_0123456789abcdef_guess"), "/v1/m/x@y.z/facts").await;

    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains(&format!("token=\"agent:{id}\"")), "{log}");
    assert!(log.contains("route=\"/v1/m/{address}/guide\"") && log.contains("route=\"/mcp\""), "{log}");
    assert!(log.contains(&format!("tool call token={id} tool=\"guide_rules\"")), "{log}");
    assert!(log.contains("status=401"), "{log}");
    for secret in [publisher.as_str(), agent.as_str(), "oagc_agt_0123456789abcdef_guess"] {
        assert!(!log.contains(secret), "a token was logged: {log}");
    }
    for content in ["bea@globex.com", "ann@acme.com", "circle back", "cal.com/scout", "Scout", MAILBOX] {
        assert!(!log.contains(content), "{content:?} was logged: {log}");
    }
}
