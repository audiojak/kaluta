//! Tasks made from email (spec §14.8). The tasks live in the account's
//! store, on this Mac; Gmail sees them only as the account's `Task` label,
//! which a thread carries while it has an open task. Label changes go
//! through the outbox like any other and are not on the user's undo stack
//! by themselves: undoing a task operation (reopen, restore, complete)
//! moves the label with it.

use mail_domain::{LabelId, ThreadId};
use mail_store::tasks::{self, TaskFields, TaskRow};
use mail_sync::LocalChange;

use crate::{Core, CoreError, CoreEvent, ErrorKind, runtime};

/// The label's name in Gmail.
pub const TASK_LABEL: &str = "Task";

/// What replying to the email means for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TaskAction {
    Reply,
    ReplyAll,
    Forward,
    None,
}

impl TaskAction {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TaskAction::Reply => "reply",
            TaskAction::ReplyAll => "reply_all",
            TaskAction::Forward => "forward",
            TaskAction::None => "none",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().replace([' ', '-'], "_").as_str() {
            "reply" => Some(TaskAction::Reply),
            "reply_all" | "replyall" => Some(TaskAction::ReplyAll),
            "forward" => Some(TaskAction::Forward),
            "none" | "" => Some(TaskAction::None),
            _ => None,
        }
    }
}

/// A task, with the email it is about.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskItem {
    pub id: i64,
    pub thread_id: String,
    pub message_id: Option<String>,
    pub title: String,
    pub notes: String,
    pub category: String,
    /// `YYYY-MM-DD`, or none.
    pub due_day: Option<String>,
    pub action: TaskAction,
    pub done: bool,
    /// Claude suggested it (rather than the user writing it).
    pub from_ai: bool,
    /// Claude's one line on why.
    pub why: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    /// The email's subject and sender; empty once the thread has left the
    /// store.
    pub subject: String,
    pub sender_name: Option<String>,
    pub sender_email: Option<String>,
}

impl From<TaskRow> for TaskItem {
    fn from(t: TaskRow) -> Self {
        TaskItem {
            id: t.id,
            thread_id: t.thread_id,
            message_id: t.message_id,
            title: t.title,
            notes: t.notes,
            category: t.category,
            due_day: t.due_day,
            action: TaskAction::parse(&t.action).unwrap_or(TaskAction::None),
            done: t.status == "done",
            from_ai: t.source == "ai",
            why: t.why,
            created_at: t.created_at,
            completed_at: t.completed_at,
            subject: t.subject,
            sender_name: t.from_name,
            sender_email: t.from_email,
        }
    }
}

/// A task to add: from the dialog, the bulk sheet, or Claude.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NewTask {
    pub thread_id: String,
    pub message_id: Option<String>,
    pub title: String,
    pub notes: String,
    pub category: String,
    pub due_day: Option<String>,
    pub action: TaskAction,
    pub why: String,
    pub from_ai: bool,
}

/// What the user can change on a task.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskEdit {
    pub title: String,
    pub notes: String,
    pub category: String,
    pub due_day: Option<String>,
    pub action: TaskAction,
}

/// A valid `YYYY-MM-DD`, trimmed; none for empty.
pub(crate) fn due_day(day: Option<String>) -> Result<Option<String>, CoreError> {
    match day.map(|d| d.trim().to_owned()).filter(|d| !d.is_empty()) {
        None => Ok(None),
        Some(d) => chrono::NaiveDate::parse_from_str(&d, "%Y-%m-%d")
            .map(|date| Some(date.format("%Y-%m-%d").to_string()))
            .map_err(|_| CoreError::new(ErrorKind::InvalidInput, format!("{d} is not a date (YYYY-MM-DD)"))),
    }
}

fn fields(
    title: String,
    notes: String,
    category: String,
    due: Option<String>,
    action: TaskAction,
) -> Result<TaskFields, CoreError> {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return Err(CoreError::new(ErrorKind::InvalidInput, "a task needs a title"));
    }
    let category = category.trim().to_owned();
    if category.is_empty() {
        return Err(CoreError::new(ErrorKind::InvalidInput, "a task needs a category"));
    }
    Ok(TaskFields {
        title,
        notes: notes.trim().to_owned(),
        category,
        due_day: due_day(due)?,
        action: action.as_str().into(),
    })
}

