//! Waiting on others (spec §14.13): the waiting check's running log, the
//! conversations waiting on someone's answer, and how each ended.

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::StoreResult;

/// Days a conversation waits before it joins the workflow, unless the user
/// changed it.
pub const DEFAULT_DAYS: i64 = 3;
/// `workflow_meta` keys.
pub const DAYS_KEY: &str = "waiting_days";
pub const LABEL_KEY: &str = "waiting_label";
/// The local day the check last finished reading everything.
pub const CHECKED_DAY_KEY: &str = "waiting_checked_day";

const DAY_MS: i64 = 86_400_000;

/// Addresses that never answer.
const AUTOMATED: [&str; 6] = ["noreply", "no-reply", "donotreply", "do-not-reply", "notifications", "mailer-daemon"];

/// A sent message the check has not read: the user wrote last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unchecked {
    pub message_id: String,
    pub thread_id: String,
    pub at: Millis,
}

/// The conversations whose latest message is the user's, sent since
/// `since` to at least one person (not a no-reply address), that the check
/// has not read, oldest first.
pub fn unchecked(conn: &Connection, since: Millis, limit: usize) -> StoreResult<Vec<Unchecked>> {
    let mut stmt = conn.prepare_cached(
        "SELECT m.id, m.gmail_id, t.gmail_id, m.internal_date
           FROM threads t
           JOIN messages m ON m.id = (SELECT id FROM messages WHERE thread_id = t.id AND is_draft = 0
                                       ORDER BY internal_date DESC, id DESC LIMIT 1)
          WHERE m.is_sent_by_me = 1 AND m.internal_date >= ?1
            AND NOT EXISTS (SELECT 1 FROM waiting_checks w WHERE w.message_id = m.gmail_id)
          ORDER BY m.internal_date, m.id",
    )?;
    let mut people =
        conn.prepare_cached("SELECT email FROM participants WHERE message_id = ?1 AND role IN ('to', 'cc')")?;
    let mut out = Vec::new();
    let mut rows = stmt.query([since])?;
    while let Some(r) = rows.next()? {
        if out.len() >= limit {
            break;
        }
        let message_id: String = r.get(1)?;
        if message_id.starts_with(crate::LOCAL_PREFIX) {
            continue;
        }
        let local: i64 = r.get(0)?;
        let emails: Vec<String> = people.query_map([local], |p| p.get(0))?.collect::<Result<_, _>>()?;
        let person = emails.iter().any(|e| {
            let e = e.to_lowercase();
            !AUTOMATED.iter().any(|a| e.contains(a))
        });
        if person {
            out.push(Unchecked { message_id, thread_id: r.get(2)?, at: r.get(3)? });
        }
    }
    Ok(out)
}

/// One entry of the running log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub message_id: String,
    pub thread_id: String,
    pub checked_at: Millis,
    pub expects: bool,
    pub reason: String,
    pub agent: Option<String>,
}

pub fn record_check(tx: &Transaction, c: &Check) -> StoreResult<()> {
    tx.execute(
        "INSERT OR REPLACE INTO waiting_checks (message_id, thread_id, checked_at, expects, reason, agent)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![c.message_id, c.thread_id, c.checked_at, c.expects, c.reason, c.agent],
    )?;
    Ok(())
}

