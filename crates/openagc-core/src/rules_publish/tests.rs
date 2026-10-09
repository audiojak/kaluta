//! Publishing to a rules server: what a snapshot holds (and never holds),
//! against an in-process `openagc-rules` on loopback for the whole flow,
//! and against a wiremock fake for the answers it should not give.

use std::time::Duration;

use futures::executor::block_on;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::agent_mailbox::AgentService;
use crate::facts::{FactEdit, FactFields, FactSource, FactUse};
use crate::guide::{
    AudienceGroup, AudienceStatus, GuideCheck, GuideCheckKind, GuideEdit, GuideEntryFields, GuideScope,
};
use crate::secrets::MemorySecrets;
use crate::{CoreConfig, EventListener};

struct Silent;
impl EventListener for Silent {
    fn on_event(&self, _account: Option<String>, _event: CoreEvent) {}
}

struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Temp {
    let dir = std::env::temp_dir().join(format!("openagc-rules-pub-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Temp(dir)
}

fn open_core(dir: &Temp, secrets: &Arc<MemorySecrets>) -> Arc<Core> {
    let core = Core::new(
        CoreConfig { data_dir: dir.0.to_string_lossy().into_owned(), log_dir: None },
        secrets.clone(),
        Arc::new(Silent),
    )
    .unwrap();
    core.debug_use_fake_agent_mail(true);
    core.rules.set_waits(Duration::from_millis(150), Duration::from_secs(2), Duration::from_millis(200));
    core
}

/// A core with an agent mailbox, Scout, holding a guide and facts.
fn setup(name: &str) -> (Temp, Arc<Core>, Arc<MemorySecrets>, String) {
    let dir = scratch(name);
    let secrets = Arc::new(MemorySecrets::default());
    let core = open_core(&dir, &secrets);
    let created = block_on(core.clone().create_agent_mailbox(
        AgentService::Primitive,
        "Scout".into(),
        None,
        format!("req-{name}"),
    ))
    .unwrap();
    let id = created.account_id;
    in_account(&core, &id, guide_and_facts(&core));
    (dir, core, secrets, id)
}

fn in_account<T>(_core: &Core, id: &str, fut: impl std::future::Future<Output = T>) -> T {
    block_on(scoped(Some(id.to_owned()), fut))
}

fn rule(statement: &str, scope: GuideScope, check: Option<GuideCheck>) -> GuideEdit {
    GuideEdit::Add {
        fields: GuideEntryFields {
            category: "B6".into(),
            kind: GuideKind::Rule,
            statement: statement.into(),
            scope,
            check,
        },
        status: GuideStatus::Accepted,
        source: crate::guide::GuideSource::You,
        // Where a learned entry came from: a message id, never published.
        origin: Some("<origin-17@mail.example>".into()),
    }
}

fn fact(category: &str, label: &str, value: &str, use_: FactUse) -> FactEdit {
    FactEdit::Add {
        fields: FactFields { category: category.into(), label: label.into(), value: value.into(), use_, as_of: None },
        status: FactStatus::Accepted,
        source: FactSource::You,
    }
}

async fn guide_and_facts(core: &Core) {
    core.apply_guide_edits(
        vec![
            rule(
                "Never say circle back",
                GuideScope::default(),
                Some(GuideCheck { kind: GuideCheckKind::BannedPhrase, value: "circle back".into() }),
            ),
            rule(
                "Be formal with customers",
                GuideScope { groups: vec!["Customers".into()], ..Default::default() },
                None,
            ),
            rule("Call her Annie", GuideScope { people: vec!["ann@acme.com".into()], ..Default::default() }, None),
            GuideEdit::Add {
                fields: GuideEntryFields {
                    category: "A1".into(),
                    kind: GuideKind::Guideline,
                    statement: "A proposal stays here".into(),
                    scope: GuideScope::default(),
                    check: None,
                },
                status: GuideStatus::Proposed,
                source: crate::guide::GuideSource::Learned,
                origin: None,
            },
        ],
        "test".into(),
    )
    .await
    .unwrap();
    core.save_audience_group(AudienceGroup {
        id: 0,
        name: "Customers".into(),
        status: AudienceStatus::Confirmed,
        description: String::new(),
        members: vec!["@acme.com".into(), "bea@globex.com".into()],
    })
    .await
    .unwrap();
    core.save_audience_group(AudienceGroup {
        id: 0,
        name: "Maybe investors".into(),
        status: AudienceStatus::Suggested,
        description: String::new(),
        members: vec!["vc@fund.example".into()],
    })
    .await
    .unwrap();
    core.apply_fact_edits(
        vec![
            fact("work", "Occupation or role", "Research agent", FactUse::Free),
            fact("availability", "Calendar link", "https://cal.example/scout", FactUse::Free),
            fact("people", "Sam Rivera", "My assistant", FactUse::Ask),
            fact("contact", "Mailing address", "1 Secret Lane", FactUse::Never),
        ],
        "test".into(),
    )
    .await
    .unwrap();
    // A global fact: every account's, the user's own (ADR 0012).
    core.apply_global_fact_edits(vec![fact("identity", "Preferred name", "Jo Global", FactUse::Free)], "test".into())
        .await
        .unwrap();
}

fn fact_id(core: &Core, id: &str, label: &str) -> (i64, crate::facts::FactScope) {
    let facts = in_account(core, id, core.list_facts(vec![FactStatus::Accepted])).unwrap();
    let f = facts.iter().find(|f| f.label == label).unwrap();
    (f.id, f.scope)
}

fn share(core: &Core, id: &str, label: &str, on: bool) {
    let (fid, scope) = fact_id(core, id, label);
    let edit = vec![FactEdit::Share { id: fid, share: on }];
    match scope {
        crate::facts::FactScope::Account => in_account(core, id, core.apply_fact_edits(edit, "share".into())).unwrap(),
        crate::facts::FactScope::Global => {
            in_account(core, id, core.apply_global_fact_edits(edit, "share".into())).unwrap()
        }
    };
}

fn published_json(core: &Core, id: &str) -> Value {
    let (snapshot, _) = block_on(core.rules_snapshot(id)).unwrap();
    serde_json::to_value(snapshot.hashed("pepper")).unwrap()
}

fn keys_of(v: &Value) -> Vec<String> {
    let mut keys: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
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

/// Every string in `v`, keys included.
fn strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|x| strings(x, out)),
        Value::Object(o) => o.iter().for_each(|(k, x)| {
            out.push(k.clone());
            strings(x, out);
        }),
        _ => {}
    }
}

