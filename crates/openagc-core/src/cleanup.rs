//! Clean Up (spec §14.12): an account's messages grouped by sender,
//! subject, list, time, category or size, and bulk actions on the groups
//! the user ticks.
//!
//! Every call names its account, since the window cleans one account
//! whatever the main window shows. All are `async` (spec §4.2): the reads
//! like any read, and the apply because it writes tens of thousands of
//! messages in one transaction (about a second for 20,000,
//! docs/performance.md), which must not happen on the caller's thread.
//! The provider side follows through the outbox in batches of 1,000,
//! with an `OutboxStatus` event after each batch.

use mail_domain::{LabelId, LabelKind, system_labels};
use mail_store::cleanup;
use mail_sync::LocalChange;

use crate::ffi::AddressInfo;
use crate::mutations::UndoToken;
use crate::{Core, CoreError, ErrorKind, runtime};

/// How groups are formed (the window's left column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupView {
    Sender,
    /// People I've Emailed: senders the user has written to.
    People,
    Subject,
    MailingList,
    /// Today, Yesterday, This Week, Last Week, then months.
    Time,
    Social,
    Promotions,
    Size,
}

impl From<CleanupView> for cleanup::View {
    fn from(v: CleanupView) -> Self {
        match v {
            CleanupView::Sender => Self::Sender,
            CleanupView::People => Self::People,
            CleanupView::Subject => Self::Subject,
            CleanupView::MailingList => Self::MailingList,
            CleanupView::Time => Self::Time,
            CleanupView::Social => Self::Social,
            CleanupView::Promotions => Self::Promotions,
            CleanupView::Size => Self::Size,
        }
    }
}

/// Which messages are grouped: the Inbox, or everything but Spam, Trash
/// and drafts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupScope {
    Inbox,
    AllMail,
}

impl From<CleanupScope> for cleanup::Scope {
    fn from(s: CleanupScope) -> Self {
        match s {
            CleanupScope::Inbox => Self::Inbox,
            CleanupScope::AllMail => Self::AllMail,
        }
    }
}

/// One row of the groups column, largest first (Time and Size in their
/// own order).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CleanupGroup {
    /// Opaque: passed back to name the group.
    pub key: String,
    pub title: String,
    /// Other names the sender or list used, most used first.
    pub aka: Vec<String>,
    /// The address (Sender, People), the list id (Mailing Lists), the
    /// senders' names (Social, Promotions) or the range (Size).
    pub detail: Option<String>,
    /// Messages in it now.
    pub count: u64,
    /// The user unsubscribed from this list or sender from Clean Up.
    pub unsubscribed: bool,
}

impl From<cleanup::Group> for CleanupGroup {
    fn from(g: cleanup::Group) -> Self {
        Self { key: g.key, title: g.title, aka: g.aka, detail: g.detail, count: g.count, unsubscribed: g.unsubscribed }
    }
}

/// One row of the messages column.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CleanupMessage {
    pub id: String,
    pub thread_id: String,
    pub from: Option<AddressInfo>,
    pub subject: String,
    /// Milliseconds since the epoch.
    pub date: i64,
    /// Bytes, as the provider reports them.
    pub size: u64,
}

impl From<cleanup::CleanupMessage> for CleanupMessage {
    fn from(m: cleanup::CleanupMessage) -> Self {
        Self {
            id: m.id.0,
            thread_id: m.thread_id.0,
            from: m.from.map(Into::into),
            subject: m.subject,
            date: m.date,
            size: m.size,
        }
    }
}

/// What the toolbar does to every message in the ticked groups.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CleanupAction {
    /// Out of the Inbox.
    Archive,
    /// As the mail list's Move: the label added, out of the Inbox. `INBOX`
    /// moves to the Inbox.
    Move {
        label_id: String,
    },
    Trash,
    /// To Spam and out of the Inbox; Gmail learns from it.
    Spam,
}

/// What an apply did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CleanupResult {
    /// Messages whose labels changed; ones already so are not counted.
    pub changed: u64,
    /// For `undo_action` and `redo_action`, as for any mail action (spec
    /// §14.6a); none when nothing changed.
    pub undo: Option<UndoToken>,
    /// The undo notice: "Archived 813 messages from Amazon".
    pub description: String,
    /// The name after Edit › Undo: "Archive", "Move", "Move to Trash",
    /// "Mark as Spam".
    pub action_name: String,
}

/// One day of the Inbox's history: its count at the start of the day.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CleanupDay {
    /// `YYYY-MM-DD` in the user's calendar.
    pub day: String,
    pub count: u64,
}

/// The Inbox Zero card under Clean Up's views (spec §14.12).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CleanupProgress {
    /// The Inbox when Clean Up was first opened, raised when the Inbox
    /// outgrew it (older Inbox mail arriving as every header loads).
    pub baseline: u64,
    /// How much of the baseline is gone, 0 to 100; 100 for an empty Inbox.
    pub percent: u8,
    /// The Inbox at the start of today.
    pub at_midnight: u64,
    /// Mail received today (by arrival, not Spam), wherever it is now.
    pub received_today: u64,
    /// `at_midnight + received_today - now`, never below zero.
    pub removed_today: u64,
    /// The Inbox now.
    pub now: u64,
    /// The Inbox at the start of each recorded day of the last 30, oldest
    /// first, ending with today's: the sparkline, with `now` as its last
    /// point.
    pub days: Vec<CleanupDay>,
}

