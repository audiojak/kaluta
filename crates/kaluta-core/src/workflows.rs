//! Workflows (spec §14.13): the start screen's overview, and each
//! workflow's set, fixed when the user starts it, with what they decided.
//! A set started today and not finished resumes where it was.

use std::sync::atomic::Ordering;

use mail_store::workflows as store;

use crate::analysis_run::day_of;
use crate::{Core, CoreError, CoreEvent, runtime};

const DAY_MS: i64 = 86_400_000;
/// How far back the user's own pace is read.
const PACE_DAYS: i64 = 14;
/// Decisions needed before the user's pace replaces the defaults.
const PACE_SAMPLES: usize = 5;
/// The most items a set holds; the rest wait for the next one.
const MAX_SET: u32 = 200;
/// A first triage looks back this far.
const FIRST_TRIAGE_MS: i64 = DAY_MS;
/// Replies look back this far.
const REPLIES_DAYS: i64 = 14;

/// The seven workflows, in the start screen's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum WorkflowKind {
    Triage,
    Replies,
    Tasks,
    Waiting,
    Reading,
    Agents,
    Teach,
}

impl WorkflowKind {
    pub const ALL: [WorkflowKind; 7] = [
        WorkflowKind::Triage,
        WorkflowKind::Replies,
        WorkflowKind::Tasks,
        WorkflowKind::Waiting,
        WorkflowKind::Reading,
        WorkflowKind::Agents,
        WorkflowKind::Teach,
    ];

    pub fn key(self) -> &'static str {
        match self {
            WorkflowKind::Triage => "triage",
            WorkflowKind::Replies => "replies",
            WorkflowKind::Tasks => "tasks",
            WorkflowKind::Waiting => "waiting",
            WorkflowKind::Reading => "reading",
            WorkflowKind::Agents => "agents",
            WorkflowKind::Teach => "teach",
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.key() == key)
    }

    /// The time per item before the user has a pace of their own.
    fn default_ms(self) -> i64 {
        match self {
            WorkflowKind::Triage => 30_000,
            WorkflowKind::Replies | WorkflowKind::Tasks => 120_000,
            WorkflowKind::Waiting | WorkflowKind::Reading | WorkflowKind::Teach => 20_000,
            WorkflowKind::Agents => 30_000,
        }
    }
}

/// One workflow on the start screen.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WorkflowSummary {
    pub kind: WorkflowKind,
    /// What is left to decide (an open set's undecided items, else a new set's).
    pub count: u32,
    pub estimate_secs: u32,
    /// The first few items' names: senders (Triage, Replies) or task titles.
    pub preview: Vec<String>,
    /// Waiting beyond today's set: tasks due later this week.
    pub later: u32,
    /// A set of this kind is open today, part decided.
    pub in_progress: bool,
    /// Triage: the people the user writes to among the senders, the most
    /// written to first (up to four), how many more of them, and how many
    /// conversations are from anyone else.
    pub people: Vec<String>,
    pub people_more: u32,
    pub from_others: u32,
    /// Triage: up to three conversations worth a look first, and why.
    pub worth: Vec<WorthLook>,
}

/// Why a conversation is worth a look (spec §14.13).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum WorthReason {
    /// It answers something the user sent.
    AnswersYou,
    /// Kaluta's suggestion is a reply or a task: it asks something.
    AsksYou,
    /// The task Kaluta suggests is due that day (`YYYY-MM-DD`).
    Due {
        day: String,
    },
    /// To the user alone, copying no one.
    ToYouAlone,
    Starred,
}

/// A conversation worth a look first.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WorthLook {
    pub thread_id: String,
    pub name: String,
    pub subject: String,
    pub reasons: Vec<WorthReason>,
}

/// One workflow's day, for *Done today*.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WorkflowDay {
    pub kind: WorkflowKind,
    pub sets_finished: u32,
    pub decisions: u32,
}

/// The start screen (spec §14.13).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WorkflowOverview {
    /// All seven, in order.
    pub workflows: Vec<WorkflowSummary>,
    /// The first with anything in it.
    pub next: Option<WorkflowKind>,
    pub estimate_secs: u32,
    pub done_today: Vec<WorkflowDay>,
}

