//! The writing guide (spec §14.9, ADR 0011): entries with their evidence,
//! learning runs batch by batch, audience groups, model examples, the
//! change log for exact undo and the versions of the accepted guide. Per
//! account, like everything in this store.

use std::collections::BTreeSet;

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::error::StoreResult;

/// An entry as stored.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EntryRow {
    pub id: i64,
    pub category: String,
    /// `rule`, `guideline` or `fact`.
    pub kind: String,
    pub statement: String,
    /// JSON: `{"groups":[],"people":[],"message_types":[],"languages":[]}`.
    pub scope_json: String,
    /// `proposed`, `accepted` or `rejected`.
    pub status: String,
    /// `learned`, `you` or `merged`.
    pub source: String,
    pub origin: Option<String>,
    pub check_json: Option<String>,
    pub support: i64,
    pub contradict: i64,
    pub contradiction_of: Option<i64>,
    pub run_id: Option<i64>,
    pub created_at: Millis,
    pub updated_at: Millis,
    pub decided_at: Option<Millis>,
}

/// A quote from the user's sent mail behind an entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EvidenceRow {
    pub message_id: String,
    pub quote: String,
    /// The quote goes against the entry rather than for it.
    pub contradicts: bool,
}

/// An entry with its evidence: what undo puts back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub entry: EntryRow,
    pub evidence: Vec<EvidenceRow>,
}

/// The statement as compared for merging: lower case, spaces collapsed,
/// the final full stop dropped.
pub fn norm(statement: &str) -> String {
    let words: Vec<String> = statement.split_whitespace().map(str::to_lowercase).collect();
    words.join(" ").trim_end_matches(['.', '!']).to_owned()
}

const ENTRY_COLUMNS: &str = "id, category, kind, statement, scope_json, status, source, origin, check_json, support,
       contradict, contradiction_of, run_id, created_at, updated_at, decided_at";

fn entry(r: &Row<'_>) -> rusqlite::Result<EntryRow> {
    Ok(EntryRow {
        id: r.get(0)?,
        category: r.get(1)?,
        kind: r.get(2)?,
        statement: r.get(3)?,
        scope_json: r.get(4)?,
        status: r.get(5)?,
        source: r.get(6)?,
        origin: r.get(7)?,
        check_json: r.get(8)?,
        support: r.get(9)?,
        contradict: r.get(10)?,
        contradiction_of: r.get(11)?,
        run_id: r.get(12)?,
        created_at: r.get(13)?,
        updated_at: r.get(14)?,
        decided_at: r.get(15)?,
    })
}

pub fn get_entry(conn: &Connection, id: i64) -> StoreResult<Option<EntryRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {ENTRY_COLUMNS} FROM guide_entries WHERE id = ?1"))?
        .query_row([id], entry)
        .optional()?)
}

/// Entries with any of `statuses` (all when empty), by category then age.
pub fn list_entries(conn: &Connection, statuses: &[&str]) -> StoreResult<Vec<EntryRow>> {
    let mut stmt = conn.prepare_cached(&format!("SELECT {ENTRY_COLUMNS} FROM guide_entries ORDER BY category, id"))?;
    let rows = stmt.query_map([], entry)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows.into_iter().filter(|r| statuses.is_empty() || statuses.contains(&r.status.as_str())).collect())
}

/// The entry in `category` with this normalised statement, if any.
pub fn find_by_norm(conn: &Connection, category: &str, statement: &str) -> StoreResult<Option<EntryRow>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {ENTRY_COLUMNS} FROM guide_entries WHERE category = ?1 AND norm = ?2 ORDER BY id LIMIT 1"
        ))?
        .query_row(params![category, norm(statement)], entry)
        .optional()?)
}

pub fn evidence(conn: &Connection, entry_id: i64) -> StoreResult<Vec<EvidenceRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT message_id, quote, contradicts FROM guide_evidence WHERE entry_id = ?1 ORDER BY contradicts, message_id",
    )?;
    let rows = stmt
        .query_map([entry_id], |r| Ok(EvidenceRow { message_id: r.get(0)?, quote: r.get(1)?, contradicts: r.get(2)? }))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Insert an entry (a new id when `id` is 0, else that id); returns its id.
