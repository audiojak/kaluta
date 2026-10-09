//! Mailbox mode (spec §10.1): the real binary, driven over MCP stdio, for
//! one agent mailbox in a scratch data directory. The "app" is a core in
//! this test process on fake agent mail: closed (no socket, no sync) or
//! running (its socket served). Nothing reaches a real service, no real
//! agent runs, and the real data directory and Keychain are never read.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::executor::block_on;
use openagc_core::{
    AgentEventInfo, AgentSendMode, AgentService, Core, CoreConfig, CoreError, CoreEvent, EventListener, GuideCheck,
    GuideCheckKind, GuideEdit, GuideEntryFields, GuideKind, GuideScope, GuideSource, GuideStatus, SecretStore,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

#[derive(Default)]
struct Secrets(Mutex<std::collections::HashMap<String, String>>);

impl SecretStore for Secrets {
    fn get(&self, key: String) -> Result<Option<String>, CoreError> {
        Ok(self.0.lock().unwrap().get(&key).cloned())
    }
    fn set(&self, key: String, value: String) -> Result<(), CoreError> {
        self.0.lock().unwrap().insert(key, value);
        Ok(())
    }
    fn delete(&self, key: String) -> Result<(), CoreError> {
        self.0.lock().unwrap().remove(&key);
        Ok(())
    }
}

/// The approvals the app was asked for: (ticket, summary).
#[derive(Default)]
struct Proposals(Mutex<Vec<(i64, String)>>);

impl EventListener for Proposals {
    fn on_event(&self, _account: Option<String>, event: CoreEvent) {
        if let CoreEvent::AgentEvents { events, .. } = event {
            for e in events {
                if let AgentEventInfo::ActionProposed { action_id, summary, .. } = e {
                    self.0.lock().unwrap().push((action_id, summary));
                }
            }
        }
    }
}

struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..400 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

/// The app's core and its agent mailbox, with one received thread and a
/// guide rule against "circle back".
struct App {
    scratch: Scratch,
    core: Arc<Core>,
    proposals: Arc<Proposals>,
    account: String,
    address: String,
}

impl App {
    async fn new(name: &str) -> Self {
        // Short, so the socket path needs no fallback directory.
        let dir = std::env::temp_dir().join(format!("oagc-mbx-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let proposals = Arc::new(Proposals::default());
        let core = Core::new(
            CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
            Arc::new(Secrets::default()),
            proposals.clone(),
        )
        .unwrap();
        core.debug_use_fake_agents();
        core.debug_use_fake_agent_mail(true);
        let agent = core
            .clone()
            .create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, format!("req-{name}"))
            .await
            .unwrap();
        core.clone().set_current_account(agent.account_id.clone()).await.unwrap();
        core.clone().start_sync().unwrap();
        core.debug_deliver_to_agent_mailbox(
            agent.account_id.clone(),
            "ada@example.com".into(),
            "Lunch?".into(),
            "Free Friday?".into(),
        )
        .unwrap();
        let reader = core.clone();
        wait_for("the delivered mail", || {
            block_on(reader.list_threads("INBOX".into(), None, 10)).map(|p| p.rows.len()).unwrap_or(0) == 1
        })
        .await;
        core.stop_sync();
        core.apply_guide_edits(
            vec![GuideEdit::Add {
                fields: GuideEntryFields {
                    category: "B6".into(),
                    kind: GuideKind::Rule,
                    statement: "Never say circle back".into(),
                    scope: GuideScope::default(),
                    check: Some(GuideCheck { kind: GuideCheckKind::BannedPhrase, value: "circle back".into() }),
                },
                status: GuideStatus::Accepted,
                source: GuideSource::You,
                origin: None,
            }],
            "test".into(),
        )
        .await
        .unwrap();
        Self { scratch: Scratch(dir), core, proposals, account: agent.account_id, address: agent.address }
    }

    fn sends(&self) -> u32 {
        // The fake mailbox holds the delivered message and every send.
        self.core.debug_agent_mailbox_message_count(self.account.clone()).saturating_sub(1)
    }

    async fn queued(&self) -> u32 {
        self.core.outbox_status().await.unwrap().pending
    }
}

struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Mcp {
    async fn start(app: &App, mailbox: &str) -> Self {
        let mut mcp = Self::spawn(app, mailbox);
        let init = mcp
            .request(
                "initialize",
                json!({ "protocolVersion": "2025-06-18", "capabilities": {},
                        "clientInfo": { "name": "claude-code", "version": "2.1" } }),
            )
            .await;
        assert_eq!(init["result"]["serverInfo"]["name"], "openagc");
        assert!(init["result"]["instructions"].as_str().unwrap().contains("agent mailbox"));
        mcp.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
        mcp
    }

    fn spawn(app: &App, mailbox: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_openagc-mcp"))
            .args(["--mailbox", mailbox, "--data-dir", app.scratch.0.to_str().unwrap()])
            // Nothing here may find the real home.
            .env("HOME", &app.scratch.0)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self { child, stdin, stdout, next: 1 }
    }

    async fn send(&mut self, message: Value) {
        self.stdin.write_all(format!("{message}\n").as_bytes()).await.unwrap();
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await;
        loop {
            let mut line = String::new();
            let n = tokio::time::timeout(Duration::from_secs(20), self.stdout.read_line(&mut line))
                .await
                .expect("answered in time")
                .unwrap();
            assert!(n > 0, "openagc-mcp closed stdout");
            let v: Value = serde_json::from_str(&line).unwrap();
            if v["id"] == id {
                return v;
            }
        }
    }

    /// A tool's structured result, or its error text.
    async fn call(&mut self, tool: &str, arguments: Value) -> Result<Value, String> {
        let r = self.request("tools/call", json!({ "name": tool, "arguments": arguments })).await;
        let result = &r["result"];
        if result["isError"] == true {
            Err(result["content"][0]["text"].as_str().unwrap_or_default().to_owned())
        } else {
            Ok(result["structuredContent"].clone())
        }
    }

    async fn reads(&mut self) -> (String, String) {
        let list = self.request("tools/list", json!({})).await;
        let names: Vec<&str> =
            list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["guide_rules", "facts_lookup", "mail_search", "mail_get_thread", "mail_send", "mail_reply"]);
        let send = &list["result"]["tools"][4];
        assert_eq!(send["annotations"]["readOnlyHint"], false);
        assert_eq!(send["inputSchema"]["required"], json!(["to", "subject", "body_markdown"]));

        let guide =
            self.call("guide_rules", json!({ "to": ["ada@example.com"], "message_type": "reply" })).await.unwrap();
        assert!(guide["writing_guide"].as_str().unwrap().contains("circle back"), "{guide}");
        assert!(guide["sends_as"].as_str().unwrap().starts_with("Scout <"));
        let facts = self.call("facts_lookup", json!({ "category": "Work" })).await.unwrap();
        assert!(facts.is_object(), "{facts}");
        let found = self.call("mail_search", json!({ "query": "lunch" })).await.unwrap();
        assert_eq!(found["threads"][0]["subject"], "Lunch?");
        let thread = found["threads"][0]["thread_id"].as_str().unwrap().to_owned();
        let read = self.call("mail_get_thread", json!({ "thread_id": thread })).await.unwrap();
        assert_eq!(read["messages"][0]["body"], "Free Friday?");
        let message = read["messages"][0]["message_id"].as_str().unwrap().to_owned();
        assert!(
            self.call("mail_archive", json!({ "thread_ids": [thread] })).await.unwrap_err().starts_with("unknown_tool")
        );
        (thread, message)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn with_the_app_closed_every_tool_works_and_sends_wait_in_the_outbox_for_the_app() {
    let app = App::new("closed").await;
    let mut mcp = Mcp::start(&app, &app.address).await;
    let (_, message) = mcp.reads().await;

    let sent = mcp
        .call(
            "mail_send",
            json!({ "to": ["ada@example.com"], "subject": "Hello", "body_markdown": "Let's circle back." }),
        )
        .await
        .unwrap();
    assert_eq!(sent["queued"], true);
    assert_eq!(sent["message"], openagc_core::QUEUED_MESSAGE);
    assert!(sent["guide_check"][0].as_str().unwrap().contains("circle back"), "the guide check reaches the agent");
    let replied =
        mcp.call("mail_reply", json!({ "message_id": message, "body_markdown": "Friday works." })).await.unwrap();
    assert_eq!(replied["queued"], true);
    assert_eq!(replied["guide_check"], json!([]));
    assert_eq!(app.queued().await, 2, "each queued once");
    assert_eq!(app.sends(), 0, "nothing sent from this process");

    // Asking before each send needs the app.
    app.core.set_agent_send_mode(app.account.clone(), AgentSendMode::Ask).unwrap();
    let asked =
        mcp.call("mail_send", json!({ "to": ["ada@example.com"], "subject": "Hi", "body_markdown": "Hi" })).await;
    assert!(asked.unwrap_err().starts_with("needs_openagc: This mailbox is set to ask before each send"));
    app.core.set_agent_send_mode(app.account.clone(), AgentSendMode::Freely).unwrap();
    drop(mcp);

    // The app opens: its sync sends both, once each.
    app.core.clone().start_sync().unwrap();
    wait_for("the queued sends", || app.sends() == 2).await;
    wait_for("the outbox to empty", || block_on(app.queued()) == 0).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(app.sends(), 2, "sent exactly once each");
    let log = app.core.list_agent_actions(20).await.unwrap();
    assert!(log.iter().all(|a| a.session_id.starts_with("outside-claude-code-")), "{log:?}");
    assert_eq!(log.iter().filter(|a| a.tool == "mail_send" && a.state == "done").count(), 2);
    app.core.stop_sync();
}

#[tokio::test(flavor = "multi_thread")]
async fn with_the_app_running_every_tool_goes_through_it_and_asking_is_the_apps() {
    let app = App::new("running").await;
    app.core.clone().start_sync().unwrap();
    app.core.clone().serve_outside_agents().unwrap();
    let mut mcp = Mcp::start(&app, &app.address).await;
    let (_, message) = mcp.reads().await;
    // The reads went through the app: its activity log has them.
    let log = app.core.list_agent_actions(20).await.unwrap();
    for tool in ["guide_rules", "facts_lookup", "mail_search", "mail_get_thread"] {
        assert!(log.iter().any(|a| a.tool == tool && a.session_id.starts_with("outside-claude-code-")), "{tool}");
    }

    // Send freely: sent now, the guide check returned.
    let sent = mcp
        .call(
            "mail_send",
            json!({ "to": ["ada@example.com"], "subject": "Hello", "body_markdown": "Let's circle back." }),
        )
        .await
        .unwrap();
    assert_eq!(sent["sent"], true);
    assert!(sent["guide_check"][0].as_str().unwrap().contains("circle back"));
    wait_for("the send", || app.sends() == 1).await;

    // Ask before each send: the app asks the user, saying who wants to.
    app.core.set_agent_send_mode(app.account.clone(), AgentSendMode::Ask).unwrap();
    let (core, proposals) = (app.core.clone(), app.proposals.clone());
    let approver = tokio::spawn(async move {
        wait_for("the proposal", || !proposals.0.lock().unwrap().is_empty()).await;
        let (ticket, summary) = proposals.0.lock().unwrap()[0].clone();
        core.resolve_agent_action(ticket, true).unwrap();
        summary
    });
    let replied =
        mcp.call("mail_reply", json!({ "message_id": message, "body_markdown": "Friday works." })).await.unwrap();
    let summary = approver.await.unwrap();
    assert!(summary.starts_with(&format!("Claude Code outside OpenAGC, as {}: Send", app.address)), "{summary}");
    assert_eq!(replied["sent"], true);
    wait_for("the approved reply", || app.sends() == 2).await;
    app.core.stop_sync();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_users_own_accounts_and_unknown_addresses_are_refused_at_start() {
    let app = App::new("refuse").await;
    app.core.clone().debug_add_demo_account("mine".into(), "me@gmail.com".into(), None, 1).await.unwrap();
    for (address, says) in [("me@gmail.com", "one of your own accounts"), ("who@example.com", "no agent mailbox")] {
        let mut mcp = Mcp::spawn(&app, address);
        let status = tokio::time::timeout(Duration::from_secs(10), mcp.child.wait()).await.unwrap().unwrap();
        assert_eq!(status.code(), Some(2));
        let mut err = String::new();
        mcp.child.stderr.take().unwrap().read_to_string(&mut err).await.unwrap();
        assert!(err.contains(says), "{err}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn with_the_app_closed_a_store_of_another_schema_is_refused_and_left_alone() {
    let app = App::new("schema").await;
    let path = app.scratch.0.join("accounts").join(&app.account).join("mail.sqlite");
    let older = mail_store::schema_version() - 1;
    // As if this binary were newer than the app that last opened the store.
    mail_store::Connection::open(&path).unwrap().execute_batch(&format!("PRAGMA user_version = {older}")).unwrap();
    let mut mcp = Mcp::start(&app, &app.address).await;
    let err = mcp.call("mail_search", json!({ "query": "" })).await.unwrap_err();
    assert!(err.contains("older OpenAGC") && err.contains("open OpenAGC once"), "{err}");
    drop(mcp);
    let version: u32 =
        mail_store::Connection::open(&path).unwrap().pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!(version, older, "not migrated");
}
