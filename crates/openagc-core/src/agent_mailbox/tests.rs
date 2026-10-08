//! Agent mailboxes against an in-memory service, and against a wiremock
//! fake of Primitive's API. Nothing reaches a real service.

use std::time::Duration;

use futures::executor::block_on;
use mail_domain::{EmailAddress, LabelId, MessageId, ThreadId};
use provider_api::{FetchedBody, FetchedMessage};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::{CoreConfig, CoreEvent, EventListener};

struct Silent;
impl EventListener for Silent {
    fn on_event(&self, _account: Option<String>, _event: CoreEvent) {}
}

struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn core(name: &str) -> (Temp, Arc<Core>, Arc<crate::secrets::MemorySecrets>) {
    let dir = std::env::temp_dir().join(format!("openagc-agent-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let secrets = Arc::new(crate::secrets::MemorySecrets::default());
    let core = Core::new(
        CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
        secrets.clone(),
        Arc::new(Silent),
    )
    .unwrap();
    (Temp(dir), core, secrets)
}

fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..400 {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {what}");
}

fn inbox_rows(core: &Core) -> usize {
    block_on(core.list_threads("INBOX".into(), None, 50)).map(|p| p.rows.len()).unwrap_or(0)
}

#[test]
fn the_agents_prompt_says_whose_mailbox_it_is_and_the_one_recipient_rule() {
    let meta = AgentMeta {
        kind: "agent".into(),
        service: AgentService::Primitive,
        address: "scout@abc.primitive.email".into(),
        name: "Scout".into(),
        created_at: 0,
        send_mode: AgentSendMode::Freely,
        managed_address: None,
        service_account: "a1".into(),
        inbox_id: None,
    };
    let limits = limits_text(AgentService::Primitive, false, None);
    let prompt = agent_prompt(&meta, &[], &limits);
    assert!(prompt.contains("goes out as Scout <scout@abc.primitive.email>"));
    assert!(prompt.contains("exactly one recipient"));
    assert!(!prompt.contains("sending limits"), "alone, nothing is shared");
    let shared = agent_prompt(&meta, &["Writer".into()], &limits);
    assert!(shared.contains("shares its service account's sending limits"), "{shared}");
    assert!(shared.contains("another agent, Writer"));
    assert!(
        agent_prompt(&meta, &["Writer".into(), "Clerk".into()], &limits).contains("2 other agents (Writer, Clerk)")
    );
}

#[test]
fn a_six_digit_code_is_found_on_its_own() {
    assert_eq!(find_code("Your code is 482913.").as_deref(), Some("482913"));
    assert_eq!(find_code("Code: 482913\nexpires in 10 minutes").as_deref(), Some("482913"));
    assert_eq!(find_code("Order 1234567 and ref A123456"), None);
    assert_eq!(find_code("no code"), None);
}

#[test]
fn creating_a_mailbox_registers_an_agent_account_with_its_key_in_the_keychain() {
    let (_t, core, secrets) = core("create");
    core.debug_use_fake_agent_mail(true);
    let created = block_on(core.clone().create_agent_mailbox(
        AgentService::Primitive,
        "  Research   Scout ".into(),
        None,
        "req-1".into(),
    ))
    .unwrap();
    assert_eq!(created.address, "research-scout@demo.primitive.email");
    assert!(!created.plan.verified && created.plan.reply_only);
    let key = secrets.0.lock().unwrap().get(&keys::mailbox_api_key(&created.account_id)).cloned();
    assert!(key.is_some_and(|k| k.starts_with("fake_")));
    assert!(core.account_is_agent(created.account_id.clone()));
    assert!(core.account_has_credentials(created.account_id.clone()).unwrap());
    assert_eq!(core.agent_mailbox_api_key(created.account_id.clone()).unwrap(), key_of(&secrets, &created.account_id));

    let accounts = block_on(core.list_accounts()).unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].kind, AccountKind::Agent);
    assert_eq!(accounts[0].service, Some(AgentService::Primitive));
    assert_eq!(accounts[0].display_name.as_deref(), Some("Research Scout"));
    assert_eq!(accounts[0].email, created.address);

    // Renaming renames the agent; an empty name is refused.
    block_on(core.rename_account(created.account_id.clone(), "Scout".into())).unwrap();
    assert_eq!(core.agent_name(&created.account_id).as_deref(), Some("Scout"));
    assert!(block_on(core.rename_account(created.account_id.clone(), " ".into())).is_err());

    // Removing it forgets the key.
    block_on(core.remove_account(created.account_id.clone())).unwrap();
    assert!(!secrets.0.lock().unwrap().contains_key(&keys::mailbox_api_key(&created.account_id)));
    assert!(block_on(core.list_accounts()).unwrap().is_empty());
}

fn key_of(secrets: &crate::secrets::MemorySecrets, account: &str) -> String {
    secrets.0.lock().unwrap().get(&keys::mailbox_api_key(account)).cloned().unwrap()
}

#[test]
fn a_lost_index_lists_the_agent_mailbox_again() {
    let (t, core, _secrets) = core("rescan");
    core.debug_use_fake_agent_mail(true);
    let created =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "req-2".into()))
            .unwrap();
    std::fs::remove_file(t.0.join("accounts").join("index.json")).unwrap();
    let accounts = block_on(core.list_accounts()).unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, created.account_id);
    assert_eq!(accounts[0].kind, AccountKind::Agent);
    assert_eq!(accounts[0].display_name.as_deref(), Some("Scout"));
}

#[test]
fn verifying_finds_the_code_in_the_users_own_mail_and_confirms_it() {
    let (_t, core, _secrets) = core("verify");
    core.debug_use_fake_agent_mail(true);
    let agent =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "req-3".into()))
            .unwrap();
    // Before a code is asked for, nothing is looked at.
    assert_eq!(block_on(core.find_agent_mailbox_code(agent.account_id.clone(), "me".into())).unwrap(), None);
    block_on(core.start_agent_mailbox_verification(agent.account_id.clone(), "me@example.com".into())).unwrap();

    // The user's own account receives the code.
    block_on(core.clone().open_account("me".into())).unwrap();
    let fake = Arc::new(provider_api::fake::FakeProvider::new("me@example.com", mail_sync::now_millis(), 50));
    let now = mail_sync::now_millis();
    let mail = |id: &str, from: &str, text: &str| FetchedMessage {
        id: MessageId::new(id),
        thread_id: ThreadId::new(id),
        label_ids: vec![LabelId::new("INBOX"), LabelId::new("UNREAD")],
        internal_date: now,
        from: Some(EmailAddress::new(None, from)),
        subject: "Your code".into(),
        snippet: text.into(),
        body: Some(FetchedBody { text: Some(text.into()), html: None, attachments: vec![] }),
        ..Default::default()
    };
    fake.seed(mail("spoof", "codes@primitive.dev.evil.example", "Your code is 999999"));
    fake.seed(mail("real", "no-reply@mail.primitive.dev", "Your verification code is 123456."));
    core.start_sync_with(fake).unwrap();
    wait_for("the code to arrive", || inbox_rows(&core) == 2);
    let code = block_on(core.find_agent_mailbox_code(agent.account_id.clone(), "me".into())).unwrap();
    assert_eq!(code.as_deref(), Some("123456"), "only mail from the service's domain is read");
    core.stop_sync();

    let wrong = block_on(core.verify_agent_mailbox(agent.account_id.clone(), "000000".into())).unwrap_err();
    assert_eq!(wrong.kind(), ErrorKind::InvalidInput);
    assert_eq!(wrong.to_string(), "That code is not right");
    let plan = block_on(core.verify_agent_mailbox(agent.account_id.clone(), "123456".into())).unwrap();
    assert!(plan.verified && !plan.reply_only);
    assert_eq!(plan.email.as_deref(), Some("me@example.com"));
    assert_eq!(block_on(core.agent_mailbox_plan(agent.account_id)).unwrap(), plan);
}

