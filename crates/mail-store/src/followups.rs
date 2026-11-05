//! Meeting follow-ups (spec §14.14): meetings read from Granola, Kaluta's
//! verdict on each, and the user's decision. The workflow's set is the
//! meetings that deserve a follow-up and have not been decided.

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::StoreResult;

/// When the follow-ups job last read Granola (ms), in `workflow_meta`.
pub const CHECKED_KEY: &str = "granola_checked_at";
/// Tries before a meeting is given up.
pub const MAX_ATTEMPTS: i64 = 3;

/// A meeting as read from Granola.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMeeting {
    pub meeting_id: String,
    pub title: String,
    pub meeting_at: Millis,
    pub participants_json: String,
    pub url: Option<String>,
}

/// A meeting and Kaluta's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowUpRow {
    pub meeting_id: String,
    pub title: String,
    pub meeting_at: Millis,
    pub participants_json: String,
    pub url: Option<String>,
    pub judged_at: Option<Millis>,
    pub follow_up: Option<bool>,
    pub reason: String,
    pub options_json: String,
    pub decided: Option<String>,
}

const COLUMNS: &str =
    "meeting_id, title, meeting_at, participants_json, url, judged_at, follow_up, reason, options_json, decided";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<FollowUpRow> {
    Ok(FollowUpRow {
        meeting_id: r.get(0)?,
        title: r.get(1)?,
        meeting_at: r.get(2)?,
        participants_json: r.get(3)?,
        url: r.get(4)?,
        judged_at: r.get(5)?,
        follow_up: r.get::<_, Option<i64>>(6)?.map(|v| v != 0),
        reason: r.get(7)?,
        options_json: r.get(8)?,
        decided: r.get(9)?,
    })
}

/// Keep meetings read from Granola; a meeting seen before keeps its
/// verdict (its title and people are brought up to date). How many were new.
pub fn upsert_meetings(tx: &Transaction<'_>, meetings: &[NewMeeting], now: Millis) -> StoreResult<usize> {
    let mut new = 0;
    let mut stmt = tx.prepare_cached(
        "INSERT INTO meeting_followups (meeting_id, title, meeting_at, participants_json, url, seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (meeting_id) DO UPDATE SET
           title = excluded.title, meeting_at = excluded.meeting_at,
           participants_json = excluded.participants_json, url = excluded.url",
    )?;
    for m in meetings {
        let existed =
            tx.prepare_cached("SELECT 1 FROM meeting_followups WHERE meeting_id = ?1")?.exists([&m.meeting_id])?;
        stmt.execute(params![m.meeting_id, m.title, m.meeting_at, m.participants_json, m.url, now])?;
        if !existed {
            new += 1;
        }
    }
    Ok(new)
}

/// Meetings not yet judged, held before `before` (their notes are done
/// by then) and after `since`, oldest first, at most `limit`.
pub fn to_judge(conn: &Connection, since: Millis, before: Millis, limit: u32) -> StoreResult<Vec<FollowUpRow>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM meeting_followups
         WHERE judged_at IS NULL AND attempts < ?3 AND meeting_at >= ?1 AND meeting_at < ?2
         ORDER BY meeting_at LIMIT ?4"
    );
    Ok(conn
        .prepare_cached(&sql)?
        .query_map(params![since, before, MAX_ATTEMPTS, limit], row)?
        .collect::<Result<_, _>>()?)
}

/// Kaluta's verdict on a meeting.
pub fn record_verdict(
    tx: &Transaction<'_>,
    meeting_id: &str,
    follow_up: bool,
    reason: &str,
    options_json: &str,
    now: Millis,
) -> StoreResult<()> {
    tx.execute(
        "UPDATE meeting_followups SET judged_at = ?2, follow_up = ?3, reason = ?4, options_json = ?5
         WHERE meeting_id = ?1",
        params![meeting_id, now, follow_up, reason, options_json],
    )?;
    Ok(())
}

/// A try at a meeting failed; after [`MAX_ATTEMPTS`] it is not tried again.
pub fn record_failure(tx: &Transaction<'_>, meeting_id: &str) -> StoreResult<()> {
    tx.execute("UPDATE meeting_followups SET attempts = attempts + 1 WHERE meeting_id = ?1", [meeting_id])?;
    Ok(())
}

/// The meetings waiting on the user: judged worth a follow-up, not
/// decided, held since `since`; newest first.
pub fn waiting(conn: &Connection, since: Millis) -> StoreResult<Vec<FollowUpRow>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM meeting_followups
         WHERE follow_up = 1 AND decided IS NULL AND meeting_at >= ?1 ORDER BY meeting_at DESC"
    );
    Ok(conn.prepare_cached(&sql)?.query_map([since], row)?.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, meeting_id: &str) -> StoreResult<Option<FollowUpRow>> {
    let sql = format!("SELECT {COLUMNS} FROM meeting_followups WHERE meeting_id = ?1");
    Ok(conn.prepare_cached(&sql)?.query_row([meeting_id], row).optional()?)
}

/// The user decided (`sent`, `none`); `None` takes it back (undo).
pub fn decide(tx: &Transaction<'_>, meeting_id: &str, decision: Option<&str>, now: Millis) -> StoreResult<()> {
    tx.execute(
        "UPDATE meeting_followups SET decided = ?2, decided_at = CASE WHEN ?2 IS NULL THEN NULL ELSE ?3 END
         WHERE meeting_id = ?1",
        params![meeting_id, decision, now],
    )?;
    Ok(())
}

