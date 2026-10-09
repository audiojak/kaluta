//! Reports of what cloud agents sent (spec §10.6, ADR 0016), pulled from an
//! agent mailbox's rules server. A report is the agent's own account of its
//! own send: untrusted text, kept as data and never followed. Each is pulled
//! once (its server and id are unique), matched to the sent message in the
//! mailbox, by the Message-ID it names or else by recipient, subject and
//! time, and linked to the AI composition it was recorded as (ADR 0013).

use std::collections::BTreeSet;

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::error::StoreResult;

/// A report without a Message-ID (or whose Message-ID is not in the
/// mailbox) matches a sent message to one of its recipients with its
/// subject sent this close to when it says it was sent.
pub const MATCH_WINDOW_MS: Millis = 10 * 60 * 1000;
/// A report not matched after this long is "reported, not seen in the
/// mailbox".
pub const NOT_SEEN_AFTER_MS: Millis = 24 * 60 * 60 * 1000;
/// Reports are looked for in the mailbox this long, then left unmatched.
pub const MATCH_FOR_MS: Millis = 14 * 24 * 60 * 60 * 1000;

/// How a report was matched to its sent message.
pub const BY_MESSAGE_ID: &str = "message_id";
pub const BY_RECIPIENT_SUBJECT: &str = "recipient_subject";

/// A report as pulled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewReport {
    /// The rules server's URL, with `#<epoch>` when the server names its
    /// database's epoch: report ids are unique within one database only.
    pub server: String,
    pub server_id: i64,
    pub agent_id: String,
    pub agent_name: String,
    /// `token` or `oauth`.
    pub agent_kind: String,
    pub received_at: Millis,
    /// Without its brackets.
    pub message_id: Option<String>,
    pub to: Vec<String>,
    pub subject: String,
    pub sent_at: Option<Millis>,
    pub body_markdown: String,
    pub checked_version: Option<i64>,
    pub check_version: Option<i64>,
    pub guide_check: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub id: i64,
    pub server: String,
    pub server_id: i64,
    pub agent_id: String,
    pub agent_name: String,
    pub agent_kind: String,
    pub received_at: Millis,
    pub pulled_at: Millis,
    pub message_id: Option<String>,
    pub to: Vec<String>,
    pub subject: String,
    pub sent_at: Option<Millis>,
    /// `None` once cleared (retention).
    pub body_markdown: Option<String>,
    pub checked_version: Option<i64>,
    pub check_version: Option<i64>,
    pub guide_check: Vec<String>,
    pub composition_id: Option<i64>,
    /// The sent message's id (`gmail_id`), once matched.
    pub matched_message_id: Option<String>,
    pub match_method: Option<String>,
    pub matched_at: Option<Millis>,
}

impl Report {
    /// When the agent says it sent the message, or when the server heard.
    pub fn at(&self) -> Millis {
        self.sent_at.unwrap_or(self.received_at)
    }
}

const COLUMNS: &str = "id, server, server_id, agent_id, agent_name, agent_kind, received_at, pulled_at, message_id, \
                       to_json, subject, sent_at, body_markdown, checked_version, check_version, guide_check_json, \
                       composition_id, matched_message_id, match_method, matched_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<Report> {
    let to: String = r.get(9)?;
    let check: String = r.get(15)?;
    Ok(Report {
        id: r.get(0)?,
        server: r.get(1)?,
        server_id: r.get(2)?,
        agent_id: r.get(3)?,
        agent_name: r.get(4)?,
        agent_kind: r.get(5)?,
        received_at: r.get(6)?,
        pulled_at: r.get(7)?,
        message_id: r.get(8)?,
        to: serde_json::from_str(&to).unwrap_or_default(),
        subject: r.get(10)?,
        sent_at: r.get(11)?,
        body_markdown: r.get(12)?,
        checked_version: r.get(13)?,
        check_version: r.get(14)?,
        guide_check: serde_json::from_str(&check).unwrap_or_default(),
        composition_id: r.get(16)?,
        matched_message_id: r.get(17)?,
        match_method: r.get(18)?,
        matched_at: r.get(19)?,
    })
}

/// A Message-ID as stored: no brackets, no surrounding space.
pub fn normalize_message_id(id: &str) -> String {
    id.trim().trim_start_matches('<').trim_end_matches('>').trim().to_owned()
}

/// The address in `Name <a@x.com>` (or the whole, trimmed), lowercased.
fn address(s: &str) -> String {
    let s = s.trim();
    let inner = match (s.rfind('<'), s.rfind('>')) {
        (Some(open), Some(close)) if open < close => &s[open + 1..close],
        _ => s,
    };
    inner.trim().to_lowercase()
}

