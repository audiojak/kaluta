//! The provider abstraction sync is written against (spec §7.6), plus the
//! HTTP, retry, token and rate-limit plumbing providers share. Gmail is the
//! only implementation in the MVP; cursors and page tokens are opaque so an
//! IMAP provider (UIDVALIDITY/MODSEQ) fits later.

mod error;
pub mod fake;
pub mod http;
pub mod mailbox;
pub mod rate_limit;
pub mod token;

use async_trait::async_trait;
use mail_domain::{EmailAddress, Label, LabelId, ListHeaders, MessageId, Millis, ThreadId};

pub use error::{ProviderError, ProviderResult};
pub use http::{HttpClient, RetryPolicy};
pub use mailbox::{
    AddedMailbox, DnsRecord, MailboxDomain, MailboxPlan, MailboxService, SendRule, SignedUp, VerificationStarted,
};
pub use rate_limit::{Priority, RateLimiter};
pub use token::{AccessToken, TokenSource};

/// Opaque position in the provider's change stream (Gmail: a historyId).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SyncCursor(pub String);

/// Opaque continuation token for a listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageToken(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub email: String,
    pub messages_total: u64,
    /// Where incremental sync starts; recorded before bootstrap lists
    /// anything so nothing arriving during bootstrap is missed (spec §7.4).
    pub cursor: SyncCursor,
}

/// Which messages to list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListFilter {
    pub label_ids: Vec<LabelId>,
    /// Provider search syntax, e.g. `newer_than:30d`.
    pub query: Option<String>,
    pub include_spam_trash: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdPage {
    pub ids: Vec<(MessageId, ThreadId)>,
    pub next: Option<PageToken>,
    /// The provider's estimate of the total, when it gives one.
    pub estimated_total: Option<u64>,
}

/// A message as fetched, before sanitizing. `html` is the original HTML.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchedMessage {
    pub id: MessageId,
    pub thread_id: ThreadId,
    pub label_ids: Vec<LabelId>,
    pub snippet: String,
    /// When the provider received it (Gmail `internalDate`).
    pub internal_date: Millis,
    pub size_estimate: u64,
    pub message_id_header: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub from: Option<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    pub subject: String,
    /// The `Date` header, if parseable.
    pub date: Option<Millis>,
    /// Mailing-list headers, when the fetch included them.
    pub list: ListHeaders,
    /// `None` when only metadata was fetched.
    pub body: Option<FetchedBody>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchedBody {
    pub text: Option<String>,
    pub html: Option<String>,
    pub attachments: Vec<FetchedAttachment>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchedAttachment {
    pub part_id: Option<String>,
    /// Provider id for fetching the bytes on demand.
    pub attachment_id: Option<String>,
    pub filename: String,
    pub mime_type: String,
    pub size: u64,
    pub content_id: Option<String>,
    pub is_inline: bool,
    /// Bytes returned with the message (small parts with no attachment id).
    pub data: Option<Vec<u8>>,
}

/// One change in the provider's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    MessageAdded { id: MessageId, thread_id: ThreadId, label_ids: Vec<LabelId> },
    MessageDeleted { id: MessageId },
    LabelsAdded { id: MessageId, label_ids: Vec<LabelId> },
    LabelsRemoved { id: MessageId, label_ids: Vec<LabelId> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSet {
    pub changes: Vec<Change>,
    /// Cursor to resume from after applying `changes`.
    pub cursor: SyncCursor,
}

/// Add/remove labels on a set of messages (archive = remove INBOX).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelOp {
    pub message_ids: Vec<MessageId>,
    pub add: Vec<LabelId>,
    pub remove: Vec<LabelId>,
}

/// Where backfill fetches message bodies from (spec §7.4, IMAP amendment):
/// the REST API by default, or a bulk transport such as Gmail IMAP. Only
/// bulk backfill goes through here; history, writes and new-mail fetches
/// stay on [`MailProvider`].
#[async_trait]
pub trait BackfillSource: Send + Sync {
    /// Full messages for `ids`, in any order; ids it cannot find are
    /// omitted, as with [`MailProvider::fetch_messages`].
    async fn fetch(&self, ids: &[MessageId]) -> ProviderResult<Vec<FetchedMessage>>;
    /// Headers only (no bodies) for `ids`, if this source can get them far
    /// more cheaply than bodies; `None` when it cannot (REST charges the
    /// same for headers as for a whole message). Used to make the list
    /// browsable before bodies arrive.
    async fn fetch_headers(&self, _ids: &[MessageId]) -> ProviderResult<Option<Vec<FetchedMessage>>> {
        Ok(None)
    }
    /// Ids of the messages matching a Gmail search (`query` in Gmail's
    /// syntax, `""` for everything), if this source can list without the
    /// API's quota; `None` when it cannot (docs/plans/imap-first-sync.md).
    async fn list(&self, _query: &str) -> ProviderResult<Option<Vec<MessageId>>> {
        Ok(None)
    }
    /// Wait up to `max` for the mailbox to change (IMAP IDLE): `Some(true)`
    /// when something arrived or changed, `Some(false)` when the wait ran
    /// out; `None` when this source cannot push.
    async fn watch(&self, _max: std::time::Duration) -> ProviderResult<Option<bool>> {
        Ok(None)
    }
    /// Whether [`BackfillSource::fetch_headers`] is cheap here at all (it
    /// may still answer `None` for a while, e.g. over its daily budget).
    /// Decides tiered download (spec §7.4 amendment 2026-09-27).
    fn cheap_headers(&self) -> bool {
        false
    }
    /// A short name for logs and diagnostics ("rest", "imap").
    fn name(&self) -> &'static str;
}

/// Backfill through the provider's own fetch at background priority.
pub struct RestBackfill(pub std::sync::Arc<dyn MailProvider>);

#[async_trait]
impl BackfillSource for RestBackfill {
    async fn fetch(&self, ids: &[MessageId]) -> ProviderResult<Vec<FetchedMessage>> {
        self.0.fetch_messages(ids, Priority::Background).await
    }

    fn name(&self) -> &'static str {
        "rest"
    }
}

