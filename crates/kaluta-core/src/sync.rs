//! Runs the sync engine for the open account (spec §7.4): bootstrap, a
//! backfill loop and an incremental poll, all on the core runtime.
//!
//! Poll interval: 30 s while the app is active, 5 min in the background,
//! immediately on `sync_now` (foreground, wake, network regained), and at
//! once when IMAP IDLE reports new mail (the poll stays as a backstop for
//! changes IDLE does not see). Transient
//! failures back off; an authorization failure stops sync and tells Swift.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mail_store::ThreadChanges;
use mail_sync::{BACKFILL_BATCH, SyncEngine, SyncError, SyncObserver, SyncPhase, SyncProgress};
use provider_api::ProviderError;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::events::{ChangeHint, CoreEvent, EventBus, SyncState};
use crate::{CoreError, ErrorKind};

pub const ACTIVE_POLL: Duration = Duration::from_secs(30);
pub const BACKGROUND_POLL: Duration = Duration::from_secs(300);
pub const DRAFT_MIRROR_INTERVAL: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(300);
/// How often sync looks whether the provider has paused it.
const PAUSE_CHECK: Duration = Duration::from_secs(1);
/// IDLE is re-issued this often: Gmail ends idle sessions at about 29 min.
pub const IDLE_RENEW: Duration = Duration::from_secs(25 * 60);
/// Inbox categories are listed again this often, and whenever the
/// download queue empties.
pub const CATEGORY_REFRESH: Duration = Duration::from_secs(10 * 60);
/// Messages per headers-first batch.
const HEADERS_BATCH: usize = 1_000;

/// Turns engine output into UI events.
pub(crate) struct EventObserver {
    pub events: EventBus,
    /// Set once sync first goes idle: the daily review waits for it (spec
    /// §14.10).
    pub settled: Option<Arc<AtomicBool>>,
}

impl SyncObserver for EventObserver {
    fn threads_changed(&self, changes: &ThreadChanges) {
        for (mailbox, change) in &changes.mailboxes {
            self.events.emit(CoreEvent::ThreadsChanged {
                mailbox_id: mailbox.clone(),
                hint: ChangeHint {
                    inserted: change.inserted.iter().cloned().collect(),
                    updated: change.updated.iter().cloned().collect(),
                    removed: change.removed.iter().cloned().collect(),
                    invalidate: false,
                },
            });
        }
    }

    fn progress(&self, progress: SyncProgress) {
        let state = match progress.phase {
            SyncPhase::Listing => SyncState::Bootstrapping,
            SyncPhase::Backfilling | SyncPhase::Incremental => SyncState::Syncing,
            SyncPhase::Idle => SyncState::Idle,
        };
        if state == SyncState::Idle
            && let Some(settled) = &self.settled
        {
            settled.store(true, Ordering::SeqCst);
        }
        self.events.emit(CoreEvent::SyncStatus {
            state,
            pending: progress.queued.min(u32::MAX as u64) as u32,
            pending_headers: progress.headers.min(u32::MAX as u64) as u32,
            message: None,
        });
    }

    /// Between the ops of a bulk change (Clean Up's batches), so a long
    /// apply shows how much is still to reach the provider.
    fn outbox_progress(&self, counts: mail_store::outbox::OutboxCounts) {
        self.events.emit(CoreEvent::OutboxStatus { pending: counts.pending, failed: counts.failed });
    }
}

/// Told about label changes made outside Kaluta (spec §11.6).
pub(crate) type ExternalChanges = Arc<dyn Fn(Vec<mail_sync::ExternalLabelChange>) + Send + Sync>;

/// Told after each incremental sync that went through: an agent mailbox
/// that publishes then pulls its cloud agents' reports (spec §10.6).
pub(crate) type AfterSync = Arc<dyn Fn() + Send + Sync>;

pub(crate) struct SyncService {
    engine: Arc<SyncEngine>,
    events: EventBus,
    active: AtomicBool,
    poll_now: Notify,
    backfill_wake: Notify,
    outbox_wake: Notify,
    drafts_wake: Notify,
    categories_wake: Notify,
    external: Option<ExternalChanges>,
    after_sync: Option<AfterSync>,
    tasks: std::sync::Mutex<Vec<JoinHandle<()>>>,
    settled: Arc<AtomicBool>,
    /// The last day whose Inbox count this service recorded (Clean Up's
    /// progress card), so a day is looked up once, not every poll.
    inbox_day: std::sync::Mutex<String>,
}

