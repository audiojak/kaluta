//! Tiered download at scale (spec §7.4): a two-year mailbox listed,
//! tiered, made browsable by the headers pass, and given its in-window
//! bodies. `five_thousand_messages…` runs in the gate as a regression
//! test; `benchmark_100k` is the measurement recorded in the spec:
//!
//! ```sh
//! cargo test --release -p mail-sync --test scale -- --ignored --nocapture
//! ```
//!
//! The source answers from the in-memory fake provider with cheap headers
//! (IMAP's cost model), so the times are the engine's and the store's,
//! not the network's.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use mail_domain::{EmailAddress, Label, LabelId, LabelKind, MessageId, ThreadId};
use mail_store::{ARCHIVE_LABEL, Db, ThreadChanges, consistency, read};
use mail_sync::{SyncEngine, SyncObserver, SyncProgress, SyncWindow};
use provider_api::fake::FakeProvider;
use provider_api::{FetchedBody, FetchedMessage};

const NOW: i64 = 1_790_000_000_000;
const DAY: i64 = 86_400_000;
const SPAN_DAYS: i64 = 730;
const PER_THREAD: usize = 3;

struct Quiet;

impl SyncObserver for Quiet {
    fn threads_changed(&self, _: &ThreadChanges) {}
    fn progress(&self, _: SyncProgress) {}
}

/// Bulk transport over the fake: counts bodies, headers are cheap.
struct Source(Arc<FakeProvider>, AtomicUsize);

#[async_trait::async_trait]
impl provider_api::BackfillSource for Source {
    async fn fetch(&self, ids: &[MessageId]) -> provider_api::ProviderResult<Vec<FetchedMessage>> {
        self.1.fetch_add(ids.len(), Ordering::SeqCst);
        use provider_api::MailProvider;
        self.0.fetch_messages(ids, provider_api::Priority::Background).await
    }
    async fn fetch_headers(&self, ids: &[MessageId]) -> provider_api::ProviderResult<Option<Vec<FetchedMessage>>> {
        use provider_api::MailProvider;
        let mut all = self.0.fetch_messages(ids, provider_api::Priority::Background).await?;
        for m in &mut all {
            m.body = None;
        }
        Ok(Some(all))
    }
    fn cheap_headers(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "scale"
    }
}

const PARAGRAPH: &str = "Thanks for the update on the plan. I have read through the notes and added a few comments \
    where the numbers did not match what we agreed last week; can you take a look before Thursday? ";

/// Message `i` of `n`: threads of three, newest first over two years; a
/// few percent of threads in the Inbox (more of the recent ones), a
/// quarter of those unread; every third message sent by me.
fn message(i: usize, n: usize) -> (FetchedMessage, bool, bool) {
    let thread = i / PER_THREAD;
    let age_ms = (i as i64) * SPAN_DAYS * DAY / n as i64;
    let recent = age_ms < 14 * DAY;
    let in_inbox = thread.is_multiple_of(25) || (recent && thread.is_multiple_of(3));
    let unread = in_inbox && thread.is_multiple_of(4);
    let from_me = i % PER_THREAD == 1;
    let mut labels = vec![];
    if from_me {
        labels.push("SENT");
    } else {
        if in_inbox {
            labels.push("INBOX");
        }
        if unread {
            labels.push("UNREAD");
        }
        labels.push(["CATEGORY_PERSONAL", "CATEGORY_UPDATES", "CATEGORY_PROMOTIONS"][thread % 3]);
    }
    if thread.is_multiple_of(10) {
        labels.push("Label_1");
    }
    let me = EmailAddress::new(Some("Me"), "me@example.com");
    let them = EmailAddress::new(Some(&format!("Person {}", thread % 500)), &format!("p{}@example.com", thread % 500));
    let text = PARAGRAPH.repeat(1 + i % 4);
    let m = FetchedMessage {
        id: MessageId::new(format!("{:016x}", 0x1a00_0000_0000_0000u64 + (n - i) as u64)),
        thread_id: ThreadId::new(format!("t{thread:07}")),
        label_ids: labels.into_iter().map(LabelId::new).collect(),
        snippet: text.chars().take(120).collect(),
        internal_date: NOW - age_ms,
        from: Some(if from_me { me.clone() } else { them.clone() }),
        to: vec![if from_me { them } else { me }],
        subject: format!("Plan for week {}", thread % 52),
        body: Some(FetchedBody { html: Some(format!("<p>{text}</p>")), text: Some(text), attachments: vec![] }),
        ..Default::default()
    };
    (m, in_inbox && !from_me, recent)
}

/// Peak resident memory, sampled with `ps` (no unsafe code in the tests).
struct RssSampler {
    peak: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn rss_kb() -> u64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}