/// A subject compared: trimmed, spaces collapsed, any case.
fn subject_key(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Keep a report pulled for the first time; `None` when it was pulled
/// before (pulling again records nothing twice).
pub fn insert(tx: &Transaction<'_>, r: &NewReport, now: Millis) -> StoreResult<Option<i64>> {
    let n = tx
        .prepare_cached(
            "INSERT INTO cloud_reports (server, server_id, agent_id, agent_name, agent_kind, received_at, pulled_at,
               message_id, to_json, subject, sent_at, body_markdown, checked_version, check_version, guide_check_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT (server, server_id) DO NOTHING",
        )?
        .execute(params![
            r.server,
            r.server_id,
            r.agent_id,
            r.agent_name,
            r.agent_kind,
            r.received_at,
            now,
            r.message_id,
            serde_json::to_string(&r.to)?,
            r.subject,
            r.sent_at,
            r.body_markdown,
            r.checked_version,
            r.check_version,
            serde_json::to_string(&r.guide_check)?,
        ])?;
    Ok((n == 1).then(|| tx.last_insert_rowid()))
}

/// The report was recorded as AI composition `composition_id`.
pub fn set_composition(tx: &Transaction<'_>, id: i64, composition_id: i64) -> StoreResult<()> {
    tx.prepare_cached("UPDATE cloud_reports SET composition_id = ?2 WHERE id = ?1")?
        .execute(params![id, composition_id])?;
    Ok(())
}

pub fn get(conn: &Connection, id: i64) -> StoreResult<Option<Report>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {COLUMNS} FROM cloud_reports WHERE id = ?1"))?
        .query_row([id], from_row)
        .optional()?)
}

/// The latest reports, newest first.
pub fn recent(conn: &Connection, limit: u32) -> StoreResult<Vec<Report>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM cloud_reports ORDER BY COALESCE(sent_at, received_at) DESC, id DESC LIMIT ?1"
        ))?
        .query_map([limit], from_row)?
        .collect::<Result<_, _>>()?)
}

/// How many reports came since `since`.
pub fn count_since(conn: &Connection, since: Millis) -> StoreResult<u32> {
    let n: i64 = conn
        .prepare_cached("SELECT COUNT(*) FROM cloud_reports WHERE received_at >= ?1")?
        .query_row([since], |r| r.get(0))?;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

/// A sent message, as matching a report needs it.
#[derive(Debug, Clone)]
struct Sent {
    gmail_id: String,
    rfc822: Option<String>,
    at: Millis,
    subject: String,
    people: Vec<String>,
}

fn sent_people(conn: &Connection, local: i64) -> StoreResult<Vec<String>> {
    Ok(conn
        .prepare_cached("SELECT email FROM participants WHERE message_id = ?1 AND role IN ('to', 'cc', 'bcc')")?
        .query_map([local], |r| r.get::<_, String>(0))?
        .map(|e| e.map(|e| e.trim().to_lowercase()))
        .collect::<Result<_, _>>()?)
}

fn sent_rows(conn: &Connection, sql: &str, p: impl rusqlite::Params) -> StoreResult<Vec<Sent>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows: Vec<(i64, Sent)> = stmt
        .query_map(p, |r| {
            Ok((
                r.get(0)?,
                Sent { gmail_id: r.get(1)?, rfc822: r.get(2)?, at: r.get(3)?, subject: r.get(4)?, people: vec![] },
            ))
        })?
        .collect::<Result<_, _>>()?;
    rows.into_iter()
        .filter(|(_, s)| !s.gmail_id.starts_with(crate::LOCAL_PREFIX))
        .map(|(local, mut s)| {
            s.people = sent_people(conn, local)?;
            Ok(s)
        })
        .collect()
}

const SENT: &str = "SELECT id, gmail_id, rfc822_message_id, internal_date, subject FROM messages
                    WHERE is_sent_by_me AND NOT is_draft";

/// A report matched now, to which sent message (its `gmail_id`) and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matched {
    pub report_id: i64,
    pub message_id: String,
    pub method: &'static str,
}