/// Forget every meeting (Granola disconnected).
pub fn clear(tx: &Transaction<'_>) -> StoreResult<()> {
    tx.execute("DELETE FROM meeting_followups", [])?;
    tx.execute("DELETE FROM workflow_meta WHERE key = ?1", [CHECKED_KEY])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    fn meeting(id: &str, at: Millis) -> NewMeeting {
        NewMeeting {
            meeting_id: id.into(),
            title: format!("Meeting {id}"),
            meeting_at: at,
            participants_json: r#"[{"name":"Sam","email":"sam@example.org"}]"#.into(),
            url: None,
        }
    }

    #[test]
    fn meetings_are_judged_once_and_wait_until_decided() {
        let db = Db::open(&crate::db::tests::temp_db_path("followups")).unwrap();
        let n = db.write_blocking(|tx| upsert_meetings(tx, &[meeting("a", 100), meeting("b", 200)], 1)).unwrap();
        assert_eq!(n, 2);
        let again = db.write_blocking(|tx| upsert_meetings(tx, &[meeting("a", 100)], 2)).unwrap();
        assert_eq!(again, 0, "seen before");
        let judge = db.read_blocking(|c| to_judge(c, 0, 150, 10)).unwrap();
        assert_eq!(judge.iter().map(|m| m.meeting_id.as_str()).collect::<Vec<_>>(), ["a"], "b is too recent");
        db.write_blocking(|tx| {
            record_verdict(tx, "a", true, "They asked for the deck.", "[]", 5)?;
            record_verdict(tx, "b", false, "A standup.", "[]", 5)
        })
        .unwrap();
        assert!(db.read_blocking(|c| to_judge(c, 0, 1000, 10)).unwrap().is_empty());
        let wait = db.read_blocking(|c| waiting(c, 0)).unwrap();
        assert_eq!(wait.len(), 1);
        assert_eq!((wait[0].follow_up, wait[0].reason.as_str()), (Some(true), "They asked for the deck."));
        db.write_blocking(|tx| upsert_meetings(tx, &[meeting("a", 100)], 9)).unwrap();
        assert_eq!(db.read_blocking(|c| get(c, "a")).unwrap().unwrap().follow_up, Some(true), "kept its verdict");
        db.write_blocking(|tx| decide(tx, "a", Some("none"), 10)).unwrap();
        assert!(db.read_blocking(|c| waiting(c, 0)).unwrap().is_empty());
        db.write_blocking(|tx| decide(tx, "a", None, 11)).unwrap();
        assert_eq!(db.read_blocking(|c| waiting(c, 0)).unwrap().len(), 1, "undone");
        // Three failures and it is given up.
        db.write_blocking(|tx| upsert_meetings(tx, &[meeting("c", 120)], 12)).unwrap();
        for _ in 0..3 {
            db.write_blocking(|tx| record_failure(tx, "c")).unwrap();
        }
        assert!(db.read_blocking(|c| to_judge(c, 0, 1000, 10)).unwrap().is_empty());
        db.write_blocking(clear).unwrap();
        assert!(db.read_blocking(|c| get(c, "a")).unwrap().is_none());
    }

    /// The migration that allows the new kind keeps every set and decision.
    #[test]
    fn existing_sets_and_decisions_survive_the_new_kind() {
        let path = crate::db::tests::temp_db_path("followups-migration");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before = crate::db::migrations().len() - 1;
        {
            let c = Connection::open(&path).unwrap();
            c.pragma_update(None, "foreign_keys", "ON").unwrap();
            for sql in &crate::db::migrations()[..before] {
                c.execute_batch(sql).unwrap();
            }
            c.execute_batch(&format!(
                "PRAGMA user_version = {before};
                 INSERT INTO workflow_sets (id, kind, day, items_json, started_at) VALUES (7, 'triage', '2026-11-03', '[\"thread:a\"]', 1);
                 INSERT INTO workflow_decisions (set_id, item, decision, decided_at, millis) VALUES (7, 'thread:a', 'archive', 2, 900);"
            ))
            .unwrap();
        }
        let db = Db::open(&path).unwrap();
        let (kind, decision): (String, String) = db
            .read_blocking(|c| {
                Ok(c.query_row(
                    "SELECT s.kind, d.decision FROM workflow_sets s JOIN workflow_decisions d ON d.set_id = s.id",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .unwrap();
        assert_eq!((kind.as_str(), decision.as_str()), ("triage", "archive"));
        db.write_blocking(|tx| {
            tx.execute(
                "INSERT INTO workflow_sets (kind, day, items_json, started_at) VALUES ('followups', '2026-11-04', '[]', 3)",
                [],
            )?;
            // The decisions still belong to their set.
            tx.execute("DELETE FROM workflow_sets WHERE id = 7", [])?;
            Ok(())
        })
        .unwrap();
        let left: i64 = db
            .read_blocking(|c| Ok(c.query_row("SELECT COUNT(*) FROM workflow_decisions", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(left, 0, "deleting a set still deletes its decisions");
        assert!(db.read_blocking(crate::consistency::check).unwrap().is_empty());
    }
}