#[test]
fn the_published_about_names_no_address_but_the_mailboxs_own() {
    // An unverified AgentMail mailbox: mailbox mode tells the agent it may
    // write only to the user's own email, naming it. A cloud agent is told
    // that without the address.
    let dir = scratch("about");
    let secrets = Arc::new(MemorySecrets::default());
    let core = open_core(&dir, &secrets);
    let created = block_on(core.clone().create_agent_mailbox(
        AgentService::AgentMail,
        "Scout".into(),
        Some("me@personal.example".into()),
        "req-about".into(),
    ))
    .unwrap();
    let id = created.account_id;
    in_account(&core, &id, guide_and_facts(&core));
    let meta = core.agent_meta(&id).unwrap();
    assert!(core.agent_limits_text(&meta).contains("me@personal.example"), "mailbox mode names it");

    let v = published_json(&core, &id);
    let about = v["mailbox"]["about"].as_str().unwrap();
    assert!(about.contains(&format!("Scout <{}>", meta.address)) && about.contains("AgentMail"), "{about}");
    assert!(about.contains("only to the user's own email"), "{about}");
    assert!(!about.contains("this Mac"), "a cloud agent's drafts are not here: {about}");
    let mut all = Vec::new();
    strings(&v, &mut all);
    for s in &all {
        for word in s.split(|c: char| c.is_whitespace() || "<>(),;:'\"[]".contains(c)) {
            let word = word.trim_end_matches('.');
            if word.contains('@') {
                assert_eq!(word, meta.address, "an address other than the mailbox's own in {s:?}");
            }
        }
    }
    assert!(!v.to_string().contains("personal.example"));

    // Primitive says its own limits, and nothing of the user either.
    let (_t, core, _, id) = setup("about-primitive");
    let about = published_json(&core, &id)["mailbox"]["about"].as_str().unwrap().to_owned();
    assert!(about.contains("exactly one recipient") && about.contains("Primitive"), "{about}");
}

