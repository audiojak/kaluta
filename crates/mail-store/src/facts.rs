//! Facts about the user (spec §14.11): what an AI may use when it writes
//! for them, in categories, with where each came from. Every change is
//! recorded with its rows before and after, for exact undo (ADR 0006); a
//! fact's id is never reused. The same schema serves an account's store
//! and the global facts store (ADR 0012).

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::error::StoreResult;

/// The migration that made these tables; the guide's F3 entries move
/// here right after it.
pub const MIGRATION_VERSION: u32 = 17;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FactRow {
    pub id: i64,
    pub category: String,
    pub label: String,
    pub value: String,
    /// `free`, `ask` or `never`.
    pub use_: String,
    pub as_of: Option<Millis>,
    /// `you`, `learned` or `writing_help`.
    pub source: String,
    /// `proposed`, `accepted` or `rejected`.
    pub status: String,
    pub created_at: Millis,
    pub updated_at: Millis,
}

/// A custom category, or a built-in one's stored state (hidden, order).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CategoryRow {
    pub key: String,
    pub name: String,
    pub description: String,
    pub position: i64,
    pub builtin: bool,
    pub hidden: bool,
    pub default_use: String,
    pub starter: Option<String>,
    pub created_at: Millis,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FactEvidence {
    pub message_id: String,
    pub quote: String,
}

/// What a change touched: facts (with their evidence) and categories.
/// Undo puts back exactly these.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub facts: Vec<(FactRow, Vec<FactEvidence>)>,
    pub categories: Vec<CategoryRow>,
    /// The global store's side of a change that moved facts between it and
    /// an account (ADR 0012): recorded in the account's store with both
    /// sides, and put back with them.
    #[serde(default)]
    pub global_facts: Vec<(FactRow, Vec<FactEvidence>)>,
    #[serde(default)]
    pub global_categories: Vec<CategoryRow>,
    /// Analysis proposals the change decided (spec §14.10), put back with
    /// the facts.
    #[serde(default)]
    pub proposals: Vec<crate::analysis::ProposalSnapshot>,
}

const FACT_COLUMNS: &str = "id, category, label, value, use, as_of, source, status, created_at, updated_at";

fn fact(r: &Row<'_>) -> rusqlite::Result<FactRow> {
    Ok(FactRow {
        id: r.get(0)?,
        category: r.get(1)?,
        label: r.get(2)?,
        value: r.get(3)?,
        use_: r.get(4)?,
        as_of: r.get(5)?,
        source: r.get(6)?,
        status: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

const CATEGORY_COLUMNS: &str = "key, name, description, position, builtin, hidden, default_use, starter, created_at";

fn category(r: &Row<'_>) -> rusqlite::Result<CategoryRow> {
    Ok(CategoryRow {
        key: r.get(0)?,
        name: r.get(1)?,
        description: r.get(2)?,
        position: r.get(3)?,
        builtin: r.get(4)?,
        hidden: r.get(5)?,
        default_use: r.get(6)?,
        starter: r.get(7)?,
        created_at: r.get(8)?,
    })
}

/// Facts in any of `statuses`, by category and label.
pub fn list(conn: &Connection, statuses: &[&str]) -> StoreResult<Vec<FactRow>> {
    let all: Vec<FactRow> = conn
        .prepare_cached(&format!("SELECT {FACT_COLUMNS} FROM facts ORDER BY category, label COLLATE NOCASE, id"))?
        .query_map([], fact)?
        .collect::<Result<_, _>>()?;
    Ok(all.into_iter().filter(|f| statuses.contains(&f.status.as_str())).collect())
}

pub fn get(conn: &Connection, id: i64) -> StoreResult<Option<FactRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {FACT_COLUMNS} FROM facts WHERE id = ?1"))?
        .query_row([id], fact)
        .optional()?)
}

/// Insert a fact (a new id when `id` is 0, else that id, for undo).
pub fn insert(tx: &Transaction<'_>, f: &FactRow) -> StoreResult<i64> {
    let id = (f.id != 0).then_some(f.id);
    tx.prepare_cached(
        "INSERT INTO facts (id, category, label, value, use, as_of, source, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?
    .execute(params![
        id,
        f.category,
        f.label,
        f.value,
        f.use_,
        f.as_of,
        f.source,
        f.status,
        f.created_at,
        f.updated_at
    ])?;
    Ok(tx.last_insert_rowid())
}

/// Write a fact back; `false` when it is not there.
pub fn write(tx: &Transaction<'_>, f: &FactRow) -> StoreResult<bool> {
    Ok(tx
        .prepare_cached(
            "UPDATE facts SET category = ?2, label = ?3, value = ?4, use = ?5, as_of = ?6, source = ?7, status = ?8,
               created_at = ?9, updated_at = ?10 WHERE id = ?1",
        )?
        .execute(params![
            f.id,
            f.category,
            f.label,
            f.value,
            f.use_,
            f.as_of,
            f.source,
            f.status,
            f.created_at,
            f.updated_at
        ])?
        > 0)
}

pub fn delete(tx: &Transaction<'_>, id: i64) -> StoreResult<()> {
    tx.prepare_cached("DELETE FROM facts WHERE id = ?1")?.execute([id])?;
    Ok(())
}

pub fn evidence(conn: &Connection, fact_id: i64) -> StoreResult<Vec<FactEvidence>> {
    Ok(conn
        .prepare_cached("SELECT message_id, quote FROM fact_evidence WHERE fact_id = ?1 ORDER BY added_at, message_id")?
        .query_map([fact_id], |r| Ok(FactEvidence { message_id: r.get(0)?, quote: r.get(1)? }))?
        .collect::<Result<_, _>>()?)
}

pub fn add_evidence(tx: &Transaction<'_>, fact_id: i64, quotes: &[FactEvidence], now: Millis) -> StoreResult<()> {
    let mut add = tx.prepare_cached(
        "INSERT OR IGNORE INTO fact_evidence (fact_id, message_id, quote, added_at) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for q in quotes {
        add.execute(params![fact_id, q.message_id, q.quote, now])?;
    }
    Ok(())
}

/// Stored categories: custom ones and built-ins' state, in order.
pub fn categories(conn: &Connection) -> StoreResult<Vec<CategoryRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {CATEGORY_COLUMNS} FROM fact_categories ORDER BY position, created_at, key"))?
        .query_map([], category)?
        .collect::<Result<_, _>>()?)
}

