//! The daily review (spec §14.10): once a calendar day per account, while
//! the app is open, match AI compositions to what the user sent and
//! compare the pairs in hidden read-only agent sessions (ADR 0007), batch
//! by batch like a learning run. A run survives quitting: the next launch
//! picks it up. Run Now starts the same review on demand.

use std::sync::Arc;

use mail_domain::Millis;
use mail_store::analysis::{self as store, NewRun, RunRow};

use crate::guide_run::GuideRunStatus;
use crate::{Core, CoreError, CoreEvent, ErrorKind, runtime};

/// Pairs per agent turn.
pub const COMPARE_BATCH: usize = 10;
/// Pairs compared a day unless the user changes it (the cost cap).
pub const DEFAULT_PAIRS_PER_DAY: u32 = 50;
/// After an attempt that could not start (no agent), wait this long.
const RETRY_MS: Millis = 60 * 60 * 1000;

/// Settings keys in `analysis_meta`.
pub(crate) mod keys {
    pub const DAILY_REVIEW: &str = "daily_review";
    pub const PAIRS_PER_DAY: &str = "pairs_per_day";
    pub const AGENT: &str = "agent";
    pub const LAST_RUN_AT: &str = "last_run_at";
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AnalysisRunInfo {
    pub id: i64,
    /// The local calendar day, YYYY-MM-DD.
    pub day: String,
    /// Started by the day's schedule rather than Run Now.
    pub daily: bool,
    pub status: GuideRunStatus,
    /// What matching found when the run started.
    pub matched: u32,
    pub unmatched: u32,
    /// Pairs sent as written (no agent needed).
    pub unchanged: u32,
    /// Pairs to compare, and compared so far.
    pub total: u32,
    pub done: u32,
    pub batches: u32,
    pub batches_done: u32,
    pub agent: Option<String>,
    pub error: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    /// About how many seconds are left, once a batch has been timed.
    pub seconds_left: Option<u32>,
}

/// Where the account's review stands (spec §14.10).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AnalysisProgress {
    /// The account has finished a learning run, so Analysis shows and
    /// reviews run.
    pub available: bool,
    /// The run in progress, else the latest.
    pub run: Option<AnalysisRunInfo>,
    /// Why today's review has not started (no agent ready), if it has not.
    pub waiting: Option<String>,
}

fn status(s: &str) -> GuideRunStatus {
    match s {
        "paused" => GuideRunStatus::Paused,
        "done" => GuideRunStatus::Done,
        "cancelled" => GuideRunStatus::Cancelled,
        "failed" => GuideRunStatus::Failed,
        _ => GuideRunStatus::Running,
    }
}

fn info(r: RunRow) -> AnalysisRunInfo {
    let size = r.batch_size.max(1) as u32;
    let total = r.total.max(0) as u32;
    let done = r.done.max(0) as u32;
    let batches = total.div_ceil(size);
    let batches_done = done.div_ceil(size);
    let seconds_left = (r.timed_batches > 0 && matches!(r.status.as_str(), "running" | "paused")).then(|| {
        let left = i64::from(batches.saturating_sub(batches_done));
        ((r.timed_ms / r.timed_batches * left) / 1000) as u32
    });
    AnalysisRunInfo {
        id: r.id,
        day: r.day,
        daily: r.trigger == "daily",
        status: status(&r.status),
        matched: r.matched.max(0) as u32,
        unmatched: r.unmatched.max(0) as u32,
        unchanged: r.unchanged.max(0) as u32,
        total,
        done,
        batches,
        batches_done,
        agent: r.agent,
        error: r.error,
        started_at: r.started_at,
        finished_at: r.finished_at,
        seconds_left,
    }
}

/// The local calendar day of `ms`.
pub(crate) fn day_of(ms: Millis) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => t.format("%Y-%m-%d").to_string(),
        chrono::LocalResult::None => String::new(),
    }
}