/// One item of a set: a conversation, a task, or a proposal.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WorkflowItem {
    /// `thread:<id>`, `task:<id>`, `rule:<id>`, `proposal:<id>`, `fact:<id>`.
    pub key: String,
    pub thread_id: Option<String>,
    pub task_id: Option<i64>,
    /// How it was decided, once it was.
    pub decision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WorkflowSet {
    pub id: i64,
    pub kind: WorkflowKind,
    /// In order; skipped items at the end.
    pub items: Vec<WorkflowItem>,
    pub started_at: i64,
    /// Started earlier today and resumed.
    pub resumed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DecisionCount {
    pub decision: String,
    pub count: u32,
}

/// What a finished set did, for its end screen.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WorkflowScore {
    pub kind: WorkflowKind,
    pub decided: u32,
    pub items: u32,
    pub millis: i64,
    pub counts: Vec<DecisionCount>,
}

fn item_from_key(key: &str, decision: Option<String>) -> WorkflowItem {
    let (kind, id) = key.split_once(':').unwrap_or((key, ""));
    WorkflowItem {
        key: key.to_owned(),
        thread_id: (kind == "thread").then(|| id.to_owned()),
        task_id: (kind == "task").then(|| id.parse().ok()).flatten(),
        decision,
    }
}

/// The local day `days` after `day` (`YYYY-MM-DD`).
fn day_after(day: &str, days: i64) -> String {
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .map(|d| (d + chrono::Duration::days(days)).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| day.to_owned())
}

/// A Triage conversation with what makes it matter.
struct Ranked {
    candidate: store::Candidate,
    /// Sent messages to its sender.
    written_to: u32,
    score: u32,
    reasons: Vec<WorthReason>,
}

/// Up to this many on the start screen's card.
const WORTH: usize = 3;
/// A score at least this high is worth a look: more than knowing the sender.
const WORTH_SCORE: u32 = 4;

/// How much each conversation likely matters (spec §14.13): it answers the
/// user (5), asks something (3), is starred (3), is from someone the user
/// writes to (2, or 3 for five messages or more), has a due day (2), is to
/// the user alone (1), or Gmail marked it important (1).
fn rank(conn: &mail_store::Connection, found: Vec<store::Candidate>) -> mail_store::StoreResult<Vec<Ranked>> {
    let known = mail_store::priority::written_to(conn)?;
    let mut out = Vec::with_capacity(found.len());
    for c in found {
        let written_to = c.from_email.as_ref().and_then(|e| known.get(&e.to_lowercase()).copied()).unwrap_or(0);
        let s = mail_store::priority::signals(conn, &c.thread_id, &c.message_id)?;
        let suggestion = store::suggestion(conn, &c.thread_id)?;
        let asks = suggestion.as_ref().is_some_and(|s| s.kind == "reply" || s.kind == "task");
        let due = suggestion
            .and_then(|s| s.task_json)
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
            // As the triage job keeps a task (`StoredTask`).
            .and_then(|v| v.get("due").and_then(|d| d.as_str()).map(str::to_owned));
        let mut score = 0;
        let mut reasons = Vec::new();
        if s.answers_you {
            score += 5;
            reasons.push(WorthReason::AnswersYou);
        }
        if asks {
            score += 3;
            reasons.push(WorthReason::AsksYou);
        }
        if let Some(day) = due {
            score += 2;
            reasons.push(WorthReason::Due { day });
        }
        if s.starred {
            score += 3;
            reasons.push(WorthReason::Starred);
        }
        if written_to > 0 {
            score += if written_to >= 5 { 3 } else { 2 };
        }
        if s.to_you_alone && c.person {
            score += 1;
            reasons.push(WorthReason::ToYouAlone);
        }
        if s.important {
            score += 1;
        }
        out.push(Ranked { candidate: c, written_to, score, reasons });
    }
    Ok(out)
}

