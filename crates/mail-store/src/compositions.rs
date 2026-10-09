//! AI compositions (spec §14.10, ADR 0013): every text an AI writes into a
//! draft, kept with its full text so the daily review can compare it with
//! what the user sent. One record per draft while the draft exists: a later
//! AI text for the same draft replaces the record's text. The record
//! outlives the draft: sending copies the draft's Message-ID into it, and
//! discarding marks it discarded.

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::error::StoreResult;

/// Where the text came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    WritingHelp,
    Agent,
    Routine,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WritingHelp => "writing_help",
            Self::Agent => "agent",
            Self::Routine => "routine",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "agent" => Self::Agent,
            "routine" => Self::Routine,
            _ => Self::WritingHelp,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Reply,
    Forward,
    New,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::Forward => "forward",
            Self::New => "new",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "reply" => Self::Reply,
            "forward" => Self::Forward,
            _ => Self::New,
        }
    }

    /// A draft's kind from its subject and whether it answers a message.
    pub fn of(subject: &str, replying: bool) -> Self {
        let subject = subject.trim().to_lowercase();
        if subject.starts_with("fwd:") || subject.starts_with("fw:") {
            Self::Forward
        } else if replying {
            Self::Reply
        } else {
            Self::New
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Waiting,
    Matched,
    Unmatched,
    Discarded,
    Reviewed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Matched => "matched",
            Self::Unmatched => "unmatched",
            Self::Discarded => "discarded",
            Self::Reviewed => "reviewed",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "matched" => Self::Matched,
            "unmatched" => Self::Unmatched,
            "discarded" => Self::Discarded,
            "reviewed" => Self::Reviewed,
            _ => Self::Waiting,
        }
    }
}

/// To and Cc, lowercased.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recipients {
    pub to: Vec<String>,
    pub cc: Vec<String>,
}

impl Recipients {
    pub fn new<'a>(to: impl IntoIterator<Item = &'a str>, cc: impl IntoIterator<Item = &'a str>) -> Self {
        let norm = |v: &str| v.trim().to_lowercase();
        Self { to: to.into_iter().map(norm).collect(), cc: cc.into_iter().map(norm).collect() }
    }
}

/// What an AI wrote into a draft.
#[derive(Debug, Clone, PartialEq)]
pub struct NewComposition {
    pub source: Source,
    pub agent: Option<String>,
    pub kind: Kind,
    pub draft_id: i64,
    pub thread_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub recipients: Recipients,
    pub subject: String,
    /// The user's request (writing help) or the agent's prompt.
    pub instruction: String,
    /// The text the AI wrote, as plain text.
    pub ai_text: String,
    pub ai_html: Option<String>,
    pub guide_version: Option<i64>,
    pub audiences: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Composition {
    pub id: i64,
    pub created_at: Millis,
    pub updated_at: Millis,
    pub source: Source,
    pub agent: Option<String>,
    pub kind: Kind,
    pub draft_id: Option<i64>,
    pub thread_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub recipients: Recipients,
    pub subject: String,
    pub instruction: String,
    /// `None` once purged (retention, ADR 0013).
    pub ai_text: Option<String>,
    pub ai_html: Option<String>,
    pub guide_version: Option<i64>,
    pub audiences: Vec<String>,
    pub rfc822_message_id: Option<String>,
    pub status: Status,
    pub matched_message_id: Option<String>,
    pub match_method: Option<String>,
    pub sent_text: Option<String>,
    pub distance: Option<f64>,
    pub reviewed_at: Option<Millis>,
}

const COLUMNS: &str = "id, created_at, updated_at, source, agent, kind, draft_id, thread_id, in_reply_to, \
                       recipients_json, subject, instruction, ai_text, ai_html, guide_version, audiences_json, \
                       rfc822_message_id, status, matched_message_id, match_method, sent_text, distance, reviewed_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<Composition> {
    let recipients: String = r.get(9)?;
    let audiences: String = r.get(15)?;
    Ok(Composition {
        id: r.get(0)?,
        created_at: r.get(1)?,
        updated_at: r.get(2)?,
        source: Source::parse(&r.get::<_, String>(3)?),
        agent: r.get(4)?,
        kind: Kind::parse(&r.get::<_, String>(5)?),
        draft_id: r.get(6)?,
        thread_id: r.get(7)?,
        in_reply_to: r.get(8)?,
        recipients: serde_json::from_str(&recipients).unwrap_or_default(),
        subject: r.get(10)?,
        instruction: r.get(11)?,
        ai_text: r.get(12)?,
        ai_html: r.get(13)?,
        guide_version: r.get(14)?,
        audiences: serde_json::from_str(&audiences).unwrap_or_default(),
        rfc822_message_id: r.get(16)?,
        status: Status::parse(&r.get::<_, String>(17)?),
        matched_message_id: r.get(18)?,
        match_method: r.get(19)?,
        sent_text: r.get(20)?,
        distance: r.get(21)?,
        reviewed_at: r.get(22)?,
    })
}

