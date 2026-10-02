//! The learning run (spec §14.9): a background job per account that takes
//! the sample batch by batch from the store, asks the user's agent in a
//! hidden, read-only session (ADR 0007), and merges what it finds. It
//! survives the window closing; quitting leaves it `running` in the store
//! and the next launch resumes it. Proposals wait until the run finishes.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use agent_api::{AgentStatus, ProviderId};
use mail_store::guide::{self as store, RunRow};

use crate::guide::{CATEGORIES, GuideEntry, GuideStatus, category};
use crate::guide_learn::{BATCH_SIZE, GuideSampleFilter, keep};
use crate::{Core, CoreError, CoreEvent, ErrorKind, runtime};

/// How long one batch may take the agent.
const TURN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Which messages a run looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GuideRunKind {
    /// The latest sent mail not analysed before (the first run).
    Latest,
    /// Mail sent since the newest analysed message.
    Newer,
    /// Mail older than the oldest analysed message.
    Older,
    /// Messages most likely to show one category or audience (`focus`).
    Improve,
    /// A fresh sample, read against the accepted guide for changes only.
    Recheck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GuideRunStatus {
    Running,
    Paused,
    Done,
    Cancelled,
    Failed,
}

impl GuideRunKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Latest => "latest",
            Self::Newer => "newer",
            Self::Older => "older",
            Self::Improve => "improve",
            Self::Recheck => "recheck",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "newer" => Self::Newer,
            "older" => Self::Older,
            "improve" => Self::Improve,
            "recheck" => Self::Recheck,
            _ => Self::Latest,
        }
    }
}