/// The start screen's Triage card: who the user knows among the senders,
/// and what is worth a look first.
fn triage_card(ranked: &[Ranked]) -> (Vec<String>, u32, u32, Vec<WorthLook>) {
    let mut people: Vec<(&str, u32, String)> = Vec::new();
    for r in ranked.iter().filter(|r| r.written_to > 0) {
        let email = r.candidate.from_email.as_deref().unwrap_or_default();
        if !people.iter().any(|(e, ..)| e.eq_ignore_ascii_case(email)) {
            people.push((email, r.written_to, name_of(&r.candidate)));
        }
    }
    people.sort_by_key(|p| std::cmp::Reverse(p.1));
    let from_others = ranked.iter().filter(|r| r.written_to == 0).count() as u32;
    let more = people.len().saturating_sub(4) as u32;
    let names = people.into_iter().take(4).map(|(_, _, n)| n).collect();
    let mut worth: Vec<&Ranked> = ranked.iter().filter(|r| r.score >= WORTH_SCORE).collect();
    // The highest first; equals in Triage's order (people, the newest
    // first), as Triage by priority takes them.
    worth.sort_by_key(|r| std::cmp::Reverse(r.score));
    let worth = worth
        .into_iter()
        .take(WORTH)
        .map(|r| WorthLook {
            thread_id: r.candidate.thread_id.clone(),
            name: name_of(&r.candidate),
            subject: r.candidate.subject.clone(),
            reasons: r.reasons.clone(),
        })
        .collect();
    (names, more, from_others, worth)
}

fn name_of(c: &store::Candidate) -> String {
    c.from_name.clone().filter(|n| !n.trim().is_empty()).or_else(|| c.from_email.clone()).unwrap_or_default()
}

/// A workflow's items as they would be if a set started now, and
/// anything waiting beyond it.
struct Fresh {
    keys: Vec<String>,
    preview: Vec<String>,
    later: u32,
    /// Triage's card (people, how many more, from others, worth a look).
    card: Option<(Vec<String>, u32, u32, Vec<WorthLook>)>,
}

impl Core {
    fn estimate_ms(conn: &mail_store::Connection, kind: WorkflowKind, now: i64) -> mail_store::StoreResult<i64> {
        Ok(store::pace(conn, kind.key(), now - PACE_DAYS * DAY_MS, PACE_SAMPLES)?.unwrap_or_else(|| kind.default_ms()))
    }

    fn set_info(
        conn: &mail_store::Connection,
        set: store::SetRow,
        resumed: bool,
    ) -> mail_store::StoreResult<WorkflowSet> {
        let decided = store::decisions(conn, set.id)?;
        let kind = WorkflowKind::from_key(&set.kind)
            .ok_or_else(|| mail_store::StoreError::Invalid(format!("unknown workflow {}", set.kind)))?;
        let items = set
            .items
            .iter()
            .map(|k| item_from_key(k, decided.iter().find(|d| d.item == *k).map(|d| d.decision.clone())))
            .collect();
        Ok(WorkflowSet { id: set.id, kind, items, started_at: set.started_at, resumed })
    }

    fn workflows_changed(&self) {
        self.account_events().emit(CoreEvent::WorkflowsChanged);
    }
}

#[uniffi::export]
impl Core {
    /// Triage sets start with what matters most, or in arrival order (the
    /// default): the app's setting.
    pub fn set_triage_by_priority(&self, on: bool) {
        self.agents.triage_by_priority.store(on, Ordering::Relaxed);
    }