fn not_found() -> CoreError {
    CoreError::new(ErrorKind::NotFound, "that task no longer exists")
}

impl Core {
    /// The account's `Task` label: the one remembered, else a user label
    /// of that name (any case), else a new one (on Gmail, so this needs
    /// the server unless the account is the demo mailbox).
    async fn task_label(&self) -> Result<String, CoreError> {
        let db = self.db()?;
        let remembered = runtime::run(async move {
            let id = db.read(|c| tasks::meta(c, tasks::LABEL_KEY)).await?;
            let labels = db.read(mail_store::read::list_labels).await?;
            Ok(id.filter(|id| labels.iter().any(|l| l.id.0 == *id)))
        })
        .await?;
        if let Some(id) = remembered {
            return Ok(id);
        }
        let label = self.create_label(TASK_LABEL.into(), None).await?;
        let db = self.db()?;
        let id = label.id.clone();
        runtime::run(async move { Ok(db.write(move |tx| tasks::set_meta(tx, tasks::LABEL_KEY, &id)).await?) }).await?;
        Ok(label.id)
    }

    /// Bring the thread's `Task` label in line with its tasks, after a task
    /// on it changed: on while it has an open task; taken off when its last
    /// one closes. A label the user put on a thread with no task here is
    /// never touched: only a change to one of its tasks gets here. Best
    /// effort: the task itself has already changed, and a label that
    /// could not be made (offline, first time) is added with the next
    /// task.
    async fn follow_task_label(&self, thread_id: &str) {
        if let Err(e) = self.try_follow_task_label(thread_id).await {
            tracing::warn!(error = %e, "task label not updated");
        }
    }

    async fn try_follow_task_label(&self, thread_id: &str) -> Result<(), CoreError> {
        let db = self.db()?;
        let thread = thread_id.to_owned();
        let open =
            runtime::run(async move { Ok(!db.read(move |c| tasks::open_for_thread(c, &thread)).await?.is_empty()) })
                .await?;
        let label = if open {
            self.task_label().await?
        } else {
            // Nothing to take off if no label was ever made.
            let db = self.db()?;
            match runtime::run(async move { Ok(db.read(|c| tasks::meta(c, tasks::LABEL_KEY)).await?) }).await? {
                Some(id) => id,
                None => return Ok(()),
            }
        };
        let (add, remove) = if open { (vec![LabelId(label)], vec![]) } else { (vec![], vec![LabelId(label)]) };
        let change = LocalChange::Labels { thread_ids: vec![ThreadId(thread_id.to_owned())], add, remove };
        self.mutate_unrecorded(change).await
    }

    async fn task_by_id(&self, id: i64) -> Result<TaskItem, CoreError> {
        let db = self.db()?;
        runtime::run(
            async move { db.read(move |c| tasks::get(c, id)).await?.map(TaskItem::from).ok_or_else(not_found) },
        )
        .await
    }

    fn tasks_changed(&self) {
        self.account_events().emit(CoreEvent::TasksChanged);
    }
}

