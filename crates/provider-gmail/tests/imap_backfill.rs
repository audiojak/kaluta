//! IMAP backfill against the in-process fake server and the REST fake;
//! nothing connects to Google (spec §7.4 IMAP amendment, Testing).

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use mail_domain::{LabelId, MessageId, ThreadId};
use mail_mime::mbox::fixture::FixtureMessage;
use provider_api::fake::FakeProvider;
use provider_api::token::StaticToken;
use provider_api::{BackfillSource, FetchedMessage, ProviderError};
use provider_gmail::imap::{ImapBackfill, ImapConfig, ImapEndpoint};
use provider_gmail::imap_fake::{FakeImapMessage, FakeImapServer};

const MSG_A: u64 = 0x18a1_0000_0000_0001;
const MSG_B: u64 = 0x18a1_0000_0000_0002;
const MSG_BIG: u64 = 0x18a1_0000_0000_0003;
const THREAD: u64 = 0x18a1_0000_0000_0000;

fn hex(n: u64) -> MessageId {
    MessageId(format!("{n:x}"))
}

async fn setup(token_accepted: &str) -> (FakeImapServer, Arc<FakeProvider>, ImapBackfill) {
    let server = FakeImapServer::start("good-token").await;
    let mut a = FixtureMessage::simple(1);
    a.attachment = Some(("notes.txt".into(), "text/plain".into(), b"attached words".to_vec()));
    server.add(FakeImapMessage {
        uid: 10,
        msgid: MSG_A,
        thrid: THREAD,
        labels: vec!["\\Inbox".into(), "\\Important".into(), "Clients/Acme".into(), "Unknown label".into()],
        flags: vec![],
        raw: a.to_rfc822(),
    });
    server.add(FakeImapMessage {
        uid: 11,
        msgid: MSG_B,
        thrid: THREAD,
        labels: vec!["\\Sent".into()],
        flags: vec!["\\Seen".into(), "\\Flagged".into()],
        raw: FixtureMessage::simple(2).to_rfc822(),
    });
    let mut big = FixtureMessage::simple(3);
    big.attachment = Some(("big.bin".into(), "application/octet-stream".into(), vec![7u8; 3 * 1024 * 1024]));
    server.add(FakeImapMessage {
        uid: 12,
        msgid: MSG_BIG,
        thrid: THREAD,
        labels: vec![],
        flags: vec![],
        raw: big.to_rfc822(),
    });

    // The REST fake holds what IMAP should not serve.
    let rest = Arc::new(FakeProvider::new("me@example.com", 1_790_000_000_000, 50));
    for id in [MSG_BIG, 0x99] {
        rest.seed(FetchedMessage {
            id: hex(id),
            thread_id: ThreadId(format!("{THREAD:x}")),
            subject: "from REST".into(),
            ..Default::default()
        });
    }
    let labels = Arc::new(RwLock::new(HashMap::from([("Clients/Acme".to_owned(), LabelId::new("Label_7"))])));
    let config = ImapConfig {
        email: "me@example.com".into(),
        endpoint: ImapEndpoint::Plain(server.addr),
        max_message_bytes: 2 * 1024 * 1024,
        daily_budget_bytes: 1 << 30,
        batch: 200,
    };
    let source = ImapBackfill::new(config, Arc::new(StaticToken(token_accepted.into())), rest.clone(), labels);
    (server, rest, source)
}

#[tokio::test]
async fn bodies_come_over_imap_with_api_ids_labels_and_flags() {
    let (server, _rest, source) = setup("good-token").await;
    let mut fetched = source.fetch(&[hex(MSG_A), hex(MSG_B)]).await.unwrap();
    fetched.sort_by(|a, b| a.id.0.cmp(&b.id.0));
    assert_eq!(fetched.len(), 2);
    let a = &fetched[0];
    assert_eq!(a.id, hex(MSG_A));
    assert_eq!(a.thread_id, ThreadId(format!("{THREAD:x}")), "X-GM-THRID in hex, as the API writes it");
    let labels: Vec<&str> = a.label_ids.iter().map(|l| l.as_str()).collect();
    assert_eq!(labels, ["IMPORTANT", "INBOX", "Label_7", "UNREAD"], "unknown names are skipped, unseen is unread");
    assert_eq!(a.subject, "Subject 1");
    let body = a.body.as_ref().unwrap();
    assert!(body.text.as_deref().unwrap().contains("Body of message 1"));
    assert_eq!(body.attachments.len(), 1);
    assert_eq!(body.attachments[0].data.as_deref(), Some(&b"attached words"[..]), "bytes come with the message");
    assert_eq!(a.internal_date, 1_756_720_800_000, "INTERNALDATE, as the API's internalDate");
    let b = &fetched[1];
    let labels: Vec<&str> = b.label_ids.iter().map(|l| l.as_str()).collect();
    assert_eq!(labels, ["SENT", "STARRED"], "seen and flagged");
    assert_eq!(server.body_fetches(), 2);
    assert_eq!(source.name(), "imap");
    assert!(source.bytes_today().await > 0);

    // The session is kept: a second batch does not log in again.
    source.fetch(&[hex(MSG_A)]).await.unwrap();
    assert_eq!(server.logins(), 1);
}