impl Core {
    /// Reviews happen on this account: not an archive, and it has finished
    /// a learning run.
    async fn analysis_available(&self) -> Result<bool, CoreError> {
        if self.effective_account_id().is_some_and(|id| self.is_archive(&id)) {
            return Ok(false);
        }
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(mail_store::compositions::recording).await?) }).await
    }

    /// The agent reviews use: the one chosen for Analysis, else the one the
    /// guide last learned with.
    async fn analysis_agent(&self) -> Result<String, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(|c| {
                    if let Some(agent) = store::meta(c, keys::AGENT)? {
                        return Ok(agent);
                    }
                    Ok(mail_store::guide::runs(c, 20)?
                        .into_iter()
                        .find(|r| r.status == "done")
                        .and_then(|r| r.agent)
                        .unwrap_or_else(|| "claude-code".into()))
                })
                .await?)
        })
        .await
    }

    fn analysis_waiting(&self, why: Option<String>) {
        let Some(account) = self.effective_account_id() else { return };
        let mut waiting = self.agents.analysis_waiting.lock().unwrap_or_else(|e| e.into_inner());
        match why {
            Some(why) => waiting.insert(account, why),
            None => waiting.remove(&account),
        };
    }

    fn emit_analysis_progress(self: &Arc<Self>) {
        let core = self.clone();
        let account = self.effective_account_id();
        runtime::runtime().spawn(crate::registry::scoped(account, async move {
            if let Ok(progress) = core.analysis_progress().await {
                core.account_events().emit(CoreEvent::AnalysisProgress { progress });
            }
        }));
    }

    async fn set_analysis_status(
        &self,
        run: i64,
        status: &'static str,
        error: Option<String>,
    ) -> Result<(), CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db.write(move |tx| store::set_run_status(tx, run, status, error.as_deref(), now)).await?)
        })
        .await
    }

    /// One look from the scheduler (spec §14.10): start the day's review
    /// once the day's first sync has settled, and pick up a run the app
    /// quit in the middle of. Quiet: problems wait for the next look.
    pub(crate) async fn analysis_tick(self: &Arc<Self>, now: Millis) {
        if let Err(e) = self.analysis_tick_inner(now).await {
            tracing::warn!(error = %e, "daily review not started");
        }
    }

    async fn analysis_tick_inner(self: &Arc<Self>, now: Millis) -> Result<(), CoreError> {
        let Some(account) = self.effective_account_id() else { return Ok(()) };
        if !self.analysis_available().await? {
            return Ok(());
        }
        self.purge_if_due(&account, now).await?;
        let db = self.db()?;
        let today = day_of(now);
        let (active, ran, daily) = runtime::run(async move {
            Ok(db
                .read(move |c| {
                    Ok((
                        store::active_run(c)?,
                        store::ran_on(c, &today)?,
                        store::meta(c, keys::DAILY_REVIEW)?.is_none_or(|v| v != "off"),
                    ))
                })
                .await?)
        })
        .await?;
        // Paused by the user on an earlier day: today's review replaces it,
        // and the pairs it had not reached go to that review.
        if let Some(old) = active.as_ref().filter(|r| r.status == "paused" && r.error.is_none() && r.day != day_of(now))
            && daily
            && !ran
        {
            let id = old.id;
            self.set_analysis_status(id, "cancelled", None).await?;
            let db = self.db()?;
            runtime::run(async move { Ok(db.write(move |tx| store::release_pairs(tx, id)).await?) }).await?;
            return Box::pin(self.analysis_tick_inner(now)).await;
        }
        if let Some(run) = active {
            let job_alive = self
                .agents
                .analysis_jobs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&account)
                .is_some_and(|j| !j.is_finished());
            // Quit in the middle of it: carry on.
            if run.status == "running" && !job_alive {
                self.spawn_analysis_job(run.id);
            }
            // Paused because the agent was not ready: try again now and then.
            if run.status == "paused" && run.error.is_some() && self.analysis_attempt_due(&account, now) {
                let agent = run.agent.clone().unwrap_or_else(|| "claude-code".into());
                if self.agent_not_ready(&agent).await?.is_none() {
                    self.set_analysis_status(run.id, "running", None).await?;
                    self.spawn_analysis_job(run.id);
                    self.emit_analysis_progress();
                }
            }
            return Ok(());
        }
        if ran || !daily {
            return Ok(());
        }
        // After the first sync goes idle, so the day's sent mail is in.
        if self.sync_service().is_some_and(|s| !s.settled()) {
            return Ok(());
        }
        if !self.analysis_attempt_due(&account, now) {
            return Ok(());
        }
        match self.start_analysis(true, None, now).await {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == ErrorKind::Agent => {
                self.analysis_waiting(Some(e.to_string()));
                self.emit_analysis_progress();
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Clear old AI drafts' texts, at most hourly (retention, ADR 0013).
    async fn purge_if_due(&self, account: &str, now: Millis) -> Result<(), CoreError> {
        {
            let mut purged = self.agents.analysis_purged.lock().unwrap_or_else(|e| e.into_inner());
            if purged.get(account).is_some_and(|at| now - at < RETRY_MS) {
                return Ok(());
            }
            purged.insert(account.to_owned(), now);
        }
        self.purge_compositions(now).await.map(|_| ())
    }

    /// Clear the texts kept longer than the account's setting.
    pub(crate) async fn purge_compositions(&self, now: Millis) -> Result<usize, CoreError> {
        let keep = i64::from(self.analysis_settings().await?.keep_days) * 24 * 60 * 60 * 1000;
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    // Cloud agents' reports keep their bodies as long (spec §10.6).
                    mail_store::cloud_reports::purge(tx, keep, now)?;
                    mail_store::compositions::purge(tx, keep, now)
                })
                .await?)
        })
        .await
    }

    /// Whether to try (again) now; records the try.
    fn analysis_attempt_due(&self, account: &str, now: Millis) -> bool {
        let mut attempts = self.agents.analysis_attempts.lock().unwrap_or_else(|e| e.into_inner());
        if attempts.get(account).is_some_and(|at| now - at < RETRY_MS) {
            return false;
        }
        attempts.insert(account.to_owned(), now);
        true
    }

    /// Match, choose today's pairs and start the job. `None` agent means
    /// the account's own.
    async fn start_analysis(
        self: &Arc<Self>,
        daily: bool,
        agent: Option<String>,
        now: Millis,
    ) -> Result<RunRow, CoreError> {
        // One start at a time: matching and counting must not run twice.
        let _starting = self.agents.analysis_start.lock().await;
        let agent = match agent {
            Some(a) => a,
            None => self.analysis_agent().await?,
        };
        if let Some(why) = self.agent_not_ready(&agent).await? {
            return Err(CoreError::new(ErrorKind::Agent, why));
        }
        let found = self.match_compositions(now).await?;
        // Sent as written: support for the entries that applied, no agent
        // needed.
        let db = self.db()?;
        let (active, unchanged) =
            runtime::run(
                async move { Ok(db.read(|c| Ok((store::active_run(c)?, store::unchanged_pairs(c)?))).await?) },
            )
            .await?;
        if active.is_some() {
            return Err(CoreError::new(ErrorKind::InvalidInput, "a review is already in progress"));
        }
        self.reinforce_unchanged(&unchanged, now).await?;
        let db = self.db()?;
        let day = day_of(now);
        let run = runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    // Checked in the same transaction: one run at a time.
                    if store::active_run(tx)?.is_some() {
                        return Ok(None);
                    }
                    // The cap is per calendar day, Run Now included.
                    let cap = store::meta(tx, keys::PAIRS_PER_DAY)?
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(DEFAULT_PAIRS_PER_DAY)
                        .saturating_sub(store::pairs_taken_on(tx, &day)?);
                    let pairs = store::pairs_to_compare(tx, cap)?;
                    let new = NewRun {
                        day: &day,
                        trigger: if daily { "daily" } else { "manual" },
                        agent: Some(&agent),
                        matched: i64::from(found.matched),
                        unmatched: i64::from(found.unmatched),
                        unchanged: unchanged.len() as i64,
                        pairs: &pairs,
                        batch_size: COMPARE_BATCH,
                    };
                    let id = store::create_run(tx, &new, now)?;
                    store::get_run(tx, id)
                })
                .await?)
        })
        .await?
        .ok_or_else(|| CoreError::new(ErrorKind::InvalidInput, "a review is already in progress"))?;
        self.analysis_waiting(None);
        self.spawn_analysis_job(run.id);
        self.emit_analysis_progress();
        Ok(run)
    }

    /// Start the job for the account's run, unless one is going.
    fn spawn_analysis_job(self: &Arc<Self>, run: i64) {
        let Some(account) = self.effective_account_id() else { return };
        let mut jobs = self.agents.analysis_jobs.lock().unwrap_or_else(|e| e.into_inner());
        if jobs.get(&account).is_some_and(|j| !j.is_finished()) {
            return;
        }
        let core = self.clone();
        let job = runtime::runtime().spawn(crate::registry::scoped(Some(account.clone()), async move {
            if let Err(e) = core.analysis_job(run).await {
                tracing::warn!(error = %e, "daily review stopped");
                let _ = core.set_analysis_status(run, "failed", Some(e.to_string())).await;
                // Pairs it did not reach wait for the next review.
                if let Ok(db) = core.db() {
                    let _ = runtime::run(async move {
                        Ok::<_, CoreError>(db.write(move |tx| store::release_pairs(tx, run)).await?)
                    })
                    .await;
                }
                core.emit_analysis_progress();
            }
        }));
        jobs.insert(account, job);
    }

    /// Batch by batch until the run is done, paused or cancelled. The job
    /// follows the account's running run: one resumed while this job was
    /// busy is carried on with when this one ends (one job per account).
    async fn analysis_job(self: &Arc<Self>, mut run: i64) -> Result<(), CoreError> {
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
                // Another run, or this one resumed while the job was leaving.
                match active {
                    Some(a) if a.status == "running" => {
                        run = a.id;
                        continue;
                    }
                    _ => return Ok(()),
                }
            }
            let agent = row.agent.clone().unwrap_or_else(|| "claude-code".into());
            let Some((batch, ids)) = next else {
                // Last, the facts in the day's sent mail (spec §14.11).
                match self.glean_facts(run, &agent).await {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::Agent => {
                        self.set_analysis_status(run, "paused", Some(format!("The agent stopped: {e}"))).await?;
                        self.emit_analysis_progress();
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                }
                self.set_analysis_status(run, "done", None).await?;
                let db = self.db()?;
                let at = mail_sync::now_millis().to_string();
                runtime::run(async move { Ok(db.write(move |tx| store::set_meta(tx, keys::LAST_RUN_AT, &at)).await?) })
                    .await?;
                self.emit_analysis_progress();
                // Looks again: another run may be waiting for this job.
                continue;
            };
            if let Some(why) = self.agent_not_ready(&agent).await? {
                self.set_analysis_status(run, "paused", Some(why)).await?;
                self.emit_analysis_progress();
                return Ok(());
            }
            let started = std::time::Instant::now();
            let asked = match self.compare_batch(run, &agent, &ids).await {
                Ok(asked) => asked,
                Err(e) if e.kind() == ErrorKind::Agent => {
                    // The agent itself failed: pause, keeping what was done.
                    self.set_analysis_status(run, "paused", Some(format!("The agent stopped: {e}"))).await?;
                    self.emit_analysis_progress();
                    return Ok(());
                }
                Err(e) => return Err(e),
            };
            let elapsed = asked.then(|| started.elapsed().as_millis() as i64);
            let db = self.db()?;
            let now = mail_sync::now_millis();
            runtime::run(
                async move { Ok(db.write(move |tx| store::finish_batch(tx, run, batch, elapsed, now)).await?) },
            )
            .await?;
            self.emit_analysis_progress();
        }
    }

    /// Compare one batch of pairs with the agent (spec §14.10); whether it
    /// was asked. An answer that cannot be read is asked for once more; a
    /// run stopped meanwhile drops the answer.
    async fn compare_batch(self: &Arc<Self>, run: i64, agent: &str, ids: &[i64]) -> Result<bool, CoreError> {
        let (pairs, entries) = self.compare_pairs(ids).await?;
        if pairs.is_empty() {
            return Ok(false);
        }
        let prompt = crate::analysis_compare::prompt(&pairs, &entries);
        for attempt in 1..=2 {
            let answer = self.ask_agent_hidden(agent, prompt.clone()).await?;
            let db = self.db()?;
            let now = runtime::run(async move { Ok(db.read(move |c| store::get_run(c, run)).await?) }).await?;
            if now.is_none_or(|r| r.status == "cancelled") {
                break;
            }
            if self.merge_compare(&pairs, &entries, &answer).await? {
                self.analysis_changed();
                break;
            }
            if attempt == 2 {
                tracing::warn!(run, "analysis batch skipped: unreadable answer");
            }
        }
        Ok(true)
    }
}

