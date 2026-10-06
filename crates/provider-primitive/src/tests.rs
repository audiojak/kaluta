//! Against a wiremock fake of Primitive's API; nothing reaches the service.

use std::time::Duration;

use provider_api::token::StaticToken;
use wiremock::matchers::{body_partial_json, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

fn fast() -> RetryPolicy {
    RetryPolicy { max_attempts: 2, base_delay: Duration::from_millis(1), max_delay: Duration::from_millis(5) }
}

fn provider(server: &MockServer) -> PrimitiveProvider {
    PrimitiveProvider::with_base(
        Arc::new(StaticToken("prim_test".into())),
        "scout@abc.primitive.email",
        fast(),
        &server.uri(),
    )
    .unwrap()
}

fn ok(data: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": data }))
}

fn ok_page(data: serde_json::Value, cursor: Option<&str>, total: u64) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "success": true, "data": data, "meta": { "total": total, "limit": 100, "cursor": cursor }
    }))
}

fn limits(hour: u32, day: u32) -> serde_json::Value {
    json!({ "storage_mb": 100, "send_per_hour": hour, "send_per_day": day, "api_per_minute": 120,
            "webhooks_max_global": 1, "webhooks_per_domain": false, "filters_per_domain": false,
            "spam_thresholds_per_domain": false })
}

const RAW: &str = "From: Ada <ada@example.com>\r\nTo: scout@abc.primitive.email\r\nSubject: Hello\r\n\
Message-ID: <m1@example.com>\r\nDate: Mon, 5 Oct 2026 10:00:00 +0000\r\n\r\nHi Scout\r\n";

