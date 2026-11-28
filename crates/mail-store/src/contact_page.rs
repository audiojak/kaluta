//! What a contact's page shows beside their facts (spec §14.16): the
//! replies the user is waiting for from them, the tasks made from their
//! mail and their recent conversations. Found by query each time, from
//! Waiting's and Tasks' own tables: nothing is copied.

use mail_domain::Millis;
use rusqlite::{Connection, params};

use crate::error::StoreResult;

/// A conversation where the user waits for this contact's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitRef {
    pub thread_id: String,
    pub message_id: String,
    pub subject: String,
    pub since: Millis,
}

/// A task on a conversation this contact wrote in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRef {
    pub id: i64,
    pub title: String,
    pub done: bool,
    pub thread_id: String,
    pub due_day: Option<String>,
    pub created_at: Millis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRef {
    pub thread_id: String,
    pub subject: String,
    /// Their or the user's latest message in it that names them.
    pub latest: Millis,
}

/// How much mail there is with a contact, and when.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MailStats {
    pub first: Option<Millis>,
    pub latest: Option<Millis>,
    /// Messages the user sent them.
    pub sent: i64,
    /// Messages they sent.
    pub received: i64,
}

fn list(emails: &[String]) -> String {
    serde_json::to_string(&emails.iter().map(|e| e.trim().to_lowercase()).collect::<Vec<_>>())
        .unwrap_or_else(|_| "[]".into())
}

/// Waits (state *waiting*) whose sent message was to one of the addresses,
/// longest first.
pub fn waiting_on(conn: &Connection, emails: &[String]) -> StoreResult<Vec<WaitRef>> {
    Ok(conn
        .prepare_cached(
            "SELECT w.thread_id, w.message_id, COALESCE(t.subject, m.subject, ''), w.since
               FROM waiting w
               JOIN messages m ON m.gmail_id = w.message_id
               LEFT JOIN threads t ON t.gmail_id = w.thread_id
              WHERE w.state = 'waiting'
                AND EXISTS (SELECT 1 FROM participants p
                             WHERE p.message_id = m.id AND p.role IN ('to', 'cc', 'bcc')
                               AND lower(p.email) IN (SELECT value FROM json_each(?1)))
              ORDER BY w.since, w.thread_id",
        )?
        .query_map([list(emails)], |r| {
            Ok(WaitRef { thread_id: r.get(0)?, message_id: r.get(1)?, subject: r.get(2)?, since: r.get(3)? })
        })?
        .collect::<Result<_, _>>()?)
}

/// The user's tasks from this contact's mail: by the task's own message
/// when it is stored, else any message of theirs in the task's
/// conversations. Open first, then newest.
pub fn tasks_from(conn: &Connection, emails: &[String]) -> StoreResult<Vec<TaskRef>> {
    Ok(conn
        .prepare_cached(
            "SELECT t.id, t.title, t.status = 'done', t.thread_id, t.due_day, t.created_at
               FROM tasks t
              WHERE CASE
                WHEN EXISTS (SELECT 1 FROM messages m WHERE m.gmail_id = t.message_id) THEN
                  EXISTS (SELECT 1 FROM messages m
                           WHERE m.gmail_id = t.message_id
                             AND lower(m.from_email) IN (SELECT value FROM json_each(?1)))
                ELSE
                  EXISTS (SELECT 1 FROM messages m JOIN threads th ON th.id = m.thread_id
                           WHERE (th.gmail_id = t.thread_id
                                  OR th.gmail_id IN (SELECT thread_id FROM task_threads WHERE task_id = t.id))
                             AND m.is_draft = 0
                             AND lower(m.from_email) IN (SELECT value FROM json_each(?1)))
                END
              ORDER BY t.status = 'done', t.created_at DESC, t.id DESC",
        )?
        .query_map([list(emails)], |r| {
            Ok(TaskRef {
                id: r.get(0)?,
                title: r.get(1)?,
                done: r.get(2)?,
                thread_id: r.get(3)?,
                due_day: r.get(4)?,
                created_at: r.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

/// The latest conversations any of the addresses takes part in.
pub fn recent_conversations(conn: &Connection, emails: &[String], limit: usize) -> StoreResult<Vec<ConversationRef>> {
    Ok(conn
        .prepare_cached(
            "SELECT th.gmail_id, th.subject, MAX(m.internal_date) AS latest
               FROM participants p
               JOIN messages m ON m.id = p.message_id
               JOIN threads th ON th.id = m.thread_id
              WHERE p.role IN ('from', 'to', 'cc', 'bcc') AND m.is_draft = 0
                AND lower(p.email) IN (SELECT value FROM json_each(?1))
              GROUP BY th.id
              ORDER BY latest DESC, th.id DESC
              LIMIT ?2",
        )?
        .query_map(params![list(emails), limit as i64], |r| {
            Ok(ConversationRef { thread_id: r.get(0)?, subject: r.get(1)?, latest: r.get(2)? })
        })?
        .collect::<Result<_, _>>()?)
}

/// First and latest mail with the addresses, and how many each way.
pub fn mail_stats(conn: &Connection, emails: &[String]) -> StoreResult<MailStats> {
    Ok(conn
        .prepare_cached(
            "SELECT MIN(m.internal_date), MAX(m.internal_date),
                    COUNT(DISTINCT CASE WHEN m.is_sent_by_me AND p.role IN ('to', 'cc', 'bcc') THEN m.id END),
                    COUNT(DISTINCT CASE WHEN NOT m.is_sent_by_me AND p.role = 'from' THEN m.id END)
               FROM participants p JOIN messages m ON m.id = p.message_id
              WHERE p.role IN ('from', 'to', 'cc', 'bcc') AND m.is_draft = 0
                AND lower(p.email) IN (SELECT value FROM json_each(?1))",
        )?
        .query_row([list(emails)], |r| {
            Ok(MailStats { first: r.get(0)?, latest: r.get(1)?, sent: r.get(2)?, received: r.get(3)? })
        })?)
}
