//! Send later (spec §14.5 amendment 2026-11-18, ADR 0022): drafts with a
//! time to be sent at. A scheduled draft stays a draft until its time; the
//! app sends it then by the ordinary send, so nothing waits in the outbox
//! and nothing shows in Sent before it has gone.

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::{StoreError, StoreResult};

/// A draft and when it is to be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduledSend {
    pub draft_id: i64,
    pub send_at: Millis,
}

/// Send a draft at `send_at`, or (`None`) no longer; the time it had
/// before, to put back. Only a draft still being edited can be scheduled.
pub fn set(tx: &Transaction, draft_id: i64, send_at: Option<Millis>, now: Millis) -> StoreResult<Option<Millis>> {
    let before = get(tx, draft_id)?;
    match send_at {
        Some(at) => {
            let editing: bool = tx
                .prepare_cached("SELECT state = 'editing' FROM drafts WHERE id = ?1")?
                .query_row([draft_id], |r| r.get(0))
                .optional()?
                .ok_or_else(|| StoreError::NotFound(format!("draft {draft_id}")))?;
            if !editing {
                return Err(StoreError::Invalid("this draft is already being sent".into()));
            }
            tx.prepare_cached(
                "INSERT INTO scheduled_sends (draft_id, send_at, set_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (draft_id) DO UPDATE SET send_at = excluded.send_at, set_at = excluded.set_at",
            )?
            .execute(params![draft_id, at, now])?;
        }
        None => {
            tx.prepare_cached("DELETE FROM scheduled_sends WHERE draft_id = ?1")?.execute([draft_id])?;
        }
    }
    Ok(before)
}

/// When a draft is to be sent, if it is scheduled.
pub fn get(conn: &Connection, draft_id: i64) -> StoreResult<Option<Millis>> {
    Ok(conn
        .prepare_cached("SELECT send_at FROM scheduled_sends WHERE draft_id = ?1")?
        .query_row([draft_id], |r| r.get(0))
        .optional()?)
}

/// Every scheduled draft still being edited, soonest first.
pub fn list(conn: &Connection) -> StoreResult<Vec<ScheduledSend>> {
    let mut stmt = conn.prepare_cached(
        "SELECT s.draft_id, s.send_at FROM scheduled_sends s JOIN drafts d ON d.id = s.draft_id
          WHERE d.state = 'editing' ORDER BY s.send_at, s.draft_id",
    )?;
    let rows = stmt.query_map([], |r| Ok(ScheduledSend { draft_id: r.get(0)?, send_at: r.get(1)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The scheduled drafts whose time has come at `now`, soonest first.
pub fn due(conn: &Connection, now: Millis) -> StoreResult<Vec<ScheduledSend>> {
    Ok(list(conn)?.into_iter().filter(|s| s.send_at <= now).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;
    use crate::drafts::{self, DraftRecord, DraftState};

    const NOW: Millis = 1_790_000_000_000;
    const HOUR: Millis = 3_600_000;

    struct Store(std::path::PathBuf, Db);

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("kaluta-scheduled-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("mail.sqlite")).unwrap();
        Store(dir, db)
    }

    fn a_draft(db: &Db, subject: &str) -> i64 {
        let d = DraftRecord { subject: subject.into(), ..Default::default() };
        db.write_blocking(move |tx| drafts::save(tx, &d, NOW)).unwrap()
    }

    #[test]
    fn a_draft_is_scheduled_changed_and_taken_back() {
        let Store(_, db) = &store("set");
        let id = a_draft(db, "Later");
        assert_eq!(db.write_blocking(move |tx| set(tx, id, Some(NOW + HOUR), NOW)).unwrap(), None);
        assert_eq!(db.read_blocking(move |c| get(c, id)).unwrap(), Some(NOW + HOUR));
        // Changed: the time it had comes back, for undo.
        assert_eq!(db.write_blocking(move |tx| set(tx, id, Some(NOW + 2 * HOUR), NOW)).unwrap(), Some(NOW + HOUR));
        assert_eq!(db.read_blocking(list).unwrap(), vec![ScheduledSend { draft_id: id, send_at: NOW + 2 * HOUR }]);
        // Taken back: an ordinary draft again.
        assert_eq!(db.write_blocking(move |tx| set(tx, id, None, NOW)).unwrap(), Some(NOW + 2 * HOUR));
        assert!(db.read_blocking(list).unwrap().is_empty());
        assert!(db.read_blocking(move |c| drafts::get(c, id)).unwrap().is_some(), "the draft is kept");
    }

    #[test]
    fn only_a_draft_being_edited_is_scheduled_or_due() {
        let Store(_, db) = &store("due");
        let (soon, later) = (a_draft(db, "Soon"), a_draft(db, "Later"));
        db.write_blocking(move |tx| set(tx, later, Some(NOW + 3 * HOUR), NOW)).unwrap();
        db.write_blocking(move |tx| set(tx, soon, Some(NOW + HOUR), NOW)).unwrap();
        assert!(db.read_blocking(|c| due(c, NOW)).unwrap().is_empty(), "not yet");
        assert_eq!(
            db.read_blocking(|c| due(c, NOW + 2 * HOUR)).unwrap().iter().map(|s| s.draft_id).collect::<Vec<_>>(),
            [soon]
        );
        assert_eq!(
            db.read_blocking(|c| due(c, NOW + 4 * HOUR)).unwrap().iter().map(|s| s.draft_id).collect::<Vec<_>>(),
            [soon, later],
            "soonest first"
        );
        // Being sent already: not due again, and not to be scheduled.
        db.write_blocking(move |tx| drafts::set_state(tx, soon, DraftState::Sending, None)).unwrap();
        assert_eq!(
            db.read_blocking(|c| due(c, NOW + 4 * HOUR)).unwrap().iter().map(|s| s.draft_id).collect::<Vec<_>>(),
            [later]
        );
        let err = db.write_blocking(move |tx| set(tx, soon, Some(NOW + HOUR), NOW)).unwrap_err();
        assert!(matches!(err, StoreError::Invalid(_)));
        let err = db.write_blocking(move |tx| set(tx, 9_999, Some(NOW + HOUR), NOW)).unwrap_err();
        assert!(matches!(err, StoreError::NotFound(_)));
        // A deleted draft takes its time with it.
        db.write_blocking(move |tx| drafts::delete(tx, later)).unwrap();
        assert_eq!(db.read_blocking(move |c| get(c, later)).unwrap(), None);
    }
}