impl GuideRunStatus {
    fn parse(s: &str) -> Self {
        match s {
            "paused" => Self::Paused,
            "done" => Self::Done,
            "cancelled" => Self::Cancelled,
            "failed" => Self::Failed,
            _ => Self::Running,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideRunRequest {
    pub kind: GuideRunKind,
    /// Messages to analyse at most.
    pub count: u32,
    pub filter: GuideSampleFilter,
    /// A category id or audience group name, for `Improve`.
    pub focus: Option<String>,
    /// The agent to ask: `claude-code` or `codex`.
    pub agent: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideRunInfo {
    pub id: i64,
    pub kind: GuideRunKind,
    pub focus: Option<String>,
    pub status: GuideRunStatus,
    /// Messages in the run, and analysed so far.
    pub total: u32,
    pub done: u32,
    pub batches: u32,
    pub batches_done: u32,
    pub agent: Option<String>,
    /// Why it paused or failed.
    pub error: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// About how many seconds are left, from the batches timed so far; nil
    /// before the first batch is done or once the run has ended.
    pub seconds_left: Option<u32>,
}

/// The two progress bars (spec §14.9).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideProgress {
    /// The run in progress, else the latest.
    pub run: Option<GuideRunInfo>,
    /// Proposals from finished runs, and how many are decided.
    pub decisions_total: u32,
    pub decisions_done: u32,
}

/// The time left: the mean time of the batches done, for each one left.
fn seconds_left(r: &RunRow, batches: u32, batches_done: u32) -> Option<u32> {
    if r.timed_batches <= 0 || !matches!(r.status.as_str(), "running" | "paused") {
        return None;
    }
    let mean_ms = r.timed_ms / r.timed_batches;
    let left = i64::from(batches.saturating_sub(batches_done));
    Some(((mean_ms * left) / 1000) as u32)
}

fn info(r: RunRow) -> GuideRunInfo {
    let size = r.batch_size.max(1) as u32;
    let batches = (r.total.max(0) as u32).div_ceil(size);
    let batches_done = (r.done.max(0) as u32).div_ceil(size);
    let left = seconds_left(&r, batches, batches_done);
    GuideRunInfo {
        id: r.id,
        kind: GuideRunKind::parse(&r.kind),
        focus: r.focus,
        status: GuideRunStatus::parse(&r.status),
        total: r.total.max(0) as u32,
        done: r.done.max(0) as u32,
        batches,
        batches_done,
        agent: r.agent,
        error: r.error,
        started_at: r.started_at,
        finished_at: r.finished_at,
        seconds_left: left,
    }
}

fn provider(id: &str) -> Result<ProviderId, CoreError> {
    match id {
        "claude-code" => Ok(ProviderId::ClaudeCode),
        "codex" => Ok(ProviderId::Codex),
        other => Err(CoreError::new(ErrorKind::InvalidInput, format!("unknown agent {other:?}"))),
    }
}

/// Why an agent cannot do the work, in the words the app shows (spec
/// §14.9: all processing goes through the user's connected agent).
pub(crate) fn not_ready(name: &str, status: &AgentStatus) -> Option<String> {
    let why = match status {
        AgentStatus::Ready { .. } => return None,
        AgentStatus::NotInstalled => format!("{name} is not installed"),
        AgentStatus::NotAuthenticated { .. } => format!("{name} is not signed in"),
        AgentStatus::UpdateRequired { .. } => format!("{name} needs updating"),
        AgentStatus::Error { message } => format!("{name} could not be checked ({message})"),
    };
    Some(format!("{why}. Connect Claude Code or Codex in Settings › Agents to learn from your mail."))
}

/// Messages most likely to show `focus`, first; then the rest.
fn rank_for_focus(
    rows: Vec<store::SentRow>,
    focus: &str,
    groups: &[crate::guide::AudienceGroup],
) -> Vec<store::SentRow> {
    let lower = |s: &str| s.trim().to_lowercase();
    let matches = |r: &store::SentRow| -> bool {
        let subject = lower(&r.subject);
        match category(focus).map(|c| c.id) {
            Some("E1") => subject.starts_with("re:") || r.in_reply_to.is_some(),
            Some("E2") => subject.starts_with("fwd:") || subject.starts_with("fw:"),
            Some("E3") => subject.contains("intro"),
            Some("E4") => ["meet", "call", "time", "schedul", "calendar"].iter().any(|w| subject.contains(w)),
            Some("E5") => ["follow", "checking in", "nudge", "reminder"].iter().any(|w| subject.contains(w)),
            Some("E8") => ["update", "status", "handoff", "hand-off"].iter().any(|w| subject.contains(w)),
            Some(_) => false,
            None => groups.iter().filter(|g| g.name.eq_ignore_ascii_case(focus)).any(|g| {
                r.to.iter().chain(&r.cc).any(|(_, e)| {
                    let e = lower(e);
                    g.members.iter().any(|m| {
                        let m = lower(m);
                        m.strip_prefix('@').map_or(e == m, |d| e.ends_with(&format!("@{d}")))
                    })
                })
            }),
        }
    };
    let (first, rest): (Vec<_>, Vec<_>) = rows.into_iter().partition(|r| matches(r));
    first.into_iter().chain(rest).collect()
}

impl Core {
    /// Choose the run's messages, newest first.
    async fn run_sample(&self, req: &GuideRunRequest) -> Result<Vec<String>, CoreError> {
        let groups = self.list_audience_groups().await?;
        let db = self.db()?;
        let req = req.clone();
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let analysed = store::analysed_messages(c)?;
                    let total = store::sent_count(c)?;
                    let skip = if req.kind == GuideRunKind::Recheck { BTreeSet::new() } else { analysed.clone() };
                    let mut rows: Vec<store::SentRow> =
                        store::sent_messages(c, total, &skip)?.into_iter().filter(|r| keep(r, &req.filter)).collect();
                    let range = store::analysed_range(c)?;
                    match req.kind {
                        GuideRunKind::Newer => {
                            let newest = range.map(|(_, max)| max).unwrap_or(i64::MIN);
                            rows.retain(|r| r.date > newest);
                        }
                        GuideRunKind::Older => {
                            let oldest = range.map(|(min, _)| min).unwrap_or(i64::MAX);
                            rows.retain(|r| r.date < oldest);
                        }
                        GuideRunKind::Improve => {
                            let focus = req.focus.clone().unwrap_or_default();
                            rows = rank_for_focus(rows, &focus, &groups);
                        }
                        GuideRunKind::Recheck => {
                            // Fresh mail first; analysed mail only to make up the count.
                            let (fresh, seen): (Vec<_>, Vec<_>) =
                                rows.into_iter().partition(|r| !analysed.contains(&r.message_id));
                            rows = fresh.into_iter().chain(seen).collect();
                        }
                        GuideRunKind::Latest => {}
                    }
                    Ok(rows.into_iter().take(req.count as usize).map(|r| r.message_id).collect())
                })
                .await?)
        })
        .await
    }

    /// Whether `agent` can do the work now; the message to show if not.
    pub(crate) async fn agent_not_ready(self: &Arc<Self>, agent: &str) -> Result<Option<String>, CoreError> {
        let id = provider(agent)?;
        let core = self.clone();
        let statuses =
            runtime::run(async move { Ok::<_, CoreError>(core.agent_runtime().manager.statuses(false).await) }).await?;
        Ok(match statuses.into_iter().find(|(p, _)| *p == id) {
            Some((p, status)) => not_ready(p.display_name(), &status),
            None => Some("No agent is available. Connect Claude Code or Codex in Settings › Agents.".into()),
        })
    }

    /// Ask the agent one prompt in a hidden, read-only session and wait for
    /// the whole answer.
    pub(crate) async fn ask_agent_hidden(self: &Arc<Self>, agent: &str, prompt: String) -> Result<String, CoreError> {
        let session = self.clone().start_read_only_agent_session(agent.to_owned(), vec![]).await?;
        self.agents.with_session(&session, |s| s.hidden = true);
        let answer = self.agents.watch_turn(&session);
        let sent = self
            .clone()
            .send_agent_prompt(
                session.clone(),
                prompt,
                crate::agents::PromptContextInfo { mailbox_id: None, selected_thread_ids: vec![], search_query: None },
            )
            .await;
        let result = match sent {
            Err(e) => Err(e),
            // On the core's runtime: callers may come from the app's own
            // executor, where tokio's timer is not available.
            Ok(()) => {
                match runtime::run(async move { Ok::<_, CoreError>(tokio::time::timeout(TURN_TIMEOUT, answer).await) })
                    .await?
                {
                    Ok(Ok(Ok(text))) => Ok(text),
                    Ok(Ok(Err(message))) => Err(CoreError::new(ErrorKind::Agent, message)),
                    Ok(Err(_)) => Err(CoreError::new(ErrorKind::Agent, "the agent session ended")),
                    Err(_) => {
                        let _ = self.clone().cancel_agent_turn(session.clone()).await;
                        Err(CoreError::new(ErrorKind::Agent, "the agent took too long"))
                    }
                }
            }
        };
        let _ = self.clone().close_agent_session(session).await;
        result
    }

    fn emit_progress(self: &Arc<Self>) {
        let core = self.clone();
        let account = self.effective_account_id();
        runtime::runtime().spawn(crate::registry::scoped(account, async move {
            if let Ok(progress) = core.guide_progress().await {
                core.account_events().emit(CoreEvent::GuideProgress { progress });
            }
        }));
    }

    async fn set_status(&self, run: i64, status: &'static str, error: Option<String>) -> Result<(), CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db.write(move |tx| store::set_run_status(tx, run, status, error.as_deref(), now)).await?)
        })
        .await
    }

    /// Start the job for the account's run in progress, unless one is
    /// already going.
    fn spawn_job(self: &Arc<Self>, run: i64) {
        let Some(account) = self.effective_account_id() else { return };
        let mut jobs = self.agents.guide_jobs.lock().unwrap_or_else(|e| e.into_inner());
        if jobs.get(&account).is_some_and(|j| !j.is_finished()) {
            return;
        }
        let core = self.clone();
        let scoped = account.clone();
        let job = runtime::runtime().spawn(crate::registry::scoped(Some(scoped), async move {
            if let Err(e) = core.run_job(run).await {
                tracing::warn!(error = %e, "guide run stopped");
                let _ = core.set_status(run, "failed", Some(e.to_string())).await;
                core.emit_progress();
            }
        }));
        jobs.insert(account, job);
    }

    /// Batch by batch until the run is done, paused or cancelled. A batch
    /// whose answer cannot be read is tried once more, then skipped.
    ///
    /// The job follows the account's run in progress: when its run was
    /// stopped while a batch was with the agent and a new one started
    /// meanwhile, it carries on with the new one (only one job per account).
    async fn run_job(self: &Arc<Self>, mut run: i64) -> Result<(), CoreError> {
        loop {
            let db = self.db()?;
            let (row, next, active) = runtime::run(async move {
                Ok(db
                    .read(move |c| Ok((store::get_run(c, run)?, store::next_batch(c, run)?, store::active_run(c)?)))
                    .await?)
            })
            .await?;
            let Some(row) = row else { return Ok(()) };
            if row.status != "running" {
                match active {
                    Some(a) if a.id != run && a.status == "running" => {
                        run = a.id;
                        continue;
                    }
                    _ => return Ok(()),
                }
            }
            let agent = row.agent.clone().unwrap_or_else(|| "claude-code".into());
            let Some((batch_no, ids)) = next else {
                self.set_status(run, "done", None).await?;
                let db = self.db()?;
                let now = mail_sync::now_millis().to_string();
                // With fewer than five audiences, suggest the obvious gaps.
                runtime::run(async move {
                    Ok(db
                        .write(move |tx| {
                            store::set_meta(tx, "last_run_at", &now)?;
                            crate::guide::fill_gaps(tx).map(|_| ())
                        })
                        .await?)
                })
                .await?;
                self.guide_changed();
                self.emit_progress();
                return Ok(());
            };
            if let Some(why) = self.agent_not_ready(&agent).await? {
                self.set_status(run, "paused", Some(why)).await?;
                self.emit_progress();
                return Ok(());
            }
            let recheck = row.kind == "recheck";
            let started = std::time::Instant::now();
            let (batch, prompt, known) = self.guide_batch_prompt(ids, row.focus.clone(), recheck).await?;
            if !batch.is_empty() {
                let mut attempts = 0;
                loop {
                    attempts += 1;
                    let answer = match self.ask_agent_hidden(&agent, prompt.clone()).await {
                        Ok(a) => a,
                        Err(e) if e.kind() == ErrorKind::Agent && attempts < 2 => continue,
                        Err(e) => {
                            // The agent itself failed: pause, keeping what was done.
                            self.set_status(run, "paused", Some(format!("The agent stopped: {e}"))).await?;
                            self.emit_progress();
                            return Ok(());
                        }
                    };
                    // Stopped while the agent worked: its answer is dropped.
                    let db = self.db()?;
                    let now = runtime::run(async move { Ok(db.read(move |c| store::get_run(c, run)).await?) }).await?;
                    if now.is_none_or(|r| r.status == "cancelled") {
                        break;
                    }
                    match self.guide_merge_answer(run, &batch, &known, &answer).await {
                        Ok(_) => break,
                        Err(_) if attempts < 2 => continue,
                        Err(e) => {
                            tracing::warn!(error = %e, batch = batch_no, "guide batch skipped: unreadable answer");
                            break;
                        }
                    }
                }
            }
            // Timed only when the agent was asked (the estimate's basis).
            let elapsed = (!batch.is_empty()).then(|| started.elapsed().as_millis() as i64);
            let db = self.db()?;
            runtime::run(async move { Ok(db.write(move |tx| store::finish_batch(tx, run, batch_no, elapsed)).await?) })
                .await?;
            self.emit_progress();
        }
    }
}

