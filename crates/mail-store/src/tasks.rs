//! Tasks made from email (spec §14.8): what to do about a thread, by when,
//! in which category. Stored per account, on this Mac only; the account's
//! `Task` label is how Gmail sees them.

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::error::StoreResult;

/// The categories a new account starts with (also in `0009_tasks.sql`).
pub const DEFAULT_CATEGORIES: &[&str] = &["Reply", "Decide", "Gather Info", "Schedule", "Review", "Admin", "Follow Up"];

/// `task_meta` key for the id of the account's `Task` label.
pub const LABEL_KEY: &str = "label_id";

/// A task as stored, with the email it is about when that is still here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskRow {
    pub id: i64,
    pub thread_id: String,
    pub message_id: Option<String>,
    pub title: String,
    pub notes: String,
    pub category: String,
    /// `YYYY-MM-DD`.
    pub due_day: Option<String>,
    /// `reply`, `reply_all`, `forward` or `none`.
    pub action: String,
    /// `open` or `done`.
    pub status: String,
    /// `ai` or `you`.
    pub source: String,
    pub why: String,
    pub created_at: Millis,
    pub completed_at: Option<Millis>,
    /// From the stored thread (empty once it has left the store).
    pub subject: String,
    pub from_name: Option<String>,
    pub from_email: Option<String>,
}

/// What a caller may set on a task; the rest is kept.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskFields {
    pub title: String,
    pub notes: String,
    pub category: String,
    pub due_day: Option<String>,
    pub action: String,
}

const SELECT: &str = "SELECT t.id, t.thread_id, t.message_id, t.title, t.notes, t.category, t.due_day, t.action,
       t.status, t.source, t.why, t.created_at, t.completed_at,
       COALESCE(NULLIF(th.subject, ''), m.subject, ''), m.from_name, m.from_email
  FROM tasks t
  LEFT JOIN threads th ON th.gmail_id = t.thread_id
  LEFT JOIN messages m ON m.id = COALESCE(
    (SELECT id FROM messages WHERE gmail_id = t.message_id),
    (SELECT id FROM messages WHERE thread_id = th.id AND is_draft = 0 ORDER BY internal_date DESC LIMIT 1))";

fn row(r: &Row<'_>) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: r.get(0)?,
        thread_id: r.get(1)?,
        message_id: r.get(2)?,
        title: r.get(3)?,
        notes: r.get(4)?,
        category: r.get(5)?,
        due_day: r.get(6)?,
        action: r.get(7)?,
        status: r.get(8)?,
        source: r.get(9)?,
        why: r.get(10)?,
        created_at: r.get(11)?,
        completed_at: r.get(12)?,
        subject: r.get(13)?,
        from_name: r.get(14)?,
        from_email: r.get(15)?,
    })
}

