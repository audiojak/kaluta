//! Remind me later (spec §14.13, amended 2026-11-09): a conversation kept
//! out of Triage until a time the user chose, then brought back to the top
//! of it. The email stays in the Inbox meanwhile; mail from someone else in
//! the conversation after the reminder was set brings it back at once.

use std::collections::HashSet;

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::StoreResult;
use crate::workflows::{self, Candidate};

/// Reminders this long past their time are forgotten.
const KEEP_MS: Millis = 30 * 86_400_000;

/// A conversation's reminder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reminder {
    pub remind_at: Millis,
    pub set_at: Millis,
}

/// Remind about a conversation at `remind_at`, or (`None`) no longer;
/// the reminder it had before, to put back.
pub fn set(tx: &Transaction, thread_id: &str, remind_at: Option<Millis>, now: Millis) -> StoreResult<Option<Reminder>> {
    let before = get(tx, thread_id)?;
    match remind_at {
        Some(at) => {
            tx.prepare_cached(
                "INSERT INTO reminders (thread_id, remind_at, set_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (thread_id) DO UPDATE SET remind_at = excluded.remind_at, set_at = excluded.set_at",
            )?
            .execute(params![thread_id, at, now])?;
        }
        None => {
            tx.prepare_cached("DELETE FROM reminders WHERE thread_id = ?1")?.execute([thread_id])?;
        }
    }
    tx.prepare_cached("DELETE FROM reminders WHERE remind_at < ?1")?.execute([now - KEEP_MS])?;
    Ok(before)
}

/// Put a reminder back as it was (undo).
pub fn restore(tx: &Transaction, thread_id: &str, reminder: Option<Reminder>) -> StoreResult<()> {
    match reminder {
        Some(r) => {
            tx.prepare_cached(
                "INSERT INTO reminders (thread_id, remind_at, set_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (thread_id) DO UPDATE SET remind_at = excluded.remind_at, set_at = excluded.set_at",
            )?
            .execute(params![thread_id, r.remind_at, r.set_at])?;
        }
        None => {
            tx.prepare_cached("DELETE FROM reminders WHERE thread_id = ?1")?.execute([thread_id])?;
        }
    }
    Ok(())
}

pub fn get(conn: &Connection, thread_id: &str) -> StoreResult<Option<Reminder>> {
    Ok(conn
        .prepare_cached("SELECT remind_at, set_at FROM reminders WHERE thread_id = ?1")?
        .query_row([thread_id], |r| Ok(Reminder { remind_at: r.get(0)?, set_at: r.get(1)? }))
        .optional()?)
}