#[test]
fn an_agent_mailbox_syncs_with_the_other_accounts_and_sends_as_the_agent_to_one_recipient() {
    let (_t, core, _secrets) = core("sync");
    core.debug_use_fake_agent_mail(true);
    let agent =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "req-4".into()))
            .unwrap();
    block_on(core.clone().set_current_account(agent.account_id.clone())).unwrap();
    assert!(block_on(core.clone().start_all_sync()).unwrap().is_empty(), "nothing needs a sign-in");
    core.debug_deliver_to_agent_mailbox(
        agent.account_id.clone(),
        "ada@example.com".into(),
        "Hello".into(),
        "Hi".into(),
    )
    .unwrap();
    wait_for("the delivered mail", || inbox_rows(&core) == 1);

    // Two recipients are refused before anything is queued.
    let address = |email: &str| crate::ffi::AddressInfo { name: None, email: email.into() };
    let mut draft = crate::DraftInfo {
        id: 0,
        thread_id: None,
        in_reply_to_message_id: None,
        to: vec![address("ada@example.com"), address("bob@example.com")],
        cc: vec![],
        bcc: vec![],
        subject: "Hi".into(),
        body_html: "<p>Hi</p>".into(),
        quoted_html: String::new(),
        attachments: vec![],
        status: crate::DraftStatus::Editing,
        error: None,
        updated_at: 0,
    };
    let id = block_on(core.save_draft(draft.clone())).unwrap();
    let err = block_on(core.send_draft(id)).unwrap_err();
    assert_eq!(err.to_string(), provider_primitive::ONE_RECIPIENT_ONLY);

    draft.id = id;
    draft.to = vec![address("ada@example.com")];
    block_on(core.save_draft(draft)).unwrap();
    block_on(core.send_draft(id)).unwrap();
    let fake = core.fake_agent_mailbox(&agent.account_id).unwrap();
    wait_for("the send", || fake.message_count() == 2);
    core.stop_sync();
}

fn ok(data: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn limits() -> serde_json::Value {
    json!({ "storage_mb": 100, "send_per_hour": 10, "send_per_day": 50, "api_per_minute": 120,
            "webhooks_max_global": 1, "webhooks_per_domain": false, "filters_per_domain": false,
            "spam_thresholds_per_domain": false })
}

#[test]
fn a_primitive_mailbox_is_created_and_synced_over_its_api() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(MockServer::start());
    let mount = |mock: Mock| rt.block_on(mock.mount(&server));
    mount(
        Mock::given(method("POST"))
            .and(path("/agent/accounts"))
            .and(body_partial_json(json!({ "terms_accepted": true, "device_name": "Scout" })))
            .respond_with(ok(json!({
                "api_key": "prim_k", "org_id": "00000000-0000-0000-0000-000000000001",
                "address": "abc.primitive.email", "plan": "agent", "limits": limits(),
                "upgrade": { "plan": "developer", "claim_path": "/agent/claim/start" }
            }))),
    );
    mount(
        Mock::given(path("/changes"))
            .and(query_param("since", "start"))
            .respond_with(ok(json!({ "changes": [], "next_cursor": "c0", "has_more": false, "baseline": true }))),
    );
    mount(
        Mock::given(path("/changes"))
            .respond_with(ok(json!({ "changes": [], "next_cursor": "c0", "has_more": false, "baseline": false }))),
    );
    mount(Mock::given(path("/emails")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "success": true, "data": [{ "id": "i1", "thread_id": "t1" }], "meta": { "total": 1, "limit": 100, "cursor": null }
    }))));
    mount(Mock::given(path("/sent-emails")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "success": true, "data": [], "meta": { "total": 0, "limit": 100, "cursor": null }
    }))));
    mount(Mock::given(path("/emails/i1")).respond_with(ok(json!({
        "id": "i1", "thread_id": "t1", "status": "completed", "received_at": "2026-10-05T10:00:00Z"
    }))));
    mount(Mock::given(path("/emails/i1/raw")).respond_with(ResponseTemplate::new(200).set_body_raw(
        b"From: Ada <ada@example.com>\r\nTo: scout@abc.primitive.email\r\nSubject: Welcome\r\n\r\nHello Scout\r\n".to_vec(),
        "message/rfc822",
    )));

    let (_t, core, secrets) = core("primitive");
    *core.agent_mail.base.lock().unwrap() = Some(server.uri());
    let agent =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "req-5".into()))
            .unwrap();
    assert_eq!(agent.address, "scout@abc.primitive.email");
    assert_eq!(key_of(&secrets, &agent.account_id), "prim_k");

    // A second agent on the same Primitive account is a local part: no call.
    let calls = rt.block_on(server.received_requests()).unwrap().len();
    let writer =
        block_on(core.clone().add_agent(agent.account_id.clone(), "Writer".into(), None, "req-5b".into())).unwrap();
    assert_eq!(writer.address, "writer@abc.primitive.email");
    assert_eq!(rt.block_on(server.received_requests()).unwrap().len(), calls, "adding an agent calls nothing");
    assert_eq!(secrets.0.lock().unwrap().len(), 1, "one key for both");
    block_on(core.clone().set_current_account(agent.account_id.clone())).unwrap();
    core.clone().start_sync().unwrap();
    wait_for("the welcome message", || inbox_rows(&core) == 1);
    let page = block_on(core.list_threads("INBOX".into(), None, 10)).unwrap();
    assert_eq!(page.rows[0].subject, "Welcome");
    core.stop_sync();
    let requests = rt.block_on(server.received_requests()).unwrap();
    assert!(
        requests.iter().filter(|r| r.url.path() != "/agent/accounts").all(|r| r
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            == Some("Bearer prim_k")),
        "every mail call carries the mailbox's key"
    );
}

fn tool(core: &Arc<Core>, session: &str, tool: permissions::Tool, args: serde_json::Value) -> agent_mcp::Outcome {
    crate::runtime::runtime().block_on(crate::agents::tools_call_for_tests(core, session, tool, args))
}