#[test]
fn facts_are_shared_by_their_use_and_store_unless_switched() {
    use crate::facts::FactScope::{Account, Global};
    assert!(crate::facts::shares_with_cloud(FactUse::Free, Account, None));
    assert!(!crate::facts::shares_with_cloud(FactUse::Ask, Account, None), "an unattended agent cannot ask");
    assert!(!crate::facts::shares_with_cloud(FactUse::Free, Global, None), "global facts are the user's own");
    assert!(crate::facts::shares_with_cloud(FactUse::Ask, Account, Some(true)));
    assert!(crate::facts::shares_with_cloud(FactUse::Free, Global, Some(true)));
    assert!(!crate::facts::shares_with_cloud(FactUse::Free, Account, Some(false)));
    assert!(!crate::facts::shares_with_cloud(FactUse::Never, Account, Some(true)), "never, whatever the switch");
}

#[test]
fn a_server_address_is_https_or_this_mac() {
    let s = parse_server(" rules.example.com/ ").unwrap();
    assert_eq!((s.url.as_str(), s.key.as_str()), ("https://rules.example.com", "rules.example.com"));
    let s = parse_server("http://127.0.0.1:8787/").unwrap();
    assert_eq!((s.url.as_str(), s.key.as_str()), ("http://127.0.0.1:8787", "127.0.0.1:8787"));
    assert_eq!(
        parse_server("https://Example.com:8443/rules/").unwrap().endpoint(&["v1", "mailboxes", "a@b.c"]).as_str(),
        "https://example.com:8443/rules/v1/mailboxes/a@b.c"
    );
    assert!(parse_server("http://localhost:9/").is_ok());
    for bad in ["", "http://rules.example.com", "ftp://x.com", "https://u:p@x.com", "https://x.com/?a=1", "https://"] {
        assert!(parse_server(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_snapshot_holds_the_guide_and_shared_facts_and_nothing_else() {
    let (_t, core, secrets, id) = setup("contents");
    core.debug_deliver_to_agent_mailbox(
        id.clone(),
        "Ada <ada@example.com>".into(),
        "Quarterly numbers".into(),
        "MAIL-BODY-NEVER-PUBLISHED".into(),
    )
    .unwrap();
    let v = published_json(&core, &id);
    let text = v.to_string();
    assert_eq!(
        keys_of(&v),
        ["audiences", "entries", "facts", "guide_version", "mailbox", "published_at", "schema_version", "version"]
    );
    assert_eq!(keys_of(&v["mailbox"]), ["about", "address", "name"]);
    assert_eq!(v["mailbox"]["name"], "Scout");
    for e in v["entries"].as_array().unwrap() {
        assert_eq!(keys_of(e), ["category", "check", "id", "kind", "scope", "statement"], "no evidence or origin");
    }
    let statements: Vec<&str> =
        v["entries"].as_array().unwrap().iter().map(|e| e["statement"].as_str().unwrap()).collect();
    assert_eq!(statements.len(), 3, "accepted entries only: {statements:?}");
    assert!(!text.contains("A proposal stays here"));
    // Addresses go as salted hashes: audience members and scoped people.
    let groups = v["audiences"]["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "confirmed groups only");
    assert!(groups[0]["members"].as_array().unwrap().iter().all(|m| writing_guide::is_hash(m.as_str().unwrap())));
    for plain in ["acme.com", "bea@globex.com", "ann@acme.com", "vc@fund.example", "origin-17"] {
        assert!(!text.contains(plain), "{plain} leaked");
    }
    // Defaults (decision 4): Use freely here, yes; Ask, global, Never: no.
    let labels = |v: &Value| -> Vec<String> {
        v["facts"].as_array().unwrap().iter().map(|f| f["label"].as_str().unwrap().to_owned()).collect()
    };
    assert_eq!(labels(&v), ["Calendar link", "Occupation or role"]);
    for kept in ["My assistant", "1 Secret Lane", "Jo Global"] {
        assert!(!text.contains(kept), "{kept} leaked");
    }
    // Never mail or keys.
    assert!(!text.contains("MAIL-BODY-NEVER-PUBLISHED") && !text.contains("Quarterly numbers"));
    for (key, value) in secrets.0.lock().unwrap().iter() {
        assert!(!text.contains(value.as_str()), "{key} leaked");
    }
    // The published form is one the server takes.
    assert!(writing_guide::Snapshot::from_json(&text).is_ok());

    // The switches: Ask on, a Use freely off, the global one on, Never on
    // (which changes nothing).
    share(&core, &id, "Sam Rivera", true);
    share(&core, &id, "Calendar link", false);
    share(&core, &id, "Preferred name", true);
    share(&core, &id, "Mailing address", true);
    let v = published_json(&core, &id);
    assert_eq!(labels(&v), ["Sam Rivera", "Occupation or role", "Preferred name"]);
    assert_eq!(v["facts"][0]["ask_before_using"], true);
    assert!(!v.to_string().contains("1 Secret Lane"));
    let facts = in_account(&core, &id, core.list_facts(vec![FactStatus::Accepted])).unwrap();
    let shared: Vec<(&str, bool)> = facts.iter().map(|f| (f.label.as_str(), f.share_with_cloud)).collect();
    assert!(shared.contains(&("Mailing address", false)) && shared.contains(&("Sam Rivera", true)), "{shared:?}");

    // The sheet's list is the same, people counted.
    let preview = block_on(core.rules_preview(id.clone())).unwrap();
    assert_eq!(preview.entries.len(), 3);
    let annie = preview.entries.iter().find(|e| e.statement == "Call her Annie").unwrap();
    assert_eq!(annie.scope, "to 1 person");
    assert!(preview.entries.iter().any(|e| e.has_check && e.scope.is_empty()));
    assert_eq!(preview.audiences, [RulesPreviewAudience { name: "Customers".into(), members: 2 }]);
    assert_eq!(
        preview.facts.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(),
        ["Sam Rivera", "Occupation or role", "Preferred name"]
    );
    assert_eq!(preview.facts_kept, 1, "Calendar link stays (Never share ones are not counted)");
}

/// An `openagc-rules` on loopback, on its own runtime.
struct RulesServer {
    rt: tokio::runtime::Runtime,
    url: String,
    dir: Temp,
}

impl RulesServer {
    fn start(name: &str) -> Self {
        Self::start_with(name, true)
    }

    /// With `oauth`, its public URL is its loopback address.
    fn start_with(name: &str, oauth: bool) -> Self {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = scratch(&format!("server-{name}"));
        let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let config = rules_server::Config {
            data_dir: dir.0.clone(),
            rate_limit_per_minute: 0,
            registration_token: None,
            public_url: oauth.then(|| url.clone()),
        };
        let app = rules_server::app(&config).unwrap();
        rt.spawn(async move { axum::serve(listener, app).await });
        Self { rt, url, dir }
    }

    fn key(&self, account: &str) -> String {
        keys::rules_publish_token(self.url.trim_start_matches("http://"), account)
    }

    /// What the server holds for `address`: its latest version and JSON.
    fn stored(&self, address: &str) -> Option<(i64, Value)> {
        let db = rules_server::Db::open(&self.dir.0).unwrap();
        let address = address.to_owned();
        db.run_now(move |c| {
            let Some(m) = rules_server::db::mailbox_by_address(c, &address)? else { return Ok(None) };
            Ok(rules_server::db::latest_snapshot(c, m.id)?.map(|s| (s.version, serde_json::from_str(&s.json).unwrap())))
        })
        .unwrap()
    }

    fn call(
        &self,
        method: reqwest::Method,
        path: &str,
        token: &str,
        body: Option<Value>,
        if_match: Option<i64>,
    ) -> (u16, Value) {
        let url = format!("{}{path}", self.url);
        let token = token.to_owned();
        self.rt.block_on(async move {
            let mut r = reqwest::Client::new().request(method, url).bearer_auth(token);
            if let Some(b) = body {
                r = r.json(&b);
            }
            if let Some(v) = if_match {
                r = r.header(IF_MATCH, v.to_string());
            }
            let a = r.send().await.unwrap();
            (a.status().as_u16(), a.json().await.unwrap_or(Value::Null))
        })
    }
}

fn status(core: &Core, id: &str) -> RulesPublication {
    core.rules_publish_status(id.to_owned()).unwrap()
}

#[test]
fn publishing_registers_pushes_on_change_and_recovers_a_lost_version() {
    let server = RulesServer::start("flow");
    let (_t, core, secrets, id) = setup("flow");
    let address = core.agent_meta(&id).unwrap().address;
    assert!(core.rules_publish_status(id.clone()).is_none());

    let s = block_on(core.clone().rules_publish_start(id.clone(), format!("{}/", server.url), None)).unwrap();
    assert_eq!((s.enabled, s.version, s.error.as_deref(), s.pending), (true, Some(1), None, false), "{s:?}");
    assert_eq!(s.server_url, server.url);
    let token = secrets.0.lock().unwrap().get(&server.key(&id)).cloned().expect("publisher token in the Keychain");
    let (version, stored) = server.stored(&address).unwrap();
    assert_eq!(version, 1);
    assert!(writing_guide::is_hash(stored["entries"][2]["scope"]["people"][0].as_str().unwrap()));
    let record = read_record(&record_path(&core.data_path(), &id)).unwrap();
    assert_eq!(stored["audiences"]["salt"], record.salt.as_str());
    assert!(
        !std::fs::read_to_string(record_path(&core.data_path(), &id)).unwrap().contains(&token),
        "token not on disk"
    );

    // An agent reads the guide as in mailbox mode: Annie's entry only for her.
    let (code, minted) = server.call(
        reqwest::Method::POST,
        &format!("/v1/mailboxes/{address}/agent-tokens"),
        &token,
        Some(json!({ "name": "test" })),
        None,
    );
    assert_eq!(code, 201, "{minted}");
    let agent = minted["token"].as_str().unwrap();
    let (_, guide) =
        server.call(reqwest::Method::GET, &format!("/v1/m/{address}/guide?to=ann@acme.com"), agent, None, None);
    assert!(guide["writing_guide"].as_str().unwrap().contains("Call her Annie"), "{guide}");
    assert!(guide["writing_guide"].as_str().unwrap().contains("Occupation or role"));
    let (_, other) =
        server.call(reqwest::Method::GET, &format!("/v1/m/{address}/guide?to=bob@else.example"), agent, None, None);
    assert!(!other["writing_guide"].as_str().unwrap().contains("Call her Annie"));

    // A change is pushed by itself, a moment later.
    in_account(
        &core,
        &id,
        core.apply_guide_edits(vec![rule("Sign as Scout", GuideScope::default(), None)], "t".into()),
    )
    .unwrap();
    wait_for("version 2", || status(&core, &id).version == Some(2));
    assert!(server.stored(&address).unwrap().1.to_string().contains("Sign as Scout"));
    assert!(!status(&core, &id).pending);

    // A version this Mac lost track of (pushed elsewhere with its token):
    // the server refuses If-Match 2, and the push goes above its version.
    let mut lost = serde_json::from_value::<Snapshot>(server.stored(&address).unwrap().1).unwrap();
    lost.version = 7;
    let (code, _) = server.call(
        reqwest::Method::PUT,
        &format!("/v1/mailboxes/{address}/snapshot"),
        &token,
        Some(serde_json::to_value(&lost).unwrap()),
        Some(2),
    );
    assert_eq!(code, 200);
    let s = block_on(core.rules_publish_now(id.clone())).unwrap();
    assert_eq!((s.version, s.error), (Some(8), None));
    assert_eq!(server.stored(&address).unwrap().0, 8);

    // Publish Now pushes a new version even when nothing changed; a change
    // that does not touch what goes (a Never share fact) pushes nothing.
    assert_eq!(block_on(core.rules_publish_now(id.clone())).unwrap().version, Some(9));
    in_account(
        &core,
        &id,
        core.apply_fact_edits(vec![fact("other", "Locker code", "4411", FactUse::Never)], "t".into()),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(status(&core, &id).version, Some(9));

    // The operator forgot the mailbox: the next push registers it again,
    // with a new salt, and the version still goes up.
    let (code, _) = server.call(reqwest::Method::DELETE, &format!("/v1/mailboxes/{address}"), &token, None, None);
    assert_eq!(code, 204);
    let s = block_on(core.rules_publish_now(id.clone())).unwrap();
    assert_eq!((s.version, s.error.as_deref()), (Some(10), None), "{s:?}");
    assert_ne!(read_record(&record_path(&core.data_path(), &id)).unwrap().salt, record.salt);
    assert_ne!(secrets.0.lock().unwrap().get(&server.key(&id)).unwrap(), &token);

    // Stopping keeps the last version on the server and pushes no more.
    block_on(core.rules_publish_stop(id.clone(), false)).unwrap();
    assert!(!status(&core, &id).enabled);
    in_account(&core, &id, core.apply_guide_edits(vec![rule("Ignored", GuideScope::default(), None)], "t".into()))
        .unwrap();
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(server.stored(&address).unwrap().0, 10);
    // Publishing again to the same server reuses the token and version.
    let s = block_on(core.clone().rules_publish_start(id.clone(), server.url.clone(), None)).unwrap();
    assert_eq!(s.version, Some(11));
    // Stopping and removing: the server forgets it, this Mac its token.
    block_on(core.rules_publish_stop(id.clone(), true)).unwrap();
    assert!(server.stored(&address).is_none());
    assert!(core.rules_publish_status(id.clone()).is_none());
    assert!(!secrets.0.lock().unwrap().contains_key(&server.key(&id)));
}

#[test]
fn a_burst_of_changes_is_one_version_and_versions_survive_a_restart() {
    let server = RulesServer::start("burst");
    let (t, core, secrets, id) = setup("burst");
    let address = core.agent_meta(&id).unwrap().address;
    assert_eq!(
        block_on(core.clone().rules_publish_start(id.clone(), server.url.clone(), None)).unwrap().version,
        Some(1)
    );
    core.rules.set_waits(Duration::from_millis(400), Duration::from_secs(5), Duration::from_millis(200));
    for n in 0..3 {
        in_account(
            &core,
            &id,
            core.apply_guide_edits(vec![rule(&format!("Rule {n}"), GuideScope::default(), None)], "t".into()),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(status(&core, &id).pending, "waiting to push");
    wait_for("the burst pushed", || status(&core, &id).version == Some(2));
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(server.stored(&address).unwrap().0, 2, "one version for three changes");
    let text = server.stored(&address).unwrap().1.to_string();
    assert!(text.contains("Rule 0") && text.contains("Rule 2"));

    // Another run of the app: the record and the Keychain carry on.
    drop(core);
    let core = open_core(&t, &secrets);
    core.resume_rules_publishing();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(status(&core, &id).version, Some(2), "nothing changed while closed: nothing pushed");
    share(&core, &id, "Sam Rivera", true);
    wait_for("version 3", || status(&core, &id).version == Some(3));
    assert!(server.stored(&address).unwrap().1.to_string().contains("My assistant"));
}

fn json_response(status: u16, body: Value) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(body)
}

#[test]
fn what_a_server_refuses_comes_back_in_words() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mock = rt.block_on(MockServer::start());
    let (_t, core, secrets, id) = setup("refusals");
    let address = core.agent_meta(&id).unwrap().address;
    let mount = |m: Mock| rt.block_on(m.mount(&mock));
    let start = |token: Option<&str>| {
        block_on(core.clone().rules_publish_start(id.clone(), mock.uri(), token.map(str::to_owned)))
    };

    // Registered already (by someone else, or by this Mac before).
    mount(
        Mock::given(method("POST"))
            .and(path("/v1/mailboxes"))
            .respond_with(json_response(409, json!({})))
            .up_to_n_times(1),
    );
    let e = start(None).unwrap_err().to_string();
    assert!(e.contains("already registered") && e.contains("forget-mailbox"), "{e}");
    assert!(core.rules_publish_status(id.clone()).is_none(), "nothing kept");
    // A closed server.
    mount(
        Mock::given(method("POST"))
            .and(path("/v1/mailboxes"))
            .respond_with(json_response(401, json!({})))
            .up_to_n_times(1),
    );
    assert!(start(None).unwrap_err().to_string().contains("registration token"));

    // Registered; the snapshot is refused (422), then a version is never
    // taken (409 every time).
    mount(
        Mock::given(method("POST"))
            .and(path("/v1/mailboxes"))
            .respond_with(json_response(201, json!({ "address": address, "publisher_token": "pub_token_1" }))),
    );
    let snapshot_path = format!("/v1/mailboxes/{address}/snapshot");
    mount(
        Mock::given(method("PUT"))
            .and(path(snapshot_path.as_str()))
            .respond_with(json_response(
                422,
                json!({ "error": "invalid_snapshot", "message": "the snapshot lists addresses" }),
            ))
            .up_to_n_times(1),
    );
    let s = start(Some("reg")).unwrap();
    assert_eq!(s.version, None);
    let error = s.error.unwrap();
    assert!(error.contains("refused the snapshot") && error.contains("lists addresses"), "{error}");
    let registered = rt.block_on(mock.received_requests()).unwrap();
    let auth = registered.iter().rev().find(|r| r.method == wiremock::http::Method::POST).unwrap();
    assert_eq!(auth.headers.get("authorization").unwrap(), "Bearer reg");
    mount(
        Mock::given(method("PUT"))
            .and(path(snapshot_path.as_str()))
            .respond_with(json_response(409, json!({ "error": "version_not_newer", "current_version": 3 }))),
    );
    let s = block_on(core.rules_publish_now(id.clone())).unwrap();
    assert!(s.error.unwrap().contains("kept refusing"));

    // Every request carried only the snapshot, with the publisher token.
    let puts: Vec<_> = rt
        .block_on(mock.received_requests())
        .unwrap()
        .into_iter()
        .filter(|r| r.method == wiremock::http::Method::PUT)
        .collect();
    assert!(puts.len() >= 2);
    for put in &puts {
        assert_eq!(put.headers.get("authorization").unwrap(), "Bearer pub_token_1");
        let body: Value = serde_json::from_slice(&put.body).unwrap();
        assert!(writing_guide::Snapshot::from_json(&body.to_string()).is_ok());
        let text = body.to_string();
        for (key, value) in secrets.0.lock().unwrap().iter().filter(|(k, _)| !k.starts_with("rules.")) {
            assert!(!text.contains(value.as_str()), "{key} leaked");
        }
    }
    // After the first 409 the push matched the server's version.
    assert!(puts.iter().any(|p| p.headers.get("if-match").is_some_and(|v| v == "\"3\"")));

    // A server that is not there: said so.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let host = closed.local_addr().unwrap().to_string();
    drop(closed);
    core.update_rules_record(&id, |r| r.server_url = format!("http://{host}")).unwrap();
    let s = block_on(core.rules_publish_now(id.clone())).unwrap();
    let error = s.error.unwrap();
    assert!(error.starts_with(&format!("Could not reach {host}")), "{error}");
}

#[test]
fn removing_the_mailbox_removes_it_from_the_server_and_forgets_the_token() {
    let server = RulesServer::start("remove");
    let (_t, core, secrets, id) = setup("remove");
    let address = core.agent_meta(&id).unwrap().address;
    block_on(core.clone().rules_publish_start(id.clone(), server.url.clone(), None)).unwrap();
    assert!(server.stored(&address).is_some());
    block_on(core.remove_account(id.clone())).unwrap();
    assert!(server.stored(&address).is_none(), "the server forgot it");
    assert!(!secrets.0.lock().unwrap().contains_key(&server.key(&id)));
}

#[test]
fn the_app_mints_connect_codes_and_tokens_lists_agents_and_revokes_them() {
    let server = RulesServer::start("agents");
    let (_t, core, secrets, id) = setup("agents");
    let address = core.agent_meta(&id).unwrap().address;
    let refused = block_on(core.rules_connect_code_mint(id.clone(), "Routine".into())).unwrap_err();
    assert_eq!(refused.kind(), ErrorKind::InvalidInput, "not published yet");
    block_on(core.clone().rules_publish_start(id.clone(), server.url.clone(), None)).unwrap();

    let before = mail_sync::now_millis();
    let code = block_on(core.rules_connect_code_mint(id.clone(), "  Weekly   outreach ".into())).unwrap();
    assert_eq!(code.name, "Weekly outreach");
    assert!(rules_server::tokens::normalize_connect_code(&code.code).is_some(), "{}", code.code);
    let ten = rules_server::oauth::CONNECT_CODE_TTL_MS;
    assert!((before + ten - 2_000..=before + ten + 2_000).contains(&code.expires_at));
    let token = block_on(core.rules_agent_token_mint(id.clone(), "A script".into())).unwrap();
    assert!(token.token.starts_with("oagc_agt_"));
    assert!(block_on(core.rules_agent_token_mint(id.clone(), " ".into())).is_err(), "a name is needed");

    // A connector signs in with the code (as the server's consent page
    // would record it): an OAuth grant beside the token.
    let db = rules_server::Db::open(&server.dir.0).unwrap();
    db.run_now(|c| {
        let m = rules_server::db::mailbox_by_address(c, &address)?.unwrap();
        rules_server::db::insert_client(
            c,
            &rules_server::db::ClientRow {
                id: "oagc_cli_test".into(),
                name: "Claude".into(),
                redirect_uris: vec!["https://claude.ai/api/mcp/auth_callback".into()],
                created_at: 1,
            },
        )?;
        rules_server::db::insert_agent_token(
            c,
            &rules_server::db::AgentTokenRow {
                id: "00000000000000aa".into(),
                mailbox_id: m.id,
                name: "Weekly outreach".into(),
                token_hash: String::new(),
                created_at: mail_sync::now_millis() + 1_000,
                revoked_at: None,
                kind: rules_server::db::KIND_OAUTH.into(),
                client_id: Some("oagc_cli_test".into()),
            },
        )
    })
    .unwrap();
    let agents = block_on(core.rules_agents(id.clone())).unwrap();
    assert_eq!(agents.len(), 2, "{agents:?}");
    assert_eq!((agents[0].name.as_str(), agents[0].kind), ("A script", RulesAgentKind::Token));
    assert_eq!(agents[0].id, token.id);
    assert_eq!(agents[1].kind, RulesAgentKind::Connector);
    assert_eq!(agents[1].client_name.as_deref(), Some("Claude"));
    assert!(agents.iter().all(|a| a.revoked_at.is_none() && a.created_at > 0));

    block_on(core.rules_agent_revoke(id.clone(), agents[1].id.clone())).unwrap();
    block_on(core.rules_agent_revoke(id.clone(), agents[1].id.clone())).unwrap();
    let agents = block_on(core.rules_agents(id.clone())).unwrap();
    assert!(agents[1].revoked_at.is_some() && agents[0].revoked_at.is_none());
    let missing = block_on(core.rules_agent_revoke(id.clone(), "00000000000000ff".into())).unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound);
    assert!(block_on(core.rules_agent_revoke(id.clone(), "../x".into())).is_err());

    // Nothing minted is kept on this Mac.
    let kept: Vec<String> = secrets.0.lock().unwrap().values().cloned().collect();
    assert!(!kept.iter().any(|v| v == &token.token || v.contains(&code.code)));
    let record = std::fs::read_to_string(record_path(&core.data_path(), &id)).unwrap();
    assert!(!record.contains(&token.token) && !record.contains(&code.code));

    // A server without OAuth says so, in words.
    let plain = RulesServer::start_with("agents-plain", false);
    block_on(core.rules_publish_stop(id.clone(), true)).unwrap();
    block_on(core.clone().rules_publish_start(id.clone(), plain.url.clone(), None)).unwrap();
    let off = block_on(core.rules_connect_code_mint(id.clone(), "Routine".into())).unwrap_err();
    assert!(off.to_string().contains("OPENAGC_RULES_PUBLIC_URL"), "{off}");
}
