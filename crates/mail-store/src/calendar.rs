//! Calendar access and holds (spec §14.9, ADR 0018).

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::StoreResult;

/// What the account's sign-in grants of the calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarAccess {
    pub free_busy: bool,
    pub holds: bool,
    /// `["primary"]`.
    pub calendars_json: String,
    pub connected_at: Millis,
}

/// A hold Kaluta made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoldRow {
    pub id: i64,
    pub event_id: String,
    pub title: String,
    pub start_at: Millis,
    pub end_at: Millis,
    pub thread_id: Option<String>,
    pub created_at: Millis,
    pub released_at: Option<Millis>,
}

pub fn access(conn: &Connection) -> StoreResult<Option<CalendarAccess>> {
    Ok(conn
        .query_row("SELECT free_busy, holds, calendars_json, connected_at FROM calendar_access WHERE id = 1", [], |r| {
            Ok(CalendarAccess {
                free_busy: r.get(0)?,
                holds: r.get(1)?,
                calendars_json: r.get(2)?,
                connected_at: r.get(3)?,
            })
        })
        .optional()?)
}

/// Record the access granted, keeping the calendars chosen.
pub fn set_access(tx: &Transaction, free_busy: bool, holds: bool, now: Millis) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO calendar_access (id, free_busy, holds, connected_at) VALUES (1, ?1, ?2, ?3)
         ON CONFLICT (id) DO UPDATE SET free_busy = excluded.free_busy, holds = excluded.holds,
           connected_at = excluded.connected_at",
        params![free_busy, holds, now],
    )?;
    Ok(())
}

/// The calendars whose busy times count.
pub fn set_calendars(tx: &Transaction, calendars_json: &str) -> StoreResult<()> {
    tx.execute("UPDATE calendar_access SET calendars_json = ?1 WHERE id = 1", [calendars_json])?;
    Ok(())
}

/// Not connected any more.
pub fn clear_access(tx: &Transaction) -> StoreResult<()> {
    tx.execute("DELETE FROM calendar_access", [])?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn add_hold(
    tx: &Transaction,
    event_id: &str,
    title: &str,
    start_at: Millis,
    end_at: Millis,
    thread_id: Option<&str>,
    now: Millis,
) -> StoreResult<i64> {
    tx.execute(
        "INSERT INTO calendar_holds (event_id, title, start_at, end_at, thread_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![event_id, title, start_at, end_at, thread_id, now],
    )?;
    Ok(tx.last_insert_rowid())
}

pub fn hold(conn: &Connection, id: i64) -> StoreResult<Option<HoldRow>> {
    Ok(conn
        .query_row(
            "SELECT id, event_id, title, start_at, end_at, thread_id, created_at, released_at
               FROM calendar_holds WHERE id = ?1",
            [id],
            row,
        )
        .optional()?)
}

/// Holds not released whose end is after `now`, soonest first.
pub fn active_holds(conn: &Connection, now: Millis) -> StoreResult<Vec<HoldRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, event_id, title, start_at, end_at, thread_id, created_at, released_at
           FROM calendar_holds WHERE released_at IS NULL AND end_at > ?1 ORDER BY start_at",
    )?;
    Ok(stmt.query_map([now], row)?.collect::<Result<_, _>>()?)
}

/// Released: whether it was held until now.
pub fn release_hold(tx: &Transaction, id: i64, now: Millis) -> StoreResult<bool> {
    Ok(tx.execute(
        "UPDATE calendar_holds SET released_at = ?2 WHERE id = ?1 AND released_at IS NULL",
        params![id, now],
    )? > 0)
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<HoldRow> {
    Ok(HoldRow {
        id: r.get(0)?,
        event_id: r.get(1)?,
        title: r.get(2)?,
        start_at: r.get(3)?,
        end_at: r.get(4)?,
        thread_id: r.get(5)?,
        created_at: r.get(6)?,
        released_at: r.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    #[test]
    fn access_is_recorded_and_holds_are_released_once() {
        let dir = std::env::temp_dir().join(format!("kaluta-calendar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("mail.sqlite")).unwrap();
        assert!(db.read_blocking(access).unwrap().is_none(), "not connected until the user connects");
        db.write_blocking(|tx| set_access(tx, true, false, 5)).unwrap();
        db.write_blocking(|tx| set_calendars(tx, r#"["primary","team@example.com"]"#)).unwrap();
        db.write_blocking(|tx| set_access(tx, true, true, 9)).unwrap();
        let a = db.read_blocking(access).unwrap().unwrap();
        assert_eq!((a.free_busy, a.holds, a.connected_at), (true, true, 9));
        assert_eq!(a.calendars_json, r#"["primary","team@example.com"]"#, "connecting again keeps the choice");

        let id = db.write_blocking(|tx| add_hold(tx, "ev1", "Hold: Launch", 100, 200, Some("t1"), 10)).unwrap();
        db.write_blocking(|tx| add_hold(tx, "ev0", "Hold: Past", 10, 50, None, 10)).unwrap();
        let active = db.read_blocking(|c| active_holds(c, 60)).unwrap();
        assert_eq!(active.iter().map(|h| h.event_id.as_str()).collect::<Vec<_>>(), ["ev1"], "the past one is over");
        assert!(db.write_blocking(move |tx| release_hold(tx, id, 70)).unwrap());
        assert!(!db.write_blocking(move |tx| release_hold(tx, id, 80)).unwrap(), "once");
        assert_eq!(db.read_blocking(move |c| hold(c, id)).unwrap().unwrap().released_at, Some(70));
        assert!(db.read_blocking(|c| active_holds(c, 60)).unwrap().is_empty());
        db.write_blocking(clear_access).unwrap();
        assert!(db.read_blocking(access).unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