#[test]
fn an_agent_mailbox_that_sends_freely_sends_without_asking_and_flags_what_breaks_the_guide() {
    use crate::guide::{
        GuideCheck, GuideCheckKind, GuideEdit, GuideEntryFields, GuideKind, GuideScope, GuideSource, GuideStatus,
    };
    use agent_mcp::Outcome;
    use permissions::Tool;

    let (_t, core, _secrets) = core("freely");
    core.debug_use_fake_agent_mail(true);
    let agent =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "req-6".into()))
            .unwrap();
    assert_eq!(core.agent_send_mode(agent.account_id.clone()).unwrap(), AgentSendMode::Freely, "the default");
    block_on(core.clone().set_current_account(agent.account_id.clone())).unwrap();
    core.clone().start_sync().unwrap();
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
    let db = core.db().unwrap();
    db.write_blocking(|tx| mail_store::agents::start_session(tx, "s1", "claude-code", 1)).unwrap();
    core.agents.register("s1", permissions::Scope::Mailbox, None);
    let draft_of = |out: Outcome| match out {
        Outcome::Ok { structured: Some(v), .. } => v["draft_id"].as_i64().unwrap(),
        other => panic!("{other:?}"),
    };

    let draft = draft_of(tool(
        &core,
        "s1",
        Tool::CreateDraft,
        json!({ "to": ["ada@example.com"], "subject": "Hi", "body_markdown": "Let's circle back tomorrow." }),
    ));
    match tool(&core, "s1", Tool::Send, json!({ "draft_id": draft })) {
        Outcome::Ok { structured: Some(v), .. } => {
            assert_eq!(v["sent"], true, "sent without asking");
            assert!(v["writing_guide_breaches"].as_str().unwrap().contains("circle back"), "the agent is told");
        }
        other => panic!("{other:?}"),
    }
    let log = block_on(core.list_agent_actions(10)).unwrap();
    let send = log.iter().find(|a| a.tool == "mail_send").unwrap();
    assert_eq!(send.state, "done");
    assert!(send.result_summary.as_deref().is_some_and(|s| s.contains("Breaks your writing guide")), "{send:?}");
    let fake = core.fake_agent_mailbox(&agent.account_id).unwrap();
    wait_for("the send", || fake.message_count() == 1);

    // Asking before each send: the send waits for the user.
    core.set_agent_send_mode(agent.account_id.clone(), AgentSendMode::Ask).unwrap();
    *core.agents.approvals.timeout.lock().unwrap() = Some(Duration::from_millis(200));
    let draft = draft_of(tool(
        &core,
        "s1",
        Tool::CreateDraft,
        json!({ "to": ["ada@example.com"], "subject": "Again", "body_markdown": "Hello" }),
    ));
    match tool(&core, "s1", Tool::Send, json!({ "draft_id": draft })) {
        Outcome::Error { code, .. } => assert_eq!(code, "approval_timeout"),
        other => panic!("{other:?}"),
    }
    assert_eq!(fake.message_count(), 1, "nothing more was sent");
    core.stop_sync();
}

#[test]
fn an_own_domain_is_added_checked_and_becomes_the_agents_address() {
    let (_t, core, _secrets) = core("domain");
    core.debug_use_fake_agent_mail(true);
    let agent =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "req-7".into()))
            .unwrap();
    let id = agent.account_id.clone();
    assert!(block_on(core.agent_domains(id.clone())).unwrap().is_empty());

    // A domain whose mail goes elsewhere: the service suggests a subdomain.
    let err = block_on(core.add_agent_domain(id.clone(), "example.com".into())).unwrap_err();
    assert_eq!(err.to_string(), provider_primitive::DOMAIN_RECEIVES_ELSEWHERE);
    assert!(block_on(core.add_agent_domain(id.clone(), "not a domain".into())).is_err());

    let added = block_on(core.add_agent_domain(id.clone(), "Agents.Example.com.".into())).unwrap();
    assert_eq!(added.domain, "agents.example.com");
    assert!(!added.verified);
    assert!(added.records.iter().any(|r| r.kind == "MX" && r.status == "pending"));
    assert!(block_on(core.agent_domain_zone_file(id.clone(), added.id.clone())).unwrap().contains(" IN MX "));

    // Not verified yet: the address cannot move there.
    assert!(block_on(core.clone().set_agent_address(id.clone(), "scout@agents.example.com".into())).is_err());
    let checked = block_on(core.check_agent_domain(id.clone(), added.id.clone())).unwrap();
    assert!(checked.verified && checked.records.iter().all(|r| r.status == "found"));

    block_on(core.clone().set_agent_address(id.clone(), "Scout@agents.example.com".into())).unwrap();
    let listed = block_on(core.list_accounts()).unwrap();
    assert_eq!(listed[0].email, "scout@agents.example.com");
    assert_eq!(core.agent_meta(&id).unwrap().managed_address.as_deref(), Some(agent.address.as_str()));
    block_on(core.clone().set_current_account(id.clone())).unwrap();
    assert_eq!(block_on(core.account_address()).unwrap(), "scout@agents.example.com", "the composer's From");

    // Elsewhere is refused; back to the service's own address is fine.
    assert!(block_on(core.clone().set_agent_address(id.clone(), "scout@other.example.org".into())).is_err());
    block_on(core.clone().set_agent_address(id.clone(), agent.address.clone())).unwrap();
    assert_eq!(block_on(core.list_accounts()).unwrap()[0].email, agent.address);
}

#[test]
fn retrying_a_creation_returns_the_same_mailbox() {
    let (_t, core, _secrets) = core("retry");
    core.debug_use_fake_agent_mail(true);
    let first =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "sheet-1".into()))
            .unwrap();
    let again =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "sheet-1".into()))
            .unwrap();
    assert_eq!(
        (first.account_id.as_str(), first.address.as_str()),
        (again.account_id.as_str(), again.address.as_str())
    );
    assert_eq!(block_on(core.list_accounts()).unwrap().len(), 1);
    assert!(
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "no spaces".into()))
            .is_err()
    );
}

#[test]
fn an_address_is_the_agents_name_at_the_managed_domain() {
    assert_eq!(
        mailbox_address("jade-emu.primitive.email", "Research Scout"),
        "research-scout@jade-emu.primitive.email"
    );
    assert_eq!(mailbox_address("me@x.example", "Scout"), "me@x.example");
    assert_eq!(local_part("  ¡Hola!  "), "hola");
    assert_eq!(local_part("???"), "agent");
}

#[test]
fn a_mailbox_stored_with_only_its_domain_is_repaired_when_sync_starts() {
    let (t, core, _secrets) = core("repair");
    core.debug_use_fake_agent_mail(true);
    let agent = block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Jade".into(), None, "old".into()))
        .unwrap();
    // As an earlier version wrote it: the managed domain alone.
    let dir = t.0.join("accounts").join(&agent.account_id);
    let mut raw: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("agent.json")).unwrap()).unwrap();
    raw["address"] = json!("jade-emu.primitive.email");
    raw["managed_address"] = json!("jade-emu.primitive.email");
    std::fs::write(dir.join("agent.json"), serde_json::to_vec(&raw).unwrap()).unwrap();
    let db = block_on(core.store_for(&agent.account_id)).unwrap();
    db.write_blocking(|tx| mail_store::read::set_sync_state(tx, "account_email", "jade-emu.primitive.email")).unwrap();
    block_on(core.rename_account(agent.account_id.clone(), "Jade".into())).unwrap();
    let mut index: serde_json::Value =
        serde_json::from_slice(&std::fs::read(t.0.join("accounts/index.json")).unwrap()).unwrap();
    index[0]["email"] = json!("jade-emu.primitive.email");
    std::fs::write(t.0.join("accounts/index.json"), serde_json::to_vec(&index).unwrap()).unwrap();

    block_on(core.clone().start_all_sync()).unwrap();
    assert_eq!(block_on(core.list_accounts()).unwrap()[0].email, "jade@jade-emu.primitive.email");
    block_on(core.clone().set_current_account(agent.account_id.clone())).unwrap();
    assert_eq!(block_on(core.account_address()).unwrap(), "jade@jade-emu.primitive.email", "the composer's From");
    core.stop_sync();
}

