//! Which transport serves each sync job, IMAP or the Gmail API
//! (docs/plans/imap-first-sync.md): IMAP by default; the API when it is
//! faster, when there is no other way, or when IMAP fails. IMAP failures
//! fall back per operation and trip a breaker after a few in a row, so a
//! flaky connection does not cost every batch a timeout. Every operation is
//! recorded for the Sync Debugger.

use std::collections::VecDeque;
use std::sync::Mutex;

use mail_domain::Millis;

/// A kind of sync work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Job {
    /// Listing message ids in the window.
    List,
    /// Headers (and snippets) without bodies.
    Headers,
    /// Whole messages.
    Bodies,
    /// Changes made elsewhere since the cursor.
    Changes,
    /// The drafts list and draft messages.
    Drafts,
    /// Searching Gmail for mail outside the store.
    Search,
    /// The user's changes going to the server (outbox).
    Write,
    /// Waiting for new mail (IMAP IDLE).
    Push,
    /// Which Inbox category each message is in (not in IMAP's labels).
    Categories,
}

impl Job {
    pub const ALL: [Job; 9] = [
        Job::List,
        Job::Headers,
        Job::Bodies,
        Job::Changes,
        Job::Drafts,
        Job::Search,
        Job::Write,
        Job::Push,
        Job::Categories,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Job::List => "list",
            Job::Headers => "headers",
            Job::Bodies => "bodies",
            Job::Changes => "changes",
            Job::Drafts => "drafts",
            Job::Search => "search",
            Job::Write => "write",
            Job::Push => "push",
            Job::Categories => "categories",
        }
    }
}

/// The transport that served an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Imap,
    Api,
}

impl Via {
    pub fn name(self) -> &'static str {
        match self {
            Via::Imap => "imap",
            Via::Api => "api",
        }
    }
}

/// One finished operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpRecord {
    pub job: Job,
    pub via: Via,
    /// Why the API served it, when IMAP would have been preferred or the
    /// job has no IMAP path yet.
    pub reason: Option<String>,
    pub at: Millis,
    pub millis: u64,
    pub items: usize,
    pub ok: bool,
}

/// After this many IMAP failures in a row the breaker opens…
pub const BREAKER_FAILURES: u32 = 3;
/// …for this long (or until the user asks for a refresh).
pub const BREAKER_OPEN_MS: Millis = 15 * 60 * 1000;
/// Operations kept for the Sync Debugger.
pub const RECORDS_KEPT: usize = 200;

#[derive(Debug, Default)]
struct State {
    failures: u32,
    open_until: Option<Millis>,
    last_error: Option<String>,
    records: VecDeque<OpRecord>,
}

/// A snapshot for diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransportSnapshot {
    /// When the breaker closes again, if it is open.
    pub breaker_open_until: Option<Millis>,
    pub consecutive_failures: u32,
    pub last_imap_error: Option<String>,
    /// Newest first.
    pub recent: Vec<OpRecord>,
}

impl TransportSnapshot {
    /// The latest record for each job, in `Job::ALL` order.
    pub fn latest_by_job(&self) -> Vec<OpRecord> {
        Job::ALL.iter().filter_map(|job| self.recent.iter().find(|r| r.job == *job).cloned()).collect()
    }
}

/// Per-account transport bookkeeping.
#[derive(Debug, Default)]
pub struct Transport {
    state: Mutex<State>,
}

impl Transport {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether IMAP may be tried now (the breaker is closed).
    pub fn imap_allowed(&self, now: Millis) -> bool {
        let mut s = self.state();
        match s.open_until {
            Some(until) if now < until => false,
            Some(_) => {
                // Half-open: one try; a failure opens it again at once.
                s.open_until = None;
                s.failures = BREAKER_FAILURES - 1;
                true
            }
            None => true,
        }
    }

    pub fn imap_succeeded(&self) {
        let mut s = self.state();
        s.failures = 0;
        s.open_until = None;
    }

    /// An IMAP failure; returns the reason to record with the fallback.
    pub fn imap_failed(&self, error: &str, now: Millis) -> String {
        let mut s = self.state();
        s.failures += 1;
        s.last_error = Some(error.to_owned());
        if s.failures >= BREAKER_FAILURES {
            s.open_until = Some(now + BREAKER_OPEN_MS);
            tracing::warn!(failures = s.failures, "IMAP failing; using the Gmail API for 15 minutes");
        }
        format!("IMAP failed: {error}")
    }

