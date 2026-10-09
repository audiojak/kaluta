//! A provider whose labels live only on this Mac and that gives sent mail
//! its own Message-ID (an agent mailbox, spec §7.9).

use std::sync::Arc;

use async_trait::async_trait;
use mail_domain::{EmailAddress, Label, LabelId, MessageId, ThreadId};
use mail_store::drafts;
use mail_store::{Db, ThreadChanges, consistency, read};
use mail_sync::{LocalChange, SyncEngine, SyncObserver, send_draft};
use provider_api::fake::FakeProvider;
use provider_api::{
    ChangeSet, FetchedBody, FetchedMessage, IdPage, LabelOp, LabelSync, ListFilter, MailProvider, PageToken, Priority,
    Profile, ProviderResult, SyncCursor,
};

const NOW: i64 = 1_790_000_000_000;

struct Quiet;
impl SyncObserver for Quiet {
    fn threads_changed(&self, _: &ThreadChanges) {}
}

/// The fake mailbox, but labels stay local (or sync as `.1` says) and a
/// sent message comes back with a Message-ID of the service's own.
struct AgentLike(Arc<FakeProvider>, LabelSync);

#[async_trait]
impl MailProvider for AgentLike {
    async fn profile(&self) -> ProviderResult<Profile> {
        self.0.profile().await
    }
    async fn list_labels(&self) -> ProviderResult<Vec<Label>> {
        self.0.list_labels().await
    }
    async fn list_message_ids(&self, filter: &ListFilter, page: Option<PageToken>) -> ProviderResult<IdPage> {
        self.0.list_message_ids(filter, page).await
    }
    async fn fetch_messages(&self, ids: &[MessageId], priority: Priority) -> ProviderResult<Vec<FetchedMessage>> {
        let mut fetched = self.0.fetch_messages(ids, priority).await?;
        for m in &mut fetched {
            if m.id.as_str().starts_with("sent") {
                m.message_id_header = Some(format!("{}@service.example", m.id.as_str()));
            }
        }
        Ok(fetched)
    }
    async fn changes_since(&self, cursor: &SyncCursor) -> ProviderResult<ChangeSet> {
        self.0.changes_since(cursor).await
    }
    async fn modify_labels(&self, _op: &LabelOp) -> ProviderResult<()> {
        Ok(())
    }
    async fn move_to_trash(&self, _id: &MessageId) -> ProviderResult<()> {
        Ok(())
    }
    async fn restore_from_trash(&self, _id: &MessageId) -> ProviderResult<()> {
        Ok(())
    }
    async fn send(&self, raw: &[u8], thread: Option<&ThreadId>) -> ProviderResult<MessageId> {
        self.0.send(raw, thread).await
    }
    async fn fetch_attachment(&self, message: &MessageId, attachment_id: &str) -> ProviderResult<Vec<u8>> {
        self.0.fetch_attachment(message, attachment_id).await
    }
    async fn save_draft(
        &self,
        existing: Option<&str>,
        _raw: &[u8],
        _thread: Option<&ThreadId>,
    ) -> ProviderResult<String> {
        Ok(existing.unwrap_or("local").to_owned())
    }
    async fn delete_draft(&self, _draft_id: &str) -> ProviderResult<()> {
        Ok(())
    }
    async fn create_label(&self, name: &str, color: Option<(&str, &str)>) -> ProviderResult<Label> {
        self.0.create_label(name, color).await
    }
    fn label_sync(&self) -> LabelSync {
        self.1
    }
    fn adopts_sent_copies(&self) -> bool {
        true
    }
}

fn message(id: &str, thread: &str, labels: &[&str]) -> FetchedMessage {
    FetchedMessage {
        id: MessageId::new(id),
        thread_id: ThreadId::new(thread),
        label_ids: labels.iter().map(|l| LabelId::new(*l)).collect(),
        internal_date: NOW,
        message_id_header: Some(format!("{id}@example.com")),
        from: Some(EmailAddress::new(None, "ada@example.com")),
        subject: format!("S {thread}"),
        body: Some(FetchedBody { text: Some("x".into()), html: None, attachments: vec![] }),
        ..Default::default()
    }
}

async fn setup(name: &str) -> (Arc<FakeProvider>, Db, SyncEngine) {
    setup_with(name, Some(LabelSync::Local)).await
}