pub fn insert_entry(tx: &Transaction<'_>, e: &EntryRow) -> StoreResult<i64> {
    tx.prepare_cached(
        "INSERT INTO guide_entries (id, category, kind, statement, norm, scope_json, status, source, origin,
           check_json, support, contradict, contradiction_of, run_id, created_at, updated_at, decided_at)
         VALUES (NULLIF(?1, 0), ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
    )?
    .execute(params![
        e.id,
        e.category,
        e.kind,
        e.statement,
        norm(&e.statement),
        e.scope_json,
        e.status,
        e.source,
        e.origin,
        e.check_json,
        e.support,
        e.contradict,
        e.contradiction_of,
        e.run_id,
        e.created_at,
        e.updated_at,
        e.decided_at
    ])?;
    Ok(tx.last_insert_rowid())
}

/// Write every column of an existing entry.
pub fn write_entry(tx: &Transaction<'_>, e: &EntryRow) -> StoreResult<bool> {
    let n = tx
        .prepare_cached(
            "UPDATE guide_entries SET category = ?2, kind = ?3, statement = ?4, norm = ?5, scope_json = ?6,
               status = ?7, source = ?8, origin = ?9, check_json = ?10, support = ?11, contradict = ?12,
               contradiction_of = ?13, run_id = ?14, created_at = ?15, updated_at = ?16, decided_at = ?17
             WHERE id = ?1",
        )?
        .execute(params![
            e.id,
            e.category,
            e.kind,
            e.statement,
            norm(&e.statement),
            e.scope_json,
            e.status,
            e.source,
            e.origin,
            e.check_json,
            e.support,
            e.contradict,
            e.contradiction_of,
            e.run_id,
            e.created_at,
            e.updated_at,
            e.decided_at
        ])?;
    Ok(n > 0)
}

pub fn delete_entry(tx: &Transaction<'_>, id: i64) -> StoreResult<bool> {
    Ok(tx.prepare_cached("DELETE FROM guide_entries WHERE id = ?1")?.execute([id])? > 0)
}

/// Add quotes (repeats ignored) and recount the entry's support and
/// contradiction from distinct messages.
pub fn add_evidence(tx: &Transaction<'_>, entry_id: i64, quotes: &[EvidenceRow]) -> StoreResult<()> {
    let mut stmt = tx.prepare_cached(
        "INSERT OR IGNORE INTO guide_evidence (entry_id, message_id, quote, contradicts) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for q in quotes {
        stmt.execute(params![entry_id, q.message_id, q.quote, q.contradicts])?;
    }
    recount(tx, entry_id)
}

fn recount(tx: &Transaction<'_>, entry_id: i64) -> StoreResult<()> {
    tx.prepare_cached(
        "UPDATE guide_entries SET
           support = (SELECT COUNT(DISTINCT message_id) FROM guide_evidence WHERE entry_id = ?1 AND NOT contradicts),
           contradict = (SELECT COUNT(DISTINCT message_id) FROM guide_evidence WHERE entry_id = ?1 AND contradicts)
         WHERE id = ?1",
    )?
    .execute([entry_id])?;
    Ok(())
}

pub fn snapshot(conn: &Connection, id: i64) -> StoreResult<Option<Snapshot>> {
    Ok(match get_entry(conn, id)? {
        Some(entry) => Some(Snapshot { evidence: evidence(conn, id)?, entry }),
        None => None,
    })
}

/// Make the entries `ids` exactly as in `snapshots`: written back (with
/// their evidence) when present, deleted when not.
pub fn restore(tx: &Transaction<'_>, ids: &[i64], snapshots: &[Snapshot]) -> StoreResult<()> {
    for id in ids {
        match snapshots.iter().find(|s| s.entry.id == *id) {
            Some(s) => {
                if !write_entry(tx, &s.entry)? {
                    insert_entry(tx, &s.entry)?;
                }
                tx.prepare_cached("DELETE FROM guide_evidence WHERE entry_id = ?1")?.execute([id])?;
                add_evidence(tx, *id, &s.evidence)?;
                // Evidence counts come from the snapshot, which may predate
                // quotes that were dropped.
                write_entry(tx, &s.entry)?;
            }
            None => {
                delete_entry(tx, *id)?;
            }
        }
    }
    Ok(())
}

