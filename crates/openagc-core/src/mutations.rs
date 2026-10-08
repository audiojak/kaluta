//! Mailbox mutations from the UI (spec §4.2, §7.4). Applied to the store at
//! once and queued for Gmail; Swift has already updated its rows, so these
//! are async and never block the main thread on the store's writer.

use mail_domain::{LabelId, ThreadId, system_labels};
use mail_sync::LocalChange;

use crate::ffi::LabelInfo;
use crate::sync::EventObserver;
use crate::{Core, CoreError, ErrorKind, runtime};

/// A user action that can be undone (spec §14.6a): which account's store
/// holds it, and its id there.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UndoToken {
    pub account_id: String,
    pub action_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct OutboxStatus {
    pub pending: u32,
    pub failed: u32,
}

/// White or near-black text, whichever reads better on `background`.
fn text_color_for(background: &str) -> &'static str {
    let hex = background.trim_start_matches('#');
    let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0) as f64;
    let luminance = 0.299 * channel(0) + 0.587 * channel(2) + 0.114 * channel(4);
    if luminance > 150.0 { "#000000" } else { "#ffffff" }
}

fn threads(ids: Vec<String>) -> Result<Vec<ThreadId>, CoreError> {
    if ids.is_empty() {
        return Err(CoreError::new(ErrorKind::InvalidInput, "no threads given"));
    }
    Ok(ids.into_iter().map(ThreadId).collect())
}

impl Core {
    /// Apply a change to the account this work acts on. With `record`, it
    /// is kept as an undoable action of that kind and its token returned,
    /// with the number of messages it changed.
    pub(crate) async fn apply(
        &self,
        change: LocalChange,
        record: Option<&str>,
    ) -> Result<(Option<UndoToken>, usize), CoreError> {
        let service = self.sync_service();
        let db = self.db()?;
        let events = self.account_events();
        let account_id = self.effective_account_id();
        let record = record.map(str::to_owned);
        let applied = runtime::run(async move {
            let applied = match service {
                Some(service) => {
                    let applied = service.engine().apply_change_recorded(change, record).await?;
                    service.outbox_changed();
                    applied
                }
                // No provider (the demo mailbox): local only.
                None => {
                    let applied = mail_sync::apply_local_change_recorded(&db, change, false, record).await?;
                    mail_sync::SyncObserver::threads_changed(
                        &EventObserver { events, settled: None },
                        &applied.changes,
                    );
                    applied
                }
            };
            Ok(applied)
        })
        .await?;
        let token = applied.action.zip(account_id).map(|(action_id, account_id)| UndoToken { account_id, action_id });
        Ok((token, applied.messages))
    }

    /// A user's action: recorded for undo.
    async fn mutate(&self, change: LocalChange, kind: &str) -> Result<Option<UndoToken>, CoreError> {
        self.apply(change, Some(kind)).await.map(|(token, _)| token)
    }

    /// An agent's or a routine's action: not on the user's undo stack
    /// (they have the activity log and a run's Undo, spec §14.6a).
    pub(crate) async fn mutate_unrecorded(&self, change: LocalChange) -> Result<(), CoreError> {
        self.apply(change, None).await.map(|_| ())
    }

    /// Apply a recorded action's diffs, or their inverse, in its account.
    async fn replay(&self, token: UndoToken, inverse: bool) -> Result<(), CoreError> {
        let account = token.account_id.clone();
        crate::registry::scoped(Some(account), async move {
            let db = self.db()?;
            let id = token.action_id;
            let action = runtime::run(async move { Ok(db.read(move |c| mail_store::undo::get(c, id)).await?) })
                .await?
                .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that action can no longer be undone"))?;
            let diffs = if inverse { action.diffs.iter().map(|d| d.inverse()).collect() } else { action.diffs };
            // A bulk action is undone in batches, as it was applied.
            let batched = crate::cleanup::is_cleanup_action(&action.kind);
            self.apply(LocalChange::Exact { diffs, batched }, None).await.map(|_| ())
        })
        .await
    }
}