#[test]
fn where_the_mailbox_may_send_follows_verification_and_domains() {
    let (_t, core, _secrets) = core("rules");
    core.debug_use_fake_agent_mail(true);
    let agent =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "rules-1".into()))
            .unwrap();
    let id = agent.account_id.clone();
    let kinds = |rules: Vec<AgentSendRule>| rules.into_iter().map(|r| (r.kind, r.value)).collect::<Vec<_>>();
    assert_eq!(
        kinds(block_on(core.agent_send_rules(id.clone())).unwrap()),
        vec![("managed_zone".to_owned(), Some("primitive.email".to_owned()))]
    );
    block_on(core.start_agent_mailbox_verification(id.clone(), "me@example.com".into())).unwrap();
    block_on(core.verify_agent_mailbox(id.clone(), "123456".into())).unwrap();
    let added = block_on(core.add_agent_domain(id.clone(), "agents.example.com".into())).unwrap();
    block_on(core.check_agent_domain(id.clone(), added.id)).unwrap();
    assert_eq!(
        kinds(block_on(core.agent_send_rules(id)).unwrap()),
        vec![
            ("managed_zone".to_owned(), Some("primitive.email".to_owned())),
            ("your_domain".to_owned(), Some("agents.example.com".to_owned())),
            ("address".to_owned(), Some("me@example.com".to_owned())),
        ]
    );
}

/// The agent's `agent.json` and the service account as a mailbox made
/// before 2026-10-08 left them: no `service_account`, no `service.json`.
fn as_before_service_accounts(t: &Temp, account_id: &str) {
    let path = t.0.join("accounts").join(account_id).join("agent.json");
    let mut raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    raw.as_object_mut().unwrap().remove("service_account");
    std::fs::write(&path, serde_json::to_vec_pretty(&raw).unwrap()).unwrap();
    std::fs::remove_dir_all(t.0.join("services")).unwrap();
}

fn service_file(t: &Temp, id: &str) -> std::path::PathBuf {
    t.0.join("services").join(id).join("service.json")
}

#[test]
fn a_mailbox_made_before_service_accounts_is_one_under_its_own_id_and_keeps_its_key() {
    let (t, core, secrets) = core("migrate");
    core.debug_use_fake_agent_mail(true);
    let old =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "old-1".into()))
            .unwrap();
    let id = old.account_id.clone();
    as_before_service_accounts(&t, &id);
    let agent_json = std::fs::read(t.0.join("accounts").join(&id).join("agent.json")).unwrap();
    let keys_before = secrets.0.lock().unwrap().clone();

    // Read, not rewritten: its service account is itself, under the same key.
    for _ in 0..2 {
        assert_eq!(core.agent_service_account(id.clone()).unwrap(), id);
        let listed = block_on(core.list_service_accounts()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].id.as_str(), listed[0].agent_account_ids.clone()), (id.as_str(), vec![id.clone()]));
        assert_eq!(listed[0].managed_domain.as_deref(), Some("demo.primitive.email"));
        assert!(core.account_has_credentials(id.clone()).unwrap());
        assert_eq!(core.agent_mailbox_api_key(id.clone()).unwrap(), key_of(&secrets, &id));
    }
    assert_eq!(std::fs::read(t.0.join("accounts").join(&id).join("agent.json")).unwrap(), agent_json);
    assert!(!service_file(&t, &id).exists(), "nothing written by reading");
    assert_eq!(*secrets.0.lock().unwrap(), keys_before, "nothing re-keyed");

    // The first change writes the record; the key keeps its name.
    block_on(core.agent_mailbox_plan(id.clone())).unwrap();
    assert!(service_file(&t, &id).is_file());
    assert_eq!(*secrets.0.lock().unwrap(), keys_before);

    // A second agent joins it; removing the first keeps the key for the second.
    let writer = block_on(core.clone().add_agent(id.clone(), "Writer".into(), None, "new-1".into())).unwrap();
    assert_eq!(writer.service_account_id, id);
    block_on(core.remove_account(id.clone())).unwrap();
    assert_eq!(*secrets.0.lock().unwrap(), keys_before, "the second agent still uses it");
    assert_eq!(core.agent_service_account(writer.account_id.clone()).unwrap(), id);
    assert!(block_on(core.agent_mailbox_plan(writer.account_id.clone())).is_ok());
    let listed = block_on(core.list_service_accounts()).unwrap();
    assert_eq!(listed[0].agent_account_ids, vec![writer.account_id.clone()]);

    // The last agent takes the key and the record with it.
    block_on(core.remove_account(writer.account_id)).unwrap();
    assert!(secrets.0.lock().unwrap().is_empty());
    assert!(!service_file(&t, &id).exists());
    assert!(block_on(core.list_service_accounts()).unwrap().is_empty());
}

#[test]
fn a_migrated_mailbox_removed_alone_forgets_its_key() {
    let (t, core, secrets) = core("migrate-remove");
    core.debug_use_fake_agent_mail(true);
    let old =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "old-2".into()))
            .unwrap();
    as_before_service_accounts(&t, &old.account_id);
    block_on(core.remove_account(old.account_id.clone())).unwrap();
    assert!(secrets.0.lock().unwrap().is_empty());
    assert!(!t.0.join("services").join(&old.account_id).exists());
}