/// Record a change as the entries before and after; returns its id.
pub fn record_change(
    tx: &Transaction<'_>,
    reason: &str,
    before: &[Snapshot],
    after: &[Snapshot],
    now: Millis,
) -> StoreResult<i64> {
    tx.prepare_cached(
        "INSERT INTO guide_changes (reason, before_json, after_json, created_at) VALUES (?1, ?2, ?3, ?4)",
    )?
    .execute(params![reason, serde_json::to_string(before)?, serde_json::to_string(after)?, now])?;
    let id = tx.last_insert_rowid();
    // Keep the last 200 changes: far more than the undo stack's 50.
    tx.prepare_cached("DELETE FROM guide_changes WHERE id <= ?1")?.execute([id - 200])?;
    Ok(id)
}

/// A recorded change: its reason, and the entries it touched before and
/// after it.
pub type Change = (String, Vec<Snapshot>, Vec<Snapshot>);

pub fn get_change(conn: &Connection, id: i64) -> StoreResult<Option<Change>> {
    let row: Option<(String, String, String)> = conn
        .prepare_cached("SELECT reason, before_json, after_json FROM guide_changes WHERE id = ?1")?
        .query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    Ok(match row {
        Some((reason, before, after)) => Some((reason, serde_json::from_str(&before)?, serde_json::from_str(&after)?)),
        None => None,
    })
}

/// Save the accepted guide as a new version; returns its number.
pub fn record_version(tx: &Transaction<'_>, reason: &str, now: Millis) -> StoreResult<i64> {
    let mut stmt = tx.prepare_cached(&format!(
        "SELECT {ENTRY_COLUMNS} FROM guide_entries WHERE status = 'accepted' ORDER BY category, id"
    ))?;
    let accepted = stmt.query_map([], entry)?.collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    tx.prepare_cached("INSERT INTO guide_versions (reason, snapshot_json, created_at) VALUES (?1, ?2, ?3)")?
        .execute(params![reason, serde_json::to_string(&accepted)?, now])?;
    Ok(tx.last_insert_rowid())
}

/// The latest version number (0 before the first).
pub fn current_version(conn: &Connection) -> StoreResult<i64> {
    Ok(conn.prepare_cached("SELECT COALESCE(MAX(id), 0) FROM guide_versions")?.query_row([], |r| r.get(0))?)
}

/// Versions, newest first: number, reason, time, accepted entries then.
pub fn versions(conn: &Connection, limit: u32) -> StoreResult<Vec<(i64, String, Millis, usize)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, reason, created_at, json_array_length(snapshot_json) FROM guide_versions ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? as usize)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

// MARK: Runs

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunRow {
    pub id: i64,
    /// `first`, `newer`, `older`, `improve`, `recheck`.
    pub kind: String,
    /// A category or audience group for `improve`.
    pub focus: Option<String>,
    /// `running`, `paused`, `done`, `cancelled`, `failed`.
    pub status: String,
    pub batch_size: i64,
    pub total: i64,
    pub done: i64,
    pub agent: Option<String>,
    pub error: Option<String>,
    pub started_at: Millis,
    pub finished_at: Option<Millis>,
}

const RUN_COLUMNS: &str = "id, kind, focus, status, batch_size, total, done, agent, error, started_at, finished_at";

fn run(r: &Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        focus: r.get(2)?,
        status: r.get(3)?,
        batch_size: r.get(4)?,
        total: r.get(5)?,
        done: r.get(6)?,
        agent: r.get(7)?,
        error: r.get(8)?,
        started_at: r.get(9)?,
        finished_at: r.get(10)?,
    })
}

/// Start a run over `message_ids`, split into batches of `batch_size`.
pub fn create_run(
    tx: &Transaction<'_>,
    kind: &str,
    focus: Option<&str>,
    agent: Option<&str>,
    message_ids: &[String],
    batch_size: usize,
    now: Millis,
) -> StoreResult<i64> {
    let batch_size = batch_size.max(1);
    tx.prepare_cached(
        "INSERT INTO guide_runs (kind, focus, status, batch_size, total, agent, started_at)
         VALUES (?1, ?2, 'running', ?3, ?4, ?5, ?6)",
    )?
    .execute(params![kind, focus, batch_size as i64, message_ids.len() as i64, agent, now])?;
    let id = tx.last_insert_rowid();
    let mut stmt = tx.prepare_cached(
        "INSERT OR IGNORE INTO guide_run_messages (run_id, message_id, batch, position) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (i, m) in message_ids.iter().enumerate() {
        stmt.execute(params![id, m, (i / batch_size) as i64, i as i64])?;
    }
    Ok(id)
}