/// `sync` `None`: the fake itself, whose labels are the truth (Gmail).
async fn setup_with(name: &str, sync: Option<LabelSync>) -> (Arc<FakeProvider>, Db, SyncEngine) {
    let dir = std::env::temp_dir().join(format!("kaluta-local-labels-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = Db::open(&dir.join("mail.sqlite")).unwrap();
    let fake = Arc::new(FakeProvider::new("scout@agents.example.com", NOW, 50));
    fake.seed(message("m1", "a", &["INBOX", "UNREAD"]));
    fake.seed(message("m2", "b", &["INBOX", "UNREAD"]));
    let provider: Arc<dyn MailProvider> = match sync {
        Some(sync) => Arc::new(AgentLike(fake.clone(), sync)),
        None => fake.clone(),
    };
    let engine = SyncEngine::new(provider, db.clone(), Arc::new(Quiet));
    engine.bootstrap_prepare().await.unwrap();
    engine.bootstrap_list_rest().await.unwrap();
    engine.backfill_all().await.unwrap();
    (fake, db, engine)
}

async fn inbox(db: &Db) -> Vec<String> {
    db.read(|c| read::list_threads(c, "INBOX", None, 10)).await.unwrap().rows.into_iter().map(|t| t.id.0).collect()
}

#[tokio::test]
async fn a_full_resync_keeps_what_was_archived_and_read_on_this_mac() {
    let (fake, db, engine) = setup("resync").await;
    engine.apply_change(LocalChange::archive(vec![ThreadId::new("a")]), true).await.unwrap();
    engine
        .apply_change(
            LocalChange::Labels {
                thread_ids: vec![ThreadId::new("b")],
                add: vec![],
                remove: vec![LabelId::new("UNREAD")],
            },
            true,
        )
        .await
        .unwrap();
    engine.drain_outbox().await.unwrap();
    assert_eq!(inbox(&db).await, vec!["b"]);

    // The change feed expired: everything is listed and fetched again.
    fake.deliver(message("m0", "z", &["SENT"]));
    fake.expire_history();
    assert!(engine.sync_incremental().await.is_err(), "a resync starts");
    engine.backfill_all().await.unwrap();
    assert_eq!(inbox(&db).await, vec!["b"], "m1 stays archived");
    let b = db.read(|c| read::list_threads(c, "INBOX", None, 10)).await.unwrap().rows.remove(0);
    assert_eq!(b.unread_count, 0, "m2 stays read");

    // Mail new to the store still takes the provider's labels.
    fake.deliver(message("m3", "c", &["INBOX", "UNREAD"]));
    engine.sync_incremental().await.unwrap();
    assert!(inbox(&db).await.contains(&"c".to_owned()));
    assert!(db.read(consistency::check).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_sent_message_with_the_services_own_message_id_leaves_no_duplicate() {
    let (_fake, db, engine) = setup("send").await;
    let draft = drafts::DraftRecord {
        to: vec![EmailAddress::new(None, "ada@example.com")],
        subject: "Hello".into(),
        body_html: "<p>Hi</p>".into(),
        ..Default::default()
    };
    let id = db.write(move |tx| drafts::save(tx, &draft, NOW)).await.unwrap();
    send_draft(&db, id, EmailAddress::new(Some("Scout"), "scout@agents.example.com"), true, 0).await.unwrap();
    engine.drain_outbox().await.unwrap();
    engine.sync_incremental().await.unwrap();
    let sent = db.read(|c| read::list_threads(c, "SENT", None, 10)).await.unwrap().rows;
    let messages: usize = sent.iter().map(|t| t.message_count as usize).sum();
    assert_eq!(messages, 1, "one sent message, not the local copy as well: {sent:?}");
    assert!(db.read(consistency::check).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_label_made_on_this_mac_survives_the_next_sync_start() {
    let (_fake, db, engine) = setup("refresh").await;
    // A label made on this Mac (as `create_label` does on an agent
    // mailbox), put on thread a.
    let label = Label {
        id: LabelId::new("Local_Receipts"),
        name: "Receipts".into(),
        kind: mail_domain::LabelKind::User,
        color: None,
        visible: true,
    };
    let id = label.id.clone();
    db.write(move |tx| {
        let mut w = mail_store::MailWriter::new(tx);
        w.upsert_labels(&[label])?;
        w.finish()
    })
    .await
    .unwrap();
    engine
        .apply_change(
            LocalChange::Labels { thread_ids: vec![ThreadId::new("a")], add: vec![id.clone()], remove: vec![] },
            true,
        )
        .await
        .unwrap();
    engine.drain_outbox().await.unwrap();

    // Sync starts again: the label list is refreshed from the provider,
    // which does not know the label.
    engine.refresh_labels().await.unwrap();
    let labels = db.read(read::list_labels).await.unwrap();
    assert!(labels.iter().any(|l| l.id == id && l.name == "Receipts"), "the label stays: {labels:?}");
    let threads = db.read(move |c| read::list_threads(c, id.as_str(), None, 10)).await.unwrap().rows;
    assert_eq!(threads.iter().map(|t| t.id.0.as_str()).collect::<Vec<_>>(), vec!["a"], "and so does its thread");
    assert!(db.read(consistency::check).await.unwrap().is_empty());
}

/// Labels that sync both ways but where Trash is only this Mac's (as
/// AgentMail's provider merges them): the fetched labels, with a stored
/// Trash kept and the Inbox left out of it.
fn trash_is_local(stored: &[LabelId], fetched: &[LabelId]) -> Vec<LabelId> {
    let trash = LabelId::new("TRASH");
    let mut out: Vec<LabelId> = fetched.to_vec();
    if stored.contains(&trash) {
        out.retain(|l| l.as_str() != "INBOX");
        out.push(trash);
    }
    out
}

async fn labels_of(db: &Db, id: &str) -> Vec<String> {
    let id = MessageId::new(id);
    let mut labels: Vec<String> = db
        .read(move |c| read::get_message(c, &id))
        .await
        .unwrap()
        .unwrap()
        .label_ids
        .into_iter()
        .map(|l| l.0)
        .collect();
    labels.sort();
    labels
}

/// What each kind of provider's labels become in a full resync after the
/// change feed expired: m1 was trashed on this Mac; m2 was read and
/// archived elsewhere, unseen by the feed.
async fn resync_after_changes_elsewhere(name: &str, sync: Option<LabelSync>) -> (Db, Vec<String>, Vec<String>) {
    let (fake, db, engine) = setup_with(name, sync).await;
    engine.apply_change(LocalChange::Trash { thread_ids: vec![ThreadId::new("a")] }, true).await.unwrap();
    engine.drain_outbox().await.unwrap();
    fake.relabel(&MessageId::new("m2"), &[], &[LabelId::new("UNREAD"), LabelId::new("INBOX")]);
    fake.expire_history();
    assert!(engine.sync_incremental().await.is_err(), "a resync starts");
    engine.backfill_all().await.unwrap();
    let (m1, m2) = (labels_of(&db, "m1").await, labels_of(&db, "m2").await);
    assert!(db.read(consistency::check).await.unwrap().is_empty());
    (db, m1, m2)
}

#[tokio::test]
async fn a_resync_takes_the_providers_labels_where_they_are_the_truth() {
    // Gmail: Trash went to the provider too, and m2's change is taken.
    let (_db, m1, m2) = resync_after_changes_elsewhere("resync-provider", None).await;
    assert_eq!(m1, ["TRASH", "UNREAD"]);
    assert!(m2.is_empty(), "read and archived: {m2:?}");
}

#[tokio::test]
async fn a_resync_keeps_labels_that_live_on_this_mac() {
    // Primitive: nothing at the provider changes what the store has.
    let (_db, m1, m2) = resync_after_changes_elsewhere("resync-local", Some(LabelSync::Local)).await;
    assert_eq!(m1, ["TRASH", "UNREAD"]);
    assert_eq!(m2, ["INBOX", "UNREAD"], "labels are this Mac's alone");
}

#[tokio::test]
async fn a_resync_repairs_labels_that_sync_both_ways_and_keeps_the_macs_own() {
    // AgentMail: m2's read and archive made elsewhere are repaired; m1,
    // trashed only here, stays in Trash.
    let (_db, m1, m2) = resync_after_changes_elsewhere("resync-both", Some(LabelSync::Both(trash_is_local))).await;
    assert_eq!(m1, ["TRASH", "UNREAD"], "the Mac's Trash stays");
    assert!(m2.is_empty(), "read and archived elsewhere: {m2:?}");
}