/// Match the reports not matched yet (within [`MATCH_FOR_MS`]) to sent
/// mail, strongest first: the sent message with the Message-ID the report
/// names; else one to a recipient of the report with its subject, sent
/// within [`MATCH_WINDOW_MS`] of when the report says (the closest). A sent
/// message matches one report at most. A matched report's composition
/// takes the sent message's Message-ID, so the daily review pairs them as
/// it pairs a draft with its sent copy.
pub fn match_reports(tx: &Transaction<'_>, now: Millis) -> StoreResult<Vec<Matched>> {
    let waiting: Vec<Report> = tx
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM cloud_reports WHERE matched_message_id IS NULL AND received_at >= ?1
             ORDER BY COALESCE(sent_at, received_at), id"
        ))?
        .query_map([now - MATCH_FOR_MS], from_row)?
        .collect::<Result<_, _>>()?;
    if waiting.is_empty() {
        return Ok(vec![]);
    }
    let mut used: BTreeSet<String> = tx
        .prepare_cached("SELECT matched_message_id FROM cloud_reports WHERE matched_message_id IS NOT NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for report in waiting {
        let by_id = match report.message_id.as_deref().filter(|m| !m.is_empty()) {
            Some(mid) => sent_rows(tx, &format!("{SENT} AND rfc822_message_id = ?1 ORDER BY internal_date"), [mid])?
                .into_iter()
                .find(|s| !used.contains(&s.gmail_id)),
            None => None,
        };
        let found = match by_id {
            Some(s) => Some((s, BY_MESSAGE_ID)),
            None => {
                let at = report.at();
                let to: Vec<String> = report.to.iter().map(|t| address(t)).collect();
                let subject = subject_key(&report.subject);
                sent_rows(
                    tx,
                    &format!("{SENT} AND internal_date BETWEEN ?1 AND ?2 ORDER BY internal_date"),
                    params![at - MATCH_WINDOW_MS, at + MATCH_WINDOW_MS],
                )?
                .into_iter()
                .filter(|s| !used.contains(&s.gmail_id))
                .filter(|s| subject_key(&s.subject) == subject)
                .filter(|s| s.people.iter().any(|p| to.contains(p)))
                .min_by_key(|s| (s.at - at).abs())
                .map(|s| (s, BY_RECIPIENT_SUBJECT))
            }
        };
        let Some((sent, method)) = found else { continue };
        tx.prepare_cached(
            "UPDATE cloud_reports SET matched_message_id = ?2, match_method = ?3, matched_at = ?4 WHERE id = ?1",
        )?
        .execute(params![report.id, sent.gmail_id, method, now])?;
        if let (Some(composition), Some(rfc822)) = (report.composition_id, sent.rfc822.as_deref()) {
            tx.prepare_cached(
                "UPDATE ai_compositions SET rfc822_message_id = ?2, updated_at = ?3 WHERE id = ?1 AND status = 'waiting'",
            )?
            .execute(params![composition, rfc822, now])?;
        }
        used.insert(sent.gmail_id.clone());
        out.push(Matched { report_id: report.id, message_id: sent.gmail_id, method });
    }
    Ok(out)
}

/// Retention (ADR 0013): report bodies older than `keep_ms` are cleared;
/// who sent what to whom, when and the check stay. How many were cleared.
pub fn purge(tx: &Transaction<'_>, keep_ms: Millis, now: Millis) -> StoreResult<usize> {
    Ok(tx
        .prepare_cached(
            "UPDATE cloud_reports SET body_markdown = NULL WHERE body_markdown IS NOT NULL AND received_at < ?1",
        )?
        .execute([now - keep_ms])?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    #[test]
    fn message_ids_and_addresses_are_read_as_stored() {
        assert_eq!(normalize_message_id(" <abc@x.com> "), "abc@x.com");
        assert_eq!(address("Ann Lee <Ann@Acme.com>"), "ann@acme.com");
        assert_eq!(address(" BEA@globex.com "), "bea@globex.com");
        assert_eq!(subject_key("  Re:   Plan "), "re: plan");
    }

    #[test]
    fn a_report_is_kept_once_and_its_body_cleared_with_the_ai_texts() {
        let dir = std::env::temp_dir().join(format!("kaluta-cloud-reports-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("mail.sqlite")).unwrap();
        let r = NewReport {
            server: "https://rules.example.com".into(),
            server_id: 7,
            agent_id: "0123456789abcdef".into(),
            agent_name: "Weekly outreach routine".into(),
            agent_kind: "oauth".into(),
            received_at: 1_000,
            to: vec!["ann@acme.com".into()],
            subject: "Plan".into(),
            body_markdown: "Hi".into(),
            guide_check: vec!["Uses “circle back”, which your rules ban".into()],
            ..Default::default()
        };
        let first = db.write_blocking({
            let r = r.clone();
            move |tx| insert(tx, &r, 2_000)
        });
        assert!(first.unwrap().is_some());
        let again = db.write_blocking({
            let r = r.clone();
            move |tx| insert(tx, &r, 3_000)
        });
        assert_eq!(again.unwrap(), None, "pulled before");
        let other_server = NewReport { server: "https://other.example".into(), ..r };
        assert!(db.write_blocking(move |tx| insert(tx, &other_server, 3_000)).unwrap().is_some());
        let listed = db.read_blocking(|c| recent(c, 10)).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].guide_check.len(), 1);
        assert_eq!((listed[1].agent_kind.as_str(), listed[1].pulled_at), ("oauth", 2_000));
        assert_eq!(db.read_blocking(|c| count_since(c, 1_000)).unwrap(), 2);
        assert_eq!(db.read_blocking(|c| count_since(c, 1_001)).unwrap(), 0);
        assert_eq!(db.write_blocking(|tx| purge(tx, 10, 1_010)).unwrap(), 0);
        assert_eq!(db.write_blocking(|tx| purge(tx, 10, 1_011)).unwrap(), 2);
        assert!(db.read_blocking(|c| recent(c, 10)).unwrap().iter().all(|r| r.body_markdown.is_none()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