/// The log, newest first.
pub fn checks(conn: &Connection, limit: u32) -> StoreResult<Vec<Check>> {
    let mut stmt = conn.prepare_cached(
        "SELECT message_id, thread_id, checked_at, expects, reason, agent FROM waiting_checks
          ORDER BY checked_at DESC, rowid DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |r| {
        Ok(Check {
            message_id: r.get(0)?,
            thread_id: r.get(1)?,
            checked_at: r.get(2)?,
            expects: r.get(3)?,
            reason: r.get(4)?,
            agent: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A conversation waiting (or that waited) on an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingRow {
    pub thread_id: String,
    pub message_id: String,
    pub since: Millis,
    pub until: Option<Millis>,
    /// `waiting`, `answered` or `let_go`.
    pub state: String,
    pub labelled: bool,
    pub nudges_json: String,
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<WaitingRow> {
    Ok(WaitingRow {
        thread_id: r.get(0)?,
        message_id: r.get(1)?,
        since: r.get(2)?,
        until: r.get(3)?,
        state: r.get(4)?,
        labelled: r.get(5)?,
        nudges_json: r.get(6)?,
    })
}

const COLUMNS: &str = "thread_id, message_id, since, until, state, labelled, nudges_json";

/// The conversation waits on `message_id`, sent at `since` (a newer sent
/// message starts the wait again, and clears what was written ahead).
pub fn start(
    tx: &Transaction,
    thread_id: &str,
    message_id: &str,
    since: Millis,
    labelled: bool,
    now: Millis,
) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO waiting (thread_id, message_id, since, until, state, labelled, nudges_json, updated_at)
         VALUES (?1, ?2, ?3, NULL, 'waiting', ?4, '[]', ?5)
         ON CONFLICT (thread_id) DO UPDATE SET message_id = excluded.message_id, since = excluded.since,
           until = NULL, state = 'waiting', labelled = labelled OR excluded.labelled, nudges_json = '[]',
           updated_at = excluded.updated_at",
        params![thread_id, message_id, since, labelled, now],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, thread_id: &str) -> StoreResult<Option<WaitingRow>> {
    Ok(conn.query_row(&format!("SELECT {COLUMNS} FROM waiting WHERE thread_id = ?1"), [thread_id], row).optional()?)
}

/// The workflow's set: waiting at least `days`, not hidden until later,
/// and still unanswered (the user's message is still the latest), the
/// longest first.
pub fn due(conn: &Connection, now: Millis, days: i64) -> StoreResult<Vec<WaitingRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {} FROM waiting w
           JOIN threads t ON t.gmail_id = w.thread_id
           JOIN messages m ON m.id = (SELECT id FROM messages WHERE thread_id = t.id AND is_draft = 0
                                       ORDER BY internal_date DESC, id DESC LIMIT 1)
          WHERE w.state = 'waiting' AND w.since <= ?1 AND (w.until IS NULL OR w.until <= ?2)
            AND m.is_sent_by_me = 1
          ORDER BY w.since, w.thread_id",
        COLUMNS.split(", ").map(|c| format!("w.{c}")).collect::<Vec<_>>().join(", ")
    ))?;
    let rows = stmt.query_map(params![now - days * DAY_MS, now], row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Waiting conversations someone has answered: a message from someone
/// else since the one waited on.
pub fn answered(conn: &Connection) -> StoreResult<Vec<WaitingRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {} FROM waiting w
           JOIN threads t ON t.gmail_id = w.thread_id
          WHERE w.state = 'waiting'
            AND EXISTS (SELECT 1 FROM messages m WHERE m.thread_id = t.id AND m.is_draft = 0
                          AND m.is_sent_by_me = 0 AND m.internal_date > w.since)",
        COLUMNS.split(", ").map(|c| format!("w.{c}")).collect::<Vec<_>>().join(", ")
    ))?;
    let rows = stmt.query_map([], row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// `answered` or `let_go`; ends the wait.
pub fn end(tx: &Transaction, thread_id: &str, state: &str, now: Millis) -> StoreResult<()> {
    tx.execute("UPDATE waiting SET state = ?2, updated_at = ?3 WHERE thread_id = ?1", params![thread_id, state, now])?;
    Ok(())
}

/// Undo of `end`: waiting again, as it was.
pub fn reopen(tx: &Transaction, thread_id: &str, now: Millis) -> StoreResult<()> {
    tx.execute("UPDATE waiting SET state = 'waiting', updated_at = ?2 WHERE thread_id = ?1", params![thread_id, now])?;
    Ok(())
}

/// Check again: hidden until `until` (`None` shows it again).
pub fn set_until(tx: &Transaction, thread_id: &str, until: Option<Millis>, now: Millis) -> StoreResult<()> {
    tx.execute("UPDATE waiting SET until = ?2, updated_at = ?3 WHERE thread_id = ?1", params![thread_id, until, now])?;
    Ok(())
}

pub fn set_nudges(tx: &Transaction, thread_id: &str, json: &str, now: Millis) -> StoreResult<()> {
    tx.execute(
        "UPDATE waiting SET nudges_json = ?2, updated_at = ?3 WHERE thread_id = ?1",
        params![thread_id, json, now],
    )?;
    Ok(())
}

/// Waiting conversations with no follow-ups written yet, the longest first.
pub fn needing_nudges(conn: &Connection, limit: u32) -> StoreResult<Vec<WaitingRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM waiting WHERE state = 'waiting' AND nudges_json = '[]' ORDER BY since LIMIT ?1"
    ))?;
    let rows = stmt.query_map([limit], row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Who a sent message waits on: its first recipient's name, else address.
pub fn who(conn: &Connection, message_id: &str) -> StoreResult<Option<String>> {
    Ok(conn
        .prepare_cached(
            "SELECT COALESCE(NULLIF(p.name, ''), p.email) FROM participants p JOIN messages m ON m.id = p.message_id
              WHERE m.gmail_id = ?1 AND p.role = 'to' ORDER BY p.position LIMIT 1",
        )?
        .query_row([message_id], |r| r.get(0))
        .optional()?)
}

/// Whether the conversation carries the label (by its Gmail id).
pub fn has_label(conn: &Connection, thread_id: &str, label_id: &str) -> StoreResult<bool> {
    Ok(conn
        .prepare_cached(
            "SELECT 1 FROM thread_labels tl JOIN threads t ON t.id = tl.thread_id JOIN labels l ON l.id = tl.label_id
              WHERE t.gmail_id = ?1 AND l.gmail_id = ?2",
        )?
        .exists(params![thread_id, label_id])?)
}

/// How many days a conversation waits before it joins the workflow.
pub fn days(conn: &Connection) -> StoreResult<i64> {
    Ok(crate::workflows::meta(conn, DAYS_KEY)?.and_then(|v| v.parse().ok()).filter(|d| *d >= 1).unwrap_or(DEFAULT_DAYS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    const NOW: Millis = 1_790_000_000_000;

    struct Store(std::path::PathBuf, Db);

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("kaluta-waiting-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("mail.sqlite")).unwrap();
        Store(dir, db)
    }

    #[test]
    fn a_checked_message_is_never_read_again_and_a_newer_one_is_read_alone() {
        let Store(_, db) = &store("log");
        crate::demo_workflows::generate(db, NOW).unwrap();
        let first = db.read_blocking(|c| unchecked(c, NOW - 14 * DAY_MS, 50)).unwrap();
        let threads: Vec<&str> = first.iter().map(|u| u.thread_id.as_str()).collect();
        assert!(threads.contains(&"demowf-quote") && threads.contains(&"demowf-ip"), "{threads:?}");
        assert!(!threads.contains(&"demowf-renewal"), "someone else wrote last");
        let all = first.clone();
        db.write_blocking(move |tx| {
            for u in &all {
                record_check(
                    tx,
                    &Check {
                        message_id: u.message_id.clone(),
                        thread_id: u.thread_id.clone(),
                        checked_at: NOW,
                        expects: true,
                        reason: "Asks for a quote.".into(),
                        agent: None,
                    },
                )?;
            }
            Ok(())
        })
        .unwrap();
        assert!(db.read_blocking(|c| unchecked(c, NOW - 14 * DAY_MS, 50)).unwrap().is_empty(), "never twice");

        // The user writes again in one conversation: that message alone.
        db.write_blocking(|tx| {
            tx.execute(
                "INSERT INTO messages (thread_id, gmail_id, from_email, subject, date, internal_date, is_sent_by_me)
                 SELECT id, 'again', 'me@example.com', 'Re', ?1, ?1, 1 FROM threads WHERE gmail_id = 'demowf-quote'",
                [NOW],
            )?;
            tx.execute(
                "INSERT INTO participants (message_id, role, position, email)
                 SELECT id, 'to', 0, 'hayden@example.com' FROM messages WHERE gmail_id = 'again'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let again = db.read_blocking(|c| unchecked(c, NOW - 14 * DAY_MS, 50)).unwrap();
        assert_eq!(again.iter().map(|u| u.message_id.as_str()).collect::<Vec<_>>(), ["again"]);
        assert_eq!(db.read_blocking(|c| checks(c, 100)).unwrap().len(), first.len());
    }

    #[test]
    fn a_wait_joins_after_its_days_and_an_answer_ends_it() {
        let Store(_, db) = &store("due");
        crate::demo_workflows::generate(db, NOW).unwrap();
        let sent = db.read_blocking(|c| unchecked(c, NOW - 14 * DAY_MS, 50)).unwrap();
        let quote = sent.iter().find(|u| u.thread_id == "demowf-quote").unwrap().clone();
        let refs = sent.iter().find(|u| u.thread_id == "demowf-references").unwrap().clone();
        let (q, r) = (quote.clone(), refs.clone());
        db.write_blocking(move |tx| {
            start(tx, &q.thread_id, &q.message_id, q.at, true, NOW)?;
            start(tx, &r.thread_id, &r.message_id, r.at, true, NOW)
        })
        .unwrap();
        assert_eq!(db.read_blocking(days).unwrap(), DEFAULT_DAYS);
        let ids = |rows: Vec<WaitingRow>| rows.into_iter().map(|w| w.thread_id).collect::<Vec<_>>();
        assert_eq!(
            ids(db.read_blocking(|c| due(c, NOW, 3)).unwrap()),
            ["demowf-quote", "demowf-references"],
            "the longest first"
        );
        assert_eq!(ids(db.read_blocking(|c| due(c, NOW, 5)).unwrap()), ["demowf-quote"], "not yet five days");

        // Check again: hidden until the day chosen.
        db.write_blocking(|tx| set_until(tx, "demowf-quote", Some(NOW + DAY_MS), NOW)).unwrap();
        assert_eq!(ids(db.read_blocking(|c| due(c, NOW, 3)).unwrap()), ["demowf-references"]);
        assert_eq!(ids(db.read_blocking(|c| due(c, NOW + DAY_MS, 3)).unwrap()).len(), 2);

        // Hayden answers.
        db.write_blocking(|tx| {
            tx.execute(
                "INSERT INTO messages (thread_id, gmail_id, from_email, subject, date, internal_date)
                 SELECT id, 'answer', 'hayden@example.com', 'Re', ?1, ?1 FROM threads WHERE gmail_id = 'demowf-quote'",
                [NOW],
            )?;
            Ok(())
        })
        .unwrap();
        assert_eq!(ids(db.read_blocking(answered).unwrap()), ["demowf-quote"]);
        assert!(!ids(db.read_blocking(|c| due(c, NOW + DAY_MS, 3)).unwrap()).contains(&"demowf-quote".to_owned()));
        db.write_blocking(|tx| end(tx, "demowf-quote", "answered", NOW)).unwrap();
        assert!(db.read_blocking(answered).unwrap().is_empty());
        assert_eq!(db.read_blocking(|c| get(c, "demowf-quote")).unwrap().unwrap().state, "answered");
    }
}