/// Whether the account records compositions: only once it has finished a
/// writing-guide learning run (spec §14.10).
pub fn recording(conn: &Connection) -> StoreResult<bool> {
    Ok(conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM guide_runs WHERE status = 'done')")?
        .query_row([], |r| r.get(0))?)
}

/// Record what an AI wrote into a draft. A record still waiting for the
/// same draft is replaced, so the last AI text before the user's edits is
/// what is kept; returns the record's id.
pub fn record(tx: &Transaction<'_>, c: &NewComposition, now: Millis) -> StoreResult<i64> {
    let recipients = serde_json::to_string(&c.recipients)?;
    let audiences = serde_json::to_string(&c.audiences)?;
    let existing: Option<i64> = tx
        .prepare_cached("SELECT id FROM ai_compositions WHERE draft_id = ?1 AND status = 'waiting' ORDER BY id DESC")?
        .query_row([c.draft_id], |r| r.get(0))
        .optional()?;
    if let Some(id) = existing {
        tx.prepare_cached(
            "UPDATE ai_compositions SET updated_at = ?2, source = ?3, agent = ?4, kind = ?5, thread_id = ?6,
               in_reply_to = ?7, recipients_json = ?8, subject = ?9, instruction = ?10, ai_text = ?11, ai_html = ?12,
               guide_version = ?13, audiences_json = ?14
             WHERE id = ?1",
        )?
        .execute(params![
            id,
            now,
            c.source.as_str(),
            c.agent,
            c.kind.as_str(),
            c.thread_id,
            c.in_reply_to,
            recipients,
            c.subject,
            c.instruction,
            c.ai_text,
            c.ai_html,
            c.guide_version,
            audiences
        ])?;
        return Ok(id);
    }
    tx.prepare_cached(
        "INSERT INTO ai_compositions (created_at, updated_at, source, agent, kind, draft_id, thread_id, in_reply_to,
           recipients_json, subject, instruction, ai_text, ai_html, guide_version, audiences_json)
         VALUES (?1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
    )?
    .execute(params![
        now,
        c.source.as_str(),
        c.agent,
        c.kind.as_str(),
        c.draft_id,
        c.thread_id,
        c.in_reply_to,
        recipients,
        c.subject,
        c.instruction,
        c.ai_text,
        c.ai_html,
        c.guide_version,
        audiences
    ])?;
    Ok(tx.last_insert_rowid())
}

/// What a cloud agent reported sending (spec §10.6): recorded as its AI
/// composition, with no draft (it was written and sent elsewhere).
#[derive(Debug, Clone, PartialEq)]
pub struct ReportedComposition {
    /// `cloud:<agent name>`.
    pub agent: String,
    pub kind: Kind,
    pub recipients: Recipients,
    pub subject: String,
    /// The body as plain text, and as HTML.
    pub ai_text: String,
    pub ai_html: Option<String>,
    /// The Message-ID the report names, if any, without brackets: the
    /// daily review pairs the record with the sent message that has it.
    pub rfc822_message_id: Option<String>,
    /// When it was sent, as the report says.
    pub at: Millis,
}

