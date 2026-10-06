//! The daily review's runs and Analysis settings (spec §14.10): one run
//! per account per calendar day, or on demand, comparing matched AI
//! compositions in batches. Recorded batch by batch, like learning runs,
//! so a run survives quitting.

use mail_domain::Millis;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use crate::error::StoreResult;

/// A pair whose sent text is this close to the AI's was sent as written.
pub const UNCHANGED: f64 = 0.05;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunRow {
    pub id: i64,
    /// The local calendar day, YYYY-MM-DD.
    pub day: String,
    /// `daily` or `manual`.
    pub trigger: String,
    /// `running`, `paused`, `done`, `cancelled`, `failed`.
    pub status: String,
    pub agent: Option<String>,
    pub error: Option<String>,
    pub matched: i64,
    pub unmatched: i64,
    pub unchanged: i64,
    pub batch_size: i64,
    /// Pairs to compare, and compared so far.
    pub total: i64,
    pub done: i64,
    pub timed_batches: i64,
    pub timed_ms: i64,
    pub started_at: Millis,
    pub finished_at: Option<Millis>,
}

const COLUMNS: &str = "id, day, trigger, status, agent, error, matched, unmatched, unchanged, batch_size, total, done, \
                       timed_batches, timed_ms, started_at, finished_at";

fn run(r: &Row<'_>) -> rusqlite::Result<RunRow> {
    Ok(RunRow {
        id: r.get(0)?,
        day: r.get(1)?,
        trigger: r.get(2)?,
        status: r.get(3)?,
        agent: r.get(4)?,
        error: r.get(5)?,
        matched: r.get(6)?,
        unmatched: r.get(7)?,
        unchanged: r.get(8)?,
        batch_size: r.get(9)?,
        total: r.get(10)?,
        done: r.get(11)?,
        timed_batches: r.get(12)?,
        timed_ms: r.get(13)?,
        started_at: r.get(14)?,
        finished_at: r.get(15)?,
    })
}

/// What a new run starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRun<'a> {
    pub day: &'a str,
    pub trigger: &'a str,
    pub agent: Option<&'a str>,
    pub matched: i64,
    pub unmatched: i64,
    pub unchanged: i64,
    /// Compositions to compare, in order.
    pub pairs: &'a [i64],
    pub batch_size: usize,
}

pub fn create_run(tx: &Transaction<'_>, new: &NewRun<'_>, now: Millis) -> StoreResult<i64> {
    let size = new.batch_size.max(1);
    tx.prepare_cached(
        "INSERT INTO analysis_runs (day, trigger, status, agent, matched, unmatched, unchanged, batch_size, total,
           started_at)
         VALUES (?1, ?2, 'running', ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?
    .execute(params![
        new.day,
        new.trigger,
        new.agent,
        new.matched,
        new.unmatched,
        new.unchanged,
        size as i64,
        new.pairs.len() as i64,
        now
    ])?;
    let id = tx.last_insert_rowid();
    let mut insert = tx.prepare_cached(
        "INSERT OR IGNORE INTO analysis_run_pairs (run_id, composition_id, batch) VALUES (?1, ?2, ?3)",
    )?;
    for (i, pair) in new.pairs.iter().enumerate() {
        insert.execute(params![id, pair, (i / size) as i64])?;
    }
    Ok(id)
}

pub fn get_run(conn: &Connection, id: i64) -> StoreResult<Option<RunRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {COLUMNS} FROM analysis_runs WHERE id = ?1"))?
        .query_row([id], run)
        .optional()?)
}

/// The run in progress (running or paused), if any: one at a time.
pub fn active_run(conn: &Connection) -> StoreResult<Option<RunRow>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {COLUMNS} FROM analysis_runs WHERE status IN ('running', 'paused') ORDER BY id DESC LIMIT 1"
        ))?
        .query_row([], run)
        .optional()?)
}

/// The latest run, whatever its state.
pub fn latest_run(conn: &Connection) -> StoreResult<Option<RunRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {COLUMNS} FROM analysis_runs ORDER BY id DESC LIMIT 1"))?
        .query_row([], run)
        .optional()?)
}