#[uniffi::export]
impl Core {
    /// Open tasks, dated ones by day then undated; with `include_done`,
    /// finished ones after them, latest first.
    pub async fn list_tasks(&self, include_done: bool) -> Result<Vec<TaskItem>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db.read(move |c| tasks::list(c, include_done)).await?.into_iter().map(TaskItem::from).collect())
        })
        .await
    }

    /// Add tasks (one from the dialog, several from the bulk sheet) and
    /// label their threads `Task`. Returns them as stored, in order.
    pub async fn create_tasks(&self, new: Vec<NewTask>) -> Result<Vec<TaskItem>, CoreError> {
        if new.is_empty() {
            return Err(CoreError::new(ErrorKind::InvalidInput, "no tasks given"));
        }
        let mut rows = Vec::with_capacity(new.len());
        for t in new {
            if t.thread_id.is_empty() {
                return Err(CoreError::new(ErrorKind::InvalidInput, "a task is about an email"));
            }
            let f = fields(t.title, t.notes, t.category, t.due_day, t.action)?;
            let why = t.why.split_whitespace().collect::<Vec<_>>().join(" ");
            rows.push((t.thread_id, t.message_id, f, if t.from_ai { "ai" } else { "you" }, why));
        }
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let threads: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
        let ids = runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    rows.iter()
                        .map(|(thread, message, f, source, why)| {
                            tasks::insert(tx, thread, message.as_deref(), f, source, why, now)
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .await?)
        })
        .await?;
        let mut seen = std::collections::BTreeSet::new();
        for thread in threads.iter().filter(|t| seen.insert(t.as_str())) {
            self.follow_task_label(thread).await;
        }
        self.tasks_changed();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(self.task_by_id(id).await?);
        }
        Ok(out)
    }

    pub async fn update_task(&self, id: i64, edit: TaskEdit) -> Result<TaskItem, CoreError> {
        let f = fields(edit.title, edit.notes, edit.category, edit.due_day, edit.action)?;
        let db = self.db()?;
        let found = runtime::run(async move { Ok(db.write(move |tx| tasks::update(tx, id, &f)).await?) }).await?;
        if !found {
            return Err(not_found());
        }
        self.tasks_changed();
        self.task_by_id(id).await
    }

    /// Mark done (or, with `done` false, open again, as undo does); the
    /// thread's `Task` label follows.
    pub async fn set_task_done(&self, id: i64, done: bool) -> Result<TaskItem, CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let found =
            runtime::run(async move { Ok(db.write(move |tx| tasks::set_done(tx, id, done, now)).await?) }).await?;
        if !found {
            return Err(not_found());
        }
        let task = self.task_by_id(id).await?;
        self.follow_task_label(&task.thread_id).await;
        self.tasks_changed();
        Ok(task)
    }

    /// Delete a task; returns it as it was, for `restore_task` (undo).
    pub async fn delete_task(&self, id: i64) -> Result<TaskItem, CoreError> {
        let task = self.task_by_id(id).await?;
        let db = self.db()?;
        runtime::run(async move { Ok(db.write(move |tx| tasks::delete(tx, id)).await?) }).await?;
        self.follow_task_label(&task.thread_id).await;
        self.tasks_changed();
        Ok(task)
    }

    /// Put a deleted task back exactly as it was.
    pub async fn restore_task(&self, task: TaskItem) -> Result<TaskItem, CoreError> {
        let row = TaskRow {
            id: task.id,
            thread_id: task.thread_id.clone(),
            message_id: task.message_id,
            title: task.title,
            notes: task.notes,
            category: task.category,
            due_day: due_day(task.due_day)?,
            action: task.action.as_str().into(),
            status: if task.done { "done" } else { "open" }.into(),
            source: if task.from_ai { "ai" } else { "you" }.into(),
            why: task.why,
            created_at: task.created_at,
            completed_at: task.completed_at,
            ..Default::default()
        };
        let db = self.db()?;
        runtime::run(async move { Ok(db.write(move |tx| tasks::restore(tx, &row)).await?) }).await?;
        self.follow_task_label(&task.thread_id).await;
        self.tasks_changed();
        self.task_by_id(task.id).await
    }

    /// Which of these threads have an open task (the bulk sheet unchecks
    /// them).
    pub async fn threads_with_open_tasks(&self, thread_ids: Vec<String>) -> Result<Vec<String>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(move |c| tasks::threads_with_open_tasks(c, &thread_ids)).await?) }).await
    }

    /// The id of the account's `Task` label, once there is one (the Inbox
    /// can hide threads carrying it).
    pub async fn task_label_id(&self) -> Result<Option<String>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            let id = db.read(|c| tasks::meta(c, tasks::LABEL_KEY)).await?;
            let labels = db.read(mail_store::read::list_labels).await?;
            Ok(id.filter(|id| labels.iter().any(|l| l.id.0 == *id)))
        })
        .await
    }

    /// The account's categories, in order.
    pub async fn task_categories(&self) -> Result<Vec<String>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(tasks::categories).await?) }).await
    }

    /// Replace the categories (Settings › Tasks): trimmed, empty ones and
    /// repeats (any case) dropped; at least one must remain.
    pub async fn set_task_categories(&self, names: Vec<String>) -> Result<Vec<String>, CoreError> {
        let mut clean: Vec<String> = Vec::new();
        for name in names {
            let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
            if !name.is_empty() && !clean.iter().any(|c| c.eq_ignore_ascii_case(&name)) {
                clean.push(name);
            }
        }
        if clean.is_empty() {
            return Err(CoreError::new(ErrorKind::InvalidInput, "keep at least one category"));
        }
        let db = self.db()?;
        let stored = clean.clone();
        runtime::run(async move { Ok(db.write(move |tx| tasks::set_categories(tx, &stored)).await?) }).await?;
        self.tasks_changed();
        Ok(clean)
    }

    /// Back to the starting set.
    pub async fn reset_task_categories(&self) -> Result<Vec<String>, CoreError> {
        self.set_task_categories(tasks::DEFAULT_CATEGORIES.iter().map(|s| (*s).to_owned()).collect()).await
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

    use super::*;
    use crate::{CoreConfig, EventListener};

    struct Noop;
    impl EventListener for Noop {
        fn on_event(&self, _: Option<String>, _: CoreEvent) {}
    }

    struct Scratch(std::path::PathBuf, Arc<Core>);
    impl Drop for Scratch {
        fn drop(&mut self) {
            self.1.stop_sync();
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn core(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("openagc-core-tasks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let core = Core::new(
            CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
            Arc::new(crate::secrets::MemorySecrets::default()),
            Arc::new(Noop),
        )
        .unwrap();
        Scratch(dir, core)
    }

    fn new_task(thread: &str, title: &str) -> NewTask {
        NewTask {
            thread_id: thread.into(),
            message_id: None,
            title: title.into(),
            notes: String::new(),
            category: "Reply".into(),
            due_day: Some("2026-10-01".into()),
            action: TaskAction::Reply,
            why: "They asked a question.".into(),
            from_ai: true,
        }
    }

    fn seeded(id: &str, thread: &str, labels: &[&str]) -> FetchedMessage {
        FetchedMessage {
            id: MessageId::new(id),
            thread_id: ThreadId::new(thread),
            label_ids: labels.iter().map(|l| LabelId::new(*l)).collect(),
            internal_date: 1_790_000_000_000,
            from: Some(EmailAddress::new(Some("Ada"), "ada@example.com")),
            subject: format!("About {thread}"),
            body: Some(FetchedBody { text: Some("Can you send the figures?".into()), html: None, attachments: vec![] }),
            ..Default::default()
        }
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

    fn inbox_ids(core: &Core) -> Vec<String> {
        block_on(core.list_threads("INBOX".into(), None, 50)).unwrap().rows.into_iter().map(|r| r.id).collect()
    }

    #[test]
    fn the_task_label_follows_the_threads_open_tasks_on_the_server() {
        let s = core("label");
        let core = &s.1;
        block_on(core.clone().open_account("acct".into())).unwrap();
        let fake = Arc::new(FakeProvider::new("me@example.com", 1_790_000_000_000, 50));
        fake.seed(seeded("m1", "t1", &["INBOX"]));
        fake.seed(seeded("m2", "t2", &["INBOX"]));
        core.start_sync_with(fake.clone()).unwrap();
        wait_until("synced", || inbox_ids(core).len() == 2);

        let made =
            block_on(core.create_tasks(vec![new_task("t1", " Send  the figures "), new_task("t1", "Book a call")]))
                .unwrap();
        assert_eq!(made[0].title, "Send the figures");
        assert_eq!(made[0].subject, "About t1");
        assert_eq!(made[0].sender_name.as_deref(), Some("Ada"));
        let label = block_on(core.task_label_id()).unwrap().expect("label made");
        let names: Vec<String> = block_on(core.list_labels()).unwrap().into_iter().map(|l| l.name).collect();
        assert!(names.contains(&"Task".to_owned()));
        wait_until("server labelled t1", || labels_of(&fake, "m1").contains(&label));
        assert!(inbox_ids(core).contains(&"t1".to_owned()), "accepting leaves the email where it is");

        // One of two done: still labelled; both done: label off.
        block_on(core.set_task_done(made[0].id, true)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(labels_of(&fake, "m1").contains(&label));
        let done = block_on(core.set_task_done(made[1].id, true)).unwrap();
        assert!(done.done && done.completed_at.is_some());
        wait_until("label off", || !labels_of(&fake, "m1").contains(&label));

        // Undo (reopen) puts it back; delete and restore do the same.
        block_on(core.set_task_done(made[1].id, false)).unwrap();
        wait_until("label back", || labels_of(&fake, "m1").contains(&label));
        let deleted = block_on(core.delete_task(made[1].id)).unwrap();
        wait_until("label off after delete", || !labels_of(&fake, "m1").contains(&label));
        let back = block_on(core.restore_task(deleted.clone())).unwrap();
        assert_eq!(back.id, deleted.id);
        wait_until("label back after restore", || labels_of(&fake, "m1").contains(&label));

        // The label is found again, not made twice.
        block_on(core.create_tasks(vec![new_task("t2", "Reply")])).unwrap();
        wait_until("t2 labelled", || labels_of(&fake, "m2").contains(&label));
        let count = block_on(core.list_labels()).unwrap().into_iter().filter(|l| l.name == "Task").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn a_task_label_put_on_by_hand_is_left_alone() {
        let s = core("by-hand");
        let core = &s.1;
        block_on(core.clone().open_account("demo".into())).unwrap();
        block_on(core.debug_seed_demo_mailbox(10)).unwrap();
        let inbox = inbox_ids(core);
        let made = block_on(core.create_tasks(vec![new_task(&inbox[0], "First")])).unwrap();
        let label = block_on(core.task_label_id()).unwrap().expect("a local label in the demo");
        // The user labels another thread Task in Gmail; no task here.
        block_on(core.modify_labels(vec![inbox[1].clone()], vec![label.clone()], vec![])).unwrap();
        block_on(core.set_task_done(made[0].id, true)).unwrap();
        let tagged: Vec<String> =
            block_on(core.list_threads(label.clone(), None, 50)).unwrap().rows.into_iter().map(|r| r.id).collect();
        assert_eq!(tagged, [inbox[1].clone()]);
    }

    #[test]
    fn tasks_are_validated_and_categories_edited() {
        let s = core("validate");
        let core = &s.1;
        block_on(core.clone().open_account("demo".into())).unwrap();
        let mut bad = new_task("t1", "  ");
        assert_eq!(block_on(core.create_tasks(vec![bad.clone()])).unwrap_err().kind(), ErrorKind::InvalidInput);
        bad.title = "Fine".into();
        bad.due_day = Some("next tuesday".into());
        assert_eq!(block_on(core.create_tasks(vec![bad])).unwrap_err().kind(), ErrorKind::InvalidInput);
        assert!(
            block_on(core.update_task(
                99,
                TaskEdit {
                    title: "x".into(),
                    notes: String::new(),
                    category: "Reply".into(),
                    due_day: None,
                    action: TaskAction::None,
                }
            ))
            .is_err()
        );

        assert_eq!(block_on(core.task_categories()).unwrap().len(), 7);
        let set =
            block_on(core.set_task_categories(vec![" Call  back ".into(), "call back".into(), "".into()])).unwrap();
        assert_eq!(set, ["Call back"]);
        assert!(block_on(core.set_task_categories(vec![" ".into()])).is_err());
        assert_eq!(block_on(core.reset_task_categories()).unwrap()[0], "Reply");
    }

    #[test]
    fn actions_and_days_parse_leniently() {
        assert_eq!(TaskAction::parse("Reply All"), Some(TaskAction::ReplyAll));
        assert_eq!(TaskAction::parse("reply-all"), Some(TaskAction::ReplyAll));
        assert_eq!(TaskAction::parse("call"), None);
        assert_eq!(due_day(Some(" 2026-9-30 ".into())).unwrap().as_deref(), Some("2026-09-30"));
        assert_eq!(due_day(Some("".into())).unwrap(), None);
        assert!(due_day(Some("2026-02-30".into())).is_err());
    }
}
