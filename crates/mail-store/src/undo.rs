//! The user's recent mail actions, recorded so they can be undone exactly
//! (spec §14.6a). Each records, per message, the labels the action really
//! added and removed: a thread that was already archived is not moved to
//! the Inbox by undoing "Archive". Per account, since each account has its
//! own store.

use std::collections::BTreeMap;

use mail_domain::{LabelId, MessageId, Millis};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::error::StoreResult;
use crate::outbox::OutboxOp;

/// Actions kept per account.
pub const KEEP: i64 = 50;

/// What one action changed on one message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageDiff {
    pub message: MessageId,
    pub added: Vec<LabelId>,
    pub removed: Vec<LabelId>,
}

impl MessageDiff {
    /// The change that reverses this one.
    pub fn inverse(&self) -> Self {
        Self { message: self.message.clone(), added: self.removed.clone(), removed: self.added.clone() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoAction {
    pub id: i64,
    /// "archive", "trash", "read", … (for the UI's wording).
    pub kind: String,
    pub diffs: Vec<MessageDiff>,
}

/// Record an action and forget all but the last [`KEEP`]. Returns its id.
pub fn record(tx: &Transaction<'_>, kind: &str, diffs: &[MessageDiff], now: Millis) -> StoreResult<i64> {
    tx.prepare_cached("INSERT INTO undo_actions (kind, diffs_json, created_at) VALUES (?1, ?2, ?3)")?
        .execute(params![kind, serde_json::to_string(diffs)?, now])?;
    let id = tx.last_insert_rowid();
    tx.prepare_cached("DELETE FROM undo_actions WHERE id <= ?1")?.execute([id - KEEP])?;
    Ok(id)
}

pub fn get(conn: &Connection, id: i64) -> StoreResult<Option<UndoAction>> {
    let row: Option<(String, String)> = conn
        .prepare_cached("SELECT kind, diffs_json FROM undo_actions WHERE id = ?1")?
        .query_row([id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    match row {
        Some((kind, json)) => Ok(Some(UndoAction { id, kind, diffs: serde_json::from_str(&json)? })),
        None => Ok(None),
    }
}

/// Provider ops for exact per-message diffs: messages with the same change
/// share an op, and Trash comes and goes through the trash endpoints.
pub fn provider_ops(tx: &Transaction<'_>, diffs: &[MessageDiff]) -> StoreResult<Vec<OutboxOp>> {
    let trash = LabelId::new(mail_domain::system_labels::TRASH);
    let inbox = LabelId::new(mail_domain::system_labels::INBOX);
    let mut groups: BTreeMap<(Vec<LabelId>, Vec<LabelId>), Vec<MessageId>> = BTreeMap::new();
    for d in diffs.iter().filter(|d| !d.added.is_empty() || !d.removed.is_empty()) {
        groups.entry((d.added.clone(), d.removed.clone())).or_default().push(d.message.clone());
    }
    let mut ops = Vec::new();
    for ((mut add, mut remove), message_ids) in groups {
        if remove.contains(&trash) {
            remove.retain(|l| *l != trash);
            ops.push(OutboxOp::Untrash { message_ids: message_ids.clone() });
        }
        if add.contains(&trash) {
            // Trashing also takes a message out of the Inbox.
            add.retain(|l| *l != trash);
            remove.retain(|l| *l != inbox);
            // Read before the change is applied: what a failure restores.
            let mut previous = Vec::with_capacity(message_ids.len());
            for m in &message_ids {
                previous.push((m.clone(), crate::outbox::current_labels(tx, m)?));
            }
            ops.push(OutboxOp::Trash { message_ids: message_ids.clone(), previous });
        }
        if !add.is_empty() || !remove.is_empty() {
            ops.push(OutboxOp::ModifyLabels { message_ids, add, remove });
        }
    }
    Ok(ops)
}