#[test]
fn two_agents_share_one_key_one_verification_and_one_set_of_domains() {
    let (t, core, secrets) = core("share");
    core.debug_use_fake_agent_mail(true);
    let scout =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "sa-1".into()))
            .unwrap();
    let service = scout.account_id.clone();
    assert!(service_file(&t, &service).is_file());
    let writer = block_on(core.clone().add_agent(service.clone(), "Writer".into(), None, "ag-2".into())).unwrap();
    assert_eq!(writer.address, "writer@demo.primitive.email");
    assert_ne!(writer.account_id, scout.account_id);

    let keys = secrets.0.lock().unwrap().clone();
    assert_eq!(keys.len(), 1, "one Keychain item: {keys:?}");
    assert!(keys.contains_key(&keys::mailbox_api_key(&service)));
    assert_eq!(
        core.agent_mailbox_api_key(writer.account_id.clone()).unwrap(),
        core.agent_mailbox_api_key(scout.account_id.clone()).unwrap()
    );
    assert!(core.account_has_credentials(writer.account_id.clone()).unwrap());

    let listed = block_on(core.list_service_accounts()).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].agent_account_ids, vec![scout.account_id.clone(), writer.account_id.clone()]);
    assert!(!listed[0].verified);
    let accounts = block_on(core.list_accounts()).unwrap();
    assert_eq!(accounts.len(), 2, "each agent is an account");
    assert_eq!(accounts[1].display_name.as_deref(), Some("Writer"));

    // Verifying through one agent verifies the service account, so both.
    block_on(core.start_agent_mailbox_verification(writer.account_id.clone(), "me@example.com".into())).unwrap();
    block_on(core.verify_agent_mailbox(writer.account_id.clone(), "123456".into())).unwrap();
    assert!(block_on(core.agent_mailbox_plan(scout.account_id.clone())).unwrap().verified);
    let listed = block_on(core.list_service_accounts()).unwrap();
    assert!(listed[0].verified);
    assert_eq!(listed[0].human_email.as_deref(), Some("me@example.com"));
    assert!(listed[0].plan.as_ref().is_some_and(|p| p.verified));

    // A domain added through one agent is the other's too.
    let added = block_on(core.add_agent_domain(scout.account_id.clone(), "agents.example.com".into())).unwrap();
    block_on(core.check_service_account_domain(service.clone(), added.id)).unwrap();
    assert_eq!(block_on(core.agent_domains(writer.account_id.clone())).unwrap().len(), 1);
    block_on(core.clone().set_agent_address(writer.account_id.clone(), "writer@agents.example.com".into())).unwrap();
    assert!(
        block_on(core.clone().set_agent_address(scout.account_id.clone(), "writer@agents.example.com".into())).is_err(),
        "another agent's address"
    );

    // Rotating the key replaces the one item both agents use.
    block_on(core.rotate_service_account_key(service.clone())).unwrap();
    assert_eq!(secrets.0.lock().unwrap().len(), 1);
    assert_ne!(key_of(&secrets, &service), keys[&keys::mailbox_api_key(&service)]);
    assert!(block_on(core.agent_mailbox_plan(writer.account_id.clone())).unwrap().verified, "the new key works");

    // Removing one agent keeps the key; removing the last forgets it.
    block_on(core.remove_account(scout.account_id.clone())).unwrap();
    assert!(secrets.0.lock().unwrap().contains_key(&keys::mailbox_api_key(&service)));
    assert!(service_file(&t, &service).is_file());
    assert_eq!(
        block_on(core.agent_mailbox_plan(writer.account_id.clone())).unwrap().email.as_deref(),
        Some("me@example.com")
    );
    block_on(core.remove_account(writer.account_id.clone())).unwrap();
    assert!(secrets.0.lock().unwrap().is_empty());
    assert!(!service_file(&t, &service).exists());
}

#[test]
fn an_agents_address_is_unique_within_its_service_account() {
    let (_t, core, _secrets) = core("unique");
    core.debug_use_fake_agent_mail(true);
    let scout =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "u-1".into()))
            .unwrap();
    let service = scout.account_id.clone();
    let err = block_on(core.clone().add_agent(service.clone(), "  SCOUT ".into(), None, "u-2".into())).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(err.to_string().contains("scout@demo.primitive.email"), "{err}");

    // A retry of the same request returns the same agent.
    let writer = block_on(core.clone().add_agent(service.clone(), "Writer".into(), None, "u-3".into())).unwrap();
    let again = block_on(core.clone().add_agent(service.clone(), "Writer".into(), None, "u-3".into())).unwrap();
    assert_eq!(writer, again);
    assert_eq!(block_on(core.list_accounts()).unwrap().len(), 2);

    // An own domain must be one of the service account's verified ones.
    assert!(
        block_on(core.clone().add_agent(
            service.clone(),
            "Clerk".into(),
            Some("agents.example.com".into()),
            "u-4".into()
        ))
        .is_err()
    );
    let added = block_on(core.add_service_account_domain(service.clone(), "agents.example.com".into())).unwrap();
    block_on(core.check_service_account_domain(service.clone(), added.id)).unwrap();
    let clerk = block_on(core.clone().add_agent(
        service.clone(),
        "Clerk".into(),
        Some("Agents.Example.com".into()),
        "u-4".into(),
    ))
    .unwrap();
    assert_eq!(clerk.address, "clerk@agents.example.com");
    assert_eq!(
        core.agent_meta(&clerk.account_id).unwrap().managed_address.as_deref(),
        Some("clerk@demo.primitive.email")
    );
    // Its managed address is taken too.
    assert!(block_on(core.clone().add_agent(service.clone(), "Clerk".into(), None, "u-5".into())).is_err());
    assert!(block_on(core.clone().add_agent("nope".into(), "Other".into(), None, "u-6".into())).is_err());
}

#[test]
fn a_second_agent_syncs_its_own_fake_mailbox() {
    let (_t, core, _secrets) = core("two-sync");
    core.debug_use_fake_agent_mail(true);
    let scout =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "s-1".into()))
            .unwrap();
    let writer =
        block_on(core.clone().add_agent(scout.account_id.clone(), "Writer".into(), None, "s-2".into())).unwrap();
    block_on(core.clone().set_current_account(writer.account_id.clone())).unwrap();
    assert!(block_on(core.clone().start_all_sync()).unwrap().is_empty(), "nothing needs a sign-in");
    core.debug_deliver_to_agent_mailbox(
        writer.account_id.clone(),
        "ada@example.com".into(),
        "Hi".into(),
        "Hello".into(),
    )
    .unwrap();
    wait_for("the delivered mail", || inbox_rows(&core) == 1);
    core.stop_sync();
}

#[test]
fn each_agent_knows_its_addresses_the_others_and_whether_it_is_the_first() {
    let (t, core, _secrets) = core("routing");
    core.debug_use_fake_agent_mail(true);
    let scout =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "rt-1".into()))
            .unwrap();
    let service = scout.account_id.clone();
    let writer = block_on(core.clone().add_agent(service.clone(), "Writer".into(), None, "rt-2".into())).unwrap();
    let added = block_on(core.add_service_account_domain(service.clone(), "agents.example.com".into())).unwrap();
    block_on(core.check_service_account_domain(service.clone(), added.id)).unwrap();
    // Use This Address: the writer moves to the service account's domain.
    block_on(core.clone().set_agent_address(writer.account_id.clone(), "writer@agents.example.com".into())).unwrap();
    // Another service account's agent is no concern of theirs.
    block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Other".into(), None, "rt-3".into())).unwrap();

    let routing = |id: &str| primitive_routing(&t.0, id, &core.agent_meta(id).unwrap());
    let s = routing(&scout.account_id);
    assert_eq!(s.own, vec!["scout@demo.primitive.email".to_owned()]);
    assert_eq!(s.others, vec!["writer@agents.example.com".to_owned(), "writer@demo.primitive.email".to_owned()]);
    assert!(s.catch_all, "the first agent");
    let w = routing(&writer.account_id);
    assert_eq!(w.own, vec!["writer@agents.example.com".to_owned(), "writer@demo.primitive.email".to_owned()]);
    assert_eq!(w.others, vec!["scout@demo.primitive.email".to_owned()]);
    assert!(!w.catch_all);
    assert_eq!(core.fellow_agents(&scout.account_id, &core.agent_meta(&scout.account_id).unwrap()), vec!["Writer"]);

    // The first goes: the writer is alone and takes what no agent has.
    let writer_meta = core.agent_meta(&writer.account_id).unwrap();
    block_on(core.remove_account(scout.account_id.clone())).unwrap();
    let w = primitive_routing(&t.0, &writer.account_id, &writer_meta);
    assert!(w.catch_all && w.others.is_empty());
}