/// Record a cloud agent's reported send; returns the record's id.
pub fn record_reported(tx: &Transaction<'_>, c: &ReportedComposition, now: Millis) -> StoreResult<i64> {
    tx.prepare_cached(
        "INSERT INTO ai_compositions (created_at, updated_at, source, agent, kind, recipients_json, subject,
           ai_text, ai_html, rfc822_message_id)
         VALUES (?1, ?2, 'agent', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?
    .execute(params![
        c.at,
        now,
        c.agent,
        c.kind.as_str(),
        serde_json::to_string(&c.recipients)?,
        c.subject,
        c.ai_text,
        c.ai_html,
        c.rfc822_message_id
    ])?;
    Ok(tx.last_insert_rowid())
}

/// Whether a record is a cloud agent's report: those are linked to their
/// sent message by the report's matching (spec §10.6), never guessed at.
pub fn is_reported(c: &Composition) -> bool {
    c.draft_id.is_none() && c.agent.as_deref().is_some_and(|a| a.starts_with(CLOUD_AGENT_PREFIX))
}

/// A cloud agent's records are `cloud:<its name>`.
pub const CLOUD_AGENT_PREFIX: &str = "cloud:";

/// The draft's recipients or subject changed without new AI text.
pub fn update_addressing(
    tx: &Transaction<'_>,
    draft_id: i64,
    recipients: &Recipients,
    subject: &str,
    now: Millis,
) -> StoreResult<()> {
    tx.prepare_cached(
        "UPDATE ai_compositions SET recipients_json = ?2, subject = ?3, updated_at = ?4
         WHERE draft_id = ?1 AND status = 'waiting'",
    )?
    .execute(params![draft_id, serde_json::to_string(recipients)?, subject, now])?;
    Ok(())
}

/// The draft is being sent with this Message-ID: the strong link to the
/// sent copy. A send that fails or is undone and sent again replaces it.
pub fn draft_sent(tx: &Transaction<'_>, draft_id: i64, rfc822_message_id: &str) -> StoreResult<()> {
    tx.prepare_cached("UPDATE ai_compositions SET rfc822_message_id = ?2 WHERE draft_id = ?1 AND status = 'waiting'")?
        .execute(params![draft_id, rfc822_message_id])?;
    Ok(())
}

/// A send was taken back (Undo Send): the record forgets its Message-ID.
pub fn draft_unsent(tx: &Transaction<'_>, draft_id: i64) -> StoreResult<()> {
    tx.prepare_cached(
        "UPDATE ai_compositions SET rfc822_message_id = NULL WHERE draft_id = ?1 AND status = 'waiting'",
    )?
    .execute([draft_id])?;
    Ok(())
}

/// The draft row is going: sent (its record waits for the sent copy) or
/// discarded (counted, never compared). The record lets go of the draft
/// id, which SQLite may hand to a new draft.
pub fn draft_gone(tx: &Transaction<'_>, draft_id: i64, sent: bool, now: Millis) -> StoreResult<()> {
    if !sent {
        // A draft whose send failed (as far as we know) may have gone out:
        // with a Message-ID it waits for that copy instead.
        tx.prepare_cached(
            "UPDATE ai_compositions SET status = 'discarded', updated_at = ?2
             WHERE draft_id = ?1 AND status = 'waiting' AND rfc822_message_id IS NULL",
        )?
        .execute(params![draft_id, now])?;
    }
    tx.prepare_cached("UPDATE ai_compositions SET draft_id = NULL WHERE draft_id = ?1")?.execute([draft_id])?;
    Ok(())
}

pub fn get(conn: &Connection, id: i64) -> StoreResult<Option<Composition>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {COLUMNS} FROM ai_compositions WHERE id = ?1"))?
        .query_row([id], from_row)
        .optional()?)
}

/// The record for a draft that still exists.
pub fn for_draft(conn: &Connection, draft_id: i64) -> StoreResult<Option<Composition>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM ai_compositions WHERE draft_id = ?1 AND status = 'waiting' ORDER BY id DESC"
        ))?
        .query_row([draft_id], from_row)
        .optional()?)
}

/// The latest records, newest first.
pub fn recent(conn: &Connection, limit: u32) -> StoreResult<Vec<Composition>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {COLUMNS} FROM ai_compositions ORDER BY created_at DESC, id DESC LIMIT ?1"))?
        .query_map([limit], from_row)?
        .collect::<Result<_, _>>()?)
}

/// Records in `status`, oldest first.
pub fn list(conn: &Connection, status: Status, limit: u32) -> StoreResult<Vec<Composition>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM ai_compositions WHERE status = ?1 ORDER BY created_at, id LIMIT ?2"
        ))?
        .query_map(params![status.as_str(), limit], from_row)?
        .collect::<Result<_, _>>()?)
}

/// Retention (spec §14.10, ADR 0013): full texts of records reviewed, or
/// given up on, longer than `keep_ms` ago are cleared; the distance,
/// status and proposal links stay. Returns how many were cleared.
pub fn purge(tx: &Transaction<'_>, keep_ms: Millis, now: Millis) -> StoreResult<usize> {
    Ok(tx
        .prepare_cached(
            "UPDATE ai_compositions SET ai_text = NULL, ai_html = NULL, sent_text = NULL, instruction = ''
             WHERE (ai_text IS NOT NULL OR sent_text IS NOT NULL)
               AND ((status = 'reviewed' AND reviewed_at < ?1)
                 OR (status IN ('unmatched', 'discarded') AND updated_at < ?1))",
        )?
        .execute([now - keep_ms])?)
}

