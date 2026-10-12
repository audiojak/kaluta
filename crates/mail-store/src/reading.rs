//! Weekly reading (spec §14.13): the conversations the sorting routine
//! filed under its bucket labels, and Kaluta's summaries of them.

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::StoreResult;

/// A filed conversation as a bucket lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filed {
    pub thread_id: String,
    /// Its latest message (Gmail id).
    pub message_id: String,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_email: Option<String>,
    pub at: Millis,
    pub unread: bool,
}

/// The label's Gmail id, by its full name ("Marked Important/1-Daily").
pub fn label_id(conn: &Connection, name: &str) -> StoreResult<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT gmail_id FROM labels WHERE name = ?1 COLLATE NOCASE")?
        .query_row([name], |r| r.get(0))
        .optional()?)
}

/// The conversations under a label, newest first; with `unread_only`,
/// those with anything unread.
pub fn filed(conn: &Connection, label_id: &str, unread_only: bool) -> StoreResult<Vec<Filed>> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.gmail_id, m.gmail_id, COALESCE(NULLIF(t.subject, ''), m.subject), m.from_name, m.from_email,
                m.internal_date,
                EXISTS (SELECT 1 FROM messages u WHERE u.thread_id = t.id AND u.is_read = 0 AND u.is_draft = 0)
           FROM thread_labels tl
           JOIN labels l ON l.id = tl.label_id
           JOIN threads t ON t.id = tl.thread_id
           JOIN messages m ON m.id = (SELECT id FROM messages WHERE thread_id = t.id AND is_draft = 0
                                       ORDER BY internal_date DESC, id DESC LIMIT 1)
          WHERE l.gmail_id = ?1
          ORDER BY m.internal_date DESC",
    )?;
    let rows = stmt.query_map([label_id], |r| {
        Ok(Filed {
            thread_id: r.get(0)?,
            message_id: r.get(1)?,
            subject: r.get(2)?,
            from_name: r.get(3)?,
            from_email: r.get(4)?,
            at: r.get(5)?,
            unread: r.get(6)?,
        })
    })?;
    let all: Vec<Filed> = rows.collect::<Result<_, _>>()?;
    Ok(if unread_only { all.into_iter().filter(|f| f.unread).collect() } else { all })
}

/// A summary as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub thread_id: String,
    pub message_id: String,
    pub line: String,
    pub summary: String,
}

pub fn save_summary(tx: &Transaction, s: &Summary, agent: Option<&str>, now: Millis) -> StoreResult<()> {
    tx.execute(
        "INSERT OR REPLACE INTO reading_summaries (thread_id, message_id, line, summary, agent, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![s.thread_id, s.message_id, s.line, s.summary, agent, now],
    )?;
    Ok(())
}

pub fn summary(conn: &Connection, thread_id: &str) -> StoreResult<Option<Summary>> {
    Ok(conn
        .prepare_cached("SELECT thread_id, message_id, line, summary FROM reading_summaries WHERE thread_id = ?1")?
        .query_row([thread_id], |r| {
            Ok(Summary { thread_id: r.get(0)?, message_id: r.get(1)?, line: r.get(2)?, summary: r.get(3)? })
        })
        .optional()?)
}

/// Filed conversations under any of `labels`, from `since`, with no
/// summary of their latest message, newest first.
pub fn to_summarise(conn: &Connection, labels: &[String], since: Millis, limit: usize) -> StoreResult<Vec<Filed>> {
    let mut have = conn.prepare_cached("SELECT 1 FROM reading_summaries WHERE thread_id = ?1 AND message_id = ?2")?;
    let mut out = Vec::new();
    for label in labels {
        for f in filed(conn, label, false)? {
            if out.len() >= limit {
                return Ok(out);
            }
            if f.at >= since && !have.exists(params![f.thread_id, f.message_id])? {
                out.push(f);
            }
        }
    }
    Ok(out)
}

/// Mark every message in the conversation read (reading and clearing keep
/// the routine's label; spec §14.13). Local only; the caller queues it for
/// Gmail.
pub fn mark_read(tx: &Transaction, thread_id: &str) -> StoreResult<()> {
    tx.execute(
        "UPDATE messages SET is_read = 1 WHERE thread_id = (SELECT id FROM threads WHERE gmail_id = ?1)",
        [thread_id],
    )?;
    Ok(())
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

    #[test]
    fn a_bucket_lists_its_mail_and_summaries_follow_the_latest_message() {
        let dir = std::env::temp_dir().join(format!("kaluta-reading-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = Store(dir.clone(), Db::open(&dir.join("mail.sqlite")).unwrap());
        let db = &s.1;
        crate::demo_workflows::generate(db, NOW).unwrap();
        let label = db
            .read_blocking(|c| label_id(c, "Marked Important/2-Weekly-Newsletters"))
            .unwrap()
            .expect("the demo has the routine's labels");
        let l = label.clone();
        let all = db.read_blocking(move |c| filed(c, &l, false)).unwrap();
        assert!(all.len() >= 2, "{all:?}");
        let labels = vec![label.clone()];
        let todo = db.read_blocking(move |c| to_summarise(c, &labels, NOW - 30 * 86_400_000, 50)).unwrap();
        assert_eq!(todo.len(), all.len());
        let first = todo[0].clone();
        let s1 = Summary {
            thread_id: first.thread_id.clone(),
            message_id: first.message_id.clone(),
            line: "Pricing pages, release notes, onboarding email".into(),
            summary: "Three short pieces on product writing.".into(),
        };
        db.write_blocking(move |tx| save_summary(tx, &s1, Some("claude-code"), NOW)).unwrap();
        let labels = vec![label.clone()];
        let left = db.read_blocking(move |c| to_summarise(c, &labels, NOW - 30 * 86_400_000, 50)).unwrap();
        assert_eq!(left.len(), all.len() - 1);
        let t = first.thread_id.clone();
        assert!(!db.read_blocking(move |c| summary(c, &t)).unwrap().unwrap().line.is_empty());

        // Reading marks it read; the label stays.
        let t = first.thread_id.clone();
        db.write_blocking(move |tx| mark_read(tx, &t)).unwrap();
        let l = label.clone();
        let unread = db.read_blocking(move |c| filed(c, &l, true)).unwrap();
        assert!(!unread.iter().any(|f| f.thread_id == first.thread_id));
        let l = label.clone();
        assert!(db.read_blocking(move |c| filed(c, &l, false)).unwrap().iter().any(|f| f.thread_id == first.thread_id));
    }
}