fn stored(core: &Arc<Core>, account: &str, id: &str) -> Option<Vec<LabelId>> {
    let db = block_on(core.store_for(account)).unwrap();
    let id = MessageId::new(id);
    db.read_blocking(move |c| mail_store::read::get_message(c, &id)).unwrap().map(|m| m.label_ids)
}

#[test]
fn two_primitive_agents_sync_only_their_own_mail_and_one_keeps_syncing_when_the_other_goes() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(MockServer::start());
    let mount = |mock: Mock| rt.block_on(mock.mount(&server));
    mount(Mock::given(method("POST")).and(path("/agent/accounts")).respond_with(ok(json!({
        "api_key": "prim_k", "address": "abc.primitive.email", "plan": "agent", "limits": limits()
    }))));
    mount(
        Mock::given(path("/changes"))
            .and(query_param("since", "start"))
            .respond_with(ok(json!({ "changes": [], "next_cursor": "c0", "has_more": false, "baseline": true }))),
    );
    // Nothing new until later; a long-poll answers after a moment.
    mount(
        Mock::given(path("/changes"))
            .respond_with(
                ok(json!({ "changes": [], "next_cursor": "c0", "has_more": false, "baseline": false }))
                    .set_delay(Duration::from_millis(50)),
            )
            .with_priority(10),
    );
    mount(Mock::given(path("/emails")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "success": true, "meta": { "total": 3, "limit": 100, "cursor": null }, "data": [
            { "id": "i1", "thread_id": null, "to_email": "scout@abc.primitive.email" },
            { "id": "i2", "thread_id": null, "to_email": "writer@abc.primitive.email" },
            { "id": "i3", "thread_id": null, "to_email": "sales@abc.primitive.email" }
        ]
    }))));
    mount(Mock::given(path("/sent-emails")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "success": true, "meta": { "total": 1, "limit": 100, "cursor": null },
        "data": [{ "id": "o1", "thread_id": null }]
    }))));
    for (id, to) in [
        ("i1", "scout@abc.primitive.email"),
        ("i2", "writer@abc.primitive.email"),
        ("i3", "sales@abc.primitive.email"),
        ("i4", "scout@abc.primitive.email"),
        ("i5", "writer+later@abc.primitive.email"),
    ] {
        mount(Mock::given(path(format!("/emails/{id}"))).respond_with(ok(json!({
            "id": id, "thread_id": null, "status": "completed", "received_at": "2026-10-08T10:00:00Z", "to_email": to
        }))));
        mount(Mock::given(path(format!("/emails/{id}/raw"))).respond_with(ResponseTemplate::new(200).set_body_raw(
            format!("From: Ada <ada@example.com>\r\nTo: {to}\r\nSubject: {id}\r\n\r\nHello\r\n").into_bytes(),
            "message/rfc822",
        )));
    }
    mount(Mock::given(path("/sent-emails/o1")).respond_with(ok(json!({
        "id": "o1", "thread_id": null, "created_at": "2026-10-08T11:00:00Z",
        "from_header": "\"Writer\" <writer@abc.primitive.email>", "to_header": "ada@example.com", "subject": "o1"
    }))));

    let (_t, core, _secrets) = core("primitive-two");
    *core.agent_mail.base.lock().unwrap() = Some(server.uri());
    let scout =
        block_on(core.clone().create_agent_mailbox(AgentService::Primitive, "Scout".into(), None, "tw-1".into()))
            .unwrap();
    let writer =
        block_on(core.clone().add_agent(scout.account_id.clone(), "Writer".into(), None, "tw-2".into())).unwrap();
    let (s, w) = (scout.account_id.clone(), writer.account_id.clone());
    block_on(core.clone().set_current_account(s.clone())).unwrap();
    assert!(block_on(core.clone().start_all_sync()).unwrap().is_empty());

    wait_for("each agent's mail", || {
        stored(&core, &s, "in:i3").is_some()
            && stored(&core, &w, "in:i2").is_some()
            && stored(&core, &w, "out:o1").is_some()
    });
    assert!(stored(&core, &s, "in:i1").is_some());
    assert!(
        stored(&core, &s, "in:i3").unwrap().contains(&LabelId::new(provider_primitive::OTHER_ADDRESSES_LABEL)),
        "mail to an address no agent has goes to the first agent, marked"
    );
    for (account, id) in [(&s, "in:i2"), (&s, "out:o1"), (&w, "in:i1"), (&w, "in:i3")] {
        assert_eq!(stored(&core, account, id), None, "{id} is not in {account}'s store");
    }

    // The first agent goes; the writer keeps syncing, and is now alone.
    block_on(core.remove_account(s.clone())).unwrap();
    mount(
        Mock::given(path("/changes"))
            .and(query_param("since", "c0"))
            .respond_with(ok(json!({
                "changes": [
                    { "kind": "email.visible", "email_id": "i4", "thread_id": null },
                    { "kind": "email.visible", "email_id": "i5", "thread_id": null }
                ],
                "next_cursor": "c0", "has_more": false, "baseline": false
            })))
            .with_priority(1),
    );
    wait_for("the new mail", || stored(&core, &w, "in:i4").is_some() && stored(&core, &w, "in:i5").is_some());
    assert_eq!(
        stored(&core, &w, "in:i4").unwrap(),
        vec![LabelId::new("INBOX"), LabelId::new("UNREAD")],
        "the removed agent's address has no agent now: the writer takes it, unmarked while alone"
    );
    core.stop_sync();
}

#[test]
fn agentmails_code_sent_at_sign_up_is_found_in_the_users_mail() {
    let (_t, core, _secrets) = core("agentmail-code");
    core.debug_use_fake_agent_mail(true);
    let agent = block_on(core.clone().create_agent_mailbox(
        AgentService::AgentMail,
        "Scout".into(),
        Some("me@example.com".into()),
        "am-code".into(),
    ))
    .unwrap();
    // No Send Code: AgentMail sent it with the sign-up.
    block_on(core.clone().open_account("me".into())).unwrap();
    let fake = Arc::new(provider_api::fake::FakeProvider::new("me@example.com", mail_sync::now_millis(), 50));
    fake.seed(FetchedMessage {
        id: MessageId::new("code"),
        thread_id: ThreadId::new("code"),
        label_ids: vec![LabelId::new("INBOX"), LabelId::new("UNREAD")],
        internal_date: mail_sync::now_millis(),
        from: Some(EmailAddress::new(None, "no-reply@agentmail.to")),
        subject: "Verify your email".into(),
        snippet: "Your code is 654321".into(),
        body: Some(FetchedBody { text: Some("Your code is 654321.".into()), html: None, attachments: vec![] }),
        ..Default::default()
    });
    core.start_sync_with(fake).unwrap();
    wait_for("the code to arrive", || inbox_rows(&core) == 1);
    let code = block_on(core.find_agent_mailbox_code(agent.account_id, "me".into())).unwrap();
    assert_eq!(code.as_deref(), Some("654321"));
    core.stop_sync();
}

