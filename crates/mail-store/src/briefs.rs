//! Meeting prep (spec §14.14 amendment 2026-11-18, ADR 0024): upcoming
//! meetings and the brief Kaluta wrote for each from the user's mail.

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::StoreResult;

/// Briefs for meetings this long past are forgotten.
const KEEP_MS: Millis = 7 * 86_400_000;

/// An upcoming meeting, as Granola's calendar lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upcoming {
    pub meeting_id: String,
    pub title: String,
    pub meeting_at: Millis,
    pub participants_json: String,
}

/// A meeting with its brief, once written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefRow {
    pub meeting_id: String,
    pub title: String,
    pub meeting_at: Millis,
    pub participants_json: String,
    pub brief: Option<String>,
}

/// One recent message with a person, for a brief.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    pub subject: String,
    pub snippet: String,
    pub at: Millis,
    pub by_me: bool,
}

/// Keep the upcoming meetings: new ones added, moved or renamed ones
/// changed. A meeting that moved keeps its brief only if its people did
/// not change.
pub fn upsert_upcoming(tx: &Transaction, meetings: &[Upcoming], now: Millis) -> StoreResult<()> {
    for m in meetings {
        tx.prepare_cached(
            "INSERT INTO meeting_briefs (meeting_id, title, meeting_at, participants_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (meeting_id) DO UPDATE SET title = excluded.title, meeting_at = excluded.meeting_at,
               brief = CASE WHEN participants_json = excluded.participants_json THEN brief END,
               participants_json = excluded.participants_json, updated_at = excluded.updated_at",
        )?
        .execute(params![m.meeting_id, m.title, m.meeting_at, m.participants_json, now])?;
    }
    tx.prepare_cached("DELETE FROM meeting_briefs WHERE meeting_at < ?1")?.execute([now - KEEP_MS])?;
    Ok(())
}

/// Meetings between `from` and `until` with no brief yet, soonest first.
pub fn to_brief(conn: &Connection, from: Millis, until: Millis, limit: u32) -> StoreResult<Vec<BriefRow>> {
    rows(
        conn,
        "brief IS NULL AND meeting_at >= ?1 AND meeting_at <= ?2 ORDER BY meeting_at LIMIT ?3",
        from,
        until,
        limit,
    )
}

/// The meetings between `from` and `until` that have a brief, soonest first.
pub fn briefed(conn: &Connection, from: Millis, until: Millis) -> StoreResult<Vec<BriefRow>> {
    rows(
        conn,
        "brief IS NOT NULL AND meeting_at >= ?1 AND meeting_at <= ?2 ORDER BY meeting_at LIMIT ?3",
        from,
        until,
        50,
    )
}

