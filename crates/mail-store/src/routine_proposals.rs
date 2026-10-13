//! Changes Kaluta proposes to a sorting routine (spec §14.13), with the
//! conversations that show them.

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::error::StoreResult;

/// A conversation behind a proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub thread_id: String,
    pub subject: String,
    /// The bucket it had been filed in (its label's name).
    pub bucket: String,
    pub at: Millis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    pub id: i64,
    pub routine_id: Option<String>,
    pub op: String,
    pub statement: String,
    pub rule: String,
    pub status: String,
    pub evidence: Vec<Evidence>,
    pub created_at: Millis,
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Proposal> {
    let evidence: String = r.get(6)?;
    Ok(Proposal {
        id: r.get(0)?,
        routine_id: r.get(1)?,
        op: r.get(2)?,
        statement: r.get(3)?,
        rule: r.get(4)?,
        status: r.get(5)?,
        evidence: serde_json::from_str(&evidence).unwrap_or_default(),
        created_at: r.get(7)?,
    })
}

const COLUMNS: &str = "id, routine_id, op, statement, rule, status, evidence_json, created_at";

/// Propose a change, or add a conversation to the same one (same key). A
/// change already accepted or rejected is not raised again: `None`.
pub fn propose(
    tx: &Transaction,
    routine_id: Option<&str>,
    statement: &str,
    rule: &str,
    match_key: &str,
    evidence: Evidence,
    now: Millis,
) -> StoreResult<Option<i64>> {
    let existing = tx
        .prepare_cached(&format!("SELECT {COLUMNS} FROM routine_proposals WHERE match_key = ?1"))?
        .query_row([match_key], row)
        .optional()?;
    match existing {
        Some(p) if p.status != "proposed" => Ok(None),
        Some(mut p) => {
            if !p.evidence.iter().any(|e| e.thread_id == evidence.thread_id) {
                p.evidence.push(evidence);
            }
            tx.execute(
                "UPDATE routine_proposals SET evidence_json = ?2, updated_at = ?3 WHERE id = ?1",
                params![p.id, serde_json::to_string(&p.evidence).unwrap_or_else(|_| "[]".into()), now],
            )?;
            Ok(Some(p.id))
        }
        None => {
            tx.execute(
                "INSERT INTO routine_proposals
                   (routine_id, op, statement, rule, match_key, status, evidence_json, created_at, updated_at)
                 VALUES (?1, 'leave_alone', ?2, ?3, ?4, 'proposed', ?5, ?6, ?6)",
                params![
                    routine_id,
                    statement,
                    rule,
                    match_key,
                    serde_json::to_string(&[evidence]).unwrap_or_else(|_| "[]".into()),
                    now
                ],
            )?;
            Ok(Some(tx.last_insert_rowid()))
        }
    }
}

/// Undo of a move back to the Inbox: the conversation no longer counts; a
/// proposal left with none goes.
pub fn withdraw(tx: &Transaction, id: i64, thread_id: &str, now: Millis) -> StoreResult<()> {
    let Some(mut p) = get(tx, id)? else { return Ok(()) };
    p.evidence.retain(|e| e.thread_id != thread_id);
    if p.evidence.is_empty() && p.status == "proposed" {
        tx.execute("DELETE FROM routine_proposals WHERE id = ?1", [id])?;
    } else {
        tx.execute(
            "UPDATE routine_proposals SET evidence_json = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, serde_json::to_string(&p.evidence).unwrap_or_else(|_| "[]".into()), now],
        )?;
    }
    Ok(())
}

pub fn get(conn: &Connection, id: i64) -> StoreResult<Option<Proposal>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {COLUMNS} FROM routine_proposals WHERE id = ?1"))?
        .query_row([id], row)
        .optional()?)
}

/// Those waiting for the user, oldest first.
pub fn proposed(conn: &Connection) -> StoreResult<Vec<Proposal>> {
    let mut stmt =
        conn.prepare_cached(&format!("SELECT {COLUMNS} FROM routine_proposals WHERE status = 'proposed' ORDER BY id"))?;
    let rows = stmt.query_map([], row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// `proposed`, `accepted` or `rejected`; the routine it went to, once known.
pub fn set_status(tx: &Transaction, id: i64, status: &str, routine_id: Option<&str>, now: Millis) -> StoreResult<()> {
    tx.execute(
        "UPDATE routine_proposals SET status = ?2, routine_id = COALESCE(?3, routine_id), updated_at = ?4,
           decided_at = CASE WHEN ?2 = 'proposed' THEN NULL ELSE ?4 END
         WHERE id = ?1",
        params![id, status, routine_id, now],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    #[test]
    fn a_proposal_gathers_conversations_and_is_not_raised_again_once_decided() {
        let dir = std::env::temp_dir().join(format!("kaluta-routine-proposals-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("mail.sqlite")).unwrap();
        let ev =
            |t: &str| Evidence { thread_id: t.into(), subject: "Issue 42".into(), bucket: "2-Weekly".into(), at: 1 };
        let add = |t: &'static str| {
            let e = ev(t);
            db.write_blocking(move |tx| {
                propose(
                    tx,
                    None,
                    "Leave mail from news@x.com in the Inbox",
                    "Mail from news@x.com",
                    "leave:news@x.com",
                    e,
                    1,
                )
            })
            .unwrap()
        };
        let id = add("t1").unwrap();
        assert_eq!(add("t2"), Some(id));
        assert_eq!(add("t2"), Some(id), "the same conversation once");
        assert_eq!(db.read_blocking(move |c| get(c, id)).unwrap().unwrap().evidence.len(), 2);
        db.write_blocking(move |tx| withdraw(tx, id, "t2", 2)).unwrap();
        assert_eq!(db.read_blocking(proposed).unwrap()[0].evidence.len(), 1);
        db.write_blocking(move |tx| set_status(tx, id, "rejected", None, 3)).unwrap();
        assert_eq!(add("t3"), None, "rejected stays rejected");
        assert!(db.read_blocking(proposed).unwrap().is_empty());
        db.write_blocking(move |tx| set_status(tx, id, "proposed", None, 4)).unwrap();
        db.write_blocking(move |tx| withdraw(tx, id, "t1", 5)).unwrap();
        assert!(db.read_blocking(move |c| get(c, id)).unwrap().is_none(), "no evidence, no proposal");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
