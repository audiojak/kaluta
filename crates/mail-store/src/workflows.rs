//! Workflows (spec §14.13): the sets the user works through, what they
//! decided and how long it took, and the queries that find each set's
//! items in the account's mail and tasks.

use mail_domain::{Millis, system_labels};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::StoreResult;

/// `workflow_meta` key: when the last triage set finished.
pub const TRIAGE_SINCE_KEY: &str = "triage_since";

const DAY_MS: Millis = 86_400_000;

/// A set as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetRow {
    pub id: i64,
    pub kind: String,
    pub day: String,
    pub items: Vec<String>,
    pub started_at: Millis,
    pub finished_at: Option<Millis>,
}

/// One decided item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionRow {
    pub item: String,
    pub decision: String,
    pub decided_at: Millis,
    pub millis: i64,
}

/// What a kind of workflow did on one day, for *Done today*.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DaySummary {
    pub kind: String,
    pub sets_finished: u32,
    pub decisions: u32,
    pub millis: i64,
}

fn set_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SetRow> {
    let items: String = r.get(3)?;
    Ok(SetRow {
        id: r.get(0)?,
        kind: r.get(1)?,
        day: r.get(2)?,
        items: serde_json::from_str(&items).unwrap_or_default(),
        started_at: r.get(4)?,
        finished_at: r.get(5)?,
    })
}

const SET_COLUMNS: &str = "id, kind, day, items_json, started_at, finished_at";

/// The set of `kind` the user started on `day` and has not finished, if any.
pub fn open_set(conn: &Connection, kind: &str, day: &str) -> StoreResult<Option<SetRow>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {SET_COLUMNS} FROM workflow_sets
              WHERE kind = ?1 AND day = ?2 AND finished_at IS NULL ORDER BY id DESC LIMIT 1"
        ))?
        .query_row(params![kind, day], set_from_row)
        .optional()?)
}

pub fn get_set(conn: &Connection, id: i64) -> StoreResult<Option<SetRow>> {
    Ok(conn
        .prepare_cached(&format!("SELECT {SET_COLUMNS} FROM workflow_sets WHERE id = ?1"))?
        .query_row([id], set_from_row)
        .optional()?)
}

pub fn create_set(tx: &Transaction<'_>, kind: &str, day: &str, items: &[String], now: Millis) -> StoreResult<i64> {
    tx.prepare_cached("INSERT INTO workflow_sets (kind, day, items_json, started_at) VALUES (?1, ?2, ?3, ?4)")?
        .execute(params![kind, day, serde_json::to_string(items)?, now])?;
    Ok(tx.last_insert_rowid())
}