fn rows(conn: &Connection, filter: &str, from: Millis, until: Millis, limit: u32) -> StoreResult<Vec<BriefRow>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT meeting_id, title, meeting_at, participants_json, brief FROM meeting_briefs WHERE {filter}"
    ))?;
    let rows = stmt.query_map(params![from, until, limit], |r| {
        Ok(BriefRow {
            meeting_id: r.get(0)?,
            title: r.get(1)?,
            meeting_at: r.get(2)?,
            participants_json: r.get(3)?,
            brief: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn set_brief(tx: &Transaction, meeting_id: &str, brief: &str, now: Millis) -> StoreResult<()> {
    tx.prepare_cached("UPDATE meeting_briefs SET brief = ?2, updated_at = ?3 WHERE meeting_id = ?1")?
        .execute(params![meeting_id, brief, now])?;
    Ok(())
}

pub fn get(conn: &Connection, meeting_id: &str) -> StoreResult<Option<BriefRow>> {
    Ok(conn
        .prepare_cached(
            "SELECT meeting_id, title, meeting_at, participants_json, brief FROM meeting_briefs WHERE meeting_id = ?1",
        )?
        .query_row([meeting_id], |r| {
            Ok(BriefRow {
                meeting_id: r.get(0)?,
                title: r.get(1)?,
                meeting_at: r.get(2)?,
                participants_json: r.get(3)?,
                brief: r.get(4)?,
            })
        })
        .optional()?)
}

/// The latest messages to or from `email`, newest first.
pub fn exchanges_with(conn: &Connection, email: &str, limit: u32) -> StoreResult<Vec<Exchange>> {
    let email = email.trim().to_lowercase();
    if email.is_empty() {
        return Ok(vec![]);
    }
    let mut stmt = conn.prepare_cached(
        "SELECT m.subject, m.snippet, m.internal_date, m.is_sent_by_me FROM messages m
          WHERE m.is_draft = 0
            AND EXISTS (SELECT 1 FROM participants p WHERE p.message_id = m.id
                          AND p.role IN ('from', 'to', 'cc') AND lower(p.email) = ?1)
          ORDER BY m.internal_date DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![email, limit], |r| {
        Ok(Exchange { subject: r.get(0)?, snippet: r.get(1)?, at: r.get(2)?, by_me: r.get(3)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    const NOW: Millis = 1_790_000_000_000;
    const HOUR: Millis = 3_600_000;

    struct Store(std::path::PathBuf, Db);

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("kaluta-briefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store(dir.clone(), Db::open(&dir.join("mail.sqlite")).unwrap())
    }

    fn meeting(id: &str, at: Millis, people: &str) -> Upcoming {
        Upcoming {
            meeting_id: id.into(),
            title: format!("Call {id}"),
            meeting_at: at,
            participants_json: people.into(),
        }
    }

    #[test]
    fn a_persons_mail_is_found_by_any_of_their_roles() {
        use mail_domain::{EmailAddress, LabelId, MessageId, ThreadId};
        let Store(_, db) = &store("exchanges");
        let message = |id: &str, from: &str, to: &str, at: Millis| crate::IncomingMessage {
            id: MessageId::new(id),
            thread_id: ThreadId::new(id),
            from: Some(EmailAddress::new(None, from)),
            to: vec![EmailAddress::new(None, to)],
            subject: id.into(),
            date: at,
            internal_date: at,
            label_ids: vec![LabelId::new("INBOX")],
            ..Default::default()
        };
        let all = vec![
            message("from-reese", "Reese@Example.org", "me@example.com", 1),
            message("to-reese", "me@example.com", "reese@example.org", 2),
            message("other", "sam@example.com", "me@example.com", 3),
        ];
        db.write_blocking(move |tx| {
            let mut w = crate::MailWriter::new(tx);
            for m in &all {
                w.upsert_message(m)?;
            }
            w.finish().map(|_| ())
        })
        .unwrap();
        let found = db.read_blocking(|c| exchanges_with(c, "REESE@example.org", 5)).unwrap();
        assert_eq!(found.iter().map(|e| e.subject.as_str()).collect::<Vec<_>>(), ["to-reese", "from-reese"]);
        assert!(db.read_blocking(|c| exchanges_with(c, " ", 5)).unwrap().is_empty());
    }

    #[test]
    fn meetings_wait_for_a_brief_and_keep_it_unless_their_people_change() {
        let Store(_, db) = &store("upcoming");
        let m = vec![meeting("a", NOW + 2 * HOUR, "[1]"), meeting("b", NOW + 30 * HOUR, "[2]")];
        db.write_blocking(move |tx| upsert_upcoming(tx, &m, NOW)).unwrap();
        let soon = db.read_blocking(|c| to_brief(c, NOW, NOW + 24 * HOUR, 5)).unwrap();
        assert_eq!(soon.iter().map(|r| r.meeting_id.as_str()).collect::<Vec<_>>(), ["a"], "within the day");
        db.write_blocking(|tx| set_brief(tx, "a", "Sam: owes the deck.", NOW)).unwrap();
        assert!(db.read_blocking(|c| to_brief(c, NOW, NOW + 24 * HOUR, 5)).unwrap().is_empty());
        assert_eq!(
            db.read_blocking(|c| briefed(c, NOW, NOW + 24 * HOUR)).unwrap()[0].brief.as_deref(),
            Some("Sam: owes the deck.")
        );
        // Moved, same people: the brief stays. New people: written again.
        let moved = vec![meeting("a", NOW + 3 * HOUR, "[1]")];
        db.write_blocking(move |tx| upsert_upcoming(tx, &moved, NOW)).unwrap();
        assert!(db.read_blocking(|c| get(c, "a")).unwrap().unwrap().brief.is_some());
        let changed = vec![meeting("a", NOW + 3 * HOUR, "[1,3]")];
        db.write_blocking(move |tx| upsert_upcoming(tx, &changed, NOW)).unwrap();
        assert!(db.read_blocking(|c| get(c, "a")).unwrap().unwrap().brief.is_none());
    }
}
