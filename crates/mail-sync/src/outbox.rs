//! Local mutations and the outbox drain (spec §7.4).

use std::collections::HashSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mail_domain::{EmailAddress, LabelId, MessageId, Millis, ThreadId, system_labels};
use mail_store::cleanup;
use mail_store::drafts::{self, DraftState};
use mail_store::outbox::{self, OutboxCounts, OutboxOp};
use mail_store::undo::{self, MessageDiff};
use mail_store::{Db, MailWriter, ThreadChanges};
use provider_api::{LabelOp, ProviderError};

use crate::engine::SyncEngine;
use crate::error::SyncResult;

/// Retries before an op is given up and rolled back.
pub const MAX_ATTEMPTS: u32 = 5;

/// How soon to look again while another drainer has an op in flight.
const BUSY_POLL_MS: Millis = 2_000;

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
    /// `batched` sends them as label changes in batches of 1,000, Trash
    /// included ([`undo::batched_ops`]), as the bulk action did.
    Exact {
        diffs: Vec<MessageDiff>,
        batched: bool,
    },
    /// Clean Up (spec §14.12): these labels on every message in the groups
    /// named by `keys`, resolved in the change's own transaction, so a
    /// group that grew since it was shown is acted on as it is now.
    /// Message-level: a mixed thread's other messages stay where they are.
    /// Reaches the provider in batches of 1,000 (`batchModify`).
    Cleanup {
        query: cleanup::Query,
        keys: Vec<String>,
        add: Vec<LabelId>,
        remove: Vec<LabelId>,
    },
}

impl LocalChange {
    pub fn archive(thread_ids: Vec<ThreadId>) -> Self {
        Self::Labels { thread_ids, add: vec![], remove: vec![LabelId::new(system_labels::INBOX)] }
    }
    pub fn move_to_inbox(thread_ids: Vec<ThreadId>) -> Self {
        Self::Labels { thread_ids, add: vec![LabelId::new(system_labels::INBOX)], remove: vec![] }
    }
    /// Mark as Junk: to Spam and out of the Inbox. Only this and
    /// `not_junk` set or clear `SPAM` (spec §14.3 amendment, junk).
    pub fn mark_junk(thread_ids: Vec<ThreadId>) -> Self {
        Self::Labels {
            thread_ids,
            add: vec![LabelId::new(system_labels::SPAM)],
            remove: vec![LabelId::new(system_labels::INBOX)],
        }
    }
    /// Not Junk: out of Spam and into the Inbox.
    pub fn not_junk(thread_ids: Vec<ThreadId>) -> Self {
        Self::Labels {
            thread_ids,
            add: vec![LabelId::new(system_labels::INBOX)],
            remove: vec![LabelId::new(system_labels::SPAM)],
        }
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

/// What a recorded change did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppliedChange {
    pub changes: ThreadChanges,
    /// The undoable action's id, when one was recorded.
    pub action: Option<i64>,
    /// Messages whose labels changed.
    pub messages: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrainReport {
    pub sent: usize,
    pub retrying: usize,
    pub failed: usize,
    /// Another drainer (another engine or process on this store) had an op
    /// in flight, so this drain stopped behind it.
    pub busy: bool,
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
    Ok(apply_local_change_recorded(db, change, queue, None).await?.changes)
}

/// [`apply_local_change`], recording what it changed per message as an
/// undoable action of `record` kind ("archive", …) when given. The
/// action's id is `None` when nothing changed or nothing was recorded.
pub async fn apply_local_change_recorded(
    db: &Db,
    change: LocalChange,
    queue: bool,
    record: Option<String>,
) -> SyncResult<AppliedChange> {
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
                LocalChange::Exact { diffs, batched } => {
                    let ops = if batched { undo::batched_ops(&diffs) } else { undo::provider_ops(tx, &diffs)? };
                    let mut w = MailWriter::new(tx);
                    let mut applied = Vec::with_capacity(diffs.len());
                    for d in diffs {
                        // A message gone since (deleted on the server) is skipped.
                        if w.modify_message_labels(&d.message, &d.added, &d.removed)? {
                            applied.push(d);
                        }
                    }
                    let changes = w.finish()?;
                    let kept: HashSet<&MessageId> = applied.iter().map(|d| &d.message).collect();
                    let ops = ops.into_iter().filter_map(|op| retain_messages(op, &kept)).collect();
                    (ops, applied, changes)
                }
                LocalChange::Cleanup { query, keys, add, remove } => {
                    let applied = cleanup::apply(tx, &query, &keys, &add, &remove)?;
                    (applied.ops, applied.diffs, applied.changes)
                }
            };
            if queue {
                for op in &ops {
                    outbox::enqueue(tx, op, now)?;
                }
            }
            let messages = diffs.iter().filter(|d| !d.added.is_empty() || !d.removed.is_empty()).count();
            let action = match record {
                Some(kind) if messages > 0 => Some(undo::record(tx, &kind, &diffs, now)?),
                _ => None,
            };
            Ok(AppliedChange { changes, action, messages })
        })
        .await?)
}

