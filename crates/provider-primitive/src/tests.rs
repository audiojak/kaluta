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
    let signed = service.sign_up("Scout", "k1", Some("me@example.com")).await.unwrap();
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
        .and(query_param("limit", "100"))
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

#[tokio::test]
async fn where_the_mailbox_may_send_is_read_broadest_first() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/send-permissions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "success": true, "data": [
            { "type": "managed_zone", "zone": "primitive.email", "description": "..." },
            { "type": "your_domain", "domain": "agents.example.com", "description": "..." },
            { "type": "address", "address": "ada@example.com", "last_received_at": "2026-10-06T00:00:00Z",
              "received_count": 2, "description": "..." },
            { "type": "something_new", "description": "..." }
        ], "meta": { "address_cap": 500, "truncated": false } })))
        .mount(&server)
        .await;
    let rules = PrimitiveService::with_base(&server.uri()).unwrap().send_rules("k").await.unwrap();
    assert_eq!(
        rules,
        vec![
            SendRule::ManagedZone("primitive.email".into()),
            SendRule::YourDomain("agents.example.com".into()),
            SendRule::Address("ada@example.com".into()),
        ]
    );
}

// Several agents on one Primitive account (ADR 0015): scout (the first) and
// writer, which has moved to an own domain and keeps its managed address.

const SCOUT: &str = "scout@abc.primitive.email";
const WRITER: &str = "writer@agents.example.com";
const WRITER_MANAGED: &str = "writer@abc.primitive.email";

fn scout_routing() -> Routing {
    Routing { own: vec![SCOUT.into()], others: vec![WRITER.into(), WRITER_MANAGED.into()], catch_all: true }
}

fn writer_routing() -> Routing {
    Routing { own: vec![WRITER.into(), WRITER_MANAGED.into()], others: vec![SCOUT.into()], catch_all: false }
}

fn agent(server: &MockServer, address: &str, routing: RoutingSource, limiter: &Arc<RateLimiter>) -> PrimitiveProvider {
    PrimitiveProvider::for_agent(
        Arc::new(StaticToken("prim_test".into())),
        address,
        routing,
        limiter.clone(),
        fast(),
        &server.uri(),
    )
    .unwrap()
}

fn fixed(routing: Routing) -> RoutingSource {
    Arc::new(move || routing.clone())
}

/// A received message: its record (with `to_email` unless `None`) and raw form.
async fn mount_inbound(server: &MockServer, id: &str, to_email: Option<&str>, raw_headers: &str) {
    Mock::given(path(format!("/emails/{id}")))
        .respond_with(ok(json!({
            "id": id, "thread_id": null, "status": "completed", "received_at": "2026-10-08T10:00:00Z",
            "to_email": to_email
        })))
        .mount(server)
        .await;
    let raw = format!("From: Ada <ada@example.com>\r\n{raw_headers}Subject: {id}\r\n\r\nHello\r\n");
    Mock::given(path(format!("/emails/{id}/raw")))
        .respond_with(ResponseTemplate::new(200).set_body_raw(raw.into_bytes(), "message/rfc822"))
        .mount(server)
        .await;
}

async fn mount_sent(server: &MockServer, id: &str, from: &str) {
    Mock::given(path(format!("/sent-emails/{id}")))
        .respond_with(ok(json!({
            "id": id, "thread_id": null, "status": "delivered", "created_at": "2026-10-08T11:00:00Z",
            "from_header": from, "to_header": "ada@example.com", "subject": id, "body_size_bytes": 2,
            "body_text": "Hi"
        })))
        .mount(server)
        .await;
}

async fn ids_of(p: &PrimitiveProvider) -> Vec<MessageId> {
    let mut ids = Vec::new();
    let mut page = None;
    loop {
        let listed = p.list_message_ids(&ListFilter::default(), page).await.unwrap();
        ids.extend(listed.ids.into_iter().map(|(id, _)| id));
        match listed.next {
            Some(next) => page = Some(next),
            None => return ids,
        }
    }
}