pub fn get_category(conn: &Connection, key: &str) -> StoreResult<Option<CategoryRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {CATEGORY_COLUMNS} FROM fact_categories WHERE key = ?1"))?
        .query_row([key], category)
        .optional()?)
}

pub fn put_category(tx: &Transaction<'_>, c: &CategoryRow) -> StoreResult<()> {
    tx.prepare_cached(
        "INSERT OR REPLACE INTO fact_categories (key, name, description, position, builtin, hidden, default_use,
           starter, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?
    .execute(params![
        c.key,
        c.name,
        c.description,
        c.position,
        c.builtin,
        c.hidden,
        c.default_use,
        c.starter,
        c.created_at
    ])?;
    Ok(())
}

pub fn delete_category(tx: &Transaction<'_>, key: &str) -> StoreResult<()> {
    tx.prepare_cached("DELETE FROM fact_categories WHERE key = ?1")?.execute([key])?;
    Ok(())
}

/// The facts and categories as they are now, for a change's record.
pub fn snapshot(conn: &Connection, fact_ids: &[i64], category_keys: &[String]) -> StoreResult<Snapshot> {
    let mut out = Snapshot::default();
    for id in fact_ids {
        if let Some(f) = get(conn, *id)? {
            out.facts.push((f, evidence(conn, *id)?));
        }
    }
    for key in category_keys {
        out.categories.extend(get_category(conn, key)?);
    }
    Ok(out)
}

/// Put facts and categories back as `to` has them: rows it lacks are
/// removed. Ids and keys come from both sides of the change.
pub fn restore(
    tx: &Transaction<'_>,
    fact_ids: &[i64],
    category_keys: &[String],
    to: &Snapshot,
    now: Millis,
) -> StoreResult<()> {
    for id in fact_ids {
        match to.facts.iter().find(|(f, _)| f.id == *id) {
            Some((f, quotes)) => {
                if !write(tx, f)? {
                    insert(tx, f)?;
                }
                tx.prepare_cached("DELETE FROM fact_evidence WHERE fact_id = ?1")?.execute([id])?;
                add_evidence(tx, *id, quotes, now)?;
            }
            None => delete(tx, *id)?,
        }
    }
    for key in category_keys {
        match to.categories.iter().find(|c| &c.key == key) {
            Some(c) => put_category(tx, c)?,
            None => delete_category(tx, key)?,
        }
    }
    crate::analysis::restore_proposals(tx, &to.proposals, now)?;
    Ok(())
}

pub fn record_change(
    tx: &Transaction<'_>,
    reason: &str,
    before: &Snapshot,
    after: &Snapshot,
    now: Millis,
) -> StoreResult<i64> {
    tx.prepare_cached(
        "INSERT INTO fact_changes (reason, before_json, after_json, created_at) VALUES (?1, ?2, ?3, ?4)",
    )?
    .execute(params![reason, serde_json::to_string(before)?, serde_json::to_string(after)?, now])?;
    let id = tx.last_insert_rowid();
    // Far more than the undo stack keeps.
    tx.prepare_cached("DELETE FROM fact_changes WHERE id <= ?1")?.execute([id - 200])?;
    Ok(id)
}

pub fn get_change(conn: &Connection, id: i64) -> StoreResult<Option<(String, Snapshot, Snapshot)>> {
    let row: Option<(String, String, String)> = conn
        .prepare_cached("SELECT reason, before_json, after_json FROM fact_changes WHERE id = ?1")?
        .query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    Ok(match row {
        Some((reason, before, after)) => Some((reason, serde_json::from_str(&before)?, serde_json::from_str(&after)?)),
        None => None,
    })
}

/// Where a fact the interview wrote ("My time zone: Pacific") belongs:
/// its category key and label, by the interview's templates (spec §14.11).
pub fn place(statement: &str) -> (String, String, String) {
    // "Label: value": a colon followed by a space, so links ("https://")
    // and times ("9:00") stay whole.
    let split =
        statement.char_indices().find(|(i, c)| *c == ':' && statement[i + 1..].starts_with(char::is_whitespace));
    let (label, value) = match split {
        Some((i, _)) if !statement[i + 1..].trim().is_empty() && i <= 60 => {
            (statement[..i].trim(), statement[i + 1..].trim())
        }
        _ => ("Note", statement.trim()),
    };
    let bare = label.strip_prefix("My ").or_else(|| label.strip_prefix("my ")).unwrap_or(label).trim();
    let lower = bare.to_lowercase();
    let (key, label) = if lower.starts_with("role") || lower == "title" || lower.starts_with("job") {
        ("work", "Occupation or role")
    } else if lower.contains("calendar") {
        ("availability", "Calendar link")
    } else if lower.contains("time zone") || lower == "timezone" {
        ("availability", "Time zone")
    } else if lower.contains("working hours") || lower.contains("usual hours") {
        ("availability", "Usual hours")
    } else if lower.contains("phone") {
        ("contact", "Phone")
    } else if lower.contains("pronoun") {
        ("identity", "Pronouns")
    } else {
        return ("other".into(), capitalised(bare), value.into());
    };
    (key.into(), label.into(), value.into())
}

fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => "Note".into(),
    }
}