    /// The start screen: each workflow's count, estimate and first names,
    /// which comes next, and what was done today; for Triage, the people
    /// the user knows and what is worth a look.
    pub async fn workflow_overview(&self) -> Result<WorkflowOverview, CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let core_free = self.workflow_fresh().await;
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let today = day_of(now);
                    let mut workflows = Vec::new();
                    for kind in WorkflowKind::ALL {
                        let fresh = core_free.fresh(c, kind, now)?;
                        let open = store::open_set(c, kind.key(), &today)?;
                        let (count, in_progress) = match &open {
                            Some(set) => {
                                let decided = store::decisions(c, set.id)?.len();
                                ((set.items.len().saturating_sub(decided)) as u32, decided > 0)
                            }
                            None => (fresh.keys.len() as u32, false),
                        };
                        let per = Core::estimate_ms(c, kind, now)?;
                        let (people, people_more, from_others, worth) = fresh.card.unwrap_or_default();
                        workflows.push(WorkflowSummary {
                            kind,
                            count,
                            estimate_secs: ((i64::from(count) * per) / 1000) as u32,
                            preview: fresh.preview,
                            later: fresh.later,
                            in_progress,
                            people,
                            people_more,
                            from_others,
                            worth,
                        });
                    }
                    let next = workflows.iter().find(|w| w.count > 0).map(|w| w.kind);
                    let estimate_secs = workflows.iter().map(|w| w.estimate_secs).sum();
                    let done_today = store::day_summary(c, &today)?
                        .into_iter()
                        .filter_map(|d| {
                            Some(WorkflowDay {
                                kind: WorkflowKind::from_key(&d.kind)?,
                                sets_finished: d.sets_finished,
                                decisions: d.decisions,
                            })
                        })
                        .collect();
                    Ok(WorkflowOverview { workflows, next, estimate_secs, done_today })
                })
                .await?)
        })
        .await
    }

    /// Start a workflow: today's open set of this kind if there is one
    /// (resumed), else a new set fixed from what is waiting now.
    pub async fn start_workflow(&self, kind: WorkflowKind) -> Result<WorkflowSet, CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let fresh_from = self.workflow_fresh().await;
        let set = runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    let today = day_of(now);
                    if let Some(open) = store::open_set(tx, kind.key(), &today)? {
                        // A task finished meanwhile shows done (spec §14.13).
                        // Someone answered meanwhile: off the list, and said so.
                        if kind == WorkflowKind::Waiting {
                            let decided: Vec<String> =
                                store::decisions(tx, open.id)?.into_iter().map(|d| d.item).collect();
                            let threads: Vec<String> = open
                                .items
                                .iter()
                                .filter(|k| !decided.contains(k))
                                .filter_map(|k| k.strip_prefix("thread:").map(str::to_owned))
                                .collect();
                            for t in mail_store::waiting::no_longer_waiting(tx, &threads)? {
                                store::decide(tx, open.id, &format!("thread:{t}"), "answered", now, 0)?;
                            }
                            let id = open.id;
                            let open = store::get_set(tx, id)?
                                .ok_or_else(|| mail_store::StoreError::NotFound(format!("workflow set {id}")))?;
                            return Core::set_info(tx, open, true);
                        }
                        // Answered or archived meanwhile, in Gmail or here: done
                        // elsewhere, so the set never shows it as new.
                        let elsewhere = match kind {
                            WorkflowKind::Tasks => store::decide_done_elsewhere(tx, open.id, now)?,
                            WorkflowKind::Triage | WorkflowKind::Replies => {
                                store::decide_handled_elsewhere(tx, open.id, now)?
                            }
                            WorkflowKind::Teach => store::decide_taught_elsewhere(tx, open.id, now)?,
                            _ => 0,
                        };
                        if elsewhere > 0 {
                            let id = open.id;
                            let open = store::get_set(tx, id)?
                                .ok_or_else(|| mail_store::StoreError::NotFound(format!("workflow set {id}")))?;
                            return Core::set_info(tx, open, true);
                        }
                        return Core::set_info(tx, open, true);
                    }
                    let fresh = fresh_from.fresh(tx, kind, now)?;
                    let id = store::create_set(tx, kind.key(), &today, &fresh.keys, now)?;
                    let set = store::get_set(tx, id)?
                        .ok_or_else(|| mail_store::StoreError::NotFound(format!("workflow set {id}")))?;
                    Core::set_info(tx, set, false)
                })
                .await?)
        })
        .await?;
        self.workflows_changed();
        Ok(set)
    }

    /// Record how an item was decided and how long it took.
    pub async fn decide_workflow_item(
        &self,
        set_id: i64,
        key: String,
        decision: String,
        millis: i64,
    ) -> Result<(), CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(
            async move { Ok(db.write(move |tx| store::decide(tx, set_id, &key, &decision, now, millis)).await?) },
        )
        .await?;
        self.workflows_changed();
        Ok(())
    }

    /// Undo a decision: the item is undecided again.
    pub async fn undo_workflow_decision(&self, set_id: i64, key: String) -> Result<bool, CoreError> {
        let db = self.db()?;
        let undone =
            runtime::run(async move { Ok(db.write(move |tx| store::undecide(tx, set_id, &key)).await?) }).await?;
        self.workflows_changed();
        Ok(undone)
    }

    /// Skip an item: it moves to the end of the set.
    pub async fn skip_workflow_item(&self, set_id: i64, key: String) -> Result<WorkflowSet, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    store::skip(tx, set_id, &key)?;
                    let set = store::get_set(tx, set_id)?
                        .ok_or_else(|| mail_store::StoreError::NotFound(format!("workflow set {set_id}")))?;
                    Core::set_info(tx, set, true)
                })
                .await?)
        })
        .await
    }

    /// Finish a set and score it. A triage set moves "new" on to when it
    /// started: mail that arrived meanwhile is the next set's.
    pub async fn finish_workflow(&self, set_id: i64) -> Result<WorkflowScore, CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let score = runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    let set = store::get_set(tx, set_id)?
                        .ok_or_else(|| mail_store::StoreError::NotFound(format!("workflow set {set_id}")))?;
                    store::finish(tx, set_id, now)?;
                    if set.kind == WorkflowKind::Triage.key() {
                        store::set_meta(tx, store::TRIAGE_SINCE_KEY, &set.started_at.to_string())?;
                    }
                    let decided = store::decisions(tx, set_id)?;
                    let mut counts: Vec<DecisionCount> = Vec::new();
                    for d in &decided {
                        match counts.iter_mut().find(|c| c.decision == d.decision) {
                            Some(c) => c.count += 1,
                            None => counts.push(DecisionCount { decision: d.decision.clone(), count: 1 }),
                        }
                    }
                    let kind = WorkflowKind::from_key(&set.kind).unwrap_or(WorkflowKind::Triage);
                    Ok(WorkflowScore {
                        kind,
                        decided: decided.len() as u32,
                        items: set.items.len() as u32,
                        millis: decided.iter().map(|d| d.millis).sum(),
                        counts,
                    })
                })
                .await?)
        })
        .await?;
        self.workflows_changed();
        Ok(score)
    }
}