#[test]
fn an_agentmail_service_account_is_created_with_the_human_email_and_never_signed_up_for_twice() {
    let (_t, core, secrets) = core("agentmail");
    core.debug_use_fake_agent_mail(true);
    // AgentMail needs the user's email.
    let missing =
        block_on(core.clone().create_agent_mailbox(AgentService::AgentMail, "Scout".into(), None, "am-0".into()));
    assert_eq!(missing.unwrap_err().kind(), ErrorKind::InvalidInput);

    let scout = block_on(core.clone().create_agent_mailbox(
        AgentService::AgentMail,
        "Scout".into(),
        Some(" me@example.com ".into()),
        "am-1".into(),
    ))
    .unwrap();
    assert_eq!(scout.address, "scout@agentmail.to");
    assert!(!scout.plan.verified && !scout.plan.reply_only);
    let service = scout.account_id.clone();
    let meta = core.agent_meta(&service).unwrap();
    assert_eq!(meta.inbox_id.as_deref(), Some("scout@agentmail.to"));
    assert_eq!(core.service_meta(&service).unwrap().human_email.as_deref(), Some("me@example.com"));

    // The same email again would rotate the organisation's key: refused
    // here, before the service is asked.
    let again = block_on(core.clone().create_agent_mailbox(
        AgentService::AgentMail,
        "Writer".into(),
        Some("ME@example.com".into()),
        "am-2".into(),
    ));
    let err = again.unwrap_err();
    assert!(err.to_string().contains("Add the agent to it instead"), "{err}");
    // A retry of the first creation returns it, without a sign-up either.
    let retry = block_on(core.clone().create_agent_mailbox(
        AgentService::AgentMail,
        "Scout".into(),
        Some("me@example.com".into()),
        "am-1".into(),
    ))
    .unwrap();
    assert_eq!(retry.account_id, scout.account_id);
    let fake = core.agent_mail.fake_services.lock().unwrap().get(&AgentService::AgentMail).cloned().unwrap();
    assert_eq!(fake.repeated_sign_ups.load(Ordering::SeqCst), 0, "never signed up twice");

    // More agents are more inboxes in the organisation, on the same key.
    let writer = block_on(core.clone().add_agent(service.clone(), "Writer".into(), None, "am-3".into())).unwrap();
    assert_eq!(writer.address, "writer@agentmail.to");
    assert_eq!(core.agent_meta(&writer.account_id).unwrap().inbox_id.as_deref(), Some("writer@agentmail.to"));
    assert_eq!(secrets.0.lock().unwrap().len(), 1, "one key for the organisation");
    block_on(core.clone().add_agent(service.clone(), "Clerk".into(), None, "am-4".into())).unwrap();
    let fourth = block_on(core.clone().add_agent(service.clone(), "Fourth".into(), None, "am-5".into())).unwrap_err();
    assert!(fourth.to_string().contains("as many inboxes as its plan allows"), "{fourth}");
    let listed = block_on(core.list_service_accounts()).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].service, AgentService::AgentMail);
    assert_eq!(listed[0].agent_account_ids.len(), 3);
}

#[test]
fn an_unverified_agentmail_account_writes_only_to_the_human_and_verifying_lifts_it() {
    let (_t, core, _secrets) = core("agentmail-verify");
    core.debug_use_fake_agent_mail(true);
    let scout = block_on(core.clone().create_agent_mailbox(
        AgentService::AgentMail,
        "Scout".into(),
        Some("me@example.com".into()),
        "amv-1".into(),
    ))
    .unwrap();
    let id = scout.account_id.clone();
    block_on(core.clone().set_current_account(id.clone())).unwrap();

    let limits = core.service_account_limits(id.clone()).unwrap();
    assert!(limits.contains("only to me@example.com"), "{limits}");
    assert!(limits.contains("3 different people in its first hour"), "{limits}");
    let prompt =
        agent_prompt(&core.agent_meta(&id).unwrap(), &[], &core.agent_limits_text(&core.agent_meta(&id).unwrap()));
    assert!(prompt.contains("only to me@example.com"), "{prompt}");
    assert!(core.check_agent_recipients(&["Me@Example.com".into()]).is_ok());
    let refused = core.check_agent_recipients(&["me@example.com".into(), "ada@example.com".into()]).unwrap_err();
    assert!(refused.to_string().contains("only to me@example.com"), "{refused}");
    // No inbox key before verifying.
    assert!(block_on(core.agent_inbox_api_key(id.clone())).is_err());

    // The code goes to the human it was made with, and no one else.
    let other = block_on(core.start_service_account_verification(id.clone(), "you@example.com".into()));
    assert!(other.unwrap_err().to_string().contains("sends the code to me@example.com"));
    block_on(core.start_service_account_verification(id.clone(), "me@example.com".into())).unwrap();
    let plan = block_on(core.verify_service_account(id.clone(), "123456".into())).unwrap();
    assert!(plan.verified);
    assert_eq!(plan.email.as_deref(), Some("me@example.com"));

    assert!(core.check_agent_recipients(&["ada@example.com".into(), "bo@example.com".into()]).is_ok());
    assert!(!core.service_account_limits(id.clone()).unwrap().contains("only to"));
    assert_eq!(block_on(core.agent_inbox_api_key(id.clone())).unwrap(), "fake_inbox_scout@agentmail.to");
    assert!(core.agent_service_terms_url(AgentService::AgentMail).starts_with("https://www.agentmail.to/"));
}

