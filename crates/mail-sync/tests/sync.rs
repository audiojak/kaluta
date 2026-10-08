//! Sync engine against the in-memory fake provider.

use std::sync::{Arc, Mutex};

use mail_domain::{EmailAddress, Label, LabelId, LabelKind, MessageId, ThreadId};
use mail_store::{ARCHIVE_LABEL, Db, ThreadChanges, consistency, queue, read};
use mail_sync::{BodyWindow, EveryHeader, SyncEngine, SyncError, SyncObserver, SyncPhase, SyncProgress, SyncWindow};
use provider_api::fake::FakeProvider;
use provider_api::{FetchedBody, FetchedMessage};

const NOW: i64 = 1_790_000_000_000;
const DAY: i64 = 86_400_000;

#[derive(Default)]
struct Recorder {
    changes: Mutex<Vec<ThreadChanges>>,
    progress: Mutex<Vec<SyncProgress>>,
}

impl SyncObserver for Recorder {
    fn threads_changed(&self, changes: &ThreadChanges) {
        self.changes.lock().unwrap().push(changes.clone());
    }
    fn progress(&self, progress: SyncProgress) {
        self.progress.lock().unwrap().push(progress);
    }
}

fn message(id: &str, thread: &str, age_days: i64, labels: &[&str]) -> FetchedMessage {
    FetchedMessage {
        id: MessageId::new(id),
        thread_id: ThreadId::new(thread),
        label_ids: labels.iter().map(|l| LabelId::new(*l)).collect(),
        snippet: format!("snippet {id}"),
        internal_date: NOW - age_days * DAY,
        from: Some(EmailAddress::new(Some("Sender"), "sender@example.com")),
        to: vec![EmailAddress::new(None, "me@example.com")],
        subject: format!("Subject {thread}"),
        body: Some(FetchedBody {
            text: Some(format!("body of {id}")),
            html: Some(format!("<p>body of {id}</p><script>alert(1)</script><img src=\"https://t.example/p.gif\">")),
            attachments: vec![],
        }),
        ..Default::default()
    }
}

