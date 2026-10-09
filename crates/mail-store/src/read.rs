//! Read queries. All run on pooled read-only connections and never touch
//! the network (spec §13 rule 1).

use mail_domain::{
    Attachment, AttachmentId, Body, BodyState, EmailAddress, Label, LabelColor, LabelId, LabelKind, Mailbox,
    MailboxKind, Message, MessageId, ThreadId, ThreadSummary,
};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Deserialize;

use crate::error::{StoreError, StoreResult};
use crate::write::ARCHIVE_LABEL;

pub const DEFAULT_PAGE_SIZE: u32 = 100;
pub const MAX_PAGE_SIZE: u32 = 500;

/// A page of thread rows with an opaque cursor for the next one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPage {
    pub rows: Vec<ThreadSummary>,
    pub next_cursor: Option<String>,
}

pub fn list_labels(conn: &Connection) -> StoreResult<Vec<Label>> {
    let mut stmt = conn.prepare_cached(
        "SELECT gmail_id, name, kind, color_bg, color_fg, visible FROM labels WHERE kind != 'virtual'
         ORDER BY kind DESC, name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |r| {
        let bg: Option<String> = r.get(3)?;
        let fg: Option<String> = r.get(4)?;
        Ok(Label {
            id: LabelId(r.get(0)?),
            name: r.get(1)?,
            kind: if r.get::<_, String>(2)? == "user" { LabelKind::User } else { LabelKind::System },
            color: bg.zip(fg).map(|(background, text)| LabelColor { background, text }),
            visible: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Sidebar entries: system mailboxes in a fixed order, then visible user
/// labels by name. Counts come from `label_stats` and
/// `inbox_category_stats`, never `COUNT(*)`.
pub fn list_mailboxes(conn: &Connection) -> StoreResult<Vec<Mailbox>> {
    const SYSTEM: &[(MailboxKind, &str, &str)] = &[
        (MailboxKind::Inbox, "INBOX", "Inbox"),
        (MailboxKind::Starred, "STARRED", "Starred"),
        (MailboxKind::Important, "IMPORTANT", "Important"),
        (MailboxKind::Sent, "SENT", "Sent"),
        (MailboxKind::Drafts, "DRAFT", "Drafts"),
        (MailboxKind::Archive, ARCHIVE_LABEL, "Archive"),
        (MailboxKind::Spam, "SPAM", "Spam"),
        (MailboxKind::Trash, "TRASH", "Trash"),
    ];
    let mut counts = conn.prepare_cached(
        "SELECT COALESCE(s.thread_count, 0), COALESCE(s.unread_thread_count, 0)
         FROM labels l LEFT JOIN label_stats s ON s.label_id = l.id WHERE l.gmail_id = ?1",
    )?;
    let mut out = Vec::new();
    for (kind, id, name) in SYSTEM {
        let (total, mut unread): (i64, i64) =
            counts.query_row([id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?.unwrap_or((0, 0));
        // With categories, the Inbox counts Primary's unread, as Gmail's
        // own Inbox count does; promotions and notifications do not add up.
        if *kind == MailboxKind::Inbox {
            let categories = inbox_categories(conn, &[])?;
            if categories.iter().any(|c| c.id != PRIMARY && c.total > 0) {
                unread = categories.iter().find(|c| c.id == PRIMARY).map_or(0, |c| i64::from(c.unread));
            }
        }
        out.push(Mailbox {
            kind: *kind,
            label_id: (*kind != MailboxKind::Archive).then(|| LabelId::new(*id)),
            name: (*name).to_owned(),
            unread_count: unread.max(0) as u32,
            total_count: total.max(0) as u32,
        });
    }
    let mut user = conn.prepare_cached(
        "SELECT l.gmail_id, l.name, COALESCE(s.thread_count, 0), COALESCE(s.unread_thread_count, 0)
         FROM labels l LEFT JOIN label_stats s ON s.label_id = l.id
         WHERE l.kind = 'user' AND l.visible ORDER BY l.name COLLATE NOCASE",
    )?;
    let rows = user.query_map([], |r| {
        Ok(Mailbox {
            kind: MailboxKind::Label,
            label_id: Some(LabelId(r.get(0)?)),
            name: r.get(1)?,
            total_count: r.get::<_, i64>(2)?.max(0) as u32,
            unread_count: r.get::<_, i64>(3)?.max(0) as u32,
        })
    })?;
    for m in rows {
        out.push(m?);
    }
    Ok(out)
}

/// The mailbox id used by thread queries: a label id, or `@archive`.
pub fn mailbox_label(mailbox: &Mailbox) -> &str {
    match &mailbox.label_id {
        Some(l) => l.as_str(),
        None => ARCHIVE_LABEL,
    }
}

/// Gmail's categories other than Primary, in tab order (spec §14.3
/// amendment 2026-09-28, categories).
pub const CATEGORIES: &[&str] = &["CATEGORY_PROMOTIONS", "CATEGORY_SOCIAL", "CATEGORY_UPDATES", "CATEGORY_FORUMS"];
/// The Primary tab. As a narrowing it means "in none of `CATEGORIES`", so
/// Inbox mail that Gmail never categorised is Primary too.
pub const PRIMARY: &str = "CATEGORY_PERSONAL";

/// List filters (spec §14.3 amendment, filters), as narrowings: they test
/// the thread's own columns rather than a label.
pub const FILTER_UNREAD: &str = "@unread";
pub const FILTER_STARRED: &str = "@starred";
pub const FILTER_ATTACHMENTS: &str = "@attachments";

/// At most this many narrowings after the mailbox
/// (`INBOX+IMPORTANT+CATEGORY_SOCIAL+!Label_7+@unread+@starred+@attachments`).
const MAX_NARROWINGS: usize = 7;

/// Threads in a mailbox, newest first, keyset-paged: cost is O(page)
/// however deep the user scrolls (spec §4.2). `mailbox` is a label id,
/// optionally followed by `+`-joined labels the threads must also carry
/// (`INBOX+IMPORTANT`: the Inbox's "Important only" view;
/// `INBOX+CATEGORY_SOCIAL`: a category tab; `PRIMARY` narrows to threads
/// in no other category; `@unread`, `@starred` and `@attachments` are the
/// list filters; `!Label_7` excludes threads carrying that label, as the
/// Inbox hides emails with tasks). Ordered by the first label.
pub fn list_threads(conn: &Connection, mailbox: &str, cursor: Option<&str>, limit: u32) -> StoreResult<ThreadPage> {
    let limit = limit.clamp(1, MAX_PAGE_SIZE);
    let (after_at, after_id) = match cursor {
        Some(c) => decode_cursor(c)?,
        None => (i64::MAX, i64::MAX),
    };
    let mut parts = mailbox.split('+');
    let label = parts.next().unwrap_or_default();
    let narrowings: Vec<&str> = parts.collect();
    if narrowings.len() > MAX_NARROWINGS {
        return Err(StoreError::Invalid(format!("mailbox {mailbox:?} narrows too many times")));
    }
    let mut sql = format!(
        "SELECT t.id, t.gmail_id, t.subject, t.snippet, t.last_message_at, t.message_count, t.unread_count,
                t.has_attachments, t.is_starred, t.participants_json, t.label_ids_json, {REPLIED}, tl.last_message_at
         FROM thread_labels tl JOIN threads t ON t.id = tl.thread_id
         WHERE tl.label_id = (SELECT id FROM labels WHERE gmail_id = ?1)
           AND (tl.last_message_at, tl.thread_id) < (?2, ?3)"
    );
    let mut values: Vec<rusqlite::types::Value> =
        vec![label.to_owned().into(), after_at.into(), after_id.into(), i64::from(limit + 1).into()];
    for narrowing in &narrowings {
        sql.push_str(&narrowing_clause(narrowing, &mut values));
    }
    sql.push_str(" ORDER BY tl.last_message_at DESC, tl.thread_id DESC LIMIT ?4");
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut last_key = None;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(values), |r| {
            let key = (r.get::<_, i64>(12)?, r.get::<_, i64>(0)?);
            Ok((key, thread_summary(r, 1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = rows.len() > limit as usize;
    let mut out = Vec::with_capacity(limit as usize);
    for (key, row) in rows.into_iter().take(limit as usize) {
        last_key = Some(key);
        out.push(row?);
    }
    let next_cursor = if has_more { last_key.map(|(at, id)| encode_cursor(at, id)) } else { None };
    Ok(ThreadPage { rows: out, next_cursor })
}

/// The SQL that narrows `tl.thread_id` to threads also carrying `label`
/// (or, for `PRIMARY`, carrying no other category; for `!label`, not
/// carrying it), binding its values.
fn narrowing_clause(label: &str, values: &mut Vec<rusqlite::types::Value>) -> String {
    match label {
        FILTER_UNREAD => return " AND t.unread_count > 0".into(),
        FILTER_STARRED => return " AND t.is_starred".into(),
        FILTER_ATTACHMENTS => return " AND t.has_attachments".into(),
        _ => {}
    }
    if let Some(excluded) = label.strip_prefix('!') {
        values.push(excluded.to_owned().into());
        return format!(
            " AND NOT EXISTS (SELECT 1 FROM thread_labels n
                 WHERE n.thread_id = tl.thread_id AND n.label_id = (SELECT id FROM labels WHERE gmail_id = ?{}))",
            values.len()
        );
    }
    if label == PRIMARY {
        let first = values.len() + 1;
        values.extend(CATEGORIES.iter().map(|c| rusqlite::types::Value::from((*c).to_owned())));
        let placeholders = (first..first + CATEGORIES.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
        format!(
            " AND NOT EXISTS (SELECT 1 FROM thread_labels n JOIN labels nl ON nl.id = n.label_id
                 WHERE n.thread_id = tl.thread_id AND nl.gmail_id IN ({placeholders}))"
        )
    } else {
        values.push(label.to_owned().into());
        format!(
            " AND EXISTS (SELECT 1 FROM thread_labels n
                 WHERE n.thread_id = tl.thread_id AND n.label_id = (SELECT id FROM labels WHERE gmail_id = ?{}))",
            values.len()
        )
    }
}

/// Threads and unread threads in one Inbox category tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryCount {
    /// `PRIMARY` or one of `CATEGORIES`.
    pub id: String,
    pub total: u32,
    pub unread: u32,
}

/// The Inbox's category tabs with their counts, Primary first, then
/// `CATEGORIES` in order; `also` narrows as in `list_threads`
/// (`IMPORTANT` for Important-only, `!Label_7` to leave out emails with
/// tasks). Every tab is returned, empty ones with zero counts; a thread in
/// two categories counts in the first. Without narrowings the counts come
/// from `inbox_category_stats`; with them, from the Inbox's threads.
pub fn inbox_categories(conn: &Connection, also: &[&str]) -> StoreResult<Vec<CategoryCount>> {
    if also.is_empty() {
        return stored_inbox_categories(conn);
    }
    count_inbox_categories(conn, also)
}

/// `inbox_categories` without narrowings, from the maintained counts.
pub(crate) fn stored_inbox_categories(conn: &Connection) -> StoreResult<Vec<CategoryCount>> {
    let mut stmt =
        conn.prepare_cached("SELECT category, thread_count, unread_thread_count FROM inbox_category_stats")?;
    let stored = stmt
        .query_map([], |r| Ok((Some(r.get::<_, String>(0)?), r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(category_counts(&stored))
}

/// `inbox_categories` counted over the Inbox's threads.
pub(crate) fn count_inbox_categories(conn: &Connection, also: &[&str]) -> StoreResult<Vec<CategoryCount>> {
    let mut values: Vec<rusqlite::types::Value> =
        CATEGORIES.iter().map(|c| rusqlite::types::Value::from((*c).to_owned())).collect();
    let placeholders = (1..=CATEGORIES.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
    let order =
        CATEGORIES.iter().enumerate().map(|(i, _)| format!("WHEN ?{} THEN {i}", i + 1)).collect::<Vec<_>>().join(" ");
    let narrowing: String = also.iter().map(|label| narrowing_clause(label, &mut values)).collect();
    let sql = format!(
        "SELECT (SELECT cl.gmail_id FROM thread_labels c JOIN labels cl ON cl.id = c.label_id
                 WHERE c.thread_id = tl.thread_id AND cl.gmail_id IN ({placeholders})
                 ORDER BY CASE cl.gmail_id {order} END LIMIT 1) AS category,
                COUNT(*), COALESCE(SUM(t.unread_count > 0), 0)
         FROM thread_labels tl JOIN threads t ON t.id = tl.thread_id
         WHERE tl.label_id = (SELECT id FROM labels WHERE gmail_id = 'INBOX'){narrowing}
         GROUP BY category"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let counted = stmt
        .query_map(rusqlite::params_from_iter(values), |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(category_counts(&counted))
}

/// Every tab, Primary first, from `(category, threads, unread)` rows where
/// no category means Primary.
fn category_counts(rows: &[(Option<String>, i64, i64)]) -> Vec<CategoryCount> {
    std::iter::once(PRIMARY)
        .chain(CATEGORIES.iter().copied())
        .map(|id| {
            let found = rows.iter().find(|(c, _, _)| c.as_deref().unwrap_or(PRIMARY) == id);
            let (total, unread) = found.map(|(_, t, u)| (*t, *u)).unwrap_or((0, 0));
            CategoryCount { id: id.to_owned(), total: total.max(0) as u32, unread: unread.max(0) as u32 }
        })
        .collect()
}

/// Of `ids`, the stored messages in the Inbox that lack `label`.
pub fn inbox_messages_missing(conn: &Connection, ids: &[MessageId], label: &LabelId) -> StoreResult<Vec<MessageId>> {
    let mut stmt = conn.prepare_cached(
        "SELECT 1 FROM messages m
         WHERE m.gmail_id = ?1
           AND EXISTS(SELECT 1 FROM message_labels a JOIN labels l ON l.id = a.label_id
                      WHERE a.message_id = m.id AND l.gmail_id = 'INBOX')
           AND NOT EXISTS(SELECT 1 FROM message_labels a JOIN labels l ON l.id = a.label_id
                          WHERE a.message_id = m.id AND l.gmail_id = ?2)",
    )?;
    let mut out = Vec::new();
    for id in ids {
        if stmt.exists([id.as_str(), label.as_str()])? {
            out.push(id.clone());
        }
    }
    Ok(out)
}

pub fn get_thread_summary(conn: &Connection, id: &ThreadId) -> StoreResult<Option<ThreadSummary>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT t.id, t.gmail_id, t.subject, t.snippet, t.last_message_at, t.message_count, t.unread_count,
                    t.has_attachments, t.is_starred, t.participants_json, t.label_ids_json, {REPLIED}
             FROM threads t WHERE t.gmail_id = ?1"
    ))?;
    match stmt.query_row([id.as_str()], |r| thread_summary(r, 1)).optional()? {
        Some(summary) => Ok(Some(summary?)),
        None => Ok(None),
    }
}

/// A thread with its messages, oldest first.
pub fn get_thread(conn: &Connection, id: &ThreadId) -> StoreResult<Option<(ThreadSummary, Vec<Message>)>> {
    let Some(summary) = get_thread_summary(conn, id)? else { return Ok(None) };
    let mut stmt = conn.prepare_cached(
        "SELECT m.id, m.gmail_id, m.rfc822_message_id, m.in_reply_to, m.references_json, m.subject, m.date,
                m.internal_date, m.snippet, m.is_read, m.is_starred, m.is_draft, m.is_sent_by_me, m.body_state,
                m.size_estimate
         FROM messages m JOIN threads t ON t.id = m.thread_id WHERE t.gmail_id = ?1
         ORDER BY m.internal_date, m.id",
    )?;
    let rows = stmt
        .query_map([id.as_str()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                Message {
                    id: MessageId(r.get(1)?),
                    thread_id: id.clone(),
                    rfc822_message_id: r.get(2)?,
                    in_reply_to: r.get(3)?,
                    references: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
                    from: None,
                    to: vec![],
                    cc: vec![],
                    bcc: vec![],
                    reply_to: vec![],
                    subject: r.get(5)?,
                    date: r.get(6)?,
                    internal_date: r.get(7)?,
                    snippet: r.get(8)?,
                    is_read: r.get(9)?,
                    is_starred: r.get(10)?,
                    is_draft: r.get(11)?,
                    is_sent_by_me: r.get(12)?,
                    label_ids: vec![],
                    body_state: if r.get::<_, String>(13)? == "full" { BodyState::Full } else { BodyState::Metadata },
                    size_estimate: r.get::<_, i64>(14)?.max(0) as u64,
                    attachments: vec![],
                },
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

    let mut messages = Vec::with_capacity(rows.len());
    for (rowid, mut m) in rows {
        fill_participants(conn, rowid, &mut m)?;
        m.label_ids = conn
            .prepare_cached(
                "SELECT l.gmail_id FROM message_labels ml JOIN labels l ON l.id = ml.label_id
                 WHERE ml.message_id = ?1 ORDER BY l.gmail_id",
            )?
            .query_map([rowid], |r| Ok(LabelId(r.get(0)?)))?
            .collect::<Result<_, _>>()?;
        m.attachments = conn
            .prepare_cached(
                "SELECT id, filename, mime_type, size, content_id, is_inline FROM attachments
                 WHERE message_id = ?1 ORDER BY id",
            )?
            .query_map([rowid], |r| {
                Ok(Attachment {
                    id: AttachmentId(r.get::<_, i64>(0)?.to_string()),
                    filename: r.get(1)?,
                    mime_type: r.get(2)?,
                    size: r.get::<_, i64>(3)?.max(0) as u64,
                    content_id: r.get(4)?,
                    is_inline: r.get(5)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        messages.push(m);
    }
    Ok(Some((summary, messages)))
}

/// One message with its thread id, by provider message id.
pub fn get_message(conn: &Connection, id: &MessageId) -> StoreResult<Option<Message>> {
    let thread: Option<String> = conn
        .prepare_cached("SELECT t.gmail_id FROM messages m JOIN threads t ON t.id = m.thread_id WHERE m.gmail_id = ?1")?
        .query_row([id.as_str()], |r| r.get(0))
        .optional()?;
    let Some(thread) = thread else { return Ok(None) };
    Ok(get_thread(conn, &ThreadId(thread))?.and_then(|(_, messages)| messages.into_iter().find(|m| &m.id == id)))
}

/// The addresses the user sends from, learnt from mail they sent (the
/// account's address and any aliases), lowercased.
pub fn sent_from_addresses(conn: &Connection) -> StoreResult<Vec<String>> {
    Ok(conn
        .prepare_cached(
            "SELECT DISTINCT lower(from_email) FROM messages
             WHERE is_sent_by_me AND from_email IS NOT NULL AND from_email != '' LIMIT 50",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?)
}

/// The stored message (not a draft) with this RFC 5322 Message-ID.
pub fn message_for_rfc822(conn: &Connection, rfc822_id: &str) -> StoreResult<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT gmail_id FROM messages WHERE rfc822_message_id = ?1 AND NOT is_draft")?
        .query_row([rfc822_id], |r| r.get(0))
        .optional()?)
}

pub fn get_body(conn: &Connection, id: &MessageId) -> StoreResult<Option<Body>> {
    Ok(conn
        .prepare_cached(
            "SELECT b.text_plain, b.html_sanitized, b.has_remote_images
             FROM bodies b JOIN messages m ON m.id = b.message_id WHERE m.gmail_id = ?1",
        )?
        .query_row([id.as_str()], |r| {
            let html: Option<String> = r.get(1)?;
            Ok(Body { text_plain: r.get(0)?, html_sanitized: html.map(current_names), has_remote_images: r.get(2)? })
        })
        .optional()?)
}

/// Bodies sanitized before the project was named Kaluta carry its old
/// image schemes and quote class; readers know only the new ones.
fn current_names(html: String) -> String {
    if !html.contains("openagc-") {
        return html;
    }
    html.replace("\"openagc-cid:", "\"kaluta-cid:")
        .replace("\"openagc-remote:", "\"kaluta-remote:")
        .replace("class=\"openagc-quote\"", "class=\"kaluta-quote\"")
}

/// Recipient suggestions for the composer: people the user writes to most
/// and most recently first (spec §14.5). Three or more characters match
/// anywhere in a name or address (trigram index); fewer match a prefix.
pub fn suggest_contacts(conn: &Connection, text: &str, limit: u32) -> StoreResult<Vec<EmailAddress>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(vec![]);
    }
    let order = "ORDER BY (c.sent_count * 3 + c.received_count) DESC, c.last_seen DESC LIMIT ?2";
    let rows = if text.chars().count() >= 3 {
        let quoted = format!("\"{}\"", text.replace('"', ""));
        conn.prepare_cached(&format!(
            "SELECT c.name, c.email FROM contacts_fts f JOIN contacts c ON c.id = f.rowid
             WHERE contacts_fts MATCH ?1 {order}"
        ))?
        .query_map(params![quoted, limit], |r| Ok(EmailAddress { name: r.get(0)?, email: r.get(1)? }))?
        .collect::<Result<Vec<_>, _>>()?
    } else {
        let like = format!("{}%", text.replace(['%', '_'], ""));
        conn.prepare_cached(&format!(
            "SELECT c.name, c.email FROM contacts c WHERE c.email LIKE ?1 OR c.name LIKE ?1 {order}"
        ))?
        .query_map(params![like, limit], |r| Ok(EmailAddress { name: r.get(0)?, email: r.get(1)? }))?
        .collect::<Result<Vec<_>, _>>()?
    };
    Ok(rows)
}

/// Text of the newest messages from `domain` (or a subdomain of it)
/// received at or after `since`: subject, then body or snippet. For
/// finding a verification code the user asked for (spec §7.9).
pub fn recent_text_from_domain(conn: &Connection, domain: &str, since: i64, limit: u32) -> StoreResult<Vec<String>> {
    let domain = domain.to_lowercase();
    // Narrowed by the address's end in SQL, so a busy inbox cannot hide
    // the code; the exact host is checked below.
    let mut stmt = conn.prepare_cached(
        "SELECT m.from_email, m.subject, m.snippet, b.text_plain FROM messages m
         LEFT JOIN bodies b ON b.message_id = m.id
         WHERE m.internal_date >= ?1 AND m.is_sent_by_me = 0 AND lower(m.from_email) LIKE ?2
         ORDER BY m.internal_date DESC LIMIT 50",
    )?;
    let rows = stmt.query_map(params![since, format!("%{domain}")], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (from, subject, snippet, text) = row?;
        let host = from.rsplit_once('@').map(|(_, h)| h.to_lowercase()).unwrap_or_default();
        if host == domain || host.ends_with(&format!(".{domain}")) {
            out.push(format!("{subject}\n{}", text.unwrap_or(snippet)));
            if out.len() >= limit as usize {
                break;
            }
        }
    }
    Ok(out)
}

pub fn sync_state(conn: &Connection, key: &str) -> StoreResult<Option<String>> {
    Ok(conn.prepare_cached("SELECT value FROM sync_state WHERE key = ?1")?.query_row([key], |r| r.get(0)).optional()?)
}

pub fn set_sync_state(tx: &rusqlite::Transaction<'_>, key: &str, value: &str) -> StoreResult<()> {
    tx.prepare_cached(
        "INSERT INTO sync_state (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )?
    .execute([key, value])?;
    Ok(())
}

// --- helpers --------------------------------------------------------------

#[derive(Deserialize)]
struct ParticipantJson {
    name: Option<String>,
    email: String,
}

/// Whether you replied in thread `t`: a message you sent after its first.
pub(crate) const REPLIED: &str = "EXISTS(SELECT 1 FROM messages rm WHERE rm.thread_id = t.id AND rm.is_sent_by_me
       AND rm.internal_date > (SELECT MIN(fm.internal_date) FROM messages fm WHERE fm.thread_id = t.id))";

/// A summary from a row shaped `id, gmail_id, subject, … label_ids_json,
/// replied`.
pub(crate) fn thread_summary_row(r: &Row<'_>) -> rusqlite::Result<StoreResult<ThreadSummary>> {
    thread_summary(r, 1)
}

/// Build a summary from columns starting at `offset` (gmail_id first).
fn thread_summary(r: &Row<'_>, offset: usize) -> rusqlite::Result<StoreResult<ThreadSummary>> {
    let participants: String = r.get(offset + 8)?;
    let labels: String = r.get(offset + 9)?;
    let parse = || -> StoreResult<ThreadSummary> {
        let participants: Vec<ParticipantJson> = serde_json::from_str(&participants)?;
        let labels: Vec<String> = serde_json::from_str(&labels)?;
        Ok(ThreadSummary {
            id: ThreadId(r.get(offset)?),
            subject: r.get(offset + 1)?,
            snippet: r.get(offset + 2)?,
            last_message_at: r.get(offset + 3)?,
            message_count: r.get::<_, i64>(offset + 4)?.max(0) as u32,
            unread_count: r.get::<_, i64>(offset + 5)?.max(0) as u32,
            has_attachments: r.get(offset + 6)?,
            is_starred: r.get(offset + 7)?,
            participants: participants.into_iter().map(|p| EmailAddress { name: p.name, email: p.email }).collect(),
            label_ids: labels.into_iter().map(LabelId).collect(),
            replied: r.get(offset + 10)?,
        })
    };
    Ok(parse())
}

fn fill_participants(conn: &Connection, message_rowid: i64, m: &mut Message) -> StoreResult<()> {
    let mut stmt = conn
        .prepare_cached("SELECT role, name, email FROM participants WHERE message_id = ?1 ORDER BY role, position")?;
    let rows = stmt.query_map([message_rowid], |r| {
        Ok((r.get::<_, String>(0)?, EmailAddress { name: r.get(1)?, email: r.get(2)? }))
    })?;
    for row in rows {
        let (role, addr) = row?;
        match role.as_str() {
            "from" => m.from = Some(addr),
            "to" => m.to.push(addr),
            "cc" => m.cc.push(addr),
            "bcc" => m.bcc.push(addr),
            "reply_to" => m.reply_to.push(addr),
            _ => {}
        }
    }
    Ok(())
}

pub(crate) fn encode_cursor(at: i64, id: i64) -> String {
    format!("{at}:{id}")
}

pub(crate) fn decode_cursor(c: &str) -> StoreResult<(i64, i64)> {
    let (a, b) = c.split_once(':').ok_or_else(|| StoreError::Invalid(format!("bad cursor {c:?}")))?;
    let parse = |s: &str| s.parse::<i64>().map_err(|_| StoreError::Invalid(format!("bad cursor {c:?}")));
    Ok((parse(a)?, parse(b)?))
}

/// Everything needed to produce an attachment's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentSource {
    pub message_id: MessageId,
    pub part_id: Option<String>,
    pub provider_attachment_id: Option<String>,
    pub filename: String,
    pub mime_type: String,
    pub content_id: Option<String>,
    pub data: Option<Vec<u8>>,
}

/// An attachment by the id the UI was given (its row id).
pub fn attachment_source(conn: &Connection, id: i64) -> StoreResult<Option<AttachmentSource>> {
    Ok(conn
        .prepare_cached(
            "SELECT m.gmail_id, a.part_id, a.gmail_attachment_id, a.filename, a.mime_type, a.content_id, a.data
             FROM attachments a JOIN messages m ON m.id = a.message_id WHERE a.id = ?1",
        )?
        .query_row([id], |r| {
            Ok(AttachmentSource {
                message_id: MessageId(r.get(0)?),
                part_id: r.get(1)?,
                provider_attachment_id: r.get(2)?,
                filename: r.get(3)?,
                mime_type: r.get(4)?,
                content_id: r.get(5)?,
                data: r.get(6)?,
            })
        })
        .optional()?)
}

/// Whether any message is stored with headers only (tiered download).
pub fn has_header_only(conn: &Connection) -> StoreResult<bool> {
    Ok(conn.query_row("SELECT EXISTS (SELECT 1 FROM messages WHERE body_state = 'metadata')", [], |r| r.get(0))?)
}