impl RssSampler {
    fn start() -> Self {
        let peak = Arc::new(AtomicU64::new(rss_kb()));
        let stop = Arc::new(AtomicBool::new(false));
        let (p, s) = (peak.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                p.fetch_max(rss_kb(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        Self { peak, stop, thread: Some(thread) }
    }

    fn finish(mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.peak.load(Ordering::Relaxed).max(rss_kb())
    }
}

#[derive(Debug)]
#[expect(dead_code, reason = "the report is read through Debug")]
struct Report {
    messages: usize,
    threads: usize,
    inbox_threads: usize,
    list: Duration,
    inbox_browsable: Duration,
    all_headers: Duration,
    bodies: Duration,
    /// (priority, queued) right after listing.
    tiers: Vec<(i64, u64)>,
    bodies_fetched: usize,
    rss_after_seed_mb: u64,
    rss_peak_mb: u64,
    store_mb: u64,
}

async fn run(n: usize, name: &str) -> Report {
    let dir = std::env::temp_dir().join(format!("kaluta-scale-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = Db::open(&dir.join("mail.sqlite")).unwrap();
    let fake = Arc::new(FakeProvider::new("me@example.com", NOW, 500));
    fake.set_labels(vec![
        Label { id: LabelId::new("INBOX"), name: "INBOX".into(), kind: LabelKind::System, color: None, visible: true },
        Label {
            id: LabelId::new("Label_1"),
            name: "Receipts".into(),
            kind: LabelKind::User,
            color: None,
            visible: true,
        },
    ]);
    let mut inbox = std::collections::HashSet::new();
    for i in 0..n {
        let (m, in_inbox, _) = message(i, n);
        if in_inbox {
            inbox.insert(m.thread_id.clone());
        }
        fake.seed(m);
    }
    let rss_after_seed_mb = rss_kb() / 1024;
    let sampler = RssSampler::start();

    let engine = SyncEngine::new(fake.clone(), db.clone(), Arc::new(Quiet));
    engine.set_window(SyncWindow::Everything).await.unwrap();
    let source = Arc::new(Source(fake.clone(), AtomicUsize::new(0)));
    engine.set_backfill_source(source.clone());

    let started = Instant::now();
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    let list = started.elapsed();
    let tiers = db
        .read(|c| {
            let mut stmt =
                c.prepare("SELECT priority, COUNT(*) FROM backfill_queue GROUP BY priority ORDER BY priority")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?.max(0) as u64)))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await
        .unwrap();

    let inbox_count =
        || async { db.read(|c| read::list_threads(c, "INBOX", None, 500)).await.map(|p| p.rows.len()).unwrap_or(0) };
    let mut inbox_browsable = None;
    while engine.headers_pass(500).await.unwrap() > 0 {
        if inbox_browsable.is_none() && inbox_count().await >= inbox.len().min(500) {
            inbox_browsable = Some(started.elapsed());
        }
    }
    let all_headers = started.elapsed();
    let inbox_browsable = inbox_browsable.unwrap_or(all_headers);

    let bodies_started = Instant::now();
    engine.backfill_all().await.unwrap();
    let bodies = bodies_started.elapsed();
    let rss_peak_mb = sampler.finish() / 1024;

    let listed = db.read(|c| read::list_threads(c, ARCHIVE_LABEL, None, 1)).await.unwrap();
    assert!(!listed.rows.is_empty());
    let problems = db.read(consistency::check).await.unwrap();
    assert!(problems.is_empty(), "{problems:?}");
    let store_mb =
        std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()?.metadata().ok()).map(|m| m.len()).sum::<u64>()
            / (1024 * 1024);
    drop(engine);
    drop(db);
    let _ = std::fs::remove_dir_all(&dir);
    Report {
        messages: n,
        threads: n.div_ceil(PER_THREAD),
        inbox_threads: inbox.len(),
        list,
        inbox_browsable,
        all_headers,
        bodies,
        tiers,
        bodies_fetched: source.1.load(Ordering::SeqCst),
        rss_after_seed_mb,
        rss_peak_mb,
        store_mb,
    }
}

/// Bodies the tiers should fetch: Inbox mail and the last 30 days
/// (the default body window).
fn expected_bodies(n: usize) -> usize {
    (0..n)
        .filter(|&i| {
            let (m, in_inbox, _) = message(i, n);
            in_inbox || NOW - m.internal_date < 30 * DAY
        })
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn five_thousand_messages_tier_list_and_fetch_only_their_window() {
    let report = run(5_000, "5k").await;
    eprintln!("{report:#?}");
    let headers_only: u64 = report.tiers.iter().filter(|(p, _)| *p >= 10).map(|(_, c)| c).sum();
    let for_bodies: u64 = report.tiers.iter().filter(|(p, _)| *p < 10).map(|(_, c)| c).sum();
    assert_eq!(for_bodies + headers_only, 5_000, "every message queued once");
    assert_eq!(for_bodies as usize, expected_bodies(5_000), "bodies for the Inbox and the last 30 days only");
    assert_eq!(report.bodies_fetched, expected_bodies(5_000), "no body outside the window was fetched");
    assert!(report.tiers.first().is_some_and(|(p, _)| *p == 0), "unread Inbox mail first");
    // Generous bounds: a regression, not a benchmark (debug build, CI).
    assert!(report.all_headers < Duration::from_secs(60), "{:?}", report.all_headers);
    assert!(report.inbox_browsable <= report.all_headers);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "benchmark: cargo test --release -p mail-sync --test scale -- --ignored --nocapture"]
async fn benchmark_100k() {
    let report = run(100_000, "100k").await;
    println!("{report:#?}");
}
