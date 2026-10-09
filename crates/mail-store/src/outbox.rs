//! Local mutations waiting to reach the provider (spec §7.4). A mutation is
//! applied to the store and queued here in the same transaction, so the UI
//! (which reads the store) and the queue never disagree. Each op records
//! exactly the messages it changed, so a permanent failure can be undone.
//!
//! Several drainers (the app, the headless MCP) may share one store: ops
//! are claimed one at a time, in order, by a named [`Claimant`] with a
//! lease ([`claim_next`], [`recover_in_flight`]; spec §7.4, outbox claims).

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mail_domain::{EmailAddress, LabelId, MessageId, Millis};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::error::StoreResult;
use crate::write::{MailWriter, ThreadChanges};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutboxOp {
    /// Add/remove labels on these messages (archive, read, star, label).
    ModifyLabels { message_ids: Vec<MessageId>, add: Vec<LabelId>, remove: Vec<LabelId> },
    /// Move these messages to Trash.
    Trash { message_ids: Vec<MessageId>, previous: Vec<(MessageId, Vec<LabelId>)> },
    /// Take these messages out of Trash (undoing a trash).
    Untrash { message_ids: Vec<MessageId> },
    /// Send a frozen message. `local_message_id` is the optimistic copy
    /// shown in Sent until the real one syncs back.
    Send {
        draft_id: i64,
        /// RFC 5322 bytes, base64.
        raw: String,
        thread_id: Option<mail_domain::ThreadId>,
        local_message_id: MessageId,
    },
    /// Mirror a local draft to the server's drafts (create or replace).
    /// Reads the draft when it runs, so one op covers any number of edits.
    SyncDraft { draft_id: i64, from: EmailAddress },
    /// Delete a server draft (discarded, or sent as a new message).
    DeleteDraft { gmail_draft_id: String },
}

