//! The sync engine (spec §7.4): bootstrap, prioritized backfill and
//! incremental history sync. Scheduling (poll intervals, app state) lives in
//! the core; this module performs one step at a time so each is testable.

use std::collections::BTreeSet;
use std::sync::Arc;

use mail_domain::{EmailAddress, LabelId, MessageId, Millis, ThreadId, system_labels};
use mail_store::{Db, IncomingMessage, MailWriter, ThreadChanges, queue, read};
use provider_api::{
    BackfillSource, Change, ListFilter, MailProvider, PageToken, Priority, ProviderError, RestBackfill,
};

use crate::convert::to_incoming;
use crate::error::{SyncError, SyncResult};

/// How far back the initial sync downloads mail (spec §7.4 amendment).
/// The inbox and the last 30 days always come down; older mail only within
/// the window. Everything else stays on the server until the window widens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncWindow {
    Month,
    #[default]
    HalfYear,
    Year,
    Everything,
}

impl SyncWindow {
    pub const ALL: [SyncWindow; 4] = [Self::Month, Self::HalfYear, Self::Year, Self::Everything];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Month => "1m",
            Self::HalfYear => "6m",
            Self::Year => "1y",
            Self::Everything => "all",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.as_str() == s)
    }
}

/// How much of the window comes down as full messages when headers are
/// cheap (spec §7.4 amendment 2026-09-27, tiered download). The Inbox
/// always does; the rest of the window gets headers only, and bodies on
/// demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BodyWindow {
    #[default]
    Month,
    HalfYear,
    Year,
    /// Everything in the sync window.
    Window,
}

impl BodyWindow {
    pub const ALL: [BodyWindow; 4] = [Self::Month, Self::HalfYear, Self::Year, Self::Window];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Month => "30d",
            Self::HalfYear => "6m",
            Self::Year => "1y",
            Self::Window => "window",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.as_str() == s)
    }

    /// The first phase priority listed for headers only.
    fn headers_from(self) -> Option<u8> {
        match self {
            Self::Month => Some(3),
            Self::HalfYear => Some(4),
            Self::Year => Some(5),
            Self::Window => None,
        }
    }

    /// The body window that keeps bodies where they are when the sync
    /// window widens from `window`: "the whole window" becomes the span
    /// `window` covered, so a wider window brings headers, not bodies.
    pub fn kept_from(self, window: SyncWindow) -> Self {
        match (self, window) {
            (Self::Window, SyncWindow::Month) => Self::Month,
            (Self::Window, SyncWindow::HalfYear) => Self::HalfYear,
            (Self::Window, SyncWindow::Year) => Self::Year,
            (body, _) => body,
        }
    }
}

/// One backfill phase: a priority and the provider list filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Phase {
    pub priority: u8,
    pub labels: &'static [&'static str],
    pub query: Option<String>,
}

/// Backfill phases for a window, most urgent first (spec §7.4). Each lists
/// message ids into the queue at its priority. Every age tier has its own
/// priority (3 = six months, 4 = a year, 5 = older) so the body window can
/// cut between them. Spam and Trash are listed whole with the last month:
/// Gmail empties them after 30 days, and All Mail leaves them out.
pub fn phases_for(window: SyncWindow) -> Vec<Phase> {
    let mut phases = vec![
        Phase { priority: 0, labels: &["INBOX"], query: Some("is:unread".into()) },
        Phase { priority: 1, labels: &["INBOX"], query: None },
        Phase { priority: 2, labels: &[], query: Some("newer_than:30d".into()) },
        Phase { priority: 2, labels: &["SPAM"], query: None },
        Phase { priority: 2, labels: &["TRASH"], query: None },
    ];
    let half = Phase { priority: 3, labels: &[], query: Some("newer_than:180d".into()) };
    let year = Phase { priority: 4, labels: &[], query: Some("newer_than:365d".into()) };
    match window {
        SyncWindow::Month => {}
        SyncWindow::HalfYear => phases.push(half),
        SyncWindow::Year => phases.extend([half, year]),
        SyncWindow::Everything => phases.extend([half, year, Phase { priority: 5, labels: &[], query: None }]),
    }
    phases
}

/// The queue priority for a phase: headers only from `headers_from` on.
fn queue_priority(phase: &Phase, headers_from: Option<u8>) -> u8 {
    match headers_from {
        Some(cut) if phase.priority >= cut => phase.priority + queue::HEADERS_ONLY,
        _ => phase.priority,
    }
}

/// Which tiering a queue was listed under, recorded so a change (IMAP on
/// or off, another body window) re-lists it.
fn tiers_tag(headers_from: Option<u8>) -> String {
    headers_from.map_or_else(|| "flat".to_owned(), |cut| format!("headers-from-{cut}"))
}

/// A phase as a Gmail search, for listing over IMAP (`X-GM-RAW`):
/// its labels as `in:` terms, then its query; `""` for everything.
fn phase_search(phase: &Phase) -> String {
    let mut terms: Vec<String> = phase.labels.iter().map(|l| format!("in:{}", l.to_ascii_lowercase())).collect();
    if let Some(q) = &phase.query {
        terms.push(q.clone());
    }
    terms.join(" ")
}

/// Phases listed before backfill starts, so the inbox fills first.
pub const INBOX_PHASES: usize = 2;
/// Phases every window shares; the rest depend on the window.
const FIXED_PHASES: usize = 5;
/// The first priority of the window's own phases.
const WINDOW_PRIORITY: u8 = 3;
pub const BACKFILL_BATCH: usize = 50;

const KEY_CURSOR: &str = "history_cursor";
const KEY_BOOTSTRAPPED: &str = "bootstrap_listed";
const KEY_EMAIL: &str = "account_email";
pub const KEY_WINDOW: &str = "sync_window";
pub const KEY_BODY_WINDOW: &str = "body_window";
const KEY_TIERS: &str = "queue_tiers";

/// [`SyncEngine::load_every_header`] for an account that is not syncing:
/// the window and body window are stored, and the queue's tiering is
/// marked stale so the next start lists the wider window
/// ([`SyncEngine::ensure_tiers`]). Returns false when the window was
/// *Everything* already.
pub async fn load_every_header_stored(db: &Db) -> SyncResult<bool> {
    Ok(db
        .write(|tx| {
            let window = read::sync_state(tx, KEY_WINDOW)?.as_deref().and_then(SyncWindow::parse).unwrap_or_default();
            if window == SyncWindow::Everything {
                return Ok(false);
            }
            let body = read::sync_state(tx, KEY_BODY_WINDOW)?
                .as_deref()
                .and_then(BodyWindow::parse)
                .unwrap_or_default()
                .kept_from(window);
            read::set_sync_state(tx, KEY_BODY_WINDOW, body.as_str())?;
            read::set_sync_state(tx, KEY_WINDOW, SyncWindow::Everything.as_str())?;
            read::set_sync_state(tx, KEY_TIERS, "stale")?;
            Ok(true)
        })
        .await?)
}