#[uniffi::export]
impl Core {
    /// Start learning (spec §14.9). Refused, with the reason to show, when
    /// the agent is not ready, when a run is already in progress, or when
    /// there is no sent mail left to analyse.
    pub async fn start_guide_run(self: Arc<Self>, request: GuideRunRequest) -> Result<GuideRunInfo, CoreError> {
        if request.count == 0 {
            return Err(CoreError::new(ErrorKind::InvalidInput, "choose at least one message"));
        }
        if request.kind == GuideRunKind::Improve && request.focus.as_deref().is_none_or(|f| f.trim().is_empty()) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "choose a category or audience to improve"));
        }
        if let Some(why) = self.agent_not_ready(&request.agent).await? {
            return Err(CoreError::new(ErrorKind::Agent, why));
        }
        let ids = self.run_sample(&request).await?;
        if ids.is_empty() {
            return Err(CoreError::new(ErrorKind::NotFound, "there is no sent mail left to analyse"));
        }
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let req = request.clone();
        let run = runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    // Checked in the same transaction, so two quick starts
                    // cannot both make a run.
                    if store::active_run(tx)?.is_some() {
                        return Ok(None);
                    }
                    let focus = req.focus.as_deref().map(|f| category(f).map(|c| c.id).unwrap_or(f).to_owned());
                    let id = store::create_run(
                        tx,
                        req.kind.as_str(),
                        focus.as_deref(),
                        Some(&req.agent),
                        &ids,
                        BATCH_SIZE,
                        now,
                    )?;
                    Ok(Some(store::get_run(tx, id)?))
                })
                .await?)
        })
        .await?
        .ok_or_else(|| CoreError::new(ErrorKind::InvalidInput, "a learning run is already in progress"))?
        .ok_or_else(|| CoreError::new(ErrorKind::Internal, "the run was not recorded"))?;
        self.spawn_job(run.id);
        self.emit_progress();
        Ok(info(run))
    }

    /// How many messages a run of this request would analyse (the
    /// dialog's preview for further analysis).
    pub async fn guide_run_preview(&self, request: GuideRunRequest) -> Result<u32, CoreError> {
        Ok(self.run_sample(&request).await?.len() as u32)
    }

    /// Pause the run after its current batch.
    pub async fn pause_guide_run(self: Arc<Self>) -> Result<(), CoreError> {
        let db = self.db()?;
        if let Some(run) = runtime::run(async move { Ok(db.read(store::active_run).await?) }).await? {
            self.set_status(run.id, "paused", None).await?;
            self.emit_progress();
        }
        Ok(())
    }

    /// Resume a paused run, or one the app quit in the middle of. Called at
    /// launch too; does nothing when there is none.
    pub async fn resume_guide_run(self: Arc<Self>) -> Result<Option<GuideRunInfo>, CoreError> {
        let db = self.db()?;
        let Some(run) = runtime::run(async move { Ok(db.read(store::active_run).await?) }).await? else {
            return Ok(None);
        };
        let agent = run.agent.clone().unwrap_or_else(|| "claude-code".into());
        if let Some(why) = self.agent_not_ready(&agent).await? {
            self.set_status(run.id, "paused", Some(why.clone())).await?;
            self.emit_progress();
            return Err(CoreError::new(ErrorKind::Agent, why));
        }
        self.set_status(run.id, "running", None).await?;
        self.spawn_job(run.id);
        self.emit_progress();
        let db = self.db()?;
        let id = run.id;
        Ok(runtime::run(async move { Ok(db.read(move |c| store::get_run(c, id)).await?) }).await?.map(info))
    }

    /// Stop the run for good; what was analysed is kept, and its proposals
    /// become decisions.
    pub async fn cancel_guide_run(self: Arc<Self>) -> Result<(), CoreError> {
        let db = self.db()?;
        if let Some(run) = runtime::run(async move { Ok(db.read(store::active_run).await?) }).await? {
            self.set_status(run.id, "cancelled", None).await?;
            self.guide_changed();
            self.emit_progress();
        }
        Ok(())
    }

    /// The run in progress (else the latest) and the decisions waiting.
    pub async fn guide_progress(&self) -> Result<GuideProgress, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(|c| {
                    let run = match store::active_run(c)? {
                        Some(r) => Some(r),
                        None => store::runs(c, 1)?.into_iter().next(),
                    };
                    let (total, done) = store::decision_counts(c)?;
                    Ok(GuideProgress { run: run.map(info), decisions_total: total, decisions_done: done })
                })
                .await?)
        })
        .await
    }

    /// Proposals ready to decide: from finished runs only (no questions
    /// until every batch is processed), by category order.
    pub async fn guide_decisions(&self) -> Result<Vec<GuideEntry>, CoreError> {
        let db = self.db()?;
        let active = runtime::run(async move { Ok(db.read(store::active_run).await?) }).await?.map(|r| r.id);
        let mut waiting: Vec<GuideEntry> = self
            .list_guide_entries(vec![GuideStatus::Proposed])
            .await?
            .into_iter()
            .filter(|e| e.run_id.is_none_or(|r| Some(r) != active))
            .collect();
        let order = |id: &str| CATEGORIES.iter().position(|c| c.id == id).unwrap_or(usize::MAX);
        waiting.sort_by_key(|e| (order(&e.category), std::cmp::Reverse(e.support), e.id));
        Ok(waiting)
    }

    /// Past runs, newest first.
    pub async fn guide_runs(&self, limit: u32) -> Result<Vec<GuideRunInfo>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(move |c| store::runs(c, limit)).await?.into_iter().map(info).collect()) })
            .await
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;
    use crate::guide::tests::demo;

    fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
        for _ in 0..500 {
            if ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("timed out waiting for {what}");
    }

    fn request(kind: GuideRunKind, count: u32) -> GuideRunRequest {
        GuideRunRequest { kind, count, filter: GuideSampleFilter::default(), focus: None, agent: "claude-code".into() }
    }

    #[test]
    fn a_run_goes_batch_by_batch_and_decisions_wait_until_it_is_done() {
        let s = demo("run");
        let core = &s.1;
        core.debug_use_fake_agents();
        block_on(core.debug_seed_demo_mailbox(150)).unwrap();
        let started = block_on(core.clone().start_guide_run(request(GuideRunKind::Latest, 45))).unwrap();
        assert_eq!(started.status, GuideRunStatus::Running);
        assert!(started.total > 20, "more than one batch");
        assert!(block_on(core.clone().start_guide_run(request(GuideRunKind::Latest, 10))).is_err(), "one at a time");
        wait_for("the run to finish", || {
            block_on(core.guide_progress()).unwrap().run.is_some_and(|r| r.status == GuideRunStatus::Done)
        });
        let progress = block_on(core.guide_progress()).unwrap();
        let run = progress.run.unwrap();
        assert_eq!((run.done, run.batches_done), (run.total, run.batches));
        assert_eq!(run.seconds_left, None, "no estimate once done");
        let id = run.id;
        let row = core.db().unwrap().read_blocking(move |c| store::get_run(c, id)).unwrap().unwrap();
        assert_eq!(row.timed_batches, i64::from(run.batches), "every batch was timed");
        let decisions = block_on(core.guide_decisions()).unwrap();
        assert!(!decisions.is_empty(), "the fake agent's proposals are waiting");
        assert_eq!(progress.decisions_total as usize, decisions.len());
        assert_eq!(progress.decisions_done, 0);
        assert!(decisions.windows(2).all(|w| {
            let at = |id: &str| CATEGORIES.iter().position(|c| c.id == id).unwrap();
            at(&w[0].category) <= at(&w[1].category)
        }));
        // The history of agent conversations stays clean.
        assert!(block_on(core.list_agent_history(30)).unwrap().is_empty());

        // Newer mail: nothing sent since; further back: the rest.
        assert_eq!(
            block_on(core.clone().start_guide_run(request(GuideRunKind::Newer, 10))).unwrap_err().kind(),
            ErrorKind::NotFound
        );
        let older = block_on(core.clone().start_guide_run(request(GuideRunKind::Older, 5)));
        if let Ok(r) = older {
            assert!(r.total <= 5);
            wait_for("the older run", || {
                block_on(core.guide_progress()).unwrap().run.is_some_and(|r| r.status == GuideRunStatus::Done)
            });
        }
    }

    #[test]
    fn a_paused_run_keeps_its_place_and_its_proposals_wait() {
        let s = demo("pause");
        let core = &s.1;
        core.debug_use_fake_agents();
        block_on(core.debug_seed_demo_mailbox(150)).unwrap();
        // Recorded but not started: as if the app quit in the middle.
        let ids = block_on(core.guide_sample(40, GuideSampleFilter::default())).unwrap();
        let run = core
            .db()
            .unwrap()
            .write_blocking(move |tx| store::create_run(tx, "latest", None, Some("claude-code"), &ids, 20, 1))
            .unwrap();
        block_on(core.clone().pause_guide_run()).unwrap();
        let p = block_on(core.guide_progress()).unwrap().run.unwrap();
        assert_eq!((p.id, p.status, p.done), (run, GuideRunStatus::Paused, 0));
        block_on(core.clone().resume_guide_run()).unwrap().unwrap();
        wait_for("the resumed run to finish", || {
            block_on(core.guide_progress()).unwrap().run.is_some_and(|r| r.status == GuideRunStatus::Done)
        });
        assert!(block_on(core.clone().resume_guide_run()).unwrap().is_none(), "nothing left to resume");
    }

    #[test]
    fn a_job_left_on_a_stopped_run_carries_on_with_the_new_one() {
        let s = demo("restart");
        let core = &s.1;
        core.debug_use_fake_agents();
        block_on(core.debug_seed_demo_mailbox(60)).unwrap();
        let ids = block_on(core.guide_sample(30, GuideSampleFilter::default())).unwrap();
        // The old run was stopped while its job waited on the agent, and a
        // new one started meanwhile: that job is the account's only job.
        let (old, new) = core
            .db()
            .unwrap()
            .write_blocking(move |tx| {
                let old = store::create_run(tx, "latest", None, Some("claude-code"), &ids, 20, 1)?;
                store::set_run_status(tx, old, "cancelled", None, 2)?;
                let new = store::create_run(tx, "latest", None, Some("claude-code"), &ids, 20, 3)?;
                Ok((old, new))
            })
            .unwrap();
        block_on(core.run_job(old)).unwrap();
        let run = block_on(core.guide_progress()).unwrap().run.unwrap();
        assert_eq!((run.id, run.status), (new, GuideRunStatus::Done), "the new run was not left stalled");
    }

    #[test]
    fn the_time_left_comes_from_the_batches_timed() {
        let row = |status: &str, timed_batches, timed_ms| RunRow {
            status: status.into(),
            batch_size: 20,
            total: 100,
            done: 40,
            timed_batches,
            timed_ms,
            ..Default::default()
        };
        let left = |r: RunRow| info(r).seconds_left;
        assert_eq!(left(row("running", 0, 0)), None, "nothing to go on before the first batch");
        assert_eq!(left(row("running", 2, 60_000)), Some(90), "30 seconds a batch, three batches left");
        assert_eq!(left(row("paused", 2, 60_000)), Some(90));
        assert_eq!(left(row("done", 5, 150_000)), None);
    }

    #[test]
    fn nothing_starts_without_a_ready_agent_or_a_focus() {
        let s = demo("no-agent");
        let core = &s.1;
        core.debug_use_fake_agents();
        block_on(core.debug_seed_demo_mailbox(40)).unwrap();
        let codex = GuideRunRequest { agent: "codex".into(), ..request(GuideRunKind::Latest, 10) };
        let err = block_on(core.clone().start_guide_run(codex)).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Agent);
        assert!(err.to_string().contains("Codex is not installed"), "{err}");
        assert!(err.to_string().contains("Settings › Agents"));
        let improve = request(GuideRunKind::Improve, 10);
        assert_eq!(block_on(core.clone().start_guide_run(improve)).unwrap_err().kind(), ErrorKind::InvalidInput);
        assert!(block_on(core.guide_progress()).unwrap().run.is_none(), "no run was recorded");
    }

    #[test]
    fn further_analysis_takes_newer_older_or_fresh_mail() {
        let s = demo("further");
        let core = &s.1;
        core.debug_use_fake_agents();
        block_on(core.debug_seed_demo_mailbox(150)).unwrap();
        let preview = |kind, count| block_on(core.guide_run_preview(request(kind, count))).unwrap();
        let first = preview(GuideRunKind::Latest, 20);
        assert_eq!(first, 20.min(preview(GuideRunKind::Latest, 10_000)));
        assert_eq!(preview(GuideRunKind::Newer, 1_000), preview(GuideRunKind::Latest, 1_000), "all is newer at first");
        block_on(core.clone().start_guide_run(request(GuideRunKind::Latest, 20))).unwrap();
        wait_for("the first run", || {
            block_on(core.guide_progress()).unwrap().run.is_some_and(|r| r.status == GuideRunStatus::Done)
        });
        assert_eq!(preview(GuideRunKind::Newer, 1_000), 0, "nothing sent since");
        let older = preview(GuideRunKind::Older, 1_000);
        assert_eq!(older, preview(GuideRunKind::Latest, 10_000), "the rest is further back");
        assert_eq!(preview(GuideRunKind::Recheck, 30), 30, "a re-check may re-read analysed mail");
    }

    #[test]
    fn improving_a_category_asks_for_the_messages_that_show_it() {
        let rows = vec![
            store::SentRow { message_id: "a".into(), subject: "Plan".into(), ..Default::default() },
            store::SentRow { message_id: "b".into(), subject: "Fwd: Invoice".into(), ..Default::default() },
            store::SentRow {
                message_id: "c".into(),
                subject: "Hello".into(),
                to: vec![(None, "ann@acme.com".into())],
                ..Default::default()
            },
        ];
        let ids = |v: Vec<store::SentRow>| v.into_iter().map(|r| r.message_id).collect::<Vec<_>>();
        assert_eq!(ids(rank_for_focus(rows.clone(), "e2", &[])), ["b", "a", "c"]);
        let customers = crate::guide::AudienceGroup {
            id: 1,
            name: "Customers".into(),
            status: crate::guide::AudienceStatus::Confirmed,
            description: String::new(),
            members: vec!["@acme.com".into()],
        };
        assert_eq!(ids(rank_for_focus(rows, "customers", &[customers])), ["c", "a", "b"]);
    }
}