fn setup(name: &str) -> (Arc<FakeProvider>, Db, Arc<Recorder>, SyncEngine) {
    let dir = std::env::temp_dir().join(format!("openagc-sync-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = Db::open(&dir.join("mail.sqlite")).unwrap();
    let fake = Arc::new(FakeProvider::new("me@example.com", NOW, 3));
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
    let recorder = Arc::new(Recorder::default());
    let engine = SyncEngine::new(fake.clone(), db.clone(), recorder.clone());
    (fake, db, recorder, engine)
}

fn seed_mailbox(fake: &FakeProvider) {
    fake.seed(message("inbox-unread", "t1", 1, &["INBOX", "UNREAD"]));
    fake.seed(message("inbox-read", "t2", 2, &["INBOX"]));
    fake.seed(message("recent", "t3", 10, &["Label_1"]));
    fake.seed(message("this-year", "t4", 100, &[]));
    fake.seed(message("ancient", "t5", 900, &[]));
}

fn assert_consistent(db: &Db) {
    let problems = db.read_blocking(consistency::check).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
}

#[tokio::test]
async fn bootstrap_queues_by_priority_and_backfill_fills_the_store() {
    let (fake, db, recorder, engine) = setup("bootstrap");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    fake.seed(message("spam", "t6", 1, &["SPAM"]));
    fake.seed(message("binned", "t7", 3, &["TRASH"]));
    assert!(engine.needs_bootstrap().await.unwrap());

    engine.bootstrap_prepare().await.unwrap();
    // Only inbox phases are queued so far, unread first.
    let queued = db.read(|c| queue::peek(c, 10)).await.unwrap();
    assert_eq!(queued.iter().map(|m| m.as_str()).collect::<Vec<_>>(), vec!["inbox-unread", "inbox-read"]);

    engine.bootstrap_list_rest().await.unwrap();
    assert!(!engine.needs_bootstrap().await.unwrap());
    let queued = db.read(|c| queue::peek(c, 10)).await.unwrap();
    assert_eq!(
        queued.iter().map(|m| m.as_str()).collect::<Vec<_>>(),
        vec!["inbox-unread", "inbox-read", "recent", "spam", "binned", "this-year", "ancient"],
        "priority order; Spam and Trash with the last month, whatever their age"
    );

    let fetched = engine.backfill_all().await.unwrap();
    assert_eq!(fetched, 7);
    assert_eq!(db.read(queue::len).await.unwrap(), 0);
    let inbox = db.read(|c| read::list_threads(c, "INBOX", None, 10)).await.unwrap();
    assert_eq!(inbox.rows.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["t1", "t2"]);
    let receipts = db.read(|c| read::list_threads(c, "Label_1", None, 10)).await.unwrap();
    assert_eq!(receipts.rows.len(), 1);
    assert_eq!(engine.account_email().await.unwrap().as_deref(), Some("me@example.com"));

    // Bodies were sanitized on the way in.
    let body = db.read(|c| read::get_body(c, &MessageId::new("inbox-unread"))).await.unwrap().unwrap();
    let html = body.html_sanitized.unwrap();
    assert!(!html.contains("script"), "{html}");
    assert!(html.contains("openagc-remote:https://t.example/p.gif"));
    assert!(body.has_remote_images);

    // The UI heard about it, and the final progress is idle.
    assert!(recorder.changes.lock().unwrap().iter().any(|c| c.mailboxes.contains_key("INBOX")));
    assert_eq!(recorder.progress.lock().unwrap().last().unwrap().phase, SyncPhase::Idle);
    assert_consistent(&db);
}

#[tokio::test]
async fn backfill_fetches_newest_first_within_a_phase() {
    let (fake, db, _recorder, engine) = setup("newest-first");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    // Seeded oldest first; the provider lists newest first regardless.
    fake.seed(message("old", "t1", 300, &[]));
    fake.seed(message("mid", "t2", 200, &[]));
    fake.seed(message("new", "t3", 100, &[]));
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    let queued = db.read(|c| queue::peek(c, 10)).await.unwrap();
    assert_eq!(queued.iter().map(|m| m.as_str()).collect::<Vec<_>>(), vec!["new", "mid", "old"]);
}

#[tokio::test]
async fn the_sync_window_bounds_the_backfill_and_can_be_widened_or_narrowed() {
    let (fake, db, _recorder, engine) = setup("window");
    seed_mailbox(&fake);
    assert_eq!(engine.window().await.unwrap(), SyncWindow::HalfYear, "default");
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    let queued = |db: &Db| {
        let q = db.read_blocking(|c| queue::peek(c, 10)).unwrap();
        q.iter().map(|m| m.as_str().to_owned()).collect::<Vec<_>>()
    };
    // 100-day-old mail is inside six months; 900-day-old mail is not.
    assert_eq!(queued(&db), vec!["inbox-unread", "inbox-read", "recent", "this-year"]);

    engine.set_window(SyncWindow::Month).await.unwrap();
    assert_eq!(queued(&db), vec!["inbox-unread", "inbox-read", "recent"], "narrowing drops queued older mail");

    engine.set_window(SyncWindow::Everything).await.unwrap();
    assert_eq!(queued(&db), vec!["inbox-unread", "inbox-read", "recent", "this-year", "ancient"]);
    assert_eq!(engine.window().await.unwrap(), SyncWindow::Everything);

    // An account from before windows existed gets the default applied once.
    db.write(|tx| Ok(tx.execute("DELETE FROM sync_state WHERE key = 'sync_window'", [])?)).await.unwrap();
    engine.ensure_window().await.unwrap();
    assert_eq!(queued(&db), vec!["inbox-unread", "inbox-read", "recent", "this-year"], "trimmed to six months");
    engine.ensure_window().await.unwrap();
    assert_eq!(engine.window().await.unwrap(), SyncWindow::HalfYear);
}

/// A backfill source that answers from the fake provider's data but
/// counts its calls, standing in for a bulk transport.
struct CountingSource(Arc<FakeProvider>, std::sync::atomic::AtomicUsize, bool);

impl CountingSource {
    /// Headers cost about as much as bodies (the default): no tiers.
    fn new(fake: &Arc<FakeProvider>) -> Self {
        Self(fake.clone(), Default::default(), false)
    }

    /// Headers are cheap (IMAP): tiered download applies.
    fn cheap(fake: &Arc<FakeProvider>) -> Self {
        Self(fake.clone(), Default::default(), true)
    }

    fn bodies(&self) -> usize {
        self.1.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl provider_api::BackfillSource for CountingSource {
    async fn fetch(&self, ids: &[MessageId]) -> provider_api::ProviderResult<Vec<FetchedMessage>> {
        self.1.fetch_add(ids.len(), std::sync::atomic::Ordering::SeqCst);
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
        self.2
    }
    fn name(&self) -> &'static str {
        "counting"
    }
}

#[tokio::test]
async fn a_headers_pass_fills_the_list_before_bodies_and_leaves_them_queued() {
    let (fake, db, _recorder, engine) = setup("headers");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    assert_eq!(engine.headers_pass(100).await.unwrap(), 0, "REST: headers cost as much as bodies, so no pass");

    engine.set_backfill_source(Arc::new(CountingSource::new(&fake)));
    assert_eq!(engine.headers_pass(3).await.unwrap(), 3);
    assert_eq!(engine.headers_pass(100).await.unwrap(), 2);
    assert_eq!(engine.headers_pass(100).await.unwrap(), 0, "every queued message has a row");
    let inbox = db.read(|c| read::list_threads(c, "INBOX", None, 10)).await.unwrap();
    assert_eq!(inbox.rows.len(), 2, "browsable already");
    assert!(db.read(|c| read::get_body(c, &MessageId::new("inbox-unread"))).await.unwrap().is_none(), "no body yet");
    assert_eq!(db.read(queue::len).await.unwrap(), 5, "still queued for bodies");

    // Opening one moves it to the front.
    engine.prioritize(vec![MessageId::new("ancient")]).await.unwrap();
    assert_eq!(db.read(|c| queue::peek(c, 1)).await.unwrap(), vec![MessageId::new("ancient")]);
    assert_eq!(engine.backfill_all().await.unwrap(), 5);
    assert!(db.read(|c| read::get_body(c, &MessageId::new("inbox-unread"))).await.unwrap().is_some());
    assert_consistent(&db);
}

fn ids(v: Vec<MessageId>) -> Vec<String> {
    v.into_iter().map(|m| m.0).collect()
}

fn body_state(db: &Db, id: &str) -> Option<String> {
    let id = id.to_owned();
    db.read_blocking(move |c| {
        Ok(c.query_row("SELECT body_state FROM messages WHERE gmail_id = ?1", [id], |r| r.get(0)).ok())
    })
    .unwrap()
}

#[tokio::test]
async fn with_cheap_headers_only_the_inbox_and_the_body_window_get_bodies() {
    let (fake, db, _recorder, engine) = setup("tiers");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    let source = Arc::new(CountingSource::cheap(&fake));
    engine.set_backfill_source(source.clone());
    assert_eq!(engine.body_window().await.unwrap(), BodyWindow::Month, "default");
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();

    // Bodies: the Inbox and the last 30 days. Older mail: headers only.
    assert_eq!(ids(db.read(|c| queue::peek(c, 10)).await.unwrap()), ["inbox-unread", "inbox-read", "recent"]);
    assert_eq!(db.read(queue::counts).await.unwrap(), (3, 2));

    // One headers pass lists everything; the headers-only tier leaves the queue.
    assert_eq!(engine.headers_pass(100).await.unwrap(), 7, "progress: 5 rows stored, 2 headers-only ids done");
    assert_eq!(db.read(queue::counts).await.unwrap(), (3, 0));
    assert_eq!(engine.headers_pass(100).await.unwrap(), 0, "nothing left for headers");
    assert_eq!(body_state(&db, "ancient").as_deref(), Some("metadata"));
    let all = db.read(|c| read::list_threads(c, ARCHIVE_LABEL, None, 10)).await.unwrap();
    assert!(all.rows.iter().any(|t| t.id.as_str() == "t5"), "old mail is listed");

    assert_eq!(engine.backfill_all().await.unwrap(), 3);
    assert_eq!(source.bodies(), 3, "no bodies outside the body window");
    assert_eq!(body_state(&db, "this-year").as_deref(), Some("metadata"));

    // Widening the body window queues bodies for mail now inside it.
    engine.set_body_window(BodyWindow::HalfYear).await.unwrap();
    assert_eq!(ids(db.read(|c| queue::peek(c, 10)).await.unwrap()), ["this-year"]);
    assert_eq!(engine.backfill_all().await.unwrap(), 1);
    assert_eq!(body_state(&db, "this-year").as_deref(), Some("full"));
    assert_eq!(body_state(&db, "ancient").as_deref(), Some("metadata"));

    // Narrowing keeps what is stored and queues nothing.
    engine.set_body_window(BodyWindow::Month).await.unwrap();
    assert_eq!(db.read(queue::counts).await.unwrap(), (0, 0));
    assert_eq!(body_state(&db, "this-year").as_deref(), Some("full"));

    // The whole window: every body.
    engine.set_body_window(BodyWindow::Window).await.unwrap();
    assert_eq!(ids(db.read(|c| queue::peek(c, 10)).await.unwrap()), ["ancient"]);
    assert_consistent(&db);
}

fn widened(outcome: EveryHeader) -> bool {
    matches!(outcome, EveryHeader::Widened { .. })
}

/// Clean Up (spec §14.12) loads every header: the window becomes
/// Everything, and older mail comes down as headers only, whatever the
/// body window was.
#[tokio::test]
async fn loading_every_header_widens_the_window_without_bodies_beyond_the_body_window() {
    for (body, bodies) in [
        (BodyWindow::Month, &["inbox-unread", "inbox-read", "recent"][..]),
        // "The whole window" meant six months: it stays six months.
        (BodyWindow::Window, &["inbox-unread", "inbox-read", "recent", "this-year"][..]),
    ] {
        let (fake, db, recorder, engine) = setup(&format!("load-headers-{}", body.as_str()));
        engine.set_window(SyncWindow::HalfYear).await.unwrap();
        engine.set_body_window(body).await.unwrap();
        seed_mailbox(&fake);
        let source = Arc::new(CountingSource::cheap(&fake));
        engine.set_backfill_source(source.clone());
        engine.bootstrap_prepare().await.unwrap();
        engine.bootstrap_list_rest().await.unwrap();
        while engine.headers_pass(100).await.unwrap() > 0 {}
        engine.backfill_all().await.unwrap();
        assert_eq!(body_state(&db, "ancient"), None, "outside the six months: not listed");
        let fetched = source.bodies();

        let kept = if body == BodyWindow::Window { BodyWindow::HalfYear } else { body };
        assert_eq!(
            engine.load_every_header(true).await.unwrap(),
            EveryHeader::Widened { body_window: (kept != body).then_some(kept) },
            "the body window changes only to keep bodies where they were, and says so"
        );
        assert_eq!(engine.window().await.unwrap(), SyncWindow::Everything);
        assert_eq!(engine.body_window().await.unwrap(), kept, "bodies stay where they were");
        assert_eq!(db.read(queue::counts).await.unwrap(), (0, 1), "the older mail is queued for headers only");
        assert_eq!(recorder.progress.lock().unwrap().last().map(|p| p.headers), Some(1), "reported");

        while engine.headers_pass(100).await.unwrap() > 0 {}
        assert_eq!(engine.backfill_all().await.unwrap(), 0, "no bodies to fetch");
        assert_eq!(source.bodies(), fetched, "no body fetched for the older mail");
        assert_eq!(body_state(&db, "ancient").as_deref(), Some("metadata"), "its headers are stored");
        let full: Vec<String> = db
            .read(|c| {
                let mut stmt =
                    c.prepare("SELECT gmail_id FROM messages WHERE body_state = 'full' ORDER BY gmail_id")?;
                let rows = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap();
        let mut want: Vec<String> = bodies.iter().map(|s| (*s).to_owned()).collect();
        want.sort();
        assert_eq!(full, want, "bodies for the Inbox and the body window only");

        assert_eq!(
            engine.load_every_header(true).await.unwrap(),
            EveryHeader::Unchanged,
            "already everything: nothing to do"
        );
        assert_consistent(&db);
    }
}

#[tokio::test]
async fn refresh_under_tiers_brings_header_only_labels_current_without_bodies() {
    let (fake, db, _recorder, engine) = setup("tiers-refresh");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    let source = Arc::new(CountingSource::cheap(&fake));
    engine.set_backfill_source(source.clone());
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    while engine.headers_pass(100).await.unwrap() > 0 {}
    engine.backfill_all().await.unwrap();
    db.write(|tx| {
        let mut w = mail_store::MailWriter::new(tx);
        w.modify_message_labels(&MessageId::new("ancient"), &[LabelId::new("Label_1")], &[])?;
        w.finish().map(|_| ())
    })
    .await
    .unwrap();

    engine.refetch_all().await.unwrap();
    assert_eq!(db.read(queue::counts).await.unwrap(), (3, 2), "old mail refreshes by headers");
    while engine.headers_pass(100).await.unwrap() > 0 {}
    engine.backfill_all().await.unwrap();
    assert_eq!(source.bodies(), 6, "the three bodies again, none for old mail");
    let receipts = db.read(|c| read::list_threads(c, "Label_1", None, 10)).await.unwrap();
    assert_eq!(receipts.rows.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["t3"], "label restored");
    let (thread, _) = db.read(|c| read::get_thread(c, &ThreadId::new("t5"))).await.unwrap().unwrap();
    assert_eq!(thread.snippet, "snippet ancient", "a header refresh keeps the snippet");
    assert_consistent(&db);
}

#[tokio::test]
async fn turning_imap_on_or_off_re_tiers_the_queue() {
    let (fake, db, _recorder, engine) = setup("retier");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    assert_eq!(db.read(queue::counts).await.unwrap(), (5, 0), "REST: bodies for the whole window");
    engine.ensure_tiers().await.unwrap();
    assert_eq!(db.read(queue::counts).await.unwrap(), (5, 0), "unchanged tiering does nothing");

    engine.set_backfill_source(Arc::new(CountingSource::cheap(&fake)));
    engine.ensure_tiers().await.unwrap();
    assert_eq!(db.read(queue::counts).await.unwrap(), (3, 2));
    while engine.headers_pass(100).await.unwrap() > 0 {}

    // Back to REST: header-only mail in the window gets its bodies.
    engine.use_rest_backfill();
    engine.ensure_tiers().await.unwrap();
    assert_eq!(
        ids(db.read(|c| queue::peek(c, 10)).await.unwrap()),
        ["inbox-unread", "inbox-read", "recent", "this-year", "ancient"]
    );
    assert_eq!(engine.backfill_all().await.unwrap(), 5);
    assert_eq!(body_state(&db, "ancient").as_deref(), Some("full"));
}

#[tokio::test]
async fn headers_only_mail_is_fetched_in_full_when_headers_stop_being_cheap() {
    let (fake, db, _recorder, engine) = setup("refused");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    engine.set_backfill_source(Arc::new(CountingSource::cheap(&fake)));
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    // IMAP refused mid-run: the source can no longer fetch headers.
    engine.use_rest_backfill();
    assert_eq!(engine.headers_pass(100).await.unwrap(), 0);
    assert_eq!(db.read(queue::counts).await.unwrap(), (5, 0));
    assert_eq!(engine.backfill_all().await.unwrap(), 5);
}

/// A cheap-headers source whose IMAP can be refused and let back, as
/// `ImapBackfill` is: refused, headers are not cheap and fetching them is
/// unavailable.
struct Refusable(CountingSource, std::sync::atomic::AtomicBool);

impl Refusable {
    fn new(fake: &Arc<FakeProvider>) -> Arc<Self> {
        Arc::new(Self(CountingSource::cheap(fake), Default::default()))
    }
    fn refuse(&self, refused: bool) {
        self.1.store(refused, std::sync::atomic::Ordering::SeqCst);
    }
    fn refused(&self) -> bool {
        self.1.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl provider_api::BackfillSource for Refusable {
    async fn fetch(&self, ids: &[MessageId]) -> provider_api::ProviderResult<Vec<FetchedMessage>> {
        self.0.fetch(ids).await
    }
    async fn fetch_headers(&self, ids: &[MessageId]) -> provider_api::ProviderResult<Option<Vec<FetchedMessage>>> {
        if self.refused() {
            return Err(provider_api::ProviderError::Unavailable("IMAP sign-in was refused".into()));
        }
        self.0.fetch_headers(ids).await
    }
    fn cheap_headers(&self) -> bool {
        !self.refused()
    }
    fn name(&self) -> &'static str {
        "refusable"
    }
}

/// Six months synced over IMAP with every header stored, so that Clean Up
/// widening leaves exactly "ancient" in the headers-only tier.
async fn six_months_over_imap(name: &str) -> (Arc<FakeProvider>, Db, Arc<Recorder>, SyncEngine, Arc<Refusable>) {
    let (fake, db, recorder, engine) = setup(name);
    engine.set_window(SyncWindow::HalfYear).await.unwrap();
    seed_mailbox(&fake);
    let source = Refusable::new(&fake);
    engine.set_backfill_source(source.clone());
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    while engine.headers_pass(100).await.unwrap() > 0 {}
    engine.backfill_all().await.unwrap();
    (fake, db, recorder, engine, source)
}

/// oagc-merk.8: Clean Up's header load does not turn into whole downloads
/// over the API unasked when IMAP is refused part way; it waits, IMAP
/// coming back resumes it, and the user's Load All Mail promotes it.
#[tokio::test]
async fn clean_ups_headers_wait_for_imap_when_it_is_refused_mid_load() {
    let (_fake, db, recorder, engine, source) = six_months_over_imap("cleanup-refused").await;
    let fetched = source.0.bodies();
    assert!(widened(engine.load_every_header(true).await.unwrap()));
    assert_eq!(db.read(queue::counts).await.unwrap(), (0, 1), "the older mail: headers only");
    assert!(!engine.headers_paused().await.unwrap(), "IMAP is fine");

    // Refused mid-load: the tier waits instead of becoming body fetches.
    source.refuse(true);
    let reports = recorder.progress.lock().unwrap().len();
    assert_eq!(engine.headers_pass(100).await.unwrap(), 0);
    assert_eq!(db.read(queue::counts).await.unwrap(), (0, 1), "not promoted");
    assert!(engine.headers_paused().await.unwrap());
    assert_eq!(recorder.progress.lock().unwrap().len(), reports + 1, "reported once, so the window asks");
    let last = *recorder.progress.lock().unwrap().last().unwrap();
    assert_eq!((last.phase, last.headers), (SyncPhase::Idle, 1), "waiting is idle, with the count for the band");
    assert_eq!(engine.headers_pass(100).await.unwrap(), 0);
    assert_eq!(recorder.progress.lock().unwrap().len(), reports + 1, "not again while it waits");
    assert_eq!(engine.backfill_all().await.unwrap(), 0, "no whole download");
    // A restart while refused keeps the tiering.
    engine.ensure_tiers().await.unwrap();
    assert_eq!(db.read(queue::counts).await.unwrap(), (0, 1), "still headers only after a restart");

    // IMAP back: headers only, as before.
    source.refuse(false);
    assert!(!engine.headers_paused().await.unwrap());
    assert_eq!(engine.headers_pass(100).await.unwrap(), 2, "progress: its row stored, its id done");
    assert_eq!(db.read(queue::counts).await.unwrap(), (0, 0));
    assert_eq!(body_state(&db, "ancient").as_deref(), Some("metadata"));
    assert_eq!(source.0.bodies(), fetched, "no body for the older mail");
    assert_consistent(&db);
}

#[tokio::test]
async fn load_all_mail_downloads_clean_ups_waiting_headers_whole() {
    let (_fake, db, _recorder, engine, source) = six_months_over_imap("cleanup-load-all").await;
    assert!(widened(engine.load_every_header(true).await.unwrap()));
    source.refuse(true);
    assert_eq!(engine.headers_pass(100).await.unwrap(), 0);
    assert!(engine.headers_paused().await.unwrap());

    assert_eq!(engine.load_waiting_headers().await.unwrap(), 1, "Load All Mail");
    assert!(!engine.headers_paused().await.unwrap());
    assert_eq!(db.read(queue::counts).await.unwrap(), (1, 0), "a whole download now");
    assert_eq!(engine.backfill_all().await.unwrap(), 1);
    assert_eq!(body_state(&db, "ancient").as_deref(), Some("full"));
    assert_consistent(&db);
}

/// A window the user chose (in Settings, or before Clean Up) keeps §7.4's
/// promotion when IMAP is refused.
#[tokio::test]
async fn a_window_the_user_chose_still_promotes_when_imap_is_refused() {
    let (_fake, db, _recorder, engine, source) = six_months_over_imap("cleanup-user-window").await;
    assert!(widened(engine.load_every_header(true).await.unwrap()));
    // The user picks Everything in Settings afterwards: the tier is theirs.
    engine.set_window(SyncWindow::Everything).await.unwrap();
    source.refuse(true);
    assert_eq!(engine.headers_pass(100).await.unwrap(), 0);
    assert!(!engine.headers_paused().await.unwrap());
    assert_eq!(db.read(queue::counts).await.unwrap(), (1, 0), "promoted, as before Clean Up");

    // Widened while not syncing, the account to sync over IMAP: Clean Up's.
    let (_fake, db, _recorder, engine, source) = six_months_over_imap("cleanup-stored").await;
    assert!(widened(mail_sync::load_every_header_stored(&db, true, true).await.unwrap()));
    source.refuse(true);
    engine.ensure_tiers().await.unwrap();
    assert_eq!(db.read(queue::counts).await.unwrap(), (0, 1), "listed headers only, waiting");
    assert!(mail_sync::cleanup_headers_paused(&db, false).await.unwrap());
    assert_eq!(mail_sync::load_waiting_headers_stored(&db).await.unwrap(), 1);
    assert_eq!(db.read(queue::counts).await.unwrap(), (1, 0));
}

/// A cheap-headers source that never returns some ids (moved to Spam or
/// Trash since they were listed).
struct Forgetful(CountingSource, Vec<MessageId>);

#[async_trait::async_trait]
impl provider_api::BackfillSource for Forgetful {
    async fn fetch(&self, ids: &[MessageId]) -> provider_api::ProviderResult<Vec<FetchedMessage>> {
        self.0.fetch(ids).await
    }
    async fn fetch_headers(&self, ids: &[MessageId]) -> provider_api::ProviderResult<Option<Vec<FetchedMessage>>> {
        let kept: Vec<MessageId> = ids.iter().filter(|id| !self.1.contains(id)).cloned().collect();
        self.0.fetch_headers(&kept).await
    }
    fn cheap_headers(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "forgetful"
    }
}

#[tokio::test]
async fn a_message_the_headers_pass_cannot_get_does_not_stall_backfill() {
    let (fake, db, _recorder, engine) = setup("headers-missing");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    let lost = MessageId::new("inbox-unread");
    engine.set_backfill_source(Arc::new(Forgetful(CountingSource::cheap(&fake), vec![lost.clone()])));
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    let mut passes = 0;
    while engine.headers_pass(2).await.unwrap() > 0 {
        passes += 1;
        assert!(passes < 20, "the headers pass must end");
    }
    assert!(db.read(queue::counts).await.unwrap().0 > 0, "bodies still queued, the lost one included");
    assert_eq!(engine.headers_pass(100).await.unwrap(), 0, "not asked for again");
    engine.backfill_all().await.unwrap();
    assert_eq!(body_state(&db, "inbox-unread").as_deref(), Some("full"), "the body backfill got it");
    assert_eq!(db.read(queue::len).await.unwrap(), 0);
}

#[tokio::test]
async fn ensure_bodies_downloads_header_only_mail_now() {
    let (fake, db, _recorder, engine) = setup("ensure");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    let source = Arc::new(CountingSource::cheap(&fake));
    engine.set_backfill_source(source.clone());
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    while engine.headers_pass(100).await.unwrap() > 0 {}
    assert_eq!(body_state(&db, "ancient").as_deref(), Some("metadata"));

    let wanted = vec![MessageId::new("ancient"), MessageId::new("inbox-read")];
    assert_eq!(engine.ensure_bodies(wanted.clone()).await.unwrap(), 2, "over the cheap source");
    assert_eq!(source.bodies(), 2);
    assert_eq!(body_state(&db, "ancient").as_deref(), Some("full"));
    assert!(!ids(db.read(|c| queue::peek(c, 10)).await.unwrap()).contains(&"inbox-read".to_owned()), "dequeued");
    assert_eq!(engine.ensure_bodies(wanted).await.unwrap(), 0, "already here");

    // Without IMAP, over the API.
    engine.use_rest_backfill();
    assert_eq!(engine.ensure_bodies(vec![MessageId::new("this-year")]).await.unwrap(), 1);
    assert_eq!(source.bodies(), 2);
    assert_eq!(body_state(&db, "this-year").as_deref(), Some("full"));
    assert_consistent(&db);
}

#[tokio::test]
async fn backfill_bodies_come_from_the_configured_source() {
    let (fake, db, _recorder, engine) = setup("source");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    assert_eq!(engine.backfill_source_name(), "rest");
    let source = Arc::new(CountingSource::new(&fake));
    engine.set_backfill_source(source.clone());
    assert_eq!(engine.backfill_source_name(), "counting");
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    assert_eq!(engine.backfill_all().await.unwrap(), 5);
    assert_eq!(source.bodies(), 5, "every body came through the source");
    engine.use_rest_backfill();
    assert_eq!(engine.backfill_source_name(), "rest");
    assert_eq!(db.read(queue::len).await.unwrap(), 0);
}

#[tokio::test]
async fn server_search_downloads_matches_outside_the_window() {
    let (fake, db, _recorder, engine) = setup("server-search");
    seed_mailbox(&fake);
    // Six months: "ancient" (900 days) stays on the server.
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    engine.backfill_all().await.unwrap();
    assert!(db.read(|c| read::get_thread(c, &ThreadId::new("t5"))).await.unwrap().is_none());

    assert_eq!(engine.search_server("t5", 10).await.unwrap(), 1, "the subject matches");
    let (summary, _) = db.read(|c| read::get_thread(c, &ThreadId::new("t5"))).await.unwrap().unwrap();
    assert_eq!(summary.subject, "Subject t5");
    assert_eq!(engine.search_server("t5", 10).await.unwrap(), 0, "already here");
    assert_eq!(engine.search_server("nothing-like-this", 10).await.unwrap(), 0);
    assert_consistent(&db);
}

#[tokio::test]
async fn server_search_finds_header_only_mail_by_body_text_and_downloads_it() {
    let (fake, db, _recorder, engine) = setup("server-search-tiers");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    let source = Arc::new(CountingSource::cheap(&fake));
    engine.set_backfill_source(source.clone());
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    while engine.headers_pass(100).await.unwrap() > 0 {}
    engine.backfill_all().await.unwrap();
    assert!(db.read(read::has_header_only).await.unwrap());
    let local = |q: &'static str| {
        let expr = mail_store::search::parse(q).unwrap();
        let page = db.read_blocking(move |c| mail_store::search::search(c, &expr, NOW, None, 50)).unwrap();
        page.rows.iter().map(|t| t.id.as_str().to_owned()).collect::<Vec<_>>()
    };
    // Only the body says "ancient"'s words; locally it has headers only.
    assert!(!local("\"body of ancient\"").contains(&"t5".to_owned()));

    assert_eq!(engine.search_server("body ancient", 10).await.unwrap(), 1);
    assert_eq!(body_state(&db, "ancient").as_deref(), Some("full"));
    assert_eq!(local("\"body of ancient\""), ["t5"]);
    assert_eq!(source.bodies(), 4, "downloaded over the cheap source");
    assert_consistent(&db);
}

#[tokio::test]
async fn refetch_all_queues_stored_messages_again_and_brings_labels_current() {
    let (fake, db, _recorder, engine) = setup("refetch");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    engine.backfill_all().await.unwrap();
    // The store has a wrong label set for one message (a fetch-path bug).
    db.write(|tx| {
        let mut w = mail_store::MailWriter::new(tx);
        w.modify_message_labels(&MessageId::new("inbox-read"), &[], &[LabelId::new("INBOX")])?;
        w.finish().map(|_| ())
    })
    .await
    .unwrap();
    assert_eq!(db.read(|c| read::list_threads(c, "INBOX", None, 10)).await.unwrap().rows.len(), 1);

    assert_eq!(engine.refetch_all().await.unwrap(), 5, "everything queued again");
    assert_eq!(engine.backfill_all().await.unwrap(), 5);
    assert_eq!(db.read(|c| read::list_threads(c, "INBOX", None, 10)).await.unwrap().rows.len(), 2, "label restored");
    assert_consistent(&db);
}

#[tokio::test]
async fn incremental_sync_applies_new_mail_label_changes_and_deletions() {
    let (fake, db, recorder, engine) = setup("incremental");
    seed_mailbox(&fake);
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    engine.backfill_all().await.unwrap();
    recorder.changes.lock().unwrap().clear();

    fake.deliver(message("new", "t7", 0, &["INBOX", "UNREAD"]));
    fake.relabel(&MessageId::new("inbox-unread"), &[], &[LabelId::new("UNREAD"), LabelId::new("INBOX")]);
    fake.delete(&MessageId::new("inbox-read"));

    let report = engine.sync_incremental().await.unwrap();
    assert_eq!((report.added, report.relabeled, report.deleted), (1, 1, 1));
    assert_eq!(report.new_mail.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["new"]);
    assert_eq!(report.new_mail[0].thread_id.as_str(), "t7");

    let inbox = db.read(|c| read::list_threads(c, "INBOX", None, 10)).await.unwrap();
    assert_eq!(inbox.rows.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["t7"]);
    let archive = db.read(|c| read::list_threads(c, ARCHIVE_LABEL, None, 10)).await.unwrap();
    assert!(archive.rows.iter().any(|t| t.id.as_str() == "t1"), "archived on the server shows as archived here");
    {
        let changes = recorder.changes.lock().unwrap();
        let inbox_change = &changes.last().unwrap().mailboxes["INBOX"];
        assert!(inbox_change.inserted.contains("t7"));
        assert!(inbox_change.removed.contains("t1") && inbox_change.removed.contains("t2"));
    }

    // Running again with nothing new is a no-op.
    let again = engine.sync_incremental().await.unwrap();
    assert_eq!((again.added, again.relabeled, again.deleted), (0, 0, 0));
    assert!(again.new_mail.is_empty());
    assert_consistent(&db);
}

#[tokio::test]
async fn mail_arriving_during_bootstrap_is_not_lost() {
    let (fake, db, _recorder, engine) = setup("during");
    seed_mailbox(&fake);
    engine.bootstrap_prepare().await.unwrap();
    // Arrives after the cursor was recorded but before listing finished.
    fake.deliver(message("mid-bootstrap", "t8", 0, &["INBOX"]));
    engine.bootstrap_list_rest().await.unwrap();
    engine.backfill_all().await.unwrap();
    let report = engine.sync_incremental().await.unwrap();
    assert!(report.new_mail.is_empty(), "already stored by the bootstrap: not announced again");
    let inbox = db.read(|c| read::list_threads(c, "INBOX", None, 10)).await.unwrap();
    assert!(inbox.rows.iter().any(|t| t.id.as_str() == "t8"));
    assert_consistent(&db);
}

#[tokio::test]
async fn label_changes_for_unfetched_messages_queue_a_fetch() {
    let (fake, db, _recorder, engine) = setup("unfetched");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    engine.bootstrap_prepare().await.unwrap(); // inbox queued, nothing fetched
    fake.relabel(&MessageId::new("ancient"), &[LabelId::new("STARRED")], &[]);
    engine.sync_incremental().await.unwrap();
    let queued = db.read(|c| queue::peek(c, 1)).await.unwrap();
    assert_eq!(queued, vec![MessageId::new("ancient")], "urgent: the user just touched it elsewhere");
    engine.backfill_batch(1).await.unwrap();
    let (summary, _) = db.read(|c| read::get_thread(c, &ThreadId::new("t5"))).await.unwrap().unwrap();
    assert!(summary.is_starred, "fetched with current labels");
}

#[tokio::test]
async fn an_expired_cursor_triggers_a_full_resync() {
    let (fake, db, _recorder, engine) = setup("expired");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    engine.backfill_all().await.unwrap();

    // A week offline: history is gone and labels changed meanwhile.
    fake.relabel(&MessageId::new("inbox-read"), &[LabelId::new("STARRED")], &[]);
    fake.expire_history();
    let err = engine.sync_incremental().await.unwrap_err();
    assert!(matches!(err, SyncError::ResyncStarted));
    assert_eq!(db.read(queue::len).await.unwrap(), 5, "everything re-queued");
    engine.backfill_all().await.unwrap();
    let (summary, _) = db.read(|c| read::get_thread(c, &ThreadId::new("t2"))).await.unwrap().unwrap();
    assert!(summary.is_starred, "resync picked up the missed change");
    // And incremental works again from the fresh cursor.
    engine.sync_incremental().await.unwrap();
    assert_consistent(&db);
}

#[tokio::test]
async fn incremental_before_bootstrap_is_an_error() {
    let (_fake, _db, _recorder, engine) = setup("early");
    assert!(matches!(engine.sync_incremental().await.unwrap_err(), SyncError::NotBootstrapped));
}

#[tokio::test]
async fn only_unread_inbox_mail_from_others_is_new_mail() {
    let (fake, _db, _recorder, engine) = setup("new-mail");
    seed_mailbox(&fake);
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    engine.backfill_all().await.unwrap();
    fake.deliver(message("hello", "t20", 0, &["INBOX", "UNREAD"]));
    fake.deliver(message("already-read", "t21", 0, &["INBOX"]));
    fake.deliver(message("mine", "t22", 0, &["SENT", "INBOX", "UNREAD"]));
    fake.deliver(message("junk", "t23", 0, &["SPAM", "UNREAD"]));
    fake.deliver(message("filtered", "t24", 0, &["UNREAD", "Label_1"]));
    let report = engine.sync_incremental().await.unwrap();
    assert_eq!(report.new_mail.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["hello"]);
}

#[tokio::test]
async fn label_changes_made_elsewhere_are_reported_and_our_own_are_not() {
    let (fake, _db, _recorder, engine) = setup("external");
    seed_mailbox(&fake);
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    engine.backfill_all().await.unwrap();

    // Something else (a cloud routine) files a message and archives it.
    fake.relabel(&MessageId::new("inbox-unread"), &[LabelId::new("Label_7")], &[LabelId::new("INBOX")]);
    // OpenAGC archives another through its outbox.
    engine.apply_change(mail_sync::LocalChange::archive(vec![ThreadId::new("t2")]), true).await.unwrap();
    engine.drain_outbox().await.unwrap();

    let report = engine.sync_incremental().await.unwrap();
    assert_eq!(report.external_label_changes.len(), 1, "{:?}", report.external_label_changes);
    let change = &report.external_label_changes[0];
    assert_eq!(change.message.as_str(), "inbox-unread");
    assert_eq!(change.thread.as_str(), "t1");
    assert_eq!(change.added, vec![LabelId::new("Label_7")]);
    assert_eq!(change.removed, vec![LabelId::new("INBOX")]);
}

#[tokio::test]
async fn drafts_sync_through_the_drafts_list_whatever_the_window() {
    let (fake, db, _recorder, engine) = setup("drafts");
    engine.set_window(SyncWindow::Month).await.unwrap();
    seed_mailbox(&fake);
    // Written on the web a year ago: outside the window, and Gmail's
    // history never mentions drafts.
    let mut old = message("draft-old", "t-draft", 400, &[]);
    old.subject = "Plan for next year".into();
    fake.seed_draft("r-web1", old);
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    engine.backfill_all().await.unwrap();

    assert_eq!(engine.sync_drafts().await.unwrap(), 1, "the draft's message came down");
    let drafts = db.read(|c| read::list_threads(c, "DRAFT", None, 10)).await.unwrap();
    assert_eq!(drafts.rows.iter().map(|t| t.subject.as_str()).collect::<Vec<_>>(), ["Plan for next year"]);
    assert_eq!(
        db.read(|c| mail_store::drafts::server_draft_for_message(c, "draft-old")).await.unwrap().as_deref(),
        Some("r-web1"),
        "which draft holds it, for editing"
    );
    assert_eq!(engine.sync_drafts().await.unwrap(), 0, "nothing new the second time");

    // Sent or discarded elsewhere: the draft's message goes.
    use provider_api::MailProvider;
    fake.delete_draft("r-web1").await.unwrap();
    engine.sync_drafts().await.unwrap();
    assert!(db.read(|c| read::list_threads(c, "DRAFT", None, 10)).await.unwrap().rows.is_empty());
    assert_consistent(&db);
}

/// An IMAP source whose connection keeps failing.
struct BrokenImap(std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl provider_api::BackfillSource for BrokenImap {
    async fn fetch(&self, _ids: &[MessageId]) -> provider_api::ProviderResult<Vec<FetchedMessage>> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(provider_api::ProviderError::Network("IMAP connection timed out".into()))
    }
    fn cheap_headers(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "imap"
    }
}

#[tokio::test]
async fn a_failing_imap_falls_back_to_the_api_per_batch_and_trips_the_breaker() {
    use mail_sync::transport::{BREAKER_FAILURES, Job, Via};
    let (fake, db, _recorder, engine) = setup("imap-breaker");
    engine.set_window(SyncWindow::Everything).await.unwrap();
    seed_mailbox(&fake);
    fake.seed(message("recent-too", "t7", 5, &[])); // a fourth body, after the breaker opens
    let broken = Arc::new(BrokenImap(Default::default()));
    engine.set_backfill_source(broken.clone());
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    // One message per batch, so every batch asks IMAP first.
    while engine.backfill_batch(1).await.unwrap() > 0 {}
    assert_eq!(db.read(queue::counts).await.unwrap().0, 0, "every body came down over the API");
    assert_eq!(
        broken.0.load(std::sync::atomic::Ordering::SeqCst),
        BREAKER_FAILURES as usize,
        "after three failures IMAP is not asked again for a while"
    );

    let snap = engine.transport_snapshot();
    assert!(snap.breaker_open_until.is_some());
    assert!(snap.last_imap_error.as_deref().is_some_and(|e| e.contains("timed out")));
    let bodies: Vec<_> = snap.recent.iter().filter(|r| r.job == Job::Bodies).collect();
    assert!(bodies.iter().all(|r| r.via == Via::Api && r.ok));
    assert!(bodies.last().unwrap().reason.as_deref().unwrap().starts_with("IMAP failed"));
    assert!(bodies[0].reason.as_deref().unwrap().starts_with("IMAP paused"), "then the breaker's reason");
    assert!(snap.recent.iter().any(|r| r.job == Job::List && r.via == Via::Api), "listing is recorded too");

    engine.reset_transport();
    assert!(engine.transport_snapshot().breaker_open_until.is_none(), "a refresh tries IMAP again");
    assert_consistent(&db);
}

#[tokio::test]
async fn inbox_categories_missing_from_downloaded_mail_are_applied_and_the_inbox_counts_primary() {
    let (fake, db, _recorder, engine) = setup("categories");
    fake.seed(message("promo", "t1", 1, &["INBOX", "UNREAD", "CATEGORY_PROMOTIONS"]));
    fake.seed(message("social", "t2", 1, &["INBOX", "UNREAD", "CATEGORY_SOCIAL"]));
    fake.seed(message("friend", "t3", 1, &["INBOX", "UNREAD"]));
    fake.seed(message("old-promo", "t4", 1, &["CATEGORY_PROMOTIONS"]));
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    engine.backfill_all().await.unwrap();
    // As if downloaded over IMAP, which carries no categories.
    db.write(|tx| {
        let mut w = mail_store::MailWriter::new(tx);
        for id in ["promo", "social"] {
            w.modify_message_labels(
                &MessageId::new(id),
                &[],
                &[LabelId::new("CATEGORY_PROMOTIONS"), LabelId::new("CATEGORY_SOCIAL")],
            )?;
        }
        w.finish()
    })
    .await
    .unwrap();
    let inbox_unread = |db: Db| async move {
        let boxes = db.read(read::list_mailboxes).await.unwrap();
        boxes.iter().find(|m| read::mailbox_label(m) == "INBOX").unwrap().unread_count
    };
    assert_eq!(inbox_unread(db.clone()).await, 3, "no categories known: everything is Primary");

    assert_eq!(engine.sync_categories().await.unwrap(), 2, "promo and social; the archived one is left alone");
    let counts = db.read(|c| read::inbox_categories(c, &[])).await.unwrap();
    let unread = |id: &str| counts.iter().find(|c| c.id == id).unwrap().unread;
    assert_eq!((unread("CATEGORY_PERSONAL"), unread("CATEGORY_PROMOTIONS"), unread("CATEGORY_SOCIAL")), (1, 1, 1));
    assert_eq!(inbox_unread(db.clone()).await, 1, "with categories, the Inbox counts Primary, as Gmail does");
    assert_eq!(engine.sync_categories().await.unwrap(), 0, "nothing left to change");
    let snap = engine.transport_snapshot();
    let op = snap.latest_by_job().into_iter().find(|r| r.job == mail_sync::transport::Job::Categories).unwrap();
    assert_eq!(op.via, mail_sync::transport::Via::Api, "no IMAP here: the API lists them");
    assert_consistent(&db);
}

#[tokio::test]
async fn a_thread_you_replied_in_is_marked_replied() {
    let (fake, db, _recorder, engine) = setup("replied");
    fake.seed(message("asked", "t1", 3, &["INBOX"]));
    fake.seed(message("answered", "t1", 2, &["SENT"]));
    fake.seed(message("started", "t2", 3, &["SENT"]));
    fake.seed(message("their-answer", "t2", 2, &["INBOX"]));
    fake.seed(message("alone", "t3", 1, &["INBOX"]));
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    engine.backfill_all().await.unwrap();
    let page = db.read(|c| read::list_threads(c, "INBOX", None, 10)).await.unwrap();
    let replied = |id: &str| page.rows.iter().find(|r| r.id.as_str() == id).unwrap().replied;
    assert!(replied("t1"), "you answered them");
    assert!(!replied("t2"), "you started it; they answered");
    assert!(!replied("t3"));
    let summary = db.read(|c| read::get_thread_summary(c, &ThreadId::new("t1"))).await.unwrap().unwrap();
    assert!(summary.replied);
}

/// The window decided to widen without asking because headers were cheap;
/// IMAP was refused before the call: nothing changes, and the answer says
/// to ask, so no whole download over the API starts unasked.
#[tokio::test]
async fn a_widening_decided_on_cheap_headers_asks_if_imap_was_refused_since() {
    let (_fake, db, _recorder, engine, source) = six_months_over_imap("cleanup-refused-before").await;
    engine.set_body_window(BodyWindow::Window).await.unwrap();
    let before = db.read(queue::counts).await.unwrap();
    source.refuse(true);
    assert_eq!(engine.load_every_header(true).await.unwrap(), EveryHeader::NeedsAsk);
    assert_eq!(engine.window().await.unwrap(), SyncWindow::HalfYear, "unchanged");
    assert_eq!(engine.body_window().await.unwrap(), BodyWindow::Window, "unchanged");
    assert_eq!(db.read(queue::counts).await.unwrap(), before, "nothing listed");

    // The user answered Load All Mail: the whole download, and the body
    // window, moot over the API, stays as the user set it.
    assert_eq!(engine.load_every_header(false).await.unwrap(), EveryHeader::Widened { body_window: None });
    assert_eq!(engine.window().await.unwrap(), SyncWindow::Everything);
    assert_eq!(engine.body_window().await.unwrap(), BodyWindow::Window);
    assert!(!engine.headers_paused().await.unwrap(), "not Clean Up's waiting tier: the user agreed");

    // Not syncing: the same, with the account's IMAP grant.
    let (_fake, db, _recorder, engine, _source) = six_months_over_imap("cleanup-refused-stored").await;
    assert_eq!(mail_sync::load_every_header_stored(&db, false, true).await.unwrap(), EveryHeader::NeedsAsk);
    assert_eq!(engine.window().await.unwrap(), SyncWindow::HalfYear);
}
