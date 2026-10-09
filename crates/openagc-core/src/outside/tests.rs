//! Mailbox mode (spec §10.1) against fake agent mail: the headless core
//! with the app closed, and the app's core over its socket. Nothing reaches
//! a real service, and no real agent runs.

use std::sync::Mutex;
use std::time::Duration;

use futures::executor::block_on;
use serde_json::json;

use super::*;
use crate::agent_mailbox::AgentService;
use crate::guide::{
    GuideCheck, GuideCheckKind, GuideEdit, GuideEntryFields, GuideKind, GuideScope, GuideSource, GuideStatus,
};
use crate::{AgentEventInfo, CoreConfig, CoreEvent, EventListener};

#[derive(Default)]
pub(crate) struct Events(Mutex<Vec<(String, AgentEventInfo)>>);

impl EventListener for Events {
    fn on_event(&self, _account: Option<String>, event: CoreEvent) {
        if let CoreEvent::AgentEvents { session_id, events } = event {
            let mut all = self.0.lock().unwrap();
            all.extend(events.into_iter().map(|e| (session_id.clone(), e)));
        }
    }
}

impl Events {
    pub(crate) fn proposals(&self) -> Vec<(String, i64, String)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|(s, e)| match e {
                AgentEventInfo::ActionProposed { action_id, summary, .. } => {
                    Some((s.clone(), *action_id, summary.clone()))
                }
                _ => None,
            })
            .collect()
    }
}

struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..400 {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {what}");
}

/// The app's core on a scratch data directory, with an agent mailbox that
/// has one received thread, a guide rule banning "circle back", and a
/// finished learning run (so AI compositions are recorded, ADR 0013).
pub(crate) struct App {
    _dir: Temp,
    pub(crate) data_dir: String,
    pub(crate) core: Arc<Core>,
    pub(crate) events: Arc<Events>,
    pub(crate) account: String,
    pub(crate) address: String,
}

