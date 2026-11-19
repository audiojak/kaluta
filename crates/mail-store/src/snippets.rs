//! Snippets (spec §14.5 amendment 2026-11-18): saved text the user puts
//! into a message by name. They live in the global store (ADR 0012).

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::{StoreError, StoreResult};

/// A saved piece of text, as HTML.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snippet {
    /// 0 until saved.
    pub id: i64,
    pub name: String,
    pub body_html: String,
    pub updated_at: Millis,
}

/// Every snippet, by name.
pub fn list(conn: &Connection) -> StoreResult<Vec<Snippet>> {
    let mut stmt =
        conn.prepare_cached("SELECT id, name, body_html, updated_at FROM snippets ORDER BY name COLLATE NOCASE, id")?;
    let rows = stmt.query_map([], |r| {
        Ok(Snippet { id: r.get(0)?, name: r.get(1)?, body_html: r.get(2)?, updated_at: r.get(3)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, id: i64) -> StoreResult<Option<Snippet>> {
    Ok(conn
        .prepare_cached("SELECT id, name, body_html, updated_at FROM snippets WHERE id = ?1")?
        .query_row([id], |r| {
            Ok(Snippet { id: r.get(0)?, name: r.get(1)?, body_html: r.get(2)?, updated_at: r.get(3)? })
        })
        .optional()?)
}

/// Save a snippet (a new one when its id is 0); returns its id. A snippet
/// needs a name.
pub fn save(tx: &Transaction<'_>, s: &Snippet, now: Millis) -> StoreResult<i64> {
    let name = s.name.trim();
    if name.is_empty() {
        return Err(StoreError::Invalid("A snippet needs a name.".into()));
    }
    if s.id == 0 {
        tx.prepare_cached("INSERT INTO snippets (name, body_html, updated_at) VALUES (?1, ?2, ?3)")?
            .execute(params![name, s.body_html, now])?;
        return Ok(tx.last_insert_rowid());
    }
    let changed = tx
        .prepare_cached("UPDATE snippets SET name = ?2, body_html = ?3, updated_at = ?4 WHERE id = ?1")?
        .execute(params![s.id, name, s.body_html, now])?;
    if changed == 0 {
        return Err(StoreError::NotFound(format!("snippet {}", s.id)));
    }
    Ok(s.id)
}

/// Delete a snippet; the snippet it was, to put back (undo).
pub fn delete(tx: &Transaction<'_>, id: i64) -> StoreResult<Option<Snippet>> {
    let before = get(tx, id)?;
    tx.prepare_cached("DELETE FROM snippets WHERE id = ?1")?.execute([id])?;
    Ok(before)
}

/// Put a deleted snippet back with its id (undo).
pub fn restore(tx: &Transaction<'_>, s: &Snippet) -> StoreResult<()> {
    tx.prepare_cached(
        "INSERT INTO snippets (id, name, body_html, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (id) DO UPDATE SET name = excluded.name, body_html = excluded.body_html,
           updated_at = excluded.updated_at",
    )?
    .execute(params![s.id, s.name, s.body_html, s.updated_at])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    struct Store(std::path::PathBuf, Db);

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("kaluta-snippets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("facts.sqlite")).unwrap();
        Store(dir, db)
    }

    #[test]
    fn snippets_are_saved_listed_by_name_changed_and_deleted() {
        let Store(_, db) = &store("crud");
        let thanks = Snippet { name: " Thanks ".into(), body_html: "<p>Thank you!</p>".into(), ..Default::default() };
        let id = db.write_blocking(move |tx| save(tx, &thanks, 1)).unwrap();
        let address = Snippet { name: "address".into(), body_html: "<p>1 Main St</p>".into(), ..Default::default() };
        db.write_blocking(move |tx| save(tx, &address, 2)).unwrap();
        let names: Vec<String> = db.read_blocking(list).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["address", "Thanks"], "by name, any case; names trimmed");

        let mut changed = db.read_blocking(move |c| get(c, id)).unwrap().unwrap();
        changed.body_html = "<p>Thanks so much!</p>".into();
        assert_eq!(db.write_blocking(move |tx| save(tx, &changed, 3)).unwrap(), id);
        let now = db.read_blocking(move |c| get(c, id)).unwrap().unwrap();
        assert_eq!((now.body_html.as_str(), now.updated_at), ("<p>Thanks so much!</p>", 3));

        let gone = db.write_blocking(move |tx| delete(tx, id)).unwrap().unwrap();
        assert_eq!(db.read_blocking(list).unwrap().len(), 1);
        db.write_blocking(move |tx| restore(tx, &gone)).unwrap();
        assert_eq!(db.read_blocking(move |c| get(c, id)).unwrap().unwrap().name, "Thanks", "back with its id");
    }

    #[test]
    fn a_snippet_needs_a_name_and_an_unknown_one_cannot_be_changed() {
        let Store(_, db) = &store("invalid");
        let blank = Snippet { name: "  ".into(), body_html: "x".into(), ..Default::default() };
        assert!(matches!(db.write_blocking(move |tx| save(tx, &blank, 1)).unwrap_err(), StoreError::Invalid(_)));
        let unknown = Snippet { id: 99, name: "x".into(), body_html: "x".into(), ..Default::default() };
        assert!(matches!(db.write_blocking(move |tx| save(tx, &unknown, 1)).unwrap_err(), StoreError::NotFound(_)));
        assert_eq!(db.write_blocking(|tx| delete(tx, 99)).unwrap(), None);
    }
}
