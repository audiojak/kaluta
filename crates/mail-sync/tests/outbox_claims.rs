//! Several drainers on one outbox (spec §7.4, outbox claims): two engines
//! in one process, and a second process racing this one or dying mid-send.
//! Every op reaches the provider exactly once and in order, and a send
//! left in flight by a dead drainer is looked for before it is sent again.
//!
//! The second process is this test binary run again for
//! [`child_drainer`], told what to do through the environment.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use mail_domain::{EmailAddress, Label, LabelId, MessageId, ThreadId};
use mail_store::drafts::{self, DraftRecord};
use mail_store::outbox::{self, OutboxOp};
use mail_store::{Db, ThreadChanges};
use mail_sync::{SyncEngine, SyncObserver, send_draft};
use provider_api::fake::FakeProvider;
use provider_api::{
    ChangeSet, FetchedMessage, IdPage, LabelOp, ListFilter, MailProvider, PageToken, Priority, Profile, ProviderResult,
    SyncCursor,
};

const NOW: i64 = 1_790_000_000_000;
const OPS: usize = 1_000;

const CHILD_MODE: &str = "KALUTA_CLAIMS_CHILD";
const CHILD_DB: &str = "KALUTA_CLAIMS_DB";
const CHILD_OUT: &str = "KALUTA_CLAIMS_OUT";

struct Quiet;
impl SyncObserver for Quiet {
    fn threads_changed(&self, _: &ThreadChanges) {}
}

/// What a send does in the provider, for the dying-drainer tests.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SendMode {
    Normal,
    /// The provider takes the message, then this process hangs until killed.
    HangAfter,
    /// This process hangs before the provider is asked.
    HangBefore,
}

/// The fake provider, counting every write call with when it began.
struct Counting {
    inner: Arc<FakeProvider>,
    /// (message id, nanoseconds since the epoch) per label change.
    label_calls: Mutex<Vec<(String, u128)>>,
    sends: AtomicU64,
    lookups: AtomicU64,
    mode: SendMode,
    /// Where a hanging send reports how far it got.
    report: Option<PathBuf>,
}

impl Counting {
    fn new(inner: Arc<FakeProvider>) -> Self {
        Self {
            inner,
            label_calls: Mutex::default(),
            sends: AtomicU64::new(0),
            lookups: AtomicU64::new(0),
            mode: SendMode::Normal,
            report: None,
        }
    }
}

fn nanos() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
}

/// Write `bytes` to `path` whole (another process polls for it).
fn publish(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

#[async_trait]
impl MailProvider for Counting {
    async fn profile(&self) -> ProviderResult<Profile> {
        self.inner.profile().await
    }
    async fn list_labels(&self) -> ProviderResult<Vec<Label>> {
        self.inner.list_labels().await
    }
    async fn list_message_ids(&self, filter: &ListFilter, page: Option<PageToken>) -> ProviderResult<IdPage> {
        self.inner.list_message_ids(filter, page).await
    }
    async fn fetch_messages(&self, ids: &[MessageId], priority: Priority) -> ProviderResult<Vec<FetchedMessage>> {
        self.inner.fetch_messages(ids, priority).await
    }
    async fn changes_since(&self, cursor: &SyncCursor) -> ProviderResult<ChangeSet> {
        self.inner.changes_since(cursor).await
    }
    async fn modify_labels(&self, op: &LabelOp) -> ProviderResult<()> {
        for m in &op.message_ids {
            self.label_calls.lock().unwrap().push((m.0.clone(), nanos()));
        }
        // A network call takes a moment: the other drainer asks meanwhile.
        tokio::time::sleep(Duration::from_micros(200)).await;
        self.inner.modify_labels(op).await
    }
    async fn move_to_trash(&self, id: &MessageId) -> ProviderResult<()> {
        self.inner.move_to_trash(id).await
    }
    async fn restore_from_trash(&self, id: &MessageId) -> ProviderResult<()> {
        self.inner.restore_from_trash(id).await
    }
    async fn send(&self, raw: &[u8], thread: Option<&ThreadId>) -> ProviderResult<MessageId> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            SendMode::Normal => self.inner.send(raw, thread).await,
            SendMode::HangBefore => {
                publish(self.report.as_ref().unwrap(), b"started");
                std::future::pending().await
            }
            SendMode::HangAfter => {
                let sent = self.inner.send(raw, thread).await;
                publish(self.report.as_ref().unwrap(), raw);
                let _ = sent;
                std::future::pending().await
            }
        }
    }
    async fn already_sent(&self, raw: &[u8]) -> ProviderResult<Option<MessageId>> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.inner.already_sent(raw).await
    }
    async fn fetch_attachment(&self, message: &MessageId, attachment_id: &str) -> ProviderResult<Vec<u8>> {
        self.inner.fetch_attachment(message, attachment_id).await
    }
    async fn save_draft(
        &self,
        existing: Option<&str>,
        raw: &[u8],
        thread: Option<&ThreadId>,
    ) -> ProviderResult<String> {
        self.inner.save_draft(existing, raw, thread).await
    }
    async fn delete_draft(&self, draft_id: &str) -> ProviderResult<()> {
        self.inner.delete_draft(draft_id).await
    }
    async fn create_label(&self, name: &str, color: Option<(&str, &str)>) -> ProviderResult<Label> {
        self.inner.create_label(name, color).await
    }
}

