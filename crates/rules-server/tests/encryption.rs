//! Encryption at rest (spec §10.6, oagc-gmn7.7), end to end: the app
//! pushes sealed snapshots with the key wrapped per agent, agents read
//! them over MCP and REST with their own credential only, reports are
//! sealed to the app, the database holds no plaintext, a revoked agent
//! opens nothing published after it, and a server can require all this.

mod common;

use common::{MAILBOX, SealingApp, Server, call, contains, snapshot, start, start_requiring};
use reqwest::StatusCode;
use rules_crypto as seal;
use serde_json::{Value, json};
use writing_guide::{Fact, Snapshot};

const GUIDE_MARKER: &str = "zebra-guide-marker-41c7";
const REPORT_MARKER: &str = "okapi-report-marker-93d2";

/// The representative guide with a fact and a rule only this test's
/// marker says.
fn marked(version: i64) -> Snapshot {
    let mut s = snapshot(version);
    s.facts.push(Fact {
        category_key: "work".into(),
        category: "Work".into(),
        label: "Desk".into(),
        value: format!("Room {GUIDE_MARKER}"),
        ask_before_using: false,
    });
    s.entries[1].statement = format!("Be formal, {GUIDE_MARKER}");
    s
}

async fn guide(s: &Server, token: &str) -> (StatusCode, Value) {
    let r = s.get(Some(token), &format!("/v1/m/{MAILBOX}/guide?to=ann@acme.com")).await;
    (r.status(), r.json().await.unwrap_or(Value::Null))
}

async fn post(s: &Server, token: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let r = s.http.post(s.url(path)).bearer_auth(token).json(&body).send().await.unwrap();
    (r.status(), r.json().await.unwrap_or(Value::Null))
}