/// `op` limited to `kept` messages, or `None` if none is left.
fn retain_messages(op: OutboxOp, kept: &HashSet<&MessageId>) -> Option<OutboxOp> {
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
    ) -> SyncResult<AppliedChange> {
        let applied = apply_local_change_recorded(self.db(), change, true, record).await?;
        self.publish_changes(&applied.changes);
        Ok(applied)
    }

    /// Send every ready op to the provider, oldest first. Transient failures
    /// are retried later with backoff; after [`MAX_ATTEMPTS`], or on a
    /// permanent error, the op is rolled back locally and marked failed.
    ///
    /// Safe to run from several engines or processes on one store (spec
    /// §7.4, outbox claims): each op is claimed in the transaction that
    /// picks it, only while nothing else is in flight, so ops go one at a
    /// time and in order whoever sends them. When another drainer has an op
    /// in flight this returns with [`DrainReport::busy`] set; ask again
    /// at [`SyncEngine::next_outbox_retry`].
    pub async fn drain_outbox(&self) -> SyncResult<DrainReport> {
        let _serialized = self.drain_lock.lock().await;
        let me = self.claimant.clone();
        self.db().write(move |tx| outbox::recover_in_flight(tx, &me, now_millis())).await?;
        let mut report = DrainReport::default();
        loop {
            let now = now_millis();
            // Claimed in one transaction with the pick: a held send
            // cancelled since stays unsent (Undo Send), and an op another
            // drainer took is not taken again.
            let me = self.claimant.clone();
            let claim = self.db().write(move |tx| outbox::claim_next(tx, me.id(), now, now + outbox::LEASE_MS)).await?;
            let queued = match claim {
                outbox::Claim::Ready(queued) => queued,
                outbox::Claim::Idle => return Ok(report),
                outbox::Claim::Busy => {
                    report.busy = true;
                    return Ok(report);
                }
            };
            let id = queued.id;
            let timer = crate::transport::Timer::start();
            // A send's optimistic copy and the id the provider gave it.
            let mut adopt: Option<(MessageId, MessageId)> = None;
            let call = async {
                SyncResult::Ok(match &queued.op {
                    OutboxOp::ModifyLabels { message_ids, add, remove } => {
                        // Before the call: history may report it before we return.
                        self.remember_own(message_ids, add, remove);
                        self.modify_labels_present(LabelOp {
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
                            match self.provider().move_to_trash(m).await {
                                // Deleted on the server: the others still go.
                                Ok(()) | Err(ProviderError::NotFound(_)) => {}
                                Err(e) => {
                                    result = Err(e);
                                    break;
                                }
                            }
                        }
                        result
                    }
                    OutboxOp::Untrash { message_ids } => {
                        self.remember_own(message_ids, &[], &[LabelId::new(system_labels::TRASH)]);
                        let mut result = Ok(());
                        for m in message_ids {
                            match self.provider().restore_from_trash(m).await {
                                Ok(()) | Err(ProviderError::NotFound(_)) => {}
                                Err(e) => {
                                    result = Err(e);
                                    break;
                                }
                            }
                        }
                        result
                    }
                    OutboxOp::Send { raw, thread_id, local_message_id, .. } => match crate::compose::decode_raw(raw) {
                        Some(bytes) => self.send_once(&bytes, thread_id.as_ref(), queued.attempts).await.map(|sent| {
                            if self.provider().adopts_sent_copies() {
                                adopt = Some((local_message_id.clone(), sent));
                            }
                        }),
                        None => Err(ProviderError::Invalid("queued message is corrupt".into())),
                    },
                    OutboxOp::SyncDraft { draft_id, from } => self.mirror_draft(*draft_id, from).await?,
                    OutboxOp::DeleteDraft { gmail_draft_id } => self.provider().delete_draft(gmail_draft_id).await,
                })
            };
            let result = self.leased(id, call).await?;
            self.record_api(
                &timer,
                crate::transport::Job::Write,
                "one API call per change, with exact undo",
                1,
                result.is_ok(),
            );
            // The outcome is recorded only while the claim is still ours:
            // past its lease another drainer may have taken the op back.
            let me = self.claimant.clone();
            let held = move |tx: &mail_store::Transaction<'_>| outbox::holds(tx, id, me.id());
            match result {
                Ok(()) => {
                    let changes = self
                        .db()
                        .write(move |tx| {
                            if !held(tx)? {
                                return Ok(ThreadChanges::default());
                            }
                            outbox::complete(tx, id)?;
                            let mut w = MailWriter::new(tx);
                            if let Some((local, sent)) = &adopt {
                                w.adopt_local_copy(local, sent)?;
                            }
                            w.finish()
                        })
                        .await?;
                    self.publish_changes(&changes);
                    report.sent += 1;
                    // Bulk changes go out as many ops: say how many are left.
                    let counts = self.db().read(outbox::counts).await?;
                    if counts.pending > 0 {
                        self.observer.outbox_progress(counts);
                    }
                }
                // A message deleted on the server: nothing left to change.
                // Multi-message ops only get here once every present id
                // was changed ([`Self::modify_labels_present`]).
                Err(ProviderError::NotFound(_)) if !matches!(queued.op, OutboxOp::Send { .. }) => {
                    self.db().write(move |tx| if held(tx)? { outbox::complete(tx, id) } else { Ok(()) }).await?;
                    report.sent += 1;
                }
                Err(e) if e.is_transient() && queued.attempts + 1 < MAX_ATTEMPTS => {
                    let at = now + backoff(queued.attempts).as_millis() as Millis;
                    let message = e.to_string();
                    self.db()
                        .write(move |tx| if held(tx)? { outbox::retry_later(tx, id, at, &message) } else { Ok(()) })
                        .await?;
                    report.retrying += 1;
                    // Later ops wait: order matters (archive then unarchive).
                    return Ok(report);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "outbox op failed permanently; rolling back");
                    // The provider's own words, shown in Drafts and the banner.
                    let message = match &e {
                        ProviderError::Forbidden(m) | ProviderError::Invalid(m) => m.clone(),
                        other => other.to_string(),
                    };
                    let changes =
                        self.db()
                            .write(move |tx| {
                                if held(tx)? { outbox::fail(tx, id, &message) } else { Ok(ThreadChanges::default()) }
                            })
                            .await?;
                    self.publish_changes(&changes);
                    report.failed += 1;
                    if matches!(e, ProviderError::Unauthorized) {
                        return Err(e.into());
                    }
                }
            }
        }
    }

    /// Send a queued message, unless an earlier attempt already got it to
    /// the provider. A send tried before (`attempts` > 0: an error that
    /// may have come after the provider took it, or left in flight by a
    /// drainer that died) is looked for first ([`MailProvider::already_sent`])
    /// and, found, taken as sent rather than sent again.
    ///
    /// [`MailProvider::already_sent`]: provider_api::MailProvider::already_sent
    async fn send_once(
        &self,
        raw: &[u8],
        thread: Option<&ThreadId>,
        attempts: u32,
    ) -> Result<MessageId, ProviderError> {
        if attempts > 0
            && let Some(sent) = self.provider().already_sent(raw).await?
        {
            tracing::info!(attempts, "an earlier attempt of this send reached the provider; not sending it again");
            return Ok(sent);
        }
        self.provider().send(raw, thread).await
    }

    /// Run an op's provider call while renewing this drainer's claim on it
    /// every [`outbox::LEASE_RENEW_MS`], so a long call (a large
    /// attachment) is not taken for a dead one by another drainer.
    async fn leased<T>(&self, id: i64, call: impl std::future::Future<Output = T>) -> T {
        let mut call = std::pin::pin!(call);
        loop {
            tokio::select! {
                out = &mut call => return out,
                () = tokio::time::sleep(Duration::from_millis(outbox::LEASE_RENEW_MS as u64)) => {
                    let me = self.claimant.clone();
                    let until = now_millis() + outbox::LEASE_MS;
                    match self.db().write(move |tx| outbox::renew(tx, id, me.id(), until)).await {
                        Ok(true) => {}
                        Ok(false) => tracing::warn!(id, "outbox claim lost to another drainer during the call"),
                        Err(e) => tracing::warn!(error = %e, id, "renewing an outbox claim failed"),
                    }
                }
            }
        }
    }

    /// One label change for many messages, where a message deleted on the
    /// server must not cost the others their change. Whether Gmail's
    /// `batchModify` fails as a whole when one id is gone is a hand-check
    /// (plan 2026-10-08), so a `NotFound` for several ids is split in
    /// halves and each retried, down to the single missing ids, which are
    /// dropped: a batch of 1,000 with one missing id takes about 20 calls.
    /// Label changes are idempotent, so a half sent twice changes nothing.
    async fn modify_labels_present(&self, op: LabelOp) -> Result<(), ProviderError> {
        let mut pending = vec![op];
        while let Some(op) = pending.pop() {
            match self.provider().modify_labels(&op).await {
                Ok(()) => {}
                Err(ProviderError::NotFound(id)) if op.message_ids.len() > 1 => {
                    tracing::debug!(%id, count = op.message_ids.len(), "a message in the batch is gone; splitting it");
                    let mut first = op.clone();
                    let second = first.message_ids.split_off(op.message_ids.len() / 2);
                    pending.push(LabelOp { message_ids: second, ..op });
                    pending.push(first);
                }
                Err(ProviderError::NotFound(id)) => tracing::debug!(%id, "gone on the server; its change is dropped"),
                Err(e) => return Err(e),
            }
        }
        Ok(())
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

    /// When to drain again: when the next op waiting on a retry becomes
    /// ready, or shortly while another drainer has an op in flight (so
    /// the ops behind it go once it is done, or are recovered if it died).
    pub async fn next_outbox_retry(&self) -> SyncResult<Option<Millis>> {
        let (retry, busy) = self.db().read(|c| Ok((outbox::next_retry_at(c)?, outbox::in_flight(c)?))).await?;
        let poll = busy.then(|| now_millis() + BUSY_POLL_MS);
        Ok(match (retry, poll) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        })
    }

    pub async fn outbox_counts(&self) -> SyncResult<OutboxCounts> {
        Ok(self.db().read(outbox::counts).await?)
    }
}