/// AgentMail's API as a wiremock fake: sign-up ("scout"), a second inbox
/// ("writer") with one welcome message, verification, the organisation,
/// and an empty scout inbox.
fn agentmail_api(rt: &tokio::runtime::Runtime, server: &MockServer) {
    let mount = |mock: Mock| rt.block_on(mock.mount(server));
    let plain = |body: serde_json::Value| ResponseTemplate::new(200).set_body_json(body);
    mount(
        Mock::given(method("POST"))
            .and(path("/v0/agent/sign-up"))
            .and(body_partial_json(json!({ "username": "scout", "human_email": "me@example.com" })))
            .respond_with(plain(
                json!({ "organization_id": "org", "inbox_id": "scout@agentmail.to", "api_key": "am_k" }),
            ))
            .expect(1),
    );
    mount(
        Mock::given(method("POST"))
            .and(path("/v0/inboxes"))
            .and(body_partial_json(json!({ "username": "writer", "display_name": "Writer", "client_id": "amw-2" })))
            .respond_with(plain(json!({
                "pod_id": "p", "inbox_id": "writer@agentmail.to", "email": "writer@agentmail.to",
                "created_at": "2026-10-08T00:00:00Z", "updated_at": "2026-10-08T00:00:00Z"
            })))
            .expect(1),
    );
    mount(Mock::given(method("POST")).and(path("/v0/agent/verify")).respond_with(plain(json!({ "verified": true }))));
    mount(Mock::given(method("GET")).and(path("/v0/organizations")).respond_with(plain(json!({
        "organization_id": "org", "inbox_count": 2, "domain_count": 0, "inbox_limit": 3,
        "created_at": "2026-10-08T00:00:00Z", "updated_at": "2026-10-08T00:00:00Z"
    }))));
    let inbox = "/v0/inboxes/writer@agentmail.to";
    mount(Mock::given(method("GET")).and(path(format!("{inbox}/messages"))).respond_with(plain(json!({
        "count": 1, "messages": [{
            "inbox_id": "writer@agentmail.to", "thread_id": "t1", "message_id": "w1", "labels": ["received", "unread"],
            "timestamp": "2026-10-08T08:00:00Z", "from": "Ada <ada@example.com>", "to": ["writer@agentmail.to"],
            "size": 90, "created_at": "2026-10-08T08:00:00Z", "updated_at": "2026-10-08T08:00:00Z"
        }]
    }))));
    mount(
        Mock::given(method("GET"))
            .and(path(format!("{inbox}/events")))
            .respond_with(plain(json!({ "count": 0, "events": [] }))),
    );
    mount(Mock::given(method("GET")).and(path(format!("{inbox}/messages/w1"))).respond_with(plain(json!({
        "inbox_id": "writer@agentmail.to", "thread_id": "t1", "message_id": "w1", "labels": ["received", "unread"],
        "timestamp": "2026-10-08T08:00:00Z", "from": "Ada <ada@example.com>", "to": ["writer@agentmail.to"],
        "size": 90, "created_at": "2026-10-08T08:00:00Z", "updated_at": "2026-10-08T08:00:00Z"
    }))));
    mount(Mock::given(method("GET")).and(path(format!("{inbox}/messages/w1/raw"))).respond_with(plain(json!({
        "message_id": "w1", "size": 90, "download_url": format!("{}/cdn/w1", server.uri()), "expires_at": "2026-10-09T00:00:00Z"
    }))));
    mount(Mock::given(method("GET")).and(path("/cdn/w1")).respond_with(ResponseTemplate::new(200).set_body_bytes(
        b"From: Ada <ada@example.com>\r\nTo: writer@agentmail.to\r\nSubject: Welcome\r\n\r\nHello Writer\r\n".to_vec(),
    )));

    for path_ in ["/v0/inboxes/scout@agentmail.to/messages", "/v0/inboxes/scout@agentmail.to/events"] {
        let body = if path_.ends_with("events") {
            json!({ "count": 0, "events": [] })
        } else {
            json!({ "count": 0, "messages": [] })
        };
        mount(Mock::given(method("GET")).and(path(path_)).respond_with(plain(body)));
    }
}

#[test]
fn an_agentmail_organisation_is_created_its_second_agent_added_and_synced_over_the_api() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(MockServer::start());
    agentmail_api(&rt, &server);
    let inbox = "/v0/inboxes/writer@agentmail.to";
    let (_t, core, secrets) = core("agentmail-api");
    *core.agent_mail.base.lock().unwrap() = Some(server.uri());
    let scout = block_on(core.clone().create_agent_mailbox(
        AgentService::AgentMail,
        "Scout".into(),
        Some("me@example.com".into()),
        "amw-1".into(),
    ))
    .unwrap();
    assert_eq!(scout.address, "scout@agentmail.to");
    assert_eq!(key_of(&secrets, &scout.account_id), "am_k");
    // A second sign-up with that email is refused before any call.
    assert!(
        block_on(core.clone().create_agent_mailbox(
            AgentService::AgentMail,
            "Writer".into(),
            Some("me@example.com".into()),
            "amw-x".into()
        ))
        .is_err()
    );
    let writer =
        block_on(core.clone().add_agent(scout.account_id.clone(), "Writer".into(), None, "amw-2".into())).unwrap();
    assert_eq!(writer.address, "writer@agentmail.to");

    // Verified once, the organisation stays verified though AgentMail's
    // organisation does not say so.
    let service = scout.account_id.clone();
    assert!(block_on(core.verify_service_account(service.clone(), "123456".into())).unwrap().verified);
    let plan = block_on(core.service_account_plan(service.clone())).unwrap();
    assert!(plan.verified);
    assert_eq!((plan.name.as_str(), plan.email.as_deref()), ("free", Some("me@example.com")));

    block_on(core.clone().set_current_account(writer.account_id.clone())).unwrap();
    core.clone().start_sync().unwrap();
    wait_for("the writer's welcome", || inbox_rows(&core) == 1);
    assert_eq!(stored(&core, &writer.account_id, "w1").unwrap(), vec![LabelId::new("INBOX"), LabelId::new("UNREAD")]);
    core.stop_sync();
    let requests = rt.block_on(server.received_requests()).unwrap();
    assert!(
        requests.iter().filter(|r| r.url.path().starts_with(inbox)).all(|r| r
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            == Some("Bearer am_k")),
        "the writer's inbox is read with the organisation's key"
    );
    assert!(requests.iter().filter(|r| r.url.path() == "/cdn/w1").all(|r| !r.headers.contains_key("authorization")));
    rt.block_on(server.verify());
}

/// oagc-uys.15: an organisation's agents share one WebSocket (a local
/// fake), subscribed to both inboxes; an event for one makes that agent
/// poll at once, long before its next 30 s poll.
#[test]
fn agentmail_agents_share_one_websocket_and_an_event_makes_its_agent_poll() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(MockServer::start());
    agentmail_api(&rt, &server);
    let ws = rt.block_on(provider_agentmail::ws_fake::FakeAgentMailSocket::start());

    let (_t, core, _secrets) = core("agentmail-push");
    *core.agent_mail.base.lock().unwrap() = Some(server.uri());
    core.set_agentmail_ws_base(Some(ws.url()));
    let scout = block_on(core.clone().create_agent_mailbox(
        AgentService::AgentMail,
        "Scout".into(),
        Some("me@example.com".into()),
        "amw-1".into(),
    ))
    .unwrap();
    let writer =
        block_on(core.clone().add_agent(scout.account_id.clone(), "Writer".into(), None, "amw-2".into())).unwrap();
    block_on(core.clone().set_current_account(writer.account_id.clone())).unwrap();
    assert!(block_on(core.clone().start_all_sync()).unwrap().is_empty());
    wait_for("the writer's welcome", || inbox_rows(&core) == 1);
    wait_for("both inboxes subscribed", || ws.subscribed_inboxes().len() == 2);
    assert_eq!(ws.subscribed_inboxes(), ["scout@agentmail.to", "writer@agentmail.to"]);
    assert_eq!(ws.connections(), 1, "one socket for the organisation");
    assert_eq!(ws.keys(), ["am_k"]);

    // Let the polls that follow subscribing finish, then push.
    std::thread::sleep(Duration::from_millis(800));
    let lists = |inbox: &str| {
        let path_ = format!("/v0/inboxes/{inbox}/messages");
        rt.block_on(server.received_requests()).unwrap().iter().filter(|r| r.url.path() == path_).count()
    };
    let (writer_before, scout_before) = (lists("writer@agentmail.to"), lists("scout@agentmail.to"));
    ws.message_received("writer@agentmail.to", "w2");
    wait_for("the writer to poll", || lists("writer@agentmail.to") > writer_before);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(lists("scout@agentmail.to"), scout_before, "the scout was not woken");
    core.stop_sync();
}
