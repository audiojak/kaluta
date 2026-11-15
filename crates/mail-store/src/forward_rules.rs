//! Forwarding rules (spec §14.15, ADR 0021): the rules, what Kaluta
//! proposes from the user's own forwards, and what each rule did with
//! each message. The store holds and finds; what matches a rule and what
//! makes a pattern is decided by its caller.

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::error::StoreResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: i64,
    /// An address, or a domain when `sender_is_domain`; lower case.
    pub sender: String,
    pub sender_is_domain: bool,
    pub subject_prefix: String,
    pub recipient: String,
    /// `auto` or `ask`.
    pub mode: String,
    pub enabled: bool,
    pub effective_from: Millis,
    pub proposal_id: Option<i64>,
    pub created_at: Millis,
}

const RULE: &str = "id, sender, sender_is_domain, subject_prefix, recipient, mode, enabled, effective_from, \
                    proposal_id, created_at";

fn rule(r: &rusqlite::Row) -> rusqlite::Result<Rule> {
    Ok(Rule {
        id: r.get(0)?,
        sender: r.get(1)?,
        sender_is_domain: r.get(2)?,
        subject_prefix: r.get(3)?,
        recipient: r.get(4)?,
        mode: r.get(5)?,
        enabled: r.get(6)?,
        effective_from: r.get(7)?,
        proposal_id: r.get(8)?,
        created_at: r.get(9)?,
    })
}

/// Insert a rule (`id` 0) or replace one. Changing what a rule matches or
/// where it sends makes it new as far as "arrived after" goes: the caller
/// passes the `effective_from` to keep or the new one.
pub fn save_rule(tx: &Transaction, r: &Rule, now: Millis) -> StoreResult<i64> {
    if r.id == 0 {
        tx.execute(
            "INSERT INTO forward_rules
               (sender, sender_is_domain, subject_prefix, recipient, mode, enabled, effective_from, proposal_id,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            params![
                r.sender,
                r.sender_is_domain,
                r.subject_prefix,
                r.recipient,
                r.mode,
                r.enabled,
                r.effective_from,
                r.proposal_id,
                now
            ],
        )?;
        return Ok(tx.last_insert_rowid());
    }
    tx.execute(
        "UPDATE forward_rules SET sender = ?2, sender_is_domain = ?3, subject_prefix = ?4, recipient = ?5, mode = ?6,
           enabled = ?7, effective_from = ?8, updated_at = ?9
         WHERE id = ?1",
        params![
            r.id,
            r.sender,
            r.sender_is_domain,
            r.subject_prefix,
            r.recipient,
            r.mode,
            r.enabled,
            r.effective_from,
            now
        ],
    )?;
    Ok(r.id)
}

/// Put back a rule exactly as it was (undo of a delete), under its old id.
pub fn restore_rule(tx: &Transaction, r: &Rule, now: Millis) -> StoreResult<()> {
    tx.execute(
        "INSERT OR REPLACE INTO forward_rules
           (id, sender, sender_is_domain, subject_prefix, recipient, mode, enabled, effective_from, proposal_id,
            created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            r.id,
            r.sender,
            r.sender_is_domain,
            r.subject_prefix,
            r.recipient,
            r.mode,
            r.enabled,
            r.effective_from,
            r.proposal_id,
            r.created_at,
            now
        ],
    )?;
    Ok(())
}

pub fn delete_rule(tx: &Transaction, id: i64) -> StoreResult<()> {
    tx.execute("DELETE FROM forward_rules WHERE id = ?1", [id])?;
    Ok(())
}

pub fn get_rule(conn: &Connection, id: i64) -> StoreResult<Option<Rule>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {RULE} FROM forward_rules WHERE id = ?1"))?
        .query_row([id], rule)
        .optional()?)
}