/// Add a task; returns its id.
#[allow(clippy::too_many_arguments)]
pub fn insert(
    tx: &Transaction<'_>,
    thread_id: &str,
    message_id: Option<&str>,
    fields: &TaskFields,
    source: &str,
    why: &str,
    now: Millis,
) -> StoreResult<i64> {
    tx.prepare_cached(
        "INSERT INTO tasks (thread_id, message_id, title, notes, category, due_day, action, source, why, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?
    .execute(params![
        thread_id,
        message_id,
        fields.title,
        fields.notes,
        fields.category,
        fields.due_day,
        fields.action,
        source,
        why,
        now
    ])?;
    Ok(tx.last_insert_rowid())
}

/// Change what the user may edit. False when there is no such task.
pub fn update(tx: &Transaction<'_>, id: i64, fields: &TaskFields) -> StoreResult<bool> {
    let n = tx
        .prepare_cached(
            "UPDATE tasks SET title = ?2, notes = ?3, category = ?4, due_day = ?5, action = ?6 WHERE id = ?1",
        )?
        .execute(params![id, fields.title, fields.notes, fields.category, fields.due_day, fields.action])?;
    Ok(n > 0)
}

/// Mark done (with the time) or open again. False when there is no such task.
pub fn set_done(tx: &Transaction<'_>, id: i64, done: bool, now: Millis) -> StoreResult<bool> {
    let n = if done {
        tx.prepare_cached("UPDATE tasks SET status = 'done', completed_at = ?2 WHERE id = ?1")?
            .execute(params![id, now])?
    } else {
        tx.prepare_cached("UPDATE tasks SET status = 'open', completed_at = NULL WHERE id = ?1")?.execute([id])?
    };
    Ok(n > 0)
}

pub fn delete(tx: &Transaction<'_>, id: i64) -> StoreResult<bool> {
    Ok(tx.prepare_cached("DELETE FROM tasks WHERE id = ?1")?.execute([id])? > 0)
}

/// Put back a deleted task as it was, id included (undo).
pub fn restore(tx: &Transaction<'_>, t: &TaskRow) -> StoreResult<()> {
    tx.prepare_cached(
        "INSERT INTO tasks (id, thread_id, message_id, title, notes, category, due_day, action, status, source,
           why, created_at, completed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
    )?
    .execute(params![
        t.id,
        t.thread_id,
        t.message_id,
        t.title,
        t.notes,
        t.category,
        t.due_day,
        t.action,
        t.status,
        t.source,
        t.why,
        t.created_at,
        t.completed_at
    ])?;
    Ok(())
}

pub fn get(conn: &Connection, id: i64) -> StoreResult<Option<TaskRow>> {
    Ok(conn.prepare_cached(&format!("{SELECT} WHERE t.id = ?1"))?.query_row([id], row).optional()?)
}

/// Open tasks first: dated ones by day, then undated, each oldest first.
/// With `include_done`, finished tasks follow, most recently finished first.
pub fn list(conn: &Connection, include_done: bool) -> StoreResult<Vec<TaskRow>> {
    let filter = if include_done { "" } else { "WHERE t.status = 'open'" };
    let sql = format!(
        "{SELECT} {filter}
         ORDER BY t.status = 'done', t.due_day IS NULL, t.due_day, t.completed_at DESC, t.created_at, t.id"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map([], row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The open tasks about a thread.
pub fn open_for_thread(conn: &Connection, thread_id: &str) -> StoreResult<Vec<TaskRow>> {
    let mut stmt =
        conn.prepare_cached(&format!("{SELECT} WHERE t.thread_id = ?1 AND t.status = 'open' ORDER BY t.id"))?;
    let rows = stmt.query_map([thread_id], row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Which of `thread_ids` have an open task.
pub fn threads_with_open_tasks(conn: &Connection, thread_ids: &[String]) -> StoreResult<Vec<String>> {
    let mut stmt = conn.prepare_cached("SELECT 1 FROM tasks WHERE thread_id = ?1 AND status = 'open' LIMIT 1")?;
    let mut out = Vec::new();
    for id in thread_ids {
        if stmt.exists([id])? {
            out.push(id.clone());
        }
    }
    Ok(out)
}

/// The account's categories in order.
pub fn categories(conn: &Connection) -> StoreResult<Vec<String>> {
    let mut stmt = conn.prepare_cached("SELECT name FROM task_categories ORDER BY position, name")?;
    let rows = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Replace the categories with `names`, in that order. Tasks keep their
/// category's name even when it leaves the list.
pub fn set_categories(tx: &Transaction<'_>, names: &[String]) -> StoreResult<()> {
    tx.execute("DELETE FROM task_categories", [])?;
    let mut stmt = tx.prepare_cached("INSERT OR IGNORE INTO task_categories (name, position) VALUES (?1, ?2)")?;
    for (i, name) in names.iter().enumerate() {
        stmt.execute(params![name, i as i64])?;
    }
    Ok(())
}

pub fn meta(conn: &Connection, key: &str) -> StoreResult<Option<String>> {
    Ok(conn.prepare_cached("SELECT value FROM task_meta WHERE key = ?1")?.query_row([key], |r| r.get(0)).optional()?)
}

pub fn set_meta(tx: &Transaction<'_>, key: &str, value: &str) -> StoreResult<()> {
    tx.prepare_cached("INSERT INTO task_meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2")?
        .execute(params![key, value])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    fn fields(title: &str, due: Option<&str>) -> TaskFields {
        TaskFields {
            title: title.into(),
            notes: String::new(),
            category: "Reply".into(),
            due_day: due.map(Into::into),
            action: "reply".into(),
        }
    }

    /// A scratch store, removed when dropped.
    struct Scratch(std::path::PathBuf, Db);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("openagc-store-tasks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("mail.sqlite")).unwrap();
        Scratch(dir, db)
    }

    #[test]
    fn tasks_list_open_by_day_then_undated_then_done() {
        let s = scratch("order");
        s.1.write_blocking(|tx| {
            insert(tx, "t1", None, &fields("later", Some("2026-10-05")), "ai", "", 1)?;
            insert(tx, "t2", None, &fields("undated", None), "you", "", 2)?;
            let done = insert(tx, "t3", None, &fields("finished", Some("2026-09-01")), "ai", "", 3)?;
            insert(tx, "t4", None, &fields("soon", Some("2026-09-30")), "ai", "why", 4)?;
            set_done(tx, done, true, 10)?;
            Ok(())
        })
        .unwrap();
        let open: Vec<String> = s.1.read_blocking(|c| list(c, false)).unwrap().into_iter().map(|t| t.title).collect();
        assert_eq!(open, ["soon", "later", "undated"]);
        let all = s.1.read_blocking(|c| list(c, true)).unwrap();
        assert_eq!(all.last().unwrap().title, "finished");
        assert_eq!(all.last().unwrap().completed_at, Some(10));
        let with = s.1.read_blocking(|c| threads_with_open_tasks(c, &["t1".into(), "t3".into()])).unwrap();
        assert_eq!(with, ["t1"]);
    }

    #[test]
    fn categories_start_with_the_defaults_and_can_be_replaced() {
        let s = scratch("categories");
        assert_eq!(s.1.read_blocking(categories).unwrap(), DEFAULT_CATEGORIES);
        s.1.write_blocking(|tx| set_categories(tx, &["Call".into(), "Reply".into(), "reply".into()])).unwrap();
        assert_eq!(s.1.read_blocking(categories).unwrap(), ["Call", "Reply"], "names are unique, any case");
        s.1.write_blocking(|tx| set_meta(tx, LABEL_KEY, "Label_7")).unwrap();
        s.1.write_blocking(|tx| set_meta(tx, LABEL_KEY, "Label_8")).unwrap();
        assert_eq!(s.1.read_blocking(|c| meta(c, LABEL_KEY)).unwrap().as_deref(), Some("Label_8"));
    }

    #[test]
    fn a_deleted_task_comes_back_as_it_was() {
        let s = scratch("restore");
        let id = s.1.write_blocking(|tx| insert(tx, "t1", Some("m1"), &fields("a", None), "ai", "because", 5)).unwrap();
        let before = s.1.read_blocking(move |c| get(c, id)).unwrap().unwrap();
        s.1.write_blocking(move |tx| delete(tx, id)).unwrap();
        assert!(s.1.read_blocking(move |c| get(c, id)).unwrap().is_none());
        let copy = before.clone();
        s.1.write_blocking(move |tx| restore(tx, &copy)).unwrap();
        assert_eq!(s.1.read_blocking(move |c| get(c, id)).unwrap(), Some(before));
    }
}