#[async_trait]
pub trait MailProvider: Send + Sync {
    async fn profile(&self) -> ProviderResult<Profile>;
    async fn list_labels(&self) -> ProviderResult<Vec<Label>>;
    async fn list_message_ids(&self, filter: &ListFilter, page: Option<PageToken>) -> ProviderResult<IdPage>;
    /// Full messages (headers, bodies, attachment metadata; not attachment
    /// bytes). Missing ids are omitted from the result, not errors.
    async fn fetch_messages(&self, ids: &[MessageId], priority: Priority) -> ProviderResult<Vec<FetchedMessage>>;
    /// Changes since `cursor`. [`ProviderError::CursorExpired`] means a full
    /// resync is needed.
    async fn changes_since(&self, cursor: &SyncCursor) -> ProviderResult<ChangeSet>;
    async fn modify_labels(&self, op: &LabelOp) -> ProviderResult<()>;
    async fn move_to_trash(&self, id: &MessageId) -> ProviderResult<()>;
    /// Take a message out of Trash (undo of a trash, spec §14.6a).
    async fn restore_from_trash(&self, id: &MessageId) -> ProviderResult<()>;
    /// Send raw RFC 5322 bytes, threaded into `thread` when given.
    async fn send(&self, raw: &[u8], thread: Option<&ThreadId>) -> ProviderResult<MessageId>;
    async fn fetch_attachment(&self, message: &MessageId, attachment_id: &str) -> ProviderResult<Vec<u8>>;
    /// Create a server draft (`existing` = `None`) or replace one; returns
    /// the draft id. [`ProviderError::NotFound`] if `existing` is gone.
    async fn save_draft(&self, existing: Option<&str>, raw: &[u8], thread: Option<&ThreadId>)
    -> ProviderResult<String>;
    async fn delete_draft(&self, draft_id: &str) -> ProviderResult<()>;
    /// The account's drafts: (draft id, the message it holds now). Gmail's
    /// change history leaves drafts out, so they are synced through this
    /// (spec §14.5 amendment 2026-09-28). `None`: the provider has no
    /// drafts API, so stored drafts are left as they are.
    async fn list_drafts(&self) -> ProviderResult<Option<Vec<(String, MessageId)>>> {
        Ok(None)
    }
    /// Create a user label. `color` is a `(background, text)` pair from the
    /// provider's palette.
    async fn create_label(&self, name: &str, color: Option<(&str, &str)>) -> ProviderResult<Label>;
    /// Labels, read state, stars and trash live only on this Mac (an agent
    /// mailbox, spec §7.9): a refetched message keeps what is stored, and
    /// the provider's labels apply only to mail new to the store.
    fn labels_are_local(&self) -> bool {
        false
    }
    /// A sent message comes back under the id [`MailProvider::send`]
    /// returned, but perhaps with another Message-ID: the optimistic local
    /// copy takes that id at once rather than waiting to be matched by
    /// Message-ID.
    fn adopts_sent_copies(&self) -> bool {
        false
    }
}