pub(crate) fn app(name: &str) -> App {
    // Short: the socket path must fit macOS's limit without the fallback.
    let dir = std::env::temp_dir().join(format!("oagc-out-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let events = Arc::new(Events::default());
    let data_dir = dir.to_string_lossy().into_owned();
    let core = Core::new(
        CoreConfig { data_dir: data_dir.clone(), log_dir: None },
        Arc::new(crate::secrets::MemorySecrets::default()),
        events.clone(),
    )
    .unwrap();
    core.debug_use_fake_agents();
    core.debug_use_fake_agent_mail(true);
    let agent = block_on(core.clone().create_agent_mailbox(
        AgentService::Primitive,
        "Scout".into(),
        None,
        format!("req-{name}"),
    ))
    .unwrap();
    block_on(core.clone().set_current_account(agent.account_id.clone())).unwrap();
    core.clone().start_sync().unwrap();
    core.debug_deliver_to_agent_mailbox(
        agent.account_id.clone(),
        "ada@example.com".into(),
        "Lunch?".into(),
        "Free Friday?".into(),
    )
    .unwrap();
    wait_for("the delivered mail", || {
        block_on(core.list_threads("INBOX".into(), None, 10)).map(|p| p.rows.len()).unwrap_or(0) == 1
    });
    core.stop_sync();
    block_on(core.apply_guide_edits(
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
    ))
    .unwrap();
    core.db()
        .unwrap()
        .write_blocking(|tx| {
            let run = mail_store::guide::create_run(tx, "learn", None, None, &[], 1, 1)?;
            mail_store::guide::set_run_status(tx, run, "done", None, 2)
        })
        .unwrap();
    App { _dir: Temp(dir), data_dir, core, events, account: agent.account_id, address: agent.address }
}

pub(crate) fn call(core: &Arc<Core>, session: &str, tool: MailboxTool, args: Value) -> Outcome {
    crate::runtime::runtime().block_on(core.call_outside(session, tool, args))
}

pub(crate) fn ok(outcome: Outcome) -> Value {
    match outcome {
        Outcome::Ok { structured: Some(v), .. } => v,
        other => panic!("{other:?}"),
    }
}

pub(crate) fn error_code(outcome: Outcome) -> String {
    match outcome {
        Outcome::Error { code, .. } => code,
        other => panic!("expected an error, got {other:?}"),
    }
}

pub(crate) fn outbox_sends(app: &App) -> i64 {
    let db = block_on(app.core.store_for(&app.account)).unwrap();
    db.read_blocking(|c| Ok(c.query_row("SELECT COUNT(*) FROM outbox WHERE kind = 'send'", [], |r| r.get(0))?)).unwrap()
}

pub(crate) fn drafts(app: &App) -> usize {
    let db = block_on(app.core.store_for(&app.account)).unwrap();
    db.read_blocking(mail_store::drafts::list).unwrap().len()
}

#[test]
fn only_agent_mailboxes_are_served_to_outside_agents() {
    let app = app("resolve");
    assert_eq!(app.core.outside_mailbox_account(&app.address.to_uppercase()).unwrap(), app.account);
    block_on(app.core.clone().debug_add_demo_account("mine".into(), "me@gmail.com".into(), None, 1)).unwrap();
    let own = app.core.outside_mailbox_account("me@gmail.com").unwrap_err();
    assert_eq!(own.kind(), ErrorKind::PermissionDenied);
    assert!(own.to_string().contains("one of your own accounts"), "{own}");
    let unknown = app.core.outside_mailbox_account("nobody@example.com").unwrap_err();
    assert!(unknown.to_string().contains("no agent mailbox nobody@example.com"), "{unknown}");
    // The headless core refuses them the same way, reading files only.
    let headless = Core::headless(&app.data_dir).unwrap();
    assert!(block_on(headless.open_outside_session("me@gmail.com", "claude-code")).is_err());
    assert!(headless.open_accounts.read().unwrap().stores.is_empty(), "no store was opened");
}

#[test]
fn with_the_app_closed_reads_stay_read_only_and_a_send_is_queued_once_for_the_app() {
    let app = app("closed");
    let headless = Core::headless(&app.data_dir).unwrap();
    assert!(headless.is_headless());
    let session =
        crate::runtime::runtime().block_on(headless.open_outside_session(&app.address, "claude-code")).unwrap();
    assert!(session.starts_with("outside-claude-code-"), "{session}");

    // Reads: the guide, facts and mail, without opening the store for writing.
    let guide = ok(call(&headless, &session, MailboxTool::GuideRules, json!({ "to": ["ada@example.com"] })));
    assert!(guide["writing_guide"].as_str().unwrap().contains("circle back"), "{guide}");
    assert_eq!(guide["sends_as"], format!("Scout <{}>", app.address));
    assert_eq!(guide["send_mode"], "send_freely");
    assert!(guide["about"].as_str().unwrap().contains("exactly one recipient"));
    let facts = ok(call(&headless, &session, MailboxTool::FactsLookup, json!({})));
    assert!(facts.is_object());
    let found = ok(call(&headless, &session, MailboxTool::Search, json!({ "query": "" })));
    let thread = found["threads"][0]["thread_id"].as_str().unwrap().to_owned();
    assert_eq!(found["threads"][0]["subject"], "Lunch?");
    let read = ok(call(&headless, &session, MailboxTool::GetThread, json!({ "thread_id": thread })));
    let message = read["messages"][0]["message_id"].as_str().unwrap().to_owned();
    assert_eq!(read["messages"][0]["body"], "Free Friday?");
    let store = block_on(headless.store_for(&app.account)).unwrap();
    assert!(!store.writer_opened(), "reading never opened the store for writing");

    // A send: checked against the guide, recorded, queued for the app.
    let queued = ok(call(
        &headless,
        &session,
        MailboxTool::Reply,
        json!({ "message_id": message, "body_markdown": "Yes! Let's circle back on Friday." }),
    ));
    assert_eq!(queued["queued"], true);
    assert_eq!(queued["message"], QUEUED_MESSAGE);
    assert!(queued.get("sent").is_none());
    assert!(queued["guide_check"][0].as_str().unwrap().contains("circle back"), "the agent is told: {queued}");
    assert!(queued["writing_guide_breaches"].as_str().unwrap().contains("circle back"));
    assert_eq!(outbox_sends(&app), 1, "queued exactly once");
    let db = app.core.db().unwrap();
    let agent: Option<String> =
        db.read_blocking(|c| Ok(c.query_row("SELECT agent FROM ai_compositions", [], |r| r.get(0))?)).unwrap();
    assert_eq!(agent.as_deref(), Some("outside:claude-code"), "recorded as AI-written");
    // The activity log names the outside agent; reads were not written.
    let log = block_on(app.core.list_agent_actions(20)).unwrap();
    assert!(log.iter().all(|a| a.session_id == session), "{log:?}");
    let tools: Vec<&str> = log.iter().rev().map(|a| a.tool.as_str()).collect();
    assert_eq!(tools, ["mail_create_draft", "mail_send"]);
    assert!(log[0].result_summary.as_deref().is_some_and(|s| s.contains("Breaks your writing guide")));

    // Asking before each send cannot be answered with the app closed.
    app.core.set_agent_send_mode(app.account.clone(), AgentSendMode::Ask).unwrap();
    let asked = call(
        &headless,
        &session,
        MailboxTool::Send,
        json!({ "to": ["ada@example.com"], "subject": "Hi", "body_markdown": "Hello" }),
    );
    assert_eq!(error_code(asked), "needs_openagc");
    app.core.set_agent_send_mode(app.account.clone(), AgentSendMode::Freely).unwrap();
    // Primitive's one recipient: refused, and no draft is left behind.
    let drafts_before = drafts(&app);
    let two = call(
        &headless,
        &session,
        MailboxTool::Send,
        json!({ "to": ["ada@example.com", "bob@example.com"], "subject": "Hi", "body_markdown": "Hello" }),
    );
    assert_eq!(error_code(two), "invalid_arguments");
    assert_eq!(drafts(&app), drafts_before);
    assert_eq!(outbox_sends(&app), 1, "nothing more queued");
    crate::runtime::runtime().block_on(headless.close_outside_session(&session));

    // The app opens and sends it, once.
    app.core.clone().start_sync().unwrap();
    let fake = app.core.fake_agent_mailbox(&app.account).unwrap();
    wait_for("the queued send", || fake.message_count() == 2);
    wait_for("the outbox to empty", || outbox_sends(&app) == 0);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(fake.message_count(), 2, "sent exactly once");
    app.core.stop_sync();
}

#[test]
fn the_headless_core_refuses_a_store_of_another_schema_in_words() {
    let app = app("schema");
    let newer = mail_store::schema_version() + 1;
    let db = app.core.db().unwrap();
    db.write_blocking(move |tx| Ok(tx.execute_batch(&format!("PRAGMA user_version = {newer}"))?)).unwrap();
    app.core.close_store(&app.account);
    let headless = Core::headless(&app.data_dir).unwrap();
    let err = block_on(headless.open_outside_session(&app.address, "codex")).unwrap_err();
    assert!(err.to_string().contains("newer OpenAGC"), "{err}");
}

#[test]
fn with_the_app_running_an_outside_agent_goes_through_its_core_and_asks_as_its_own_agents_do() {
    let app = app("running");
    app.core.clone().start_sync().unwrap();
    app.core.clone().serve_outside_agents().unwrap();
    let socket = outside_socket(std::path::Path::new(&app.data_dir)).expect("the app says where its socket is");
    let rt = crate::runtime::runtime();
    // Only agent mailboxes, here too.
    block_on(app.core.clone().debug_add_demo_account("mine".into(), "me@gmail.com".into(), None, 1)).unwrap();
    let refused = rt.block_on(agent_mcp::ShimClient::connect_mailbox(&socket, "me@gmail.com", "codex"));
    assert!(refused.err().is_some_and(|e| e.to_string().contains("one of your own accounts")));

    let client = rt.block_on(agent_mcp::ShimClient::connect_mailbox(&socket, &app.address, "claude-code")).unwrap();
    let call = |tool: &str, args: Value| rt.block_on(client.call(tool, args));
    let guide = ok(call("guide_rules", json!({})));
    assert!(guide["writing_guide"].as_str().unwrap().contains("circle back"));
    let found = ok(call("mail_search", json!({ "query": "lunch" })));
    assert_eq!(found["threads"].as_array().unwrap().len(), 1);
    assert_eq!(error_code(call("mail_archive", json!({ "thread_ids": ["x"] }))), "unknown_tool", "mailbox tools only");

    // Send freely: it goes, and the guide check comes back.
    let fake = app.core.fake_agent_mailbox(&app.account).unwrap();
    let sent = ok(call(
        "mail_send",
        json!({ "to": ["ada@example.com"], "subject": "Hi", "body_markdown": "Let's circle back." }),
    ));
    assert_eq!(sent["sent"], true);
    assert!(sent["guide_check"][0].as_str().unwrap().contains("circle back"));
    wait_for("the send", || fake.message_count() == 2);
    let log = block_on(app.core.list_agent_actions(20)).unwrap();
    assert!(log.iter().any(|a| a.tool == "guide_rules" && a.session_id.starts_with("outside-claude-code-")));

    // Ask before each send: the approval is the app's, with who asks.
    app.core.set_agent_send_mode(app.account.clone(), AgentSendMode::Ask).unwrap();
    let answer = |approve: bool| {
        let (core, events) = (app.core.clone(), app.events.clone());
        let seen = events.proposals().len();
        std::thread::spawn(move || {
            wait_for("the proposal", || events.proposals().len() > seen);
            let (_, ticket, summary) = events.proposals().pop().unwrap();
            core.resolve_agent_action(ticket, approve).unwrap();
            summary
        })
    };
    let approver = answer(true);
    let approved =
        ok(call("mail_send", json!({ "to": ["ada@example.com"], "subject": "Again", "body_markdown": "Hello" })));
    let summary = approver.join().unwrap();
    assert!(
        summary.starts_with(&format!("Claude Code outside OpenAGC, as {}: Send “Again”", app.address)),
        "{summary}"
    );
    assert!(summary.ends_with("\n\nHello"), "the message is in the card: {summary:?}");
    assert_eq!(approved["sent"], true);
    wait_for("the approved send", || fake.message_count() == 3);
    let drafts_before = drafts(&app);
    let decliner = answer(false);
    let declined = call("mail_send", json!({ "to": ["ada@example.com"], "subject": "No", "body_markdown": "Hello" }));
    decliner.join().unwrap();
    assert_eq!(error_code(declined), "rejected_by_user");
    assert_eq!(drafts(&app), drafts_before, "a declined send leaves no draft");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(fake.message_count(), 3);
    app.core.stop_sync();
}