fn now_millis() -> Millis {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as Millis).unwrap_or(0)
}

/// Move the writing guide's F3 entries ("Facts about me") into facts, in
/// the migration's transaction, with a guide version noting it. Accepted,
/// proposed and rejected alike, so a rejected fact is not proposed again.
pub fn move_guide_facts(tx: &Transaction<'_>) -> StoreResult<usize> {
    let rows: Vec<(i64, String, String, String, Millis, Millis)> = tx
        .prepare(
            "SELECT id, statement, status, source, created_at, updated_at FROM guide_entries WHERE category = 'F3'",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
        .collect::<Result<_, _>>()?;
    if rows.is_empty() {
        return Ok(0);
    }
    let now = now_millis();
    let mut accepted = false;
    let mut used: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
    for (id, statement, status, source, created_at, updated_at) in &rows {
        let (category, mut label, value) = place(statement);
        // One label per category: a second "Note" becomes "Note 2".
        if status == "accepted" {
            let base = label.clone();
            let mut n = 1;
            while !used.insert((category.clone(), label.to_lowercase())) {
                n += 1;
                label = format!("{base} {n}");
            }
        }
        let f = FactRow {
            id: 0,
            use_: if category == "people" { "ask".into() } else { "free".into() },
            category,
            label,
            value,
            as_of: None,
            source: if source == "learned" { "learned".into() } else { "you".into() },
            status: status.clone(),
            created_at: *created_at,
            updated_at: *updated_at,
        };
        let fact_id = insert(tx, &f)?;
        let quotes: Vec<FactEvidence> = crate::guide::evidence(tx, *id)?
            .into_iter()
            .filter(|e| !e.contradicts)
            .map(|e| FactEvidence { message_id: e.message_id, quote: e.quote })
            .collect();
        add_evidence(tx, fact_id, &quotes, now)?;
        crate::guide::delete_entry(tx, *id)?;
        accepted |= status == "accepted";
    }
    if accepted {
        crate::guide::record_version(tx, "facts moved to Facts", now)?;
    }
    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interview_facts_find_their_place() {
        let p = |s: &str| place(s);
        assert_eq!(p("My role: CEO, Actual AI"), ("work".into(), "Occupation or role".into(), "CEO, Actual AI".into()));
        assert_eq!(p("My calendar link for booking time: https://cal.com/j").0, "availability");
        assert_eq!(p("My time zone: Pacific").1, "Time zone");
        assert_eq!(p("My working hours: 9 to 5").1, "Usual hours");
        assert_eq!(p("My phone number: +1 555").0, "contact");
        assert_eq!(p("My pronouns: they/them").0, "identity");
        assert_eq!(
            p("What my company does: email software"),
            ("other".into(), "What my company does".into(), "email software".into())
        );
        assert_eq!(p("I live in Oakland"), ("other".into(), "Note".into(), "I live in Oakland".into()));
        assert_eq!(p("Book time at https://cal.com/j").2, "Book time at https://cal.com/j", "links stay whole");
        assert_eq!(
            p("My working hours: 9:00-17:00"),
            ("availability".into(), "Usual hours".into(), "9:00-17:00".into())
        );
    }

    #[test]
    fn opening_an_old_store_moves_its_f3_entries_into_facts() {
        let dir = std::env::temp_dir().join(format!("openagc-facts-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mail.sqlite");
        // A store as it was before facts had their own tables.
        {
            let mut conn = Connection::open(&path).unwrap();
            let tx = conn.transaction().unwrap();
            for (i, sql) in crate::db::migrations().iter().enumerate().take(MIGRATION_VERSION as usize - 1) {
                tx.execute_batch(sql).unwrap();
                tx.execute_batch(&format!("PRAGMA user_version = {}", i + 1)).unwrap();
            }
            for (statement, status) in
                [("My role: CEO", "accepted"), ("My time zone: Pacific", "rejected"), ("Sign off with 'J'", "accepted")]
            {
                let category = if statement.starts_with("My") { "F3" } else { "B6" };
                tx.execute(
                    "INSERT INTO guide_entries (category, kind, statement, norm, scope_json, status, source, created_at,
                       updated_at) VALUES (?1, 'fact', ?2, ?4, '{}', ?3, 'you', 1, 2)",
                    params![category, statement, status, crate::guide::norm(statement)],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }
        let db = crate::Db::open(&path).unwrap();
        let facts = db.read_blocking(|c| list(c, &["accepted", "rejected"])).unwrap();
        assert_eq!(facts.len(), 2);
        let role = facts.iter().find(|f| f.label == "Occupation or role").unwrap();
        assert_eq!((role.category.as_str(), role.value.as_str(), role.status.as_str()), ("work", "CEO", "accepted"));
        assert!(facts.iter().any(|f| f.label == "Time zone" && f.status == "rejected"));
        let left: Vec<String> = db
            .read_blocking(|c| {
                Ok(c.prepare("SELECT category FROM guide_entries")?
                    .query_map([], |r| r.get(0))?
                    .collect::<Result<Vec<String>, _>>()?)
            })
            .unwrap();
        assert_eq!(left, vec!["B6".to_owned()], "the F3 entries left the guide");
        let version: String = db
            .read_blocking(|c| Ok(c.query_row("SELECT reason FROM guide_versions ORDER BY id DESC", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(version, "facts moved to Facts");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_change_is_undone_exactly_and_ids_are_not_reused() {
        let dir = std::env::temp_dir().join(format!("openagc-facts-undo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = crate::Db::open(&dir.join("mail.sqlite")).unwrap();
        let row = FactRow {
            category: "work".into(),
            label: "Team".into(),
            value: "Mail".into(),
            use_: "free".into(),
            source: "you".into(),
            status: "accepted".into(),
            ..Default::default()
        };
        let (id, change) = db
            .write_blocking(move |tx| {
                let before = snapshot(tx, &[], &[])?;
                let id = insert(tx, &row)?;
                let after = snapshot(tx, &[id], &[])?;
                Ok((id, record_change(tx, "add", &before, &after, 1)?))
            })
            .unwrap();
        db.write_blocking(move |tx| {
            let (_, before, _) = get_change(tx, change)?.unwrap();
            restore(tx, &[id], &[], &before, 2)
        })
        .unwrap();
        assert!(db.read_blocking(move |c| get(c, id)).unwrap().is_none(), "undone");
        let next = db
            .write_blocking(|tx| {
                insert(
                    tx,
                    &FactRow {
                        category: "other".into(),
                        label: "x".into(),
                        value: "y".into(),
                        use_: "free".into(),
                        source: "you".into(),
                        status: "accepted".into(),
                        ..Default::default()
                    },
                )
            })
            .unwrap();
        assert!(next > id, "an id is never used twice");
        db.write_blocking(move |tx| {
            let (_, _, after) = get_change(tx, change)?.unwrap();
            restore(tx, &[id], &[], &after, 3)
        })
        .unwrap();
        assert_eq!(db.read_blocking(move |c| get(c, id)).unwrap().unwrap().value, "Mail", "redone with its own id");
        let _ = std::fs::remove_dir_all(dir);
    }
}
