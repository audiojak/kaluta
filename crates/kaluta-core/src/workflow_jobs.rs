//! Workflow background jobs (spec §14.13): the work Kaluta does ahead of a
//! workflow (triage suggestions, the waiting check, reading summaries,
//! nudges). The scheduler looks every tick; the jobs run while the app is
//! open, after sync has settled, while no workflow is open, with the agent
//! the daily review uses (§14.10), each in hidden read-only sessions
//! (ADR 0007), within a per-account daily cap. A job finds its own work in
//! the store and the runner counts what it did per day, so a quit only
//! pauses it: the next launch carries on within the same cap.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use mail_domain::Millis;
use mail_store::workflows as store;

use crate::analysis_run::day_of;
use crate::{Core, CoreError, CoreEvent, ErrorKind, runtime};

/// After the agent could not do the work, wait this long.
const RETRY_MS: Millis = 60 * 60 * 1000;

/// What a job's step does: up to `budget` items, the number it did (none
/// when there is nothing to do).
pub(crate) type StepFuture = Pin<Box<dyn Future<Output = Result<u32, CoreError>> + Send>>;

/// One background job.
pub(crate) struct Job {
    /// Its name in `workflow_job_days`.
    pub name: &'static str,
    /// Items a day per account: the cost cap.
    pub daily_cap: u32,
    /// One batch of its work.
    pub step: fn(Arc<Core>, JobStep) -> StepFuture,
}

/// What a step is given.
pub(crate) struct JobStep {
    /// The agent to ask, ready.
    pub agent: String,
    /// Items left in today's cap.
    pub budget: u32,
    pub now: Millis,
}

/// The jobs, in the order they run; each workflow's lands with it
/// (`docs/plans/workflows.md`).
pub(crate) const JOBS: &[Job] = &[
    Job { name: "triage", daily_cap: crate::triage::DAILY_CAP, step: crate::triage::step },
    Job { name: "waiting", daily_cap: crate::waiting::DAILY_CAP, step: crate::waiting::step },
    Job { name: "nudges", daily_cap: crate::waiting::NUDGE_DAILY_CAP, step: crate::waiting::nudge_step },
    Job { name: "reading", daily_cap: crate::reading::DAILY_CAP, step: crate::reading::step },
];

impl Core {
    /// While a workflow is on screen its items must hold still: jobs wait.
    pub(crate) fn workflow_open(&self) -> bool {
        self.agents.workflow_open.load(Ordering::Relaxed)
    }

    /// One look from the scheduler for the account in scope.
    pub(crate) async fn workflow_jobs_tick(self: &Arc<Self>, now: Millis) {
        self.run_jobs_tick(JOBS, now).await;
    }

    pub(crate) async fn run_jobs_tick(self: &Arc<Self>, jobs: &'static [Job], now: Millis) {
        if jobs.is_empty() || self.workflow_open() || !self.agents.workflow_jobs_on.load(Ordering::Relaxed) {
            return;
        }
        let Some(account) = self.effective_account_id() else { return };
        if self.is_archive(&account) || self.sync_service().is_some_and(|s| !s.settled()) {
            return;
        }
        if self
            .agents
            .workflow_job_retry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&account)
            .is_some_and(|at| now < *at)
        {
            return;
        }
        let mut running = self.agents.workflow_jobs.lock().unwrap_or_else(|e| e.into_inner());
        if running.get(&account).is_some_and(|j| !j.is_finished()) {
            return;
        }
        let core = self.clone();
        let scoped = account.clone();
        let job = runtime::runtime().spawn(crate::registry::scoped(Some(account.clone()), async move {
            if let Err(e) = core.run_jobs(jobs).await {
                tracing::warn!(error = %e, "workflow jobs waiting");
                if e.kind() == ErrorKind::Agent {
                    core.agents
                        .workflow_job_retry
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(scoped, mail_sync::now_millis() + RETRY_MS);
                }
            }
        }));
        running.insert(account, job);
    }

    /// Each job in turn, batch by batch, until none has work or budget
    /// left, a workflow opens, or the agent fails.
    async fn run_jobs(self: &Arc<Self>, jobs: &'static [Job]) -> Result<(), CoreError> {
        let agent = self.analysis_agent().await?;
        if let Some(why) = self.agent_not_ready(&agent).await? {
            return Err(CoreError::new(ErrorKind::Agent, why));
        }
        loop {
            let mut did = 0;
            for job in jobs {
                if self.workflow_open() {
                    return Ok(());
                }
                let now = mail_sync::now_millis();
                let day = day_of(now);
                let db = self.db()?;
                let (name, d) = (job.name, day.clone());
                let used =
                    runtime::run(async move { Ok(db.read(move |c| store::job_items(c, name, &d)).await?) }).await?;
                let budget = job.daily_cap.saturating_sub(used);
                if budget == 0 {
                    continue;
                }
                let result = (job.step)(self.clone(), JobStep { agent: agent.clone(), budget, now }).await;
                let (items, error) = match &result {
                    Ok(n) => ((*n).min(budget), None),
                    Err(e) => (0, Some(e.to_string())),
                };
                if items > 0 || error.is_some() {
                    let db = self.db()?;
                    let at = mail_sync::now_millis();
                    runtime::run(async move {
                        Ok(db.write(move |tx| store::record_job(tx, name, &day, items, error.as_deref(), at)).await?)
                    })
                    .await?;
                }
                match result {
                    Ok(_) if items > 0 => {
                        did += items;
                        self.account_events().emit(CoreEvent::WorkflowsChanged);
                    }
                    Ok(_) => {}
                    Err(e) if e.kind() == ErrorKind::Agent => return Err(e),
                    // Another job may still have work; this one tries again
                    // on the next look.
                    Err(e) => tracing::warn!(job = name, error = %e, "workflow job failed"),
                }
            }
            if did == 0 {
                return Ok(());
            }
        }
    }
}