/// The card's numbers at `now`: also what opening Clean Up records,
/// today's count at midnight (if the first sync after midnight has not
/// recorded it) and the baseline (set once; raised, never lowered).
pub(crate) fn progress_at(
    tx: &mail_store::Transaction<'_>,
    now: i64,
    utc_offset_secs: i64,
) -> mail_store::StoreResult<CleanupProgress> {
    cleanup::record_today(tx, now, utc_offset_secs)?;
    let inbox = cleanup::inbox_count(tx)?;
    if !cleanup::set_baseline_once(tx, inbox, now)? {
        cleanup::raise_baseline(tx, inbox)?;
    }
    let p = cleanup::progress(tx, now, utc_offset_secs)?;
    Ok(CleanupProgress {
        baseline: p.baseline.unwrap_or(p.now),
        percent: p.percent(),
        at_midnight: p.at_midnight,
        received_today: p.received_today,
        removed_today: p.removed_today,
        now: p.now,
        days: p.days.into_iter().map(|(day, count)| CleanupDay { day, count }).collect(),
    })
}

/// Seconds east of UTC on this Mac now.
pub(crate) fn utc_offset_now() -> i64 {
    i64::from(chrono::Local::now().offset().local_minus_utc())
}

/// Undo records of Clean Up's applies start with this; they are undone
/// and redone in batches, as they were applied.
const KIND_PREFIX: &str = "cleanup_";

pub(crate) fn is_cleanup_action(kind: &str) -> bool {
    kind.starts_with(KIND_PREFIX)
}

/// A query at this moment, in the user's time zone (the Time view's
/// Today and Yesterday).
fn query(view: CleanupView, scope: CleanupScope) -> cleanup::Query {
    cleanup::Query {
        view: view.into(),
        scope: scope.into(),
        now: mail_sync::now_millis(),
        utc_offset_secs: utc_offset_now(),
    }
}

/// "1 message", "2,500 messages".
fn messages(n: usize) -> String {
    if n == 1 {
        return "1 message".into();
    }
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("{out} messages")
}

/// Which messages, for the notice: "813 messages from Amazon", "20 large
/// messages", "5 messages from 3 groups". `title` is the one group's.
fn which(view: CleanupView, n: usize, groups: usize, title: Option<&str>) -> String {
    match (groups, title) {
        (1, Some(title)) => match view {
            CleanupView::Size => {
                let size = title.to_lowercase();
                if n == 1 { format!("1 {size} message") } else { messages(n).replacen(" ", &format!(" {size} "), 1) }
            }
            CleanupView::Subject if title == "(no subject)" => format!("{} with no subject", messages(n)),
            CleanupView::Subject => format!("{} with the subject “{title}”", messages(n)),
            _ => format!("{} from {title}", messages(n)),
        },
        (1, None) => messages(n),
        _ => format!("{} from {groups} groups", messages(n)),
    }
}

#[uniffi::export]
impl Core {
    /// The groups of `view` in `account_id`, filtered by `filter` (title,
    /// aka, address or key containing it, ignoring case).
    pub async fn cleanup_groups(
        &self,
        account_id: String,
        view: CleanupView,
        scope: CleanupScope,
        filter: String,
    ) -> Result<Vec<CleanupGroup>, CoreError> {
        let db = self.store_for(&account_id).await?;
        let q = query(view, scope);
        runtime::run(async move {
            let groups = db.read(move |c| cleanup::groups(c, &q, &filter)).await?;
            Ok(groups.into_iter().map(Into::into).collect())
        })
        .await
    }