#[uniffi::export]
impl Core {
    /// The mutations below return a token for undo (spec §14.6a), or none
    /// when nothing changed (archiving what is already archived).
    pub async fn archive(&self, thread_ids: Vec<String>) -> Result<Option<UndoToken>, CoreError> {
        self.mutate(LocalChange::archive(threads(thread_ids)?), "archive").await
    }

    pub async fn move_to_inbox(&self, thread_ids: Vec<String>) -> Result<Option<UndoToken>, CoreError> {
        self.mutate(LocalChange::move_to_inbox(threads(thread_ids)?), "move_to_inbox").await
    }

    /// Mark as Junk (spec §14.3 amendment, junk): the threads go to Spam
    /// and leave the Inbox; Gmail learns from it. A user action only:
    /// `modify_labels` and agent tools may not set `SPAM`.
    pub async fn mark_junk(&self, thread_ids: Vec<String>) -> Result<Option<UndoToken>, CoreError> {
        self.mutate(LocalChange::mark_junk(threads(thread_ids)?), "junk").await
    }

    /// Not Junk: out of Spam, back in the Inbox.
    pub async fn not_junk(&self, thread_ids: Vec<String>) -> Result<Option<UndoToken>, CoreError> {
        self.mutate(LocalChange::not_junk(threads(thread_ids)?), "not_junk").await
    }

    pub async fn set_read(&self, thread_ids: Vec<String>, read: bool) -> Result<Option<UndoToken>, CoreError> {
        self.mutate(LocalChange::set_read(threads(thread_ids)?, read), if read { "read" } else { "unread" }).await
    }

    pub async fn set_starred(&self, thread_ids: Vec<String>, starred: bool) -> Result<Option<UndoToken>, CoreError> {
        self.mutate(LocalChange::set_starred(threads(thread_ids)?, starred), if starred { "star" } else { "unstar" })
            .await
    }

    /// Reverse a recorded action exactly: per message, only the labels it
    /// changed, whatever happened since. Works in the token's account.
    pub async fn undo_action(&self, token: UndoToken) -> Result<(), CoreError> {
        self.replay(token, true).await
    }

    /// Apply an undone action again.
    pub async fn redo_action(&self, token: UndoToken) -> Result<(), CoreError> {
        self.replay(token, false).await
    }

    /// Add/remove user labels. System labels that change a message's
    /// nature (spam, trash, draft, sent) are refused here.
    pub async fn modify_labels(
        &self,
        thread_ids: Vec<String>,
        add: Vec<String>,
        remove: Vec<String>,
    ) -> Result<Option<UndoToken>, CoreError> {
        let protected = |l: &String| system_labels::PROTECTED.contains(&l.as_str());
        if add.iter().chain(&remove).any(protected) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "spam, trash, draft and sent are not labels to set"));
        }
        let kind = if remove.is_empty() {
            "label"
        } else if add.is_empty() {
            "unlabel"
        } else {
            "relabel"
        };
        let change = LocalChange::Labels {
            thread_ids: threads(thread_ids)?,
            add: add.into_iter().map(LabelId).collect(),
            remove: remove.into_iter().map(LabelId).collect(),
        };
        self.mutate(change, kind).await
    }

    /// Create a user label, or return the one with that name (spec §10.2,
    /// §11). A `/` path creates any missing parents first, so the label
    /// tree never has holes OpenAGC made (spec §14.3). `color` is a
    /// background like `#fb4c2f` for the label itself; parents get none.
    /// Needs Gmail to be reachable: label ids come from the server.
    pub async fn create_label(&self, name: String, color: Option<String>) -> Result<LabelInfo, CoreError> {
        let name = name.trim().trim_matches('/').to_owned();
        let segments: Vec<&str> = name.split('/').collect();
        for depth in 1..segments.len() {
            self.create_single_label(segments[..depth].join("/"), None).await?;
        }
        self.create_single_label(name, color).await
    }

    pub async fn trash(&self, thread_ids: Vec<String>) -> Result<Option<UndoToken>, CoreError> {
        self.mutate(LocalChange::Trash { thread_ids: threads(thread_ids)? }, "trash").await
    }

    pub async fn outbox_status(&self) -> Result<OutboxStatus, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            let c = db.read(mail_store::outbox::counts).await?;
            Ok(OutboxStatus { pending: c.pending, failed: c.failed })
        })
        .await
    }

    /// Forget changes that could not be applied (they were already undone).
    pub async fn clear_failed_changes(&self) -> Result<(), CoreError> {
        let db = self.db()?;
        let events = self.account_events();
        runtime::run(async move {
            db.write(|tx| mail_store::outbox::clear_failed(tx).map(|_| ())).await?;
            let c = db.read(mail_store::outbox::counts).await?;
            events.emit(crate::CoreEvent::OutboxStatus { pending: c.pending, failed: c.failed });
            Ok(())
        })
        .await
    }
}