/// What a set query needs from the core, taken before moving onto the
/// store's thread (the store's closures may not borrow the core).
#[derive(Clone)]
struct WorkflowFresh {
    /// The Agents set (spec §14.13): `action:<ticket>` for each proposal
    /// waiting, `code:<account>/<message>` for each recent code in an agent
    /// mailbox, `run:<routine>/<run>` for this morning's sorting run.
    agent_keys: Vec<String>,
    /// Triage sets start with what matters most (an option, spec §14.13).
    triage_by_priority: bool,
}

impl WorkflowFresh {
    fn fresh(&self, conn: &mail_store::Connection, kind: WorkflowKind, now: i64) -> mail_store::StoreResult<Fresh> {
        if kind == WorkflowKind::Agents {
            return Ok(Fresh { keys: self.agent_keys.clone(), preview: vec![], later: 0, card: None });
        }
        fresh_set(conn, kind, now, self.triage_by_priority)
    }
}

impl Core {
    async fn workflow_fresh(&self) -> WorkflowFresh {
        let mut keys: Vec<String> =
            self.agents.approvals.pending_tickets().into_iter().map(|t| format!("action:{t}")).collect();
        if let Ok(work) = self.agents_work().await {
            keys.extend(work.codes.iter().map(|c| format!("code:{}/{}", c.account_id, c.message_id)));
        }
        if let Some((routine, run)) = self.morning_run().await {
            keys.push(format!("run:{routine}/{run}"));
        }
        WorkflowFresh { agent_keys: keys, triage_by_priority: self.agents.triage_by_priority.load(Ordering::Relaxed) }
    }

    /// Today's latest successful run of a sorting routine, not undone.
    async fn morning_run(&self) -> Option<(String, i64)> {
        let start = crate::waiting::start_of_day(mail_sync::now_millis());
        for routine in self.list_routines().await.ok()? {
            let runs = self.list_routine_runs(routine.id.clone(), 5).await.ok()?;
            if let Some(run) = runs.iter().find(|r| r.started_at >= start && r.status == "succeeded") {
                return Some((routine.id, run.run_id));
            }
        }
        None
    }
}

/// Where "new" starts for Triage: when the last finished set started, else
/// a day back.
pub(crate) fn triage_since(conn: &mail_store::Connection, now: i64) -> mail_store::StoreResult<i64> {
    Ok(store::meta(conn, store::TRIAGE_SINCE_KEY)?.and_then(|v| v.parse().ok()).unwrap_or(now - FIRST_TRIAGE_MS))
}