pub fn decisions(conn: &Connection, set_id: i64) -> StoreResult<Vec<DecisionRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT item, decision, decided_at, millis FROM workflow_decisions WHERE set_id = ?1 ORDER BY decided_at",
    )?;
    let rows = stmt.query_map([set_id], |r| {
        Ok(DecisionRow { item: r.get(0)?, decision: r.get(1)?, decided_at: r.get(2)?, millis: r.get(3)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Record (or replace) the decision on one item.
pub fn decide(
    tx: &Transaction<'_>,
    set_id: i64,
    item: &str,
    decision: &str,
    now: Millis,
    millis: i64,
) -> StoreResult<()> {
    tx.prepare_cached(
        "INSERT INTO workflow_decisions (set_id, item, decision, decided_at, millis) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (set_id, item) DO UPDATE SET decision = excluded.decision,
           decided_at = excluded.decided_at, millis = excluded.millis",
    )?
    .execute(params![set_id, item, decision, now, millis.max(0)])?;
    Ok(())
}

/// Undo a decision: the item is undecided again, where it was in the set.
pub fn undecide(tx: &Transaction<'_>, set_id: i64, item: &str) -> StoreResult<bool> {
    Ok(tx
        .prepare_cached("DELETE FROM workflow_decisions WHERE set_id = ?1 AND item = ?2")?
        .execute(params![set_id, item])?
        > 0)
}

/// Skip an item: it moves to the end of the set.
pub fn skip(tx: &Transaction<'_>, set_id: i64, item: &str) -> StoreResult<()> {
    let Some(set) = get_set(tx, set_id)? else { return Ok(()) };
    let mut items: Vec<String> = set.items.into_iter().filter(|i| i != item).collect();
    items.push(item.to_owned());
    tx.prepare_cached("UPDATE workflow_sets SET items_json = ?2 WHERE id = ?1")?
        .execute(params![set_id, serde_json::to_string(&items)?])?;
    Ok(())
}

pub fn finish(tx: &Transaction<'_>, set_id: i64, now: Millis) -> StoreResult<()> {
    tx.prepare_cached("UPDATE workflow_sets SET finished_at = ?2 WHERE id = ?1 AND finished_at IS NULL")?
        .execute(params![set_id, now])?;
    Ok(())
}

pub fn meta(conn: &Connection, key: &str) -> StoreResult<Option<String>> {
    Ok(conn
        .prepare_cached("SELECT value FROM workflow_meta WHERE key = ?1")?
        .query_row([key], |r| r.get(0))
        .optional()?)
}

pub fn set_meta(tx: &Transaction<'_>, key: &str, value: &str) -> StoreResult<()> {
    tx.prepare_cached(
        "INSERT INTO workflow_meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )?
    .execute(params![key, value])?;
    Ok(())
}

/// Each kind's sets finished and decisions made on `day`.
pub fn day_summary(conn: &Connection, day: &str) -> StoreResult<Vec<DaySummary>> {
    let mut stmt = conn.prepare_cached(
        "SELECT s.kind,
                (SELECT COUNT(*) FROM workflow_sets f WHERE f.kind = s.kind AND f.day = ?1 AND f.finished_at IS NOT NULL),
                COUNT(d.item), COALESCE(SUM(d.millis), 0)
           FROM workflow_sets s LEFT JOIN workflow_decisions d ON d.set_id = s.id
          WHERE s.day = ?1
          GROUP BY s.kind ORDER BY s.kind",
    )?;
    let rows = stmt.query_map([day], |r| {
        Ok(DaySummary { kind: r.get(0)?, sets_finished: r.get(1)?, decisions: r.get(2)?, millis: r.get(3)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The median time the user takes per decision in `kind` since `since`,
/// when there are at least `min_samples` to go on.
pub fn pace(conn: &Connection, kind: &str, since: Millis, min_samples: usize) -> StoreResult<Option<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT d.millis FROM workflow_decisions d JOIN workflow_sets s ON s.id = d.set_id
          WHERE s.kind = ?1 AND d.decided_at >= ?2 AND d.millis > 0 ORDER BY d.millis",
    )?;
    let times: Vec<i64> = stmt.query_map(params![kind, since], |r| r.get(0))?.collect::<Result<_, _>>()?;
    if times.len() < min_samples.max(1) {
        return Ok(None);
    }
    Ok(Some(times[times.len() / 2]))
}

/// A conversation a workflow may hold: its latest message from someone else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub thread_id: String,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_email: Option<String>,
    pub at: Millis,
    /// From a person (Primary, or no category), not a service.
    pub person: bool,
    /// The latest message's Gmail id.
    pub message_id: String,
}

fn label_rowid(conn: &Connection, gmail_id: &str) -> StoreResult<i64> {
    Ok(conn
        .prepare_cached("SELECT id FROM labels WHERE gmail_id = ?1")?
        .query_row([gmail_id], |r| r.get(0))
        .optional()?
        .unwrap_or(-1))
}

/// Inbox conversations whose latest message is someone else's and arrived
/// at or after `since`; `asking`: only those from people whose latest
/// message asks something and that have no open task (Replies).
fn inbox_candidates(conn: &Connection, since: Millis, asking: bool, limit: u32) -> StoreResult<Vec<Candidate>> {
    let inbox = label_rowid(conn, system_labels::INBOX)?;
    let personal = label_rowid(conn, "CATEGORY_PERSONAL")?;
    let mut stmt = conn.prepare_cached(
        "SELECT t.gmail_id, COALESCE(NULLIF(t.subject, ''), m.subject), m.from_name, m.from_email, m.internal_date,
                (EXISTS (SELECT 1 FROM thread_labels p WHERE p.thread_id = t.id AND p.label_id = ?2)
                 OR NOT EXISTS (SELECT 1 FROM thread_labels c JOIN labels l ON l.id = c.label_id
                                 WHERE c.thread_id = t.id AND l.gmail_id LIKE 'CATEGORY\\_%' ESCAPE '\\')) AS person,
                instr(m.snippet, '?') > 0 OR instr(m.subject, '?') > 0 AS asks,
                m.gmail_id,
                EXISTS (SELECT 1 FROM tasks k WHERE k.thread_id = t.gmail_id AND k.status = 'open') AS tasked
           FROM thread_labels i
           JOIN threads t ON t.id = i.thread_id
           JOIN messages m ON m.id = (SELECT id FROM messages WHERE thread_id = t.id AND is_draft = 0
                                       ORDER BY internal_date DESC, id DESC LIMIT 1)
          WHERE i.label_id = ?1 AND m.is_sent_by_me = 0 AND m.internal_date >= ?3
          ORDER BY m.internal_date DESC
          LIMIT ?4",
    )?;
    let rows = stmt.query_map(params![inbox, personal, since, limit], |r| {
        Ok((
            Candidate {
                thread_id: r.get(0)?,
                subject: r.get(1)?,
                from_name: r.get(2)?,
                from_email: r.get(3)?,
                at: r.get(4)?,
                person: r.get(5)?,
                message_id: r.get(7)?,
            },
            r.get::<_, bool>(6)?,
            r.get::<_, bool>(8)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (c, asks, tasked) = row?;
        if !asking || (c.person && asks && !tasked) {
            out.push(c);
        }
    }
    Ok(out)
}

/// Triage's set (spec §14.13): new conversations since `since`, people
/// first, newest first; then automated mail grouped by sender, the
/// sender with the newest mail first.
pub fn triage_candidates(conn: &Connection, since: Millis, limit: u32) -> StoreResult<Vec<Candidate>> {
    let all = inbox_candidates(conn, since, false, limit)?;
    let (people, automated): (Vec<Candidate>, Vec<Candidate>) = all.into_iter().partition(|c| c.person);
    let mut groups: Vec<(String, Vec<Candidate>)> = Vec::new();
    for c in automated {
        let sender = c.from_email.clone().unwrap_or_default().to_lowercase();
        match groups.iter_mut().find(|(s, _)| *s == sender) {
            Some((_, g)) => g.push(c),
            None => groups.push((sender, vec![c])),
        }
    }
    Ok(people.into_iter().chain(groups.into_iter().flat_map(|(_, g)| g)).collect())
}

/// Replies' set (spec §14.13): people's conversations in the Inbox from
/// the last `days` days whose latest message asks the user something and
/// that have no open task. (Until triage suggestions exist, "asks" is a
/// question mark in the latest message's subject or snippet.)
pub fn replies_candidates(conn: &Connection, now: Millis, days: i64, limit: u32) -> StoreResult<Vec<Candidate>> {
    inbox_candidates(conn, now - days * DAY_MS, true, limit)
}

/// An open task due on or before a day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueTask {
    pub id: i64,
    pub thread_id: String,
    pub title: String,
    pub due_day: String,
}

/// Tasks' set (spec §14.13): open tasks due on or before `today`, those
/// carried over from earlier days first.
pub fn tasks_due(conn: &Connection, today: &str) -> StoreResult<Vec<DueTask>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, thread_id, title, due_day FROM tasks
          WHERE status = 'open' AND due_day IS NOT NULL AND due_day <= ?1
          ORDER BY due_day, created_at, id",
    )?;
    let rows = stmt.query_map([today], |r| {
        Ok(DueTask { id: r.get(0)?, thread_id: r.get(1)?, title: r.get(2)?, due_day: r.get(3)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Open tasks due after `today` up to and including `until`.
pub fn tasks_due_later(conn: &Connection, today: &str, until: &str) -> StoreResult<u32> {
    Ok(conn
        .prepare_cached(
            "SELECT COUNT(*) FROM tasks WHERE status = 'open' AND due_day IS NOT NULL AND due_day > ?1 AND due_day <= ?2",
        )?
        .query_row(params![today, until], |r| r.get(0))?)
}

/// Teach Kaluta's queue (spec §14.13): proposed rules (the learning run's
/// and the daily review's) and proposed facts.
pub fn teach_count(conn: &Connection) -> StoreResult<u32> {
    Ok(conn
        .prepare_cached(
            "SELECT (SELECT COUNT(*) FROM guide_entries WHERE status = 'proposed')
                  + (SELECT COUNT(*) FROM analysis_proposals WHERE status = 'proposed')
                  + (SELECT COUNT(*) FROM facts WHERE status = 'proposed')
                  + (SELECT COUNT(*) FROM routine_proposals WHERE status = 'proposed')",
        )?
        .query_row([], |r| r.get(0))?)
}

/// Teach Kaluta's items, grouped by kind: changes to the sorting routine
/// (`routine:<id>`), proposed rules (`rule:<id>` from the learning run,
/// `proposal:<id>` from the daily review), then proposed facts
/// (`fact:<id>`).
pub fn teach_items(conn: &Connection) -> StoreResult<Vec<String>> {
    let mut out = Vec::new();
    for (prefix, sql) in [
        ("routine", "SELECT id FROM routine_proposals WHERE status = 'proposed' ORDER BY id"),
        ("rule", "SELECT id FROM guide_entries WHERE status = 'proposed' ORDER BY id"),
        ("proposal", "SELECT id FROM analysis_proposals WHERE status = 'proposed' ORDER BY id"),
        ("fact", "SELECT id FROM facts WHERE status = 'proposed' ORDER BY id"),
    ] {
        let mut stmt = conn.prepare_cached(sql)?;
        let ids: Vec<i64> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        out.extend(ids.into_iter().map(|id| format!("{prefix}:{id}")));
    }
    Ok(out)
}

/// Tasks finished outside the set (a reply sent in Triage, the task list)
/// are decided `elsewhere`, with no time spent: they show done and the set
/// moves past them. The number decided.
pub fn decide_done_elsewhere(tx: &Transaction, set_id: i64, now: Millis) -> StoreResult<usize> {
    let Some(set) = get_set(tx, set_id)? else { return Ok(0) };
    let decided: Vec<String> = decisions(tx, set_id)?.into_iter().map(|d| d.item).collect();
    let mut open = tx.prepare_cached("SELECT status = 'open' FROM tasks WHERE id = ?1")?;
    let mut n = 0;
    for key in set.items.iter().filter(|k| !decided.contains(k)) {
        let Some(id) = key.strip_prefix("task:").and_then(|id| id.parse::<i64>().ok()) else { continue };
        let still_open = open.query_row([id], |r| r.get::<_, bool>(0)).optional()?.unwrap_or(false);
        if !still_open {
            decide(tx, set_id, key, "elsewhere", now, 0)?;
            n += 1;
        }
    }
    Ok(n)
}

/// A triage suggestion as stored (spec §14.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionRow {
    pub thread_id: String,
    pub message_id: String,
    /// `reply`, `task`, `archive` or `leave`.
    pub kind: String,
    pub seconds: Option<u32>,
    pub reason: String,
    pub answers_json: String,
    pub task_json: Option<String>,
    pub agent: Option<String>,
    pub created_at: Millis,
}

/// Keep a suggestion, replacing the conversation's last.
pub fn save_suggestion(tx: &Transaction, s: &SuggestionRow) -> StoreResult<()> {
    tx.execute(
        "INSERT OR REPLACE INTO triage_suggestions
           (thread_id, message_id, kind, seconds, reason, answers_json, task_json, agent, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            s.thread_id,
            s.message_id,
            s.kind,
            s.seconds,
            s.reason,
            s.answers_json,
            s.task_json,
            s.agent,
            s.created_at
        ],
    )?;
    Ok(())
}

/// The conversation's suggestion, if it is for the conversation as it is
/// now: its latest message is still the one suggested for.
pub fn suggestion(conn: &Connection, thread_id: &str) -> StoreResult<Option<SuggestionRow>> {
    Ok(conn
        .query_row(
            "SELECT s.thread_id, s.message_id, s.kind, s.seconds, s.reason, s.answers_json, s.task_json, s.agent,
                    s.created_at
               FROM triage_suggestions s
               JOIN threads t ON t.gmail_id = s.thread_id
               JOIN messages m ON m.id = (SELECT id FROM messages WHERE thread_id = t.id AND is_draft = 0
                                           ORDER BY internal_date DESC, id DESC LIMIT 1)
              WHERE s.thread_id = ?1 AND m.gmail_id = s.message_id",
            [thread_id],
            |r| {
                Ok(SuggestionRow {
                    thread_id: r.get(0)?,
                    message_id: r.get(1)?,
                    kind: r.get(2)?,
                    seconds: r.get(3)?,
                    reason: r.get(4)?,
                    answers_json: r.get(5)?,
                    task_json: r.get(6)?,
                    agent: r.get(7)?,
                    created_at: r.get(8)?,
                })
            },
        )
        .optional()?)
}

/// Triage's next conversations to suggest for, in the set's order: new
/// since `since`, with no suggestion for their latest message.
pub fn to_suggest(conn: &Connection, since: Millis, limit: usize) -> StoreResult<Vec<Candidate>> {
    let mut have = conn.prepare_cached("SELECT 1 FROM triage_suggestions WHERE thread_id = ?1 AND message_id = ?2")?;
    let mut out = Vec::new();
    for c in triage_candidates(conn, since, 500)? {
        if out.len() >= limit {
            break;
        }
        if !have.exists(params![c.thread_id, c.message_id])? {
            out.push(c);
        }
    }
    Ok(out)
}

/// What a background job did on one day (spec §14.13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDay {
    pub job: String,
    pub day: String,
    pub items: u32,
    pub last_run_at: Millis,
    pub last_error: Option<String>,
}

/// Items `job` has done on `day`.
pub fn job_items(conn: &Connection, job: &str, day: &str) -> StoreResult<u32> {
    Ok(conn
        .query_row("SELECT items FROM workflow_job_days WHERE job = ?1 AND day = ?2", params![job, day], |r| {
            r.get::<_, u32>(0)
        })
        .optional()?
        .unwrap_or(0))
}

/// Count `items` more for `job` on `day`, with how the attempt ended.
pub fn record_job(
    tx: &Transaction,
    job: &str,
    day: &str,
    items: u32,
    error: Option<&str>,
    now: Millis,
) -> StoreResult<()> {
    tx.execute(
        "INSERT INTO workflow_job_days (job, day, items, last_run_at, last_error) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (job, day) DO UPDATE SET items = items + excluded.items,
           last_run_at = excluded.last_run_at, last_error = excluded.last_error",
        params![job, day, items, now, error],
    )?;
    Ok(())
}

/// Every job's record for `day`.
pub fn job_days(conn: &Connection, day: &str) -> StoreResult<Vec<JobDay>> {
    let mut stmt = conn.prepare(
        "SELECT job, day, items, last_run_at, last_error FROM workflow_job_days WHERE day = ?1 ORDER BY job",
    )?;
    let rows = stmt.query_map(params![day], |r| {
        Ok(JobDay { job: r.get(0)?, day: r.get(1)?, items: r.get(2)?, last_run_at: r.get(3)?, last_error: r.get(4)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    const NOW: Millis = 1_790_000_000_000;

    struct Store(std::path::PathBuf, Db);

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("kaluta-workflows-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("mail.sqlite")).unwrap();
        Store(dir, db)
    }

    fn keys(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_set_resumes_skips_to_the_end_and_finishes() {
        let Store(_, db) = &store("sets");
        let id = db
            .write_blocking(|tx| {
                create_set(tx, "triage", "2026-10-11", &keys(&["thread:a", "thread:b", "thread:c"]), NOW)
            })
            .unwrap();
        assert_eq!(db.read_blocking(move |c| open_set(c, "triage", "2026-10-11")).unwrap().unwrap().id, id);
        assert!(
            db.read_blocking(move |c| open_set(c, "triage", "2026-10-12")).unwrap().is_none(),
            "a new day, a new set"
        );

        db.write_blocking(move |tx| skip(tx, id, "thread:a")).unwrap();
        let set = db.read_blocking(move |c| get_set(c, id)).unwrap().unwrap();
        assert_eq!(set.items, keys(&["thread:b", "thread:c", "thread:a"]));

        db.write_blocking(move |tx| decide(tx, id, "thread:b", "archive", NOW + 1_000, 1_000)).unwrap();
        db.write_blocking(move |tx| decide(tx, id, "thread:b", "task", NOW + 2_000, 2_000)).unwrap();
        let d = db.read_blocking(move |c| decisions(c, id)).unwrap();
        assert_eq!((d.len(), d[0].decision.as_str()), (1, "task"), "deciding again replaces");
        assert!(db.write_blocking(move |tx| undecide(tx, id, "thread:b")).unwrap());
        assert!(db.read_blocking(move |c| decisions(c, id)).unwrap().is_empty());

        db.write_blocking(move |tx| finish(tx, id, NOW + 3_000)).unwrap();
        assert!(db.read_blocking(move |c| open_set(c, "triage", "2026-10-11")).unwrap().is_none());
    }

    #[test]
    fn the_day_summary_and_the_pace_come_from_decisions() {
        let Store(_, db) = &store("pace");
        let id = db
            .write_blocking(move |tx| {
                create_set(tx, "tasks", "2026-10-11", &keys(&["task:1", "task:2", "task:3"]), NOW)
            })
            .unwrap();
        for (i, ms) in [("task:1", 30_000), ("task:2", 90_000), ("task:3", 60_000)] {
            db.write_blocking(move |tx| decide(tx, id, i, "done", NOW, ms)).unwrap();
        }
        db.write_blocking(move |tx| finish(tx, id, NOW)).unwrap();
        let day = db.read_blocking(move |c| day_summary(c, "2026-10-11")).unwrap();
        assert_eq!(day, vec![DaySummary { kind: "tasks".into(), sets_finished: 1, decisions: 3, millis: 180_000 }]);
        assert_eq!(db.read_blocking(move |c| pace(c, "tasks", NOW - DAY_MS, 3)).unwrap(), Some(60_000));
        assert_eq!(db.read_blocking(move |c| pace(c, "tasks", NOW - DAY_MS, 4)).unwrap(), None, "too few to go on");
    }

    #[test]
    fn triage_puts_people_first_and_groups_automated_mail_by_sender() {
        let Store(_, db) = &store("triage");
        crate::demo_workflows::generate(db, NOW).unwrap();
        let set = db.read_blocking(move |c| triage_candidates(c, NOW - DAY_MS, 500)).unwrap();
        assert!(!set.is_empty());
        let first_automated = set.iter().position(|c| !c.person).unwrap();
        assert!(set[..first_automated].iter().all(|c| c.person), "people first");
        assert!(set[first_automated..].iter().all(|c| !c.person));
        // Automated mail from one sender sits together.
        let senders: Vec<String> = set[first_automated..].iter().filter_map(|c| c.from_email.clone()).collect();
        let mut seen: Vec<&String> = Vec::new();
        for (i, s) in senders.iter().enumerate() {
            if seen.contains(&s) {
                assert_eq!(&senders[i - 1], s, "{senders:?}");
            }
            seen.push(s);
        }
        assert!(set.iter().all(|c| c.at >= NOW - DAY_MS));
        assert!(!set.iter().any(|c| c.thread_id == "demowf-ip"), "the user's own last word is not new mail");
    }

    #[test]
    fn replies_hold_peoples_questions_without_a_task() {
        let Store(_, db) = &store("replies");
        crate::demo_workflows::generate(db, NOW).unwrap();
        db.write_blocking(move |tx| {
            tx.execute(
                "INSERT INTO tasks (thread_id, title, category, source, created_at) VALUES ('demowf-renewal', 'x', 'Decide', 'you', 0)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let ids: Vec<String> = db
            .read_blocking(move |c| replies_candidates(c, NOW, 14, 500))
            .unwrap()
            .into_iter()
            .map(|c| c.thread_id)
            .collect();
        assert!(ids.contains(&"demowf-board-prep".to_owned()), "{ids:?}");
        assert!(!ids.contains(&"demowf-renewal".to_owned()), "a task takes it to Tasks");
        assert!(!ids.contains(&"demowf-standup".to_owned()), "asks nothing");
        assert!(!ids.contains(&"demowf-outage".to_owned()), "not a person");
    }

    #[test]
    fn tasks_due_carry_over_first() {
        let Store(_, db) = &store("tasks");
        db.write_blocking(move |tx| {
            for (t, due) in [("a", "2026-10-11"), ("b", "2026-10-09"), ("c", "2026-10-13"), ("d", "2026-10-25")] {
                tx.execute(
                    "INSERT INTO tasks (thread_id, title, category, due_day, source, created_at) VALUES (?1, 'x', 'Admin', ?2, 'you', 0)",
                    params![t, due],
                )?;
            }
            Ok(())
        })
        .unwrap();
        let due: Vec<String> =
            db.read_blocking(move |c| tasks_due(c, "2026-10-11")).unwrap().into_iter().map(|t| t.thread_id).collect();
        assert_eq!(due, keys(&["b", "a"]));
        assert_eq!(db.read_blocking(move |c| tasks_due_later(c, "2026-10-11", "2026-10-17")).unwrap(), 1);
    }

    #[test]
    fn a_jobs_items_add_up_per_day() {
        let s = store("jobs");
        let db = &s.1;
        db.write_blocking(|tx| {
            record_job(tx, "triage", "2026-10-11", 3, None, 1)?;
            record_job(tx, "triage", "2026-10-11", 2, Some("The agent stopped"), 2)?;
            record_job(tx, "triage", "2026-10-12", 1, None, 3)
        })
        .unwrap();
        assert_eq!(db.read_blocking(|c| job_items(c, "triage", "2026-10-11")).unwrap(), 5);
        assert_eq!(db.read_blocking(|c| job_items(c, "waiting", "2026-10-11")).unwrap(), 0);
        let days = db.read_blocking(|c| job_days(c, "2026-10-11")).unwrap();
        assert_eq!(days.len(), 1);
        assert_eq!((days[0].items, days[0].last_run_at), (5, 2));
        assert_eq!(days[0].last_error.as_deref(), Some("The agent stopped"));
    }

    #[test]
    fn a_suggestion_counts_until_the_conversation_changes() {
        let Store(_, db) = &store("suggestions");
        crate::demo_workflows::generate(db, NOW).unwrap();
        let since = NOW - DAY_MS;
        let all = db.read_blocking(move |c| to_suggest(c, since, 500)).unwrap();
        let first = all[0].clone();
        let row = SuggestionRow {
            thread_id: first.thread_id.clone(),
            message_id: first.message_id.clone(),
            kind: "reply".into(),
            seconds: Some(20),
            reason: "They ask for a time.".into(),
            answers_json: "[]".into(),
            task_json: None,
            agent: Some("claude-code".into()),
            created_at: NOW,
        };
        db.write_blocking(move |tx| save_suggestion(tx, &row)).unwrap();
        let left = db.read_blocking(move |c| to_suggest(c, since, 500)).unwrap();
        assert_eq!(left.len(), all.len() - 1);
        assert!(!left.iter().any(|c| c.thread_id == first.thread_id));
        let thread = first.thread_id.clone();
        assert_eq!(db.read_blocking(move |c| suggestion(c, &thread)).unwrap().unwrap().kind, "reply");

        // A newer message: the suggestion no longer counts.
        let thread = first.thread_id.clone();
        db.write_blocking(move |tx| {
            tx.execute(
                "INSERT INTO messages (thread_id, gmail_id, from_email, subject, date, internal_date)
                 SELECT id, 'newer', 'ann@example.com', 'Re', ?2, ?2 FROM threads WHERE gmail_id = ?1",
                params![thread, NOW],
            )?;
            Ok(())
        })
        .unwrap();
        let thread = first.thread_id.clone();
        assert!(db.read_blocking(move |c| suggestion(c, &thread)).unwrap().is_none());
        let again = db.read_blocking(move |c| to_suggest(c, since, 500)).unwrap();
        assert!(again.iter().any(|c| c.thread_id == first.thread_id && c.message_id == "newer"));
    }

    #[test]
    fn tasks_finished_elsewhere_are_decided_with_no_time() {
        let Store(_, db) = &store("elsewhere");
        let (a, b) = db
            .write_blocking(|tx| {
                for t in ["a", "b"] {
                    tx.execute(
                        "INSERT INTO tasks (thread_id, title, category, source, created_at) VALUES (?1, ?1, 'Reply', 'you', 0)",
                        [t],
                    )?;
                }
                Ok((1i64, 2i64))
            })
            .unwrap();
        let items = vec![format!("task:{a}"), format!("task:{b}"), "task:99".into()];
        let set = db.write_blocking(move |tx| create_set(tx, "tasks", "2026-10-12", &items, NOW)).unwrap();
        db.write_blocking(move |tx| {
            tx.execute("UPDATE tasks SET status = 'done' WHERE id = ?1", [b])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(db.write_blocking(move |tx| decide_done_elsewhere(tx, set, NOW)).unwrap(), 2, "done, and deleted");
        let d = db.read_blocking(move |c| decisions(c, set)).unwrap();
        assert!(d.iter().all(|d| d.decision == "elsewhere" && d.millis == 0));
        assert!(!d.iter().any(|d| d.item == format!("task:{a}")), "an open task stays to decide");
        assert_eq!(db.write_blocking(move |tx| decide_done_elsewhere(tx, set, NOW)).unwrap(), 0, "once");
    }
}