/// A message the user sent, as matching needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SentCandidate {
    pub message_id: String,
    pub thread_id: String,
    pub rfc822_message_id: Option<String>,
    pub at: Millis,
    /// Lowercased To and Cc addresses.
    pub to: Vec<String>,
    pub cc: Vec<String>,
}

/// Messages the user sent at or after `since`, oldest first. Optimistic
/// copies of sends not yet confirmed are left out: the real copy, with the
/// same Message-ID, replaces them.
pub fn sent_since(conn: &Connection, since: Millis) -> StoreResult<Vec<SentCandidate>> {
    let mut stmt = conn.prepare_cached(
        "SELECT m.id, m.gmail_id, t.gmail_id, m.rfc822_message_id, m.internal_date
         FROM messages m JOIN threads t ON t.id = m.thread_id
         WHERE m.is_sent_by_me AND NOT m.is_draft AND m.internal_date >= ?1
         ORDER BY m.internal_date, m.id",
    )?;
    let mut people = conn.prepare_cached(
        "SELECT role, email FROM participants WHERE message_id = ?1 AND role IN ('to', 'cc') ORDER BY role, position",
    )?;
    let mut out = Vec::new();
    let mut rows = stmt.query([since])?;
    while let Some(r) = rows.next()? {
        let message_id: String = r.get(1)?;
        if message_id.starts_with(crate::LOCAL_PREFIX) {
            continue;
        }
        let local: i64 = r.get(0)?;
        let mut c = SentCandidate {
            message_id,
            thread_id: r.get(2)?,
            rfc822_message_id: r.get(3)?,
            at: r.get(4)?,
            ..Default::default()
        };
        for p in people.query_map([local], |p| Ok((p.get::<_, String>(0)?, p.get::<_, String>(1)?)))? {
            let (role, email) = p?;
            if role == "to" { c.to.push(email.to_lowercase()) } else { c.cc.push(email.to_lowercase()) }
        }
        out.push(c);
    }
    Ok(out)
}

