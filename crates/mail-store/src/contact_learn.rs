//! What learning about contacts reads (spec §14.16): the conversations
//! the daily job has not read yet, the mail between the user and one
//! contact for a deep dive, and the people written to in a period.

use mail_domain::{Millis, SenderAuth};
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::StoreResult;

fn list(emails: &[String]) -> String {
    serde_json::to_string(&emails.iter().map(|e| e.trim().to_lowercase()).collect::<Vec<_>>())
        .unwrap_or_else(|_| "[]".into())
}

/// A message is someone's the user has written to: a participant who is
/// not the user and has `contacts.sent_count > 0`.
const WITH_A_CONTACT: &str = "EXISTS (SELECT 1 FROM messages m2
                JOIN participants p ON p.message_id = m2.id
                JOIN contacts c ON c.email = p.email
               WHERE m2.thread_id = m.thread_id AND p.role IN ('from', 'to', 'cc', 'bcc') AND c.sent_count > 0
                 AND lower(p.email) NOT IN (SELECT value FROM json_each(?2)))";

/// Conversations with a message that arrived at or after `since` which
/// the daily job has not read, and with someone the user has written to
/// (`own`: the account's addresses, who do not count). Oldest unread
/// first, so nothing is skipped when a day's limit is reached.
pub fn unread_conversations(
    conn: &Connection,
    since: Millis,
    own: &[String],
    limit: usize,
) -> StoreResult<Vec<String>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT th.gmail_id, MIN(m.internal_date) AS first_new
               FROM messages m JOIN threads th ON th.id = m.thread_id
              WHERE m.internal_date >= ?1 AND m.is_draft = 0 AND m.gmail_id NOT LIKE 'local-%'
                AND NOT EXISTS (SELECT 1 FROM contact_reads r WHERE r.message_id = m.gmail_id AND r.person_id = 0)
                AND {WITH_A_CONTACT}
              GROUP BY th.id
              ORDER BY first_new, th.id
              LIMIT ?3"
        ))?
        .query_map(params![since, list(own), limit as i64], |r| r.get(0))?
        .collect::<Result<_, _>>()?)
}

/// How many such conversations gained mail between two times, whether
/// read or not (for the estimate shown before learning is turned on).
pub fn conversations_with_contacts_between(
    conn: &Connection,
    from: Millis,
    to: Millis,
    own: &[String],
) -> StoreResult<i64> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT COUNT(DISTINCT m.thread_id) FROM messages m
              WHERE m.internal_date >= ?1 AND m.internal_date < ?3 AND m.is_draft = 0
                AND m.gmail_id NOT LIKE 'local-%' AND {WITH_A_CONTACT}"
        ))?
        .query_row(params![from, list(own), to], |r| r.get(0))?)
}

/// What the receiving server concluded about a message's sender.
pub fn sender_auth(conn: &Connection, message_id: &str) -> StoreResult<SenderAuth> {
    let stored: Option<Option<String>> = conn
        .prepare_cached("SELECT sender_auth FROM messages WHERE gmail_id = ?1")?
        .query_row([message_id], |r| r.get(0))
        .optional()?;
    Ok(SenderAuth::parse(stored.flatten().as_deref()))
}

/// Whom a stored message was sent to (To and Cc), in order.
pub fn recipients_of(conn: &Connection, message_id: &str) -> StoreResult<Vec<String>> {
    Ok(conn
        .prepare_cached(
            "SELECT p.email FROM participants p JOIN messages m ON m.id = p.message_id
              WHERE m.gmail_id = ?1 AND p.role IN ('to', 'cc')
              ORDER BY p.role = 'cc', p.position",
        )?
        .query_map([message_id], |r| r.get(0))?
        .collect::<Result<_, _>>()?)
}

/// The conversation a stored message is in.
pub fn thread_of(conn: &Connection, message_id: &str) -> StoreResult<Option<String>> {
    Ok(conn
        .prepare_cached(
            "SELECT th.gmail_id FROM messages m JOIN threads th ON th.id = m.thread_id WHERE m.gmail_id = ?1",
        )?
        .query_row([message_id], |r| r.get(0))
        .optional()?)
}

/// The messages between the user and any of `emails` (from, to, cc or
/// bcc): the newest `max`, returned oldest first, as (message id, thread
/// id, date).
pub fn messages_with(conn: &Connection, emails: &[String], max: usize) -> StoreResult<Vec<(String, String, Millis)>> {
    let mut newest: Vec<(String, String, Millis)> = conn
        .prepare_cached(
            "SELECT m.gmail_id, th.gmail_id, m.internal_date
               FROM messages m JOIN threads th ON th.id = m.thread_id
              WHERE m.is_draft = 0 AND m.gmail_id NOT LIKE 'local-%'
                AND EXISTS (SELECT 1 FROM participants p
                             WHERE p.message_id = m.id AND p.role IN ('from', 'to', 'cc', 'bcc')
                               AND lower(p.email) IN (SELECT value FROM json_each(?1)))
              ORDER BY m.internal_date DESC, m.id DESC
              LIMIT ?2",
        )?
        .query_map(params![list(emails), max as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    newest.reverse();
    Ok(newest)
}

/// The addresses the user sent mail to (To, Cc or Bcc) at or after
/// `since`, with how many messages, the most written to first. `own`
/// addresses are left out.
pub fn written_to_since(conn: &Connection, since: Millis, own: &[String]) -> StoreResult<Vec<(String, i64)>> {
    Ok(conn
        .prepare_cached(
            "SELECT lower(p.email) AS email, COUNT(DISTINCT m.id) AS n
               FROM messages m JOIN participants p ON p.message_id = m.id
              WHERE m.is_sent_by_me AND m.is_draft = 0 AND m.internal_date >= ?1
                AND p.role IN ('to', 'cc', 'bcc')
                AND lower(p.email) NOT IN (SELECT value FROM json_each(?2))
              GROUP BY lower(p.email)
              ORDER BY n DESC, email",
        )?
        .query_map(params![since, list(own)], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?)
}
