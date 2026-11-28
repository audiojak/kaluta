//! Search over a small hand-built mailbox with known answers.

use mail_domain::{Body, EmailAddress, Label, LabelId, LabelKind, MessageId, ThreadId};
use mail_store::search::{parse, search};
use mail_store::{Db, IncomingAttachment, IncomingMessage, MailWriter};

const NOW: i64 = 1_790_000_000_000;
const DAY: i64 = 86_400_000;

struct M<'a> {
    id: &'a str,
    thread: &'a str,
    from: (&'a str, &'a str),
    to: &'a str,
    cc: Option<&'a str>,
    subject: &'a str,
    body: &'a str,
    labels: &'a [&'a str],
    age_days: i64,
    attachment: Option<&'a str>,
    size: u64,
}

fn open() -> Db {
    let dir = std::env::temp_dir().join(format!("kaluta-search-{}-{}", std::process::id(), fastid()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = Db::open(&dir.join("mail.sqlite")).unwrap();
    let messages = vec![
        M {
            id: "1",
            thread: "invoice",
            from: ("Billing", "billing@example.net"),
            to: "me@example.com",
            cc: None,
            subject: "Invoice #4821 for September",
            body: "Your AWS invoice is ready",
            labels: &["INBOX", "UNREAD", "Label_1"],
            age_days: 2,
            attachment: Some("invoice-4821.pdf"),
            size: 90_000,
        },
        M {
            id: "2",
            thread: "plan",
            from: ("Alex Rivera", "alex.rivera@example.com"),
            to: "me@example.com",
            cc: Some("sam.chen@example.org"),
            subject: "Q3 planning",
            body: "Let's review the quarterly roadmap on Thursday",
            labels: &["INBOX"],
            age_days: 10,
            attachment: None,
            size: 4_000,
        },
        M {
            id: "3",
            thread: "plan",
            from: ("Me", "me@example.com"),
            to: "alex.rivera@example.com",
            cc: None,
            subject: "Re: Q3 planning",
            body: "Thursday works for me",
            labels: &["SENT"],
            age_days: 9,
            attachment: None,
            size: 2_000,
        },
        M {
            id: "4",
            thread: "old",
            from: ("Jordan Park", "jordan.park@example.net"),
            to: "me@example.com",
            cc: None,
            subject: "Café recommendations",
            body: "Try the crème brûlée",
            labels: &["STARRED"],
            age_days: 400,
            attachment: None,
            size: 3_000,
        },
        M {
            id: "5",
            thread: "spam",
            from: ("Winner", "prize@example.net"),
            to: "me@example.com",
            cc: None,
            subject: "You won an invoice prize",
            body: "invoice invoice",
            labels: &["SPAM"],
            age_days: 1,
            attachment: None,
            size: 1_000,
        },
    ];
    db.write_blocking(move |tx| {
        let mut w = MailWriter::new(tx);
        w.upsert_labels(&[Label {
            id: LabelId::new("Label_1"),
            name: "Big Customers".into(),
            kind: LabelKind::User,
            color: None,
            visible: true,
        }])?;
        for m in &messages {
            w.upsert_message(&IncomingMessage {
                id: MessageId::new(m.id),
                thread_id: ThreadId::new(m.thread),
                from: Some(EmailAddress::new(Some(m.from.0), m.from.1)),
                to: vec![EmailAddress::new(None, m.to)],
                cc: m.cc.map(|c| vec![EmailAddress::new(None, c)]).unwrap_or_default(),
                subject: m.subject.into(),
                snippet: m.body.chars().take(40).collect(),
                date: NOW - m.age_days * DAY,
                internal_date: NOW - m.age_days * DAY,
                label_ids: m.labels.iter().map(|l| LabelId::new(*l)).collect(),
                size_estimate: m.size,
                body: Some(Body { text_plain: Some(m.body.into()), ..Default::default() }),
                attachments: m
                    .attachment
                    .map(|f| {
                        vec![IncomingAttachment {
                            filename: f.into(),
                            mime_type: "application/pdf".into(),
                            ..Default::default()
                        }]
                    })
                    .unwrap_or_default(),
                ..Default::default()
            })?;
        }
        w.finish()?;
        Ok(())
    })
    .unwrap();
    db
}

/// Unique per call; tests run in parallel and the clock is too coarse.
fn fastid() -> usize {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

fn q(db: &Db, query: &str) -> Vec<String> {
    let expr = parse(query).unwrap_or_else(|e| panic!("{query}: {e}"));
    db.read_blocking(|c| search(c, &expr, NOW, None, 50)).unwrap().rows.into_iter().map(|t| t.id.0).collect()
}

#[test]
fn free_text_matches_subject_body_and_addresses_with_prefixes() {
    let db = open();
    assert_eq!(q(&db, "invoice"), vec!["invoice"], "spam excluded by default");
    assert_eq!(q(&db, "quarter"), vec!["plan"], "prefix match on the last token");
    assert_eq!(q(&db, "rivera"), vec!["plan"]);
    assert_eq!(q(&db, "cafe"), vec!["old"], "diacritics folded");
    assert_eq!(q(&db, "thursday roadmap"), vec!["plan"], "words are ANDed within a thread's messages");
    assert!(q(&db, "nothing-matches-this").is_empty());
}

#[test]
fn phrases_and_fields() {
    let db = open();
    assert_eq!(q(&db, r#""review the quarterly""#), vec!["plan"]);
    assert!(q(&db, r#""quarterly the review""#).is_empty(), "phrases keep order");
    assert_eq!(q(&db, "from:alex"), vec!["plan"]);
    assert_eq!(q(&db, "from:alex.rivera@example.com"), vec!["plan"]);
    assert_eq!(q(&db, "to:alex"), vec!["plan"], "my reply was to Alex");
    assert_eq!(q(&db, "cc:sam"), vec!["plan"]);
    assert_eq!(q(&db, "subject:planning"), vec!["plan"]);
    assert_eq!(q(&db, "filename:pdf"), vec!["invoice"]);
    assert_eq!(q(&db, "label:Big-Customers"), vec!["invoice"]);
    assert_eq!(q(&db, r#"label:"big customers""#), vec!["invoice"]);
}

#[test]
fn flags_mailboxes_dates_and_sizes() {
    let db = open();
    assert_eq!(q(&db, "is:unread"), vec!["invoice"]);
    assert_eq!(q(&db, "is:starred"), vec!["old"]);
    assert_eq!(q(&db, "has:attachment"), vec!["invoice"]);
    assert_eq!(q(&db, "in:sent"), vec!["plan"]);
    assert_eq!(q(&db, "in:archive"), vec!["plan", "old"], "threads with any non-inbox message; spam excluded");
    assert_eq!(q(&db, "in:spam"), vec!["spam"]);
    assert_eq!(q(&db, "newer_than:7d"), vec!["invoice"]);
    assert_eq!(q(&db, "older_than:1y"), vec!["old"]);
    assert_eq!(q(&db, "larger:50K"), vec!["invoice"]);
    assert_eq!(q(&db, "smaller:2500 in:sent"), vec!["plan"]);
}

#[test]
fn boolean_structure() {
    let db = open();
    assert_eq!(q(&db, "invoice OR cafe"), vec!["invoice", "old"]);
    assert_eq!(q(&db, "-in:inbox"), vec!["plan", "old"], "plan has a sent message outside the inbox");
    assert_eq!(q(&db, "in:inbox -from:billing"), vec!["plan"]);
    assert_eq!(q(&db, "(is:unread OR is:starred) -cafe"), vec!["invoice"]);
    assert_eq!(q(&db, "in:anywhere invoice"), vec!["spam", "invoice"], "anywhere includes spam, newest first");
}

#[test]
fn hostile_input_is_just_text() {
    let db = open();
    for evil in [r#"x" OR 1=1 --"#, "NEAR(a b)", "subject:\"a\" OR", "*", "' ; DROP TABLE messages; --"] {
        if let Ok(expr) = parse(evil) {
            db.read_blocking(|c| search(c, &expr, NOW, None, 50)).unwrap();
        }
    }
    assert_eq!(q(&db, "invoice"), vec!["invoice"], "store intact");
}

#[test]
fn results_page_with_a_cursor() {
    let db = open();
    let expr = parse("in:anywhere").unwrap();
    let first = db.read_blocking(|c| search(c, &expr, NOW, None, 2)).unwrap();
    assert_eq!(first.rows.len(), 2);
    let cursor = first.next_cursor.clone().unwrap();
    let second = db.read_blocking(|c| search(c, &expr, NOW, Some(&cursor), 2)).unwrap();
    let all: Vec<_> = first.rows.iter().chain(&second.rows).map(|t| t.id.0.clone()).collect();
    assert_eq!(all, vec!["spam", "invoice", "plan", "old"]);
}

#[test]
fn tag_finds_mail_with_a_person_who_has_that_contact_tag() {
    use mail_store::contact_facts::{self as contacts, ContactFactRow};
    let db = open();
    db.write_blocking(|tx| {
        let tag = |tx: &mail_store::Transaction<'_>, email: &str, tag_id: &str, status: &str| {
            let person_id = contacts::ensure_person(tx, email, NOW)?;
            contacts::insert(
                tx,
                &ContactFactRow {
                    person_id,
                    kind: "tag".into(),
                    value: tag_id.into(),
                    class: "other".into(),
                    use_: "free".into(),
                    source: "you".into(),
                    risk: "low".into(),
                    status: status.into(),
                    ..Default::default()
                },
            )
        };
        tag(tx, "alex.rivera@example.com", "rel.customer", "accepted")?;
        tag(tx, "alex.rivera@example.com", "rel.services-client", "accepted")?;
        tag(tx, "alex.rivera@example.com", "role.vp", "accepted")?;
        tag(tx, "billing@example.net", "rel.vendor", "accepted")?;
        tag(tx, "sam.chen@example.org", "role.board-member", "accepted")?;
        // Held for review, and rejected: neither finds anything.
        tag(tx, "jordan.park@example.net", "rel.customer", "proposed")?;
        tag(tx, "jordan.park@example.net", "rel.friend", "rejected")?;
        // The user is tagged by nothing, even if a row said so.
        Ok(())
    })
    .unwrap();
    assert_eq!(q(&db, "tag:customer"), vec!["plan"], "from them, or sent to them");
    assert_eq!(q(&db, "tag:CUSTOMER"), vec!["plan"]);
    assert_eq!(q(&db, r#"tag:"services client""#), vec!["plan"]);
    assert_eq!(q(&db, "tag:services-client"), vec!["plan"]);
    assert_eq!(q(&db, "tag:rel.customer"), vec!["plan"], "a tag's id works too");
    assert_eq!(q(&db, "tag:vendor"), vec!["invoice"]);
    assert_eq!(q(&db, "tag:board-member"), vec!["plan"], "a cc'd contact; a name in both sets matches either");
    assert!(q(&db, "tag:friend").is_empty(), "a rejected tag");
    assert!(q(&db, "tag:wizard").is_empty(), "an unknown name finds nothing");
    assert_eq!(q(&db, "tag:customer OR tag:vendor"), vec!["invoice", "plan"]);
    assert_eq!(q(&db, "in:inbox -tag:vendor"), vec!["plan"]);
    assert_eq!(q(&db, "tag:vendor has:attachment newer_than:30d"), vec!["invoice"]);
    assert!(q(&db, "tag:customer has:attachment").is_empty());
    assert_eq!(q(&db, "tag:customer in:sent"), vec!["plan"]);
    // Jordan's held tag found nothing; accepted, it does.
    assert!(!q(&db, "tag:customer").contains(&"old".to_string()));
    db.write_blocking(|tx| {
        tx.execute("UPDATE contact_facts SET status = 'accepted' WHERE status = 'proposed'", [])?;
        Ok(())
    })
    .unwrap();
    assert_eq!(q(&db, "tag:customer"), vec!["plan", "old"]);
    // The names a query asks for, for the "No tag named …" line.
    let asked = mail_store::search::tag_names(&parse("invoice (tag:customer OR -tag:wizard) from:alex").unwrap());
    assert_eq!(asked, vec!["customer".to_string(), "wizard".to_string()]);
}
