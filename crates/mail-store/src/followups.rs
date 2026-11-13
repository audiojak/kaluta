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

/// Granola lists a meeting that has no note yet (it has not happened, or
/// its notes are not written) as a calendar entry under an `evt_` id, with
/// the invitation's addresses for people; once the note exists the same
/// meeting comes back under the note's own id. Only notes are meetings
/// here: there is nothing to judge in a calendar entry, and keeping both
/// showed the meeting twice.
pub fn is_calendar_entry(meeting_id: &str) -> bool {
    meeting_id.starts_with("evt_")
}

/// `meeting_id LIKE` this, with `ESCAPE '\'`, is a calendar entry.
const CALENDAR_IDS: &str = "evt\\_%";

/// Forget the calendar entries kept before they were left out. A decision
/// made on one goes to its note (same title and time) if that has none,
/// in the meeting and in any workflow set still open, so nothing already
/// decided is asked again. How many were forgotten.
pub fn drop_calendar_entries(tx: &Transaction<'_>) -> StoreResult<usize> {
    let pairs: Vec<(String, Option<String>)> = tx
        .prepare(&format!(
            "SELECT e.meeting_id,
                    (SELECT n.meeting_id FROM meeting_followups n
                      WHERE n.title = e.title AND n.meeting_at = e.meeting_at
                        AND n.meeting_id NOT LIKE '{CALENDAR_IDS}' ESCAPE '\\' LIMIT 1)
               FROM meeting_followups e WHERE e.meeting_id LIKE '{CALENDAR_IDS}' ESCAPE '\\'"
        ))?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    if pairs.is_empty() {
        return Ok(0);
    }
    for (entry, note) in &pairs {
        let (old, new) = (format!("meeting:{entry}"), note.as_ref().map(|n| format!("meeting:{n}")));
        if let (Some(note), Some(new)) = (note, &new) {
            tx.execute(
                "UPDATE meeting_followups
                    SET decided = (SELECT decided FROM meeting_followups WHERE meeting_id = ?1),
                        decided_at = (SELECT decided_at FROM meeting_followups WHERE meeting_id = ?1)
                  WHERE meeting_id = ?2 AND decided IS NULL
                    AND (SELECT decided FROM meeting_followups WHERE meeting_id = ?1) IS NOT NULL",
                params![entry, note],
            )?;
            // In a set still open, the note's item takes the decision.
            tx.execute(
                "UPDATE OR IGNORE workflow_decisions SET item = ?2
                  WHERE item = ?1 AND set_id IN (SELECT id FROM workflow_sets WHERE finished_at IS NULL)",
                params![old, new],
            )?;
        }
        tx.execute(
            "DELETE FROM workflow_decisions
              WHERE item = ?1 AND set_id IN (SELECT id FROM workflow_sets WHERE finished_at IS NULL)",
            [&old],
        )?;
        let open: Vec<(i64, String)> = tx
            .prepare(
                "SELECT id, items_json FROM workflow_sets WHERE finished_at IS NULL AND instr(items_json, ?1) > 0",
            )?
            .query_map([&old], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (set_id, items_json) in open {
            let items: Vec<String> = serde_json::from_str(&items_json)?;
            let kept: Vec<String> = items.into_iter().filter(|i| i != &old).collect();
            crate::workflows::set_items(tx, set_id, &kept)?;
        }
        tx.execute("DELETE FROM meeting_followups WHERE meeting_id = ?1", [entry])?;
    }
    Ok(pairs.len())
}

/// Set once the follow-ups written before 2026-11-12 have been sent back.
const UNDATED_KEY: &str = "followups_undated";
/// Words that place an email on the day of its meeting.
const DAY_WORDS: [&str; 7] =
    ["today", "yesterday", "this morning", "this afternoon", "this evening", "tonight", "tomorrow"];

/// Follow-ups used to be written as if sent on the day of the meeting
/// ("Good to talk today"), and they are sent days later. Once: those
/// still waiting on the user that name such a day are judged again, so
/// they are rewritten without one. They keep their old words until then.
/// How many were sent back.
pub fn rewrite_dated(tx: &Transaction<'_>) -> StoreResult<usize> {
    if crate::workflows::meta(tx, UNDATED_KEY)?.is_some() {
        return Ok(0);
    }
    let names_a_day =
        DAY_WORDS.iter().map(|w| format!("lower(options_json) LIKE '%{w}%'")).collect::<Vec<_>>().join(" OR ");
    let n = tx.execute(
        &format!(
            "UPDATE meeting_followups SET judged_at = NULL, attempts = 0
              WHERE follow_up = 1 AND decided IS NULL AND ({names_a_day})"
        ),
        [],
    )?;
    crate::workflows::set_meta(tx, UNDATED_KEY, "1")?;
    Ok(n)
}

/// Keep meetings read from Granola; a meeting seen before keeps its
/// verdict (its title and people are brought up to date). Calendar
/// entries are left out, and any kept earlier are forgotten. How many
/// were new.
pub fn upsert_meetings(tx: &Transaction<'_>, meetings: &[NewMeeting], now: Millis) -> StoreResult<usize> {
    drop_calendar_entries(tx)?;
    rewrite_dated(tx)?;
    let mut new = 0;
    let mut stmt = tx.prepare_cached(
        "INSERT INTO meeting_followups (meeting_id, title, meeting_at, participants_json, url, seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT (meeting_id) DO UPDATE SET
           title = excluded.title, meeting_at = excluded.meeting_at,
           participants_json = excluded.participants_json, url = excluded.url",
    )?;
    for m in meetings.iter().filter(|m| !is_calendar_entry(&m.meeting_id)) {
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
    fn a_calendar_entry_is_not_a_meeting_and_one_kept_before_gives_its_note_its_decision() {
        let db = Db::open(&crate::db::tests::temp_db_path("followups-calendar")).unwrap();
        // As kept before: the entry read ahead of the meeting, then its note.
        let twin = |id: &str| NewMeeting { title: "Infra demo".into(), ..meeting(id, 500) };
        let kept = [twin("evt_abc123"), twin("note-1"), meeting("evt_lunch", 600), meeting("note-2", 700)];
        db.write_blocking(move |tx| {
            for m in kept {
                tx.execute(
                    "INSERT INTO meeting_followups (meeting_id, title, meeting_at, participants_json, url, seen_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                    params![m.meeting_id, m.title, m.meeting_at, m.participants_json, m.url],
                )?;
            }
            decide(tx, "evt_abc123", Some("none"), 9)?;
            let set = crate::workflows::create_set(
                tx,
                "followups",
                "2026-11-12",
                &[
                    "meeting:evt_abc123".into(),
                    "meeting:note-1".into(),
                    "meeting:evt_lunch".into(),
                    "meeting:note-2".into(),
                ],
                1,
            )?;
            crate::workflows::decide(tx, set, "meeting:evt_abc123", "none", 9, 0)?;
            Ok(())
        })
        .unwrap();

        // The next read of Granola lists a calendar entry again, and a note.
        let new =
            db.write_blocking(|tx| upsert_meetings(tx, &[meeting("evt_tomorrow", 900), meeting("note-3", 800)], 20));
        assert_eq!(new.unwrap(), 1, "the entry is not kept");
        let ids: Vec<String> = db
            .read_blocking(|c| {
                Ok(c.prepare("SELECT meeting_id FROM meeting_followups ORDER BY meeting_id")?
                    .query_map([], |r| r.get(0))?
                    .collect::<Result<_, _>>()?)
            })
            .unwrap();
        assert_eq!(ids, ["note-1", "note-2", "note-3"]);
        let note = db.read_blocking(|c| get(c, "note-1")).unwrap().unwrap();
        assert_eq!(note.decided.as_deref(), Some("none"), "decided once, on the entry: not asked again");
        assert_eq!(db.read_blocking(|c| get(c, "note-2")).unwrap().unwrap().decided, None);
        let (items, decided) = db
            .read_blocking(|c| {
                let set = crate::workflows::open_set(c, "followups", "2026-11-12")?.unwrap();
                let decided: Vec<String> =
                    crate::workflows::decisions(c, set.id)?.into_iter().map(|d| d.item).collect();
                Ok((set.items, decided))
            })
            .unwrap();
        assert_eq!(items, ["meeting:note-1", "meeting:note-2"]);
        assert_eq!(decided, ["meeting:note-1"]);
        assert_eq!(db.write_blocking(drop_calendar_entries).unwrap(), 0, "nothing left to forget");
    }

    #[test]
    fn follow_ups_that_name_the_day_are_judged_again_once() {
        let db = Db::open(&crate::db::tests::temp_db_path("followups-undated")).unwrap();
        db.write_blocking(|tx| {
            upsert_meetings(tx, &[meeting("a", 100), meeting("b", 200), meeting("c", 300), meeting("d", 400)], 1)?;
            record_verdict(tx, "a", true, "Promised the deck.", r#"[{"body":"Good to talk Today."}]"#, 5)?;
            record_verdict(tx, "b", true, "Next steps.", r#"[{"body":"Thanks for our conversation."}]"#, 5)?;
            record_verdict(tx, "c", true, "Thanks.", r#"[{"body":"Great to meet this morning."}]"#, 5)?;
            record_verdict(tx, "d", false, "A standup.", "[]", 5)?;
            decide(tx, "c", Some("sent"), 6)?;
            // As before this change: nothing has been sent back yet.
            tx.execute("DELETE FROM workflow_meta WHERE key = ?1", [UNDATED_KEY])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(db.write_blocking(rewrite_dated).unwrap(), 1, "a: waiting, and it says today");
        let again = db.read_blocking(|c| to_judge(c, 0, 1000, 10)).unwrap();
        assert_eq!(again.iter().map(|m| m.meeting_id.as_str()).collect::<Vec<_>>(), ["a"]);
        assert_eq!(db.read_blocking(|c| waiting(c, 0)).unwrap().len(), 2, "a still shows, with its old words");
        db.write_blocking(|tx| record_verdict(tx, "a", true, "Promised the deck.", r#"[{"body":"Still today."}]"#, 9))
            .unwrap();
        assert_eq!(db.write_blocking(rewrite_dated).unwrap(), 0, "once only, whatever the new words");
        assert!(db.read_blocking(|c| to_judge(c, 0, 1000, 10)).unwrap().is_empty());
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
