//! Send later (spec §14.5 amendment 2026-11-18, ADR 0022): a draft with a
//! time to be sent at. It stays a draft until then; the app asks, when a
//! time comes and at launch, for the due ones to be sent, by the ordinary
//! send. Kaluta has no server, so nothing goes while it is not running: a
//! draft found later than the caller's grace is reported, not sent.

use mail_store::scheduled as store;

use crate::{Core, CoreError, ErrorKind, runtime};

/// A scheduled draft: whose it is, when it goes, and its subject to name it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScheduledSendInfo {
    pub account_id: String,
    pub draft_id: i64,
    /// Unix ms.
    pub send_at: i64,
    pub subject: String,
}

/// What became of the drafts whose time had come.
#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct ScheduledDue {
    /// Sent now.
    pub sent: Vec<ScheduledSendInfo>,
    /// Later than the grace allowed: still scheduled, for the user to
    /// decide (send now, another time, or keep as a draft).
    pub overdue: Vec<ScheduledSendInfo>,
    /// Could not be sent (no recipient, say): drafts again, unscheduled.
    pub failed: Vec<ScheduledSendInfo>,
}

/// Of the scheduled drafts, those whose time has come at `now`: the ones
/// to send, and the ones more than `grace_ms` late, which are not sent
/// unasked.
fn sort_due(all: Vec<ScheduledSendInfo>, now: i64, grace_ms: i64) -> (Vec<ScheduledSendInfo>, Vec<ScheduledSendInfo>) {
    all.into_iter().filter(|s| s.send_at <= now).partition(|s| now - s.send_at <= grace_ms)
}

impl Core {
    /// The open accounts' ids, to look in each.
    fn open_account_ids(&self) -> Vec<String> {
        self.open_accounts.read().unwrap_or_else(|e| e.into_inner()).stores.keys().cloned().collect()
    }

    /// This account's scheduled drafts, soonest first.
    async fn scheduled_here(&self) -> Result<Vec<ScheduledSendInfo>, CoreError> {
        let Some(account_id) = self.effective_account_id() else { return Ok(vec![]) };
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    store::list(c)?
                        .into_iter()
                        .map(|s| {
                            let subject =
                                mail_store::drafts::get(c, s.draft_id)?.map(|d| d.subject).unwrap_or_default();
                            Ok(ScheduledSendInfo {
                                account_id: account_id.clone(),
                                draft_id: s.draft_id,
                                send_at: s.send_at,
                                subject,
                            })
                        })
                        .collect::<mail_store::StoreResult<Vec<_>>>()
                })
                .await?)
        })
        .await
    }

    /// A scheduled draft that could not be sent is an ordinary draft again
    /// (the send itself unschedules in its own transaction).
    pub(crate) async fn unschedule(&self, draft_id: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move { Ok(db.write(move |tx| store::set(tx, draft_id, None, now)).await?) }).await?;
        Ok(())
    }
}