pub fn get_run(conn: &Connection, id: i64) -> StoreResult<Option<RunRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {RUN_COLUMNS} FROM guide_runs WHERE id = ?1"))?
        .query_row([id], run)
        .optional()?)
}

/// The run still in progress (running or paused), if any: one at a time.
pub fn active_run(conn: &Connection) -> StoreResult<Option<RunRow>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {RUN_COLUMNS} FROM guide_runs WHERE status IN ('running', 'paused') ORDER BY id DESC LIMIT 1"
        ))?
        .query_row([], run)
        .optional()?)
}

/// Runs, newest first.
pub fn runs(conn: &Connection, limit: u32) -> StoreResult<Vec<RunRow>> {
    let mut stmt = conn.prepare_cached(&format!("SELECT {RUN_COLUMNS} FROM guide_runs ORDER BY id DESC LIMIT ?1"))?;
    let rows = stmt.query_map([limit], run)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The next batch not yet analysed: its number and messages.
pub fn next_batch(conn: &Connection, run_id: i64) -> StoreResult<Option<(i64, Vec<String>)>> {
    let batch: Option<i64> = conn
        .prepare_cached("SELECT MIN(batch) FROM guide_run_messages WHERE run_id = ?1 AND NOT done")?
        .query_row([run_id], |r| r.get(0))?;
    let Some(batch) = batch else { return Ok(None) };
    let mut stmt = conn.prepare_cached(
        "SELECT message_id FROM guide_run_messages WHERE run_id = ?1 AND batch = ?2 ORDER BY position",
    )?;
    let ids = stmt.query_map(params![run_id, batch], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
    Ok(Some((batch, ids)))
}

/// Mark a batch analysed and count it in the run's progress.
pub fn finish_batch(tx: &Transaction<'_>, run_id: i64, batch: i64) -> StoreResult<()> {
    tx.prepare_cached("UPDATE guide_run_messages SET done = 1 WHERE run_id = ?1 AND batch = ?2")?
        .execute(params![run_id, batch])?;
    tx.prepare_cached(
        "UPDATE guide_runs SET done = (SELECT COUNT(*) FROM guide_run_messages WHERE run_id = ?1 AND done) WHERE id = ?1",
    )?
    .execute([run_id])?;
    Ok(())
}

pub fn set_run_status(
    tx: &Transaction<'_>,
    run_id: i64,
    status: &str,
    error: Option<&str>,
    now: Millis,
) -> StoreResult<()> {
    let finished = matches!(status, "done" | "cancelled" | "failed").then_some(now);
    tx.prepare_cached("UPDATE guide_runs SET status = ?2, error = ?3, finished_at = ?4 WHERE id = ?1")?
        .execute(params![run_id, status, error, finished])?;
    Ok(())
}

/// Every message some run has analysed.
pub fn analysed_messages(conn: &Connection) -> StoreResult<BTreeSet<String>> {
    let mut stmt = conn.prepare_cached("SELECT DISTINCT message_id FROM guide_run_messages WHERE done")?;
    let ids = stmt.query_map([], |r| r.get(0))?.collect::<Result<BTreeSet<String>, _>>()?;
    Ok(ids)
}

// MARK: Audience groups

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GroupRow {
    pub id: i64,
    pub name: String,
    /// `suggested`, `confirmed` or `rejected`.
    pub status: String,
    pub description: String,
    pub position: i64,
    /// Addresses or `@domain`s.
    pub members: Vec<String>,
}

pub fn groups(conn: &Connection) -> StoreResult<Vec<GroupRow>> {
    let mut stmt = conn
        .prepare_cached("SELECT id, name, status, description, position FROM audience_groups ORDER BY position, id")?;
    let mut rows = stmt
        .query_map([], |r| {
            Ok(GroupRow {
                id: r.get(0)?,
                name: r.get(1)?,
                status: r.get(2)?,
                description: r.get(3)?,
                position: r.get(4)?,
                members: vec![],
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut members =
        conn.prepare_cached("SELECT pattern FROM audience_members WHERE group_id = ?1 ORDER BY pattern")?;
    for g in &mut rows {
        g.members = members.query_map([g.id], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
    }
    Ok(rows)
}

/// Add or update a group by name (any case); members are replaced.
pub fn save_group(tx: &Transaction<'_>, g: &GroupRow) -> StoreResult<i64> {
    tx.prepare_cached(
        "INSERT INTO audience_groups (name, status, description, position) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (name) DO UPDATE SET status = ?2, description = ?3, position = ?4",
    )?
    .execute(params![g.name, g.status, g.description, g.position])?;
    let id: i64 =
        tx.prepare_cached("SELECT id FROM audience_groups WHERE name = ?1")?.query_row([&g.name], |r| r.get(0))?;
    tx.prepare_cached("DELETE FROM audience_members WHERE group_id = ?1")?.execute([id])?;
    let mut stmt = tx.prepare_cached("INSERT OR IGNORE INTO audience_members (group_id, pattern) VALUES (?1, ?2)")?;
    for m in &g.members {
        stmt.execute(params![id, m.trim().to_lowercase()])?;
    }
    Ok(id)
}

pub fn delete_group(tx: &Transaction<'_>, id: i64) -> StoreResult<bool> {
    Ok(tx.prepare_cached("DELETE FROM audience_groups WHERE id = ?1")?.execute([id])? > 0)
}

// MARK: Examples and settings

pub fn examples(conn: &Connection) -> StoreResult<Vec<(String, String)>> {
    let mut stmt = conn.prepare_cached("SELECT message_id, message_type FROM guide_examples ORDER BY added_at")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn set_example(tx: &Transaction<'_>, message_id: &str, message_type: Option<&str>, now: Millis) -> StoreResult<()> {
    match message_type {
        Some(t) => tx
            .prepare_cached(
                "INSERT INTO guide_examples (message_id, message_type, added_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT (message_id) DO UPDATE SET message_type = ?2",
            )?
            .execute(params![message_id, t, now])?,
        None => tx.prepare_cached("DELETE FROM guide_examples WHERE message_id = ?1")?.execute([message_id])?,
    };
    Ok(())
}

pub fn meta(conn: &Connection, key: &str) -> StoreResult<Option<String>> {
    Ok(conn.prepare_cached("SELECT value FROM guide_meta WHERE key = ?1")?.query_row([key], |r| r.get(0)).optional()?)
}

pub fn set_meta(tx: &Transaction<'_>, key: &str, value: &str) -> StoreResult<()> {
    tx.prepare_cached(
        "INSERT INTO guide_meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
    )?
    .execute(params![key, value])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    struct Scratch(std::path::PathBuf, Db);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("openagc-store-guide-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("mail.sqlite")).unwrap();
        Scratch(dir, db)
    }

    fn row(category: &str, statement: &str, status: &str) -> EntryRow {
        EntryRow {
            category: category.into(),
            kind: "guideline".into(),
            statement: statement.into(),
            scope_json: "{}".into(),
            status: status.into(),
            source: "learned".into(),
            created_at: 1,
            updated_at: 1,
            ..Default::default()
        }
    }

    fn quote(m: &str, q: &str) -> EvidenceRow {
        EvidenceRow { message_id: m.into(), quote: q.into(), contradicts: false }
    }

    #[test]
    fn entries_merge_by_statement_and_count_their_evidence() {
        let s = scratch("entries");
        let id =
            s.1.write_blocking(|tx| {
                let id = insert_entry(tx, &row("B6", "Sign off with 'John'.", "proposed"))?;
                add_evidence(tx, id, &[quote("m1", "John"), quote("m2", "John"), quote("m2", "Thanks, John")])?;
                add_evidence(
                    tx,
                    id,
                    &[
                        quote("m1", "John"),
                        EvidenceRow { message_id: "m3".into(), quote: "JK".into(), contradicts: true },
                    ],
                )?;
                Ok(id)
            })
            .unwrap();
        let e = s.1.read_blocking(move |c| get_entry(c, id)).unwrap().unwrap();
        assert_eq!((e.support, e.contradict), (2, 1), "distinct messages, not quotes");
        let found = s.1.read_blocking(|c| find_by_norm(c, "B6", "  sign OFF with 'john' ")).unwrap();
        assert_eq!(found.map(|f| f.id), Some(id));
        assert!(s.1.read_blocking(|c| find_by_norm(c, "B5", "sign off with 'john'")).unwrap().is_none());
        assert_eq!(s.1.read_blocking(|c| list_entries(c, &["accepted"])).unwrap().len(), 0);
    }

    #[test]
    fn a_recorded_change_restores_exactly() {
        let s = scratch("undo");
        let (id, change) =
            s.1.write_blocking(|tx| {
                let id = insert_entry(tx, &row("C1", "Use US spelling", "proposed"))?;
                add_evidence(tx, id, &[quote("m1", "color")])?;
                Ok(id)
            })
            .map(|id| (id, 0))
            .unwrap();
        let _ = change;
        let before = s.1.read_blocking(move |c| snapshot(c, id)).unwrap().unwrap();
        // Accept it and delete another's evidence; then add a new entry.
        let (change, new_id) =
            s.1.write_blocking(move |tx| {
                let mut e = before.entry.clone();
                e.status = "accepted".into();
                write_entry(tx, &e)?;
                let new_id = insert_entry(tx, &row("C4", "Use contractions", "accepted"))?;
                let after = vec![
                    Snapshot { entry: e, evidence: before.evidence.clone() },
                    Snapshot {
                        entry: EntryRow { id: new_id, ..row("C4", "Use contractions", "accepted") },
                        evidence: vec![],
                    },
                ];
                let change = record_change(tx, "decide", std::slice::from_ref(&before), &after, 2)?;
                record_version(tx, "decide", 2)?;
                Ok((change, new_id))
            })
            .unwrap();
        assert_eq!(s.1.read_blocking(current_version).unwrap(), 1);
        let (_, b, a) = s.1.read_blocking(move |c| get_change(c, change)).unwrap().unwrap();
        s.1.write_blocking(move |tx| restore(tx, &[id, new_id], &b)).unwrap();
        let e = s.1.read_blocking(move |c| get_entry(c, id)).unwrap().unwrap();
        assert_eq!(e.status, "proposed");
        assert_eq!(e.support, 1);
        assert!(s.1.read_blocking(move |c| get_entry(c, new_id)).unwrap().is_none(), "undo removes what it added");
        s.1.write_blocking(move |tx| restore(tx, &[id, new_id], &a)).unwrap();
        assert_eq!(s.1.read_blocking(move |c| get_entry(c, new_id)).unwrap().unwrap().statement, "Use contractions");
        assert_eq!(s.1.read_blocking(move |c| get_entry(c, id)).unwrap().unwrap().status, "accepted");
        let v = s.1.read_blocking(|c| versions(c, 5)).unwrap();
        assert_eq!(v[0].3, 2, "the version held both accepted entries");
    }

    #[test]
    fn runs_go_batch_by_batch_and_remember_what_was_analysed() {
        let s = scratch("runs");
        let ids: Vec<String> = (0..45).map(|i| format!("m{i}")).collect();
        let run_id =
            s.1.write_blocking(move |tx| create_run(tx, "first", None, Some("claude-code"), &ids, 20, 5)).unwrap();
        let mut batches = vec![];
        while let Some((n, msgs)) = s.1.read_blocking(move |c| next_batch(c, run_id)).unwrap() {
            batches.push(msgs.len());
            s.1.write_blocking(move |tx| finish_batch(tx, run_id, n)).unwrap();
        }
        assert_eq!(batches, [20, 20, 5]);
        let r = s.1.read_blocking(move |c| get_run(c, run_id)).unwrap().unwrap();
        assert_eq!((r.total, r.done, r.status.as_str()), (45, 45, "running"));
        assert_eq!(s.1.read_blocking(active_run).unwrap().map(|r| r.id), Some(run_id));
        s.1.write_blocking(move |tx| set_run_status(tx, run_id, "done", None, 9)).unwrap();
        assert!(s.1.read_blocking(active_run).unwrap().is_none());
        assert_eq!(s.1.read_blocking(analysed_messages).unwrap().len(), 45);
    }

    #[test]
    fn groups_keep_their_members_and_names_are_unique() {
        let s = scratch("groups");
        s.1.write_blocking(|tx| {
            save_group(
                tx,
                &GroupRow {
                    name: "Customers".into(),
                    status: "suggested".into(),
                    members: vec!["@acme.com".into(), " Ann@Example.com ".into()],
                    ..Default::default()
                },
            )?;
            save_group(tx, &GroupRow { name: "customers".into(), status: "confirmed".into(), ..Default::default() })?;
            Ok(())
        })
        .unwrap();
        let g = s.1.read_blocking(groups).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].status, "confirmed");
        assert!(g[0].members.is_empty(), "members are replaced");
        s.1.write_blocking(|tx| set_meta(tx, "signature", "John")).unwrap();
        assert_eq!(s.1.read_blocking(|c| meta(c, "signature")).unwrap().as_deref(), Some("John"));
    }
}
