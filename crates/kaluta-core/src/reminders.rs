//! Remind me later (spec §14.13, amended 2026-11-09): B in Triage puts a
//! conversation off until a time the user picks; it comes back to the top
//! of Triage then, or as soon as someone writes in it. The email stays in
//! the Inbox meanwhile; nothing changes in Gmail.

use mail_store::reminders as store;

use crate::events::CoreEvent;
use crate::{Core, CoreError, runtime};

/// A conversation's reminder: when it comes back, and when it was set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct ReminderInfo {
    pub remind_at: i64,
    pub set_at: i64,
}

impl From<store::Reminder> for ReminderInfo {
    fn from(r: store::Reminder) -> Self {
        ReminderInfo { remind_at: r.remind_at, set_at: r.set_at }
    }
}

#[uniffi::export]
impl Core {
    /// Remind about a conversation at `at` (ms), or (`None`) no longer; the
    /// reminder it had before, for undo.
    pub async fn remind_later(&self, thread_id: String, at: Option<i64>) -> Result<Option<ReminderInfo>, CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let before =
            runtime::run(async move { Ok(db.write(move |tx| store::set(tx, &thread_id, at, now)).await?) }).await?;
        self.account_events().emit(CoreEvent::WorkflowsChanged);
        Ok(before.map(Into::into))
    }

    /// Put a conversation's reminder back as it was (undo).
    pub async fn restore_reminder(&self, thread_id: String, reminder: Option<ReminderInfo>) -> Result<(), CoreError> {
        let db = self.db()?;
        let back = reminder.map(|r| store::Reminder { remind_at: r.remind_at, set_at: r.set_at });
        runtime::run(async move { Ok(db.write(move |tx| store::restore(tx, &thread_id, back)).await?) }).await?;
        self.account_events().emit(CoreEvent::WorkflowsChanged);
        Ok(())
    }

    /// A conversation's reminder, if it has one.
    pub async fn reminder(&self, thread_id: String) -> Result<Option<ReminderInfo>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(move |c| store::get(c, &thread_id)).await?) })
            .await
            .map(|r| r.map(Into::into))
    }
}