/// Receives what sync changed; the core turns it into UI events.
pub trait SyncObserver: Send + Sync {
    fn threads_changed(&self, changes: &ThreadChanges);
    fn progress(&self, _progress: SyncProgress) {}
    /// An outbox op reached the provider and more are waiting (a bulk
    /// change goes out as many ops, spec §14.12).
    fn outbox_progress(&self, _counts: mail_store::outbox::OutboxCounts) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPhase {
    Listing,
    Backfilling,
    Incremental,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncProgress {
    pub phase: SyncPhase,
    /// Messages still waiting for a full fetch.
    pub queued: u64,
    /// Messages waiting for headers only (tiered download).
    pub headers: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IncrementalReport {
    pub added: usize,
    pub deleted: usize,
    pub relabeled: usize,
    /// Messages that arrived since the last sync, unread in the Inbox and
    /// not sent by the user: what a new-mail notification is about.
    pub new_mail: Vec<NewMail>,
    /// Label changes made outside OpenAGC.
    pub external_label_changes: Vec<ExternalLabelChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMail {
    pub id: MessageId,
    pub thread_id: ThreadId,
    pub from: Option<EmailAddress>,
    pub subject: String,
    pub snippet: String,
}

impl NewMail {
    /// `m` is new mail worth announcing if it is unread in the Inbox, not
    /// from the user, and was not already stored (e.g. by a backfill).
    fn from_incoming(m: &IncomingMessage, already_stored: bool) -> Option<Self> {
        let has = |l: &str| m.label_ids.iter().any(|x| x.as_str() == l);
        let fresh = !already_stored
            && has(system_labels::INBOX)
            && has(system_labels::UNREAD)
            && !has(system_labels::SENT)
            && !has(system_labels::SPAM)
            && !has(system_labels::TRASH);
        fresh.then(|| Self {
            id: m.id.clone(),
            thread_id: m.thread_id.clone(),
            from: m.from.clone(),
            subject: m.subject.clone(),
            snippet: m.snippet.clone(),
        })
    }
}

pub struct SyncEngine {
    provider: Arc<dyn MailProvider>,
    /// Where backfill gets bodies; REST unless a bulk source is set.
    backfill: std::sync::RwLock<Arc<dyn BackfillSource>>,
    db: Db,
    pub(crate) observer: Arc<dyn SyncObserver>,
    /// Serializes outbox drains so one op is never sent twice.
    pub(crate) drain_lock: tokio::sync::Mutex<()>,
    /// Label changes OpenAGC itself pushed recently, so history sync can
    /// tell them from changes made elsewhere (spec §11.6).
    pub(crate) own_changes: std::sync::Mutex<Vec<OwnChange>>,
    /// Queued ids the headers pass asked for and did not get (gone from
    /// All Mail since they were listed): left to the body backfill, which
    /// falls back to the API, and not asked for again.
    headers_missed: std::sync::Mutex<std::collections::HashSet<MessageId>>,
    /// IMAP or the API per job, the breaker, and recent operations
    /// (docs/plans/imap-first-sync.md).
    pub(crate) transport: crate::transport::Transport,
}

/// One label change the outbox pushed.
#[derive(Debug, Clone)]
pub(crate) struct OwnChange {
    pub message: MessageId,
    pub label: LabelId,
    pub added: bool,
    pub at: Millis,
}

/// A label change seen in history that OpenAGC did not make: another
/// client, a filter, or a cloud routine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalLabelChange {
    pub message: MessageId,
    pub thread: ThreadId,
    pub added: Vec<LabelId>,
    pub removed: Vec<LabelId>,
}

/// How long an own change is remembered.
const OWN_CHANGE_WINDOW: Millis = 2 * 60 * 60 * 1000;

impl SyncEngine {
    pub fn new(provider: Arc<dyn MailProvider>, db: Db, observer: Arc<dyn SyncObserver>) -> Self {
        Self {
            backfill: std::sync::RwLock::new(Arc::new(RestBackfill(provider.clone()))),
            provider,
            db,
            observer,
            drain_lock: tokio::sync::Mutex::new(()),
            own_changes: std::sync::Mutex::new(Vec::new()),
            headers_missed: Default::default(),
            transport: Default::default(),
        }
    }

    /// Which transport served each job lately, the breaker, recent
    /// operations: for the Sync Debugger and the sync footer.
    pub fn transport_snapshot(&self) -> crate::transport::TransportSnapshot {
        self.transport.snapshot()
    }

    /// Try IMAP again at once (the user asked for a refresh).
    pub fn reset_transport(&self) {
        self.transport.reset();
    }

    /// Record an operation served by the API by design (no IMAP path, or
    /// the API is the faster one for this job).
    pub(crate) fn record_api(
        &self,
        timer: &crate::transport::Timer,
        job: crate::transport::Job,
        reason: &str,
        items: usize,
        ok: bool,
    ) {
        self.transport.record(timer.finish(job, crate::transport::Via::Api, Some(reason.to_owned()), items, ok));
    }

    /// Whole messages: over IMAP when the account has it and the breaker
    /// allows, else over the API, recording which and why.
    async fn fetch_bodies(
        &self,
        ids: &[MessageId],
        priority: Priority,
    ) -> SyncResult<Vec<provider_api::FetchedMessage>> {
        use crate::transport::{Job, Timer, Via};
        let source = self.backfill.read().unwrap_or_else(|e| e.into_inner()).clone();
        let reason = match self.imap_gate(source.as_ref()) {
            Ok(()) => {
                let timer = Timer::start();
                match source.fetch(ids).await {
                    Ok(v) => {
                        self.transport.imap_succeeded();
                        self.transport.record(timer.finish(Job::Bodies, Via::Imap, None, v.len(), true));
                        return Ok(v);
                    }
                    Err(e) => self.imap_error(&e),
                }
            }
            Err(reason) => reason,
        };
        let timer = Timer::start();
        let result = self.provider.fetch_messages(ids, priority).await;
        self.transport.record(timer.finish(
            Job::Bodies,
            Via::Api,
            Some(reason),
            result.as_ref().map_or(0, Vec::len),
            result.is_ok(),
        ));
        Ok(result?)
    }

    /// Headers without bodies, when IMAP can give them cheaply; `None`
    /// otherwise (the API charges as much for headers as for bodies).
    async fn fetch_headers(&self, ids: &[MessageId]) -> SyncResult<Option<Vec<provider_api::FetchedMessage>>> {
        use crate::transport::{Job, Timer, Via};
        let source = self.backfill.read().unwrap_or_else(|e| e.into_inner()).clone();
        if self.imap_gate(source.as_ref()).is_err() {
            return Ok(None);
        }
        let timer = Timer::start();
        match source.fetch_headers(ids).await {
            Ok(Some(v)) => {
                self.transport.imap_succeeded();
                self.transport.record(timer.finish(Job::Headers, Via::Imap, None, v.len(), true));
                Ok(Some(v))
            }
            Ok(None) => Ok(None),
            Err(e) => {
                let reason = self.imap_error(&e);
                self.transport.record(timer.finish(Job::Headers, Via::Imap, Some(reason), 0, false));
                Ok(None)
            }
        }
    }

    /// Ids matching a Gmail search, over IMAP; why not, when it cannot.
    async fn list_via_imap(&self, query: &str, job: crate::transport::Job) -> Result<Vec<MessageId>, String> {
        use crate::transport::{Timer, Via};
        let source = self.backfill.read().unwrap_or_else(|e| e.into_inner()).clone();
        self.imap_gate(source.as_ref())?;
        let timer = Timer::start();
        match source.list(query).await {
            Ok(Some(ids)) => {
                self.transport.imap_succeeded();
                self.transport.record(timer.finish(job, Via::Imap, None, ids.len(), true));
                Ok(ids)
            }
            Ok(None) => Err("this download source cannot list".into()),
            Err(e) => Err(self.imap_error(&e)),
        }
    }

    /// Whether IMAP may serve now; otherwise why not.
    fn imap_gate(&self, source: &dyn BackfillSource) -> Result<(), String> {
        match source.name() {
            "rest" => Err("IMAP is not set up for this account".into()),
            "imap-refused" => Err("IMAP sign-in was refused for this account".into()),
            _ if !self.transport.imap_allowed(crate::outbox::now_millis()) => Err(self.transport.breaker_reason()),
            _ => Ok(()),
        }
    }

    /// Wait up to `max` for new mail over IMAP IDLE: `Some(true)` when the
    /// mailbox changed (run an incremental round now), `Some(false)` when
    /// the wait ran out, `None` when push is not available now (no IMAP,
    /// the breaker open, or the connection failed; the poll carries on).
    /// IDLE failures do not count towards the breaker: a connection that a
    /// router drops after a few idle minutes says nothing about fetching.
    pub async fn wait_for_push(&self, max: std::time::Duration) -> Option<bool> {
        use crate::transport::{Job, Timer, Via};
        let source = self.backfill.read().unwrap_or_else(|e| e.into_inner()).clone();
        self.imap_gate(source.as_ref()).ok()?;
        let timer = Timer::start();
        match source.watch(max).await {
            Ok(Some(changed)) => {
                self.transport.record(timer.finish(Job::Push, Via::Imap, None, usize::from(changed), true));
                Some(changed)
            }
            Ok(None) => None,
            Err(e) => {
                tracing::debug!(error = %e, "IMAP IDLE ended");
                self.transport.record(timer.finish(Job::Push, Via::Imap, Some(e.to_string()), 0, false));
                None
            }
        }
    }

    /// Inbox categories for stored mail. Gmail's IMAP leaves categories out
    /// of a message's labels, so mail downloaded over IMAP would all look
    /// like Primary: each category is listed as a Gmail search
    /// (`in:inbox category:promotions`, over IMAP; the API as fallback)
    /// and stored Inbox messages missing it get it, losing any other
    /// category. Only adds what is listed: mail that moved to Primary
    /// elsewhere arrives through the change history. Returns how many
    /// messages changed.
    pub async fn sync_categories(&self) -> SyncResult<usize> {
        use crate::transport::{Job, Timer};
        let mut listed: Vec<(LabelId, Vec<MessageId>)> = Vec::new();
        for category in read::CATEGORIES {
            let search = format!("in:inbox category:{}", category.trim_start_matches("CATEGORY_").to_ascii_lowercase());
            let ids = match self.list_via_imap(&search, Job::Categories).await {
                Ok(ids) => ids,
                Err(reason) => {
                    let filter = ListFilter {
                        label_ids: vec![LabelId::new(system_labels::INBOX), LabelId::new(*category)],
                        query: None,
                        include_spam_trash: false,
                    };
                    let mut ids = Vec::new();
                    let mut page = None;
                    loop {
                        let timer = Timer::start();
                        let result = self.provider.list_message_ids(&filter, page.take()).await;
                        let count = result.as_ref().map_or(0, |r| r.ids.len());
                        self.record_api(&timer, Job::Categories, &reason, count, result.is_ok());
                        let result = result?;
                        ids.extend(result.ids.into_iter().map(|(id, _)| id));
                        match result.next {
                            Some(next) => page = Some(next),
                            None => break,
                        }
                    }
                    ids
                }
            };
            listed.push((LabelId::new(*category), ids));
        }
        let changes = self
            .db
            .write(move |tx| {
                let mut wanted: Vec<(MessageId, LabelId)> = Vec::new();
                for (category, ids) in &listed {
                    for id in read::inbox_messages_missing(tx, ids, category)? {
                        wanted.push((id, category.clone()));
                    }
                }
                let mut w = MailWriter::new(tx);
                for (id, category) in &wanted {
                    let others: Vec<LabelId> = read::CATEGORIES
                        .iter()
                        .filter(|c| **c != category.as_str())
                        .map(|c| LabelId::new(*c))
                        .collect();
                    w.modify_message_labels(id, std::slice::from_ref(category), &others)?;
                }
                Ok((wanted.len(), w.finish()?))
            })
            .await?;
        let (changed, thread_changes) = changes;
        self.publish(&thread_changes);
        if changed > 0 {
            tracing::info!(changed, "inbox categories applied");
        }
        Ok(changed)
    }

    /// Measure each job both ways for the Sync Debugger
    /// (docs/plans/imap-first-sync.md, "Measure before deciding"): list
    /// ids, fetch headers, fetch bodies, and read changes, over IMAP and
    /// over the API. Reads only: nothing is stored, no mail changes, and
    /// the breaker and the operation log are left alone. IMAP bytes count
    /// towards the day's budget, as they are real downloads.
    pub async fn compare_transports(
        &self,
        sizes: crate::transport::ComparisonSizes,
    ) -> SyncResult<Vec<crate::transport::Comparison>> {
        use crate::transport::{Comparison, Job, Via};
        async fn timed<T, F: std::future::Future<Output = Result<T, String>>>(f: F) -> (Result<T, String>, u64) {
            let started = std::time::Instant::now();
            let result = f.await;
            (result, started.elapsed().as_millis() as u64)
        }
        fn row<T>(
            job: Job,
            via: Via,
            note: Option<&str>,
            (result, millis): &(Result<T, String>, u64),
            count: impl Fn(&T) -> usize,
        ) -> Comparison {
            Comparison {
                job,
                via,
                note: note.map(str::to_owned),
                millis: *millis,
                items: result.as_ref().map_or(0, count),
                error: result.as_ref().err().cloned(),
            }
        }
        let source = self.backfill.read().unwrap_or_else(|e| e.into_inner()).clone();
        let imap_off = self.imap_gate(source.as_ref()).err();
        let unavailable =
            |job: Job| Comparison { job, via: Via::Imap, note: None, millis: 0, items: 0, error: imap_off.clone() };
        let mut rows = Vec::new();

        // Listing.
        let api_list = timed(async {
            let mut ids = Vec::new();
            let mut page = None;
            loop {
                let listed = self
                    .provider
                    .list_message_ids(&ListFilter::default(), page.take())
                    .await
                    .map_err(|e| e.to_string())?;
                ids.extend(listed.ids.into_iter().map(|(id, _)| id));
                match listed.next {
                    Some(next) if ids.len() < sizes.list => page = Some(next),
                    _ => return Ok(ids),
                }
            }
        })
        .await;
        let list_note = format!("API: up to {} ids, 500 a page; IMAP: all of All Mail in one search", sizes.list);
        rows.push(row(Job::List, Via::Api, Some(&list_note), &api_list, Vec::len));
        let imap_list = if imap_off.is_some() {
            None
        } else {
            let listed =
                timed(async { source.list("").await.map_err(|e| e.to_string())?.ok_or_else(|| "cannot list".into()) })
                    .await;
            rows.push(row(Job::List, Via::Imap, Some(&list_note), &listed, Vec::len));
            listed.0.ok()
        };
        if imap_list.is_none() && imap_off.is_some() {
            rows.push(unavailable(Job::List));
        }
        let sample: Vec<MessageId> = match (&api_list.0, imap_list) {
            (Ok(ids), _) if !ids.is_empty() => ids.clone(),
            (_, Some(ids)) => ids,
            _ => Vec::new(),
        };

        // Headers: the API has no cheaper headers-only fetch here.
        let head: Vec<MessageId> = sample.iter().take(sizes.headers).cloned().collect();
        if imap_off.is_some() {
            rows.push(unavailable(Job::Headers));
        } else {
            let fetched = timed(async {
                source.fetch_headers(&head).await.map_err(|e| e.to_string())?.ok_or_else(|| "no headers".into())
            })
            .await;
            rows.push(row(Job::Headers, Via::Imap, None, &fetched, Vec::len));
        }
        rows.push(Comparison {
            job: Job::Headers,
            via: Via::Api,
            note: Some("the API charges as much for headers as for whole messages".into()),
            millis: 0,
            items: 0,
            error: Some("not measured".into()),
        });

        // Bodies.
        let bodies: Vec<MessageId> = sample.iter().take(sizes.bodies).cloned().collect();
        let body_note = "IMAP sends messages over its size cap through the API";
        let api_bodies = timed(async {
            self.provider.fetch_messages(&bodies, Priority::Background).await.map_err(|e| e.to_string())
        })
        .await;
        rows.push(row(Job::Bodies, Via::Api, None, &api_bodies, Vec::len));
        if imap_off.is_some() {
            rows.push(unavailable(Job::Bodies));
        } else {
            let fetched = timed(async { source.fetch(&bodies).await.map_err(|e| e.to_string()) }).await;
            rows.push(row(Job::Bodies, Via::Imap, Some(body_note), &fetched, Vec::len));
        }

        // Changes: the API's history against re-reading labels and flags.
        let cursor = self.db.read(|c| read::sync_state(c, KEY_CURSOR)).await?;
        let api_changes = timed(async {
            let cursor = cursor.ok_or_else(|| "not synced yet".to_owned())?;
            self.provider.changes_since(&provider_api::SyncCursor(cursor)).await.map_err(|e| e.to_string())
        })
        .await;
        rows.push(row(Job::Changes, Via::Api, Some("history since the last sync"), &api_changes, |s| s.changes.len()));
        if imap_off.is_some() {
            rows.push(unavailable(Job::Changes));
        } else {
            let limit = sizes.changes;
            let reread = timed(async {
                let ids = source
                    .list("newer_than:30d")
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "cannot list".to_owned())?;
                let ids: Vec<MessageId> = ids.into_iter().take(limit).collect();
                source.fetch_headers(&ids).await.map_err(|e| e.to_string())?.ok_or_else(|| "no headers".into())
            })
            .await;
            let note =
                format!("no change log over IMAP: labels and flags re-read for up to {limit} of the last 30 days");
            rows.push(row(Job::Changes, Via::Imap, Some(&note), &reread, Vec::len));
        }
        Ok(rows)
    }

    /// Note an IMAP error: a failure counts towards the breaker; a source
    /// that is unavailable by design (refused, budget used) does not.
    fn imap_error(&self, e: &provider_api::ProviderError) -> String {
        match e {
            provider_api::ProviderError::Unavailable(why) => why.clone(),
            other => self.transport.imap_failed(&other.to_string(), crate::outbox::now_millis()),
        }
    }

    /// Use another source for backfill bodies (spec §7.4, IMAP amendment).
    pub fn set_backfill_source(&self, source: Arc<dyn BackfillSource>) {
        tracing::info!(source = source.name(), "backfill source set");
        *self.backfill.write().unwrap_or_else(|e| e.into_inner()) = source;
    }

    /// Go back to fetching bodies over the provider's API.
    pub fn use_rest_backfill(&self) {
        self.set_backfill_source(Arc::new(RestBackfill(self.provider.clone())));
    }

    /// Whether the backfill source fetches headers cheaply now (IMAP, not
    /// refused): older mail can then come down as headers only.
    pub fn cheap_headers(&self) -> bool {
        self.backfill.read().unwrap_or_else(|e| e.into_inner()).cheap_headers()
    }

    /// The backfill source's name, for diagnostics.
    pub fn backfill_source_name(&self) -> &'static str {
        self.backfill.read().unwrap_or_else(|e| e.into_inner()).name()
    }

    /// True until the first bootstrap has listed every phase.
    pub async fn needs_bootstrap(&self) -> SyncResult<bool> {
        Ok(self.db.read(|c| read::sync_state(c, KEY_BOOTSTRAPPED)).await?.is_none())
    }

    pub async fn account_email(&self) -> SyncResult<Option<String>> {
        Ok(self.db.read(|c| read::sync_state(c, KEY_EMAIL)).await?)
    }

    /// Bootstrap step 1 (fast): refresh labels, record the history cursor
    /// before listing anything, and queue the inbox phases. Incremental sync
    /// may run as soon as this returns.
    pub async fn bootstrap_prepare(&self) -> SyncResult<()> {
        self.refresh_labels().await?;
        let profile = self.provider.profile().await?;
        let cursor = profile.cursor.0.clone();
        let email = profile.email.clone();
        self.db
            .write(move |tx| {
                read::set_sync_state(tx, KEY_EMAIL, &email)?;
                // Keep an existing cursor on resume; a fresh one on first run.
                if read::sync_state(tx, KEY_CURSOR)?.is_none() {
                    read::set_sync_state(tx, KEY_CURSOR, &cursor)?;
                }
                Ok(())
            })
            .await?;
        let window = self.window().await?;
        let headers_from = self.headers_from().await?;
        // Record the window so a later default change does not widen it,
        // and the tiering the queue is listed under.
        self.db
            .write(move |tx| {
                read::set_sync_state(tx, KEY_WINDOW, window.as_str())?;
                read::set_sync_state(tx, KEY_TIERS, &tiers_tag(headers_from))
            })
            .await?;
        let phases = phases_for(window);
        for phase in &phases[..INBOX_PHASES] {
            self.list_phase_with(phase, false, headers_from).await?;
        }
        self.report(SyncPhase::Listing).await;
        Ok(())
    }

    /// An account synced before windows existed has none recorded: apply
    /// the default once, which trims its queue to the window.
    pub async fn ensure_window(&self) -> SyncResult<()> {
        if self.db.read(|c| read::sync_state(c, KEY_WINDOW)).await?.is_none() {
            tracing::info!(window = SyncWindow::default().as_str(), "applying the default sync window");
            self.set_window(SyncWindow::default()).await?;
        }
        Ok(())
    }

    /// Bootstrap step 2 (slow, can run alongside backfill): queue the rest.
    pub async fn bootstrap_list_rest(&self) -> SyncResult<()> {
        let phases = phases_for(self.window().await?);
        let headers_from = self.headers_from().await?;
        for phase in &phases[INBOX_PHASES..] {
            self.list_phase_with(phase, false, headers_from).await?;
        }
        self.db.write(|tx| read::set_sync_state(tx, KEY_BOOTSTRAPPED, "1")).await?;
        self.report(SyncPhase::Backfilling).await;
        Ok(())
    }

    /// How far back this account downloads mail.
    pub async fn window(&self) -> SyncResult<SyncWindow> {
        let stored = self.db.read(|c| read::sync_state(c, KEY_WINDOW)).await?;
        Ok(stored.as_deref().and_then(SyncWindow::parse).unwrap_or_default())
    }

    /// Change the window. Queued fetches beyond the shared phases are
    /// dropped and the window's own phases re-listed, so widening
    /// downloads more and narrowing stops downloading older mail. Mail
    /// already stored is kept either way.
    pub async fn set_window(&self, window: SyncWindow) -> SyncResult<()> {
        self.db.write(move |tx| read::set_sync_state(tx, KEY_WINDOW, window.as_str())).await?;
        self.relist_window().await
    }

    /// Download every header (Clean Up, spec §14.12): the window becomes
    /// *Everything* and the body window keeps bodies where they were
    /// ([`BodyWindow::kept_from`]), so with cheap headers the older mail
    /// is listed for headers only. Returns false when the window was
    /// *Everything* already (nothing changes).
    pub async fn load_every_header(&self) -> SyncResult<bool> {
        let window = self.window().await?;
        if window == SyncWindow::Everything {
            return Ok(false);
        }
        let body = self.body_window().await?.kept_from(window);
        self.db
            .write(move |tx| {
                read::set_sync_state(tx, KEY_BODY_WINDOW, body.as_str())?;
                read::set_sync_state(tx, KEY_WINDOW, SyncWindow::Everything.as_str())
            })
            .await?;
        self.relist_window().await?;
        Ok(true)
    }

    /// Which part of the window gets full messages when headers are cheap.
    pub async fn body_window(&self) -> SyncResult<BodyWindow> {
        let stored = self.db.read(|c| read::sync_state(c, KEY_BODY_WINDOW)).await?;
        Ok(stored.as_deref().and_then(BodyWindow::parse).unwrap_or_default())
    }

    /// Change the body window. Widening queues bodies for header-only mail
    /// now inside it; narrowing stops fetching bodies outside it (bodies
    /// already stored are kept).
    pub async fn set_body_window(&self, body_window: BodyWindow) -> SyncResult<()> {
        self.db.write(move |tx| read::set_sync_state(tx, KEY_BODY_WINDOW, body_window.as_str())).await?;
        self.ensure_tiers().await
    }

    /// The first phase priority listed for headers only, or `None` when
    /// every phase gets bodies: the source cannot fetch headers cheaply
    /// (REST), or the body window is the whole sync window.
    async fn headers_from(&self) -> SyncResult<Option<u8>> {
        let cheap = self.backfill.read().unwrap_or_else(|e| e.into_inner()).cheap_headers();
        Ok(if cheap { self.body_window().await?.headers_from() } else { None })
    }

    /// Re-list the window's own phases if the queue was listed under
    /// another tiering (IMAP turned on or off, the body window changed, or
    /// an account from before tiers). Called when sync starts.
    pub async fn ensure_tiers(&self) -> SyncResult<()> {
        let want = tiers_tag(self.headers_from().await?);
        let have = self.db.read(|c| read::sync_state(c, KEY_TIERS)).await?;
        // A queue from before tiers was listed flat.
        if have.as_deref().unwrap_or("flat") != want {
            tracing::info!(tiers = %want, "re-listing the sync window for tiered download");
            self.relist_window().await?;
        }
        self.db.write(move |tx| read::set_sync_state(tx, KEY_TIERS, &want)).await?;
        Ok(())
    }

    /// Drop queued fetches beyond the shared phases and list the window's
    /// own phases again under the current window and tiering.
    async fn relist_window(&self) -> SyncResult<()> {
        let bootstrapped = self
            .db
            .write(move |tx| {
                queue::clear_from_priority(tx, WINDOW_PRIORITY)?;
                read::sync_state(tx, KEY_BOOTSTRAPPED)
            })
            .await?
            .is_some();
        if bootstrapped {
            let headers_from = self.headers_from().await?;
            for phase in &phases_for(self.window().await?)[FIXED_PHASES..] {
                self.list_phase_with(phase, false, headers_from).await?;
            }
            self.db.write(move |tx| read::set_sync_state(tx, KEY_TIERS, &tiers_tag(headers_from))).await?;
            self.report(SyncPhase::Backfilling).await;
        }
        Ok(())
    }

    /// Fetch and store up to `max` queued messages, most urgent first.
    /// Returns how many were processed (0 when the queue is empty).
    pub async fn backfill_batch(&self, max: usize) -> SyncResult<usize> {
        let ids = self.db.read(move |c| queue::peek(c, max)).await?;
        if ids.is_empty() {
            return Ok(0);
        }
        tracing::debug!(count = ids.len(), "backfill batch: fetching");
        let fetched = self.fetch_bodies(&ids, Priority::Background).await?;
        tracing::debug!(count = fetched.len(), "backfill batch: storing");
        let incoming: Vec<_> = fetched.into_iter().map(to_incoming).collect();
        let processed = ids.len();
        let keep_labels = self.provider.labels_are_local();
        let changes = self
            .db
            .write(move |tx| {
                let mut w = MailWriter::new(tx).keeping_labels(keep_labels);
                for m in &incoming {
                    w.upsert_message(m)?;
                }
                // Ids the provider no longer has are dropped from the queue too.
                queue::remove(tx, &ids)?;
                w.finish()
            })
            .await?;
        self.publish(&changes);
        self.report(SyncPhase::Backfilling).await;
        Ok(processed)
    }

    /// Queue every message in the window again, fetched anew even if
    /// stored (labels and bodies come back current). The user's repair
    /// button; also useful after a bug in a fetch path.
    pub async fn refetch_all(&self) -> SyncResult<u64> {
        let headers_from = self.headers_from().await?;
        for phase in &phases_for(self.window().await?) {
            self.list_phase_with(phase, true, headers_from).await?;
        }
        let queued = self.db.read(queue::len).await?;
        self.report(SyncPhase::Backfilling).await;
        Ok(queued)
    }

    /// Headers-first (spec §7.4 IMAP amendment): store header-only rows for
    /// up to `max` queued messages that have no row yet, so the list is
    /// browsable before their bodies arrive (they stay queued for bodies),
    /// then for the headers-only tier (tiered download), which leaves the
    /// queue. Returns the progress made (rows stored plus headers-only ids
    /// finished); 0 when the source cannot do it now or nothing is left.
    pub async fn headers_pass(&self, max: usize) -> SyncResult<usize> {
        let missed = self.headers_missed.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let skip = missed.len();
        let ids: Vec<MessageId> = self
            .db
            .read(move |c| queue::for_headers(c, max + skip))
            .await?
            .into_iter()
            .filter(|id| !missed.contains(id))
            .take(max)
            .collect();
        if ids.is_empty() {
            return Ok(0);
        }
        let source = self.backfill.read().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(headers) = self.fetch_headers(&ids).await? else {
            if !source.cheap_headers() {
                // Headers cost as much as bodies now (IMAP refused): fetch
                // the headers-only tier in full rather than never.
                let promoted = self.db.write(queue::promote_headers_only).await?;
                if promoted > 0 {
                    tracing::info!(promoted, "headers-only messages queued for bodies");
                }
            }
            return Ok(0);
        };
        let incoming: Vec<_> = headers.into_iter().map(to_incoming).collect();
        let returned: std::collections::HashSet<MessageId> = incoming.iter().map(|m| m.id.clone()).collect();
        let stored = incoming.len();
        let requested = ids.clone();
        let keep_labels = self.provider.labels_are_local();
        let (changes, dropped) = self
            .db
            .write(move |tx| {
                let mut w = MailWriter::new(tx).keeping_labels(keep_labels);
                for m in &incoming {
                    w.upsert_message(m)?;
                }
                // Headers-only ids are done, found or not (a message the
                // source no longer has is not coming back).
                let dropped = queue::remove_headers_only(tx, &requested)?;
                Ok((w.finish()?, dropped))
            })
            .await?;
        // Ids waiting for bodies that the source did not return stay for
        // the body backfill; this pass stops asking for them.
        {
            let mut missed = self.headers_missed.lock().unwrap_or_else(|e| e.into_inner());
            missed.extend(ids.into_iter().filter(|id| !returned.contains(id)));
        }
        self.publish(&changes);
        self.report(SyncPhase::Backfilling).await;
        // Progress, not ids asked for: a pass that stored nothing and
        // dropped nothing must end the loop.
        Ok(stored + dropped)
    }

    /// Search Gmail itself for `query` and download up to `max` matching
    /// messages this store does not have in full (mail outside the sync
    /// window, spec §7.4 follow-up; header-only mail, tiered download).
    /// Interactive: the user is waiting. Returns how many were downloaded.
    pub async fn search_server(&self, query: &str, max: usize) -> SyncResult<usize> {
        // Over IMAP first (Gmail's search syntax, no quota), newest first.
        let ids = match self.list_via_imap(query, crate::transport::Job::Search).await {
            Ok(ids) => ids.into_iter().take(max).collect(),
            Err(reason) => {
                let filter = ListFilter { label_ids: vec![], query: Some(query.to_owned()), include_spam_trash: false };
                let timer = crate::transport::Timer::start();
                let found = self.provider.list_message_ids(&filter, None).await;
                let count = found.as_ref().map_or(0, |p| p.ids.len());
                self.record_api(&timer, crate::transport::Job::Search, &reason, count, found.is_ok());
                found?.ids.into_iter().map(|(id, _)| id).take(max).collect::<Vec<MessageId>>()
            }
        };
        // Matches stored with headers only get their bodies too (tiered
        // download), the same way an opened message does.
        self.ensure_bodies(ids).await
    }

    /// Download these messages' bodies now if they are not stored in full
    /// (spec §7.4 tiered download: a header-only message the user opens or
    /// an agent reads). Over IMAP when the source is IMAP, else over the
    /// API at interactive priority. Returns how many were downloaded.
    pub async fn ensure_bodies(&self, ids: Vec<MessageId>) -> SyncResult<usize> {
        let missing = self.db.read(move |c| queue::missing(c, &ids)).await?;
        if missing.is_empty() {
            return Ok(0);
        }
        let fetched = self.fetch_bodies(&missing, Priority::Interactive).await?;
        let incoming: Vec<_> = fetched.into_iter().filter(|m| m.body.is_some()).map(to_incoming).collect();
        let count = incoming.len();
        let keep_labels = self.provider.labels_are_local();
        let changes = self
            .db
            .write(move |tx| {
                let mut w = MailWriter::new(tx).keeping_labels(keep_labels);
                for m in &incoming {
                    w.upsert_message(m)?;
                }
                let done: Vec<MessageId> = incoming.iter().map(|m| m.id.clone()).collect();
                queue::remove(tx, &done)?;
                w.finish()
            })
            .await?;
        self.publish(&changes);
        self.report(SyncPhase::Backfilling).await;
        Ok(count)
    }

    /// Fetch these messages next (the user opened one whose body is not
    /// here yet).
    pub async fn prioritize(&self, ids: Vec<MessageId>) -> SyncResult<()> {
        self.db.write(move |tx| queue::enqueue_urgent(tx, &ids)).await?;
        Ok(())
    }

    /// Drain the queue completely (tests and small mailboxes).
    pub async fn backfill_all(&self) -> SyncResult<usize> {
        let mut total = 0;
        loop {
            let n = self.backfill_batch(BACKFILL_BATCH).await?;
            if n == 0 {
                return Ok(total);
            }
            total += n;
        }
    }

    /// Drafts through the provider's drafts list, since Gmail's change
    /// history leaves them out (spec §14.5 amendment 2026-09-28): every
    /// draft's message is stored in full whatever the window, draft
    /// messages whose draft is gone (sent or discarded elsewhere) are
    /// removed, and which draft holds which message is recorded for
    /// editing. Returns how many draft messages were downloaded.
    pub async fn sync_drafts(&self) -> SyncResult<usize> {
        let timer = crate::transport::Timer::start();
        let drafts = self.provider.list_drafts().await;
        let count = drafts.as_ref().ok().and_then(|d| d.as_ref()).map_or(0, Vec::len);
        self.record_api(
            &timer,
            crate::transport::Job::Drafts,
            "draft ids come only from the API",
            count,
            drafts.is_ok(),
        );
        let Some(listed) = drafts? else { return Ok(0) };
        let held: Vec<String> = listed.iter().map(|(_, m)| m.0.clone()).collect();
        let (missing, stale) = self
            .db
            .read({
                let held = held.clone();
                move |c| {
                    let mut full =
                        c.prepare_cached("SELECT 1 FROM messages WHERE gmail_id = ?1 AND body_state = 'full'")?;
                    let mut missing = Vec::new();
                    for id in &held {
                        if !full.exists([id])? {
                            missing.push(MessageId(id.clone()));
                        }
                    }
                    let mut drafts = c.prepare_cached("SELECT gmail_id FROM messages WHERE is_draft")?;
                    let stored = drafts.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                    let stale: Vec<MessageId> =
                        stored.into_iter().filter(|id| !held.contains(id)).map(MessageId).collect();
                    Ok((missing, stale))
                }
            })
            .await?;
        // Draft messages over IMAP like any other (All Mail or Drafts).
        let fetched =
            if missing.is_empty() { vec![] } else { self.fetch_bodies(&missing, Priority::Background).await? };
        let downloaded = fetched.len();
        let incoming: Vec<_> = fetched.into_iter().map(to_incoming).collect();
        let pairs: Vec<(String, String)> = listed.into_iter().map(|(d, m)| (d, m.0)).collect();
        let keep_labels = self.provider.labels_are_local();
        let changes = self
            .db
            .write(move |tx| {
                let mut w = MailWriter::new(tx).keeping_labels(keep_labels);
                for m in &incoming {
                    w.upsert_message(m)?;
                }
                for id in &stale {
                    w.delete_message(id)?;
                }
                let changes = w.finish()?;
                mail_store::drafts::replace_server_drafts(tx, &pairs)?;
                Ok(changes)
            })
            .await?;
        self.publish(&changes);
        Ok(downloaded)
    }

    /// Apply the provider's history since the stored cursor. On an expired
    /// cursor the store is queued for a full resync and
    /// [`SyncError::ResyncStarted`] is returned.
    pub async fn sync_incremental(&self) -> SyncResult<IncrementalReport> {
        let Some(cursor) = self.db.read(|c| read::sync_state(c, KEY_CURSOR)).await? else {
            return Err(SyncError::NotBootstrapped);
        };
        // Push local intent first so history does not appear to undo it.
        self.drain_outbox().await?;
        let timer = crate::transport::Timer::start();
        let changed = self.provider.changes_since(&provider_api::SyncCursor(cursor)).await;
        self.record_api(
            &timer,
            crate::transport::Job::Changes,
            "faster over the API: Gmail's IMAP keeps no change log",
            changed.as_ref().map_or(0, |s| s.changes.len()),
            changed.is_ok(),
        );
        let set = match changed {
            Ok(set) => set,
            Err(ProviderError::CursorExpired) => {
                self.start_resync().await?;
                return Err(SyncError::ResyncStarted);
            }
            Err(e) => return Err(e.into()),
        };

        // Fetch every message added since the cursor (and any we are told
        // about but do not have), then apply all changes in one transaction.
        let mut to_fetch: BTreeSet<MessageId> = BTreeSet::new();
        for change in &set.changes {
            if let Change::MessageAdded { id, .. } = change {
                to_fetch.insert(id.clone());
            }
        }
        let deleted: BTreeSet<MessageId> = set
            .changes
            .iter()
            .filter_map(|c| match c {
                Change::MessageDeleted { id } => Some(id.clone()),
                _ => None,
            })
            .collect();
        to_fetch.retain(|id| !deleted.contains(id));
        let fetch_ids: Vec<MessageId> = to_fetch.into_iter().collect();
        let fetched = if fetch_ids.is_empty() {
            vec![]
        } else {
            self.provider.fetch_messages(&fetch_ids, Priority::Background).await?
        };
        let incoming: Vec<_> = fetched.into_iter().map(to_incoming).collect();
        let new_cursor = set.cursor.0.clone();
        let changes_in = set.changes;

        let keep_labels = self.provider.labels_are_local();
        let (changes, report) = self
            .db
            .write(move |tx| {
                let mut report = IncrementalReport::default();
                let mut w = MailWriter::new(tx).keeping_labels(keep_labels);
                for m in &incoming {
                    let stored =
                        tx.prepare_cached("SELECT 1 FROM messages WHERE gmail_id = ?1")?.exists([m.id.as_str()])?;
                    report.new_mail.extend(NewMail::from_incoming(m, stored));
                    w.upsert_message(m)?;
                    report.added += 1;
                }
                let fetched_ids: BTreeSet<&str> = incoming.iter().map(|m| m.id.as_str()).collect();
                let mut unknown: Vec<MessageId> = Vec::new();
                let mut external: Vec<(MessageId, Vec<LabelId>, bool)> = Vec::new();
                for change in &changes_in {
                    match change {
                        Change::MessageAdded { .. } => {}
                        Change::MessageDeleted { id } => {
                            if w.delete_message(id)? {
                                report.deleted += 1;
                            }
                            queue::remove(tx, std::slice::from_ref(id))?;
                        }
                        // A freshly fetched message already has current labels.
                        Change::LabelsAdded { id, .. } | Change::LabelsRemoved { id, .. }
                            if fetched_ids.contains(id.as_str()) => {}
                        Change::LabelsAdded { id, label_ids } => {
                            if w.modify_message_labels(id, label_ids, &[])? {
                                report.relabeled += 1;
                                external.push((id.clone(), label_ids.clone(), true));
                            } else {
                                unknown.push(id.clone());
                            }
                        }
                        Change::LabelsRemoved { id, label_ids } => {
                            if w.modify_message_labels(id, &[], label_ids)? {
                                report.relabeled += 1;
                                external.push((id.clone(), label_ids.clone(), false));
                            } else {
                                unknown.push(id.clone());
                            }
                        }
                    }
                }
                // Label changes for messages not stored yet: make sure a
                // backfill will fetch them with current labels.
                queue::enqueue_urgent(tx, &unknown)?;
                read::set_sync_state(tx, KEY_CURSOR, &new_cursor)?;
                // Thread ids for the label changes, while we hold the store.
                let mut threads: Vec<(MessageId, ThreadId, Vec<LabelId>, bool)> = Vec::new();
                for (id, labels, added) in external {
                    if let Some(m) = read::get_message(tx, &id)? {
                        threads.push((id, m.thread_id, labels, added));
                    }
                }
                Ok((w.finish()?, (report, threads)))
            })
            .await?;
        let (mut report, labeled) = report;
        report.external_label_changes = self.not_ours(labeled);
        self.publish(&changes);
        // Drafts are not in the history; a failure here waits for the next
        // round rather than failing this one.
        if let Err(e) = self.sync_drafts().await {
            tracing::warn!(error = %e, "draft sync failed; retried next round");
        }
        self.report(SyncPhase::Incremental).await;
        Ok(report)
    }

    /// Drop the changes OpenAGC's outbox made; group the rest per message.
    fn not_ours(&self, labeled: Vec<(MessageId, ThreadId, Vec<LabelId>, bool)>) -> Vec<ExternalLabelChange> {
        let now = crate::outbox::now_millis();
        let mut own = self.own_changes.lock().unwrap_or_else(|e| e.into_inner());
        own.retain(|c| now - c.at < OWN_CHANGE_WINDOW);
        // Sets, not scans: a bulk change (Clean Up) comes back from history
        // as tens of thousands of label changes.
        let ours: std::collections::HashSet<(&MessageId, &LabelId, bool)> =
            own.iter().map(|c| (&c.message, &c.label, c.added)).collect();
        let mut out: Vec<ExternalLabelChange> = Vec::new();
        let mut index: std::collections::HashMap<MessageId, usize> = std::collections::HashMap::new();
        for (message, thread, labels, added) in labeled {
            let theirs: Vec<LabelId> = labels.into_iter().filter(|l| !ours.contains(&(&message, l, added))).collect();
            if theirs.is_empty() {
                continue;
            }
            let i = *index.entry(message.clone()).or_insert_with(|| {
                out.push(ExternalLabelChange { message, thread, added: vec![], removed: vec![] });
                out.len() - 1
            });
            let entry = &mut out[i];
            if added { entry.added.extend(theirs) } else { entry.removed.extend(theirs) }
        }
        out
    }

    /// Remember label changes the outbox just pushed.
    pub(crate) fn remember_own(&self, messages: &[MessageId], add: &[LabelId], remove: &[LabelId]) {
        let at = crate::outbox::now_millis();
        let mut own = self.own_changes.lock().unwrap_or_else(|e| e.into_inner());
        for m in messages {
            own.extend(add.iter().map(|l| OwnChange { message: m.clone(), label: l.clone(), added: true, at }));
            own.extend(remove.iter().map(|l| OwnChange { message: m.clone(), label: l.clone(), added: false, at }));
        }
    }

    /// Label list changes are not in history; refresh them wholesale.
    ///
    /// When labels live on this Mac ([`MailProvider::labels_are_local`]),
    /// the user labels in the store are the truth: the provider's list only
    /// adds labels that are missing, and nothing the user made is dropped
    /// or renamed. Otherwise the provider's list is the truth (Gmail).
    pub async fn refresh_labels(&self) -> SyncResult<()> {
        let local = self.provider.labels_are_local();
        let mut labels = self.provider.list_labels().await?;
        let mut keep: Vec<LabelId> = labels.iter().map(|l| l.id.clone()).collect();
        let changes = self
            .db
            .write(move |tx| {
                if local {
                    let user: BTreeSet<LabelId> = read::list_labels(tx)?
                        .into_iter()
                        .filter(|l| l.kind == mail_domain::LabelKind::User)
                        .map(|l| l.id)
                        .collect();
                    labels.retain(|l| !user.contains(&l.id));
                    keep.extend(user);
                }
                let mut w = MailWriter::new(tx);
                w.upsert_labels(&labels)?;
                w.retain_labels(&keep)?;
                w.finish()
            })
            .await?;
        self.publish(&changes);
        Ok(())
    }

    /// The history cursor expired: take a fresh cursor and re-list
    /// everything. Stored messages are refetched so labels are current.
    async fn start_resync(&self) -> SyncResult<()> {
        tracing::warn!("history cursor expired; starting full resync");
        let profile = self.provider.profile().await?;
        let cursor = profile.cursor.0;
        self.db
            .write(move |tx| {
                read::set_sync_state(tx, KEY_CURSOR, &cursor)?;
                tx.execute("DELETE FROM sync_state WHERE key = ?1", [KEY_BOOTSTRAPPED])?;
                Ok(())
            })
            .await?;
        let headers_from = self.headers_from().await?;
        for phase in &phases_for(self.window().await?) {
            self.list_phase_with(phase, true, headers_from).await?;
        }
        self.db.write(|tx| read::set_sync_state(tx, KEY_BOOTSTRAPPED, "1")).await?;
        Ok(())
    }

    async fn list_phase_with(&self, phase: &Phase, refetch: bool, headers_from: Option<u8>) -> SyncResult<()> {
        let priority = queue_priority(phase, headers_from);
        let filter = ListFilter {
            label_ids: phase.labels.iter().map(|l| LabelId::new(*l)).collect(),
            query: phase.query.clone(),
            include_spam_trash: phase.labels.iter().any(|l| matches!(*l, "SPAM" | "TRASH")),
        };
        // Over IMAP first: the same phase as a Gmail search, no quota.
        let reason = match self.list_via_imap(&phase_search(phase), crate::transport::Job::List).await {
            Ok(ids) => {
                self.db.write(move |tx| queue::enqueue(tx, priority, &ids, refetch)).await?;
                return Ok(());
            }
            Err(reason) => reason,
        };
        let mut page: Option<PageToken> = None;
        loop {
            tracing::debug!(priority, has_page = page.is_some(), "listing phase page");
            let timer = crate::transport::Timer::start();
            let listed = self.provider.list_message_ids(&filter, page.take()).await;
            let count = listed.as_ref().map_or(0, |r| r.ids.len());
            self.record_api(&timer, crate::transport::Job::List, &reason, count, listed.is_ok());
            let result = listed?;
            let ids: Vec<MessageId> = result.ids.into_iter().map(|(id, _)| id).collect();
            self.db.write(move |tx| queue::enqueue(tx, priority, &ids, refetch)).await?;
            match result.next {
                Some(next) => page = Some(next),
                None => return Ok(()),
            }
        }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn provider(&self) -> &dyn MailProvider {
        self.provider.as_ref()
    }

    pub(crate) fn publish_changes(&self, changes: &ThreadChanges) {
        self.publish(changes);
    }

    fn publish(&self, changes: &ThreadChanges) {
        if !changes.is_empty() {
            self.observer.threads_changed(changes);
        }
    }

    async fn report(&self, phase: SyncPhase) {
        let (queued, headers) = self.db.read(queue::counts).await.unwrap_or((0, 0));
        let phase = if queued + headers == 0 && phase == SyncPhase::Backfilling { SyncPhase::Idle } else { phase };
        self.observer.progress(SyncProgress { phase, queued, headers });
    }
}