impl SyncService {
    pub fn start(
        engine: Arc<SyncEngine>,
        events: EventBus,
        handle: &tokio::runtime::Handle,
        external: Option<ExternalChanges>,
        after_sync: Option<AfterSync>,
        settled: Arc<AtomicBool>,
    ) -> Arc<Self> {
        let service = Arc::new(Self {
            engine,
            events,
            active: AtomicBool::new(true),
            poll_now: Notify::new(),
            backfill_wake: Notify::new(),
            outbox_wake: Notify::new(),
            drafts_wake: Notify::new(),
            categories_wake: Notify::new(),
            external,
            after_sync,
            tasks: std::sync::Mutex::new(Vec::new()),
            settled,
            inbox_day: std::sync::Mutex::new(String::new()),
        });
        let main = handle.spawn(service.clone().run());
        let pauses = handle.spawn(service.clone().watch_pauses());
        service.tasks.lock().unwrap_or_else(|e| e.into_inner()).extend([main, pauses]);
        service
    }

    /// Tell the app when the provider pauses every request (a rate limit)
    /// and when it resumes: sync is waiting inside the provider then, and
    /// says nothing itself.
    async fn watch_pauses(self: Arc<Self>) {
        let mut told: Option<i64> = None;
        loop {
            let until = self
                .engine
                .provider()
                .paused_for()
                .await
                .map(|left| mail_sync::now_millis() + i64::try_from(left.as_millis()).unwrap_or(i64::MAX / 2));
            // A pause made longer is told again; one counting down is not.
            let changed = match (told, until) {
                (None, None) => false,
                (Some(a), Some(b)) => (b - a).abs() > 2_000,
                _ => true,
            };
            if changed {
                self.events.emit(CoreEvent::SyncPaused { until });
                told = until;
            }
            tokio::time::sleep(PAUSE_CHECK).await;
        }
    }

    pub fn set_active(&self, active: bool) {
        let was = self.active.swap(active, Ordering::SeqCst);
        if active && !was {
            self.poll_now.notify_one();
        }
    }

    /// Whether sync has gone idle at least once since it started.
    pub fn settled(&self) -> bool {
        self.settled.load(Ordering::SeqCst)
    }

    pub fn engine(&self) -> &SyncEngine {
        &self.engine
    }

    /// A change was queued; push it now.
    pub fn outbox_changed(&self) {
        self.outbox_wake.notify_one();
    }

    /// Fetch these bodies next and wake the backfill.
    pub async fn prioritize(&self, ids: Vec<mail_domain::MessageId>) -> Result<(), mail_sync::SyncError> {
        self.engine.prioritize(ids).await?;
        self.backfill_wake.notify_one();
        Ok(())
    }

    /// Mirror edited drafts to the server now (the composer closed).
    pub fn flush_drafts(&self) {
        self.drafts_wake.notify_one();
    }

    pub fn sync_now(&self) {
        self.poll_now.notify_one();
        self.backfill_wake.notify_one();
    }