/// A set's items from the store (every kind but Agents).
fn fresh_set(
    conn: &mail_store::Connection,
    kind: WorkflowKind,
    now: i64,
    by_priority: bool,
) -> mail_store::StoreResult<Fresh> {
    let today = day_of(now);
    Ok(match kind {
        WorkflowKind::Triage => {
            let since = triage_since(conn, now)?;
            let found = store::triage_candidates(conn, since, MAX_SET)?;
            let mut ranked = rank(conn, found)?;
            let card = triage_card(&ranked);
            if by_priority {
                // Stable: equals keep people first, newest first.
                ranked.sort_by_key(|r| std::cmp::Reverse(r.score));
            }
            Fresh {
                preview: ranked.iter().take(5).map(|r| name_of(&r.candidate)).collect(),
                keys: ranked.iter().map(|r| format!("thread:{}", r.candidate.thread_id)).collect(),
                later: 0,
                card: Some(card),
            }
        }
        WorkflowKind::Replies => {
            let found = store::replies_candidates(conn, now, REPLIES_DAYS, MAX_SET)?;
            Fresh {
                preview: found.iter().take(5).map(name_of).collect(),
                keys: found.iter().map(|c| format!("thread:{}", c.thread_id)).collect(),
                later: 0,
                card: None,
            }
        }
        WorkflowKind::Tasks => {
            let due = store::tasks_due(conn, &today)?;
            Fresh {
                preview: due.iter().take(5).map(|t| t.title.clone()).collect(),
                keys: due.iter().map(|t| format!("task:{}", t.id)).collect(),
                later: store::tasks_due_later(conn, &today, &day_after(&today, 6))?,
                card: None,
            }
        }
        WorkflowKind::Teach => Fresh { keys: store::teach_items(conn)?, preview: vec![], later: 0, card: None },
        // Waiting a few days, the longest first (spec §14.13).
        WorkflowKind::Waiting => {
            let due = mail_store::waiting::due(conn, now, mail_store::waiting::days(conn)?)?;
            let mut preview = Vec::new();
            for w in due.iter().take(5) {
                preview.extend(mail_store::waiting::who(conn, &w.message_id)?);
            }
            Fresh {
                keys: due.iter().map(|w| format!("thread:{}", w.thread_id)).collect(),
                preview,
                later: 0,
                card: None,
            }
        }
        // The buckets due today, their unread mail (spec §14.13).
        WorkflowKind::Reading => {
            let due = crate::reading::due_threads(conn, now)?;
            Fresh {
                preview: due.iter().take(5).map(|f| f.subject.clone()).collect(),
                keys: due.iter().map(|f| format!("thread:{}", f.thread_id)).collect(),
                later: 0,
                card: None,
            }
        }
        // Agents is the caller's.
        WorkflowKind::Agents => Fresh { keys: vec![], preview: vec![], later: 0, card: None },
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::executor::block_on;

    use super::*;
    use crate::CoreConfig;
    use crate::tasks::{NewTask, TaskAction};

    struct Silent;
    impl crate::EventListener for Silent {
        fn on_event(&self, _: Option<String>, _: CoreEvent) {}
    }

    fn demo_core(name: &str) -> (Arc<Core>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("kaluta-core-workflows-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let core = Core::new(
            CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
            Arc::new(crate::secrets::MemorySecrets::default()),
            Arc::new(Silent),
        )
        .unwrap();
        block_on(core.clone().open_account("demo".into())).unwrap();
        block_on(core.debug_seed_workflow_demo(50)).unwrap();
        (core, dir)
    }

    fn summary(o: &WorkflowOverview, kind: WorkflowKind) -> &WorkflowSummary {
        o.workflows.iter().find(|w| w.kind == kind).unwrap()
    }

    #[test]
    fn the_overview_starts_with_triage_and_counts_each_workflow() {
        let (core, dir) = demo_core("overview");
        let today = day_of(mail_sync::now_millis());
        block_on(core.create_tasks(vec![NewTask {
            thread_id: "demowf-designer".into(),
            message_id: None,
            title: "Decide on the designer offer".into(),
            notes: String::new(),
            category: "Decide".into(),
            due_day: Some(today),
            action: TaskAction::Reply,
            why: String::new(),
            from_ai: true,
        }]))
        .unwrap();
        let o = block_on(core.workflow_overview()).unwrap();
        assert_eq!(o.workflows.len(), 7);
        assert_eq!(o.next, Some(WorkflowKind::Triage));
        let triage = summary(&o, WorkflowKind::Triage);
        assert!(triage.count >= 10, "{triage:?}");
        assert_eq!(triage.estimate_secs, triage.count * 30, "30 s each before the user has a pace");
        assert_eq!(triage.preview.first().map(String::as_str), Some("Sam Rivera"), "the newest person first");
        assert_eq!(summary(&o, WorkflowKind::Tasks).count, 1);
        assert!(summary(&o, WorkflowKind::Replies).count >= 3);
        assert_eq!(summary(&o, WorkflowKind::Waiting).count, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn triage_names_the_people_the_user_knows_and_can_start_with_what_matters() {
        let (core, dir) = demo_core("priority");
        let o = block_on(core.workflow_overview()).unwrap();
        let triage = summary(&o, WorkflowKind::Triage);
        assert!(!triage.people.is_empty(), "the demo's user writes to people in the Inbox");
        assert!(triage.people.len() <= 4);
        assert!(triage.from_others > 0, "automated mail is from no one the user writes to");
        assert!(!triage.worth.is_empty() && triage.worth.len() <= WORTH);
        assert!(triage.worth.iter().all(|w| !w.reasons.is_empty() || !w.name.is_empty()));
        // Off by default: arrival order, people first.
        let arrival = block_on(core.start_workflow(WorkflowKind::Triage)).unwrap();
        block_on(core.finish_workflow(arrival.id)).ok();
        let _ = std::fs::remove_dir_all(&dir);

        let (core, dir) = demo_core("priority-on");
        core.set_triage_by_priority(true);
        let o = block_on(core.workflow_overview()).unwrap();
        let worth = summary(&o, WorkflowKind::Triage).worth.clone();
        let set = block_on(core.start_workflow(WorkflowKind::Triage)).unwrap();
        assert_eq!(set.items[0].thread_id.as_deref(), Some(worth[0].thread_id.as_str()), "what matters most first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_set_is_fixed_resumes_and_finishing_triage_moves_new_on() {
        let (core, dir) = demo_core("cycle");
        let set = block_on(core.start_workflow(WorkflowKind::Triage)).unwrap();
        assert!(!set.resumed && set.items.len() >= 10);
        let first = set.items[0].key.clone();
        block_on(core.decide_workflow_item(set.id, first.clone(), "archive".into(), 4_000)).unwrap();

        let again = block_on(core.start_workflow(WorkflowKind::Triage)).unwrap();
        assert!(again.resumed && again.id == set.id);
        assert_eq!(again.items[0].decision.as_deref(), Some("archive"));
        let o = block_on(core.workflow_overview()).unwrap();
        let triage = summary(&o, WorkflowKind::Triage);
        assert_eq!(triage.count as usize, set.items.len() - 1, "what is left of the open set");
        assert!(triage.in_progress);

        let second = again.items[1].key.clone();
        let skipped = block_on(core.skip_workflow_item(set.id, second.clone())).unwrap();
        assert_eq!(skipped.items.last().unwrap().key, second);

        assert!(block_on(core.undo_workflow_decision(set.id, first.clone())).unwrap());
        block_on(core.decide_workflow_item(set.id, first, "task".into(), 6_000)).unwrap();
        let score = block_on(core.finish_workflow(set.id)).unwrap();
        assert_eq!((score.decided, score.millis), (1, 6_000));
        assert_eq!(score.counts, vec![DecisionCount { decision: "task".into(), count: 1 }]);

        let o = block_on(core.workflow_overview()).unwrap();
        assert_eq!(summary(&o, WorkflowKind::Triage).count, 0, "nothing new since the set started");
        let day = o.done_today.iter().find(|d| d.kind == WorkflowKind::Triage).unwrap();
        assert_eq!((day.sets_finished, day.decisions), (1, 1));
        let fresh = block_on(core.start_workflow(WorkflowKind::Triage)).unwrap();
        assert!(fresh.id != set.id && fresh.items.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