impl OutboxOp {
    fn kind(&self) -> &'static str {
        match self {
            Self::ModifyLabels { .. } => "modify_labels",
            Self::Trash { .. } => "trash",
            Self::Untrash { .. } => "untrash",
            Self::Send { .. } => "send",
            Self::SyncDraft { .. } => "sync_draft",
            Self::DeleteDraft { .. } => "delete_draft",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedOp {
    pub id: i64,
    pub op: OutboxOp,
    pub attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutboxCounts {
    pub pending: u32,
    pub failed: u32,
}

pub fn enqueue(tx: &Transaction<'_>, op: &OutboxOp, now: Millis) -> StoreResult<i64> {
    enqueue_held(tx, op, now, None)
}

/// Queue an op that must not run before `not_before` (a send held for
/// Undo Send, spec §14.6a).
pub fn enqueue_held(tx: &Transaction<'_>, op: &OutboxOp, now: Millis, not_before: Option<Millis>) -> StoreResult<i64> {
    tx.prepare_cached("INSERT INTO outbox (kind, payload_json, created_at, next_attempt_at) VALUES (?1, ?2, ?3, ?4)")?
        .execute(params![op.kind(), serde_json::to_string(op)?, now, not_before])?;
    Ok(tx.last_insert_rowid())
}

/// How long a claim on an in-flight op lasts unless renewed. The drainer
/// renews it every [`LEASE_RENEW_MS`] while the provider call runs.
pub const LEASE_MS: Millis = 60_000;
/// How often a drainer renews its claim while the provider call runs.
pub const LEASE_RENEW_MS: Millis = 15_000;

/// Where claimants keep their lock files, next to the store.
const CLAIMS_DIR: &str = "outbox-claims";
/// Prefix of a claimant that could not make its lock file: whether it is
/// alive cannot be told, so only its lease ends its claims.
const NO_LOCK: &str = "nolock-";

/// One outbox drainer (spec §7.4, outbox claims): the app's sync engine
/// for an account, or a second process (the headless MCP) draining the
/// same store. Its claims name it. While it lives it holds an exclusive
/// `flock` on `outbox-claims/<id>.lock` beside the store; the kernel
/// releases that lock when the process dies, however it dies, so another
/// drainer can tell a dead claimant from a slow one at once, and a reused
/// pid cannot fool it.
#[derive(Debug)]
pub struct Claimant {
    id: String,
    dir: Option<PathBuf>,
    /// The locked file and its path, removed on drop.
    lock: Option<(File, PathBuf)>,
}

impl Claimant {
    /// A new claimant for the store at `db_path`, unique among every
    /// process and every engine in it. Lock files left by dead claimants
    /// are cleared on the way.
    pub fn register(db_path: &Path) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
        let id = format!("{}-{nanos:x}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        let dir = db_path.parent().map(|p| p.join(CLAIMS_DIR)).filter(|d| std::fs::create_dir_all(d).is_ok());
        let Some(dir) = dir else {
            tracing::warn!("no directory for outbox claim locks; in-flight ops are recovered by lease only");
            return Self { id: format!("{NO_LOCK}{id}"), dir: None, lock: None };
        };
        sweep(&dir);
        let path = dir.join(format!("{id}.lock"));
        let locked = lock_own(&path);
        match locked {
            Ok(file) => Self { id, dir: Some(dir), lock: Some((file, path)) },
            Err(e) => {
                tracing::warn!(error = %e, "outbox claim lock failed; in-flight ops are recovered by lease only");
                Self { id: format!("{NO_LOCK}{id}"), dir: Some(dir), lock: None }
            }
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether the claimant `other` is certainly gone: its lock file is
    /// missing or no longer locked. `false` when it lives or cannot be
    /// told (then its lease decides).
    pub fn is_gone(&self, other: &str) -> bool {
        let Some(dir) = &self.dir else { return false };
        if other.starts_with(NO_LOCK) || other.is_empty() || other.contains(['/', '\\']) || other.contains("..") {
            return false;
        }
        let path = dir.join(format!("{other}.lock"));
        match File::options().read(true).write(true).open(&path) {
            Err(e) => e.kind() == std::io::ErrorKind::NotFound,
            Ok(file) => match file.try_lock() {
                // Unlocked: dead, and its file is removed, if the name still
                // means the file locked here. A new file under the name is
                // a claimant still registering: not gone.
                Ok(()) if same_file(&path, &file) => {
                    let _ = std::fs::remove_file(&path);
                    true
                }
                Ok(()) => std::fs::symlink_metadata(&path).is_err(),
                Err(_) => false,
            },
        }
    }
}

/// Whether `path` still names the very file `file` has open.
fn same_file(path: &Path, file: &File) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(path), file.metadata()) {
        (Ok(named), Ok(open)) => named.dev() == open.dev() && named.ino() == open.ino(),
        _ => false,
    }
}

/// Remove the lock file at `path`, which `file` holds locked, only while
/// the name still refers to it: never a newer file of the same name.
fn remove_if_same(path: &Path, file: &File) {
    if same_file(path, file) {
        let _ = std::fs::remove_file(path);
    }
}

/// Create and lock this claimant's own lock file. A sweep may lock and
/// remove a file between its creation and our lock (it looks like a dead
/// claimant's for that instant), so after locking, the name must still
/// refer to the file we locked; if not (or the sweep holds it), start
/// again with a fresh file.
fn lock_own(path: &Path) -> std::io::Result<File> {
    let mut last = std::io::Error::other("lock file kept being swept");
    for _ in 0..100 {
        let file = File::options().read(true).write(true).create(true).truncate(false).open(path)?;
        match file.try_lock() {
            Ok(()) if same_file(path, &file) => return Ok(file),
            Ok(()) => {}
            Err(e) => last = std::io::Error::from(e),
        }
        drop(file);
        std::thread::yield_now();
    }
    Err(last)
}

impl Drop for Claimant {
    fn drop(&mut self) {
        if let Some((_, path)) = &self.lock {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Remove the lock files of dead claimants: only one this sweep holds
/// locked itself, and only while its name still refers to that file.
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for path in entries.flatten().map(|e| e.path()) {
        if path.extension().is_some_and(|e| e == "lock")
            && let Ok(file) = File::options().read(true).write(true).open(&path)
            && file.try_lock().is_ok()
        {
            remove_if_same(&path, &file);
        }
    }
}

/// What [`claim_next`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// This op is now in flight, claimed by the caller.
    Ready(QueuedOp),
    /// Nothing is ready (empty, held, or waiting on a retry).
    Idle,
    /// Another drainer has an op in flight: nothing may pass it, since
    /// the outbox runs strictly in order.
    Busy,
}

/// Take the next op for sending, in the same write transaction that picks
/// it: the oldest ready op ([`next_ready`]), and only when no op is in
/// flight, so two drainers (two processes on one store) never send ops
/// out of order or one op twice. The claim names `claimant` and lasts
/// until `lease_until` unless renewed ([`renew`]).
pub fn claim_next(tx: &Transaction<'_>, claimant: &str, now: Millis, lease_until: Millis) -> StoreResult<Claim> {
    if tx.prepare_cached("SELECT 1 FROM outbox WHERE state = 'in_flight'")?.exists([])? {
        return Ok(Claim::Busy);
    }
    let Some(op) = next_ready(tx, now)? else { return Ok(Claim::Idle) };
    let claimed = tx.execute(
        "UPDATE outbox SET state = 'in_flight', claimed_by = ?2, lease_until = ?3 WHERE id = ?1 AND state = 'pending'",
        params![op.id, claimant, lease_until],
    )?;
    Ok(if claimed == 1 { Claim::Ready(op) } else { Claim::Idle })
}

/// Extend `claimant`'s claim on op `id`: `false` if it no longer holds it
/// (its lease ran out and another drainer took the op back).
pub fn renew(tx: &Transaction<'_>, id: i64, claimant: &str, lease_until: Millis) -> StoreResult<bool> {
    Ok(tx.execute(
        "UPDATE outbox SET lease_until = ?3 WHERE id = ?1 AND state = 'in_flight' AND claimed_by = ?2",
        params![id, claimant, lease_until],
    )? == 1)
}

/// Whether `claimant` still holds op `id` in flight. A drainer records the
/// outcome of its call only while it does.
pub fn holds(conn: &Connection, id: i64, claimant: &str) -> StoreResult<bool> {
    Ok(conn
        .prepare_cached("SELECT 1 FROM outbox WHERE id = ?1 AND state = 'in_flight' AND claimed_by = ?2")?
        .exists(params![id, claimant])?)
}

/// Whether any op is in flight (here or in another process).
pub fn in_flight(conn: &Connection) -> StoreResult<bool> {
    Ok(conn.prepare_cached("SELECT 1 FROM outbox WHERE state = 'in_flight'")?.exists([])?)
}

/// Ops left in flight by an interrupted drain are pending again: `me`'s
/// own (called with its drain lock held, when none of its ops can be in
/// flight), those of claimants that are gone ([`Claimant::is_gone`]) or
/// whose lease ran out, and those claimed by no one (left by a build from
/// before claims). A send so returned may have reached the provider: its
/// attempt is counted, and a send tried before is looked for at the
/// provider before it goes again (`mail_sync`'s drain).
pub fn recover_in_flight(tx: &Transaction<'_>, me: &Claimant, now: Millis) -> StoreResult<usize> {
    let rows: Vec<(i64, Option<String>, Option<Millis>)> = tx
        .prepare_cached("SELECT id, claimed_by, lease_until FROM outbox WHERE state = 'in_flight'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let mut released = 0;
    for (id, by, lease_until) in rows {
        let release = match by.as_deref() {
            None => true,
            Some(by) if by == me.id() => true,
            Some(by) => lease_until.is_none_or(|l| l <= now) || me.is_gone(by),
        };
        if release {
            tracing::info!(id, claimant = by.as_deref().unwrap_or("-"), "outbox op left in flight is pending again");
            released += tx.execute(
                "UPDATE outbox SET state = 'pending', claimed_by = NULL, lease_until = NULL,
                   attempts = attempts + (kind = 'send')
                 WHERE id = ?1 AND state = 'in_flight'",
                [id],
            )?;
        }
    }
    Ok(released)
}

/// Undo Send: take a held send back before it goes. Removes the
/// optimistic Sent copy and returns the draft to editing. `None` if the
/// send has already started (or finished).
/// Only a send never tried and still within its hold: once Gmail has been
/// asked, it may have accepted the message.
pub fn cancel_send(tx: &Transaction<'_>, draft_id: i64, now: Millis) -> StoreResult<Option<ThreadChanges>> {
    let row: Option<(i64, String)> = tx
        .query_row(
            "SELECT id, payload_json FROM outbox WHERE kind = 'send' AND state = 'pending'
               AND attempts = 0 AND next_attempt_at > ?2
               AND json_extract(payload_json, '$.draft_id') = ?1",
            params![draft_id, now],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((id, json)) = row else { return Ok(None) };
    let OutboxOp::Send { local_message_id, .. } = serde_json::from_str(&json)? else { return Ok(None) };
    tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
    let mut w = MailWriter::new(tx);
    w.delete_message(&local_message_id)?;
    let changes = w.finish()?;
    crate::drafts::set_state(tx, draft_id, crate::drafts::DraftState::Editing, None)?;
    // That Message-ID never went out: the AI composition is matched again
    // by thread or recipient, or by the next send's id (spec §14.10).
    crate::compositions::draft_unsent(tx, draft_id)?;
    Ok(Some(changes))
}

/// When a draft's send stops being held for Undo Send, if it is still
/// held at `now` (never attempted, its time not come): what an agent's
/// approval card counts down to (spec §14.6a).
pub fn send_held_until(conn: &Connection, draft_id: i64, now: Millis) -> StoreResult<Option<Millis>> {
    Ok(conn
        .query_row(
            "SELECT next_attempt_at FROM outbox WHERE kind = 'send' AND state = 'pending'
               AND attempts = 0 AND next_attempt_at > ?2
               AND json_extract(payload_json, '$.draft_id') = ?1",
            params![draft_id, now],
            |r| r.get(0),
        )
        .optional()?)
}

/// Sends still held for Undo Send at `now`.
pub fn held_sends(conn: &Connection, now: Millis) -> StoreResult<u32> {
    Ok(conn
        .query_row(
            "SELECT COUNT(*) FROM outbox WHERE kind = 'send' AND state = 'pending' AND attempts = 0
           AND next_attempt_at > ?1",
            [now],
            |r| r.get::<_, i64>(0),
        )?
        .max(0) as u32)
}

/// Sends not yet handed to Gmail that quitting should wait for: held,
/// due, or being sent right now (not ones waiting to retry after an
/// error; those go at next launch).
pub fn unsent_sends(conn: &Connection) -> StoreResult<u32> {
    Ok(conn
        .query_row(
            "SELECT COUNT(*) FROM outbox WHERE kind = 'send'
           AND ((state = 'pending' AND attempts = 0) OR state = 'in_flight')",
            [],
            |r| r.get::<_, i64>(0),
        )?
        .max(0) as u32)
}

/// Let every held send go now (the app is quitting, spec §14.6a).
pub fn release_held_sends(tx: &Transaction<'_>, now: Millis) -> StoreResult<usize> {
    Ok(tx.execute(
        "UPDATE outbox SET next_attempt_at = NULL WHERE kind = 'send' AND state = 'pending' AND attempts = 0
           AND next_attempt_at > ?1",
        [now],
    )?)
}

/// The oldest pending op, strictly in order: if it is waiting on a retry,
/// nothing runs yet, since a later op (an undo, an unarchive) must not
/// reach the server before the one it follows. Sends still held for Undo
/// Send are the exception: they step aside until their time.
pub fn next_ready(conn: &Connection, now: Millis) -> StoreResult<Option<QueuedOp>> {
    let row: Option<(i64, String, i64, Option<Millis>)> = conn
        .prepare_cached(
            "SELECT id, payload_json, attempts, next_attempt_at FROM outbox
             WHERE state = 'pending'
               AND NOT (kind = 'send' AND attempts = 0 AND next_attempt_at IS NOT NULL AND next_attempt_at > ?1)
             ORDER BY id LIMIT 1",
        )?
        .query_row([now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .optional()?;
    match row {
        Some((_, _, _, Some(at))) if at > now => Ok(None),
        Some((id, json, attempts, _)) => {
            Ok(Some(QueuedOp { id, op: serde_json::from_str(&json)?, attempts: attempts.max(0) as u32 }))
        }
        None => Ok(None),
    }
}

/// When the next pending op becomes ready, if any is waiting on a retry.
pub fn next_retry_at(conn: &Connection) -> StoreResult<Option<Millis>> {
    Ok(conn.query_row("SELECT MIN(next_attempt_at) FROM outbox WHERE state = 'pending'", [], |r| r.get(0))?)
}

/// The provider accepted the op. A sent draft is deleted now, along with
/// its server copy.
pub fn complete(tx: &Transaction<'_>, id: i64) -> StoreResult<()> {
    let row: Option<(String, Millis)> = tx
        .query_row("SELECT payload_json, created_at FROM outbox WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    if let Some((json, created_at)) = row
        && let OutboxOp::Send { draft_id, .. } = serde_json::from_str(&json)?
    {
        crate::drafts::discard(tx, draft_id, created_at)?;
    }
    tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
    Ok(())
}

pub fn retry_later(tx: &Transaction<'_>, id: i64, next_attempt_at: Millis, error: &str) -> StoreResult<()> {
    tx.execute(
        "UPDATE outbox SET state = 'pending', attempts = attempts + 1, next_attempt_at = ?2, last_error = ?3,
           claimed_by = NULL, lease_until = NULL
         WHERE id = ?1",
        params![id, next_attempt_at, error],
    )?;
    Ok(())
}

/// Give up on an op: undo its local effect and keep it as `failed` so the
/// user can see what did not reach the server.
pub fn fail(tx: &Transaction<'_>, id: i64, error: &str) -> StoreResult<ThreadChanges> {
    let json: String = tx.query_row("SELECT payload_json FROM outbox WHERE id = ?1", [id], |r| r.get(0))?;
    let op: OutboxOp = serde_json::from_str(&json)?;
    let mut w = MailWriter::new(tx);
    match &op {
        OutboxOp::ModifyLabels { message_ids, add, remove } => {
            for m in message_ids {
                w.modify_message_labels(m, remove, add)?;
            }
        }
        OutboxOp::Trash { previous, .. } => {
            for (m, labels) in previous {
                let current = current_labels(tx, m)?;
                w.modify_message_labels(m, labels, &current)?;
            }
        }
        OutboxOp::Untrash { message_ids } => {
            let trash = [LabelId::new(mail_domain::system_labels::TRASH)];
            for m in message_ids {
                w.modify_message_labels(m, &trash, &[])?;
            }
        }
        // The optimistic Sent copy goes; the draft comes back with the error.
        OutboxOp::Send { draft_id, local_message_id, .. } => {
            w.delete_message(local_message_id)?;
            crate::drafts::set_state(tx, *draft_id, crate::drafts::DraftState::Failed, Some(error))?;
        }
        // Nothing local to undo: the local draft is the source of truth.
        OutboxOp::SyncDraft { .. } | OutboxOp::DeleteDraft { .. } => {}
    }
    let changes = w.finish()?;
    tx.execute(
        "UPDATE outbox SET state = 'failed', last_error = ?2, claimed_by = NULL, lease_until = NULL WHERE id = ?1",
        params![id, error],
    )?;
    Ok(changes)
}

/// Whether a mirror op for this draft is already waiting.
pub fn has_pending_draft_sync(tx: &Transaction<'_>, draft_id: i64) -> StoreResult<bool> {
    Ok(tx
        .prepare_cached(
            "SELECT 1 FROM outbox WHERE kind = 'sync_draft' AND state = 'pending'
               AND json_extract(payload_json, '$.draft_id') = ?1",
        )?
        .exists([draft_id])?)
}

pub fn counts(conn: &Connection) -> StoreResult<OutboxCounts> {
    let (pending, failed): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(state = 'pending'), 0), COALESCE(SUM(state = 'failed'), 0) FROM outbox",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(OutboxCounts { pending: pending.max(0) as u32, failed: failed.max(0) as u32 })
}

/// Drop failed ops the user has acknowledged.
pub fn clear_failed(tx: &Transaction<'_>) -> StoreResult<usize> {
    Ok(tx.execute("DELETE FROM outbox WHERE state = 'failed'", [])?)
}

pub fn current_labels(tx: &Transaction<'_>, m: &MessageId) -> StoreResult<Vec<LabelId>> {
    Ok(tx
        .prepare_cached(
            "SELECT l.gmail_id FROM message_labels ml JOIN labels l ON l.id = ml.label_id
             JOIN messages m ON m.id = ml.message_id WHERE m.gmail_id = ?1",
        )?
        .query_map([m.as_str()], |r| Ok(LabelId(r.get(0)?)))?
        .collect::<Result<_, _>>()?)
}

/// What a thread-level label change actually touches: the messages of these
/// threads on which it would alter labels, with their current labels.
pub fn affected_messages(
    tx: &Transaction<'_>,
    thread_ids: &[mail_domain::ThreadId],
    add: &[LabelId],
    remove: &[LabelId],
) -> StoreResult<Vec<(MessageId, Vec<LabelId>)>> {
    let mut out = Vec::new();
    let mut messages = tx.prepare_cached(
        "SELECT m.gmail_id FROM messages m JOIN threads t ON t.id = m.thread_id WHERE t.gmail_id = ?1 ORDER BY m.id",
    )?;
    for t in thread_ids {
        let ids: Vec<MessageId> =
            messages.query_map([t.as_str()], |r| Ok(MessageId(r.get(0)?)))?.collect::<Result<_, _>>()?;
        for m in ids {
            let labels = current_labels(tx, &m)?;
            let changes = add.iter().any(|l| !labels.contains(l)) || remove.iter().any(|l| labels.contains(l));
            if changes {
                out.push((m, labels));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::Db;

    fn store(name: &str) -> (Db, PathBuf) {
        let dir = std::env::temp_dir().join(format!("kaluta-outbox-claims-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("mail.sqlite");
        (Db::open(&path).unwrap(), path)
    }

    fn label_op(n: usize) -> OutboxOp {
        OutboxOp::ModifyLabels {
            message_ids: vec![MessageId::new(format!("m{n}"))],
            add: vec![LabelId::new("STARRED")],
            remove: vec![],
        }
    }

    fn state(db: &Db, id: i64) -> (String, Option<String>, i64) {
        db.read_blocking(|c| {
            Ok(c.query_row("SELECT state, claimed_by, attempts FROM outbox WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?)
        })
        .unwrap()
    }

    #[test]
    fn one_op_is_in_flight_at_a_time_and_only_in_order() {
        let (db, path) = store("order");
        let (a, b) = (Claimant::register(&path), Claimant::register(&path));
        assert_ne!(a.id(), b.id());
        let first = db.write_blocking(|tx| enqueue(tx, &label_op(1), 1)).unwrap();
        db.write_blocking(|tx| enqueue(tx, &label_op(2), 1)).unwrap();
        let (ida, idb) = (a.id().to_owned(), b.id().to_owned());
        let claim = db.write_blocking(move |tx| claim_next(tx, &ida, 10, 100)).unwrap();
        assert!(matches!(claim, Claim::Ready(ref q) if q.id == first), "{claim:?}");
        // The second op waits behind the first, whoever asks.
        assert_eq!(db.write_blocking(move |tx| claim_next(tx, &idb, 10, 100)).unwrap(), Claim::Busy);
        assert_eq!(state(&db, first), ("in_flight".into(), Some(a.id().to_owned()), 0));
        let ida = a.id().to_owned();
        assert!(db.write_blocking(move |tx| renew(tx, first, &ida, 200)).unwrap());
        let idb = b.id().to_owned();
        assert!(!db.write_blocking(move |tx| renew(tx, first, &idb, 200)).unwrap(), "only the claimant renews");
        db.write_blocking(move |tx| complete(tx, first)).unwrap();
        let idb = b.id().to_owned();
        assert!(matches!(db.write_blocking(move |tx| claim_next(tx, &idb, 10, 100)).unwrap(), Claim::Ready(_)));
        let ida = a.id().to_owned();
        assert_eq!(db.write_blocking(move |tx| claim_next(tx, &ida, 10, 100)).unwrap(), Claim::Busy);
    }

    #[test]
    fn a_live_claimant_keeps_its_op_until_its_lease_runs_out() {
        let (db, path) = store("lease");
        let (owner, other) = (Arc::new(Claimant::register(&path)), Arc::new(Claimant::register(&path)));
        let id = db.write_blocking(|tx| enqueue(tx, &label_op(1), 1)).unwrap();
        let oid = owner.id().to_owned();
        db.write_blocking(move |tx| claim_next(tx, &oid, 10, 100)).unwrap();
        assert!(!other.is_gone(owner.id()));
        let o = other.clone();
        assert_eq!(db.write_blocking(move |tx| recover_in_flight(tx, &o, 99)).unwrap(), 0, "alive, lease running");
        let o = other.clone();
        assert_eq!(db.write_blocking(move |tx| recover_in_flight(tx, &o, 100)).unwrap(), 1, "lease ran out");
        assert_eq!(state(&db, id), ("pending".into(), None, 0), "a label change is not counted as an attempt");
    }

    #[test]
    fn a_dead_claimants_ops_come_back_at_once_and_a_send_counts_an_attempt() {
        let (db, path) = store("dead");
        let survivor = Arc::new(Claimant::register(&path));
        let dead = Claimant::register(&path);
        let send = OutboxOp::Send {
            draft_id: 1,
            raw: String::new(),
            thread_id: None,
            local_message_id: MessageId::new("local-1"),
        };
        let id = db.write_blocking(move |tx| enqueue(tx, &send, 1)).unwrap();
        let did = dead.id().to_owned();
        db.write_blocking(move |tx| claim_next(tx, &did, 10, i64::MAX)).unwrap();
        let dead_id = dead.id().to_owned();
        drop(dead);
        assert!(survivor.is_gone(&dead_id));
        let s = survivor.clone();
        assert_eq!(db.write_blocking(move |tx| recover_in_flight(tx, &s, 11)).unwrap(), 1);
        assert_eq!(state(&db, id), ("pending".into(), None, 1), "the send may have gone: checked before resending");
    }

    #[test]
    fn a_claimant_whose_process_died_is_gone_though_its_lock_file_stayed() {
        let (_db, path) = store("stale-file");
        let me = Claimant::register(&path);
        // A lock file nobody holds: what a killed process leaves.
        let dir = path.parent().unwrap().join(CLAIMS_DIR);
        std::fs::write(dir.join("4242-1-0.lock"), b"").unwrap();
        assert!(me.is_gone("4242-1-0"));
        assert!(!dir.join("4242-1-0.lock").exists(), "cleared");
        assert!(me.is_gone("never-registered"));
        assert!(!me.is_gone(me.id()), "a live one is not");
        assert!(!me.is_gone("nolock-1"), "one without a lock file is left to its lease");
        assert!(!me.is_gone("../escape"));
    }

    #[test]
    fn ops_left_in_flight_by_this_drainer_or_a_build_without_claims_come_back() {
        let (db, path) = store("own");
        let me = Arc::new(Claimant::register(&path));
        let mine = db.write_blocking(|tx| enqueue(tx, &label_op(1), 1)).unwrap();
        let legacy = db.write_blocking(|tx| enqueue(tx, &label_op(2), 1)).unwrap();
        let id = me.id().to_owned();
        db.write_blocking(move |tx| claim_next(tx, &id, 10, i64::MAX)).unwrap();
        db.write_blocking(move |tx| {
            tx.execute("UPDATE outbox SET state = 'in_flight' WHERE id = ?1", [legacy])?;
            Ok(())
        })
        .unwrap();
        let m = me.clone();
        assert_eq!(db.write_blocking(move |tx| recover_in_flight(tx, &m, 11)).unwrap(), 2);
        assert_eq!(state(&db, mine).0, "pending");
        assert_eq!(state(&db, legacy).0, "pending");
    }

    #[test]
    fn a_sweep_never_takes_a_new_claimants_lock_file() {
        // Claimants register while others sweep as hard as they can: every
        // claimant must end up holding a lock on the file its name points
        // to, so nobody can take it for gone while it lives.
        let (_db, path) = store("sweep-race");
        let dir = path.parent().unwrap().join(CLAIMS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        let observer = Claimant::register(&path);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sweepers: Vec<_> = (0..3)
            .map(|_| {
                let (dir, stop) = (dir.clone(), stop.clone());
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        sweep(&dir);
                    }
                })
            })
            .collect();
        let mut bad = Vec::new();
        for _ in 0..3000 {
            let c = Claimant::register(&path);
            if c.id().starts_with(NO_LOCK) {
                bad.push(format!("{} has no lock", c.id()));
            } else if observer.is_gone(c.id()) {
                bad.push(format!("{} taken for gone while alive", c.id()));
            }
        }
        stop.store(true, Ordering::Relaxed);
        for s in sweepers {
            s.join().unwrap();
        }
        assert!(bad.is_empty(), "{} of 3000: {:?}", bad.len(), &bad[..bad.len().min(5)]);
    }
}