fn store_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kaluta-claims-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("mail.sqlite")
}

fn fake() -> Arc<FakeProvider> {
    Arc::new(FakeProvider::new("me@example.com", NOW, 50))
}

/// `OPS` label changes, one message each, in order.
async fn queue_label_ops(db: &Db) {
    db.write(|tx| {
        for n in 0..OPS {
            let op = OutboxOp::ModifyLabels {
                message_ids: vec![MessageId::new(format!("m{n:04}"))],
                add: vec![LabelId::new("STARRED")],
                remove: vec![],
            };
            outbox::enqueue(tx, &op, NOW)?;
        }
        Ok(())
    })
    .await
    .unwrap();
}

/// Drain until nothing is pending or in flight anywhere.
async fn drain_until_empty(engine: &SyncEngine, db: &Db) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let report = engine.drain_outbox().await.expect("a drain never fails for another drainer");
        let (counts, busy) = db.read(|c| Ok((outbox::counts(c)?, outbox::in_flight(c)?))).await.unwrap();
        if counts.pending == 0 && !busy {
            return;
        }
        assert!(Instant::now() < deadline, "the outbox did not empty");
        if report.busy || report.sent == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    }
}

/// Every label change exactly once, and in queue order by the time each
/// call began.
fn assert_once_in_order(calls: Vec<(String, u128)>) {
    let mut per_op: BTreeMap<String, usize> = BTreeMap::new();
    for (m, _) in &calls {
        *per_op.entry(m.clone()).or_default() += 1;
    }
    assert_eq!(per_op.len(), OPS, "every op reached the provider");
    let twice: Vec<_> = per_op.iter().filter(|(_, n)| **n != 1).collect();
    assert!(twice.is_empty(), "ops sent more than once: {twice:?}");
    let mut by_time = calls;
    by_time.sort_by_key(|(_, at)| *at);
    let order: Vec<&str> = by_time.iter().map(|(m, _)| m.as_str()).collect();
    let mut sorted = order.clone();
    sorted.sort();
    assert_eq!(order, sorted, "ops reached the provider in queue order");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_engines_on_one_store_send_every_op_once_and_in_order() {
    let path = store_path("two-engines");
    let db_a = Db::open(&path).unwrap();
    let db_b = Db::open(&path).unwrap();
    queue_label_ops(&db_a).await;
    let provider = Arc::new(Counting::new(fake()));
    let a = Arc::new(SyncEngine::new(provider.clone(), db_a.clone(), Arc::new(Quiet)));
    let b = Arc::new(SyncEngine::new(provider.clone(), db_b.clone(), Arc::new(Quiet)));
    let ta = tokio::spawn(async move { drain_until_empty(&a, &db_a).await });
    let tb = tokio::spawn(async move { drain_until_empty(&b, &db_b).await });
    ta.await.unwrap();
    tb.await.unwrap();
    let calls = provider.label_calls.lock().unwrap().clone();
    assert_once_in_order(calls);
}

#[tokio::test]
async fn a_live_drainers_op_waits_for_it_and_one_past_its_lease_is_taken_back() {
    let path = store_path("lease");
    let db = Db::open(&path).unwrap();
    queue_label_ops(&db).await;
    // Another drainer, alive (its lock held), holds the first op.
    let other = outbox::Claimant::register(&path);
    let other_id = other.id().to_owned();
    let lease = mail_sync::now_millis() + 60_000;
    let id = other_id.clone();
    db.write(move |tx| outbox::claim_next(tx, &id, NOW, lease)).await.unwrap();

    let provider = Arc::new(Counting::new(fake()));
    let engine = SyncEngine::new(provider.clone(), db.clone(), Arc::new(Quiet));
    let report = engine.drain_outbox().await.unwrap();
    assert!(report.busy, "{report:?}");
    assert_eq!(report.sent, 0);
    assert!(provider.label_calls.lock().unwrap().is_empty(), "nothing passes the op in flight");
    let retry = engine.next_outbox_retry().await.unwrap().unwrap();
    assert!(retry <= mail_sync::now_millis() + 2_000, "looks again soon");

    // Its lease runs out (a hung drainer): the op is taken back and sent.
    db.write(|tx| {
        tx.execute("UPDATE outbox SET lease_until = 0 WHERE state = 'in_flight'", [])?;
        Ok(())
    })
    .await
    .unwrap();
    drain_until_empty(&engine, &db).await;
    assert_once_in_order(provider.label_calls.lock().unwrap().clone());
    drop(other);
}

#[tokio::test]
async fn ops_left_in_flight_by_a_crashed_run_go_at_the_next_launch() {
    let path = store_path("relaunch");
    let db = Db::open(&path).unwrap();
    queue_label_ops(&db).await;
    // The last run claimed the first op and crashed (its engine is gone);
    // a build from before claims left the second in flight with no name.
    let dead = outbox::Claimant::register(&path);
    let dead_id = dead.id().to_owned();
    drop(dead);
    db.write(move |tx| {
        tx.execute(
            &format!(
                "UPDATE outbox SET state = 'in_flight', claimed_by = '{dead_id}', lease_until = {}
                 WHERE id = (SELECT MIN(id) FROM outbox)",
                i64::MAX
            ),
            [],
        )?;
        tx.execute("UPDATE outbox SET state = 'in_flight' WHERE id = (SELECT MIN(id) + 1 FROM outbox)", [])?;
        Ok(())
    })
    .await
    .unwrap();
    let provider = Arc::new(Counting::new(fake()));
    let engine = SyncEngine::new(provider.clone(), db.clone(), Arc::new(Quiet));
    let report = engine.drain_outbox().await.unwrap();
    assert_eq!(report.sent, OPS, "recovered at once, without waiting out a lease");
    assert_once_in_order(provider.label_calls.lock().unwrap().clone());
}

// ---- A second process ----------------------------------------------------

fn spawn_child(mode: &str, db: &Path, out: &Path) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["child_drainer", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_MODE, mode)
        .env(CHILD_DB, db)
        .env(CHILD_OUT, out)
        .spawn()
        .unwrap()
}