fn agent_entry<'a>(agents: &'a Value, id: &str) -> &'a Value {
    agents["agent_tokens"].as_array().unwrap().iter().find(|a| a["id"] == id).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn sealed_snapshots_are_read_by_agents_with_their_own_key_and_nothing_plain_is_stored() {
    let s = start(0, None).await;
    let publisher = s.registered().await;
    // An agent minted before the mailbox ever pushed encrypted has no key.
    let (early_id, early) = s.mint(&publisher, "Weekly outreach routine").await;
    assert!(agent_entry(&s.agents(&publisher).await, &early_id)["agent_key"].is_null());

    let mut app = SealingApp::new();
    let (status, body) = s.push_sealed(&publisher, &mut app, &marked(1), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"], 1);

    // Its next request makes its key; until the app wraps the snapshot key
    // for it, it is told it cannot read yet.
    let (status, body) = guide(&s, &early).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("not_readable")), "{body}");
    let listed = s.agents(&publisher).await;
    assert!(agent_entry(&listed, &early_id)["agent_key"].is_string());
    assert_eq!(agent_entry(&listed, &early_id)["readable"], false);
    assert_eq!(s.rewrap(&publisher, &app).await, 1);
    let (status, body) = guide(&s, &early).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["writing_guide"].as_str().unwrap().contains(GUIDE_MARKER));
    assert_eq!(body["version"], 1);

    // One minted now has its key at once: a wrap makes it read.
    let (late_id, late) = s.mint(&publisher, "Claude Code").await;
    assert!(agent_entry(&s.agents(&publisher).await, &late_id)["agent_key"].is_string());
    assert_eq!(s.rewrap(&publisher, &app).await, 1, "only the one that could not read");
    let client = s.mcp(&late).await.expect("connect");
    let facts = call(&client, "facts_lookup", json!({ "query": "Room" })).await;
    assert!(facts.to_string().contains(GUIDE_MARKER), "{facts}");
    let checked = call(
        &client,
        "check_draft",
        json!({ "to": ["ann@acme.com"], "subject": "Plan", "body_markdown": "Let's circle back" }),
    )
    .await;
    assert_eq!(checked["guide_check"][0], "Uses “circle back”, which your rules ban");

    // The next push: a new key, wrapped for both, read by both.
    let (status, _) = s.push_sealed(&publisher, &mut app, &marked(2), Some("1")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(guide(&s, &early).await.1["version"], 2);
    assert_eq!(call(&client, "guide_rules", json!({})).await["version"], 2);

    // A report, sealed to the app.
    let report = json!({
        "message_id": "<r1@mail.example>", "to": ["ann@acme.com"], "subject": "Plan",
        "body_markdown": format!("Hi Ann, {REPORT_MARKER}, let's circle back"), "checked_version": 2,
    });
    let (status, filed) = post(&s, &late, &format!("/v1/m/{MAILBOX}/reports"), report).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{filed}");
    assert_eq!(
        filed["guide_check"][0], "Uses “circle back”, which your rules ban",
        "checked in plaintext, then sealed"
    );

    // What the server wrote holds neither the guide nor the report, nor
    // any token: only boxes and hashes.
    let stored = s.stored_bytes();
    assert!(!stored.is_empty());
    for secret in [GUIDE_MARKER, REPORT_MARKER, "circle back", "cal.com/scout", early.as_str(), late.as_str()] {
        assert!(!contains(&stored, secret), "the database holds {secret:?}");
    }

    // The app pulls the report and opens it with its private key.
    let r = s.get(Some(&publisher), &format!("/v1/mailboxes/{MAILBOX}/reports")).await;
    let pulled: Value = r.json().await.unwrap();
    let report = &pulled["reports"][0];
    assert_eq!((report["subject"].as_str(), report["body_markdown"].as_str()), (Some(""), Some("")));
    let sealed = seal::unb64(report["sealed"].as_str().unwrap()).unwrap();
    let opened = seal::open_for_app(&app.key, &sealed, &seal::report_context(MAILBOX, &late_id)).unwrap();
    let opened: Value = serde_json::from_slice(&opened).unwrap();
    assert_eq!(opened["message_id"], "r1@mail.example");
    assert_eq!(opened["to"], json!(["ann@acme.com"]));
    assert!(opened["body_markdown"].as_str().unwrap().contains(REPORT_MARKER));
    assert_eq!(opened["guide_check"][0], "Uses “circle back”, which your rules ban");
    assert!(seal::open_for_app(&SealingApp::new().key, &sealed, &seal::report_context(MAILBOX, &late_id)).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn only_an_agents_own_token_opens_its_key_and_a_revoked_one_opens_nothing_published_after() {
    let s = start(0, None).await;
    let publisher = s.registered().await;
    let mut app = SealingApp::new();
    assert_eq!(s.push_sealed(&publisher, &mut app, &marked(1), None).await.0, StatusCode::OK);
    let (gone_id, gone) = s.mint(&publisher, "Old routine").await;
    let (kept_id, kept) = s.mint(&publisher, "New routine").await;
    assert_eq!(s.rewrap(&publisher, &app).await, 2);

    // The stored key wrap opens with the agent's own token only.
    let before = s.backup();
    let wrap_of = |c: &rusqlite::Connection, id: &str| -> Option<Vec<u8>> {
        c.query_row("SELECT key_wrap FROM agent_tokens WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    };
    let gone_wrap = wrap_of(&before, &gone_id).unwrap();
    let gone_key = seal::unwrap_agent_key(&seal::credential_key(&gone, &gone_id), &gone_wrap, &gone_id).unwrap();
    assert!(seal::unwrap_agent_key(&seal::credential_key(&kept, &gone_id), &gone_wrap, &gone_id).is_err());
    let forged = rules_server::tokens::agent_token(&gone_id);
    assert!(seal::unwrap_agent_key(&seal::credential_key(&forged, &gone_id), &gone_wrap, &gone_id).is_err());
    // A wrong token is not let in either.
    assert_eq!(guide(&s, &forged).await.0, StatusCode::UNAUTHORIZED);

    // Revoked: refused, and its key and wraps are gone from the database.
    let r = s
        .http
        .delete(s.url(&format!("/v1/mailboxes/{MAILBOX}/agent-tokens/{gone_id}")))
        .bearer_auth(&publisher)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(guide(&s, &gone).await.0, StatusCode::UNAUTHORIZED);
    let after = s.backup();
    assert_eq!(wrap_of(&after, &gone_id), None);
    let wraps_for = |c: &rusqlite::Connection, id: &str| -> i64 {
        c.query_row("SELECT COUNT(*) FROM snapshot_keys WHERE agent_id = ?1", [id], |r| r.get(0)).unwrap()
    };
    assert_eq!(wraps_for(&after, &gone_id), 0);

    // The next push is under a key wrapped for the agent still connected.
    assert_eq!(s.push_sealed(&publisher, &mut app, &marked(2), Some("1")).await.0, StatusCode::OK);
    assert_eq!(guide(&s, &kept).await.1["version"], 2);
    let latest = s.backup();
    assert_eq!(wraps_for(&latest, &gone_id), 0, "nothing new is wrapped for a revoked agent");
    let (key_id, sealed): (String, Vec<u8>) = latest
        .query_row("SELECT key_id, sealed FROM snapshots WHERE version = 2", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    // Even with the key it had, from a backup made before it was revoked,
    // the revoked agent opens no wrap of version 2's key, in any copy.
    let mut tried = 0;
    for copy in [&before, &after, &latest] {
        let mut stmt = copy.prepare("SELECT agent_id, wrap FROM snapshot_keys WHERE key_id = ?1").unwrap();
        let rows: Vec<(String, Vec<u8>)> =
            stmt.query_map([&key_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        for (agent, wrap) in rows {
            tried += 1;
            assert_ne!(agent, gone_id);
            assert!(seal::unwrap_snapshot_key(&gone_key, &wrap, &agent, &key_id).is_err());
            assert!(seal::unwrap_snapshot_key(&gone_key, &wrap, &gone_id, &key_id).is_err());
        }
    }
    assert_eq!(tried, 1, "version 2's key is wrapped for the one agent left");
    assert!(seal::open_snapshot(&seal::SecretKey::random(), &key_id, MAILBOX, 2, Some(1), &sealed).is_err());
    // What the revoked agent could read while connected stays what it was.
    let old_wrap: Vec<u8> = before
        .query_row(
            "SELECT k.wrap FROM snapshot_keys k JOIN snapshots s ON s.key_id = k.key_id \
             WHERE s.version = 1 AND k.agent_id = ?1",
            [&gone_id],
            |r| r.get(0),
        )
        .unwrap();
    let (old_key_id, old_sealed): (String, Vec<u8>) = before
        .query_row("SELECT key_id, sealed FROM snapshots WHERE version = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    let old_key = seal::unwrap_snapshot_key(&gone_key, &old_wrap, &gone_id, &old_key_id).unwrap();
    assert!(
        seal::open_snapshot(&old_key, &old_key_id, MAILBOX, 1, Some(1), &old_sealed).unwrap().contains(GUIDE_MARKER)
    );
    let _ = kept_id;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mailbox_that_published_plaintext_drops_it_at_its_first_encrypted_push() {
    let s = start(0, None).await;
    let publisher = s.registered().await;
    let (_, agent) = s.mint(&publisher, "Routine").await;
    assert_eq!(s.publish(&publisher, None, marked(1).to_json().unwrap()).await.0, StatusCode::OK);
    assert_eq!(guide(&s, &agent).await.0, StatusCode::OK, "plaintext still works when not required");
    assert!(contains(&s.stored_bytes(), GUIDE_MARKER));

    let mut app = SealingApp::new();
    assert_eq!(s.push_sealed(&publisher, &mut app, &marked(2), Some("1")).await.0, StatusCode::OK);
    let stored = s.stored_bytes();
    assert!(!contains(&stored, GUIDE_MARKER), "the plaintext version is wiped, file and log");
    let kept: i64 = s.backup().query_row("SELECT COUNT(*) FROM snapshots", [], |r| r.get(0)).unwrap();
    assert_eq!(kept, 1);
    // The agent connected before: its key is made at its next request, and
    // it reads once the app has wrapped for it.
    assert_eq!(guide(&s, &agent).await.0, StatusCode::CONFLICT);
    assert_eq!(s.rewrap(&publisher, &app).await, 1);
    assert_eq!(guide(&s, &agent).await.1["version"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_requires_encryption_refuses_plaintext() {
    let s = start_requiring(true).await;
    let info: Value = s.get(None, "/v1/server").await.json().await.unwrap();
    assert_eq!(info["encryption"], "required");
    let publisher = s.registered().await;
    let (_, agent) = s.mint(&publisher, "Routine").await;
    let (status, body) = s.publish(&publisher, None, marked(1).to_json().unwrap()).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("encryption_required")));
    let report = json!({ "to": ["ann@acme.com"], "subject": "Plan", "body_markdown": "Hi" });
    let (status, body) = post(&s, &agent, &format!("/v1/m/{MAILBOX}/reports"), report.clone()).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("not_encrypted")));

    let mut app = SealingApp::new();
    assert_eq!(s.push_sealed(&publisher, &mut app, &marked(1), None).await.0, StatusCode::OK);
    assert_eq!(post(&s, &agent, &format!("/v1/m/{MAILBOX}/reports"), report).await.0, StatusCode::ACCEPTED);

    let optional = start_requiring(false).await;
    let info: Value = optional.get(None, "/v1/server").await.json().await.unwrap();
    assert_eq!(info["encryption"], "optional");
}

#[tokio::test(flavor = "multi_thread")]
async fn sealed_pushes_are_checked_as_far_as_the_server_can() {
    let s = start(0, None).await;
    let publisher = s.registered().await;
    let app = SealingApp::new();
    let key = seal::SecretKey::random();
    let good = seal::SealedSnapshot {
        encryption: seal::ENCRYPTION_VERSION,
        key_id: "k1".into(),
        version: 1,
        published_at: 1,
        schema_version: 1,
        address: MAILBOX.into(),
        ciphertext: seal::b64(&seal::seal_snapshot(&key, "k1", MAILBOX, 1, 1, "{}")),
        app_key: app.key.public_base64(),
        wraps: vec![],
    };
    for (bad, why) in [
        (seal::SealedSnapshot { encryption: 1, ..good.clone() }, "encryption 1"),
        (seal::SealedSnapshot { encryption: 3, ..good.clone() }, "encryption 3"),
        // The schema is bound to the box, so the server refuses one it
        // cannot read before storing anything.
        (seal::SealedSnapshot { schema_version: 99, ..good.clone() }, "schema_version 99"),
        (seal::SealedSnapshot { schema_version: 0, ..good.clone() }, "schema_version 0"),
        (seal::SealedSnapshot { address: "other@agents.example".into(), ..good.clone() }, "another mailbox"),
        (seal::SealedSnapshot { key_id: "a b".into(), ..good.clone() }, "key_id"),
        (seal::SealedSnapshot { ciphertext: "!!".into(), ..good.clone() }, "ciphertext"),
        (seal::SealedSnapshot { app_key: seal::b64(&[0u8; 32]), ..good.clone() }, "app_key"),
        (
            seal::SealedSnapshot {
                wraps: vec![seal::KeyWrap { agent_id: "nope".into(), wrap: seal::b64(b"x") }],
                ..good.clone()
            },
            "agent",
        ),
    ] {
        let (status, body) = s.publish(&publisher, None, serde_json::to_string(&bad).unwrap()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{why}: {body}");
        assert!(body["message"].as_str().unwrap().contains(why), "{why}: {body}");
    }
    assert_eq!(s.publish(&publisher, None, serde_json::to_string(&good).unwrap()).await.0, StatusCode::OK);
    // Wraps only for a kept key.
    let r = s
        .http
        .post(s.url(&format!("/v1/mailboxes/{MAILBOX}/snapshot/keys")))
        .bearer_auth(&publisher)
        .json(&json!({ "key_id": "k9", "wraps": [] }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn after_the_apps_key_changes_agents_are_told_not_readable_never_an_older_version() {
    let s = start(0, None).await;
    let publisher = s.registered().await;
    let (_, agent) = s.mint(&publisher, "Routine").await;
    let mut app = SealingApp::new();
    assert_eq!(s.push_sealed(&publisher, &mut app, &marked(1), None).await.0, StatusCode::OK);
    let _ = guide(&s, &agent).await;
    assert_eq!(s.rewrap(&publisher, &app).await, 1);
    assert_eq!(guide(&s, &agent).await.1["version"], 1);
    // The Mac lost its key pair (a new Mac, a Keychain reset): its next
    // push cannot open the agent's key, so version 2 is wrapped for nobody.
    let mut reset = SealingApp::new();
    let (status, body) = s.push_sealed(&publisher, &mut reset, &marked(2), Some("1")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = guide(&s, &agent).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::CONFLICT, Some("not_readable")),
        "never version 1: {body}"
    );
    let client = s.mcp(&agent).await.expect("connect");
    let refused = client
        .call_tool(rmcp::model::CallToolRequestParams::new("guide_rules").with_arguments(serde_json::Map::new()))
        .await
        .unwrap();
    assert_eq!(refused.is_error, Some(true), "{refused:?}");
    // Its key is sealed to the new app key at that request; the app wraps.
    assert_eq!(s.rewrap(&publisher, &reset).await, 1);
    assert_eq!(guide(&s, &agent).await.1["version"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_filed_in_plaintext_are_sealed_at_the_first_encrypted_push_which_is_never_undone() {
    let s = start(0, None).await;
    let publisher = s.registered().await;
    let (agent_id, agent) = s.mint(&publisher, "Routine").await;
    assert_eq!(s.publish(&publisher, None, marked(1).to_json().unwrap()).await.0, StatusCode::OK);
    let report = json!({ "to": ["ann@acme.com"], "subject": "Plan", "body_markdown": format!("Hi, {REPORT_MARKER}") });
    assert_eq!(post(&s, &agent, &format!("/v1/m/{MAILBOX}/reports"), report).await.0, StatusCode::ACCEPTED);
    assert!(contains(&s.stored_bytes(), REPORT_MARKER), "kept in plaintext while the mailbox is");

    let mut app = SealingApp::new();
    assert_eq!(s.push_sealed(&publisher, &mut app, &marked(2), Some("1")).await.0, StatusCode::OK);
    let stored = s.stored_bytes();
    for plain in [REPORT_MARKER, GUIDE_MARKER] {
        assert!(!contains(&stored, plain), "{plain} is left in the file or its log");
    }
    // The app opens the report it would have read in plaintext.
    let pulled: Value =
        s.get(Some(&publisher), &format!("/v1/mailboxes/{MAILBOX}/reports")).await.json().await.unwrap();
    let r = &pulled["reports"][0];
    assert_eq!(r["body_markdown"], "");
    let sealed = seal::unb64(r["sealed"].as_str().unwrap()).unwrap();
    let opened = seal::open_for_app(&app.key, &sealed, &seal::report_context(MAILBOX, &agent_id)).unwrap();
    let opened: Value = serde_json::from_slice(&opened).unwrap();
    assert!(opened["body_markdown"].as_str().unwrap().contains(REPORT_MARKER));
    assert_eq!(opened["to"], json!(["ann@acme.com"]));

    // A plaintext push now is refused, on a server that allows plaintext.
    let (status, body) = s.publish(&publisher, Some("2"), marked(3).to_json().unwrap()).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("encryption_required")));
    assert!(!contains(&s.stored_bytes(), GUIDE_MARKER));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_comes_to_require_encryption_serves_nothing_stored_in_plaintext() {
    let optional = start(0, None).await;
    let publisher = optional.registered().await;
    let (_, agent) = optional.mint(&publisher, "Routine").await;
    assert_eq!(optional.publish(&publisher, None, marked(1).to_json().unwrap()).await.0, StatusCode::OK);
    assert_eq!(guide(&optional, &agent).await.0, StatusCode::OK);
    // The operator turns KALUTA_RULES_REQUIRE_ENCRYPTION on.
    let required = common::start_in(optional.dir.clone(), true).await;
    let (status, body) = guide(&required, &agent).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("not_readable")), "{body}");
    assert!(body["message"].as_str().unwrap().contains("required encryption"), "{body}");
    let (status, body) = post(
        &required,
        &agent,
        &format!("/v1/m/{MAILBOX}/check"),
        json!({ "to": ["ann@acme.com"], "body_markdown": "Hi" }),
    )
    .await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::CONFLICT, Some("not_readable")));
    // The app's next push is encrypted; agents read again once wrapped.
    let mut app = SealingApp::new();
    assert_eq!(required.push_sealed(&publisher, &mut app, &marked(2), Some("1")).await.0, StatusCode::OK);
    let _ = guide(&required, &agent).await;
    assert_eq!(required.rewrap(&publisher, &app).await, 1);
    assert_eq!(guide(&required, &agent).await.1["version"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn forgetting_revoking_and_acknowledging_leave_nothing_in_the_file_or_its_log() {
    let s = start(0, None).await;
    let publisher = s.registered().await;
    let (agent_id, agent) = s.mint(&publisher, "Routine").await;
    assert_eq!(s.publish(&publisher, None, marked(1).to_json().unwrap()).await.0, StatusCode::OK);
    let report = json!({ "to": ["ann@acme.com"], "subject": "Plan", "body_markdown": format!("Hi, {REPORT_MARKER}") });
    assert_eq!(post(&s, &agent, &format!("/v1/m/{MAILBOX}/reports"), report).await.0, StatusCode::ACCEPTED);
    assert!(contains(&s.stored_bytes(), REPORT_MARKER));
    let (status, _) =
        post(&s, &publisher, &format!("/v1/mailboxes/{MAILBOX}/reports/ack"), json!({ "up_to_id": 1 })).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!contains(&s.stored_bytes(), REPORT_MARKER), "an acknowledged report is gone from file and log");

    // A revoked agent's key wrap is gone from both.
    let mut app = SealingApp::new();
    assert_eq!(s.push_sealed(&publisher, &mut app, &marked(2), Some("1")).await.0, StatusCode::OK);
    let _ = guide(&s, &agent).await;
    let wrap: Vec<u8> =
        s.backup().query_row("SELECT key_wrap FROM agent_tokens WHERE id = ?1", [&agent_id], |r| r.get(0)).unwrap();
    let r = s
        .http
        .delete(s.url(&format!("/v1/mailboxes/{MAILBOX}/agent-tokens/{agent_id}")))
        .bearer_auth(&publisher)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let stored = s.stored_bytes();
    assert!(!stored.windows(wrap.len()).any(|w| w == wrap.as_slice()), "the revoked key wrap is left");

    // Forgotten: the mailbox's rows are gone from both.
    let r = s.http.delete(s.url(&format!("/v1/mailboxes/{MAILBOX}"))).bearer_auth(&publisher).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert!(!contains(&s.stored_bytes(), MAILBOX), "the forgotten address is left");
}
