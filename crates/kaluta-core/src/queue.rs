//! The queue of changes on their way to the provider, as the user sees it
//! (spec §7.4, *the queue*): what is waiting, retrying or given up, in
//! words, with Try Again and Dismiss.

use mail_domain::{LabelId, MessageId, system_labels};
use mail_store::outbox::{self, OutboxEntry, OutboxOp};
use mail_store::{Connection, StoreResult};

use crate::sync::EventObserver;
use crate::{Core, CoreError, runtime};

/// Where a queued change is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum QueuedChangeState {
    /// Goes as soon as the ones before it have.
    Waiting,
    /// Failed for now (the network, a busy server); tried again at
    /// `next_attempt_at`.
    Retrying,
    /// Being sent now.
    Sending,
    /// Given up: undone here, kept for the user to try again or dismiss.
    Failed,
}

/// One change in the queue.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QueuedChange {
    pub id: i64,
    /// "Archive", "Add “Waiting”", "Send": what it does.
    pub action: String,
    /// What it acts on: "“Q3 planning”", "“Q3 planning” and 4 more
    /// conversations".
    pub target: String,
    pub state: QueuedChangeState,
    /// Tries so far.
    pub attempts: u32,
    /// When it was made, ms since the epoch.
    pub created_at: i64,
    /// When the next try is due (Retrying).
    pub next_attempt_at: Option<i64>,
    /// The last error, in the provider's words.
    pub error: Option<String>,
}

/// A label's name for the user: system labels by what changing them means.
fn label_words(id: &LabelId, added: bool, names: &[(String, String)]) -> String {
    match (id.as_str(), added) {
        (system_labels::INBOX, false) => "Archive".into(),
        (system_labels::INBOX, true) => "Move to the Inbox".into(),
        (system_labels::UNREAD, false) => "Mark as read".into(),
        (system_labels::UNREAD, true) => "Mark as unread".into(),
        (system_labels::STARRED, true) => "Star".into(),
        (system_labels::STARRED, false) => "Unstar".into(),
        (system_labels::SPAM, true) => "Mark as spam".into(),
        (system_labels::SPAM, false) => "Not spam".into(),
        (system_labels::TRASH, true) => "Move to the Trash".into(),
        (system_labels::TRASH, false) => "Restore from the Trash".into(),
        (other, added) => {
            let name = names.iter().find(|(id, _)| id == other).map_or(other, |(_, n)| n.as_str());
            if added { format!("Add “{name}”") } else { format!("Remove “{name}”") }
        }
    }
}

/// What a label change does, in words: "Archive", "Add “Waiting”".
fn labels_action(add: &[LabelId], remove: &[LabelId], names: &[(String, String)]) -> String {
    // Archiving with a label added is a move: name the label first.
    let mut words: Vec<String> = add.iter().map(|l| label_words(l, true, names)).collect();
    words.extend(remove.iter().map(|l| label_words(l, false, names)));
    if words.is_empty() { "Change labels".into() } else { words.join(", ") }
}