async fn fetched(p: &PrimitiveProvider, ids: &[MessageId]) -> Vec<(MessageId, Vec<LabelId>)> {
    let mut out: Vec<_> =
        p.fetch_messages(ids, Priority::Background).await.unwrap().into_iter().map(|m| (m.id, m.label_ids)).collect();
    out.sort();
    out
}

fn inbox(marked: bool) -> Vec<LabelId> {
    let mut labels = vec![LabelId::new("INBOX"), LabelId::new("UNREAD")];
    if marked {
        labels.push(LabelId::new(OTHER_ADDRESSES_LABEL));
        labels.sort();
    }
    labels
}

#[tokio::test]
async fn two_agents_on_one_account_each_keep_their_own_received_and_sent_mail() {
    let server = MockServer::start().await;
    // The listing carries recipients for some rows and not for others.
    Mock::given(path("/emails"))
        .respond_with(ok_page(
            json!([
                { "id": "i1", "thread_id": null, "to_email": "Scout@abc.primitive.email" },
                { "id": "i2", "thread_id": null, "to_email": "writer+news@abc.primitive.email" },
                { "id": "i3", "thread_id": null, "to_email": "sales@abc.primitive.email" },
                { "id": "i4", "thread_id": null },
                { "id": "i5", "thread_id": null },
                { "id": "i6", "thread_id": null, "to_email": "writer@agents.example.com" }
            ]),
            None,
            6,
        ))
        .mount(&server)
        .await;
    Mock::given(path("/sent-emails"))
        .respond_with(ok_page(
            json!([
                { "id": "o1", "thread_id": null, "from_header": "\"Scout\" <scout@abc.primitive.email>" },
                { "id": "o2", "thread_id": null },
                { "id": "o3", "thread_id": null }
            ]),
            None,
            3,
        ))
        .mount(&server)
        .await;
    mount_inbound(&server, "i1", Some("Scout@abc.primitive.email"), "To: scout@abc.primitive.email\r\n").await;
    mount_inbound(&server, "i2", Some("writer+news@abc.primitive.email"), "To: ada@example.com\r\n").await;
    mount_inbound(&server, "i3", Some("sales@abc.primitive.email"), "To: sales@abc.primitive.email\r\n").await;
    // No `to_email` on the record: the raw message says where it went.
    mount_inbound(&server, "i4", None, "Delivered-To: writer@abc.primitive.email\r\nTo: list@example.org\r\n").await;
    mount_inbound(&server, "i5", None, "To: Bob <bob@example.com>\r\nCc: scout+x@abc.primitive.email\r\n").await;
    mount_inbound(&server, "i6", Some("writer@agents.example.com"), "To: writer@agents.example.com\r\n").await;
    mount_sent(&server, "o1", "\"Scout\" <scout@abc.primitive.email>").await;
    mount_sent(&server, "o2", "\"Writer\" <Writer@agents.example.com>").await;
    mount_sent(&server, "o3", "someone@abc.primitive.email").await;

    let limiter = rate_limiter();
    let scout = agent(&server, SCOUT, fixed(scout_routing()), &limiter);
    let writer = agent(&server, WRITER, fixed(writer_routing()), &limiter);

    // Listing leaves out the rows that say they are another's.
    let (in_, out) = (inbound_id, outbound_id);
    let scout_ids = ids_of(&scout).await;
    assert_eq!(scout_ids, vec![in_("i1"), in_("i3"), in_("i4"), in_("i5"), out("o1"), out("o2"), out("o3")]);
    let writer_ids = ids_of(&writer).await;
    assert_eq!(writer_ids, vec![in_("i2"), in_("i4"), in_("i5"), in_("i6"), out("o2"), out("o3")]);

    // Fetching decides the rest: the record's recipient, else the raw one's.
    assert_eq!(
        fetched(&scout, &scout_ids).await,
        vec![
            (in_("i1"), inbox(false)),
            (in_("i3"), inbox(true)),
            (in_("i5"), inbox(false)),
            (out("o1"), vec![LabelId::new("SENT")]),
            (out("o3"), vec![LabelId::new(OTHER_ADDRESSES_LABEL), LabelId::new("SENT")]),
        ],
        "scout: its own, a +tag of its own, and what no agent has (marked)"
    );
    assert_eq!(
        fetched(&writer, &writer_ids).await,
        vec![
            (in_("i2"), inbox(false)),
            (in_("i4"), inbox(false)),
            (in_("i6"), inbox(false)),
            (out("o2"), vec![LabelId::new("SENT")])
        ],
        "writer: its managed address with a +tag, Delivered-To, its own domain, and what it sent"
    );
    // Even asked for another's message by id, an agent does not take it.
    assert!(fetched(&writer, &[in_("i1"), in_("i3")]).await.is_empty());

    // The marker label is listed where it is used.
    let has_marker =
        |labels: Vec<Label>| labels.iter().any(|l| l.id.as_str() == OTHER_ADDRESSES_LABEL && l.kind == LabelKind::User);
    assert!(has_marker(scout.list_labels().await.unwrap()));
    assert!(!has_marker(writer.list_labels().await.unwrap()));
    // The raw form of another agent's mail with a recipient on its record
    // is never asked for.
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.iter().filter(|r| r.url.path() == "/emails/i1/raw").count(), 1, "scout's fetch only");
}