#[tokio::test]
async fn big_and_unknown_messages_go_over_the_api() {
    let (server, rest, source) = setup("good-token").await;
    let fetched = source.fetch(&[hex(MSG_A), hex(MSG_BIG), hex(0x99)]).await.unwrap();
    let mut subjects: Vec<&str> = fetched.iter().map(|m| m.subject.as_str()).collect();
    subjects.sort();
    assert_eq!(subjects, ["Subject 1", "from REST", "from REST"]);
    assert_eq!(server.body_fetches(), 1, "only the small, known message over IMAP");
    assert_eq!(rest.fetch_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_refused_login_means_the_api_from_then_on() {
    // The source says so; the engine's transport serves the batch over the
    // API (docs/plans/imap-first-sync.md) and records why.
    let (server, rest, source) = setup("wrong-token").await;
    assert!(source.fetch(&[hex(MSG_BIG)]).await.is_err());
    assert!(source.is_refused());
    assert_eq!(source.name(), "imap-refused");
    let again = source.fetch(&[hex(MSG_BIG)]).await.unwrap_err();
    assert!(matches!(again, ProviderError::Unavailable(_)), "refused is unavailable, not a failure: {again}");
    assert_eq!(server.logins(), 0);
    assert_eq!(rest.fetch_calls.load(std::sync::atomic::Ordering::SeqCst), 0, "no second login attempt, no API");
}

#[tokio::test]
async fn the_daily_budget_hands_over_to_the_api() {
    let (server, _rest, source) = {
        let (server, rest, _) = setup("good-token").await;
        let labels = Arc::new(RwLock::new(HashMap::new()));
        let config = ImapConfig {
            email: "me@example.com".into(),
            endpoint: ImapEndpoint::Plain(server.addr),
            max_message_bytes: 2 * 1024 * 1024,
            daily_budget_bytes: 10, // spent by the first message
            batch: 200,
        };
        let source = ImapBackfill::new(config, Arc::new(StaticToken("good-token".into())), rest.clone(), labels);
        (server, rest, source)
    };
    source.fetch(&[hex(MSG_A)]).await.unwrap();
    assert_eq!(server.body_fetches(), 1);
    let again = source.fetch(&[hex(MSG_BIG)]).await.unwrap_err();
    assert!(matches!(again, ProviderError::Unavailable(_)), "over budget: the API's turn");
    assert_eq!(server.body_fetches(), 1, "over budget: no more IMAP today");
}

#[tokio::test]
async fn headers_come_without_bodies_for_a_browsable_list() {
    let (server, _rest, source) = setup("good-token").await;
    let headers = source.fetch_headers(&[hex(MSG_A), hex(MSG_B), hex(0x99)]).await.unwrap().expect("IMAP can");
    assert_eq!(headers.len(), 2, "only what All Mail has");
    assert!(headers.iter().all(|m| m.body.is_none()));
    let a = headers.iter().find(|m| m.id == hex(MSG_A)).unwrap();
    assert_eq!(a.subject, "Subject 1");
    assert_eq!(a.thread_id, ThreadId(format!("{THREAD:x}")));
    assert!(a.label_ids.iter().any(|l| l.as_str() == "INBOX"));
    assert_eq!(server.header_fetches(), 2);
    assert_eq!(server.body_fetches(), 0);

    let (_s2, _r2, refused) = setup("wrong-token").await;
    assert!(refused.fetch_headers(&[hex(MSG_A)]).await.is_err(), "no cheap headers without IMAP");
}

fn crlf(s: &str) -> Vec<u8> {
    s.replace("\r\n", "\n").replace('\n', "\r\n").into_bytes()
}

fn headed(content_headers: &str, body: &str) -> Vec<u8> {
    crlf(&format!(
        "From: Sam <sam@example.com>\nTo: me@example.com\nSubject: Snippet\nMessage-ID: <s@example.com>\n\
         Date: Mon, 01 Sep 2025 10:00:00 +0000\nMIME-Version: 1.0\n{content_headers}\n{body}"
    ))
}

#[tokio::test]
async fn headers_carry_a_snippet_from_the_first_bytes_of_the_text() {
    let (server, _rest, source) = setup("good-token").await;
    let long_qp = "Caf=C3=A9 au lait =E2=80=94 ".repeat(120);
    let long_b64 = {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode("Base64 body words. ".repeat(200));
        encoded.as_bytes().chunks(76).map(|c| String::from_utf8_lossy(c).into_owned() + "\n").collect::<String>()
    };
    let cases: Vec<(u64, Vec<u8>, &str)> = vec![
        (
            0x2001,
            headed("Content-Type: text/plain; charset=utf-8\n", "Hello there,\nplain   text.\n"),
            "Hello there, plain text.",
        ),
        (
            0x2002,
            headed("Content-Type: text/plain; charset=utf-8\nContent-Transfer-Encoding: quoted-printable\n", &long_qp),
            "Café au lait — Café au lait —",
        ),
        (
            0x2003,
            headed("Content-Type: text/plain; charset=utf-8\nContent-Transfer-Encoding: base64\n", &long_b64),
            "Base64 body words. Base64 body words.",
        ),
        (
            0x2004,
            headed(
                "Content-Type: text/html; charset=utf-8\n",
                "<html><body><p>Hello <b>HTML</b> world</p></body></html>\n",
            ),
            "Hello HTML world",
        ),
        (
            0x2005,
            headed(
                "Content-Type: multipart/alternative; boundary=\"b\"\n",
                &format!(
                    "--b\nContent-Type: text/plain; charset=utf-8\n\nThe plain part first.\n--b\n\
                     Content-Type: text/html; charset=utf-8\n\n<p>{}</p>\n--b--\n",
                    "html ".repeat(1000)
                ),
            ),
            "The plain part first.",
        ),
    ];
    for (i, (msgid, raw, _)) in cases.iter().enumerate() {
        server.add(FakeImapMessage {
            uid: 100 + i as u32,
            msgid: *msgid,
            thrid: *msgid,
            labels: vec![],
            flags: vec!["\\Seen".into()],
            raw: raw.clone(),
        });
    }
    let ids: Vec<MessageId> = cases.iter().map(|(m, _, _)| hex(*m)).collect();
    let headers = source.fetch_headers(&ids).await.unwrap().expect("IMAP can");
    assert_eq!(headers.len(), cases.len());
    for (msgid, raw, expected) in &cases {
        let m = headers.iter().find(|m| m.id == hex(*msgid)).unwrap();
        assert!(m.body.is_none());
        assert!(m.snippet.starts_with(expected), "{msgid:x}: {:?} should start {expected:?}", m.snippet);
        assert!(m.snippet.chars().count() <= 160);
        assert_eq!(m.size_estimate, raw.len() as u64, "the whole message's size, not the bytes fetched");
    }
    assert_eq!(server.body_fetches(), 0);
    assert!(source.bytes_today().await < 5 * 2048 + 5 * 400, "headers and 2 KB of text each");
}

#[tokio::test]
async fn a_message_that_left_all_mail_since_the_map_loaded_comes_over_the_api() {
    let (server, rest, source) = setup("good-token").await;
    source.fetch(&[hex(MSG_A)]).await.unwrap(); // loads the map, including B
    rest.seed(FetchedMessage {
        id: hex(MSG_B),
        thread_id: ThreadId(format!("{THREAD:x}")),
        subject: "B from REST".into(),
        ..Default::default()
    });
    server.remove(11); // B moved to Trash
    let fetched = source.fetch(&[hex(MSG_B)]).await.unwrap();
    assert_eq!(fetched.len(), 1, "never silently dropped");
    assert_eq!(fetched[0].subject, "B from REST");
}

#[tokio::test]
async fn gmail_searches_list_ids_over_imap_without_the_api() {
    let (server, rest, source) = setup("good-token").await;
    let ids = |q: &'static str| {
        let source = &source;
        async move {
            let mut out: Vec<String> =
                source.list(q).await.unwrap().expect("IMAP can list").into_iter().map(|m| m.0).collect();
            out.sort();
            out
        }
    };
    assert_eq!(ids("in:inbox").await, [hex(MSG_A).0], "the Inbox");
    assert_eq!(ids("in:inbox is:unread").await, [hex(MSG_A).0], "unread in the Inbox");
    assert_eq!(ids("is:starred").await, [hex(MSG_B).0]);
    let mut all = vec![hex(MSG_A).0, hex(MSG_B).0, hex(MSG_BIG).0];
    all.sort();
    assert_eq!(ids("").await, all, "everything in All Mail");
    assert_eq!(server.searches(), 4);
    assert_eq!(rest.fetch_calls.load(std::sync::atomic::Ordering::SeqCst), 0, "no API calls");

    let (_s2, _r2, refused) = setup("wrong-token").await;
    assert!(refused.list("in:inbox").await.is_err(), "refused: the engine lists over the API");
}
