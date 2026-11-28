//! The contact page's queries (spec §14.16): waits, tasks and recent
//! conversations found from Waiting's and Tasks' own tables.

use std::path::PathBuf;

use mail_domain::{EmailAddress, LabelId, MessageId, ThreadId};
use mail_store::contact_page::{mail_stats, recent_conversations, tasks_from, waiting_on};
use mail_store::tasks::TaskFields;
use mail_store::{Db, IncomingMessage, MailWriter, tasks, waiting};

const ME: &str = "me@example.com";
const REESE: &str = "reese@acme.example";
const REESE_HOME: &str = "Reese@Home.example";
const ADA: &str = "ada@other.example";

fn open(name: &str) -> Db {
    let dir: PathBuf = std::env::temp_dir().join(format!("kaluta-contact-page-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Db::open(&dir.join("mail.sqlite")).unwrap()
}

fn mail(id: &str, thread: &str, at: i64, from: &str, to: &[&str], cc: &[&str]) -> IncomingMessage {
    IncomingMessage {
        id: MessageId::new(id),
        thread_id: ThreadId::new(thread),
        from: Some(EmailAddress::new(None, from)),
        to: to.iter().map(|t| EmailAddress::new(None, t)).collect(),
        cc: cc.iter().map(|t| EmailAddress::new(None, t)).collect(),
        subject: format!("About {thread}"),
        date: at,
        internal_date: at,
        label_ids: vec![LabelId::new(if from == ME { "SENT" } else { "INBOX" })],
        ..Default::default()
    }
}

fn seeded(name: &str) -> Db {
    let db = open(name);
    db.write_blocking(|tx| {
        let mut w = MailWriter::new(tx);
        for m in [
            mail("a1", "pricing", 100, REESE, &[ME], &[]),
            mail("a2", "pricing", 200, ME, &[REESE], &[]),
            mail("b1", "contract", 300, ME, &[ADA], &[REESE_HOME]),
            mail("c1", "offsite", 400, ADA, &[ME], &[]),
            mail("c2", "offsite", 500, ME, &[ADA], &[]),
            mail("d1", "intro", 600, REESE_HOME, &[ME, ADA], &[]),
            mail("e1", "audit", 700, ADA, &[ME], &[]),
            mail("e2", "audit", 800, REESE, &[ME], &[]),
        ] {
            w.upsert_message(&m)?;
        }
        w.finish()?;
        waiting::start(tx, "pricing", "a2", 200, false, 900)?;
        waiting::start(tx, "contract", "b1", 300, false, 900)?;
        waiting::start(tx, "offsite", "c2", 500, false, 900)?;
        tx.execute("UPDATE waiting SET state = 'answered' WHERE thread_id = 'contract'", [])?;
        let task = |title: &str| TaskFields {
            title: title.into(),
            category: "Reply".into(),
            action: "none".into(),
            ..Default::default()
        };
        // On Reese's own message; on Ada's message in a thread Reese also wrote in;
        // with no message, in a conversation Reese wrote in; one only linked to it.
        tasks::insert(tx, "pricing", Some("a1"), &task("Answer the pricing question"), "you", "", 1_000)?;
        tasks::insert(tx, "audit", Some("e1"), &task("Send Ada the audit files"), "you", "", 1_001)?;
        tasks::insert(tx, "intro", None, &task("Follow up on the intro"), "ai", "", 1_002)?;
        let linked = tasks::insert(tx, "offsite", None, &task("Book the offsite"), "you", "", 1_003)?;
        tx.execute("INSERT INTO task_threads (task_id, thread_id, added_at) VALUES (?1, 'audit', 1)", [linked])?;
        let done = tasks::insert(tx, "audit", Some("e2"), &task("Reply about the audit"), "you", "", 1_004)?;
        tx.execute("UPDATE tasks SET status = 'done', completed_at = 2000 WHERE id = ?1", [done])?;
        Ok(())
    })
    .unwrap();
    db
}

#[test]
fn waits_are_those_whose_sent_message_was_to_them() {
    let db = seeded("waits");
    let reese = vec![REESE.to_string(), REESE_HOME.to_string()];
    let waits = db.read_blocking(move |c| waiting_on(c, &reese)).unwrap();
    assert_eq!(
        waits.iter().map(|w| w.thread_id.as_str()).collect::<Vec<_>>(),
        vec!["pricing"],
        "answered waits are not shown"
    );
    assert_eq!((waits[0].message_id.as_str(), waits[0].since, waits[0].subject.as_str()), ("a2", 200, "About pricing"));
    let ada = db.read_blocking(|c| waiting_on(c, &[ADA.to_string()])).unwrap();
    assert_eq!(ada.iter().map(|w| w.thread_id.as_str()).collect::<Vec<_>>(), vec!["offsite"]);
    assert!(db.read_blocking(|c| waiting_on(c, &[])).unwrap().is_empty());
}

#[test]
fn tasks_are_those_from_their_mail_open_first() {
    let db = seeded("tasks");
    let reese = vec![REESE.to_string(), REESE_HOME.to_string()];
    let found = db.read_blocking(move |c| tasks_from(c, &reese)).unwrap();
    let titles: Vec<(&str, bool)> = found.iter().map(|t| (t.title.as_str(), t.done)).collect();
    assert_eq!(
        titles,
        vec![
            ("Book the offsite", false),
            ("Follow up on the intro", false),
            ("Answer the pricing question", false),
            ("Reply about the audit", true),
        ],
        "a task on someone else's message is theirs, not this contact's"
    );
    let ada = db.read_blocking(|c| tasks_from(c, &[ADA.to_string()])).unwrap();
    assert_eq!(
        ada.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
        vec!["Book the offsite", "Send Ada the audit files"]
    );
}

#[test]
fn recent_conversations_and_counts_cover_every_address() {
    let db = seeded("recent");
    let reese = vec![REESE.to_string(), REESE_HOME.to_string()];
    let (recent, stats) =
        db.read_blocking(move |c| Ok((recent_conversations(c, &reese, 3)?, mail_stats(c, &reese)?))).unwrap();
    assert_eq!(
        recent.iter().map(|r| (r.thread_id.as_str(), r.latest)).collect::<Vec<_>>(),
        vec![("audit", 800), ("intro", 600), ("contract", 300)]
    );
    assert_eq!((stats.first, stats.latest, stats.sent, stats.received), (Some(100), Some(800), 2, 3));
    let nobody = db.read_blocking(|c| mail_stats(c, &["nobody@example.com".to_string()])).unwrap();
    assert_eq!((nobody.first, nobody.sent, nobody.received), (None, 0, 0));
}