#[tokio::test]
async fn sign_up_accepts_the_terms_and_returns_the_key_address_and_plan() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent/accounts"))
        .and(header("idempotency-key", "k1"))
        .and(body_partial_json(json!({ "terms_accepted": true, "device_name": "Scout" })))
        .respond_with(ok(json!({
            "api_key": "prim_new", "org_id": "00000000-0000-0000-0000-000000000001",
            "address": "scout@abc.primitive.email", "plan": "agent", "limits": limits(10, 50),
            "upgrade": { "plan": "developer", "claim_path": "/agent/claim/start" }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let service = PrimitiveService::with_base(&server.uri()).unwrap();
    let signed = service.sign_up("Scout", "k1").await.unwrap();
    assert_eq!(signed.api_key.expose(), "prim_new");
    assert_eq!(signed.address, "scout@abc.primitive.email");
    assert_eq!(
        signed.plan,
        MailboxPlan {
            name: "agent".into(),
            verified: false,
            reply_only: true,
            send_per_hour: 10,
            send_per_day: 50,
            email: None
        }
    );
}

#[tokio::test]
async fn verification_sends_a_code_and_a_wrong_code_says_why() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/agent/claim/start"))
        .and(header("authorization", "Bearer prim_key"))
        .and(body_partial_json(json!({ "email": "me@example.com" })))
        .respond_with(ok(json!({ "claim_session_id": "c", "resend_after_seconds": 30, "expires_in_seconds": 600 })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/agent/claim/verify"))
        .and(body_partial_json(json!({ "verification_code": "000000" })))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "success": false, "error": { "code": "invalid_code", "message": "That code is not right" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/agent/claim/verify"))
        .and(body_partial_json(json!({ "verification_code": "123456" })))
        .respond_with(ok(json!({
            "org_id": "00000000-0000-0000-0000-000000000001", "plan": "developer",
            "email": "me@example.com", "limits": limits(1000, 10000)
        })))
        .mount(&server)
        .await;
    let service = PrimitiveService::with_base(&server.uri()).unwrap();
    let started = service.start_verification("prim_key", "me@example.com").await.unwrap();
    assert_eq!(started, VerificationStarted { resend_after_secs: 30, expires_in_secs: 600 });
    let err = service.verify("prim_key", "000000").await.unwrap_err();
    assert_eq!(err, ProviderError::Invalid("That code is not right".into()));
    let plan = service.verify("prim_key", " 123456 ").await.unwrap();
    assert!(plan.verified && !plan.reply_only);
    assert_eq!(plan.email.as_deref(), Some("me@example.com"));
}

#[tokio::test]
async fn the_plan_comes_from_the_account() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/account"))
        .respond_with(ok(json!({
            "id": "00000000-0000-0000-0000-000000000001", "email": "scout@abc.primitive.email",
            "plan": "agent", "limits": limits(10, 50), "entitlements": [],
            "managed_inbox_address": "scout@abc.primitive.email", "created_at": "2026-10-06T00:00:00Z",
            "discard_content_on_webhook_confirmed": false
        })))
        .mount(&server)
        .await;
    let plan = PrimitiveService::with_base(&server.uri()).unwrap().plan("prim_key").await.unwrap();
    assert!(!plan.verified);
    // The managed inbox is not a verified email.
    assert_eq!(plan.email, None);
}

#[tokio::test]
async fn listing_walks_received_then_sent_mail_and_filters_by_label() {
    let server = MockServer::start().await;
    Mock::given(path("/emails"))
        .and(query_param("cursor", "c2"))
        .respond_with(ok_page(json!([{ "id": "i2", "thread_id": null }]), None, 2))
        .mount(&server)
        .await;
    Mock::given(path("/emails"))
        .respond_with(ok_page(json!([{ "id": "i1", "thread_id": "t1" }]), Some("c2"), 2))
        .mount(&server)
        .await;
    Mock::given(path("/sent-emails"))
        .respond_with(ok_page(json!([{ "id": "o1", "thread_id": "t1" }]), None, 1))
        .mount(&server)
        .await;
    let p = provider(&server);
    let all = ListFilter::default();
    let first = p.list_message_ids(&all, None).await.unwrap();
    assert_eq!(first.ids, vec![(inbound_id("i1"), ThreadId("t1".into()))]);
    let second = p.list_message_ids(&all, first.next).await.unwrap();
    assert_eq!(second.ids, vec![(inbound_id("i2"), ThreadId("in:i2".into()))]);
    let third = p.list_message_ids(&all, second.next).await.unwrap();
    assert_eq!(third.ids, vec![(outbound_id("o1"), ThreadId("t1".into()))]);
    assert_eq!(third.next, None);

    let sent = ListFilter { label_ids: vec![LabelId::new("SENT")], ..Default::default() };
    let only = p.list_message_ids(&sent, None).await.unwrap();
    assert_eq!(only.ids, vec![(outbound_id("o1"), ThreadId("t1".into()))]);
    // Spam, Trash and searches Primitive cannot run list nothing.
    for filter in [
        ListFilter { label_ids: vec![LabelId::new("TRASH")], ..Default::default() },
        ListFilter { query: Some("in:inbox category:promotions".into()), ..Default::default() },
    ] {
        assert!(p.list_message_ids(&filter, None).await.unwrap().ids.is_empty());
    }
}

#[test]
fn a_window_lists_from_a_date() {
    let filter = ListFilter { query: Some("newer_than:30d".into()), ..Default::default() };
    let (sources, since) = plan_listing(&filter, 31 * 86_400_000).unwrap();
    assert_eq!(sources, vec![Source::Inbound, Source::Outbound]);
    assert_eq!(since.as_deref(), Some("1970-01-02T00:00:00Z"));
    let unread =
        ListFilter { label_ids: vec![LabelId::new("INBOX")], query: Some("is:unread".into()), ..Default::default() };
    assert_eq!(plan_listing(&unread, 0).unwrap().0, vec![Source::Inbound]);
}

#[tokio::test]
async fn received_mail_is_parsed_from_its_raw_form_and_falls_back_to_the_record() {
    let server = MockServer::start().await;
    Mock::given(path("/emails/i1"))
        .respond_with(ok(json!({
            "id": "i1", "thread_id": "t1", "status": "completed", "received_at": "2026-10-05T10:00:05Z",
            "raw_size_bytes": 200
        })))
        .mount(&server)
        .await;
    Mock::given(path("/emails/i1/raw"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(RAW.as_bytes().to_vec(), "message/rfc822"))
        .mount(&server)
        .await;
    Mock::given(path("/emails/i2"))
        .respond_with(ok(json!({
            "id": "i2", "thread_id": null, "status": "rejected", "received_at": "2026-10-05T11:00:00Z",
            "from_header": "Bob <bob@example.com>", "to_email": "scout@abc.primitive.email",
            "subject": "Spammy", "body_text": "Buy now", "message_id": "<m2@example.com>",
            "content_discarded_at": "2026-10-05T11:00:01Z"
        })))
        .mount(&server)
        .await;
    let p = provider(&server);
    let mut fetched = p
        .fetch_messages(&[inbound_id("i1"), inbound_id("i2"), inbound_id("gone")], Priority::Background)
        .await
        .unwrap();
    fetched.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(fetched.len(), 2, "a missing message is left out");
    let m = &fetched[0];
    assert_eq!(m.id, inbound_id("i1"));
    assert_eq!(m.thread_id, ThreadId("t1".into()));
    assert_eq!(m.label_ids, vec![LabelId::new("INBOX"), LabelId::new("UNREAD")]);
    assert_eq!(m.subject, "Hello");
    assert_eq!(m.from.as_ref().unwrap().email, "ada@example.com");
    assert_eq!(m.message_id_header.as_deref(), Some("m1@example.com"));
    assert_eq!(m.internal_date, 1_791_194_405_000);
    assert_eq!(m.body.as_ref().unwrap().text.as_deref().map(str::trim), Some("Hi Scout"));
    let spam = &fetched[1];
    assert_eq!(spam.label_ids, vec![LabelId::new("SPAM"), LabelId::new("UNREAD")]);
    assert_eq!(spam.from.as_ref().unwrap().email, "bob@example.com");
    assert_eq!(spam.message_id_header.as_deref(), Some("m2@example.com"));
    assert_eq!(spam.body.as_ref().unwrap().text.as_deref(), Some("Buy now"));
    let requests = server.received_requests().await.unwrap();
    assert!(!requests.iter().any(|r| r.url.path() == "/emails/i2/raw"), "discarded content is not asked for");
}

#[tokio::test]
async fn sent_mail_is_rebuilt_from_its_record() {
    let server = MockServer::start().await;
    Mock::given(path("/sent-emails/o1"))
        .respond_with(ok(json!({
            "id": "o1", "thread_id": "t1", "status": "delivered", "created_at": "2026-10-05T12:00:00Z",
            "from_header": "Scout <scout@abc.primitive.email>", "to_header": "Ada <ada@example.com>",
            "subject": "Re: Hello", "body_size_bytes": 12, "message_id": "<o1@primitive>",
            "in_reply_to": "<m1@example.com>", "email_references": "<m0@example.com> <m1@example.com>",
            "body_text": "Hi Ada", "attachments": [
                { "filename": "a.txt", "content_type": "text/plain", "size_bytes": 3, "sha256": "x",
                  "part_index": 2, "tar_path": "a.txt" }
            ]
        })))
        .mount(&server)
        .await;
    Mock::given(path("/sent-emails/o1/attachments/2"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(b"abc".to_vec(), "text/plain"))
        .mount(&server)
        .await;
    let p = provider(&server);
    let m = p.fetch_messages(&[outbound_id("o1")], Priority::Interactive).await.unwrap().remove(0);
    assert_eq!(m.label_ids, vec![LabelId::new("SENT")]);
    assert_eq!(m.to[0].email, "ada@example.com");
    assert_eq!(m.in_reply_to.as_deref(), Some("m1@example.com"));
    assert_eq!(m.references, vec!["m0@example.com".to_owned(), "m1@example.com".to_owned()]);
    let attachment = &m.body.as_ref().unwrap().attachments[0];
    assert_eq!(attachment.attachment_id.as_deref(), Some("2"));
    assert_eq!(p.fetch_attachment(&m.id, "2").await.unwrap(), b"abc");
}

#[tokio::test]
async fn changes_map_to_added_and_deleted_mail_across_pages() {
    let server = MockServer::start().await;
    let row = |kind: &str, email: Option<&str>, sent: Option<&str>| {
        json!({ "kind": kind, "email_id": email, "sent_email_id": sent, "thread_id": "t1", "status": null,
                "agent_address": null, "reason": null, "changed_at": "2026-10-05T12:00:00Z" })
    };
    Mock::given(path("/changes"))
        .and(query_param("since", "c1"))
        .respond_with(ok(json!({
            "changes": [row("email.visible", Some("i1"), None), row("thread.read_state_changed", None, None)],
            "next_cursor": "c2", "has_more": true, "baseline": false
        })))
        .mount(&server)
        .await;
    Mock::given(path("/changes"))
        .and(query_param("since", "c2"))
        .respond_with(ok(json!({
            "changes": [row("sent_email.created", None, Some("o1")), row("email.deleted", Some("i0"), None)],
            "next_cursor": "c3", "has_more": false, "baseline": false
        })))
        .mount(&server)
        .await;
    Mock::given(path("/changes"))
        .and(query_param("since", "old"))
        .respond_with(ResponseTemplate::new(410).set_body_json(json!({
            "success": false, "error": { "code": "cursor_expired", "message": "Changes were pruned" }
        })))
        .mount(&server)
        .await;
    let p = provider(&server);
    let set = p.changes_since(&SyncCursor("c1".into())).await.unwrap();
    assert_eq!(set.cursor, SyncCursor("c3".into()));
    assert_eq!(
        set.changes,
        vec![
            Change::MessageAdded {
                id: inbound_id("i1"),
                thread_id: ThreadId("t1".into()),
                label_ids: vec![LabelId::new("INBOX")]
            },
            Change::MessageAdded {
                id: outbound_id("o1"),
                thread_id: ThreadId("t1".into()),
                label_ids: vec![LabelId::new("SENT")]
            },
            Change::MessageDeleted { id: inbound_id("i0") },
        ]
    );
    assert_eq!(p.changes_since(&SyncCursor("old".into())).await.unwrap_err(), ProviderError::CursorExpired);
}

#[tokio::test]
async fn the_profile_takes_the_baseline_cursor_and_push_long_polls_from_it() {
    let server = MockServer::start().await;
    Mock::given(path("/changes"))
        .and(query_param("since", "start"))
        .respond_with(ok(json!({ "changes": [], "next_cursor": "base", "has_more": false, "baseline": true })))
        .mount(&server)
        .await;
    Mock::given(path("/changes"))
        .and(query_param("since", "base"))
        .and(query_param("wait", "5"))
        .respond_with(ok(json!({
            "changes": [{ "kind": "email.visible", "email_id": "i9", "sent_email_id": null, "thread_id": null,
                          "status": null, "agent_address": null, "reason": null,
                          "changed_at": "2026-10-05T12:00:00Z" }],
            "next_cursor": "base2", "has_more": false, "baseline": false
        })))
        .mount(&server)
        .await;
    Mock::given(path("/emails")).respond_with(ok_page(json!([]), None, 3)).mount(&server).await;
    Mock::given(path("/sent-emails")).respond_with(ok_page(json!([]), None, 2)).mount(&server).await;
    let p = Arc::new(provider(&server));
    let profile = p.profile().await.unwrap();
    assert_eq!(profile.email, "scout@abc.primitive.email");
    assert_eq!(profile.messages_total, 5);
    assert_eq!(profile.cursor, SyncCursor("base".into()));
    let push = PrimitivePush(p);
    assert_eq!(push.watch(Duration::from_secs(5)).await.unwrap(), Some(true));
}

#[tokio::test]
async fn sending_posts_one_recipient_with_threading_and_an_idempotency_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/send-mail"))
        .and(header_exists("idempotency-key"))
        .and(body_partial_json(json!({
            "from": "\"Scout\" <scout@abc.primitive.email>", "to": "ada@example.com", "subject": "Re: Hello",
            "in_reply_to": "<m1@example.com>", "references": ["<m1@example.com>"]
        })))
        .respond_with(ok(json!({
            "id": "o2", "status": "queued", "from": "scout@abc.primitive.email", "queue_id": null,
            "accepted": ["ada@example.com"], "rejected": [], "client_idempotency_key": "k", "request_id": "r",
            "content_hash": "h", "idempotent_replay": false
        })))
        .expect(1)
        .mount(&server)
        .await;
    let p = provider(&server);
    let raw = "From: Scout <scout@abc.primitive.email>\r\nTo: Ada <ada@example.com>\r\nSubject: Re: Hello\r\n\
Message-ID: <r1@openagc>\r\nIn-Reply-To: <m1@example.com>\r\nReferences: <m1@example.com>\r\n\r\nThanks\r\n";
    assert_eq!(p.send(raw.as_bytes(), None).await.unwrap(), outbound_id("o2"));

    let two = "From: scout@abc.primitive.email\r\nTo: a@example.com\r\nCc: b@example.com\r\nSubject: x\r\n\r\nx\r\n";
    assert_eq!(p.send(two.as_bytes(), None).await.unwrap_err(), ProviderError::Forbidden(ONE_RECIPIENT_ONLY.into()));
}

#[test]
fn an_empty_subject_is_sent_as_no_subject() {
    let raw = "From: scout@abc.primitive.email\r\nTo: ada@example.com\r\nSubject: \r\n\r\nHi\r\n";
    let body = send_body(raw.as_bytes(), "scout@abc.primitive.email").unwrap();
    assert_eq!(body["subject"], "(no subject)");
    let raw = "From: scout@abc.primitive.email\r\nTo: ada@example.com\r\n\r\nHi\r\n";
    assert_eq!(send_body(raw.as_bytes(), "scout@abc.primitive.email").unwrap()["subject"], "(no subject)");
}

#[test]
fn the_idempotency_key_follows_the_message_id() {
    let a = "Message-ID: <same@x>\r\nSubject: one\r\n\r\na";
    let b = "Message-ID: <same@x>\r\nSubject: two\r\n\r\nb";
    assert_eq!(idempotency_key(a.as_bytes()), idempotency_key(b.as_bytes()));
    assert_ne!(idempotency_key(a.as_bytes()), idempotency_key(b"Subject: none\r\n\r\nc"));
}

#[tokio::test]
async fn labels_drafts_and_trash_stay_on_the_mac() {
    let server = MockServer::start().await;
    let p = provider(&server);
    p.modify_labels(&LabelOp { message_ids: vec![inbound_id("i1")], add: vec![], remove: vec![LabelId::new("INBOX")] })
        .await
        .unwrap();
    p.move_to_trash(&inbound_id("i1")).await.unwrap();
    assert_eq!(p.save_draft(None, b"x", None).await.unwrap(), "local");
    let label = p.create_label("Clients/Acme", None).await.unwrap();
    assert_eq!(label.id, LabelId::new("Local_Clients_Acme"));
    assert!(server.received_requests().await.unwrap().is_empty(), "nothing reached Primitive");
}

fn record(purpose: &str, status: &str) -> serde_json::Value {
    json!({ "type": if purpose == "inbound_mx" { "MX" } else { "TXT" }, "name": "agents",
            "fqdn": format!("{purpose}.agents.example.com"), "value": "v", "priority": 10, "ttl": 300,
            "required": true, "purpose": purpose, "status": status })
}

#[tokio::test]
async fn a_domain_is_added_with_its_records_checked_and_listed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/domains"))
        .and(body_partial_json(json!({ "domain": "agents.example.com" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "success": true, "data": {
            "id": "d1", "org_id": "o", "domain": "agents.example.com", "verified": false,
            "verification_token": "t", "created_at": "2026-10-06T00:00:00Z",
            "dns_records": [record("inbound_mx", "pending"), record("dkim", "pending")]
        }})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/domains/d1/verify"))
        .respond_with(ok(json!({ "verified": false, "mxFound": true, "txtFound": false,
                                 "dns_records": [record("inbound_mx", "found"), record("dkim", "missing")],
                                 "error": "DKIM not found" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/domains"))
        .respond_with(ok(json!([{ "id": "d1", "org_id": "o", "domain": "agents.example.com", "verified": false,
                                  "is_active": true, "created_at": "2026-10-06T00:00:00Z", "dns_health": "healthy" }])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/domains/d1/zone-file"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("agents.example.com. 300 IN MX 10 in.primitive.dev.\n"),
        )
        .mount(&server)
        .await;
    let service = PrimitiveService::with_base(&server.uri()).unwrap();
    let added = service.add_domain("k", " Agents.Example.com. ").await.unwrap();
    assert_eq!(added.domain, "agents.example.com");
    assert_eq!(added.records.len(), 2);
    assert_eq!(added.records[0].kind, "MX");
    let checked = service.verify_domain("k", "d1").await.unwrap();
    assert!(!checked.verified);
    assert_eq!(checked.domain, "agents.example.com");
    assert_eq!(checked.records.iter().map(|r| r.status.as_str()).collect::<Vec<_>>(), vec!["found", "missing"]);
    assert!(service.zone_file("k", "d1").await.unwrap().contains("IN MX"));
}

#[tokio::test]
async fn a_domain_that_receives_mail_elsewhere_suggests_a_subdomain() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/domains"))
        .and(body_partial_json(json!({ "domain": "example.com" })))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "success": false, "error": { "code": "mx_conflict", "message": "MX points elsewhere" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/domains"))
        .and(body_partial_json(json!({ "domain": "taken.example" })))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "success": false, "error": { "code": "conflict", "message": "Domain already claimed" }
        })))
        .mount(&server)
        .await;
    let service = PrimitiveService::with_base(&server.uri()).unwrap();
    assert_eq!(
        service.add_domain("k", "example.com").await.unwrap_err(),
        ProviderError::Invalid(DOMAIN_RECEIVES_ELSEWHERE.into())
    );
    assert_eq!(
        service.add_domain("k", "taken.example").await.unwrap_err(),
        ProviderError::Invalid(DOMAIN_TAKEN.into())
    );
}

#[tokio::test]
async fn a_refused_recipient_is_said_in_plain_words() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/send-mail"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({
            "success": false, "error": { "code": "recipient_not_allowed",
            "message": "cannot send to john@actual.ai. All granted recipient-scope gates denied:\n- send_to_known_addresses: ..." }
        })))
        .mount(&server)
        .await;
    let raw = "From: scout@abc.primitive.email\r\nTo: john@actual.ai\r\nSubject: x\r\nMessage-ID: <x@y>\r\n\r\nx\r\n";
    match provider(&server).send(raw.as_bytes(), None).await.unwrap_err() {
        ProviderError::Forbidden(m) => {
            assert!(m.starts_with("Primitive won't send to john@actual.ai:"), "{m}");
            assert!(m.contains("written to it first"));
        }
        other => panic!("{other:?}"),
    }
}
