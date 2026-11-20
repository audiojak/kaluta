//! Reply-landed alerts (spec §14.7 amendment 2026-11-18): conversations
//! the user wants to hear about as soon as someone answers. A watch tells
//! once, then is gone.

use mail_domain::{Millis, ThreadId};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::StoreResult;

/// Watch a conversation, or (`false`) no longer; whether it was watched.
pub fn set(tx: &Transaction, thread: &ThreadId, on: bool, now: Millis) -> StoreResult<bool> {
    let was = is_watched(tx, thread)?;
    if on {
        tx.prepare_cached("INSERT OR IGNORE INTO watches (thread_id, set_at) VALUES (?1, ?2)")?
            .execute(params![thread.as_str(), now])?;
    } else {
        tx.prepare_cached("DELETE FROM watches WHERE thread_id = ?1")?.execute([thread.as_str()])?;
    }
    Ok(was)
}

pub fn is_watched(conn: &Connection, thread: &ThreadId) -> StoreResult<bool> {
    Ok(conn
        .prepare_cached("SELECT 1 FROM watches WHERE thread_id = ?1")?
        .query_row([thread.as_str()], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Every watched conversation.
pub fn list(conn: &Connection) -> StoreResult<Vec<ThreadId>> {
    let mut stmt = conn.prepare_cached("SELECT thread_id FROM watches ORDER BY set_at, thread_id")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0).map(ThreadId))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Of `threads` (where mail just arrived), the watched ones someone else
/// has written in since the watch was set: their watches end here, so
/// each tells once.
pub fn landed(tx: &Transaction, threads: &[ThreadId]) -> StoreResult<Vec<ThreadId>> {
    let mut out = Vec::new();
    for thread in threads {
        let answered: bool = tx
            .prepare_cached(
                "SELECT EXISTS (SELECT 1 FROM watches w JOIN threads t ON t.gmail_id = w.thread_id
                                  JOIN messages m ON m.thread_id = t.id
                                 WHERE w.thread_id = ?1 AND m.is_draft = 0 AND m.is_sent_by_me = 0
                                   AND m.internal_date > w.set_at)",
            )?
            .query_row([thread.as_str()], |r| r.get(0))?;
        if answered && !out.contains(thread) {
            tx.prepare_cached("DELETE FROM watches WHERE thread_id = ?1")?.execute([thread.as_str()])?;
            out.push(thread.clone());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use mail_domain::{EmailAddress, LabelId, MessageId};

    use super::*;
    use crate::{Db, IncomingMessage, MailWriter};

    struct Store(std::path::PathBuf, Db);

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("kaluta-watches-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store(dir.clone(), Db::open(&dir.join("mail.sqlite")).unwrap())
    }

    fn message(db: &Db, id: &str, at: Millis, mine: bool) {
        let from = if mine { "me@example.com" } else { "sam@example.com" };
        let m = IncomingMessage {
            id: MessageId::new(id),
            thread_id: ThreadId::new("t1"),
            from: Some(EmailAddress::new(None, from)),
            date: at,
            internal_date: at,
            // Sent mail carries SENT, which marks it the user's.
            label_ids: vec![LabelId::new(if mine { "SENT" } else { "INBOX" })],
            ..Default::default()
        };
        db.write_blocking(move |tx| {
            let mut w = MailWriter::new(tx);
            w.upsert_message(&m)?;
            w.finish().map(|_| ())
        })
        .unwrap();
    }

    #[test]
    fn a_watch_tells_once_when_someone_else_writes() {
        let Store(_, db) = &store("landed");
        let t1 = ThreadId::new("t1");
        message(db, "m1", 100, false);
        let t = t1.clone();
        assert!(!db.write_blocking(move |tx| set(tx, &t, true, 200)).unwrap());
        assert_eq!(db.read_blocking(list).unwrap(), std::slice::from_ref(&t1));
        // The user's own message, and the one before the watch: nothing.
        message(db, "m2", 300, true);
        let ts = vec![t1.clone()];
        assert!(db.write_blocking(move |tx| landed(tx, &ts)).unwrap().is_empty());
        // Someone answers: told once, and the watch is gone.
        message(db, "m3", 400, false);
        let ts = vec![t1.clone(), t1.clone()];
        assert_eq!(db.write_blocking(move |tx| landed(tx, &ts)).unwrap(), std::slice::from_ref(&t1));
        let ts = vec![t1.clone()];
        assert!(db.write_blocking(move |tx| landed(tx, &ts)).unwrap().is_empty(), "once");
        assert!(db.read_blocking(list).unwrap().is_empty());
        // Unwatched by hand.
        let t = t1.clone();
        db.write_blocking(move |tx| set(tx, &t, true, 500)).unwrap();
        let t = t1.clone();
        assert!(db.write_blocking(move |tx| set(tx, &t, false, 600)).unwrap());
        assert!(db.read_blocking(list).unwrap().is_empty());
    }
}
