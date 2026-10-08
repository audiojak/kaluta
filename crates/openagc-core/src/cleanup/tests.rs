//! Clean Up's apply against `FakeProvider`: message-level changes, one undo
//! entry per apply, batches of 1,000 at the provider, and groups resolved
//! when the action runs.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::executor::block_on;
use mail_domain::{EmailAddress, Label, LabelId, LabelKind, MessageId, ThreadId};
use provider_api::fake::FakeProvider;
use provider_api::{FetchedBody, FetchedMessage};

use super::*;
use crate::account::SyncWindow;
use crate::{CoreConfig, CoreEvent, EventListener};

const NOW: i64 = 1_790_000_000_000;

#[derive(Default)]
struct Recorder(Mutex<Vec<CoreEvent>>);
impl EventListener for Recorder {
    fn on_event(&self, _account: Option<String>, event: CoreEvent) {
        self.0.lock().unwrap().push(event);
    }
}

struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn core(name: &str) -> (Temp, Arc<Core>, Arc<Recorder>) {
    let dir = std::env::temp_dir().join(format!("openagc-cleanup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let events = Arc::new(Recorder::default());
    let core = Core::new(
        CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
        Arc::new(crate::secrets::MemorySecrets::default()),
        events.clone(),
    )
    .unwrap();
    (Temp(dir), core, events)
}

fn message(id: &str, thread: &str, from: &str, labels: &[&str]) -> FetchedMessage {
    FetchedMessage {
        id: MessageId::new(id),
        thread_id: ThreadId::new(thread),
        label_ids: labels.iter().map(|l| LabelId::new(*l)).collect(),
        internal_date: NOW,
        from: Some(EmailAddress::new(None, from)),
        subject: format!("from {from}"),
        body: Some(FetchedBody { text: Some("hi".into()), html: None, attachments: vec![] }),
        ..Default::default()
    }
}

/// A synced account "acct" over `fake`, once `messages` are stored.
fn synced(name: &str, fake: &Arc<FakeProvider>, messages: u64) -> (Temp, Arc<Core>, Arc<Recorder>) {
    let (temp, core, events) = core(name);
    block_on(core.clone().open_account("acct".into())).unwrap();
    core.start_sync_with(fake.clone()).unwrap();
    wait_until("the mailbox synced", || all_mail_count(&core) == messages);
    (temp, core, events)
}

fn all_mail_count(core: &Core) -> u64 {
    let groups = block_on(core.cleanup_groups("acct".into(), CleanupView::Size, CleanupScope::AllMail, String::new()));
    groups.map(|g| g.iter().map(|g| g.count).sum()).unwrap_or(0)
}

fn sender(email: &str) -> Vec<String> {
    vec![email.to_owned()]
}

fn apply(core: &Core, scope: CleanupScope, keys: Vec<String>, action: CleanupAction) -> CleanupResult {
    block_on(core.cleanup_apply("acct".into(), CleanupView::Sender, scope, keys, action)).unwrap()
}

fn labels_of(fake: &FakeProvider, id: &str) -> Vec<String> {
    let mut labels: Vec<String> =
        fake.message(&MessageId::new(id)).unwrap().label_ids.into_iter().map(|l| l.0).collect();
    labels.sort();
    labels
}

fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..1_500 {
        if ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn a_trash_of_2500_messages_goes_out_in_three_batches_and_comes_back_in_three() {
    let fake = Arc::new(FakeProvider::new("me@example.com", NOW, 500));
    for i in 0..2_500 {
        fake.seed(message(&format!("m{i:04}"), &format!("t{i:04}"), "deals@shop.example", &["INBOX", "UNREAD"]));
    }
    fake.seed(message("keep", "tkeep", "friend@example.com", &["INBOX"]));
    let (_temp, core, events) = synced("trash", &fake, 2_501);
    let groups =
        block_on(core.cleanup_groups("acct".into(), CleanupView::Sender, CleanupScope::Inbox, String::new())).unwrap();
    assert_eq!((groups[0].key.as_str(), groups[0].count), ("deals@shop.example", 2_500));

    let done = apply(&core, CleanupScope::Inbox, sender("deals@shop.example"), CleanupAction::Trash);
    assert_eq!(done.changed, 2_500);
    assert_eq!(done.description, "Moved 2,500 messages from deals@shop.example to the Trash");
    assert_eq!(done.action_name, "Move to Trash");
    let token = done.undo.expect("one undo entry");
    let inbox = |core: &Core| {
        block_on(core.cleanup_count(
            "acct".into(),
            CleanupView::Sender,
            CleanupScope::Inbox,
            sender("deals@shop.example"),
        ))
        .unwrap()
    };
    assert_eq!(inbox(&core), 0, "local at once");

    let sent = |ops: Vec<provider_api::LabelOp>| ops.iter().map(|op| op.message_ids.len()).sum::<usize>();
    wait_until("the server trashed them", || sent(fake.label_ops()) == 2_500);
    assert!((0..2_500).all(|i| labels_of(&fake, &format!("m{i:04}")) == ["TRASH", "UNREAD"]));
    let sizes = |ops: &[provider_api::LabelOp]| ops.iter().map(|op| op.message_ids.len()).collect::<Vec<_>>();
    let ops = fake.label_ops();
    assert_eq!(sizes(&ops), [1_000, 1_000, 500], "batchModify's limit, one call per batch");
    assert!(ops.iter().all(|op| op.add == [LabelId::new("TRASH")] && op.remove == [LabelId::new("INBOX")]));
    assert_eq!(labels_of(&fake, "keep"), ["INBOX"], "other senders untouched");
    assert!(
        events.0.lock().unwrap().iter().any(|e| matches!(e, CoreEvent::OutboxStatus { pending: 1.., .. })),
        "progress between batches"
    );

    block_on(core.undo_action(token.clone())).unwrap();
    assert_eq!(inbox(&core), 2_500, "undo is local at once too");
    wait_until("the server untrashed them", || sent(fake.label_ops()) == 5_000);
    let undo_ops = fake.label_ops()[3..].to_vec();
    assert_eq!(sizes(&undo_ops), [1_000, 1_000, 500]);
    assert!(undo_ops.iter().all(|op| op.add == [LabelId::new("INBOX")] && op.remove == [LabelId::new("TRASH")]));
    assert!((0..2_500).all(|i| labels_of(&fake, &format!("m{i:04}")) == ["INBOX", "UNREAD"]));

    // Redo trashes them again, in batches as well.
    block_on(core.redo_action(token)).unwrap();
    wait_until("the server trashed them again", || sent(fake.label_ops()) == 7_500);
    assert!((0..2_500).all(|i| labels_of(&fake, &format!("m{i:04}")) == ["TRASH", "UNREAD"]));
    assert_eq!(sizes(&fake.label_ops()[6..]), [1_000, 1_000, 500]);
    core.stop_sync();
}

#[test]
fn archive_then_undo_restores_the_inbox_exactly() {
    let fake = Arc::new(FakeProvider::new("me@example.com", NOW, 50));
    fake.seed(message("a1", "ta1", "news@paper.example", &["INBOX", "UNREAD"]));
    fake.seed(message("a2", "ta2", "news@paper.example", &["INBOX", "STARRED"]));
    // Already archived: in All Mail, not in the Inbox.
    fake.seed(message("a3", "ta3", "News@Paper.example", &["Label_7"]));
    let (_temp, core, _events) = synced("archive", &fake, 3);

    let done = apply(&core, CleanupScope::AllMail, sender("news@paper.example"), CleanupAction::Archive);
    assert_eq!(done.changed, 2, "the archived one changed nothing");
    assert_eq!(done.description, "Archived 2 messages from news@paper.example");
    wait_until("the server archived them", || labels_of(&fake, "a1") == ["UNREAD"]);
    assert_eq!(labels_of(&fake, "a2"), ["STARRED"]);

    block_on(core.undo_action(done.undo.unwrap())).unwrap();
    wait_until("the server has them back", || labels_of(&fake, "a1") == ["INBOX", "UNREAD"]);
    assert_eq!(labels_of(&fake, "a2"), ["INBOX", "STARRED"]);
    assert_eq!(labels_of(&fake, "a3"), ["Label_7"], "undo does not put the archived one in the Inbox");
    let inbox = block_on(core.cleanup_count(
        "acct".into(),
        CleanupView::Sender,
        CleanupScope::Inbox,
        sender("news@paper.example"),
    ))
    .unwrap();
    assert_eq!(inbox, 2);

    // Nothing left to change: no undo entry.
    let again = apply(&core, CleanupScope::Inbox, sender("nobody@example.com"), CleanupAction::Archive);
    assert_eq!((again.changed, again.undo), (0, None));
    core.stop_sync();
}

#[test]
fn spam_and_its_undo_reach_the_server() {
    let fake = Arc::new(FakeProvider::new("me@example.com", NOW, 50));
    fake.seed(message("s1", "ts1", "spammy@bulk.example", &["INBOX", "UNREAD"]));
    fake.seed(message("s2", "ts2", "spammy@bulk.example", &["INBOX"]));
    let (_temp, core, _events) = synced("spam", &fake, 2);

    let done = apply(&core, CleanupScope::Inbox, sender("spammy@bulk.example"), CleanupAction::Spam);
    assert_eq!(done.description, "Moved 2 messages from spammy@bulk.example to Spam");
    let spam = block_on(core.list_threads("SPAM".into(), None, 10)).unwrap().rows.len();
    assert_eq!(spam, 2);
    wait_until("the server marked them spam", || labels_of(&fake, "s1") == ["SPAM", "UNREAD"]);

    block_on(core.undo_action(done.undo.unwrap())).unwrap();
    assert!(block_on(core.list_threads("SPAM".into(), None, 10)).unwrap().rows.is_empty());
    wait_until("the server undid it", || labels_of(&fake, "s1") == ["INBOX", "UNREAD"]);
    assert_eq!(labels_of(&fake, "s2"), ["INBOX"]);
    core.stop_sync();
}

#[test]
fn a_group_is_acted_on_as_it_is_when_the_action_runs() {
    let fake = Arc::new(FakeProvider::new("me@example.com", NOW, 50));
    for i in 0..3 {
        fake.seed(message(&format!("g{i}"), &format!("tg{i}"), "promo@store.example", &["INBOX"]));
    }
    let (_temp, core, _events) = synced("changes", &fake, 3);
    let shown =
        block_on(core.cleanup_groups("acct".into(), CleanupView::Sender, CleanupScope::Inbox, String::new())).unwrap();
    assert_eq!(shown[0].count, 3);

    // Between showing and acting: one more arrives, one is archived on the web.
    fake.deliver(message("g3", "tg3", "promo@store.example", &["INBOX"]));
    fake.relabel(&MessageId::new("g0"), &[], &[LabelId::new("INBOX")]);
    core.sync_now();
    wait_until("the changes synced", || {
        block_on(core.cleanup_messages(
            "acct".into(),
            CleanupView::Sender,
            CleanupScope::Inbox,
            sender("promo@store.example"),
            0,
            10,
        ))
        .unwrap()
        .iter()
        .map(|m| m.id.clone())
        .collect::<std::collections::BTreeSet<_>>()
            == ["g1", "g2", "g3"].map(String::from).into()
    });

    let done = apply(&core, CleanupScope::Inbox, sender("promo@store.example"), CleanupAction::Archive);
    assert_eq!(done.changed, 3, "the new one is in, the archived one is not");
    let token = done.undo.unwrap();
    let action_id = token.action_id;
    let recorded = core.db().unwrap().read_blocking(|c| mail_store::undo::get(c, action_id)).unwrap().unwrap();
    let mut ids: Vec<&str> = recorded.diffs.iter().map(|d| d.message.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["g1", "g2", "g3"], "the undo entry records exactly what changed");
    assert!(recorded.diffs.iter().all(|d| d.added.is_empty() && d.removed == [LabelId::new("INBOX")]));
    assert_eq!(recorded.kind, "cleanup_archive");
    wait_until("the server archived them", || labels_of(&fake, "g3").is_empty());

    block_on(core.undo_action(token)).unwrap();
    wait_until("the server has them back", || labels_of(&fake, "g3") == ["INBOX"]);
    assert_eq!(labels_of(&fake, "g0"), Vec::<String>::new(), "the one archived elsewhere stays archived");
    core.stop_sync();
}

#[test]
fn only_the_groups_messages_change_in_a_mixed_thread() {
    let fake = Arc::new(FakeProvider::new("me@example.com", NOW, 50));
    fake.set_labels(vec![Label {
        id: LabelId::new("Label_1"),
        name: "Receipts".into(),
        kind: LabelKind::User,
        color: None,
        visible: true,
    }]);
    fake.seed(message("order", "mixed", "orders@shop.example", &["INBOX"]));
    let mut reply = message("reply", "mixed", "friend@example.com", &["INBOX", "UNREAD"]);
    reply.internal_date = NOW + 1;
    fake.seed(reply);
    let (_temp, core, _events) = synced("mixed", &fake, 2);

    let done = apply(
        &core,
        CleanupScope::Inbox,
        sender("orders@shop.example"),
        CleanupAction::Move { label_id: "Label_1".into() },
    );
    assert_eq!(done.changed, 1);
    assert_eq!(done.description, "Moved 1 message from orders@shop.example to “Receipts”");
    // The thread stays in the Inbox for the reply, and is under Receipts too.
    let inbox = block_on(core.list_threads("INBOX".into(), None, 10)).unwrap().rows;
    assert_eq!(inbox.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["mixed"]);
    let receipts = block_on(core.list_threads("Label_1".into(), None, 10)).unwrap().rows;
    assert_eq!(receipts.len(), 1);
    wait_until("the server moved it", || labels_of(&fake, "order") == ["Label_1"]);
    assert_eq!(labels_of(&fake, "reply"), ["INBOX", "UNREAD"], "the person's reply stays where it is");

    // Labels that are not places to move to are refused.
    for label in ["TRASH", "SENT", "UNREAD"] {
        let err = block_on(core.cleanup_apply(
            "acct".into(),
            CleanupView::Sender,
            CleanupScope::AllMail,
            sender("friend@example.com"),
            CleanupAction::Move { label_id: label.into() },
        ))
        .unwrap_err();
        assert!(matches!(err.kind(), crate::ErrorKind::InvalidInput | crate::ErrorKind::NotFound), "{label}");
    }
    core.stop_sync();
}

#[test]
fn the_notice_names_the_group_or_counts_them() {
    assert_eq!(which(CleanupView::Sender, 813, 1, Some("Amazon")), "813 messages from Amazon");
    assert_eq!(which(CleanupView::Sender, 12_345, 3, Some("ignored")), "12,345 messages from 3 groups");
    assert_eq!(which(CleanupView::Size, 20, 1, Some("Extra Large")), "20 extra large messages");
    assert_eq!(which(CleanupView::Size, 1, 1, Some("Tiny")), "1 tiny message");
    assert_eq!(which(CleanupView::Subject, 2, 1, Some("Weekly digest")), "2 messages with the subject “Weekly digest”");
    assert_eq!(which(CleanupView::Subject, 2, 1, Some("(no subject)")), "2 messages with no subject");
    assert_eq!(
        which(CleanupView::Time, 1_000_000, 1, Some("September 2026")),
        "1,000,000 messages from September 2026"
    );
}

#[test]
fn the_demo_mailbox_cleans_up_locally() {
    let (_temp, core, _events) = core("demo");
    block_on(core.clone().open_account("demo".into())).unwrap();
    block_on(core.debug_seed_demo_mailbox(80)).unwrap();
    let groups =
        block_on(core.cleanup_groups("demo".into(), CleanupView::Sender, CleanupScope::Inbox, String::new())).unwrap();
    let biggest = groups[0].clone();
    let done = block_on(core.cleanup_apply(
        "demo".into(),
        CleanupView::Sender,
        CleanupScope::Inbox,
        vec![biggest.key.clone()],
        CleanupAction::Archive,
    ))
    .unwrap();
    assert_eq!(done.changed, biggest.count);
    assert_eq!(done.description, format!("Archived {} from {}", messages(biggest.count as usize), biggest.title));
    let count = |core: &Core| {
        block_on(core.cleanup_count("demo".into(), CleanupView::Sender, CleanupScope::Inbox, vec![biggest.key.clone()]))
            .unwrap()
    };
    assert_eq!(count(&core), 0);
    assert_eq!(block_on(core.outbox_status()).unwrap().pending, 0, "no provider, no outbox");
    block_on(core.undo_action(done.undo.unwrap())).unwrap();
    assert_eq!(count(&core), biggest.count);
}

/// Registers "acct" as a Gmail account, with IMAP granted or not.
fn register_gmail(core: &Core, imap: bool) {
    crate::registry::save_index(
        &core.data_path(),
        &[crate::registry::IndexEntry {
            id: "acct".into(),
            kind: crate::registry::AccountKind::Gmail,
            email: "me@example.com".into(),
            display_name: None,
            avatar_file: None,
            added_at: 0,
            imap: imap.then_some(true),
            named_by_user: false,
            service: None,
        }],
    )
    .unwrap();
}

/// A message `age_days` old, as the API lists it (no body: IMAP has it).
fn aged(id: &str, age_days: i64, labels: &[&str]) -> FetchedMessage {
    let mut m = message(id, &format!("t-{id}"), "news@example.com", labels);
    m.internal_date = NOW - age_days * 86_400_000;
    m
}

fn body_state(core: &Core, id: &str) -> Option<String> {
    let db = block_on(core.store_for("acct")).unwrap();
    let id = id.to_owned();
    db.read_blocking(move |c| {
        Ok(c.query_row("SELECT body_state FROM messages WHERE gmail_id = ?1", [id], |r| r.get(0)).ok())
    })
    .unwrap()
}

/// The five messages of the IMAP tests, aged in days: the Inbox, the last
/// 30 days, the rest of six months, last year and older.
const IMAP_AGES: [(&str, i64, bool); 5] =
    [("inbox", 1, true), ("recent", 10, false), ("spring", 100, false), ("lastyear", 250, false), ("old", 900, false)];

fn imap_id(n: usize) -> u64 {
    0x1a0000000000001u64 + n as u64
}

fn imap_hex(n: usize) -> String {
    format!("{:x}", imap_id(n))
}

/// The IMAP fake and the API's fake holding the same mailbox.
async fn imap_mailbox() -> (provider_gmail::imap_fake::FakeImapServer, Arc<FakeProvider>) {
    use provider_gmail::imap_fake::{FakeImapMessage, FakeImapServer};
    let server = FakeImapServer::start("tok").await;
    server.set_now(NOW);
    let rest = Arc::new(FakeProvider::new("me@example.com", NOW, 50));
    for (n, (name, age, inbox)) in IMAP_AGES.iter().enumerate() {
        let labels: &[&str] = if *inbox { &["INBOX"] } else { &[] };
        // With its body: over the API a message comes down whole.
        rest.seed(aged(&imap_hex(n), *age, labels));
        let date = chrono::DateTime::from_timestamp_millis(NOW - age * 86_400_000).unwrap().to_rfc2822();
        server.add(FakeImapMessage {
            uid: n as u32 + 1,
            msgid: imap_id(n),
            thrid: imap_id(n),
            labels: if *inbox { vec!["\\Inbox".into()] } else { vec![] },
            flags: vec!["\\Seen".into()],
            raw: format!(
                "From: News <news@example.com>\r\nTo: me@example.com\r\nSubject: {name}\r\nMessage-ID: <{name}@example.com>\r\nDate: {date}\r\n\r\nBody of {name}\r\n"
            )
            .into_bytes(),
        });
    }
    (server, rest)
}

/// Start syncing "acct" with a fresh IMAP source on `server`.
fn start_imap(
    core: &Arc<Core>,
    server: &provider_gmail::imap_fake::FakeImapServer,
    rest: &Arc<FakeProvider>,
    refusal_lasts: Duration,
) {
    use provider_gmail::imap::{ImapConfig, ImapEndpoint};
    let config =
        ImapConfig { endpoint: ImapEndpoint::Plain(server.addr), refusal_lasts, ..ImapConfig::gmail("me@example.com") };
    let imap = core.imap_source("acct", config, Arc::new(provider_api::token::StaticToken("tok".into())), rest.clone());
    core.start_sync_with_backfill(rest.clone(), imap).unwrap();
}

/// Wait until `stored` messages are listed and nothing is queued.
async fn settled(core: &Arc<Core>, stored: u64) {
    for _ in 0..400 {
        let status = core.cleanup_load_status("acct".into()).await.unwrap();
        let groups =
            core.cleanup_groups("acct".into(), CleanupView::Size, CleanupScope::AllMail, String::new()).await.unwrap();
        if groups.iter().map(|g| g.count).sum::<u64>() == stored && status.headers_waiting + status.bodies_waiting == 0
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("sync did not settle at {stored} messages");
}

/// Wait until `ok` holds for the account's load status.
async fn status_until(core: &Arc<Core>, what: &str, ok: impl Fn(&CleanupLoadStatus) -> bool) -> CleanupLoadStatus {
    for _ in 0..400 {
        let status = core.cleanup_load_status("acct".into()).await.unwrap();
        if ok(&status) {
            return status;
        }
        core.sync_now();
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn opening_clean_up_over_imap_loads_every_header_and_no_body_beyond_the_body_window() {
    let (_temp, core, _events) = core("load-imap");
    register_gmail(&core, true);
    block_on(core.clone().open_account("acct".into())).unwrap();
    // Six months (the default) and bodies for the last 30 days (the default).
    crate::runtime::runtime().block_on(async {
        let (server, rest) = imap_mailbox().await;
        start_imap(&core, &server, &rest, Duration::from_secs(3600));
        settled(&core, 3).await;
        assert_eq!(body_state(&core, &imap_hex(0)).as_deref(), Some("full"), "the Inbox in full");
        assert_eq!(body_state(&core, &imap_hex(1)).as_deref(), Some("full"), "the last 30 days in full");
        assert_eq!(body_state(&core, &imap_hex(2)).as_deref(), Some("metadata"), "the rest of six months: headers");
        assert_eq!(body_state(&core, &imap_hex(4)), None, "outside the window");
        let bodies = server.body_fetches();

        let status = core.cleanup_load_status("acct".into()).await.unwrap();
        assert!(status.has_sync_window && status.cheap_headers && !status.headers_paused, "{status:?}");
        assert_eq!(status.window, SyncWindow::HalfYear);

        // Opening Clean Up: every header, no body.
        assert!(core.cleanup_load_every_header("acct".into()).await.unwrap());
        settled(&core, 5).await;
        assert_eq!(core.cleanup_load_status("acct".into()).await.unwrap().window, SyncWindow::Everything);
        assert_eq!(core.body_window_for("acct".into()).await.unwrap(), crate::account::BodyWindow::Month, "unchanged");
        assert_eq!(body_state(&core, &imap_hex(3)).as_deref(), Some("metadata"));
        assert_eq!(body_state(&core, &imap_hex(4)).as_deref(), Some("metadata"));
        assert_eq!(server.body_fetches(), bodies, "no body downloaded for the older mail");
        assert_eq!(rest.fetch_calls.load(std::sync::atomic::Ordering::SeqCst), 0, "nothing over the API");
        assert!(!core.cleanup_load_every_header("acct".into()).await.unwrap(), "already everything");
    });
    core.stop_sync();
}

/// oagc-merk.8: IMAP refused after Clean Up began loading every header.
/// The headers wait rather than coming down whole over the API unasked;
/// the window asks with the count and the time; Load All Mail downloads
/// them whole.
#[test]
fn imap_refused_mid_load_pauses_clean_ups_headers_until_load_all_mail() {
    let (_temp, core, _events) = core("load-imap-refused");
    register_gmail(&core, true);
    block_on(core.clone().open_account("acct".into())).unwrap();
    crate::runtime::runtime().block_on(async {
        let (server, rest) = imap_mailbox().await;
        start_imap(&core, &server, &rest, Duration::from_secs(3600));
        settled(&core, 3).await;
        // Clean Up widens while the account is between syncs, then Gmail
        // refuses IMAP as the header load starts.
        core.stop_sync();
        assert!(core.cleanup_load_every_header("acct".into()).await.unwrap());
        server.refuse_logins();
        start_imap(&core, &server, &rest, Duration::from_secs(3600));
        let status = status_until(&core, "the header load to wait", |s| s.headers_paused).await;
        assert!(!status.cheap_headers);
        assert_eq!((status.headers_waiting, status.bodies_waiting), (2, 0), "last year and older: headers only");
        assert!(server.refusals() > 0);
        let estimate = core.cleanup_load_estimate("acct".into()).await.unwrap();
        assert_eq!(estimate.messages, Some(2), "what is left, not Gmail's count");
        assert_eq!(estimate.seconds, Some(provider_gmail::rest_download_seconds(2)));
        assert_eq!(rest.fetch_calls.load(std::sync::atomic::Ordering::SeqCst), 0, "nothing downloaded unasked");
        assert_eq!(body_state(&core, &imap_hex(4)), None);

        // Load All Mail: the rest whole, over the API.
        assert_eq!(core.cleanup_load_waiting_headers("acct".into()).await.unwrap(), 2);
        settled(&core, 5).await;
        assert!(!core.cleanup_load_status("acct".into()).await.unwrap().headers_paused);
        assert_eq!(body_state(&core, &imap_hex(3)).as_deref(), Some("full"));
        assert_eq!(body_state(&core, &imap_hex(4)).as_deref(), Some("full"));
        assert!(rest.fetch_calls.load(std::sync::atomic::Ordering::SeqCst) > 0, "over the API");
    });
    core.stop_sync();
}

/// IMAP coming back resumes Clean Up's header load: headers only, no
/// body and nothing over the API.
#[test]
fn imap_coming_back_resumes_clean_ups_headers() {
    let (_temp, core, _events) = core("load-imap-back");
    register_gmail(&core, true);
    block_on(core.clone().open_account("acct".into())).unwrap();
    crate::runtime::runtime().block_on(async {
        let (server, rest) = imap_mailbox().await;
        start_imap(&core, &server, &rest, Duration::from_secs(3600));
        settled(&core, 3).await;
        let bodies = server.body_fetches();
        core.stop_sync();
        assert!(core.cleanup_load_every_header("acct".into()).await.unwrap());
        server.refuse_logins();
        // A refusal that lasts a moment, so IMAP is tried again soon.
        start_imap(&core, &server, &rest, Duration::from_millis(200));
        for _ in 0..400 {
            if server.refusals() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(server.refusals() >= 2, "refused");
        server.allow_logins();
        status_until(&core, "the headers to arrive", |s| s.headers_waiting + s.bodies_waiting == 0).await;
        settled(&core, 5).await;
        assert_eq!(body_state(&core, &imap_hex(3)).as_deref(), Some("metadata"));
        assert_eq!(body_state(&core, &imap_hex(4)).as_deref(), Some("metadata"));
        assert_eq!(server.body_fetches(), bodies, "no body for the older mail");
        assert_eq!(rest.fetch_calls.load(std::sync::atomic::Ordering::SeqCst), 0, "nothing over the API");
    });
    core.stop_sync();
}

#[test]
fn without_imap_clean_up_can_say_how_much_older_mail_there_is_and_how_long_it_takes() {
    let fake = Arc::new(FakeProvider::new("me@example.com", NOW, 50));
    fake.seed(aged("inbox", 1, &["INBOX"]));
    fake.seed(aged("recent", 10, &[]));
    for n in 0..3 {
        fake.seed(aged(&format!("old{n}"), 400 + n, &[]));
    }
    let (temp, core, _events) = core("load-rest");
    register_gmail(&core, false);
    block_on(core.clone().open_account("acct".into())).unwrap();
    block_on(core.set_sync_window_for("acct".into(), SyncWindow::Month)).unwrap();
    core.start_sync_with(fake.clone()).unwrap();
    wait_until("the month synced", || all_mail_count(&core) == 2);
    let status = block_on(core.cleanup_load_status("acct".into())).unwrap();
    assert!(status.has_sync_window && !status.cheap_headers, "the API: headers are not cheap");
    assert_eq!(status.window, SyncWindow::Month);
    let estimate = block_on(core.cleanup_load_estimate("acct".into())).unwrap();
    assert_eq!(estimate.messages, Some(3), "Gmail's count less what is here");
    assert_eq!(estimate.seconds, Some(1), "250 a minute");
    assert_eq!(provider_gmail::rest_download_seconds(43_000), 10_320, "about 2.9 hours for a large mailbox");

    // Load All Mail: every message, whole.
    assert!(block_on(core.cleanup_load_every_header("acct".into())).unwrap());
    wait_until("all mail downloaded", || all_mail_count(&core) == 5);
    assert_eq!(body_state(&core, "old0").as_deref(), Some("full"), "over the API a header costs a whole message");
    core.stop_sync();
    drop(temp);
}

#[test]
fn imported_and_demo_mailboxes_have_nothing_to_load() {
    let (_temp, core, _events) = core("load-none");
    block_on(core.clone().open_account("demo".into())).unwrap();
    let status = block_on(core.cleanup_load_status("demo".into())).unwrap();
    assert!(!status.has_sync_window);
    assert!(block_on(core.cleanup_load_every_header("demo".into())).is_err());
}

#[test]
fn progress_sets_the_baseline_on_opening_and_counts_what_was_removed() {
    let (_temp, core, _events) = core("progress");
    block_on(core.clone().open_account("demo".into())).unwrap();
    block_on(core.debug_seed_demo_mailbox(80)).unwrap();
    let first = block_on(core.cleanup_progress("demo".into())).unwrap();
    assert!(first.now > 0);
    assert_eq!(first.baseline, first.now, "the Inbox when Clean Up first opened");
    assert_eq!((first.at_midnight, first.received_today, first.removed_today), (first.now, 0, 0));
    assert_eq!(first.percent, 0);
    assert_eq!(first.days.len(), 1, "today, recorded on opening");

    let groups =
        block_on(core.cleanup_groups("demo".into(), CleanupView::Sender, CleanupScope::Inbox, String::new())).unwrap();
    let biggest = groups[0].clone();
    block_on(core.cleanup_apply(
        "demo".into(),
        CleanupView::Sender,
        CleanupScope::Inbox,
        vec![biggest.key],
        CleanupAction::Archive,
    ))
    .unwrap();
    let after = block_on(core.cleanup_progress("demo".into())).unwrap();
    assert_eq!(after.baseline, first.baseline, "set once");
    assert_eq!(after.now, first.now - biggest.count);
    assert_eq!(after.removed_today, biggest.count);
    assert_eq!(after.at_midnight, first.at_midnight);
    assert_eq!(u64::from(after.percent), biggest.count * 100 / first.baseline);
}

#[test]
fn progress_at_a_fixed_moment_and_offset() {
    let (_temp, core, _events) = core("progress-fixed");
    block_on(core.clone().open_account("demo".into())).unwrap();
    block_on(core.debug_seed_demo_mailbox(40)).unwrap();
    let db = block_on(core.store_for("demo")).unwrap();
    // The demo's newest mail arrived at NOW: a moment later, at UTC+2,
    // what arrived since that local midnight is today's.
    let offset = 2 * 3600;
    let midnight = mail_store::cleanup::local_midnight(NOW, offset);
    let p = db.write_blocking(move |tx| progress_at(tx, NOW + 1, offset)).unwrap();
    let today = db
        .read_blocking(move |c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM messages WHERE internal_date >= ?1 AND is_sent_by_me = 0",
                [midnight],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .unwrap();
    assert_eq!(p.received_today, today as u64);
    assert_eq!(p.days.last().map(|d| d.day.clone()), Some(mail_store::cleanup::day_key(NOW, offset)));
    assert_eq!(p.at_midnight + p.received_today - p.removed_today, p.now);
}

#[test]
fn the_first_sync_of_the_day_records_the_inbox() {
    let fake = Arc::new(FakeProvider::new("me@example.com", NOW, 500));
    for i in 0..3 {
        fake.seed(message(&format!("m{i}"), &format!("t{i}"), "a@example.com", &["INBOX"]));
    }
    let (_temp, core, _events) = synced("history", &fake, 3);
    let db = block_on(core.store_for("acct")).unwrap();
    let today = mail_store::cleanup::day_key(mail_sync::now_millis(), utc_offset_now());
    wait_until("today's count recorded", || {
        let today = today.clone();
        db.read_blocking(move |c| mail_store::cleanup::inbox_days(c, &today)).unwrap().len() == 1
    });
    let days = db.read_blocking(|c| mail_store::cleanup::inbox_days(c, "2000-01-01")).unwrap();
    assert_eq!(days, [(today, 3)]);
    assert_eq!(db.read_blocking(mail_store::cleanup::baseline).unwrap(), None, "only opening Clean Up sets it");
}