/// The conversations `messages` are in, in words.
fn target(conn: &Connection, messages: &[MessageId]) -> StoreResult<String> {
    let mut subjects: Vec<String> = Vec::new();
    let mut stmt = conn.prepare_cached(
        "SELECT t.gmail_id, t.subject FROM messages m JOIN threads t ON t.id = m.thread_id WHERE m.gmail_id = ?1",
    )?;
    let mut threads: Vec<String> = Vec::new();
    for m in messages {
        let mut rows = stmt.query([m.as_str()])?;
        let found = match rows.next()? {
            Some(r) => Some((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            None => None,
        };
        if let Some((thread, subject)) = found
            && !threads.contains(&thread)
        {
            threads.push(thread);
            subjects.push(subject);
        }
    }
    let quote = |s: &str| if s.trim().is_empty() { "(no subject)".to_owned() } else { format!("“{}”", s.trim()) };
    Ok(match subjects.as_slice() {
        [] => {
            let n = messages.len();
            format!("{n} message{}", if n == 1 { "" } else { "s" })
        }
        [one] => quote(one),
        [first, rest @ ..] => {
            let n = rest.len();
            format!("{} and {n} more conversation{}", quote(first), if n == 1 { "" } else { "s" })
        }
    })
}

/// A queued op in the user's words.
fn describe(conn: &Connection, e: OutboxEntry, names: &[(String, String)], now: i64) -> StoreResult<QueuedChange> {
    let (action, target) = match &e.op {
        OutboxOp::ModifyLabels { message_ids, add, remove } => {
            (labels_action(add, remove, names), target(conn, message_ids)?)
        }
        OutboxOp::Trash { message_ids, .. } => ("Move to the Trash".into(), target(conn, message_ids)?),
        OutboxOp::Untrash { message_ids } => ("Restore from the Trash".into(), target(conn, message_ids)?),
        OutboxOp::Send { local_message_id, draft_id, .. } => {
            let mut stmt = conn.prepare_cached("SELECT subject FROM messages WHERE gmail_id = ?1")?;
            let mut rows = stmt.query([local_message_id.as_str()])?;
            let subject: Option<String> = match rows.next()? {
                Some(r) => Some(r.get(0)?),
                None => None,
            };
            let subject = match subject {
                Some(s) => Some(s),
                None => mail_store::drafts::get(conn, *draft_id)?.map(|d| d.subject),
            };
            ("Send".into(), subject.map_or_else(|| "an email".into(), |s| format!("“{}”", s.trim())))
        }
        OutboxOp::SyncDraft { .. } => ("Save a draft".into(), String::new()),
        OutboxOp::DeleteDraft { .. } => ("Delete a draft".into(), String::new()),
    };
    let state = match e.state.as_str() {
        "failed" => QueuedChangeState::Failed,
        "in_flight" => QueuedChangeState::Sending,
        _ if e.attempts > 0 && e.next_attempt_at.is_some_and(|at| at > now) => QueuedChangeState::Retrying,
        _ => QueuedChangeState::Waiting,
    };
    Ok(QueuedChange {
        id: e.id,
        action,
        target,
        state,
        attempts: e.attempts,
        created_at: e.created_at,
        next_attempt_at: e.next_attempt_at,
        error: e.last_error,
    })
}

/// Every queued change of the store, oldest first.
pub(crate) fn queued_changes(conn: &Connection, now: i64) -> StoreResult<Vec<QueuedChange>> {
    let names: Vec<(String, String)> =
        mail_store::read::list_labels(conn)?.into_iter().map(|l| (l.id.0, l.name)).collect();
    outbox::entries(conn)?.into_iter().map(|e| describe(conn, e, &names, now)).collect()
}

#[uniffi::export]
impl Core {
    /// The changes on their way to the provider, and those given up, for
    /// the account this work acts on (spec §7.4, the queue).
    pub async fn queued_changes(&self) -> Result<Vec<QueuedChange>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            let now = mail_sync::now_millis();
            Ok(db.read(move |c| queued_changes(c, now)).await?)
        })
        .await
    }

    /// Try again: the failed change `id` (every failed one when `None`) is
    /// made here again and queued afresh, and changes waiting on a retry
    /// go now.
    pub async fn retry_queued_changes(&self, id: Option<i64>) -> Result<(), CoreError> {
        let db = self.db()?;
        let events = self.account_events();
        let service = self.sync_service();
        runtime::run(async move {
            let now = mail_sync::now_millis();
            let (_, changes) = db
                .write(move |tx| {
                    let requeued = outbox::requeue_failed(tx, id, now)?;
                    outbox::retry_now(tx)?;
                    Ok(requeued)
                })
                .await?;
            mail_sync::SyncObserver::threads_changed(
                &EventObserver { events: events.clone(), settled: None },
                &changes,
            );
            let c = db.read(outbox::counts).await?;
            events.emit(crate::CoreEvent::OutboxStatus { pending: c.pending, failed: c.failed });
            if let Some(service) = service {
                service.outbox_changed();
            }
            Ok(())
        })
        .await
    }

    /// Dismiss the failed change `id`: the user has seen it.
    pub async fn dismiss_queued_change(&self, id: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        let events = self.account_events();
        runtime::run(async move {
            db.write(move |tx| outbox::discard_failed(tx, id).map(|_| ())).await?;
            let c = db.read(outbox::counts).await?;
            events.emit(crate::CoreEvent::OutboxStatus { pending: c.pending, failed: c.failed });
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_changes_read_as_actions() {
        let names = vec![("Label_24".to_owned(), "Waiting".to_owned())];
        let l = |s: &str| LabelId::new(s);
        assert_eq!(labels_action(&[], &[l("INBOX")], &names), "Archive");
        assert_eq!(labels_action(&[l("Label_24")], &[], &names), "Add “Waiting”");
        assert_eq!(labels_action(&[l("Label_24")], &[l("INBOX")], &names), "Add “Waiting”, Archive");
        assert_eq!(labels_action(&[], &[l("UNREAD")], &names), "Mark as read");
        assert_eq!(labels_action(&[l("Label_9")], &[], &names), "Add “Label_9”");
    }
}