    pub fn stop(&self) {
        for task in self.tasks.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            task.abort();
        }
    }

    async fn run(self: Arc<Self>) {
        // Bootstrap (resumable: prepare keeps an existing cursor).
        match self.engine.needs_bootstrap().await {
            Ok(true) => {
                self.status(SyncState::Bootstrapping, 0);
                if let Err(e) = self.retrying(|| self.engine.bootstrap_prepare()).await {
                    self.fail(e);
                    return;
                }
                let lister = self.clone();
                let task = tokio::spawn(async move {
                    match lister.retrying(|| lister.engine.bootstrap_list_rest()).await {
                        Ok(()) => lister.backfill_wake.notify_one(),
                        Err(e) => lister.fail(e),
                    }
                });
                self.tasks.lock().unwrap_or_else(|e| e.into_inner()).push(task);
            }
            Ok(false) => {
                if let Err(e) = self.retrying(|| self.engine.ensure_window()).await {
                    self.fail(e);
                    return;
                }
                // Tiered download (spec §7.4): IMAP on or off since the
                // queue was listed re-lists the window.
                if let Err(e) = self.retrying(|| self.engine.ensure_tiers()).await {
                    self.fail(e);
                    return;
                }
            }
            Err(e) => {
                self.fail(e);
                return;
            }
        }

        let backfiller = self.clone();
        let backfill = tokio::spawn(async move { backfiller.backfill_loop().await });
        let pusher = self.clone();
        let outbox = tokio::spawn(async move { pusher.outbox_loop().await });
        let mirror = self.clone();
        let drafts = tokio::spawn(async move { mirror.drafts_loop().await });
        let listener = self.clone();
        let push = tokio::spawn(async move { listener.push_loop().await });
        let sorter = self.clone();
        let categories = tokio::spawn(async move { sorter.categories_loop().await });
        self.tasks.lock().unwrap_or_else(|e| e.into_inner()).extend([backfill, outbox, drafts, push, categories]);
        self.poll_loop().await;
    }

    async fn backfill_loop(self: Arc<Self>) {
        let mut backoff = Duration::from_secs(2);
        let mut busy = false;
        loop {
            // Headers first where the source makes them cheap (IMAP), so
            // the list fills in minutes; bodies follow (spec §7.4).
            loop {
                match self.engine.headers_pass(HEADERS_BATCH).await {
                    Ok(0) => break,
                    Ok(_) => {
                        busy = true;
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "headers pass failed; bodies continue");
                        break;
                    }
                }
            }
            match self.engine.backfill_batch(BACKFILL_BATCH).await {
                Ok(0) => {
                    backoff = Duration::from_secs(2);
                    // Downloads finished: their categories can be applied.
                    if std::mem::take(&mut busy) {
                        self.categories_wake.notify_one();
                    }
                    // Nothing queued: sleep until something is.
                    tokio::select! {
                        () = self.backfill_wake.notified() => {}
                        () = tokio::time::sleep(Duration::from_secs(60)) => {}
                    }
                }
                Ok(_) => {
                    backoff = Duration::from_secs(2);
                    busy = true;
                }
                Err(e) if Self::is_fatal(&e) => {
                    self.fail(e);
                    return;
                }
                Err(e) => {
                    // Anything else (a bad page, a store hiccup) is retried
                    // rather than leaving the mailbox half-synced.
                    self.offline_or_error(&e);
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }
    }

    /// Errors no retry can fix: the user must sign in or grant access.
    fn is_fatal(e: &SyncError) -> bool {
        matches!(e, SyncError::Provider(ProviderError::Unauthorized | ProviderError::Forbidden(_)))
    }

    /// Push queued changes as they are made; wake again at the next retry.
    async fn outbox_loop(self: Arc<Self>) {
        loop {
            match self.engine.drain_outbox().await {
                Ok(_) => {}
                Err(e) if matches!(e, SyncError::Provider(ProviderError::Unauthorized)) => {
                    self.fail(e);
                    return;
                }
                Err(e) => tracing::warn!(error = %e, "outbox drain failed"),
            }
            if let Ok(c) = self.engine.outbox_counts().await {
                self.events.emit(CoreEvent::OutboxStatus { pending: c.pending, failed: c.failed });
            }
            let wait = self
                .engine
                .next_outbox_retry()
                .await
                .ok()
                .flatten()
                .map(|at| Duration::from_millis((at - mail_sync::now_millis()).max(250) as u64))
                .unwrap_or(Duration::from_secs(60));
            tokio::select! {
                () = self.outbox_wake.notified() => {}
                () = tokio::time::sleep(wait) => {}
            }
        }
    }

    /// Every 30 s (or when a composer closes), queue a server update for
    /// each draft edited since the last one (spec §14.5).
    async fn drafts_loop(self: Arc<Self>) {
        loop {
            tokio::select! {
                () = self.drafts_wake.notified() => {}
                () = tokio::time::sleep(DRAFT_MIRROR_INTERVAL) => {}
            }
            let db = self.engine.db().clone();
            let email: String = match db.read(|c| mail_store::read::sync_state(c, "account_email")).await {
                Ok(Some(email)) => email,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(error = %e, "reading the account address failed");
                    continue;
                }
            };
            match mail_sync::schedule_draft_sync(&db, mail_domain::EmailAddress::new(None, &email)).await {
                Ok(0) => {}
                Ok(_) => self.outbox_wake.notify_one(),
                Err(e) => tracing::warn!(error = %e, "scheduling draft sync failed"),
            }
        }
    }

    /// Inbox categories for mail downloaded over IMAP, which leaves them
    /// out: at start, when the download queue empties, and every
    /// [`CATEGORY_REFRESH`].
    async fn categories_loop(self: Arc<Self>) {
        loop {
            if let Err(e) = self.engine.sync_categories().await {
                tracing::warn!(error = %e, "inbox categories not refreshed");
            }
            tokio::select! {
                () = self.categories_wake.notified() => {}
                () = tokio::time::sleep(CATEGORY_REFRESH) => {}
            }
        }
    }

    /// New mail at once over IMAP IDLE; when push is not available, try
    /// again later, backing off.
    async fn push_loop(self: Arc<Self>) {
        let first = Duration::from_secs(30);
        let mut backoff = first;
        loop {
            match self.engine.wait_for_push(IDLE_RENEW).await {
                Some(true) => {
                    backoff = first;
                    self.poll_now.notify_one();
                }
                Some(false) => backoff = first,
                None => {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }
    }

    async fn poll_loop(self: Arc<Self>) {
        let mut backoff = Duration::from_secs(5);
        loop {
            self.status(SyncState::Checking, 0);
            match self.engine.sync_incremental().await {
                Ok(report) => {
                    backoff = Duration::from_secs(5);
                    // The day's mail is in: the daily review may start (spec
                    // §14.10). A synced account with nothing to download
                    // never reports Idle, so this marks it too.
                    self.settled.store(true, Ordering::SeqCst);
                    if !report.external_label_changes.is_empty()
                        && let Some(notify) = &self.external
                    {
                        notify(report.external_label_changes.clone());
                    }
                    if !report.new_mail.is_empty() {
                        let messages = report.new_mail.into_iter().map(Into::into).collect();
                        self.events.emit(CoreEvent::NewMail { messages });
                    }
                    self.record_inbox_day().await;
                    if let Some(after) = &self.after_sync {
                        after();
                    }
                }
                Err(SyncError::ResyncStarted) => self.backfill_wake.notify_one(),
                Err(e) if Self::is_fatal(&e) => {
                    self.fail(e);
                    return;
                }
                Err(e) => {
                    self.offline_or_error(&e);
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                    continue;
                }
            }
            // New mail may have queued unfetched ids.
            self.backfill_wake.notify_one();
            let interval = if self.active.load(Ordering::SeqCst) { ACTIVE_POLL } else { BACKGROUND_POLL };
            tokio::select! {
                () = self.poll_now.notified() => {}
                () = tokio::time::sleep(interval) => {}
            }
        }
    }

    /// The first sync after local midnight records the Inbox's count at the
    /// start of the day, for Clean Up's progress card (spec §14.12). Not
    /// before the first listing is done, nor while the Inbox phases are
    /// still being fetched (the listing marks the account bootstrapped
    /// before the backfill stores them).
    async fn record_inbox_day(&self) {
        let now = mail_sync::now_millis();
        let offset = crate::cleanup::utc_offset_now();
        let day = mail_store::cleanup::day_key(now, offset);
        if *self.inbox_day.lock().unwrap_or_else(|e| e.into_inner()) == day
            || !matches!(self.engine.needs_bootstrap().await, Ok(false))
        {
            return;
        }
        // While the Inbox phases are still being fetched nothing is
        // recorded and `inbox_day` stays unset, so a later poll tries again.
        let recorded = self
            .engine
            .db()
            .write(move |tx| {
                mail_store::cleanup::record_today(tx, now, offset)?;
                mail_store::cleanup::today_recorded(tx, now, offset)
            })
            .await;
        match recorded {
            Ok(true) => *self.inbox_day.lock().unwrap_or_else(|e| e.into_inner()) = day,
            Ok(false) => tracing::debug!("the Inbox is still filling; today's count waits"),
            Err(e) => tracing::warn!(error = %e, "the Inbox's count for today was not recorded"),
        }
    }

    /// Retry a transient-failing step with backoff; return other errors.
    async fn retrying<F, Fut>(&self, mut step: F) -> Result<(), SyncError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<(), SyncError>>,
    {
        let mut backoff = Duration::from_secs(2);
        loop {
            match step().await {
                Err(e) if !Self::is_fatal(&e) => {
                    self.offline_or_error(&e);
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
                other => return other,
            }
        }
    }

    fn status(&self, state: SyncState, pending: u32) {
        self.events.emit(CoreEvent::SyncStatus { state, pending, pending_headers: 0, message: None });
    }

    fn offline_or_error(&self, e: &SyncError) {
        tracing::warn!(error = %e, "sync step failed; will retry");
        let offline = matches!(e, SyncError::Provider(ProviderError::Network(_)));
        self.events.emit(CoreEvent::SyncStatus {
            state: if offline { SyncState::Offline } else { SyncState::Error },
            pending: 0,
            pending_headers: 0,
            message: Some(e.to_string()),
        });
    }

    fn fail(&self, e: SyncError) {
        tracing::error!(error = %e, "sync stopped");
        self.events.emit(CoreEvent::SyncStatus {
            state: SyncState::Error,
            pending: 0,
            pending_headers: 0,
            message: Some(e.to_string()),
        });
        let kind = match &e {
            SyncError::Provider(ProviderError::Unauthorized) => ErrorKind::Auth,
            SyncError::Provider(ProviderError::Forbidden(_)) => ErrorKind::PermissionDenied,
            SyncError::Store(_) => ErrorKind::Storage,
            _ => ErrorKind::Network,
        };
        let CoreError::Failed { kind, message } = CoreError::new(kind, e.to_string());
        self.events.emit(CoreEvent::Error { kind, message });
    }
}
