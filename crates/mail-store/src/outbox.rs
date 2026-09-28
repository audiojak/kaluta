//! Local mutations waiting to reach the provider (spec §7.4). A mutation is
//! applied to the store and queued here in the same transaction, so the UI
//! (which reads the store) and the queue never disagree. Each op records
//! exactly the messages it changed, so a permanent failure can be undone.

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

/// Take an op for sending: `false` if it is no longer pending (a held
/// send was cancelled in the meantime).
pub fn claim(tx: &Transaction<'_>, id: i64) -> StoreResult<bool> {
    Ok(tx.execute("UPDATE outbox SET state = 'in_flight' WHERE id = ?1 AND state = 'pending'", [id])? == 1)
}

/// Ops left in flight by an interrupted drain are pending again. Only
/// called with the drain lock held, when nothing can be in flight.
pub fn release_in_flight(tx: &Transaction<'_>) -> StoreResult<usize> {
    Ok(tx.execute("UPDATE outbox SET state = 'pending' WHERE state = 'in_flight'", [])?)
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
    Ok(Some(changes))
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
        "UPDATE outbox SET state = 'pending', attempts = attempts + 1, next_attempt_at = ?2, last_error = ?3
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
    tx.execute("UPDATE outbox SET state = 'failed', last_error = ?2 WHERE id = ?1", params![id, error])?;
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