#[uniffi::export]
impl Core {
    /// Send a draft at `send_at` (Unix ms), or (`None`) no longer: it is
    /// an ordinary draft again. Returns the time it had before, for undo.
    pub async fn schedule_send(&self, draft_id: i64, send_at: Option<i64>) -> Result<Option<i64>, CoreError> {
        self.refuse_if_archive()?;
        let now = mail_sync::now_millis();
        if send_at.is_some_and(|at| at <= now) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "That time has already passed."));
        }
        let db = self.db()?;
        runtime::run(async move {
            db.write(move |tx| store::set(tx, draft_id, send_at, now)).await.map_err(|e| match e {
                mail_store::StoreError::Invalid(m) => CoreError::new(ErrorKind::InvalidInput, m),
                other => other.into(),
            })
        })
        .await
    }

    /// When a draft is to be sent (Unix ms), if it is scheduled.
    pub async fn scheduled_send(&self, draft_id: i64) -> Result<Option<i64>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(move |c| store::get(c, draft_id)).await?) }).await
    }

    /// Every open account's scheduled drafts, soonest first.
    pub async fn scheduled_sends(&self) -> Result<Vec<ScheduledSendInfo>, CoreError> {
        let mut all = vec![];
        for account in self.open_account_ids() {
            all.extend(crate::registry::scoped(Some(account), self.scheduled_here()).await?);
        }
        all.sort_by_key(|s| (s.send_at, s.draft_id));
        Ok(all)
    }

    /// Send the scheduled drafts whose time has come, in every open
    /// account. One more than `grace_ms` late is not sent: Kaluta was not
    /// running at its time, and what it says may no longer hold, so the
    /// user decides.
    pub async fn send_scheduled_due(&self, grace_ms: i64) -> Result<ScheduledDue, CoreError> {
        let now = mail_sync::now_millis();
        let (due, overdue) = sort_due(self.scheduled_sends().await?, now, grace_ms);
        let mut out = ScheduledDue { overdue, ..Default::default() };
        for info in due {
            // No hold: no one is there to take it back.
            let sent =
                crate::registry::scoped(Some(info.account_id.clone()), self.send_draft_holding(info.draft_id, Some(0)))
                    .await;
            match sent {
                Ok(_) => out.sent.push(info),
                Err(error) => {
                    tracing::warn!(draft = info.draft_id, %error, "scheduled send");
                    // Not to be tried again at every tick: a draft again.
                    let _ =
                        crate::registry::scoped(Some(info.account_id.clone()), self.unschedule(info.draft_id)).await;
                    out.failed.push(info);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::executor::block_on;

    use crate::{Core, CoreConfig, CoreEvent, EventListener};

    struct Noop;
    impl EventListener for Noop {
        fn on_event(&self, _: Option<String>, _: CoreEvent) {}
    }

    fn draft_to(subject: &str, to: &[&str]) -> crate::compose::DraftInfo {
        crate::compose::DraftInfo {
            id: 0,
            thread_id: None,
            in_reply_to_message_id: None,
            to: to.iter().map(|e| crate::ffi::AddressInfo { name: None, email: (*e).into() }).collect(),
            cc: vec![],
            bcc: vec![],
            subject: subject.into(),
            body_html: "<p>Hello</p>".into(),
            quoted_html: String::new(),
            attachments: vec![],
            status: crate::compose::DraftStatus::Editing,
            error: None,
            updated_at: 0,
        }
    }

    /// Move a draft's time into the past, as if the clock had run on.
    fn set_time(core: &Core, draft_id: i64, send_at: i64) {
        core.db()
            .unwrap()
            .write_blocking(move |tx| {
                Ok(tx.execute("UPDATE scheduled_sends SET send_at = ?2 WHERE draft_id = ?1", (draft_id, send_at))?)
            })
            .unwrap();
    }

    /// Which drafts go and which are asked about is decided without
    /// touching a store.
    #[test]
    fn what_is_due_goes_and_what_is_too_late_is_asked_about() {
        let at = |draft_id, send_at| super::ScheduledSendInfo {
            account_id: "a".into(),
            draft_id,
            send_at,
            subject: String::new(),
        };
        let (now, hour) = (10 * 3_600_000, 3_600_000);
        let all = vec![at(1, now - 2 * hour), at(2, now - hour), at(3, now), at(4, now + 1)];
        let (due, overdue) = super::sort_due(all, now, hour);
        assert_eq!(due.iter().map(|s| s.draft_id).collect::<Vec<_>>(), [2, 3], "an hour late still goes");
        assert_eq!(overdue.iter().map(|s| s.draft_id).collect::<Vec<_>>(), [1]);
        let (due, overdue) = super::sort_due(vec![at(4, now + 1)], now, hour);
        assert!(due.is_empty() && overdue.is_empty(), "not its time yet");
    }

    /// An end-to-end test of the send path, not of a state transition: it
    /// sends mail through the core in the demo mailbox, which sends
    /// locally at once, so no provider and no sync are started. Driven
    /// with the workspace's `futures` executor, as the compose tests are.
    #[test]
    fn a_scheduled_draft_stays_a_draft_until_its_time_then_goes() {
        let dir = std::env::temp_dir().join(format!("kaluta-core-scheduled-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let core = Core::new(
            CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
            Arc::new(crate::secrets::MemorySecrets::default()),
            Arc::new(Noop),
        )
        .unwrap();
        block_on(core.clone().open_account("demo".into())).unwrap();
        let now = mail_sync::now_millis();
        let hour = 3_600_000;
        let sent = |subject: &str| {
            block_on(core.list_threads("SENT".into(), None, 50)).unwrap().rows.iter().any(|t| t.subject == subject)
        };

        // Scheduled: still a draft, nothing in Sent, nothing queued to send.
        let id = block_on(core.save_draft(draft_to("Later", &["sam@example.com"]))).unwrap();
        assert!(block_on(core.schedule_send(id, Some(now - 1))).is_err(), "a time already past is refused");
        assert_eq!(block_on(core.schedule_send(id, Some(now + hour))).unwrap(), None);
        assert_eq!(block_on(core.scheduled_send(id)).unwrap(), Some(now + hour));
        let listed = block_on(core.scheduled_sends()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            (listed[0].draft_id, listed[0].subject.as_str(), listed[0].account_id.as_str()),
            (id, "Later", "demo")
        );
        assert_eq!(block_on(core.get_draft(id)).unwrap().unwrap().status, crate::compose::DraftStatus::Editing);
        assert!(!sent("Later"));
        assert_eq!(core.unsent_send_count_now(), 0, "quitting has nothing to wait for or release");

        // Not its time yet: nothing goes.
        assert_eq!(block_on(core.send_scheduled_due(hour)).unwrap(), super::ScheduledDue::default());
        // Rescheduling returns the time it had, for undo.
        assert_eq!(block_on(core.schedule_send(id, Some(now + 2 * hour))).unwrap(), Some(now + hour));

        // Its time comes: sent, and no longer scheduled.
        set_time(&core, id, now - 1_000);
        let due = block_on(core.send_scheduled_due(hour)).unwrap();
        assert_eq!(due.sent.iter().map(|s| s.draft_id).collect::<Vec<_>>(), [id]);
        assert!(due.overdue.is_empty() && due.failed.is_empty());
        assert!(sent("Later"));
        assert!(block_on(core.scheduled_sends()).unwrap().is_empty());
        assert!(block_on(core.send_scheduled_due(hour)).unwrap().sent.is_empty(), "never twice");

        // Found long after its time (Kaluta was not running): kept, reported.
        let late = block_on(core.save_draft(draft_to("Late", &["sam@example.com"]))).unwrap();
        block_on(core.schedule_send(late, Some(now + hour))).unwrap();
        set_time(&core, late, now - 2 * hour);
        let due = block_on(core.send_scheduled_due(hour)).unwrap();
        assert_eq!(due.overdue.iter().map(|s| s.draft_id).collect::<Vec<_>>(), [late]);
        assert!(due.sent.is_empty() && !sent("Late"));
        assert_eq!(block_on(core.scheduled_send(late)).unwrap(), Some(now - 2 * hour), "still scheduled");
        // Sent by hand instead: the ordinary send, which unschedules it.
        block_on(core.send_draft(late)).unwrap();
        assert!(sent("Late"));
        assert_eq!(block_on(core.scheduled_send(late)).unwrap(), None);
        assert!(block_on(core.send_scheduled_due(hour)).unwrap().sent.is_empty());

        // Cannot be sent (no one to send to): a draft again, not retried.
        let empty = block_on(core.save_draft(draft_to("No one", &[]))).unwrap();
        block_on(core.schedule_send(empty, Some(now + hour))).unwrap();
        set_time(&core, empty, now - 1_000);
        let due = block_on(core.send_scheduled_due(hour)).unwrap();
        assert_eq!(due.failed.iter().map(|s| s.draft_id).collect::<Vec<_>>(), [empty]);
        assert_eq!(block_on(core.scheduled_send(empty)).unwrap(), None);
        assert!(block_on(core.get_draft(empty)).unwrap().is_some(), "the draft is kept");

        // Unscheduling and deleting.
        let gone = block_on(core.save_draft(draft_to("Gone", &["sam@example.com"]))).unwrap();
        block_on(core.schedule_send(gone, Some(now + hour))).unwrap();
        assert_eq!(block_on(core.schedule_send(gone, None)).unwrap(), Some(now + hour));
        block_on(core.schedule_send(gone, Some(now + hour))).unwrap();
        block_on(core.delete_draft(gone)).unwrap();
        assert!(block_on(core.scheduled_sends()).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