#[uniffi::export]
impl Core {
    /// Run Now (spec §14.10): the day's review on demand, even when it ran
    /// today. Refused, with the reason to show, when the account has not
    /// finished a learning run, when the agent is not ready, or when a
    /// review is in progress.
    pub async fn start_analysis_run(self: Arc<Self>, agent: Option<String>) -> Result<AnalysisRunInfo, CoreError> {
        if !self.analysis_available().await? {
            return Err(CoreError::new(
                ErrorKind::InvalidInput,
                "Analysis starts once the writing guide has learned from your mail",
            ));
        }
        Ok(info(self.start_analysis(false, agent, mail_sync::now_millis()).await?))
    }

    /// Pause the review after its current batch.
    pub async fn pause_analysis_run(self: Arc<Self>) -> Result<(), CoreError> {
        let db = self.db()?;
        if let Some(run) = runtime::run(async move { Ok(db.read(store::active_run).await?) }).await? {
            self.set_analysis_status(run.id, "paused", None).await?;
            self.emit_analysis_progress();
        }
        Ok(())
    }

    /// Resume a paused review.
    pub async fn resume_analysis_run(self: Arc<Self>) -> Result<Option<AnalysisRunInfo>, CoreError> {
        let db = self.db()?;
        let Some(run) = runtime::run(async move { Ok(db.read(store::active_run).await?) }).await? else {
            return Ok(None);
        };
        let agent = run.agent.clone().unwrap_or_else(|| "claude-code".into());
        if let Some(why) = self.agent_not_ready(&agent).await? {
            self.set_analysis_status(run.id, "paused", Some(why.clone())).await?;
            self.emit_analysis_progress();
            return Err(CoreError::new(ErrorKind::Agent, why));
        }
        self.set_analysis_status(run.id, "running", None).await?;
        self.spawn_analysis_job(run.id);
        self.emit_analysis_progress();
        let db = self.db()?;
        let id = run.id;
        Ok(runtime::run(async move { Ok(db.read(move |c| store::get_run(c, id)).await?) }).await?.map(info))
    }

