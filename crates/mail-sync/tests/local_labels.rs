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
    ChangeSet, FetchedBody, FetchedMessage, IdPage, LabelOp, ListFilter, MailProvider, PageToken, Priority, Profile,
    ProviderResult, SyncCursor,
};

const NOW: i64 = 1_790_000_000_000;

struct Quiet;
impl SyncObserver for Quiet {
    fn threads_changed(&self, _: &ThreadChanges) {}
}

/// The fake mailbox, but labels stay local and a sent message comes back
/// with a Message-ID of the service's own.
struct AgentLike(Arc<FakeProvider>);

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
    fn labels_are_local(&self) -> bool {
        true
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
    let dir = std::env::temp_dir().join(format!("openagc-local-labels-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let db = Db::open(&dir.join("mail.sqlite")).unwrap();
    let fake = Arc::new(FakeProvider::new("scout@agents.example.com", NOW, 50));
    fake.seed(message("m1", "a", &["INBOX", "UNREAD"]));
    fake.seed(message("m2", "b", &["INBOX", "UNREAD"]));
    let engine = SyncEngine::new(Arc::new(AgentLike(fake.clone())), db.clone(), Arc::new(Quiet));
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