/// Local label ids must differ even when created in the same millisecond
/// (a path creates its parents back to back).
fn next_local_label() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl Core {
    /// One label, no parents: returns the existing label with that name
    /// (case-insensitive) or creates it. The text color is chosen for
    /// contrast with `color`.
    async fn create_single_label(&self, name: String, color: Option<String>) -> Result<LabelInfo, CoreError> {
        let name = name.trim().trim_matches('/').to_owned();
        if name.is_empty() || name.len() > 225 || name.split('/').any(|part| part.trim().is_empty()) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "a label needs a name (nest with /)"));
        }
        if system_labels::PROTECTED
            .iter()
            .chain(&["INBOX", "UNREAD", "STARRED", "IMPORTANT"])
            .any(|s| s.eq_ignore_ascii_case(&name))
        {
            return Err(CoreError::new(ErrorKind::InvalidInput, format!("{name} is a system label")));
        }
        let db = self.db()?;
        let service = self.sync_service();
        let events = self.account_events();
        runtime::run(async move {
            let wanted = name.clone();
            let labels = db.read(mail_store::read::list_labels).await?;
            if let Some(existing) = labels.into_iter().find(|l| l.name.eq_ignore_ascii_case(&wanted)) {
                return Ok(existing.into());
            }
            let color = color.map(|bg| {
                let fg = text_color_for(&bg);
                (bg, fg.to_owned())
            });
            let label = match &service {
                Some(service) => {
                    let provider = service.engine().provider();
                    let with_color = color.as_ref().map(|(bg, fg)| (bg.as_str(), fg.as_str()));
                    match provider.create_label(&name, with_color).await {
                        // Gmail only accepts palette colors; try again plain.
                        Err(provider_api::ProviderError::Invalid(_)) if with_color.is_some() => {
                            provider.create_label(&name, None).await?
                        }
                        other => other?,
                    }
                }
                // The demo mailbox has no server: a local id.
                None => mail_domain::Label {
                    id: LabelId(format!("Label_local_{}_{}", mail_sync::now_millis(), next_local_label())),
                    name,
                    kind: mail_domain::LabelKind::User,
                    color: color.map(|(background, text)| mail_domain::LabelColor { background, text }),
                    visible: true,
                },
            };
            let stored = label.clone();
            db.write(move |tx| {
                let mut w = mail_store::MailWriter::new(tx);
                w.upsert_labels(std::slice::from_ref(&stored))?;
                w.finish().map(|_| ())
            })
            .await?;
            // The sidebar reloads its labels on any change event.
            events.emit(crate::CoreEvent::ThreadsChanged {
                mailbox_id: label.id.0.clone(),
                hint: crate::ChangeHint::default(),
            });
            Ok(label.into())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use futures::executor::block_on;
    use mail_domain::{EmailAddress, LabelId, MessageId, ThreadId};
    use provider_api::fake::FakeProvider;
    use provider_api::{FetchedBody, FetchedMessage};

    use crate::{Core, CoreConfig, CoreEvent, EventListener};

    struct Noop;
    impl EventListener for Noop {
        fn on_event(&self, _: Option<String>, _: CoreEvent) {}
    }

    fn core(name: &str) -> Arc<Core> {
        let dir = std::env::temp_dir().join(format!("openagc-core-mut-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Core::new(
            CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
            Arc::new(crate::secrets::MemorySecrets::default()),
            Arc::new(Noop),
        )
        .unwrap()
    }

    fn inbox_ids(core: &Core) -> Vec<String> {
        block_on(core.list_threads("INBOX".into(), None, 500)).unwrap().rows.into_iter().map(|r| r.id).collect()
    }

    #[test]
    fn demo_mutations_apply_locally_without_an_outbox() {
        let core = core("demo");
        block_on(core.clone().open_account("demo".into())).unwrap();
        block_on(core.debug_seed_demo_mailbox(80)).unwrap();
        let first = inbox_ids(&core)[0].clone();
        block_on(core.archive(vec![first.clone()])).unwrap();
        assert!(!inbox_ids(&core).contains(&first));
        assert_eq!(block_on(core.outbox_status()).unwrap().pending, 0);
        block_on(core.move_to_inbox(vec![first.clone()])).unwrap();
        assert!(inbox_ids(&core).contains(&first));
        let err = block_on(core.modify_labels(vec![first.clone()], vec!["TRASH".into()], vec![])).unwrap_err();
        assert_eq!(err.kind(), crate::ErrorKind::InvalidInput);
        let err = block_on(core.modify_labels(vec![first], vec!["SPAM".into()], vec![])).unwrap_err();
        assert_eq!(err.kind(), crate::ErrorKind::InvalidInput, "only Mark as Junk sets SPAM");
    }

    #[test]
    fn junk_goes_to_spam_on_the_server_and_back_with_undo_or_not_junk() {
        let core = core("junk");
        block_on(core.clone().open_account("acct".into())).unwrap();
        let fake = Arc::new(FakeProvider::new("me@example.com", 1_790_000_000_000, 50));
        fake.seed(seeded("m1", "t1", &["INBOX", "UNREAD"]));
        fake.seed(seeded("m2", "t2", &["INBOX"]));
        core.start_sync_with(fake.clone()).unwrap();
        wait_until("both synced", || inbox_ids(&core).len() == 2);
        let spam_ids = |core: &Core| -> Vec<String> {
            block_on(core.list_threads("SPAM".into(), None, 50)).unwrap().rows.into_iter().map(|r| r.id).collect()
        };

        let token = block_on(core.mark_junk(vec!["t1".into()])).unwrap().expect("t1 changed");
        assert_eq!(inbox_ids(&core), ["t2"], "local at once");
        assert_eq!(spam_ids(&core), ["t1"]);
        wait_until("server marked it spam", || {
            let labels = labels_of(&fake, "m1");
            labels.contains(&"SPAM".to_owned()) && !labels.contains(&"INBOX".to_owned())
        });

        block_on(core.undo_action(token)).unwrap();
        assert!(inbox_ids(&core).contains(&"t1".to_owned()) && spam_ids(&core).is_empty());
        wait_until("server undid it", || {
            let labels = labels_of(&fake, "m1");
            !labels.contains(&"SPAM".to_owned()) && labels.contains(&"INBOX".to_owned())
        });

        block_on(core.mark_junk(vec!["t2".into()])).unwrap();
        block_on(core.not_junk(vec!["t2".into()])).unwrap().expect("t2 changed");
        assert!(spam_ids(&core).is_empty());
        wait_until("server has it back in the Inbox", || labels_of(&fake, "m2") == ["INBOX"]);
        assert!(block_on(core.not_junk(vec!["t2".into()])).unwrap().is_none(), "not junk already: nothing to undo");
        core.stop_sync();
    }

    #[test]
    fn synced_mutations_reach_the_server_through_the_outbox_loop() {
        let core = core("synced");
        block_on(core.clone().open_account("acct".into())).unwrap();
        let fake = Arc::new(FakeProvider::new("me@example.com", 1_790_000_000_000, 50));
        fake.seed(FetchedMessage {
            id: MessageId::new("m1"),
            thread_id: ThreadId::new("t1"),
            label_ids: vec![LabelId::new("INBOX"), LabelId::new("UNREAD")],
            internal_date: 1_790_000_000_000,
            from: Some(EmailAddress::new(None, "a@example.com")),
            body: Some(FetchedBody { text: Some("hi".into()), html: None, attachments: vec![] }),
            ..Default::default()
        });
        core.start_sync_with(fake.clone()).unwrap();
        for _ in 0..200 {
            if inbox_ids(&core).len() == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        block_on(core.archive(vec!["t1".into()])).unwrap();
        assert!(inbox_ids(&core).is_empty(), "local at once");
        for _ in 0..200 {
            if !fake.message(&MessageId::new("m1")).unwrap().label_ids.contains(&LabelId::new("INBOX")) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !fake.message(&MessageId::new("m1")).unwrap().label_ids.contains(&LabelId::new("INBOX")),
            "server archived"
        );
        // The op is removed just after the server call returns.
        for _ in 0..200 {
            if block_on(core.outbox_status()).unwrap().pending == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(block_on(core.outbox_status()).unwrap().pending, 0);

        // Labels are created on the server, then stored; a path creates its
        // missing parent first, without the child's color.
        let label = block_on(core.create_label("Sorted/Later".into(), Some("#ffad47".into()))).unwrap();
        assert_eq!(label.id, "Label_2");
        assert_eq!(label.text_color.as_deref(), Some("#000000"), "dark text on a light color");
        let labels = block_on(core.list_labels()).unwrap();
        let parent = labels.iter().find(|l| l.name == "Sorted").expect("parent created");
        assert_eq!(parent.id, "Label_1");
        assert_eq!(parent.background_color, None);
        assert!(labels.iter().any(|l| l.id == "Label_2" && l.name == "Sorted/Later"));
        let same = block_on(core.create_label("SORTED/LATER".into(), None)).unwrap();
        assert_eq!(same.id, "Label_2");
        let deeper = block_on(core.create_label("Sorted/Later/Soon".into(), None)).unwrap();
        assert_eq!(deeper.id, "Label_3", "existing parents are reused");
        core.stop_sync();
    }

    fn labels_of(fake: &FakeProvider, id: &str) -> Vec<String> {
        fake.message(&MessageId::new(id)).unwrap().label_ids.into_iter().map(|l| l.0).collect()
    }

    fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..300 {
            if ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    fn seeded(id: &str, thread: &str, labels: &[&str]) -> FetchedMessage {
        FetchedMessage {
            id: MessageId::new(id),
            thread_id: ThreadId::new(thread),
            label_ids: labels.iter().map(|l| LabelId::new(*l)).collect(),
            internal_date: 1_790_000_000_000,
            from: Some(EmailAddress::new(None, "a@example.com")),
            body: Some(FetchedBody { text: Some("hi".into()), html: None, attachments: vec![] }),
            ..Default::default()
        }
    }

    #[test]
    fn undo_reverses_exactly_what_the_action_changed() {
        let core = core("undo-exact");
        block_on(core.clone().open_account("demo".into())).unwrap();
        block_on(core.debug_seed_demo_mailbox(40)).unwrap();
        let inbox = inbox_ids(&core);
        let (a, b) = (inbox[0].clone(), inbox[1].clone());
        // b is already archived before the action.
        assert!(block_on(core.archive(vec![b.clone()])).unwrap().is_some());
        let token = block_on(core.archive(vec![a.clone(), b.clone()])).unwrap().expect("a changed");
        assert_eq!(token.account_id, "demo");
        assert!(!inbox_ids(&core).contains(&a));

        block_on(core.undo_action(token.clone())).unwrap();
        let now = inbox_ids(&core);
        assert!(now.contains(&a), "undo brings back what the action archived");
        assert!(!now.contains(&b), "but not a thread that was already archived");

        block_on(core.redo_action(token.clone())).unwrap();
        assert!(!inbox_ids(&core).contains(&a), "redo archives it again");
        block_on(core.undo_action(token)).unwrap();
        assert!(inbox_ids(&core).contains(&a));

        // Nothing changed, nothing to undo.
        assert!(block_on(core.archive(vec![b])).unwrap().is_none());
    }

    #[test]
    fn undo_reaches_the_server_for_labels_and_trash_even_after_sync() {
        let core = core("undo-synced");
        block_on(core.clone().open_account("acct".into())).unwrap();
        let fake = Arc::new(FakeProvider::new("me@example.com", 1_790_000_000_000, 50));
        fake.seed(seeded("m1", "t1", &["INBOX", "UNREAD"]));
        fake.seed(seeded("m2", "t2", &["INBOX"]));
        fake.seed(seeded("m3", "t3", &["Label_9"]));
        core.start_sync_with(fake.clone()).unwrap();
        wait_until("bootstrap", || inbox_ids(&core).len() == 2);

        // Multi-thread archive, synced, then history applied, then undo.
        let token = block_on(core.archive(vec!["t1".into(), "t2".into(), "t3".into()])).unwrap().unwrap();
        wait_until("server archived", || !labels_of(&fake, "m1").contains(&"INBOX".into()));
        core.sync_now();
        std::thread::sleep(Duration::from_millis(100));
        block_on(core.undo_action(token)).unwrap();
        assert_eq!(inbox_ids(&core).len(), 2, "local at once");
        wait_until("server back in the Inbox", || {
            labels_of(&fake, "m1").contains(&"INBOX".into()) && labels_of(&fake, "m2").contains(&"INBOX".into())
        });
        assert!(!labels_of(&fake, "m3").contains(&"INBOX".into()), "t3 was never in the Inbox");
        assert!(labels_of(&fake, "m1").contains(&"UNREAD".into()), "other labels untouched");

        // Trash and undo: out of Trash on the server, back in the Inbox.
        let token = block_on(core.trash(vec!["t2".into()])).unwrap().unwrap();
        wait_until("server trashed", || labels_of(&fake, "m2").contains(&"TRASH".into()));
        block_on(core.undo_action(token.clone())).unwrap();
        wait_until("server untrashed", || {
            let l = labels_of(&fake, "m2");
            !l.contains(&"TRASH".into()) && l.contains(&"INBOX".into())
        });
        // Redo trashes it again.
        block_on(core.redo_action(token)).unwrap();
        wait_until("server trashed again", || labels_of(&fake, "m2").contains(&"TRASH".into()));
        assert!(!inbox_ids(&core).contains(&"t2".to_owned()));
        core.stop_sync();
    }

    #[test]
    fn tokens_only_work_in_their_own_account_and_the_last_fifty_are_kept() {
        let core = core("undo-accounts");
        block_on(core.clone().open_account("demo".into())).unwrap();
        block_on(core.debug_seed_demo_mailbox(20)).unwrap();
        let t = inbox_ids(&core)[0].clone();
        let mut last = None;
        for i in 0..60 {
            last = block_on(core.set_starred(vec![t.clone()], i % 2 == 0)).unwrap();
        }
        let last = last.unwrap();
        let first = super::UndoToken { account_id: "demo".into(), action_id: last.action_id - 59 };
        assert_eq!(block_on(core.undo_action(first)).unwrap_err().kind(), crate::ErrorKind::NotFound, "pruned");
        let kept = super::UndoToken { account_id: "demo".into(), action_id: last.action_id - 49 };
        block_on(core.undo_action(kept)).unwrap();

        let foreign = super::UndoToken { account_id: "work".into(), action_id: last.action_id };
        assert!(block_on(core.undo_action(foreign)).is_err(), "another account's store has no such action");
    }
}