    /// Stop the review; what was compared is kept. Pairs not reached wait
    /// for the next review.
    pub async fn cancel_analysis_run(self: Arc<Self>) -> Result<(), CoreError> {
        let db = self.db()?;
        if let Some(run) = runtime::run(async move { Ok(db.read(store::active_run).await?) }).await? {
            self.set_analysis_status(run.id, "cancelled", None).await?;
            let db = self.db()?;
            let id = run.id;
            // Pairs not compared go back to waiting for a review.
            runtime::run(async move {
                Ok(db
                    .write(move |tx| {
                        tx.execute("DELETE FROM analysis_run_pairs WHERE run_id = ?1 AND NOT done", [id])?;
                        Ok(())
                    })
                    .await?)
            })
            .await?;
            self.emit_analysis_progress();
        }
        Ok(())
    }

    /// The review in progress (else the latest), and why none has started.
    pub async fn analysis_progress(&self) -> Result<AnalysisProgress, CoreError> {
        let available = self.analysis_available().await?;
        let db = self.db()?;
        let run = runtime::run(async move {
            Ok(db
                .read(|c| match store::active_run(c)? {
                    Some(r) => Ok(Some(r)),
                    None => store::latest_run(c),
                })
                .await?)
        })
        .await?;
        let waiting = self.effective_account_id().and_then(|account| {
            self.agents.analysis_waiting.lock().unwrap_or_else(|e| e.into_inner()).get(&account).cloned()
        });
        Ok(AnalysisProgress { available, run: run.map(info), waiting })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use futures::executor::block_on;
    use mail_store::compositions::{self, Kind, NewComposition, Recipients, Source};

    use super::*;

    const DAY: Millis = 24 * 60 * 60 * 1000;

    pub(crate) fn rt<T>(f: impl std::future::Future<Output = T>) -> T {
        runtime::runtime().block_on(f)
    }

    pub(crate) fn learned(core: &Core) {
        let db = core.db().unwrap();
        rt(db.write(|tx| {
            let run = mail_store::guide::create_run(tx, "latest", None, Some("claude-code"), &[], 20, 1)?;
            mail_store::guide::set_run_status(tx, run, "done", None, 2)
        }))
        .unwrap();
    }

    /// Stop the app's own scheduler, which ticks on its own task with the
    /// wall clock, so a test that drives `analysis_tick` itself with the
    /// time it chooses is the only thing reading and changing the runs.
    /// Called before [`learned`]: until then a tick in flight finds
    /// Analysis unavailable and changes nothing.
    pub(crate) fn stop_scheduler(core: &Core) {
        let task = core.agents.scheduler.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(task) = task {
            task.abort();
            let _ = rt(task);
        }
    }

    /// The app's own scheduler would start the day's review meanwhile.
    pub(crate) fn no_schedule(core: &Core) {
        let db = core.db().unwrap();
        rt(db.write(|tx| store::set_meta(tx, keys::DAILY_REVIEW, "off"))).unwrap();
    }

    /// A composition already matched, changed by `distance`.
    fn pair(core: &Core, at: Millis, distance: f64) -> i64 {
        let db = core.db().unwrap();
        rt(db.write(move |tx| {
            let id = compositions::record(
                tx,
                &NewComposition {
                    source: Source::Agent,
                    agent: None,
                    kind: Kind::New,
                    draft_id: -at,
                    thread_id: None,
                    in_reply_to: None,
                    recipients: Recipients::default(),
                    subject: String::new(),
                    instruction: String::new(),
                    ai_text: "Hi Ann, Friday works.".into(),
                    ai_html: None,
                    guide_version: None,
                    audiences: vec![],
                },
                at,
            )?;
            compositions::set_matched(
                tx,
                id,
                &format!("m{id}"),
                "thread_next",
                "Hi Ann, Friday is fine.",
                distance,
                at,
            )?;
            Ok(id)
        }))
        .unwrap()
    }

    pub(crate) fn wait_done(core: &Arc<Core>) -> AnalysisRunInfo {
        for _ in 0..3000 {
            // The app's own scheduler may be the one starting it.
            let progress = block_on(core.analysis_progress()).unwrap();
            if let Some(run) = progress.run
                && run.status != GuideRunStatus::Running
            {
                return run;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the review did not finish");
    }

    /// The day's review waits for sync to go idle first.
    fn settled(core: &Core) {
        for _ in 0..500 {
            if core.sync_service().is_none_or(|s| s.settled()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("sync never went idle");
    }

    fn runs(core: &Core) -> usize {
        let db = core.db().unwrap();
        rt(db.read(|c| Ok(c.query_row("SELECT COUNT(*) FROM analysis_runs", [], |r| r.get::<_, i64>(0))?))).unwrap()
            as usize
    }

    #[test]
    fn nothing_runs_before_the_first_learning_run() {
        let s = crate::guide::tests::demo("analysis-gate");
        let core = &s.1;
        core.debug_use_fake_agents();
        rt(core.analysis_tick(mail_sync::now_millis()));
        assert_eq!(runs(core), 0);
        assert!(!block_on(core.analysis_progress()).unwrap().available);
        let refused = rt(core.clone().start_analysis_run(None)).unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn the_review_runs_once_a_day_within_the_cap_and_run_now_runs_again() {
        let s = crate::guide::tests::demo("analysis-daily");
        let core = &s.1;
        core.debug_use_fake_agents();
        learned(core);
        let now = mail_sync::now_millis();
        let first = pair(core, now - 3 * DAY, 0.3);
        let second = pair(core, now - 2 * DAY, 0.2);
        let third = pair(core, now - DAY, 0.4);
        let same = pair(core, now - DAY, 0.0);
        rt(async { core.db().unwrap().write(|tx| store::set_meta(tx, keys::PAIRS_PER_DAY, "2")).await }).unwrap();

        // Not before the first sync has settled.
        if core.sync_service().is_some_and(|s| !s.settled()) {
            rt(core.analysis_tick_inner(now)).unwrap();
            assert_eq!(runs(core), 0);
            core.agents.analysis_attempts.lock().unwrap().clear();
        }
        settled(core);
        rt(core.analysis_tick_inner(now)).unwrap();
        let run = wait_done(core);
        assert_eq!(run.status, GuideRunStatus::Done);
        assert!(run.daily);
        assert_eq!((run.total, run.done, run.unchanged), (2, 2, 1), "capped at two, oldest first");
        let db = core.db().unwrap();
        let status = |id: i64| rt(db.read(move |c| compositions::get(c, id))).unwrap().unwrap().status;
        assert_eq!((status(first), status(second)), (compositions::Status::Reviewed, compositions::Status::Reviewed));
        assert_eq!(status(third), compositions::Status::Matched, "over the cap: waits for tomorrow");
        assert_eq!(status(same), compositions::Status::Reviewed, "sent as written needs no agent");

        // The same day again: nothing new starts.
        core.agents.analysis_attempts.lock().unwrap().clear();
        rt(core.analysis_tick(now + 1000));
        assert_eq!(runs(core), 1);

        // Run Now runs again today, within the same day's cap.
        let manual = rt(core.clone().start_analysis_run(None)).unwrap();
        assert!(!manual.daily);
        assert_eq!(wait_done(core).total, 0, "the day's two are used");
        assert_eq!(status(third), compositions::Status::Matched);

        // The next day the schedule runs again, and takes the one left.
        core.agents.analysis_attempts.lock().unwrap().clear();
        rt(core.analysis_tick(now + DAY));
        assert_eq!(runs(core), 3);
        assert_eq!(wait_done(core).total, 1);
        assert_eq!(status(third), compositions::Status::Reviewed);
    }

    #[test]
    fn a_run_left_running_by_a_quit_carries_on() {
        let s = crate::guide::tests::demo("analysis-resume");
        let core = &s.1;
        core.debug_use_fake_agents();
        no_schedule(core);
        learned(core);
        let now = mail_sync::now_millis();
        let a = pair(core, now - DAY, 0.3);
        let db = core.db().unwrap();
        // As a quit leaves it: running, no job.
        let run = rt(db.write(move |tx| {
            let new = NewRun {
                day: &day_of(now),
                trigger: "daily",
                agent: Some("claude-code"),
                matched: 1,
                unmatched: 0,
                unchanged: 0,
                pairs: &[a],
                batch_size: COMPARE_BATCH,
            };
            store::create_run(tx, &new, now)
        }))
        .unwrap();
        rt(core.analysis_tick(now + 1000));
        let done = wait_done(core);
        assert_eq!((done.id, done.status, done.done), (run, GuideRunStatus::Done, 1));
        assert_eq!(runs(core), 1, "today's review is that one");
    }

    #[test]
    fn pause_resume_and_cancel() {
        let s = crate::guide::tests::demo("analysis-pause");
        let core = &s.1;
        core.debug_use_fake_agents();
        no_schedule(core);
        learned(core);
        let now = mail_sync::now_millis();
        let pairs: Vec<i64> = (0..25).map(|i| pair(core, now - DAY + i, 0.3)).collect();
        let db = core.db().unwrap();
        let run = rt(db.write(move |tx| {
            let new = NewRun {
                day: &day_of(now),
                trigger: "manual",
                agent: Some("claude-code"),
                matched: 25,
                unmatched: 0,
                unchanged: 0,
                pairs: &pairs,
                batch_size: COMPARE_BATCH,
            };
            let id = store::create_run(tx, &new, now)?;
            store::set_run_status(tx, id, "paused", None, now)?;
            Ok(id)
        }))
        .unwrap();
        // Paused by the user: the scheduler leaves it alone.
        rt(core.analysis_tick(now));
        assert_eq!(block_on(core.analysis_progress()).unwrap().run.unwrap().status, GuideRunStatus::Paused);
        let resumed = rt(core.clone().resume_analysis_run()).unwrap().unwrap();
        assert_eq!(resumed.id, run);
        assert_eq!(wait_done(core).done, 25);

        // Cancelling gives the pairs not reached back.
        let more: Vec<i64> = (0..3).map(|i| pair(core, now - DAY + 100 + i, 0.3)).collect();
        let more2 = more.clone();
        let again = rt(db.write(move |tx| {
            let new = NewRun {
                day: &day_of(now),
                trigger: "manual",
                agent: Some("claude-code"),
                matched: 3,
                unmatched: 0,
                unchanged: 0,
                pairs: &more2,
                batch_size: 1,
            };
            let id = store::create_run(tx, &new, now)?;
            store::set_run_status(tx, id, "paused", None, now)?;
            Ok(id)
        }))
        .unwrap();
        rt(core.clone().cancel_analysis_run()).unwrap();
        let row = rt(db.read(move |c| store::get_run(c, again))).unwrap().unwrap();
        assert_eq!(row.status, "cancelled");
        assert_eq!(rt(db.read(|c| store::pairs_to_compare(c, 50))).unwrap(), more);
    }

    #[test]
    fn a_run_resumed_while_the_job_is_busy_is_carried_on_with() {
        let s = crate::guide::tests::demo("analysis-follow");
        let core = &s.1;
        core.debug_use_fake_agents();
        no_schedule(core);
        learned(core);
        let now = mail_sync::now_millis();
        let first: Vec<i64> = (0..30).map(|i| pair(core, now - DAY + i, 0.3)).collect();
        let second: Vec<i64> = (0..5).map(|i| pair(core, now - DAY + 100 + i, 0.3)).collect();
        let db = core.db().unwrap();
        let make = |pairs: Vec<i64>, status: &'static str| {
            rt(db.write(move |tx| {
                let new = NewRun {
                    day: &day_of(now),
                    trigger: "manual",
                    agent: Some("claude-code"),
                    matched: 0,
                    unmatched: 0,
                    unchanged: 0,
                    pairs: &pairs,
                    batch_size: COMPARE_BATCH,
                };
                let id = store::create_run(tx, &new, now)?;
                store::set_run_status(tx, id, status, None, now)?;
                Ok(id)
            }))
            .unwrap()
        };
        let a = make(first, "running");
        let b = make(second, "paused");
        core.spawn_analysis_job(a);
        // Resumed while the job is on the first run: no second job starts.
        rt(core.clone().resume_analysis_run()).unwrap();
        let started = std::time::Instant::now();
        let done = |id: i64| rt(db.read(move |c| store::get_run(c, id))).unwrap().unwrap().status == "done";
        while !(done(a) && done(b)) {
            assert!(started.elapsed() < std::time::Duration::from_secs(10), "the second run was left waiting");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// A review paused by the user at `paused_at`, with one pair left.
    fn paused_review(core: &Core, paused_at: Millis) -> i64 {
        let left = pair(core, paused_at - DAY, 0.3);
        let db = core.db().unwrap();
        rt(db.write(move |tx| {
            let new = NewRun {
                day: &day_of(paused_at),
                trigger: "daily",
                agent: Some("claude-code"),
                matched: 1,
                unmatched: 0,
                unchanged: 0,
                pairs: &[left],
                batch_size: COMPARE_BATCH,
            };
            let id = store::create_run(tx, &new, paused_at)?;
            store::set_run_status(tx, id, "paused", None, paused_at)?;
            Ok(id)
        }))
        .unwrap()
    }

    /// The first moment of the local day after the one `ms` falls on.
    fn next_local_midnight(ms: Millis) -> Millis {
        use chrono::TimeZone;
        let today = chrono::Local.timestamp_millis_opt(ms).earliest().unwrap().date_naive();
        let midnight = today.succ_opt().unwrap().and_hms_opt(0, 0, 0).unwrap();
        chrono::Local.from_local_datetime(&midnight).earliest().unwrap().timestamp_millis()
    }

    /// The paused run is replaced by `now`'s: cancelled, with the pair it
    /// had not reached in the new run.
    fn replaced(core: &Arc<Core>, old: i64) {
        let today = wait_done(core);
        assert_ne!(today.id, old);
        assert_eq!(today.total, 1, "the pair the paused run had not reached");
        let db = core.db().unwrap();
        assert_eq!(rt(db.read(move |c| store::get_run(c, old))).unwrap().unwrap().status, "cancelled");
    }

    // The scheduler is stopped and every tick is given its time: the app's
    // own tick, running meanwhile with the wall clock, could take the day's
    // one attempt (its start refused while the test made the paused run)
    // and leave today's review an hour away (oagc-ol1h).
    #[test]
    fn a_review_paused_on_an_earlier_day_gives_way_to_todays() {
        let s = crate::guide::tests::demo("analysis-stale-pause");
        let core = &s.1;
        stop_scheduler(core);
        core.debug_use_fake_agents();
        learned(core);
        let now = mail_sync::now_millis();
        let old = paused_review(core, now - 2 * DAY);
        settled(core);
        rt(core.analysis_tick_inner(now)).unwrap();
        replaced(core, old);
    }

    #[test]
    fn a_review_paused_late_yesterday_gives_way_just_after_midnight_and_not_before() {
        let s = crate::guide::tests::demo("analysis-midnight");
        let core = &s.1;
        stop_scheduler(core);
        core.debug_use_fake_agents();
        learned(core);
        let midnight = next_local_midnight(mail_sync::now_millis());
        let old = paused_review(core, midnight - 60_000);
        settled(core);
        // The same day still: it stays paused, and nothing starts.
        rt(core.analysis_tick_inner(midnight - 1)).unwrap();
        let progress = block_on(core.analysis_progress()).unwrap().run.unwrap();
        assert_eq!((progress.id, progress.status), (old, GuideRunStatus::Paused));
        assert_eq!(runs(core), 1);
        // The next day: today's review takes over.
        rt(core.analysis_tick_inner(midnight)).unwrap();
        replaced(core, old);
        let today = block_on(core.analysis_progress()).unwrap().run.unwrap();
        assert_eq!(today.day, day_of(midnight));
        assert_ne!(today.day, day_of(midnight - 1));
    }

    #[test]
    fn days_are_local_calendar_days() {
        let now = mail_sync::now_millis();
        assert_eq!(day_of(now).len(), 10);
        assert_ne!(day_of(now), day_of(now + DAY));
    }
}
