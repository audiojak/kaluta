//! The backfill queue (spec §7.4): message ids waiting for a full fetch,
//! drained in priority order (0 = most urgent).
//!
//! Priorities from [`HEADERS_ONLY`] up are the tiered-download amendment
//! (2026-09-27): those ids want headers only. The body backfill never
//! drains them; the headers pass stores their headers and drops them.

use mail_domain::MessageId;
use rusqlite::{Connection, Transaction, params};

use crate::error::StoreResult;

/// Queue priorities at or above this want headers only.
pub const HEADERS_ONLY: u8 = 10;

/// Queue ids at `priority`, keeping the more urgent priority if an id is
/// already queued. Ids whose message is already fully stored (or, at a
/// headers-only priority, stored at all) are skipped unless `refetch` is set.
pub fn enqueue(tx: &Transaction<'_>, priority: u8, ids: &[MessageId], refetch: bool) -> StoreResult<usize> {
    let mut exists = tx.prepare_cached(if priority >= HEADERS_ONLY {
        "SELECT 1 FROM messages WHERE gmail_id = ?1"
    } else {
        "SELECT 1 FROM messages WHERE gmail_id = ?1 AND body_state = 'full'"
    })?;
    let mut current = tx.prepare_cached("SELECT priority FROM backfill_queue WHERE gmail_id = ?1")?;
    let mut insert = tx.prepare_cached("INSERT INTO backfill_queue (priority, gmail_id) VALUES (?1, ?2)")?;
    let mut raise = tx.prepare_cached("UPDATE backfill_queue SET priority = ?1 WHERE gmail_id = ?2")?;
    let mut queued = 0;
    for id in ids {
        if !refetch && exists.exists([id.as_str()])? {
            continue;
        }
        let existing: Option<u8> = current.query_row([id.as_str()], |r| r.get(0)).ok();
        match existing {
            None => {
                insert.execute(params![priority, id.as_str()])?;
                queued += 1;
            }
            Some(p) if priority < p => {
                raise.execute(params![priority, id.as_str()])?;
            }
            Some(_) => {}
        }
    }
    Ok(queued)
}

/// The next `limit` ids, most urgent first. Does not remove them.
/// Queue ids at the front of the most urgent priority: mail the user just
/// touched elsewhere, or that just arrived, is fetched before any backlog.
/// Ids already fetched in full are skipped; queued ones move to the front.
pub fn enqueue_urgent(tx: &Transaction<'_>, ids: &[MessageId]) -> StoreResult<usize> {
    let mut exists = tx.prepare_cached("SELECT 1 FROM messages WHERE gmail_id = ?1 AND body_state = 'full'")?;
    let mut remove = tx.prepare_cached("DELETE FROM backfill_queue WHERE gmail_id = ?1")?;
    let mut insert = tx.prepare_cached(
        "INSERT INTO backfill_queue (seq, priority, gmail_id)
         VALUES ((SELECT COALESCE(MIN(seq), 0) - 1 FROM backfill_queue), 0, ?1)",
    )?;
    let mut queued = 0;
    // Inserted last-to-first so the caller's order is kept at the front.
    for id in ids.iter().rev() {
        if exists.exists([id.as_str()])? {
            continue;
        }
        remove.execute([id.as_str()])?;
        insert.execute([id.as_str()])?;
        queued += 1;
    }
    Ok(queued)
}

/// The next `limit` ids waiting for bodies, most urgent first. Does not
/// remove them; headers-only ids are never returned.
pub fn peek(conn: &Connection, limit: usize) -> StoreResult<Vec<MessageId>> {
    let mut stmt =
        conn.prepare_cached("SELECT gmail_id FROM backfill_queue WHERE priority < ?2 ORDER BY priority, seq LIMIT ?1")?;
    let ids =
        stmt.query_map(params![limit as i64, HEADERS_ONLY], |r| Ok(MessageId(r.get(0)?)))?.collect::<Result<_, _>>()?;
    Ok(ids)
}

