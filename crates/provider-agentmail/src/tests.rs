//! Against a wiremock fake of AgentMail's API; nothing reaches the service.

use std::time::Duration;

use provider_api::token::StaticToken;
use provider_api::{AddedMailbox, MailboxService};
use wiremock::matchers::{body_partial_json, header, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::*;

const INBOX: &str = "scout@agentmail.to";
const KEY: &str = "am_us_test";

fn fast() -> RetryPolicy {
    RetryPolicy { max_attempts: 3, base_delay: Duration::from_millis(1), max_delay: Duration::from_millis(5) }
}

fn provider_for(server: &MockServer, inbox: &str, limiter: Arc<RateLimiter>) -> AgentMailProvider {
    AgentMailProvider::for_inbox(Arc::new(StaticToken(KEY.into())), inbox, inbox, limiter, fast(), &server.uri())
        .unwrap()
}

fn provider(server: &MockServer) -> AgentMailProvider {
    provider_for(server, INBOX, rate_limiter())
}

fn service(server: &MockServer) -> AgentMailService {
    AgentMailService::with_base(&server.uri()).unwrap()
}

fn inbox_path(rest: &str) -> String {
    format!("/v0/inboxes/{INBOX}/{rest}")
}

fn ok(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

fn error(status: u16, code: &str, message: &str, fix: &str) -> ResponseTemplate {
    ResponseTemplate::new(status).set_body_json(json!({
        "name": "Error", "code": code, "message": message, "fix": fix
    }))
}

fn item(id: &str, thread: &str, labels: &[&str], at: &str) -> serde_json::Value {
    json!({
        "inbox_id": INBOX, "thread_id": thread, "message_id": id, "labels": labels,
        "timestamp": at, "from": "Ada <ada@example.com>", "to": [INBOX], "size": 120,
        "updated_at": at, "created_at": at, "subject": format!("About {id}")
    })
}

fn event(id: &str, kind: &str, message: &str, label: &str, at: &str) -> serde_json::Value {
    json!({
        "organization_id": "org", "pod_id": "pod", "inbox_id": INBOX, "event_id": id,
        "event_type": kind, "message_id": message, "label": label, "event_at": at, "created_at": at
    })
}

const RAW: &str = "From: Ada <ada@example.com>\r\nTo: scout@agentmail.to\r\nSubject: Hello\r\n\
Message-ID: <m1@example.com>\r\nDate: Mon, 5 Oct 2026 10:00:00 +0000\r\n\r\nHi Scout\r\n";

fn outgoing(to: &[&str], subject: &str, in_reply_to: Option<&str>, date: Millis) -> Vec<u8> {
    mail_mime::build(&mail_mime::OutgoingMessage {
        from: EmailAddress::new(Some("Scout"), INBOX),
        to: to.iter().map(|t| EmailAddress::new(None, t)).collect(),
        subject: subject.into(),
        html: "<p>Hello there</p>".into(),
        message_id: "q1@kaluta.local".into(),
        in_reply_to: in_reply_to.map(str::to_owned),
        references: in_reply_to.map(|p| vec![p.to_owned()]).unwrap_or_default(),
        date,
        ..Default::default()
    })
    .unwrap()
}

#[tokio::test]
async fn sign_up_gives_the_human_email_and_returns_the_key_inbox_and_address() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v0/agent/sign-up"))
        .and(body_partial_json(json!({ "username": "research-scout", "human_email": "me@example.com" })))
        .respond_with(ok(json!({
            "organization_id": "org_1", "inbox_id": "research-scout@agentmail.to", "api_key": "am_us_new"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let signed = service(&server).sign_up("Research Scout", "req-1", Some("me@example.com")).await.unwrap();
    assert_eq!(signed.api_key.expose(), "am_us_new");
    assert_eq!(signed.address, "research-scout@agentmail.to");
    assert_eq!(signed.inbox_id.as_deref(), Some("research-scout@agentmail.to"));
    assert!(!signed.plan.verified && !signed.plan.reply_only);
}

#[tokio::test]
async fn an_inbox_id_that_is_not_an_address_gives_the_username_on_agentmail() {
    let server = MockServer::start().await;
    Mock::given(path("/v0/agent/sign-up"))
        .respond_with(ok(json!({ "organization_id": "o", "inbox_id": "inb_123", "api_key": "k" })))
        .mount(&server)
        .await;
    let signed = service(&server).sign_up("Scout", "r", Some("me@example.com")).await.unwrap();
    assert_eq!(signed.address, "scout@agentmail.to");
    assert_eq!(signed.inbox_id.as_deref(), Some("inb_123"));
}

#[tokio::test]
async fn verification_resends_to_the_same_human_and_a_code_verifies_the_organisation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v0/agent/human"))
        .and(header("authorization", "Bearer am_us_test"))
        .and(body_partial_json(json!({ "human_email": "me@example.com" })))
        .respond_with(ok(json!({ "human_email": "me@example.com", "instructions": "Check your email" })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v0/agent/verify"))
        .and(body_partial_json(json!({ "otp_code": "000000" })))
        .respond_with(error(400, "validation_error", "Invalid OTP code", ""))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v0/agent/verify"))
        .and(body_partial_json(json!({ "otp_code": "123456" })))
        .respond_with(ok(json!({ "verified": true })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v0/organizations"))
        .respond_with(ok(json!({
            "organization_id": "org", "inbox_count": 1, "domain_count": 0, "inbox_limit": 3,
            "updated_at": "2026-10-08T00:00:00Z", "created_at": "2026-10-08T00:00:00Z"
        })))
        .mount(&server)
        .await;
    let service = service(&server);
    let started = service.start_verification(KEY, "me@example.com").await.unwrap();
    assert_eq!(started.expires_in_secs, 24 * 60 * 60);
    assert_eq!(service.verify(KEY, "000000").await.unwrap_err(), ProviderError::Invalid("Invalid OTP code".into()));
    let plan = service.verify(KEY, " 123456 ").await.unwrap();
    assert!(plan.verified);
    assert_eq!(plan.name, "free");
    // The organisation says nothing of verification: the core keeps it.
    let read = service.plan(KEY).await.unwrap();
    assert_eq!((read.name.as_str(), read.verified), ("free", false));
}

#[tokio::test]
async fn adding_an_inbox_is_idempotent_and_says_when_the_name_is_taken_or_the_plan_is_full() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v0/inboxes"))
        .and(body_partial_json(json!({ "username": "writer", "display_name": "Writer", "client_id": "req-2" })))
        .respond_with(ok(json!({
            "pod_id": "pod", "inbox_id": "writer@agentmail.to", "email": "writer@agentmail.to",
            "updated_at": "2026-10-08T00:00:00Z", "created_at": "2026-10-08T00:00:00Z", "display_name": "Writer"
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v0/inboxes"))
        .and(body_partial_json(json!({ "username": "scout" })))
        .respond_with(error(403, "resource_taken", "Inbox already exists", "Choose a different username"))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v0/inboxes"))
        .and(body_partial_json(json!({ "username": "fourth" })))
        .respond_with(error(403, "limit_exceeded", "Inbox limit reached", "Upgrade your plan"))
        .mount(&server)
        .await;
    let service = service(&server);
    let added = service.add_mailbox(KEY, "writer", None, "Writer", "req-2").await.unwrap();
    assert_eq!(added, AddedMailbox { address: "writer@agentmail.to".into(), inbox_id: "writer@agentmail.to".into() });
    let taken = service.add_mailbox(KEY, "scout", Some("agentmail.to"), "Scout", "req-3").await.unwrap_err();
    assert_eq!(taken, ProviderError::Invalid(USERNAME_TAKEN.into()));
    let full = service.add_mailbox(KEY, "fourth", None, "Fourth", "req-4").await.unwrap_err();
    assert_eq!(full, ProviderError::Invalid(INBOX_LIMIT.into()));
    // The default domain is not sent: AgentMail's own.
    let first = &server.received_requests().await.unwrap()[0];
    assert!(serde_json::from_slice::<serde_json::Value>(&first.body).unwrap().get("domain").is_none());
}

#[tokio::test]
async fn an_inbox_key_needs_a_verified_organisation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(inbox_path("api-keys")))
        .and(header("authorization", "Bearer unverified"))
        .respond_with(error(
            403,
            "missing_permission",
            "Forbidden",
            "Complete agent verification via POST /v0/agent/verify",
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(inbox_path("api-keys")))
        .and(header("authorization", "Bearer verified"))
        .and(body_partial_json(json!({ "name": "Kaluta Scout" })))
        .respond_with(ok(json!({
            "api_key_id": "k1", "api_key": "am_us_inbox", "prefix": "am_us", "name": "Kaluta Scout",
            "created_at": "2026-10-08T00:00:00Z", "inbox_id": INBOX
        })))
        .mount(&server)
        .await;
    let service = service(&server);
    let refused = service.mailbox_api_key("unverified", INBOX, "Kaluta Scout").await.unwrap_err();
    assert_eq!(refused, ProviderError::Forbidden(NOT_VERIFIED_YET.into()));
    let key = service.mailbox_api_key("verified", INBOX, "Kaluta Scout").await.unwrap();
    assert_eq!(key.expose(), "am_us_inbox");
}

#[tokio::test]
async fn listing_pages_through_the_inbox_and_keeps_what_the_filter_asks_for() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .and(query_param_is_missing("page_token"))
        .respond_with(ok(json!({
            "count": 2, "limit": 2, "next_page_token": "p2",
            "messages": [item("m3", "t3", &["sent"], "2026-10-08T10:00:00Z"),
                         item("m2", "t2", &["received", "archived"], "2026-10-08T09:00:00Z")]
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .and(query_param("page_token", "p2"))
        .respond_with(ok(json!({
            "count": 1, "messages": [item("m1", "t1", &["received", "unread"], "2026-10-08T08:00:00Z")]
        })))
        .mount(&server)
        .await;
    let p = provider(&server);
    let all = ListFilter::default();
    let first = p.list_message_ids(&all, None).await.unwrap();
    assert_eq!(first.ids.len(), 2);
    assert_eq!(first.next, Some(PageToken("p2".into())));
    let second = p.list_message_ids(&all, first.next).await.unwrap();
    assert_eq!(second.ids, vec![(MessageId::new("m1"), ThreadId::new("t1"))]);
    assert_eq!(second.next, None);

    // The Inbox: received and not archived.
    let inbox = ListFilter { label_ids: vec![LabelId::new("INBOX")], ..Default::default() };
    assert!(p.list_message_ids(&inbox, None).await.unwrap().ids.is_empty());
    let sent = ListFilter { label_ids: vec![LabelId::new("SENT")], ..Default::default() };
    assert_eq!(p.list_message_ids(&sent, None).await.unwrap().ids, vec![(MessageId::new("m3"), ThreadId::new("t3"))]);
    // Trash is the Mac's: nothing is asked.
    let trash = ListFilter { label_ids: vec![LabelId::new("TRASH")], ..Default::default() };
    let before = server.received_requests().await.unwrap().len();
    assert!(p.list_message_ids(&trash, None).await.unwrap().ids.is_empty());
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test]
async fn a_message_is_fetched_as_raw_mime_with_its_labels_and_without_one_from_its_fields() {
    let server = MockServer::start().await;
    let mut m1 = item("m1", "t1", &["received", "unread", "billing"], "2026-10-08T08:00:00Z");
    m1["text"] = json!("Hi Scout");
    Mock::given(method("GET")).and(path(inbox_path("messages/m1"))).respond_with(ok(m1)).mount(&server).await;
    Mock::given(method("GET"))
        .and(path(inbox_path("messages/m1/raw")))
        .respond_with(ok(json!({
            "message_id": "m1", "size": RAW.len(),
            "download_url": format!("{}/cdn/m1.eml?sig=x", server.uri()), "expires_at": "2026-10-09T00:00:00Z"
        })))
        .mount(&server)
        .await;
    // The signed URL takes no key.
    Mock::given(method("GET"))
        .and(path("/cdn/m1.eml"))
        .respond_with(move |r: &Request| {
            if r.headers.contains_key("authorization") {
                ResponseTemplate::new(400)
            } else {
                ResponseTemplate::new(200).set_body_bytes(RAW.as_bytes())
            }
        })
        .mount(&server)
        .await;
    let mut m2 = item("m2", "t1", &["sent"], "2026-10-08T09:00:00Z");
    m2["text"] = json!("Thanks Ada");
    m2["in_reply_to"] = json!("<m1@example.com>");
    m2["attachments"] =
        json!([{ "attachment_id": "a1", "size": 3, "filename": "a.txt", "content_type": "text/plain" }]);
    Mock::given(method("GET")).and(path(inbox_path("messages/m2"))).respond_with(ok(m2)).mount(&server).await;
    Mock::given(method("GET"))
        .and(path(inbox_path("messages/m2/raw")))
        .respond_with(error(404, "not_found", "Raw message not found", ""))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("messages/m2/attachments/a1")))
        .respond_with(ok(json!({
            "attachment_id": "a1", "size": 3, "download_url": format!("{}/cdn/a1", server.uri()),
            "expires_at": "2026-10-09T00:00:00Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/cdn/a1"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abc".to_vec()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("messages/gone")))
        .respond_with(error(404, "not_found", "Message not found", ""))
        .mount(&server)
        .await;

    let p = provider(&server);
    let ids = [MessageId::new("m1"), MessageId::new("m2"), MessageId::new("gone")];
    let mut fetched = p.fetch_messages(&ids, Priority::Interactive).await.unwrap();
    fetched.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    assert_eq!(fetched.len(), 2, "a message gone is left out");
    let m1 = &fetched[0];
    assert_eq!(m1.subject, "Hello");
    assert_eq!(m1.message_id_header.as_deref(), Some("m1@example.com"));
    assert_eq!(m1.label_ids, vec![LabelId::new("INBOX"), LabelId::new("UNREAD"), LabelId::new("billing")]);
    assert_eq!(m1.thread_id, ThreadId::new("t1"));
    let m2 = &fetched[1];
    assert_eq!(m2.label_ids, vec![LabelId::new("SENT")]);
    assert_eq!(m2.subject, "About m2");
    assert_eq!(m2.in_reply_to.as_deref(), Some("m1@example.com"));
    let body = m2.body.as_ref().unwrap();
    assert_eq!(body.text.as_deref(), Some("Thanks Ada"));
    assert_eq!(body.attachments[0].attachment_id.as_deref(), Some("a1"));
    assert_eq!(p.fetch_attachment(&m2.id, "a1").await.unwrap(), b"abc");
}

#[tokio::test]
async fn label_changes_go_out_as_patch_or_batch_update_and_trash_stays_local() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path(inbox_path("messages/m1")))
        .and(body_partial_json(json!({ "add_labels": ["archived"], "remove_labels": [] })))
        .respond_with(ok(json!({ "message_id": "m1", "labels": ["received", "archived"] })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(inbox_path("messages/batch-update")))
        .and(body_partial_json(json!({
            "message_ids": ["m1", "m2"], "add_labels": ["read"], "remove_labels": ["unread"]
        })))
        .respond_with(ok(json!({ "limit": 2, "count": 2, "updates": [] })))
        .expect(1)
        .mount(&server)
        .await;
    let p = provider(&server);
    let op = |ids: &[&str], add: &[&str], remove: &[&str]| LabelOp {
        message_ids: ids.iter().map(|i| MessageId::new(*i)).collect(),
        add: add.iter().map(|l| LabelId::new(*l)).collect(),
        remove: remove.iter().map(|l| LabelId::new(*l)).collect(),
    };
    p.modify_labels(&op(&["m1"], &[], &["INBOX"])).await.unwrap();
    p.modify_labels(&op(&["m2", "m1", "m2"], &[], &["UNREAD"])).await.unwrap();
    // Trash and spam: nothing goes out.
    p.modify_labels(&op(&["m1"], &["TRASH"], &[])).await.unwrap();
    p.modify_labels(&op(&["m1"], &["SPAM"], &[])).await.unwrap();
    p.move_to_trash(&MessageId::new("m1")).await.unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn polling_finds_new_mail_by_time_and_label_changes_from_the_events_down_to_the_last_seen() {
    let server = MockServer::start().await;
    // At the start: one message and one event.
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .and(query_param_is_missing("after"))
        .respond_with(ok(json!({ "count": 1, "messages": [item("m1", "t1", &["received"], "2026-10-08T08:00:00Z")] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("events")))
        .and(query_param("limit", "1"))
        .respond_with(ok(json!({
            "count": 1, "next_page_token": "older",
            "events": [event("e1", "label.added", "m1", "unread", "2026-10-08T08:00:00Z")]
        })))
        .mount(&server)
        .await;
    let p = provider(&server);
    let profile = p.profile().await.unwrap();
    assert_eq!(profile.email, INBOX);

    // Later: m2 arrived; m1 was read and archived elsewhere, over two pages
    // of events.
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .and(query_param("after", "2026-10-08T07:00:00.000Z"))
        .respond_with(ok(json!({ "count": 2, "messages": [
            item("m2", "t2", &["received", "unread"], "2026-10-08T09:00:00Z"),
            item("m1", "t1", &["received", "archived"], "2026-10-08T08:00:00Z")
        ] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("events")))
        .and(query_param("limit", "100"))
        .and(query_param_is_missing("page_token"))
        .respond_with(ok(json!({ "count": 2, "next_page_token": "e-p2", "events": [
            event("e4", "label.added", "m1", "archived", "2026-10-08T09:30:00Z"),
            event("e3", "label.added", "m1", "read", "2026-10-08T09:20:00Z")
        ] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("events")))
        .and(query_param("page_token", "e-p2"))
        .respond_with(ok(json!({ "count": 2, "next_page_token": "e-p3", "events": [
            event("e2", "label_removed", "m1", "unread", "2026-10-08T09:10:00Z"),
            event("e1", "label.added", "m1", "unread", "2026-10-08T08:00:00Z")
        ] })))
        .mount(&server)
        .await;
    let set = p.changes_since(&profile.cursor).await.unwrap();
    assert_eq!(
        set.changes,
        vec![
            Change::MessageAdded {
                id: MessageId::new("m2"),
                thread_id: ThreadId::new("t2"),
                label_ids: vec![LabelId::new("INBOX"), LabelId::new("UNREAD")]
            },
            Change::LabelsRemoved { id: MessageId::new("m1"), label_ids: vec![LabelId::new("UNREAD")] },
            Change::LabelsRemoved { id: MessageId::new("m1"), label_ids: vec![LabelId::new("INBOX")] },
        ],
        "m1 is not new; the events come oldest first; `read` means nothing more"
    );
    let cursor: wire::Cursor = serde_json::from_str(&set.cursor.0).unwrap();
    assert_eq!(cursor.event.as_deref(), Some("e4"));
    assert_eq!(cursor.seen, vec!["m1".to_owned(), "m2".to_owned()]);
    // Page three was never needed.
    let asked: Vec<String> = server.received_requests().await.unwrap().iter().map(|r| r.url.to_string()).collect();
    assert!(!asked.iter().any(|u| u.contains("e-p3")), "{asked:?}");

    // A cursor the provider cannot read means a full resync.
    assert_eq!(p.changes_since(&SyncCursor("garbage".into())).await.unwrap_err(), ProviderError::CursorExpired);
}

#[tokio::test]
async fn a_send_becomes_agentmails_json_with_the_outbox_header_and_an_idempotency_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(inbox_path("messages/send")))
        .and(header("authorization", "Bearer am_us_test"))
        .and(body_partial_json(json!({
            "to": ["ada@example.com", "bo@example.com"], "subject": "Plans",
            "headers": { "X-Kaluta-Outbox-Id": "q1@kaluta.local" }
        })))
        .respond_with(ok(json!({ "message_id": "sent-1", "thread_id": "t9" })))
        .expect(1)
        .mount(&server)
        .await;
    let p = provider(&server);
    let raw = outgoing(&["ada@example.com", "bo@example.com"], "Plans", None, now_millis());
    assert_eq!(p.send(&raw, None).await.unwrap(), MessageId::new("sent-1"));
    let request = &server.received_requests().await.unwrap()[0];
    let key = request.headers.get("idempotency-key").unwrap().to_str().unwrap().to_owned();
    assert!(key.len() == 32 && key.chars().all(|c| c.is_ascii_hexdigit()), "{key}");
    let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
    assert!(body["html"].as_str().unwrap().contains("Hello there"));
}

#[tokio::test]
async fn a_reply_goes_through_the_reply_endpoint_and_falls_back_to_send_when_the_parent_is_unknown() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(inbox_path("messages/%3Cm1@example.com%3E/reply")))
        .and(body_partial_json(json!({ "to": ["ada@example.com"], "reply_all": false })))
        .respond_with(ok(json!({ "message_id": "sent-2", "thread_id": "t1" })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(inbox_path("messages/%3Cnope@example.com%3E/reply")))
        .respond_with(error(404, "not_found", "Message not found", ""))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(inbox_path("messages/send")))
        .and(body_partial_json(json!({ "headers": { "In-Reply-To": "<nope@example.com>" } })))
        .respond_with(ok(json!({ "message_id": "sent-3", "thread_id": "t5" })))
        .expect(1)
        .mount(&server)
        .await;
    let p = provider(&server);
    let reply = outgoing(&["ada@example.com"], "Re: Hello", Some("m1@example.com"), now_millis());
    assert_eq!(p.send(&reply, None).await.unwrap(), MessageId::new("sent-2"));
    let orphan = outgoing(&["ada@example.com"], "Re: Gone", Some("nope@example.com"), now_millis());
    assert_eq!(p.send(&orphan, None).await.unwrap(), MessageId::new("sent-3"));
}

#[tokio::test]
async fn after_a_timeout_the_retry_finds_the_mail_that_went_out_and_never_sends_twice() {
    let server = MockServer::start().await;
    // The send goes out, but the answer comes too late.
    Mock::given(method("POST"))
        .and(path(inbox_path("messages/send")))
        .respond_with(ok(json!({ "message_id": "sent-1", "thread_id": "t9" })).set_delay(Duration::from_millis(800)))
        .mount(&server)
        .await;
    // The listing leaves the headers out; the message itself has them.
    let mut listed = item("sent-1", "t9", &["sent"], "2026-10-08T10:00:00Z");
    listed["subject"] = json!("Plans");
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .respond_with(ok(json!({ "count": 2, "messages": [
            listed.clone(), item("other", "t2", &["received"], "2026-10-08T09:00:00Z")
        ] })))
        .mount(&server)
        .await;
    listed["headers"] = json!({ "x-kaluta-outbox-id": "q1@kaluta.local" });
    Mock::given(method("GET")).and(path(inbox_path("messages/sent-1"))).respond_with(ok(listed)).mount(&server).await;

    let p = provider(&server).with_send_timeout(Duration::from_millis(200)).unwrap();
    let raw = outgoing(&["ada@example.com"], "Plans", None, now_millis());
    let first = p.send(&raw, None).await.unwrap_err();
    assert!(first.is_transient(), "the outbox retries: {first:?}");
    // The outbox's retry: found, so not sent again.
    assert_eq!(p.send(&raw, None).await.unwrap(), MessageId::new("sent-1"));
    // And a retry in a later run (the message queued long ago) checks too.
    let fresh = provider(&server);
    let old = outgoing(&["ada@example.com"], "Plans", None, now_millis() - 60 * 60 * 1000);
    assert_eq!(fresh.send(&old, None).await.unwrap(), MessageId::new("sent-1"));

    let sends = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path().ends_with("/messages/send"))
        .count();
    assert_eq!(sends, 1, "the message went out once");
}

#[tokio::test]
async fn another_process_finds_a_fresh_send_a_dead_one_left_by_its_header() {
    let server = MockServer::start().await;
    let mut listed = item("sent-1", "t9", &["sent"], "2026-10-08T10:00:00Z");
    listed["subject"] = json!("Plans");
    listed["headers"] = json!({ "x-kaluta-outbox-id": "q1@kaluta.local" });
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .respond_with(ok(json!({ "count": 1, "messages": [listed] })))
        .mount(&server)
        .await;
    // Queued a moment ago, so a send's own check would not look; the
    // outbox asks, since the send was left in flight.
    let raw = outgoing(&["ada@example.com"], "Plans", None, now_millis());
    assert_eq!(provider(&server).already_sent(&raw).await.unwrap(), Some(MessageId::new("sent-1")));
    // Another send (another Message-ID) is not taken for it.
    let other = String::from_utf8(outgoing(&["ada@example.com"], "Other", None, now_millis()))
        .unwrap()
        .replace("q1@kaluta.local", "q2@kaluta.local");
    assert_eq!(provider(&server).already_sent(other.as_bytes()).await.unwrap(), None);
}

#[tokio::test]
async fn a_send_left_in_flight_before_the_rename_is_found_by_its_old_header() {
    let server = MockServer::start().await;
    let mut listed = item("sent-1", "t9", &["sent"], "2026-10-08T10:00:00Z");
    listed["subject"] = json!("Plans");
    listed["headers"] = json!({ "x-openagc-outbox-id": "q1@kaluta.local" });
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .respond_with(ok(json!({ "count": 1, "messages": [listed] })))
        .mount(&server)
        .await;
    let raw = outgoing(&["ada@example.com"], "Plans", None, now_millis());
    assert_eq!(provider(&server).already_sent(&raw).await.unwrap(), Some(MessageId::new("sent-1")));
}

#[tokio::test]
async fn a_retry_with_nothing_found_sends_with_the_same_idempotency_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(inbox_path("messages/send")))
        .respond_with(ResponseTemplate::new(502))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(inbox_path("messages/send")))
        .respond_with(ok(json!({ "message_id": "sent-1", "thread_id": "t9" })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .respond_with(ok(json!({ "count": 0, "messages": [] })))
        .expect(1)
        .mount(&server)
        .await;
    let p = provider(&server);
    let raw = outgoing(&["ada@example.com"], "Plans", None, now_millis());
    assert!(matches!(p.send(&raw, None).await.unwrap_err(), ProviderError::Server { status: 502, .. }));
    assert_eq!(p.send(&raw, None).await.unwrap(), MessageId::new("sent-1"));
    let keys: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .map(|r| r.headers.get("idempotency-key").unwrap().to_str().unwrap().to_owned())
        .collect();
    assert_eq!(keys.len(), 2, "the client itself did not repeat the send");
    assert_eq!(keys[0], keys[1]);
}

#[tokio::test]
async fn sends_say_why_agentmail_refused_them_and_oversized_ones_are_not_tried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(inbox_path("messages/send")))
        .respond_with(error(
            403,
            "message_rejected",
            "Recipient not allowed",
            "Sending is restricted to the human until verification: POST /v0/agent/verify",
        ))
        .mount(&server)
        .await;
    let p = provider(&server);
    let raw = outgoing(&["stranger@example.com"], "Hi", None, now_millis());
    assert_eq!(p.send(&raw, None).await.unwrap_err(), ProviderError::Forbidden(SENDS_ONLY_TO_HUMAN.into()));

    let big = mail_mime::build(&mail_mime::OutgoingMessage {
        from: EmailAddress::new(None, INBOX),
        to: vec![EmailAddress::new(None, "ada@example.com")],
        subject: "Big".into(),
        html: "<p>x</p>".into(),
        message_id: "big@kaluta.local".into(),
        attachments: vec![mail_mime::OutgoingAttachment {
            filename: "big.bin".into(),
            mime_type: "application/octet-stream".into(),
            data: vec![7; 5 * 1024 * 1024],
        }],
        date: now_millis(),
        ..Default::default()
    })
    .unwrap();
    let before = server.received_requests().await.unwrap().len();
    let err = p.send(&big, None).await.unwrap_err();
    assert!(matches!(&err, ProviderError::Invalid(m) if m.contains("6 MB")), "{err:?}");
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test]
async fn a_rate_limit_is_waited_out() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .respond_with(
            error(429, "rate_limit_exceeded", "Too many requests", "Slow down").insert_header("retry-after", "0"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .respond_with(ok(json!({ "count": 0, "messages": [] })))
        .mount(&server)
        .await;
    // A roomy limiter, so the pause after the 429 is short.
    let p = provider_for(&server, INBOX, Arc::new(RateLimiter::new(60_000, 0)));
    let ids = p.list_message_ids(&ListFilter::default(), None).await.unwrap();
    assert!(ids.ids.is_empty());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn two_agents_of_one_organisation_read_their_own_inboxes_with_one_key() {
    let server = MockServer::start().await;
    for (inbox, id) in [("scout@agentmail.to", "s1"), ("writer@agentmail.to", "w1")] {
        Mock::given(method("GET"))
            .and(path(format!("/v0/inboxes/{inbox}/messages")))
            .and(header("authorization", "Bearer am_us_test"))
            .respond_with(ok(json!({ "count": 1, "messages": [item(id, id, &["received"], "2026-10-08T08:00:00Z")] })))
            .mount(&server)
            .await;
    }
    let shared = rate_limiter();
    let scout = provider_for(&server, "scout@agentmail.to", shared.clone());
    let writer = provider_for(&server, "writer@agentmail.to", shared);
    let all = ListFilter::default();
    assert_eq!(scout.list_message_ids(&all, None).await.unwrap().ids[0].0, MessageId::new("s1"));
    assert_eq!(writer.list_message_ids(&all, None).await.unwrap().ids[0].0, MessageId::new("w1"));
}

#[tokio::test]
async fn labels_made_on_the_mac_are_names_and_agentmails_own_are_refused() {
    let server = MockServer::start().await;
    let p = provider(&server);
    let label = p.create_label(" Receipts ", None).await.unwrap();
    assert_eq!((label.id.as_str(), label.name.as_str(), label.kind), ("Receipts", "Receipts", LabelKind::User));
    assert!(matches!(p.create_label("unread", None).await, Err(ProviderError::Invalid(_))));
    assert!(matches!(p.label_sync(), LabelSync::Both(_)) && p.adopts_sent_copies());
    assert!(server.received_requests().await.unwrap().is_empty());
}

fn cursor_at(at: &str, event: &str, event_at: &str) -> SyncCursor {
    cursor_text(&wire::Cursor {
        at: parse_time(at).unwrap(),
        seen: vec![],
        event: Some(event.into()),
        event_at: parse_time(event_at).unwrap(),
    })
}

#[tokio::test]
async fn more_new_mail_than_a_poll_reads_is_an_expired_cursor_not_a_gap() {
    let server = MockServer::start().await;
    // Every page of new mail says there is another.
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .respond_with(ok(json!({ "count": 1, "next_page_token": "more", "messages": [
            item("m9", "t9", &["received"], "2026-10-08T09:00:00Z")
        ] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("events")))
        .respond_with(ok(json!({ "count": 0, "events": [] })))
        .mount(&server)
        .await;
    let p = provider(&server);
    let err = p.changes_since(&cursor_at("2026-10-08T08:00:00Z", "e1", "2026-10-08T08:00:00Z")).await.unwrap_err();
    assert_eq!(err, ProviderError::CursorExpired, "a resync lists what the poll could not");
    let pages =
        server.received_requests().await.unwrap().iter().filter(|r| r.url.path().ends_with("/messages")).count();
    assert_eq!(pages, MAX_PAGES);
}

#[tokio::test]
async fn events_that_are_not_label_changes_are_read_past() {
    let server = MockServer::start().await;
    // The newest event is a message event, with no label (and no message id
    // at the top level): neither the start nor a poll stumbles on it.
    let received = json!({
        "event_id": "e5", "event_type": "message.received", "event_at": "2026-10-08T09:40:00Z",
        "message": { "inbox_id": INBOX, "message_id": "m2" }
    });
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .respond_with(ok(json!({ "count": 0, "messages": [] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("events")))
        .and(query_param("limit", "1"))
        .respond_with(ok(json!({ "count": 1, "events": [received.clone()] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("events")))
        .and(query_param("limit", "100"))
        .respond_with(ok(json!({ "count": 3, "events": [
            received,
            event("e4", "label.added", "m1", "starred", "2026-10-08T09:30:00Z"),
            event("e1", "label.added", "m1", "unread", "2026-10-08T08:00:00Z")
        ] })))
        .mount(&server)
        .await;
    let p = provider(&server);
    let profile = p.profile().await.unwrap();
    let start: wire::Cursor = serde_json::from_str(&profile.cursor.0).unwrap();
    assert_eq!(start.event.as_deref(), Some("e5"));
    let set = p.changes_since(&cursor_at("2026-10-08T08:00:00Z", "e1", "2026-10-08T08:00:00Z")).await.unwrap();
    assert_eq!(
        set.changes,
        vec![Change::LabelsAdded { id: MessageId::new("m1"), label_ids: vec![LabelId::new("STARRED")] }]
    );
    let cursor: wire::Cursor = serde_json::from_str(&set.cursor.0).unwrap();
    assert_eq!(cursor.event.as_deref(), Some("e5"), "the newest event seen, whatever its kind");
}

#[tokio::test]
async fn spam_and_trash_events_move_mail_out_of_the_inbox_and_back_as_agentmail_has_it() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(inbox_path("messages")))
        .respond_with(ok(json!({ "count": 0, "messages": [] })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("events")))
        .respond_with(ok(json!({ "count": 5, "events": [
            event("e6", "label.removed", "m3", "spam", "2026-10-08T09:50:00Z"),
            event("e5", "label.removed", "m2", "trash", "2026-10-08T09:40:00Z"),
            event("e4", "label.added", "m2", "trash", "2026-10-08T09:30:00Z"),
            event("e3", "label.added", "m1", "spam", "2026-10-08T09:20:00Z"),
            event("e1", "label.added", "m1", "unread", "2026-10-08T08:00:00Z")
        ] })))
        .mount(&server)
        .await;
    // Out of Trash, m2 is received mail in the Inbox; m3, out of Spam, was
    // archived there.
    Mock::given(method("GET"))
        .and(path(inbox_path("messages/m2")))
        .respond_with(ok(item("m2", "t2", &["received"], "2026-10-08T07:00:00Z")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(inbox_path("messages/m3")))
        .respond_with(ok(item("m3", "t3", &["received", "archived"], "2026-10-08T07:00:00Z")))
        .mount(&server)
        .await;
    let p = provider(&server);
    let set = p.changes_since(&cursor_at("2026-10-08T08:00:00Z", "e1", "2026-10-08T08:00:00Z")).await.unwrap();
    let ids = |v: &[&str]| v.iter().map(|l| LabelId::new(*l)).collect::<Vec<_>>();
    assert_eq!(
        set.changes,
        vec![
            Change::LabelsAdded { id: MessageId::new("m1"), label_ids: ids(&["SPAM"]) },
            Change::LabelsRemoved { id: MessageId::new("m1"), label_ids: ids(&["INBOX"]) },
            Change::LabelsAdded { id: MessageId::new("m2"), label_ids: ids(&["TRASH"]) },
            Change::LabelsRemoved { id: MessageId::new("m2"), label_ids: ids(&["INBOX"]) },
            Change::LabelsRemoved { id: MessageId::new("m2"), label_ids: ids(&["TRASH"]) },
            Change::LabelsAdded { id: MessageId::new("m2"), label_ids: ids(&["INBOX"]) },
            Change::LabelsRemoved { id: MessageId::new("m3"), label_ids: ids(&["SPAM"]) },
        ],
        "as `to_local` reads the same labels"
    );
}

#[tokio::test]
async fn a_failed_signed_download_never_carries_its_url() {
    // Nothing listens on port 1 of the loopback address: the connection
    // is refused locally, and the error must not quote the signed URL.
    let server = MockServer::start().await;
    let p = provider(&server);
    let signed = "http://127.0.0.1:1/cdn/m1.eml?X-Amz-Signature=deadbeef1234&X-Amz-Credential=AKIAX";
    let err = p.download(signed).await.unwrap_err();
    let text = format!("{err} {err:?}");
    assert!(matches!(err, ProviderError::Network(_)), "{err:?}");
    for leak in ["deadbeef1234", "AKIAX", "X-Amz", "/cdn/m1.eml", "127.0.0.1:1"] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }
}
