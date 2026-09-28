//! Local mutations and the outbox drain (spec §7.4).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mail_domain::{EmailAddress, LabelId, MessageId, Millis, ThreadId, system_labels};
use mail_store::drafts::{self, DraftState};
use mail_store::outbox::{self, OutboxCounts, OutboxOp};
use mail_store::undo::{self, MessageDiff};
use mail_store::{Db, MailWriter, ThreadChanges};
use provider_api::{LabelOp, ProviderError};

use crate::engine::SyncEngine;
use crate::error::SyncResult;

/// Retries before an op is given up and rolled back.
pub const MAX_ATTEMPTS: u32 = 5;

/// A change the user (or an agent) makes to threads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalChange {
    Labels {
        thread_ids: Vec<ThreadId>,
        add: Vec<LabelId>,
        remove: Vec<LabelId>,
    },
    Trash {
        thread_ids: Vec<ThreadId>,
    },
    /// Exactly these per-message changes: undoing or redoing a recorded
    /// action (spec §14.6a). Applied as recorded, whatever changed since.
    Exact {
        diffs: Vec<MessageDiff>,
    },
}

impl LocalChange {
    pub fn archive(thread_ids: Vec<ThreadId>) -> Self {
        Self::Labels { thread_ids, add: vec![], remove: vec![LabelId::new(system_labels::INBOX)] }
    }
    pub fn move_to_inbox(thread_ids: Vec<ThreadId>) -> Self {
        Self::Labels { thread_ids, add: vec![LabelId::new(system_labels::INBOX)], remove: vec![] }
    }
    pub fn set_read(thread_ids: Vec<ThreadId>, read: bool) -> Self {
        let unread = vec![LabelId::new(system_labels::UNREAD)];
        if read {
            Self::Labels { thread_ids, add: vec![], remove: unread }
        } else {
            Self::Labels { thread_ids, add: unread, remove: vec![] }
        }
    }
    pub fn set_starred(thread_ids: Vec<ThreadId>, starred: bool) -> Self {
        let star = vec![LabelId::new(system_labels::STARRED)];
        if starred {
            Self::Labels { thread_ids, add: star, remove: vec![] }
        } else {
            Self::Labels { thread_ids, add: vec![], remove: star }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrainReport {
    pub sent: usize,
    pub retrying: usize,
    pub failed: usize,
}

pub fn now_millis() -> Millis {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as Millis).unwrap_or_default()
}

fn backoff(attempts: u32) -> Duration {
    Duration::from_secs(2u64.saturating_pow(attempts + 1).min(300))
}

/// What one message's labels become: `add` it lacked, `remove` it had.
fn diff_for(message: &MessageId, before: &[LabelId], add: &[LabelId], remove: &[LabelId]) -> MessageDiff {
    MessageDiff {
        message: message.clone(),
        added: add.iter().filter(|l| !before.contains(l)).cloned().collect(),
        removed: remove.iter().filter(|l| before.contains(l)).cloned().collect(),
    }
}

/// Apply a change to the store, queueing it for the provider when `queue`
/// is set, in one transaction. Used directly for accounts with no provider
/// (the demo mailbox) and through [`SyncEngine::apply_change`] otherwise.
pub async fn apply_local_change(db: &Db, change: LocalChange, queue: bool) -> SyncResult<ThreadChanges> {
    Ok(apply_local_change_recorded(db, change, queue, None).await?.0)
}

/// [`apply_local_change`], recording what it changed per message as an
/// undoable action of `record` kind ("archive", …) when given. Returns the
/// action's id, or `None` when nothing changed or nothing was recorded.
pub async fn apply_local_change_recorded(
    db: &Db,
    change: LocalChange,
    queue: bool,
    record: Option<String>,
) -> SyncResult<(ThreadChanges, Option<i64>)> {
    let now = now_millis();
    Ok(db
        .write(move |tx| {
            let (ops, diffs, changes) = match change {
                LocalChange::Labels { thread_ids, add, remove } => {
                    let affected = outbox::affected_messages(tx, &thread_ids, &add, &remove)?;
                    let mut w = MailWriter::new(tx);
                    for (m, _) in &affected {
                        w.modify_message_labels(m, &add, &remove)?;
                    }
                    let changes = w.finish()?;
                    let diffs: Vec<MessageDiff> =
                        affected.iter().map(|(m, before)| diff_for(m, before, &add, &remove)).collect();
                    let message_ids = affected.into_iter().map(|(m, _)| m).collect::<Vec<_>>();
                    let op = (!message_ids.is_empty()).then_some(OutboxOp::ModifyLabels { message_ids, add, remove });
                    (op.into_iter().collect::<Vec<_>>(), diffs, changes)
                }
                LocalChange::Trash { thread_ids } => {
                    let trash = vec![LabelId::new(system_labels::TRASH)];
                    let inbox = vec![LabelId::new(system_labels::INBOX)];
                    let affected = outbox::affected_messages(tx, &thread_ids, &trash, &[])?;
                    let mut w = MailWriter::new(tx);
                    for (m, _) in &affected {
                        w.modify_message_labels(m, &trash, &inbox)?;
                    }
                    let changes = w.finish()?;
                    let diffs: Vec<MessageDiff> =
                        affected.iter().map(|(m, before)| diff_for(m, before, &trash, &inbox)).collect();
                    let message_ids = affected.iter().map(|(m, _)| m.clone()).collect::<Vec<_>>();
                    let op = (!message_ids.is_empty()).then_some(OutboxOp::Trash { message_ids, previous: affected });
                    (op.into_iter().collect(), diffs, changes)
                }
                LocalChange::Exact { diffs } => {
                    let ops = undo::provider_ops(tx, &diffs)?;
                    let mut w = MailWriter::new(tx);
                    let mut applied = Vec::with_capacity(diffs.len());
                    for d in diffs {
                        // A message gone since (deleted on the server) is skipped.
                        if w.modify_message_labels(&d.message, &d.added, &d.removed)? {
                            applied.push(d);
                        }
                    }
                    let changes = w.finish()?;
                    let kept: Vec<&MessageId> = applied.iter().map(|d| &d.message).collect();
                    let ops = ops.into_iter().filter_map(|op| retain_messages(op, &kept)).collect();
                    (ops, applied, changes)
                }
            };
            if queue {
                for op in &ops {
                    outbox::enqueue(tx, op, now)?;
                }
            }
            let changed = diffs.iter().any(|d| !d.added.is_empty() || !d.removed.is_empty());
            let token = match record {
                Some(kind) if changed => Some(undo::record(tx, &kind, &diffs, now)?),
                _ => None,
            };
            Ok((changes, token))
        })
        .await?)
}

/// `op` limited to `kept` messages, or `None` if none is left.
fn retain_messages(op: OutboxOp, kept: &[&MessageId]) -> Option<OutboxOp> {
    let keep = |ids: Vec<MessageId>| ids.into_iter().filter(|m| kept.contains(&m)).collect::<Vec<_>>();
    let op = match op {
        OutboxOp::ModifyLabels { message_ids, add, remove } => {
            OutboxOp::ModifyLabels { message_ids: keep(message_ids), add, remove }
        }
        OutboxOp::Untrash { message_ids } => OutboxOp::Untrash { message_ids: keep(message_ids) },
        OutboxOp::Trash { message_ids, previous } => OutboxOp::Trash {
            message_ids: keep(message_ids),
            previous: previous.into_iter().filter(|(m, _)| kept.contains(&m)).collect(),
        },
        other => return Some(other),
    };
    match &op {
        OutboxOp::ModifyLabels { message_ids, .. }
        | OutboxOp::Untrash { message_ids }
        | OutboxOp::Trash { message_ids, .. }
            if message_ids.is_empty() =>
        {
            None
        }
        _ => Some(op),
    }
}

impl SyncEngine {
    /// Apply a change locally and queue it for the provider. Returns what
    /// changed for the UI (also published to the observer).
    pub async fn apply_change(&self, change: LocalChange, queue: bool) -> SyncResult<ThreadChanges> {
        let changes = apply_local_change(self.db(), change, queue).await?;
        self.publish_changes(&changes);
        Ok(changes)
    }

    /// [`SyncEngine::apply_change`], recorded as an undoable action.
    pub async fn apply_change_recorded(
        &self,
        change: LocalChange,
        record: Option<String>,
    ) -> SyncResult<(ThreadChanges, Option<i64>)> {
        let (changes, token) = apply_local_change_recorded(self.db(), change, true, record).await?;
        self.publish_changes(&changes);
        Ok((changes, token))
    }

    /// Send every ready op to the provider, oldest first. Transient failures
    /// are retried later with backoff; after [`MAX_ATTEMPTS`], or on a
    /// permanent error, the op is rolled back locally and marked failed.
    pub async fn drain_outbox(&self) -> SyncResult<DrainReport> {
        let _serialized = self.drain_lock.lock().await;
        self.db().write(outbox::release_in_flight).await?;
        let mut report = DrainReport::default();
        loop {
            let now = now_millis();
            let Some(queued) = self.db().read(move |c| outbox::next_ready(c, now)).await? else {
                return Ok(report);
            };
            // Claimed first: a held send cancelled since it was read stays
            // unsent (Undo Send).
            let claimed_id = queued.id;
            if !self.db().write(move |tx| outbox::claim(tx, claimed_id)).await? {
                continue;
            }
            let result = match &queued.op {
                OutboxOp::ModifyLabels { message_ids, add, remove } => {
                    // Before the call: history may report it before we return.
                    self.remember_own(message_ids, add, remove);
                    self.provider()
                        .modify_labels(&LabelOp {
                            message_ids: message_ids.clone(),
                            add: add.clone(),
                            remove: remove.clone(),
                        })
                        .await
                }
                OutboxOp::Trash { message_ids, .. } => {
                    self.remember_own(
                        message_ids,
                        &[LabelId::new(system_labels::TRASH)],
                        &[LabelId::new(system_labels::INBOX)],
                    );
                    let mut result = Ok(());
                    for m in message_ids {
                        if let Err(e) = self.provider().move_to_trash(m).await {
                            result = Err(e);
                            break;
                        }
                    }
                    result
                }
                OutboxOp::Untrash { message_ids } => {
                    self.remember_own(message_ids, &[], &[LabelId::new(system_labels::TRASH)]);
                    let mut result = Ok(());
                    for m in message_ids {
                        if let Err(e) = self.provider().restore_from_trash(m).await {
                            result = Err(e);
                            break;
                        }
                    }
                    result
                }
                OutboxOp::Send { raw, thread_id, .. } => match crate::compose::decode_raw(raw) {
                    Some(bytes) => self.provider().send(&bytes, thread_id.as_ref()).await.map(|_| ()),
                    None => Err(ProviderError::Invalid("queued message is corrupt".into())),
                },
                OutboxOp::SyncDraft { draft_id, from } => self.mirror_draft(*draft_id, from).await?,
                OutboxOp::DeleteDraft { gmail_draft_id } => self.provider().delete_draft(gmail_draft_id).await,
            };
            let id = queued.id;
            match result {
                Ok(()) => {
                    self.db().write(move |tx| outbox::complete(tx, id)).await?;
                    report.sent += 1;
                }
                // A message deleted on the server: nothing left to change.
                Err(ProviderError::NotFound(_)) if !matches!(queued.op, OutboxOp::Send { .. }) => {
                    self.db().write(move |tx| outbox::complete(tx, id)).await?;
                    report.sent += 1;
                }
                Err(e) if e.is_transient() && queued.attempts + 1 < MAX_ATTEMPTS => {
                    let at = now + backoff(queued.attempts).as_millis() as Millis;
                    let message = e.to_string();
                    self.db().write(move |tx| outbox::retry_later(tx, id, at, &message)).await?;
                    report.retrying += 1;
                    // Later ops wait: order matters (archive then unarchive).
                    return Ok(report);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "outbox op failed permanently; rolling back");
                    let message = e.to_string();
                    let changes = self.db().write(move |tx| outbox::fail(tx, id, &message)).await?;
                    self.publish_changes(&changes);
                    report.failed += 1;
                    if matches!(e, ProviderError::Unauthorized) {
                        return Err(e.into());
                    }
                }
            }
        }
    }

    /// Create or replace a draft's server copy. A draft deleted or being
    /// sent by now needs nothing; a server copy deleted elsewhere is
    /// recreated.
    async fn mirror_draft(&self, draft_id: i64, from: &EmailAddress) -> SyncResult<Result<(), ProviderError>> {
        let Some(draft) = self.db().read(move |c| drafts::get(c, draft_id)).await? else {
            return Ok(Ok(()));
        };
        if draft.state == DraftState::Sending {
            return Ok(Ok(()));
        }
        let existing = draft.gmail_draft_id.clone();
        let thread = draft.thread_id.clone().map(ThreadId);
        // A draft that cannot be built (an attachment file gone) fails this
        // op rather than stalling the queue behind it.
        let raw = match crate::compose::draft_raw(self.db(), draft, from).await {
            Ok(raw) => raw,
            Err(e) => return Ok(Err(ProviderError::Invalid(e.to_string()))),
        };
        let provider = self.provider();
        let saved = match provider.save_draft(existing.as_deref(), &raw, thread.as_ref()).await {
            Err(ProviderError::NotFound(_)) if existing.is_some() => {
                provider.save_draft(None, &raw, thread.as_ref()).await
            }
            other => other,
        };
        let gmail_id = match saved {
            Ok(id) => id,
            Err(e) => return Ok(Err(e)),
        };
        let now = now_millis();
        self.db()
            .write(move |tx| {
                if drafts::get(tx, draft_id)?.is_some() {
                    drafts::set_gmail_draft_id(tx, draft_id, Some(&gmail_id))
                } else {
                    // Discarded while we were uploading: remove the copy too.
                    outbox::enqueue(tx, &OutboxOp::DeleteDraft { gmail_draft_id: gmail_id }, now).map(|_| ())
                }
            })
            .await?;
        Ok(Ok(()))
    }

    /// When the next op waiting on a retry becomes ready.
    pub async fn next_outbox_retry(&self) -> SyncResult<Option<Millis>> {
        Ok(self.db().read(outbox::next_retry_at).await?)
    }

    pub async fn outbox_counts(&self) -> SyncResult<OutboxCounts> {
        Ok(self.db().read(outbox::counts).await?)
    }
}