    /// The reason IMAP is skipped while the breaker is open.
    pub fn breaker_reason(&self) -> String {
        let s = self.state();
        match &s.last_error {
            Some(e) => format!("IMAP paused after repeated failures ({e})"),
            None => "IMAP paused after repeated failures".to_owned(),
        }
    }

    /// Close the breaker (the user asked for a refresh).
    pub fn reset(&self) {
        let mut s = self.state();
        s.failures = 0;
        s.open_until = None;
    }

    pub fn record(&self, record: OpRecord) {
        let mut s = self.state();
        if s.records.len() == RECORDS_KEPT {
            s.records.pop_back();
        }
        s.records.push_front(record);
    }

    pub fn snapshot(&self) -> TransportSnapshot {
        let s = self.state();
        TransportSnapshot {
            breaker_open_until: s.open_until,
            consecutive_failures: s.failures,
            last_imap_error: s.last_error.clone(),
            recent: s.records.iter().cloned().collect(),
        }
    }
}

/// How much the Sync Debugger's comparison reads each way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComparisonSizes {
    /// Ids listed over the API (IMAP lists the whole of All Mail in one
    /// search).
    pub list: usize,
    pub headers: usize,
    pub bodies: usize,
    /// Messages whose labels and flags IMAP re-reads for "changes".
    pub changes: usize,
}

impl Default for ComparisonSizes {
    fn default() -> Self {
        Self { list: 10_000, headers: 500, bodies: 500, changes: 5_000 }
    }
}

/// One measurement: a job done one way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub job: Job,
    pub via: Via,
    /// What exactly was measured, when it differs between the two ways.
    pub note: Option<String>,
    pub millis: u64,
    pub items: usize,
    /// Why it could not be measured, or the error it ended with.
    pub error: Option<String>,
}

/// Times an operation for its record.
pub(crate) struct Timer {
    started: std::time::Instant,
    at: Millis,
}

impl Timer {
    pub(crate) fn start() -> Self {
        Self { started: std::time::Instant::now(), at: crate::outbox::now_millis() }
    }

    pub(crate) fn finish(&self, job: Job, via: Via, reason: Option<String>, items: usize, ok: bool) -> OpRecord {
        OpRecord { job, via, reason, at: self.at, millis: self.started.elapsed().as_millis() as u64, items, ok }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_breaker_opens_after_repeated_failures_and_half_opens_later() {
        let t = Transport::default();
        assert!(t.imap_allowed(0));
        t.imap_failed("timeout", 0);
        t.imap_failed("timeout", 1);
        assert!(t.imap_allowed(2), "two failures: still trying");
        t.imap_failed("timeout", 2);
        assert!(!t.imap_allowed(3), "three in a row: paused");
        assert!(t.breaker_reason().contains("timeout"));
        assert!(t.imap_allowed(2 + BREAKER_OPEN_MS), "after the pause: one more try");
        t.imap_failed("timeout", 2 + BREAKER_OPEN_MS);
        assert!(!t.imap_allowed(3 + BREAKER_OPEN_MS), "and a failure pauses again at once");
        t.reset();
        assert!(t.imap_allowed(4 + BREAKER_OPEN_MS), "a refresh closes it");
        t.imap_failed("x", 5 + BREAKER_OPEN_MS);
        t.imap_succeeded();
        t.imap_failed("x", 6 + BREAKER_OPEN_MS);
        t.imap_failed("x", 7 + BREAKER_OPEN_MS);
        assert!(t.imap_allowed(8 + BREAKER_OPEN_MS), "a success resets the count");
    }

    #[test]
    fn records_are_kept_newest_first_and_bounded() {
        let t = Transport::default();
        for i in 0..(RECORDS_KEPT + 5) {
            t.record(OpRecord {
                job: if i % 2 == 0 { Job::Bodies } else { Job::Changes },
                via: Via::Api,
                reason: None,
                at: i as Millis,
                millis: 1,
                items: i,
                ok: true,
            });
        }
        let snap = t.snapshot();
        assert_eq!(snap.recent.len(), RECORDS_KEPT);
        assert_eq!(snap.recent[0].items, RECORDS_KEPT + 4);
        let latest = snap.latest_by_job();
        assert_eq!(latest.iter().map(|r| r.job).collect::<Vec<_>>(), [Job::Bodies, Job::Changes]);
    }
}