/// Conversations kept out of Triage for now: their time has not come and
/// no one has written since.
pub fn hidden(conn: &Connection, now: Millis) -> StoreResult<HashSet<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT r.thread_id FROM reminders r
          WHERE r.remind_at > ?1
            AND NOT EXISTS (SELECT 1 FROM threads t JOIN messages m ON m.thread_id = t.id
                             WHERE t.gmail_id = r.thread_id AND m.is_draft = 0 AND m.is_sent_by_me = 0
                               AND m.internal_date > r.set_at)",
    )?;
    let rows = stmt.query_map([now], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Reminders still to come (the Dashboard's "later").
pub fn pending(conn: &Connection, now: Millis) -> StoreResult<u32> {
    Ok(hidden(conn, now)?.len() as u32)
}

/// Conversations whose reminder is due and that Triage has not decided
/// since, still in the Inbox with the last word someone else's; the
/// earliest reminder first.
pub fn due(conn: &Connection, now: Millis) -> StoreResult<Vec<Candidate>> {
    let mut stmt = conn.prepare_cached(
        "SELECT r.thread_id FROM reminders r
          WHERE r.remind_at <= ?1
            AND NOT EXISTS (SELECT 1 FROM workflow_decisions d JOIN workflow_sets s ON s.id = d.set_id
                             WHERE s.kind = 'triage' AND d.item = 'thread:' || r.thread_id
                               AND d.decided_at >= r.remind_at)
          ORDER BY r.remind_at, r.thread_id",
    )?;
    let ids: Vec<String> = stmt.query_map([now], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for id in ids {
        out.extend(workflows::inbox_candidate(conn, &id)?);
    }
    Ok(out)
}

/// Bring due reminders into an open triage set: each goes up next, and
/// one decided earlier in the set (put off from it) is undecided. How
/// many came back.
pub fn bring_back(tx: &Transaction, set_id: i64, now: Millis) -> StoreResult<usize> {
    let Some(set) = workflows::get_set(tx, set_id)? else { return Ok(0) };
    let decided: HashSet<String> = workflows::decisions(tx, set_id)?.into_iter().map(|d| d.item).collect();
    let mut items = set.items.clone();
    let mut n = 0;
    // The earliest last, so it ends up first.
    for c in due(tx, now)?.into_iter().rev() {
        let key = format!("thread:{}", c.thread_id);
        let waiting = items.contains(&key) && !decided.contains(&key);
        if decided.contains(&key) {
            workflows::undecide(tx, set_id, &key)?;
        } else if waiting {
            continue;
        }
        items.retain(|k| *k != key);
        let at = items.iter().position(|k| !decided.contains(k) || *k == key).unwrap_or(items.len());
        items.insert(at, key);
        n += 1;
    }
    if n > 0 {
        workflows::set_items(tx, set_id, &items)?;
    }
    Ok(n)
}

/// How many due reminders an open set does not hold undecided yet.
pub fn due_outside(conn: &Connection, set: &workflows::SetRow, now: Millis) -> StoreResult<u32> {
    let decided: HashSet<String> = workflows::decisions(conn, set.id)?.into_iter().map(|d| d.item).collect();
    Ok(due(conn, now)?
        .iter()
        .map(|c| format!("thread:{}", c.thread_id))
        .filter(|k| !set.items.contains(k) || decided.contains(k))
        .count() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;
    use crate::workflows::{create_set, decide, get_set, open_set};

    const NOW: Millis = 1_790_000_000_000;
    const HOUR: Millis = 3_600_000;

    struct Store(std::path::PathBuf, Db);

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("kaluta-reminders-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("mail.sqlite")).unwrap();
        crate::demo_workflows::generate(&db, NOW).unwrap();
        Store(dir, db)
    }

    /// A person's conversation in the demo's Inbox.
    fn a_thread(db: &Db) -> String {
        db.read_blocking(|c| workflows::triage_candidates(c, NOW - 86_400_000, 500))
            .unwrap()
            .into_iter()
            .find(|c| c.person)
            .unwrap()
            .thread_id
    }

    #[test]
    fn a_reminder_keeps_it_out_of_triage_until_its_time_then_puts_it_first() {
        let Store(_, db) = &store("time");
        let id = a_thread(db);
        let t = id.clone();
        let before = db.write_blocking(move |tx| set(tx, &t, Some(NOW + 3 * HOUR), NOW)).unwrap();
        assert_eq!(before, None);
        let triage = |db: &Db, now: Millis| {
            db.read_blocking(move |c| workflows::triage_set(c, NOW - 86_400_000, now, 500)).unwrap()
        };
        assert!(!triage(db, NOW + HOUR).iter().any(|c| c.thread_id == id), "put off");
        assert_eq!(db.read_blocking(|c| pending(c, NOW + HOUR)).unwrap(), 1);
        let later = triage(db, NOW + 3 * HOUR);
        assert_eq!(later[0].thread_id, id, "due: first");
        assert_eq!(later.iter().filter(|c| c.thread_id == id).count(), 1);
        assert_eq!(db.read_blocking(|c| pending(c, NOW + 3 * HOUR)).unwrap(), 0);
        // Still due long after "new" has moved on.
        let much_later = db
            .read_blocking(move |c| workflows::triage_set(c, NOW + 2 * 86_400_000, NOW + 2 * 86_400_000, 500))
            .unwrap();
        assert_eq!(much_later[0].thread_id, id);

        // Decided in Triage after it came due: done with.
        let key = format!("thread:{id}");
        let items = vec![key.clone()];
        let set_id = db.write_blocking(move |tx| create_set(tx, "triage", "2026-10-12", &items, NOW)).unwrap();
        db.write_blocking(move |tx| decide(tx, set_id, &key, "archive", NOW + 4 * HOUR, 1)).unwrap();
        assert!(db.read_blocking(|c| due(c, NOW + 5 * HOUR)).unwrap().is_empty());
    }

    #[test]
    fn new_mail_in_the_conversation_brings_it_back_and_clearing_it_too() {
        let Store(_, db) = &store("mail");
        let id = a_thread(db);
        let t = id.clone();
        // Set before the conversation's latest message: as if written since.
        db.write_blocking(move |tx| set(tx, &t, Some(NOW + 3 * 86_400_000), 0)).unwrap();
        assert!(db.read_blocking(|c| hidden(c, NOW)).unwrap().is_empty(), "someone wrote since");
        let t = id.clone();
        let before = db.write_blocking(move |tx| set(tx, &t, Some(NOW + HOUR), NOW)).unwrap();
        assert_eq!(before.map(|r| r.remind_at), Some(NOW + 3 * 86_400_000));
        assert!(db.read_blocking(|c| hidden(c, NOW)).unwrap().contains(&id));
        let t = id.clone();
        db.write_blocking(move |tx| restore(tx, &t, None)).unwrap();
        assert!(db.read_blocking(|c| hidden(c, NOW)).unwrap().is_empty());
        assert_eq!(db.read_blocking(move |c| get(c, &id)).unwrap(), None);
    }

    #[test]
    fn a_due_reminder_comes_back_into_the_open_set_up_next() {
        let Store(_, db) = &store("open");
        let found = db.read_blocking(|c| workflows::triage_candidates(c, NOW - 86_400_000, 500)).unwrap();
        let keys: Vec<String> = found.iter().take(4).map(|c| format!("thread:{}", c.thread_id)).collect();
        let (a, b, c) = (keys[0].clone(), keys[1].clone(), keys[2].clone());
        let set_id = db.write_blocking(move |tx| create_set(tx, "triage", "2026-10-11", &keys, NOW)).unwrap();
        // The first was put off from this set; the second decided.
        let (a2, b2) = (a.clone(), b.clone());
        db.write_blocking(move |tx| decide(tx, set_id, &a2, "remind", NOW, 1)).unwrap();
        db.write_blocking(move |tx| decide(tx, set_id, &b2, "archive", NOW, 1)).unwrap();
        let a_id = a.trim_start_matches("thread:").to_owned();
        db.write_blocking(move |tx| set(tx, &a_id, Some(NOW + HOUR), NOW)).unwrap();
        let open = db.read_blocking(|c| open_set(c, "triage", "2026-10-11")).unwrap().unwrap();
        assert_eq!(db.read_blocking(move |cn| due_outside(cn, &open, NOW + 2 * HOUR)).unwrap(), 1);

        assert_eq!(db.write_blocking(move |tx| bring_back(tx, set_id, NOW + 30 * 60_000)).unwrap(), 0, "not yet");
        assert_eq!(db.write_blocking(move |tx| bring_back(tx, set_id, NOW + 2 * HOUR)).unwrap(), 1);
        let set = db.read_blocking(move |cn| get_set(cn, set_id)).unwrap().unwrap();
        // After what is decided, before what waits.
        assert_eq!(&set.items[..3], &[b.clone(), a.clone(), c]);
        let decided = db.read_blocking(move |cn| workflows::decisions(cn, set_id)).unwrap();
        assert!(!decided.iter().any(|d| d.item == a), "undecided again");
        // Already waiting in the set: nothing moves.
        assert_eq!(db.write_blocking(move |tx| bring_back(tx, set_id, NOW + 2 * HOUR)).unwrap(), 0);
    }
}