/// Whether a daily run was made for `day` (whatever became of it).
pub fn ran_on(conn: &Connection, day: &str) -> StoreResult<bool> {
    Ok(conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM analysis_runs WHERE day = ?1 AND trigger = 'daily')")?
        .query_row([day], |r| r.get(0))?)
}

/// The next batch not yet compared: its number and compositions.
pub fn next_batch(conn: &Connection, run_id: i64) -> StoreResult<Option<(i64, Vec<i64>)>> {
    let batch: Option<i64> = conn
        .prepare_cached("SELECT MIN(batch) FROM analysis_run_pairs WHERE run_id = ?1 AND NOT done")?
        .query_row([run_id], |r| r.get(0))?;
    let Some(batch) = batch else { return Ok(None) };
    let ids = conn
        .prepare_cached(
            "SELECT composition_id FROM analysis_run_pairs WHERE run_id = ?1 AND batch = ?2 ORDER BY composition_id",
        )?
        .query_map(params![run_id, batch], |r| r.get(0))?
        .collect::<Result<Vec<i64>, _>>()?;
    Ok(Some((batch, ids)))
}

/// A batch was compared: its compositions are reviewed, and the run counts
/// it; `elapsed_ms` is how long the agent took, when it was asked.
pub fn finish_batch(
    tx: &Transaction<'_>,
    run_id: i64,
    batch: i64,
    elapsed_ms: Option<i64>,
    now: Millis,
) -> StoreResult<()> {
    if let Some(ms) = elapsed_ms {
        tx.prepare_cached(
            "UPDATE analysis_runs SET timed_batches = timed_batches + 1, timed_ms = timed_ms + ?2 WHERE id = ?1",
        )?
        .execute(params![run_id, ms.max(0)])?;
    }
    tx.prepare_cached(
        "UPDATE ai_compositions SET status = 'reviewed', reviewed_at = ?3, updated_at = ?3
         WHERE id IN (SELECT composition_id FROM analysis_run_pairs WHERE run_id = ?1 AND batch = ?2)",
    )?
    .execute(params![run_id, batch, now])?;
    tx.prepare_cached("UPDATE analysis_run_pairs SET done = 1 WHERE run_id = ?1 AND batch = ?2")?
        .execute(params![run_id, batch])?;
    tx.prepare_cached(
        "UPDATE analysis_runs SET done = (SELECT COUNT(*) FROM analysis_run_pairs WHERE run_id = ?1 AND done)
         WHERE id = ?1",
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
    tx.prepare_cached("UPDATE analysis_runs SET status = ?2, error = ?3, finished_at = ?4 WHERE id = ?1")?
        .execute(params![run_id, status, error, finished])?;
    Ok(())
}

/// Matched pairs the user changed, oldest first: what a run compares. At
/// most `limit` (the cost cap); the rest wait for the next day.
pub fn pairs_to_compare(conn: &Connection, limit: u32) -> StoreResult<Vec<i64>> {
    Ok(conn
        .prepare_cached(
            "SELECT id FROM ai_compositions WHERE status = 'matched' AND distance > ?1
             AND id NOT IN (SELECT composition_id FROM analysis_run_pairs)
             ORDER BY created_at, id LIMIT ?2",
        )?
        .query_map(params![UNCHANGED, limit], |r| r.get(0))?
        .collect::<Result<_, _>>()?)
}

/// Matched pairs sent as written: they need no agent.
pub fn unchanged_pairs(conn: &Connection) -> StoreResult<Vec<i64>> {
    Ok(conn
        .prepare_cached(
            "SELECT id FROM ai_compositions WHERE status = 'matched' AND distance <= ?1 ORDER BY created_at, id",
        )?
        .query_map([UNCHANGED], |r| r.get(0))?
        .collect::<Result<_, _>>()?)
}

/// Mark compositions reviewed without comparing them.
pub fn mark_reviewed(tx: &Transaction<'_>, ids: &[i64], now: Millis) -> StoreResult<()> {
    let mut stmt = tx.prepare_cached(
        "UPDATE ai_compositions SET status = 'reviewed', reviewed_at = ?2, updated_at = ?2 WHERE id = ?1",
    )?;
    for id in ids {
        stmt.execute(params![id, now])?;
    }
    Ok(())
}

pub fn meta(conn: &Connection, key: &str) -> StoreResult<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT value FROM analysis_meta WHERE key = ?1")?
        .query_row([key], |r| r.get(0))
        .optional()?)
}