    /// The messages of the groups named by `keys`, newest first, a page at
    /// a time.
    pub async fn cleanup_messages(
        &self,
        account_id: String,
        view: CleanupView,
        scope: CleanupScope,
        keys: Vec<String>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<CleanupMessage>, CoreError> {
        let db = self.store_for(&account_id).await?;
        let q = query(view, scope);
        runtime::run(async move {
            let rows = db.read(move |c| cleanup::messages(c, &q, &keys, offset, limit)).await?;
            Ok(rows.into_iter().map(Into::into).collect())
        })
        .await
    }

    /// How many messages the groups named by `keys` hold now.
    pub async fn cleanup_count(
        &self,
        account_id: String,
        view: CleanupView,
        scope: CleanupScope,
        keys: Vec<String>,
    ) -> Result<u64, CoreError> {
        let db = self.store_for(&account_id).await?;
        let q = query(view, scope);
        runtime::run(async move { Ok(db.read(move |c| cleanup::count(c, &q, &keys)).await?) }).await
    }

    /// Apply `action` to every message in the groups named by `keys`, as
    /// they are now: one undoable action, whatever the number of messages,
    /// recording per message what changed (ADR 0006). The provider gets
    /// the changes through the outbox in batches of 1,000. A user's action:
    /// agents' bulk caps do not apply.
    pub async fn cleanup_apply(
        &self,
        account_id: String,
        view: CleanupView,
        scope: CleanupScope,
        keys: Vec<String>,
        action: CleanupAction,
    ) -> Result<CleanupResult, CoreError> {
        if keys.is_empty() {
            return Err(CoreError::new(ErrorKind::InvalidInput, "no groups given"));
        }
        let db = self.store_for(&account_id).await?;
        let q = query(view, scope);
        let inbox = LabelId::new(system_labels::INBOX);
        let groups = keys.len();
        let one = (groups == 1).then(|| keys[0].clone());
        let (labels, title) = runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let labels = mail_store::read::list_labels(c)?;
                    // Read before acting: a group the action empties is gone after.
                    let title = match one {
                        Some(key) => cleanup::groups(c, &q, "")?.into_iter().find(|g| g.key == key).map(|g| g.title),
                        None => None,
                    };
                    Ok((labels, title))
                })
                .await?)
        })
        .await?;
        let (add, remove, kind, action_name, done) = match &action {
            CleanupAction::Archive => (vec![], vec![inbox], "archive", "Archive", "Archived {}".to_owned()),
            CleanupAction::Move { label_id } => {
                let label = labels
                    .iter()
                    .find(|l| l.id.as_str() == label_id)
                    .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that label no longer exists"))?;
                if label.id == inbox {
                    (vec![inbox], vec![], "move", "Move", "Moved {} to the Inbox".to_owned())
                } else if label.kind == LabelKind::User {
                    let to = format!("Moved {{}} to “{}”", label.name);
                    (vec![label.id.clone()], vec![inbox], "move", "Move", to)
                } else {
                    return Err(CoreError::new(
                        ErrorKind::InvalidInput,
                        format!("{} is not a label to move to", label.name),
                    ));
                }
            }
            CleanupAction::Trash => (
                vec![LabelId::new(system_labels::TRASH)],
                vec![inbox],
                "trash",
                "Move to Trash",
                "Moved {} to the Trash".to_owned(),
            ),
            CleanupAction::Spam => (
                vec![LabelId::new(system_labels::SPAM)],
                vec![inbox],
                "spam",
                "Mark as Spam",
                "Moved {} to Spam".to_owned(),
            ),
        };
        let change = LocalChange::Cleanup { query: q, keys, add, remove };
        let record = format!("{KIND_PREFIX}{kind}");
        let (undo, changed) = crate::registry::scoped(Some(account_id), self.apply(change, Some(&record))).await?;
        let description = if changed == 0 {
            "Nothing to change: those messages are already there".to_owned()
        } else {
            done.replacen("{}", &which(view, changed, groups, title.as_deref()), 1)
        };
        Ok(CleanupResult { changed: changed as u64, undo, description, action_name: action_name.to_owned() })
    }

    /// The Inbox Zero card's numbers for `account_id` (spec §14.12). Called
    /// as Clean Up opens and after each change, so it also records what
    /// opening records: today's count at midnight and the baseline.
    pub async fn cleanup_progress(&self, account_id: String) -> Result<CleanupProgress, CoreError> {
        let db = self.store_for(&account_id).await?;
        let (now, offset) = (mail_sync::now_millis(), utc_offset_now());
        runtime::run(async move { Ok(db.write(move |tx| progress_at(tx, now, offset)).await?) }).await
    }

    /// Development hook for snapshots: the Inbox's count at the start of
    /// each of the last `counts.len()` days, today's last, and the
    /// baseline. Replaces what was recorded for those days.
    pub async fn debug_seed_inbox_history(
        &self,
        account_id: String,
        counts: Vec<u64>,
        baseline: u64,
    ) -> Result<(), CoreError> {
        let db = self.store_for(&account_id).await?;
        let (now, offset) = (mail_sync::now_millis(), utc_offset_now());
        runtime::run(async move {
            db.write(move |tx| {
                let days = counts.len() as i64;
                for (i, count) in counts.into_iter().enumerate() {
                    let at = now - (days - 1 - i as i64) * 86_400_000;
                    tx.execute("DELETE FROM inbox_history WHERE day = ?1", [cleanup::day_key(at, offset)])?;
                    cleanup::record_inbox_day(tx, &cleanup::day_key(at, offset), count)?;
                }
                tx.execute("DELETE FROM cleanup_meta WHERE key LIKE 'baseline%'", [])?;
                cleanup::set_baseline_once(tx, baseline, now)?;
                Ok(())
            })
            .await?;
            Ok(())
        })
        .await
    }
}

mod load;
pub use load::{CleanupLoadEstimate, CleanupLoadStatus};
mod unsubscribe;
pub use unsubscribe::{CleanupUnsubscribeMethod, CleanupUnsubscribeResult, CleanupUnsubscribeTarget};

#[cfg(test)]
mod tests;
