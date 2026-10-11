//! The workflow demo (docs/plans/design-review.md): a demo mailbox dated
//! from now, mail that keeps arriving while the app is open, and agents
//! that use the mail tools. Development only: the app turns it on with
//! `-KalutaDemoWorkflows YES`, on a throwaway data directory.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mail_store::MailWriter;

use crate::{ChangeHint, Core, CoreError, CoreEvent, ErrorKind, runtime};

/// The synthetic mailbox ends this long before now, so the hand-written
/// conversations are the newest mail.
const BULK_ENDS_BEFORE_NOW: i64 = 2 * 86_400_000;

/// One arrival loop per launch.
static ARRIVING: AtomicBool = AtomicBool::new(false);

#[uniffi::export]
impl Core {
    /// Fill the open account with the workflow demo: `threads` synthetic
    /// threads ending two days ago, then the hand-written conversations
    /// dated from now. Returns the number of messages written.
    pub async fn debug_seed_workflow_demo(&self, threads: u32) -> Result<u32, CoreError> {
        let db = self.db()?;
        let account =
            self.current_account_id().ok_or_else(|| CoreError::new(ErrorKind::NotFound, "no account is open"))?;
        let written = runtime::run(async move {
            let now = mail_sync::now_millis();
            let spec =
                mail_store::demo::DemoSpec { threads, newest_at: now - BULK_ENDS_BEFORE_NOW, ..Default::default() };
            let stats = tokio::task::spawn_blocking(move || -> mail_store::StoreResult<u32> {
                let bulk = mail_store::demo::generate(&db, &spec)?;
                let written = mail_store::demo_workflows::generate(&db, now)?;
                db.write_blocking(|tx| mail_store::read::set_sync_state(tx, "account_email", mail_store::demo::ME))?;
                Ok(bulk.messages + written.messages)
            })
            .await
            .map_err(|e| CoreError::new(ErrorKind::Internal, e.to_string()))??;
            Ok(stats)
        })
        .await?;
        // Listed like any account (with no sign-in, so it never syncs), so
        // the account menu can come back to it from an agent mailbox.
        self.register_account(crate::registry::IndexEntry {
            id: account,
            kind: crate::registry::AccountKind::Gmail,
            email: mail_store::demo::ME.to_owned(),
            display_name: Some("Demo Mailbox".to_owned()),
            avatar_file: None,
            added_at: mail_sync::now_millis(),
            imap: None,
            named_by_user: false,
            service: None,
        })
        .await?;
        Ok(written)
    }

    /// Deliver the workflow demo's arrivals to the open account, one every
    /// `interval_secs`, as sync would: the lists update and the new mail is
    /// announced. Runs until the app quits; a second call does nothing.
    pub fn debug_start_demo_arrivals(&self, interval_secs: u32) -> Result<(), CoreError> {
        let account =
            self.current_account_id().ok_or_else(|| CoreError::new(ErrorKind::NotFound, "no account is open"))?;
        let db = self.db()?;
        if ARRIVING.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        tracing::info!(account = %account, interval_secs, "demo arrivals start");
        let events = self.events.for_account(Some(account));
        let interval = Duration::from_secs(u64::from(interval_secs.max(1)));
        runtime::runtime().spawn(async move {
            let mut n = 0;
            loop {
                tokio::time::sleep(interval).await;
                let message = loop {
                    let next = mail_store::demo_workflows::arrival(n, mail_sync::now_millis());
                    n += 1;
                    if let Some(m) = next {
                        break m;
                    }
                };
                let incoming = message.clone();
                let written = db
                    .write(move |tx| {
                        let mut w = MailWriter::new(tx);
                        w.upsert_message(&incoming)?;
                        w.finish()
                    })
                    .await;
                let changes = match written {
                    Ok(changes) => {
                        tracing::info!(thread = %message.thread_id.as_str(), "demo arrival");
                        changes
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "demo arrival not written");
                        continue;
                    }
                };
                for (mailbox, change) in changes.mailboxes {
                    events.emit(CoreEvent::ThreadsChanged {
                        mailbox_id: mailbox,
                        hint: ChangeHint {
                            inserted: change.inserted.into_iter().collect(),
                            updated: change.updated.into_iter().collect(),
                            removed: change.removed.into_iter().collect(),
                            invalidate: false,
                        },
                    });
                }
                let has = |l: &str| message.label_ids.iter().any(|id| id.as_str() == l);
                if has("INBOX") && has("UNREAD") {
                    let new = mail_sync::NewMail {
                        id: message.id.clone(),
                        thread_id: message.thread_id.clone(),
                        from: message.from.clone(),
                        subject: message.subject.clone(),
                        snippet: message.snippet.clone(),
                    };
                    events.emit(CoreEvent::NewMail { messages: vec![new.into()] });
                }
            }
        });
        Ok(())
    }
}

/// Whether arrivals were started in this process (tests).
#[cfg(test)]
pub(crate) fn arriving() -> bool {
    ARRIVING.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::CoreConfig;
    use futures::executor::block_on;

    #[derive(Default)]
    struct Recorder(std::sync::Mutex<Vec<CoreEvent>>);
    impl crate::EventListener for Recorder {
        fn on_event(&self, _account: Option<String>, event: CoreEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn the_workflow_demo_is_dated_from_now_and_mail_keeps_arriving() {
        let dir = std::env::temp_dir().join(format!("kaluta-core-demo-workflows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let recorder = Arc::new(Recorder::default());
        let core = Core::new(
            CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
            Arc::new(crate::secrets::MemorySecrets::default()),
            recorder.clone(),
        )
        .unwrap();
        block_on(core.clone().open_account("demo".into())).unwrap();
        let written = block_on(core.debug_seed_workflow_demo(50)).unwrap();
        assert!(written > 50);
        let accounts = block_on(core.list_accounts()).unwrap();
        assert!(accounts.iter().any(|a| a.id == "demo"), "listed, so the account menu can return to it");

        let inbox = block_on(core.search_threads("in:inbox".into(), None, 1)).unwrap();
        let newest = inbox.rows.first().unwrap();
        assert!(mail_sync::now_millis() - newest.last_message_at < 3_600_000, "the newest mail is from the last hour");
        let important = block_on(core.search_threads("is:important newer_than:1d".into(), None, 50)).unwrap();
        assert!(important.rows.len() >= 6, "work for the sorting routine");

        core.debug_start_demo_arrivals(1).unwrap();
        assert!(arriving());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let got = recorder.0.lock().unwrap().iter().any(|e| matches!(e, CoreEvent::NewMail { .. }));
            if got {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "no arrival announced");
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