/// Every rule, oldest first.
pub fn rules(conn: &Connection) -> StoreResult<Vec<Rule>> {
    let mut stmt = conn.prepare_cached(&format!("SELECT {RULE} FROM forward_rules ORDER BY id"))?;
    let rows = stmt.query_map([], rule)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// How many a rule has forwarded, and when last.
pub fn rule_sent(conn: &Connection, rule_id: i64) -> StoreResult<(u32, Option<Millis>)> {
    Ok(conn
        .prepare_cached("SELECT COUNT(*), MAX(at) FROM forward_rule_log WHERE rule_id = ?1 AND outcome = 'sent'")?
        .query_row([rule_id], |r| Ok((r.get(0)?, r.get(1)?)))?)
}

/// How many a rule has forwarded since `since` (its daily cap).
pub fn sent_since(conn: &Connection, rule_id: i64, since: Millis) -> StoreResult<u32> {
    Ok(conn
        .prepare_cached("SELECT COUNT(*) FROM forward_rule_log WHERE rule_id = ?1 AND outcome = 'sent' AND at >= ?2")?
        .query_row(params![rule_id, since], |r| r.get(0))?)
}

// ---- The log ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    pub rule_id: i64,
    pub message_id: String,
    pub thread_id: String,
    /// Where the rule sent it, or would.
    pub recipient: String,
    /// `sent`, `asked`, `dismissed` or `skipped`.
    pub outcome: String,
    pub reason: Option<String>,
    pub at: Millis,
}

fn logged(r: &rusqlite::Row) -> rusqlite::Result<Logged> {
    Ok(Logged {
        rule_id: r.get(0)?,
        message_id: r.get(1)?,
        thread_id: r.get(2)?,
        recipient: r.get(3)?,
        outcome: r.get(4)?,
        reason: r.get(5)?,
        at: r.get(6)?,
    })
}

const LOG: &str = "rule_id, message_id, thread_id, recipient, outcome, reason, at";

/// Claim a message for a rule: `true` when this is the first the rule has
/// done with it. A message already logged is left as it is, so nothing is
/// forwarded or asked about twice, across restarts and re-syncs too.
pub fn log_first(
    tx: &Transaction,
    rule_id: i64,
    message_id: &str,
    thread_id: &str,
    recipient: &str,
    outcome: &str,
    now: Millis,
) -> StoreResult<bool> {
    let n = tx.execute(
        "INSERT OR IGNORE INTO forward_rule_log (rule_id, message_id, thread_id, recipient, outcome, at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![rule_id, message_id, thread_id, recipient, outcome, now],
    )?;
    Ok(n == 1)
}

/// What became of a message the rule asked about (`sent`, `dismissed`), or
/// of one whose forward could not be made (`skipped`, with why). `true`
/// when it was in the state expected, so two answers cannot both count.
pub fn log_resolve(
    tx: &Transaction,
    rule_id: i64,
    message_id: &str,
    from: &str,
    outcome: &str,
    reason: Option<&str>,
    now: Millis,
) -> StoreResult<bool> {
    let n = tx.execute(
        "UPDATE forward_rule_log SET outcome = ?4, reason = ?5, at = ?6
         WHERE rule_id = ?1 AND message_id = ?2 AND outcome = ?3",
        params![rule_id, message_id, from, outcome, reason, now],
    )?;
    Ok(n == 1)
}

/// Take a claim back (the forward could not be queued): it is tried again.
pub fn log_remove(tx: &Transaction, rule_id: i64, message_id: &str) -> StoreResult<()> {
    tx.execute("DELETE FROM forward_rule_log WHERE rule_id = ?1 AND message_id = ?2", params![rule_id, message_id])?;
    Ok(())
}

/// What rules did in a conversation, oldest first: the reader's marks and
/// the suggestion still waiting (`asked`).
pub fn log_for_thread(conn: &Connection, thread_id: &str) -> StoreResult<Vec<Logged>> {
    let mut stmt =
        conn.prepare_cached(&format!("SELECT {LOG} FROM forward_rule_log WHERE thread_id = ?1 ORDER BY at, rule_id"))?;
    let rows = stmt.query_map([thread_id], logged)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// What a rule did with one message, if anything yet.
pub fn logged_one(conn: &Connection, rule_id: i64, message_id: &str) -> StoreResult<Option<Logged>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {LOG} FROM forward_rule_log WHERE rule_id = ?1 AND message_id = ?2"))?
        .query_row(params![rule_id, message_id], logged)
        .optional()?)
}