/// What a headers pass should fetch, most urgent first: ids waiting for
/// bodies that have no row yet (so the list fills before bodies arrive),
/// then every headers-only id.
pub fn for_headers(conn: &Connection, limit: usize) -> StoreResult<Vec<MessageId>> {
    let mut stmt = conn.prepare_cached(
        "SELECT q.gmail_id FROM backfill_queue q
         WHERE q.priority >= ?2 OR NOT EXISTS (SELECT 1 FROM messages m WHERE m.gmail_id = q.gmail_id)
         ORDER BY q.priority, q.seq LIMIT ?1",
    )?;
    let ids =
        stmt.query_map(params![limit as i64, HEADERS_ONLY], |r| Ok(MessageId(r.get(0)?)))?.collect::<Result<_, _>>()?;
    Ok(ids)
}

/// Drop these ids if they are queued for headers only (their headers are
/// stored now); ids waiting for bodies stay.
/// Returns how many were dropped.
pub fn remove_headers_only(tx: &Transaction<'_>, ids: &[MessageId]) -> StoreResult<usize> {
    let mut stmt = tx.prepare_cached("DELETE FROM backfill_queue WHERE gmail_id = ?1 AND priority >= ?2")?;
    let mut dropped = 0;
    for id in ids {
        dropped += stmt.execute(params![id.as_str(), HEADERS_ONLY])?;
    }
    Ok(dropped)
}

/// Turn every headers-only id into a body fetch at its tier's priority
/// (the source can no longer fetch headers cheaply).
pub fn promote_headers_only(tx: &Transaction<'_>) -> StoreResult<usize> {
    Ok(tx.execute("UPDATE backfill_queue SET priority = priority - ?1 WHERE priority >= ?1", [HEADERS_ONLY])?)
}

pub fn remove(tx: &Transaction<'_>, ids: &[MessageId]) -> StoreResult<()> {
    let mut stmt = tx.prepare_cached("DELETE FROM backfill_queue WHERE gmail_id = ?1")?;
    for id in ids {
        stmt.execute([id.as_str()])?;
    }
    Ok(())
}

/// Drop everything queued at `priority` or lower urgency (a narrower sync
/// window); fetched mail is untouched.
pub fn clear_from_priority(tx: &Transaction<'_>, priority: u8) -> StoreResult<usize> {
    Ok(tx.execute("DELETE FROM backfill_queue WHERE priority >= ?1", [priority])?)
}

/// Queue priorities below this are the Inbox phases (unread, then the
/// rest of the Inbox; spec §7.4) and mail that just arrived.
pub const INBOX_PRIORITIES: u8 = 2;

/// Whether the Inbox is still filling: an id queued at an Inbox priority
/// has no row yet, so counting the Inbox now would come out short.
/// Header-only rows count as stored (their labels are known).
pub fn inbox_filling(conn: &Connection) -> StoreResult<bool> {
    Ok(conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM backfill_queue q WHERE q.priority < ?1
           AND NOT EXISTS (SELECT 1 FROM messages m WHERE m.gmail_id = q.gmail_id))",
        )?
        .query_row([INBOX_PRIORITIES], |r| r.get(0))?)
}

pub fn len(conn: &Connection) -> StoreResult<u64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM backfill_queue", [], |r| r.get::<_, i64>(0))?.max(0) as u64)
}

/// Queued (bodies, headers only).
pub fn counts(conn: &Connection) -> StoreResult<(u64, u64)> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(priority < ?1), 0), COALESCE(SUM(priority >= ?1), 0) FROM backfill_queue",
        [HEADERS_ONLY],
        |r| Ok((r.get::<_, i64>(0)?.max(0) as u64, r.get::<_, i64>(1)?.max(0) as u64)),
    )?)
}

/// Ids of the given ones that are not stored with a full body.
pub fn missing(conn: &Connection, ids: &[MessageId]) -> StoreResult<Vec<MessageId>> {
    let mut stmt = conn.prepare_cached("SELECT 1 FROM messages WHERE gmail_id = ?1 AND body_state = 'full'")?;
    let mut out = Vec::new();
    for id in ids {
        if !stmt.exists([id.as_str()])? {
            out.push(id.clone());
        }
    }
    Ok(out)
}