#[tokio::test]
async fn two_agents_long_poll_one_feed_with_their_own_cursors() {
    let server = MockServer::start().await;
    Mock::given(path("/changes"))
        .and(query_param("since", "start"))
        .respond_with(ok(json!({ "changes": [], "next_cursor": "base", "has_more": false, "baseline": true })))
        .mount(&server)
        .await;
    // Reading the feed does not consume it: the same cursor gives the same
    // changes to whoever asks.
    let arrival = ok(json!({
        "changes": [
            { "kind": "email.visible", "email_id": "i9", "sent_email_id": null, "thread_id": null,
              "changed_at": "2026-10-08T12:00:00Z" },
            { "kind": "sent_email.created", "email_id": null, "sent_email_id": "o9", "thread_id": null,
              "changed_at": "2026-10-08T12:00:01Z" }
        ],
        "next_cursor": "next", "has_more": false, "baseline": false
    }));
    Mock::given(path("/changes"))
        .and(query_param("since", "base"))
        .respond_with(arrival.set_delay(Duration::from_millis(100)))
        .mount(&server)
        .await;
    Mock::given(path("/emails")).respond_with(ok_page(json!([]), None, 0)).mount(&server).await;
    Mock::given(path("/sent-emails")).respond_with(ok_page(json!([]), None, 0)).mount(&server).await;
    mount_inbound(&server, "i9", Some("writer@abc.primitive.email"), "To: writer@abc.primitive.email\r\n").await;
    mount_sent(&server, "o9", "scout@abc.primitive.email").await;

    let limiter = rate_limiter();
    let scout = Arc::new(agent(&server, SCOUT, fixed(scout_routing()), &limiter));
    let writer = Arc::new(agent(&server, WRITER, fixed(writer_routing()), &limiter));
    scout.profile().await.unwrap();
    writer.profile().await.unwrap();
    // Both wait at once; each wakes.
    let (scout_push, writer_push) = (PrimitivePush(scout.clone()), PrimitivePush(writer.clone()));
    let (a, b) = tokio::join!(scout_push.watch(Duration::from_secs(5)), writer_push.watch(Duration::from_secs(5)));
    assert_eq!((a.unwrap(), b.unwrap()), (Some(true), Some(true)));
    let polls = server.received_requests().await.unwrap();
    let waits = polls.iter().filter(|r| r.url.query().is_some_and(|q| q.contains("since=base") && q.contains("wait=")));
    assert_eq!(waits.count(), 2, "one long-poll each, from each one's cursor");

    // Each syncs the same changes and keeps its own.
    for (p, mine) in [(&scout, outbound_id("o9")), (&writer, inbound_id("i9"))] {
        let set = p.changes_since(&SyncCursor("base".into())).await.unwrap();
        assert_eq!(set.cursor, SyncCursor("next".into()));
        let added: Vec<MessageId> = set
            .changes
            .iter()
            .filter_map(|c| match c {
                Change::MessageAdded { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        let kept: Vec<MessageId> = fetched(p, &added).await.into_iter().map(|(id, _)| id).collect();
        assert_eq!(kept, vec![mine]);
    }
}

#[tokio::test]
async fn when_an_agent_is_removed_the_first_takes_its_mail_unmarked_once_alone() {
    let server = MockServer::start().await;
    mount_inbound(&server, "i1", Some("writer@abc.primitive.email"), "").await;
    mount_inbound(&server, "i2", Some("sales@abc.primitive.email"), "").await;
    let routing = Arc::new(Mutex::new(scout_routing()));
    let source: RoutingSource = {
        let routing = routing.clone();
        Arc::new(move || routing.lock().unwrap().clone())
    };
    let scout = agent(&server, SCOUT, source, &rate_limiter());
    let ids = [inbound_id("i1"), inbound_id("i2")];
    assert_eq!(fetched(&scout, &ids).await, vec![(inbound_id("i2"), inbox(true))]);
    // Writer is removed: its mail has no agent now, and scout is alone.
    routing.lock().unwrap().others.clear();
    assert_eq!(fetched(&scout, &ids).await, vec![(inbound_id("i1"), inbox(false)), (inbound_id("i2"), inbox(false))]);
    assert!(scout.list_labels().await.unwrap().iter().all(|l| l.id.as_str() != OTHER_ADDRESSES_LABEL));
}

#[tokio::test]
async fn a_second_agent_sends_from_its_own_address() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/send-mail"))
        .and(body_partial_json(json!({ "from": "\"Writer\" <writer@agents.example.com>", "to": "ada@example.com" })))
        .respond_with(ok(json!({ "id": "o5", "status": "queued", "accepted": ["ada@example.com"], "rejected": [] })))
        .expect(1)
        .mount(&server)
        .await;
    let writer = agent(&server, WRITER, fixed(writer_routing()), &rate_limiter());
    // Whatever From the composer wrote, the agent's address goes.
    let raw = "From: Writer <scout@abc.primitive.email>\r\nTo: ada@example.com\r\nSubject: Hi\r\nMessage-ID: <w1@x>\r\n\r\nHi\r\n";
    assert_eq!(writer.send(raw.as_bytes(), None).await.unwrap(), outbound_id("o5"));
}

// A long-poll that starts before the first sync has a cursor waits for it,
// not for its whole wait: an agent's push loop that started first would
// otherwise hear nothing for up to 20 s (oagc-7ouz).
#[tokio::test]
async fn a_long_poll_without_a_cursor_returns_when_the_first_cursor_is_seen() {
    let server = MockServer::start().await;
    let writer = Arc::new(agent(&server, WRITER, fixed(writer_routing()), &rate_limiter()));
    let waiting = tokio::spawn({
        let writer = writer.clone();
        async move { writer.wait_for_change(Duration::from_secs(600)).await }
    });
    tokio::task::yield_now().await;
    writer.remember_cursor("c0");
    let answered = tokio::time::timeout(Duration::from_secs(5), waiting).await;
    assert!(matches!(answered, Ok(Ok(Ok(false)))), "{answered:?}");
    // A cursor seen before the wait starts: it long-polls from it at once.
    Mock::given(path("/changes"))
        .and(query_param("since", "c0"))
        .respond_with(ok(json!({ "changes": [{ "kind": "email.visible", "email_id": "i1", "thread_id": null }],
                                 "next_cursor": "c1", "has_more": false, "baseline": false })))
        .mount(&server)
        .await;
    assert!(writer.wait_for_change(Duration::from_secs(600)).await.unwrap());
}