/// Sent messages already matched to a record: each matches at most one.
pub fn matched_messages(conn: &Connection) -> StoreResult<std::collections::BTreeSet<String>> {
    Ok(conn
        .prepare_cached("SELECT matched_message_id FROM ai_compositions WHERE matched_message_id IS NOT NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?)
}

/// The record became `message_id` (spec §14.10): the user's own text and
/// how far it is from the AI's.
pub fn set_matched(
    tx: &Transaction<'_>,
    id: i64,
    message_id: &str,
    method: &str,
    sent_text: &str,
    distance: f64,
    now: Millis,
) -> StoreResult<()> {
    tx.prepare_cached(
        "UPDATE ai_compositions SET status = 'matched', matched_message_id = ?2, match_method = ?3, sent_text = ?4,
           distance = ?5, updated_at = ?6
         WHERE id = ?1 AND status = 'waiting'",
    )?
    .execute(params![id, message_id, method, sent_text, distance, now])?;
    Ok(())
}

pub fn set_status(tx: &Transaction<'_>, id: i64, status: Status, now: Millis) -> StoreResult<()> {
    tx.prepare_cached("UPDATE ai_compositions SET status = ?2, updated_at = ?3 WHERE id = ?1")?.execute(params![
        id,
        status.as_str(),
        now
    ])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;
    use crate::drafts::{self, DraftRecord};

    fn db(name: &str) -> (Db, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("openagc-compositions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (Db::open(&dir.join("mail.sqlite")).unwrap(), dir)
    }

    fn draft(db: &Db) -> i64 {
        let d = DraftRecord { subject: "Lunch".into(), body_html: "<p>Hi</p>".into(), ..Default::default() };
        db.write_blocking(move |tx| drafts::save(tx, &d, 1)).unwrap()
    }

    fn composition(draft_id: i64, text: &str) -> NewComposition {
        NewComposition {
            source: Source::WritingHelp,
            agent: Some("claude-code".into()),
            kind: Kind::New,
            draft_id,
            thread_id: None,
            in_reply_to: None,
            recipients: Recipients::new(["Alex@Example.com"], []),
            subject: "Lunch".into(),
            instruction: "say yes".into(),
            ai_text: text.into(),
            ai_html: None,
            guide_version: Some(3),
            audiences: vec!["Friends".into()],
        }
    }

    #[test]
    fn a_later_ai_text_replaces_the_record_and_a_send_keeps_the_message_id() {
        let (db, dir) = db("send");
        let id = draft(&db);
        let first = db
            .write_blocking({
                let c = composition(id, "Yes, lunch works.");
                move |tx| record(tx, &c, 10)
            })
            .unwrap();
        let second = db
            .write_blocking({
                let c = composition(id, "Yes! Lunch on Friday works.");
                move |tx| record(tx, &c, 20)
            })
            .unwrap();
        assert_eq!(first, second, "one record per draft");
        let c = db.read_blocking(move |c| for_draft(c, id)).unwrap().unwrap();
        assert_eq!(c.ai_text.as_deref(), Some("Yes! Lunch on Friday works."));
        assert_eq!(c.recipients.to, vec!["alex@example.com".to_owned()]);
        assert_eq!((c.created_at, c.updated_at), (10, 20));

        db.write_blocking(move |tx| {
            drafts::set_rfc822_id(tx, id, "abc.openagc@example.com")?;
            drafts::set_state(tx, id, drafts::DraftState::Sending, None)?;
            drafts::discard(tx, id, 30)
        })
        .unwrap();
        let sent = db.read_blocking(move |c| get(c, first)).unwrap().unwrap();
        assert_eq!(sent.status, Status::Waiting, "a sent draft waits for its copy");
        assert_eq!(sent.rfc822_message_id.as_deref(), Some("abc.openagc@example.com"));
        assert_eq!(sent.draft_id, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn discarding_the_draft_marks_the_record_and_a_reused_id_starts_afresh() {
        let (db, dir) = db("discard");
        let id = draft(&db);
        let first = db
            .write_blocking({
                let c = composition(id, "Hello");
                move |tx| record(tx, &c, 10)
            })
            .unwrap();
        db.write_blocking(move |tx| drafts::discard(tx, id, 20)).unwrap();
        let gone = db.read_blocking(move |c| get(c, first)).unwrap().unwrap();
        assert_eq!((gone.status, gone.draft_id, gone.updated_at), (Status::Discarded, None, 20));
        // SQLite reuses the highest rowid once it is deleted.
        let again = draft(&db);
        assert_eq!(again, id);
        assert!(db.read_blocking(move |c| for_draft(c, again)).unwrap().is_none());
        let second = db
            .write_blocking({
                let c = composition(again, "Hi again");
                move |tx| record(tx, &c, 30)
            })
            .unwrap();
        assert_ne!(first, second);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_draft_whose_send_failed_keeps_waiting_when_discarded() {
        let (db, dir) = db("failed-send");
        let id = draft(&db);
        let record_id = db
            .write_blocking({
                let c = composition(id, "Hello");
                move |tx| record(tx, &c, 10)
            })
            .unwrap();
        db.write_blocking(move |tx| {
            drafts::set_rfc822_id(tx, id, "y.openagc@example.com")?;
            drafts::set_state(tx, id, drafts::DraftState::Failed, Some("timed out"))?;
            drafts::discard(tx, id, 20)
        })
        .unwrap();
        let r = db.read_blocking(move |c| get(c, record_id)).unwrap().unwrap();
        assert_eq!((r.status, r.rfc822_message_id.as_deref()), (Status::Waiting, Some("y.openagc@example.com")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn undo_send_forgets_the_message_id() {
        let (db, dir) = db("unsent");
        let id = draft(&db);
        let record_id = db
            .write_blocking({
                let c = composition(id, "Hello");
                move |tx| record(tx, &c, 10)
            })
            .unwrap();
        db.write_blocking(move |tx| {
            drafts::set_rfc822_id(tx, id, "x.openagc@example.com")?;
            draft_unsent(tx, id)
        })
        .unwrap();
        assert_eq!(db.read_blocking(move |c| get(c, record_id)).unwrap().unwrap().rfc822_message_id, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn recording_waits_for_a_finished_learning_run() {
        let (db, dir) = db("gate");
        assert!(!db.read_blocking(recording).unwrap());
        let run = db
            .write_blocking(|tx| crate::guide::create_run(tx, "learn", None, Some("claude-code"), &[], 20, 1))
            .unwrap();
        assert!(!db.read_blocking(recording).unwrap(), "a running run is not enough");
        db.write_blocking(move |tx| crate::guide::set_run_status(tx, run, "done", None, 2)).unwrap();
        assert!(db.read_blocking(recording).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn kinds() {
        assert_eq!(Kind::of("Fwd: Plans", true), Kind::Forward);
        assert_eq!(Kind::of("Re: Plans", true), Kind::Reply);
        assert_eq!(Kind::of("Plans", false), Kind::New);
    }
}