fn wait_for(path: &Path, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !path.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("the second process exited early: {status}");
        }
        assert!(Instant::now() < deadline, "the second process never got to {path:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The second process, when this binary is run for it; otherwise nothing.
#[test]
fn child_drainer() {
    let Ok(mode) = std::env::var(CHILD_MODE) else { return };
    let db_path = PathBuf::from(std::env::var(CHILD_DB).unwrap());
    let out = PathBuf::from(std::env::var(CHILD_OUT).unwrap());
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    rt.block_on(async move {
        let db = Db::open(&db_path).unwrap();
        let mut provider = Counting::new(fake());
        provider.mode = match mode.as_str() {
            "hang_after_send" => SendMode::HangAfter,
            "hang_before_send" => SendMode::HangBefore,
            _ => SendMode::Normal,
        };
        provider.report = Some(out.clone());
        let provider = Arc::new(provider);
        let engine = SyncEngine::new(provider.clone(), db.clone(), Arc::new(Quiet));
        if mode == "race" {
            publish(&out.with_extension("ready"), b"");
            drain_until_empty(&engine, &db).await;
            let calls = provider.label_calls.lock().unwrap().clone();
            let lines: Vec<String> = calls.iter().map(|(m, at)| format!("{m} {at}")).collect();
            publish(&out, lines.join("\n").as_bytes());
        } else {
            // Hangs inside the send until killed.
            engine.drain_outbox().await.unwrap();
            panic!("the send should have hung");
        }
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_processes_racing_on_one_outbox_send_every_op_exactly_once() {
    let path = store_path("two-processes");
    let db = Db::open(&path).unwrap();
    queue_label_ops(&db).await;
    let out = path.with_file_name("child-calls.txt");
    let mut child = spawn_child("race", &path, &out);
    // Start together.
    wait_for(&out.with_extension("ready"), &mut child);
    let provider = Arc::new(Counting::new(fake()));
    let engine = SyncEngine::new(provider.clone(), db.clone(), Arc::new(Quiet));
    drain_until_empty(&engine, &db).await;
    let status = tokio::task::spawn_blocking(move || child.wait()).await.unwrap().unwrap();
    assert!(status.success(), "the second process failed: {status}");
    let theirs: Vec<(String, u128)> = std::fs::read_to_string(&out)
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (m, at) = l.split_once(' ').unwrap();
            (m.to_owned(), at.parse().unwrap())
        })
        .collect();
    let ours = provider.label_calls.lock().unwrap().clone();
    eprintln!("this process sent {} ops, the other {}", ours.len(), theirs.len());
    let mut all = ours;
    all.extend(theirs);
    assert_once_in_order(all);
}

fn me() -> EmailAddress {
    EmailAddress::new(Some("Me"), "me@example.com")
}

/// A queued send, never tried.
async fn queue_send(db: &Db) -> i64 {
    let draft = DraftRecord {
        to: vec![EmailAddress::new(None, "sam@example.org")],
        subject: "Hello".into(),
        body_html: "<p>Hi</p>".into(),
        ..Default::default()
    };
    let id = db.write(move |tx| drafts::save(tx, &draft, NOW)).await.unwrap();
    send_draft(db, id, me(), true, 0).await.unwrap();
    id
}

/// A second process takes the send and is killed inside the provider
/// call; this one then drains. Returns this process's provider and what
/// the killed one reported.
async fn killed_mid_send(name: &str, mode: &str, server_has_it: bool) -> (Arc<Counting>, Db, i64) {
    let path = store_path(name);
    let db = Db::open(&path).unwrap();
    let draft = queue_send(&db).await;
    let out = path.with_file_name("child-send.eml");
    let mut child = spawn_child(mode, &path, &out);
    wait_for(&out, &mut child);
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(db.read(outbox::in_flight).await.unwrap(), "the send was left in flight");

    let server = fake();
    if server_has_it {
        // The provider took the message before the process died.
        let raw = std::fs::read(&out).unwrap();
        server.send(&raw, None).await.unwrap();
    }
    let provider = Arc::new(Counting::new(server));
    let engine = SyncEngine::new(provider.clone(), db.clone(), Arc::new(Quiet));
    let report = engine.drain_outbox().await.unwrap();
    assert_eq!(report.sent, 1, "recovered at once: the killed process's lock is free");
    assert!(!db.read(outbox::in_flight).await.unwrap());
    assert_eq!(db.read(outbox::counts).await.unwrap(), outbox::OutboxCounts::default());
    assert!(db.read(move |c| drafts::get(c, draft)).await.unwrap().is_none(), "the draft is done with");
    (provider, db, draft)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_whose_drainer_was_killed_after_the_provider_took_it_is_not_sent_again() {
    let (provider, _db, _) = killed_mid_send("killed-after", "hang_after_send", true).await;
    assert_eq!(provider.lookups.load(Ordering::SeqCst), 1, "looked for first");
    assert_eq!(provider.sends.load(Ordering::SeqCst), 0, "found, so not sent again");
    assert_eq!(provider.inner.message_count(), 1, "one copy at the provider");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_send_whose_drainer_was_killed_before_the_provider_took_it_goes_once() {
    let (provider, _db, _) = killed_mid_send("killed-before", "hang_before_send", false).await;
    assert_eq!(provider.lookups.load(Ordering::SeqCst), 1, "looked for first");
    assert_eq!(provider.sends.load(Ordering::SeqCst), 1, "not found, so sent");
    assert_eq!(provider.inner.message_count(), 1);
}