/// The messages a rule has dealt with already.
pub fn logged_messages(conn: &Connection, rule_id: i64) -> StoreResult<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare_cached("SELECT message_id FROM forward_rule_log WHERE rule_id = ?1")?;
    let rows = stmt.query_map([rule_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Every message some rule forwarded: such forwards are the rule's doing,
/// not the user's, and never evidence for a new rule.
pub fn rule_sent_messages(conn: &Connection) -> StoreResult<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare_cached("SELECT DISTINCT message_id FROM forward_rule_log WHERE outcome = 'sent'")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The conversations with a suggestion waiting.
pub fn asked_threads(conn: &Connection) -> StoreResult<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT l.thread_id FROM forward_rule_log l JOIN forward_rules r ON r.id = l.rule_id
         WHERE l.outcome = 'asked' AND r.enabled = 1",
    )?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// ---- Mail the rules and the watching read ----

/// A received message, as far as a rule cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    pub message_id: String,
    pub thread_id: String,
    /// Lower case.
    pub from_email: String,
    pub subject: String,
    pub at: Millis,
}

/// Mail received from an address (or, with `domain`, from any address at
/// it) at or after `since`, oldest first: not the user's own, not drafts,
/// not in Spam or Trash.
pub fn received_from(conn: &Connection, sender: &str, domain: bool, since: Millis) -> StoreResult<Vec<Received>> {
    // An address is looked up by the index; a domain reads the senders.
    let (pattern, test) = if domain {
        (format!("%@{}", like_escape(sender)), "m.from_email LIKE ?1 ESCAPE '\\'")
    } else {
        (sender.to_owned(), "m.from_email = ?1 COLLATE NOCASE")
    };
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT m.gmail_id, t.gmail_id, LOWER(m.from_email), m.subject, m.internal_date
         FROM messages m JOIN threads t ON t.id = m.thread_id
         WHERE {test} AND m.internal_date >= ?2
           AND m.is_sent_by_me = 0 AND m.is_draft = 0
           AND NOT EXISTS (SELECT 1 FROM message_labels ml JOIN labels l ON l.id = ml.label_id
                           WHERE ml.message_id = m.id AND l.gmail_id IN ('SPAM', 'TRASH'))
         ORDER BY m.internal_date, m.id"
    ))?;
    let rows = stmt.query_map(params![pattern, since], |r| {
        Ok(Received {
            message_id: r.get(0)?,
            thread_id: r.get(1)?,
            from_email: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            subject: r.get(3)?,
            at: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// A forward the user sent, with the message it forwarded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentForward {
    /// The forward itself.
    pub forward_id: String,
    pub forwarded_at: Millis,
    /// Lower case.
    pub recipient: String,
    pub original: Received,
}

/// Forwards the user sent at or after `since`, oldest first: sent mail
/// whose subject starts `Fwd:` or `Fw:`, to exactly one recipient, with
/// the newest received message before it in its conversation (what it
/// forwarded). A forward with no such message is left out.
pub fn sent_forwards(conn: &Connection, since: Millis) -> StoreResult<Vec<SentForward>> {
    let mut stmt = conn.prepare_cached(
        "SELECT f.gmail_id, f.internal_date,
                (SELECT LOWER(p.email) FROM participants p
                 WHERE p.message_id = f.id AND p.role IN ('to', 'cc', 'bcc')),
                o.gmail_id, t.gmail_id, LOWER(o.from_email), o.subject, o.internal_date
         FROM messages f
         JOIN threads t ON t.id = f.thread_id
         JOIN messages o ON o.id = (
           SELECT x.id FROM messages x
           WHERE x.thread_id = f.thread_id AND x.is_sent_by_me = 0 AND x.is_draft = 0
             AND x.internal_date <= f.internal_date AND x.id <> f.id
           ORDER BY x.internal_date DESC, x.id DESC LIMIT 1)
         WHERE f.is_sent_by_me = 1 AND f.is_draft = 0 AND f.internal_date >= ?1
           AND (f.subject LIKE 'fwd:%' OR f.subject LIKE 'fw:%')
           AND (SELECT COUNT(*) FROM participants p
                WHERE p.message_id = f.id AND p.role IN ('to', 'cc', 'bcc')) = 1
           AND o.from_email IS NOT NULL AND o.from_email <> ''
         ORDER BY f.internal_date, f.id",
    )?;
    let rows = stmt.query_map([since], |r| {
        Ok(SentForward {
            forward_id: r.get(0)?,
            forwarded_at: r.get(1)?,
            recipient: r.get(2)?,
            original: Received {
                message_id: r.get(3)?,
                thread_id: r.get(4)?,
                from_email: r.get(5)?,
                subject: r.get(6)?,
                at: r.get(7)?,
            },
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

// ---- Proposals ----

/// A forwarded message behind a proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub thread_id: String,
    pub message_id: String,
    pub subject: String,
    pub at: Millis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub id: i64,
    pub statement: String,
    pub sender: String,
    pub subject_prefix: String,
    pub recipient: String,
    pub status: String,
    pub evidence: Vec<Evidence>,
    pub rule_id: Option<i64>,
    pub created_at: Millis,
}

const PROPOSAL: &str = "id, statement, sender, subject_prefix, recipient, status, evidence_json, rule_id, created_at";

fn proposal(r: &rusqlite::Row) -> rusqlite::Result<Proposal> {
    let evidence: String = r.get(6)?;
    Ok(Proposal {
        id: r.get(0)?,
        statement: r.get(1)?,
        sender: r.get(2)?,
        subject_prefix: r.get(3)?,
        recipient: r.get(4)?,
        status: r.get(5)?,
        evidence: serde_json::from_str(&evidence).unwrap_or_default(),
        rule_id: r.get(7)?,
        created_at: r.get(8)?,
    })
}

/// What is asked for when proposing.
pub struct NewProposal<'a> {
    pub statement: &'a str,
    pub sender: &'a str,
    pub subject_prefix: &'a str,
    pub recipient: &'a str,
    pub match_key: &'a str,
    pub evidence: Vec<Evidence>,
}

/// Propose a rule, or bring the same one's evidence up to date (same
/// key). One already accepted or rejected is not raised again: `None`.
pub fn propose(tx: &Transaction, new: &NewProposal<'_>, now: Millis) -> StoreResult<Option<i64>> {
    let existing: Option<(i64, String)> = tx
        .prepare_cached("SELECT id, status FROM forward_rule_proposals WHERE match_key = ?1")?
        .query_row([new.match_key], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let evidence = serde_json::to_string(&new.evidence).unwrap_or_else(|_| "[]".into());
    match existing {
        Some((_, status)) if status != "proposed" => Ok(None),
        Some((id, _)) => {
            tx.execute(
                "UPDATE forward_rule_proposals SET evidence_json = ?2, statement = ?3, updated_at = ?4 WHERE id = ?1",
                params![id, evidence, new.statement, now],
            )?;
            Ok(Some(id))
        }
        None => {
            tx.execute(
                "INSERT INTO forward_rule_proposals
                   (statement, sender, subject_prefix, recipient, match_key, status, evidence_json, created_at,
                    updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'proposed', ?6, ?7, ?7)",
                params![new.statement, new.sender, new.subject_prefix, new.recipient, new.match_key, evidence, now],
            )?;
            Ok(Some(tx.last_insert_rowid()))
        }
    }
}

/// A proposal still waiting whose pattern no longer holds goes.
pub fn withdraw_proposed(tx: &Transaction, match_key: &str) -> StoreResult<()> {
    tx.execute("DELETE FROM forward_rule_proposals WHERE match_key = ?1 AND status = 'proposed'", [match_key])?;
    Ok(())
}

pub fn get_proposal(conn: &Connection, id: i64) -> StoreResult<Option<Proposal>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {PROPOSAL} FROM forward_rule_proposals WHERE id = ?1"))?
        .query_row([id], proposal)
        .optional()?)
}

/// Those waiting for the user, oldest first.
pub fn proposed(conn: &Connection) -> StoreResult<Vec<Proposal>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {PROPOSAL} FROM forward_rule_proposals WHERE status = 'proposed' ORDER BY id"
    ))?;
    let rows = stmt.query_map([], proposal)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The keys waiting for the user (to withdraw those that stopped holding).
pub fn proposed_keys(conn: &Connection) -> StoreResult<Vec<String>> {
    let mut stmt = conn.prepare_cached("SELECT match_key FROM forward_rule_proposals WHERE status = 'proposed'")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// `proposed`, `accepted` or `rejected`, with the rule made from it.
pub fn set_proposal_status(
    tx: &Transaction,
    id: i64,
    status: &str,
    rule_id: Option<i64>,
    now: Millis,
) -> StoreResult<()> {
    tx.execute(
        "UPDATE forward_rule_proposals SET status = ?2, rule_id = ?3, updated_at = ?4,
           decided_at = CASE WHEN ?2 = 'proposed' THEN NULL ELSE ?4 END
         WHERE id = ?1",
        params![id, status, rule_id, now],
    )?;
    Ok(())
}