#[uniffi::export]
impl Core {
    /// The app shows a workflow (`true`) or leaves it: background jobs wait
    /// while one is open, so its items hold still (spec §14.13).
    pub fn set_workflow_open(&self, open: bool) {
        self.agents.workflow_open.store(open, Ordering::Relaxed);
    }

    /// Workflows are on in the app (or off again): the background jobs run
    /// only while they are. Off until the app says.
    pub fn set_workflow_jobs_on(&self, on: bool) {
        self.agents.workflow_jobs_on.store(on, Ordering::Relaxed);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::AtomicU32;

    use super::*;
    use crate::analysis_run::tests::{learned, rt, stop_scheduler};

    /// Work waiting for the test job, and the budgets it was given.
    static PENDING: AtomicU32 = AtomicU32::new(0);
    static BUDGETS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

    fn step(_core: Arc<Core>, step: JobStep) -> StepFuture {
        Box::pin(async move {
            assert!(!step.agent.is_empty() && step.now > 0, "a ready agent and the time");
            BUDGETS.lock().unwrap().push(step.budget);
            // Three items a batch, as a real job takes a batch at a time.
            let n = PENDING.load(Ordering::SeqCst).min(step.budget).min(3);
            PENDING.fetch_sub(n, Ordering::SeqCst);
            Ok(n)
        })
    }

    static TEST_JOBS: &[Job] = &[Job { name: "test", daily_cap: 7, step }];

    pub(crate) fn wait(core: &Arc<Core>) {
        for _ in 0..500 {
            let done = core.agents.workflow_jobs.lock().unwrap().values().all(|j| j.is_finished());
            if done {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("the jobs did not stop");
    }

    /// Run the app's jobs once sync has settled; the triage items done
    /// on this look.
    pub(crate) fn settle_and_run(core: &Arc<Core>) -> u32 {
        settled(core);
        let day = day_of(mail_sync::now_millis());
        let db = core.db().unwrap();
        let d = day.clone();
        let before = rt(db.read(move |c| store::job_items(c, "triage", &d))).unwrap();
        core.agents.workflow_job_retry.lock().unwrap().clear();
        core.set_workflow_jobs_on(true);
        rt(core.workflow_jobs_tick(mail_sync::now_millis()));
        wait(core);
        rt(db.read(move |c| store::job_items(c, "triage", &day))).unwrap() - before
    }

    fn items(core: &Core) -> u32 {
        let db = core.db().unwrap();
        let day = day_of(mail_sync::now_millis());
        rt(db.read(move |c| store::job_items(c, "test", &day))).unwrap()
    }

    fn settled(core: &Core) {
        for _ in 0..500 {
            if core.sync_service().is_none_or(|s| s.settled()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("sync never went idle");
    }

    #[test]
    fn jobs_run_batch_by_batch_within_the_days_cap_and_wait_for_an_open_workflow() {
        let s = crate::guide::tests::demo("workflow-jobs");
        let core = &s.1;
        stop_scheduler(core);
        core.debug_use_fake_agents();
        learned(core);
        settled(core);
        PENDING.store(10, Ordering::SeqCst);

        // Nothing while workflows are off in the app.
        rt(core.run_jobs_tick(TEST_JOBS, mail_sync::now_millis()));
        wait(core);
        assert_eq!(items(core), 0);
        core.set_workflow_jobs_on(true);

        // Not while a workflow is open.
        core.set_workflow_open(true);
        rt(core.run_jobs_tick(TEST_JOBS, mail_sync::now_millis()));
        wait(core);
        assert_eq!(items(core), 0);
        core.set_workflow_open(false);

        // Batch by batch until the cap of seven: 3, 3, 1.
        rt(core.run_jobs_tick(TEST_JOBS, mail_sync::now_millis()));
        wait(core);
        assert_eq!(items(core), 7);
        assert_eq!(PENDING.load(Ordering::SeqCst), 3, "the rest waits for tomorrow");
        assert_eq!(*BUDGETS.lock().unwrap(), vec![7, 4, 1]);

        // The cap is used: another look does nothing more today.
        rt(core.run_jobs_tick(TEST_JOBS, mail_sync::now_millis()));
        wait(core);
        assert_eq!(items(core), 7);
        assert_eq!(BUDGETS.lock().unwrap().len(), 3);
    }
}