pub fn set_meta(tx: &Transaction<'_>, key: &str, value: &str) -> StoreResult<()> {
    tx.prepare_cached("INSERT OR REPLACE INTO analysis_meta (key, value) VALUES (?1, ?2)")?.execute([key, value])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;
    use crate::compositions::{self, Kind, NewComposition, Recipients, Source};

    fn db(name: &str) -> (Db, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("openagc-analysis-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (Db::open(&dir.join("mail.sqlite")).unwrap(), dir)
    }

    fn matched(db: &Db, at: Millis, distance: f64) -> i64 {
        db.write_blocking(move |tx| {
            let id = compositions::record(
                tx,
                &NewComposition {
                    source: Source::Agent,
                    agent: None,
                    kind: Kind::New,
                    draft_id: at,
                    thread_id: None,
                    in_reply_to: None,
                    recipients: Recipients::default(),
                    subject: String::new(),
                    instruction: String::new(),
                    ai_text: "text".into(),
                    ai_html: None,
                    guide_version: None,
                    audiences: vec![],
                },
                at,
            )?;
            compositions::set_matched(tx, id, &format!("m{id}"), "thread_next", "text", distance, at)?;
            Ok(id)
        })
        .unwrap()
    }

    #[test]
    fn a_run_compares_the_oldest_changed_pairs_in_batches_up_to_the_cap() {
        let (db, dir) = db("run");
        let a = matched(&db, 3, 0.4);
        let b = matched(&db, 1, 0.2);
        let same = matched(&db, 2, 0.0);
        let c = matched(&db, 4, 0.3);
        assert_eq!(db.read_blocking(|c| pairs_to_compare(c, 2)).unwrap(), vec![b, a], "oldest first, capped");
        assert_eq!(db.read_blocking(unchanged_pairs).unwrap(), vec![same]);

        let pairs = vec![b, a];
        let run = db
            .write_blocking(move |tx| {
                let new = NewRun {
                    day: "2026-10-05",
                    trigger: "daily",
                    agent: Some("claude-code"),
                    matched: 4,
                    unmatched: 0,
                    unchanged: 1,
                    pairs: &pairs,
                    batch_size: 1,
                };
                create_run(tx, &new, 10)
            })
            .unwrap();
        assert!(db.read_blocking(|c| ran_on(c, "2026-10-05")).unwrap());
        assert!(!db.read_blocking(|c| ran_on(c, "2026-10-06")).unwrap());
        assert_eq!(
            db.read_blocking(|c| pairs_to_compare(c, 50)).unwrap(),
            vec![c],
            "taken pairs are not offered again"
        );
        assert_eq!(db.read_blocking(move |c| next_batch(c, run)).unwrap(), Some((0, vec![b])));
        db.write_blocking(move |tx| finish_batch(tx, run, 0, Some(1500), 20)).unwrap();
        assert_eq!(db.read_blocking(move |c| next_batch(c, run)).unwrap(), Some((1, vec![a])));
        let row = db.read_blocking(move |c| get_run(c, run)).unwrap().unwrap();
        assert_eq!((row.total, row.done, row.timed_batches, row.timed_ms), (2, 1, 1, 1500));
        let reviewed = db.read_blocking(move |c| compositions::get(c, b)).unwrap().unwrap();
        assert_eq!((reviewed.status, reviewed.reviewed_at), (compositions::Status::Reviewed, Some(20)));
        assert_eq!(db.read_blocking(active_run).unwrap().map(|r| r.id), Some(run));
        db.write_blocking(move |tx| set_run_status(tx, run, "done", None, 30)).unwrap();
        assert!(db.read_blocking(active_run).unwrap().is_none());
        assert_eq!(db.read_blocking(latest_run).unwrap().unwrap().finished_at, Some(30));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn settings_are_kept() {
        let (db, dir) = db("meta");
        assert_eq!(db.read_blocking(|c| meta(c, "pairs_per_day")).unwrap(), None);
        db.write_blocking(|tx| set_meta(tx, "pairs_per_day", "20")).unwrap();
        assert_eq!(db.read_blocking(|c| meta(c, "pairs_per_day")).unwrap().as_deref(), Some("20"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
