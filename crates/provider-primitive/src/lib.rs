//! Primitive (primitive.dev) agent mailboxes (spec §7.9, ADR 0014).
//!
//! [`PrimitiveService`] creates and verifies accounts; [`PrimitiveProvider`]
//! syncs one over the REST API, mapping it onto the store's Gmail-shaped
//! model: received mail is `in:<uuid>` with `INBOX`, sent mail `out:<uuid>`
//! with `SENT`, and everything the user does to labels stays local.
//!
//! Primitive has no drafts, labels or server search, takes one recipient
//! per send, and keeps its change feed 7 days.
//!
//! Several agents share one Primitive account (ADR 0015): each agent's
//! provider keeps the mail addressed to it and the mail it sent
//! ([`routing`]). The change feed's cursor is the client's (`since=`), not
//! consumed by reading, so each agent long-polls with a cursor of its own
//! and none takes changes from another; one rate limiter per account
//! ([`PrimitiveProvider::for_agent`]) keeps them under the key's limit.

pub mod routing;
mod wire;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use futures::stream::{FuturesUnordered, StreamExt};
use mail_domain::{EmailAddress, Label, LabelColor, LabelId, LabelKind, MessageId, Millis, ThreadId, system_labels};
use provider_api::{
    BackfillSource, Change, ChangeSet, DnsRecord, FetchedAttachment, FetchedBody, FetchedMessage, HttpClient, IdPage,
    LabelOp, ListFilter, MailProvider, MailboxDomain, MailboxPlan, MailboxService, PageToken, Priority, Profile,
    ProviderError, ProviderResult, RateLimiter, RetryPolicy, SendRule, SignedUp, SyncCursor, TokenSource,
    VerificationStarted,
};
pub use routing::{Keep, OTHER_ADDRESSES_LABEL, Routing, RoutingSource};
use serde_json::json;
use sha2::{Digest, Sha256};

pub const PRIMITIVE_API: &str = "https://api.primitive.dev/v1";
pub const TERMS_URL: &str = "https://www.primitive.dev/terms";
/// The dashboard's sign-in: email first, or Google, GitHub or Spotify.
pub const DASHBOARD_URL: &str = "https://www.primitive.dev/login";
/// Verification codes come from Primitive's own domain.
pub const CODE_SENDER_DOMAIN: &str = "primitive.dev";

/// Concurrent message fetches; the rate limiter paces them.
const FETCH_CONCURRENCY: usize = 6;
/// Rows per listing page.
const PAGE_SIZE: u32 = 100;
/// Longest long-poll Primitive serves.
const MAX_WAIT: Duration = Duration::from_secs(20);
/// Snippet length, as for Gmail.
const SNIPPET_CHARS: usize = 200;

const INBOUND: &str = "in:";
const OUTBOUND: &str = "out:";

/// The agent plan allows 120 requests a minute; stay under it. One per
/// Primitive account: its agents share it, as they share the key.
pub fn rate_limiter() -> Arc<RateLimiter> {
    Arc::new(RateLimiter::new(100, 20))
}

/// A Primitive mail id as the store's message id.
pub fn inbound_id(uuid: &str) -> MessageId {
    MessageId(format!("{INBOUND}{uuid}"))
}

pub fn outbound_id(uuid: &str) -> MessageId {
    MessageId(format!("{OUTBOUND}{uuid}"))
}

/// One synced agent on a Primitive account.
pub struct PrimitiveProvider {
    http: HttpClient,
    base: String,
    /// The agent's address (From for sends).
    address: String,
    /// Which of the account's mail is this agent's.
    routing: RoutingSource,
    /// The newest change cursor seen, for long-polling without consuming.
    last_cursor: Mutex<Option<String>>,
    /// Woken when the first cursor is seen, so a long-poll that started
    /// before it waits for it rather than sleeping its whole wait.
    cursor_seen: tokio::sync::Notify,
}

impl PrimitiveProvider {
    pub fn new(tokens: Arc<dyn TokenSource>, address: &str) -> ProviderResult<Self> {
        Self::with_base(tokens, address, RetryPolicy::default(), PRIMITIVE_API)
    }

    /// For tests: another base URL and retry policy. The agent is alone on
    /// its account: all the account's mail is its own.
    pub fn with_base(
        tokens: Arc<dyn TokenSource>,
        address: &str,
        retry: RetryPolicy,
        base: &str,
    ) -> ProviderResult<Self> {
        let only = Routing::only(address);
        Self::for_agent(tokens, address, Arc::new(move || only.clone()), rate_limiter(), retry, base)
    }

    /// One agent of an account with several: `routing` says which mail is
    /// its own (read again as it syncs), `limiter` is the account's.
    pub fn for_agent(
        tokens: Arc<dyn TokenSource>,
        address: &str,
        routing: RoutingSource,
        limiter: Arc<RateLimiter>,
        retry: RetryPolicy,
        base: &str,
    ) -> ProviderResult<Self> {
        Ok(Self {
            http: HttpClient::new(tokens, limiter, retry)?,
            base: base.trim_end_matches('/').to_owned(),
            address: address.to_owned(),
            routing,
            last_cursor: Mutex::new(None),
            cursor_seen: tokio::sync::Notify::new(),
        })
    }

    fn routing(&self) -> Routing {
        (self.routing)()
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{path}", self.base)
    }

    fn remember_cursor(&self, cursor: &str) {
        *self.last_cursor.lock().unwrap_or_else(|e| e.into_inner()) = Some(cursor.to_owned());
        self.cursor_seen.notify_waiters();
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        priority: Priority,
    ) -> ProviderResult<wire::Envelope<T>> {
        let url = self.url(path);
        self.http.json(1, priority, |c| c.get(&url).query(query)).await
    }

    /// A message, if it is this agent's (`None` too when it is gone).
    async fn fetch_one(
        &self,
        id: &MessageId,
        priority: Priority,
        routing: &Routing,
    ) -> ProviderResult<Option<FetchedMessage>> {
        let result = if let Some(uuid) = id.as_str().strip_prefix(INBOUND) {
            self.fetch_inbound(uuid, priority, routing).await
        } else if let Some(uuid) = id.as_str().strip_prefix(OUTBOUND) {
            self.fetch_outbound(uuid, priority, routing).await
        } else {
            return Ok(None);
        };
        match result {
            Ok(m) => Ok(m),
            // Deleted between listing and fetching.
            Err(ProviderError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// A received message: its record (thread, time) and its raw MIME;
    /// the record's own fields when the raw form is gone. `None` when it
    /// is another agent's: by the record's recipient, else by the raw
    /// message's.
    async fn fetch_inbound(
        &self,
        uuid: &str,
        priority: Priority,
        routing: &Routing,
    ) -> ProviderResult<Option<FetchedMessage>> {
        let record: wire::Envelope<wire::Inbound> = self.get(&format!("emails/{uuid}"), &[], priority).await?;
        let record = record.data;
        let by_record = record
            .to_email
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .map(|t| routing.keeps(routing::addresses_in(t).iter().map(String::as_str)));
        if by_record == Some(None) {
            return Ok(None);
        }
        let url = self.url(&format!("emails/{uuid}/raw"));
        let raw = if record.content_discarded_at.is_some() {
            None
        } else {
            match self.http.bytes(1, priority, |c| c.get(&url)).await {
                Ok((bytes, _)) => Some(bytes),
                Err(ProviderError::NotFound(_) | ProviderError::Forbidden(_)) => None,
                Err(e) => return Err(e),
            }
        };
        let keep = match by_record {
            Some(keep) => keep,
            None => {
                let named = raw.as_deref().map(routing::raw_recipients).unwrap_or_default();
                routing.keeps(named.iter().map(String::as_str))
            }
        };
        let Some(keep) = keep else { return Ok(None) };
        Ok(Some(marked(inbound_message(&record, raw.as_deref()), keep, routing)))
    }

    /// A sent message, if this agent sent it (or no agent did, for the
    /// first agent).
    async fn fetch_outbound(
        &self,
        uuid: &str,
        priority: Priority,
        routing: &Routing,
    ) -> ProviderResult<Option<FetchedMessage>> {
        let record: wire::Envelope<wire::Sent> = self.get(&format!("sent-emails/{uuid}"), &[], priority).await?;
        let from = routing::addresses_in(&record.data.from_header);
        let Some(keep) = routing.keeps(from.iter().map(String::as_str)) else { return Ok(None) };
        Ok(Some(marked(outbound_message(&record.data), keep, routing)))
    }

    /// Long-poll the change feed from the newest cursor seen: whether
    /// anything changed within `max`. The cursor is this provider's own:
    /// other agents of the account poll the same feed with theirs, and a
    /// change to another agent's mail wakes this one too (its sync then
    /// finds nothing of its own).
    pub async fn wait_for_change(&self, max: Duration) -> ProviderResult<bool> {
        // Registered before the cursor is read, so a cursor remembered in
        // between still wakes it.
        let seen = self.cursor_seen.notified();
        tokio::pin!(seen);
        seen.as_mut().enable();
        let cursor = self.last_cursor.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(cursor) = cursor else {
            // Nothing synced yet: the poll takes the first cursor. Wait for
            // it (or the whole wait), then the caller long-polls from it:
            // sleeping the whole wait would leave this agent deaf to new
            // mail for up to MAX_WAIT after it starts (oagc-7ouz).
            tokio::select! {
                () = seen => {}
                () = tokio::time::sleep(max.min(MAX_WAIT)) => {}
            }
            return Ok(false);
        };
        let wait = max.min(MAX_WAIT).as_secs().max(1);
        let page: wire::Envelope<wire::Changes> = self
            .get(
                "changes",
                &[("since", cursor), ("wait", wait.to_string()), ("limit", "1".into())],
                Priority::Background,
            )
            .await?;
        Ok(!page.data.changes.is_empty())
    }

    /// One listing page: inbound then outbound, as `filter` asks.
    async fn list_page(
        &self,
        source: Source,
        cursor: Option<String>,
        since: Option<String>,
    ) -> ProviderResult<(Vec<(MessageId, ThreadId)>, Option<String>, Option<u64>)> {
        let mut query = vec![("limit", PAGE_SIZE.to_string())];
        if let Some(cursor) = cursor {
            query.push(("cursor", cursor));
        }
        if let Some(since) = since {
            query.push(("date_from", since));
        }
        // Rows that say whose they are are kept or left out here; the others
        // are decided when fetched.
        let routing = self.routing();
        let ours = |addresses: Option<&String>| match addresses.filter(|a| !a.trim().is_empty()) {
            Some(a) => routing.keeps(routing::addresses_in(a).iter().map(String::as_str)).is_some(),
            None => true,
        };
        match source {
            Source::Inbound => {
                let page: wire::Envelope<Vec<wire::InboundRow>> =
                    self.get("emails", &query, Priority::Background).await?;
                let meta = page.meta.unwrap_or_default();
                let ids = page
                    .data
                    .into_iter()
                    .filter(|r| ours(r.to_email.as_ref()))
                    .map(|r| {
                        let id = inbound_id(&r.id);
                        let thread = ThreadId(r.thread_id.unwrap_or_else(|| id.as_str().to_owned()));
                        (id, thread)
                    })
                    .collect();
                Ok((ids, meta.cursor, meta.total))
            }
            Source::Outbound => {
                let page: wire::Envelope<Vec<wire::SentRow>> =
                    self.get("sent-emails", &query, Priority::Background).await?;
                let meta = page.meta.unwrap_or_default();
                let ids = page
                    .data
                    .into_iter()
                    .filter(|r| ours(r.from_header.as_ref()))
                    .map(|r| {
                        let id = outbound_id(&r.id);
                        let thread = ThreadId(r.thread_id.unwrap_or_else(|| id.as_str().to_owned()));
                        (id, thread)
                    })
                    .collect();
                Ok((ids, meta.cursor, meta.total))
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Inbound,
    Outbound,
}

impl Source {
    fn tag(self) -> &'static str {
        match self {
            Self::Inbound => "in",
            Self::Outbound => "out",
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "in" => Some(Self::Inbound),
            "out" => Some(Self::Outbound),
            _ => None,
        }
    }
}

/// Which listings a filter covers, and from when. `None`: nothing at
/// Primitive matches (Spam, Trash, a label, or a search it cannot run).
fn plan_listing(filter: &ListFilter, now: Millis) -> Option<(Vec<Source>, Option<String>)> {
    let mut sources = vec![Source::Inbound, Source::Outbound];
    for label in &filter.label_ids {
        match label.as_str() {
            system_labels::INBOX => sources.retain(|s| *s == Source::Inbound),
            system_labels::SENT => sources.retain(|s| *s == Source::Outbound),
            _ => return None,
        }
    }
    let mut since = None;
    if let Some(query) = filter.query.as_deref() {
        for term in query.split_whitespace() {
            if term == "is:unread" {
                // Everything received is new to this Mac.
                sources.retain(|s| *s == Source::Inbound);
            } else {
                // `newer_than:<n>d`; anything else Primitive cannot search.
                let days = term.strip_prefix("newer_than:").and_then(|d| d.strip_suffix('d'))?;
                let days: i64 = days.parse().ok()?;
                since = Some(rfc3339(now - days * 86_400_000));
            }
        }
    }
    (!sources.is_empty()).then_some((sources, since))
}

fn rfc3339(millis: Millis) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn parse_time(s: &str) -> Option<Millis> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp_millis())
}

fn now_millis() -> Millis {
    chrono::Utc::now().timestamp_millis()
}

fn snippet(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(SNIPPET_CHARS).collect()
}

fn thread_of(thread: Option<&String>, id: &MessageId) -> ThreadId {
    ThreadId(thread.cloned().unwrap_or_else(|| id.as_str().to_owned()))
}

/// Mail no agent's address is on, kept by the first agent, carries the
/// marker label while the account has several agents.
fn marked(mut m: FetchedMessage, keep: Keep, routing: &Routing) -> FetchedMessage {
    if keep == Keep::Unclaimed && routing.marks() {
        m.label_ids.push(LabelId::new(OTHER_ADDRESSES_LABEL));
        m.label_ids.sort();
    }
    m
}

/// A received message from its record and raw MIME. Rejected mail is
/// filed as Spam.
fn inbound_message(record: &wire::Inbound, raw: Option<&[u8]>) -> FetchedMessage {
    let id = inbound_id(&record.id);
    let thread_id = thread_of(record.thread_id.as_ref(), &id);
    let received = parse_time(&record.received_at).unwrap_or(0);
    let mut label_ids = vec![LabelId::new(system_labels::UNREAD)];
    label_ids.push(LabelId::new(if record.status == "rejected" { system_labels::SPAM } else { system_labels::INBOX }));
    label_ids.sort();
    let parsed = raw.and_then(|r| mail_mime::parse(r).ok());
    match parsed {
        Some(parsed) => {
            let h = parsed.headers;
            let snippet = snippet(parsed.text.as_deref().or(parsed.html.as_deref()).unwrap_or(""));
            FetchedMessage {
                id,
                thread_id,
                label_ids,
                snippet,
                internal_date: received,
                size_estimate: record.raw_size_bytes.unwrap_or(raw.map_or(0, |r| r.len() as u64)),
                message_id_header: h.message_id,
                in_reply_to: h.in_reply_to,
                references: h.references,
                from: h.from,
                to: h.to,
                cc: h.cc,
                bcc: h.bcc,
                reply_to: h.reply_to,
                subject: h.subject,
                date: h.date.or(Some(received)),
                list: h.list,
                body: Some(FetchedBody {
                    text: parsed.text,
                    html: parsed.html,
                    attachments: parsed
                        .attachments
                        .into_iter()
                        .enumerate()
                        .map(|(i, a)| FetchedAttachment {
                            part_id: Some((i + 1).to_string()),
                            attachment_id: None,
                            filename: a.filename,
                            mime_type: a.mime_type,
                            size: a.size,
                            content_id: a.content_id,
                            is_inline: a.is_inline,
                            // Here with the message: nothing to fetch later.
                            data: Some(a.data),
                        })
                        .collect(),
                }),
            }
        }
        None => {
            let from = record.from_header.as_deref().or(record.from_email.as_deref()).unwrap_or_default();
            let to = record.to_email.as_deref().unwrap_or_default();
            let h = mail_mime::parse_headers([("From", from), ("To", to)]);
            let text = record.body_text.clone();
            FetchedMessage {
                id,
                thread_id,
                label_ids,
                snippet: snippet(text.as_deref().or(record.body_html.as_deref()).unwrap_or("")),
                internal_date: received,
                size_estimate: record.raw_size_bytes.unwrap_or(0),
                message_id_header: record.message_id.as_deref().map(strip_angles),
                in_reply_to: None,
                references: vec![],
                from: h.from,
                to: h.to,
                cc: vec![],
                bcc: vec![],
                reply_to: vec![],
                subject: record.subject.clone().unwrap_or_default(),
                date: Some(received),
                // No raw message: no list headers.
                list: Default::default(),
                body: Some(FetchedBody { text, html: record.body_html.clone(), attachments: vec![] }),
            }
        }
    }
}

/// A sent message, rebuilt from its record (Primitive keeps no raw form
/// of what it sent). Attachments are listed and fetched on demand.
fn outbound_message(record: &wire::Sent) -> FetchedMessage {
    let id = outbound_id(&record.id);
    let thread_id = thread_of(record.thread_id.as_ref(), &id);
    let created = parse_time(&record.created_at).unwrap_or(0);
    let cc = record.cc.clone().unwrap_or_default().join(", ");
    let h = mail_mime::parse_headers([
        ("From", record.from_header.as_str()),
        ("To", record.to_header.as_str()),
        ("Cc", cc.as_str()),
    ]);
    let references = record
        .email_references
        .as_deref()
        .map(|r| r.split_whitespace().map(strip_angles).collect())
        .unwrap_or_default();
    let text = record.body_text.clone();
    FetchedMessage {
        id,
        thread_id,
        label_ids: vec![LabelId::new(system_labels::SENT)],
        snippet: snippet(text.as_deref().or(record.body_html.as_deref()).unwrap_or("")),
        internal_date: created,
        size_estimate: record.body_size_bytes,
        message_id_header: record.message_id.as_deref().map(strip_angles),
        in_reply_to: record.in_reply_to.as_deref().map(strip_angles),
        references,
        from: h.from,
        to: h.to,
        cc: h.cc,
        bcc: vec![],
        reply_to: vec![],
        subject: record.subject.clone(),
        date: Some(created),
        list: Default::default(),
        body: Some(FetchedBody {
            text,
            html: record.body_html.clone(),
            attachments: record
                .attachments
                .iter()
                .map(|a| FetchedAttachment {
                    part_id: Some(a.part_index.to_string()),
                    attachment_id: Some(a.part_index.to_string()),
                    filename: a.filename.clone().unwrap_or_else(|| "attachment".into()),
                    mime_type: a.content_type.clone(),
                    size: a.size_bytes,
                    content_id: None,
                    is_inline: false,
                    data: None,
                })
                .collect(),
        }),
    }
}

fn strip_angles(s: &str) -> String {
    s.trim().trim_start_matches('<').trim_end_matches('>').to_owned()
}

fn angled(id: &str) -> String {
    format!("<{}>", strip_angles(id))
}

/// Primitive refused a recipient. Even verified, a free account writes only
/// to people who wrote to it first, the email it was verified with, its own
/// verified domains, Primitive addresses and domains that opt in.
fn not_allowed(to: &str) -> String {
    format!(
        "Primitive won't send to {to}: this mailbox may only write to people who have written to it first, to \
         the email it was verified with, and to domains you have added to it. Have {to} write to it first, or \
         add their domain under Use Your Own Domain if it is yours"
    )
}

/// The service's message for a send it cannot make.
pub const ONE_RECIPIENT_ONLY: &str = "Primitive sends to one recipient per message; remove the others";

/// `/send-mail`'s body for a message the composer built.
fn send_body(raw: &[u8], address: &str) -> ProviderResult<serde_json::Value> {
    let parsed = mail_mime::parse(raw).map_err(|e| ProviderError::Invalid(e.to_string()))?;
    let h = &parsed.headers;
    let recipients: Vec<&EmailAddress> = h.to.iter().chain(&h.cc).chain(&h.bcc).collect();
    let [to] = recipients.as_slice() else {
        return Err(ProviderError::Forbidden(ONE_RECIPIENT_ONLY.into()));
    };
    let from = match h.from.as_ref().and_then(|f| f.name.as_deref()) {
        Some(name) => format!("\"{}\" <{address}>", name.replace(['"', '\\'], "")),
        None => address.to_owned(),
    };
    // Primitive refuses an empty subject (HTTP 400); Gmail takes one.
    let subject = if h.subject.trim().is_empty() { "(no subject)" } else { h.subject.as_str() };
    let mut body = json!({
        "from": from,
        "to": to.email,
        "subject": subject,
    });
    if let Some(text) = &parsed.text {
        body["body_text"] = json!(text);
    }
    if let Some(html) = &parsed.html {
        body["body_html"] = json!(html);
    }
    if parsed.text.is_none() && parsed.html.is_none() {
        body["body_text"] = json!("");
    }
    if let Some(parent) = &h.in_reply_to {
        body["in_reply_to"] = json!(angled(parent));
    }
    if !h.references.is_empty() {
        body["references"] = json!(h.references.iter().map(|r| angled(r)).collect::<Vec<_>>());
    }
    if !parsed.attachments.is_empty() {
        let engine = base64::engine::general_purpose::STANDARD;
        body["attachments"] = json!(
            parsed
                .attachments
                .iter()
                .map(|a| json!({
                    "filename": a.filename,
                    "content_type": a.mime_type,
                    "content_base64": engine.encode(&a.data),
                }))
                .collect::<Vec<_>>()
        );
    }
    Ok(body)
}

/// One send's idempotency key: its Message-ID, else its bytes.
fn idempotency_key(raw: &[u8]) -> String {
    let id = mail_mime::parse(raw).ok().and_then(|p| p.headers.message_id);
    let digest = Sha256::digest(id.as_deref().map_or(raw, str::as_bytes));
    digest.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

fn system_label(id: &str, name: &str, visible: bool) -> Label {
    Label { id: LabelId::new(id), name: name.into(), kind: LabelKind::System, color: None, visible }
}

/// The labels a Primitive mailbox has: the system ones. User labels are
/// created on the Mac.
pub fn labels() -> Vec<Label> {
    vec![
        system_label(system_labels::INBOX, "INBOX", true),
        system_label(system_labels::SENT, "SENT", true),
        system_label(system_labels::DRAFT, "DRAFT", true),
        system_label(system_labels::SPAM, "SPAM", true),
        system_label(system_labels::TRASH, "TRASH", true),
        system_label(system_labels::STARRED, "STARRED", true),
        system_label(system_labels::UNREAD, "UNREAD", false),
    ]
}

#[async_trait]
impl MailProvider for PrimitiveProvider {
    async fn profile(&self) -> ProviderResult<Profile> {
        // The baseline cursor first, so nothing arriving while the first
        // listing runs is missed (spec §7.4).
        let page: wire::Envelope<wire::Changes> =
            self.get("changes", &[("since", "start".into())], Priority::Interactive).await?;
        let cursor = page.data.next_cursor;
        self.remember_cursor(&cursor);
        let inbound: wire::Envelope<Vec<wire::InboundRow>> =
            self.get("emails", &[("limit", "1".into())], Priority::Interactive).await?;
        let sent: wire::Envelope<Vec<wire::SentRow>> =
            self.get("sent-emails", &[("limit", "1".into())], Priority::Interactive).await?;
        let total = inbound.meta.and_then(|m| m.total).unwrap_or(0) + sent.meta.and_then(|m| m.total).unwrap_or(0);
        Ok(Profile { email: self.address.clone(), messages_total: total, cursor: SyncCursor(cursor) })
    }

    async fn list_labels(&self) -> ProviderResult<Vec<Label>> {
        let mut all = labels();
        all.extend(self.routing().label());
        Ok(all)
    }

    async fn list_message_ids(&self, filter: &ListFilter, page: Option<PageToken>) -> ProviderResult<IdPage> {
        let Some((sources, since)) = plan_listing(filter, now_millis()) else {
            return Ok(IdPage { ids: vec![], next: None, estimated_total: Some(0) });
        };
        // The token is `<source>|<cursor>`: which listing, and where in it.
        let (source, cursor) = match page {
            Some(PageToken(token)) => {
                let (tag, cursor) = token.split_once('|').unwrap_or((token.as_str(), ""));
                let source =
                    Source::from_tag(tag).ok_or_else(|| ProviderError::Invalid(format!("bad page token {token}")))?;
                (source, (!cursor.is_empty()).then(|| cursor.to_owned()))
            }
            None => (sources[0], None),
        };
        let (ids, next_cursor, total) = self.list_page(source, cursor, since).await?;
        let next = match next_cursor.filter(|c| !c.is_empty()) {
            Some(c) => Some(PageToken(format!("{}|{c}", source.tag()))),
            None => sources.iter().skip_while(|s| **s != source).nth(1).map(|s| PageToken(format!("{}|", s.tag()))),
        };
        Ok(IdPage { ids, next, estimated_total: total })
    }

    /// The messages that are this agent's; others' are left out, as if gone.
    async fn fetch_messages(&self, ids: &[MessageId], priority: Priority) -> ProviderResult<Vec<FetchedMessage>> {
        let routing = self.routing();
        let mut out = Vec::with_capacity(ids.len());
        let mut pending = ids.iter();
        let mut running = FuturesUnordered::new();
        loop {
            while running.len() < FETCH_CONCURRENCY {
                match pending.next() {
                    Some(id) => running.push(self.fetch_one(id, priority, &routing)),
                    None => break,
                }
            }
            match running.next().await {
                Some(result) => {
                    if let Some(m) = result? {
                        out.push(m);
                    }
                }
                None => return Ok(out),
            }
        }
    }

    /// The feed is the account's: every agent sees every arrival and
    /// sending, and [`Self::fetch_messages`] leaves out what is another's.
    async fn changes_since(&self, cursor: &SyncCursor) -> ProviderResult<ChangeSet> {
        let mut since = cursor.0.clone();
        let mut changes = Vec::new();
        loop {
            let page: wire::Envelope<wire::Changes> = self
                // Primitive answers 400 to a limit above 100.
                .get("changes", &[("since", since.clone()), ("limit", "100".into())], Priority::Interactive)
                .await?;
            let page = page.data;
            for row in page.changes {
                let thread = |id: &MessageId| thread_of(row.thread_id.as_ref(), id);
                match (row.kind.as_str(), &row.email_id, &row.sent_email_id) {
                    ("email.visible", Some(uuid), _) => {
                        let id = inbound_id(uuid);
                        changes.push(Change::MessageAdded {
                            thread_id: thread(&id),
                            label_ids: vec![LabelId::new(system_labels::INBOX)],
                            id,
                        });
                    }
                    ("sent_email.created", _, Some(uuid)) => {
                        let id = outbound_id(uuid);
                        changes.push(Change::MessageAdded {
                            thread_id: thread(&id),
                            label_ids: vec![LabelId::new(system_labels::SENT)],
                            id,
                        });
                    }
                    ("email.deleted", Some(uuid), _) => changes.push(Change::MessageDeleted { id: inbound_id(uuid) }),
                    ("sent_email.deleted", _, Some(uuid)) => {
                        changes.push(Change::MessageDeleted { id: outbound_id(uuid) });
                    }
                    // Read state, mutes, agents and access: not mail.
                    _ => {}
                }
            }
            since = page.next_cursor;
            if !page.has_more {
                break;
            }
        }
        self.remember_cursor(&since);
        Ok(ChangeSet { changes, cursor: SyncCursor(since) })
    }

    /// Labels are local only (spec §7.9): nothing to tell Primitive.
    async fn modify_labels(&self, _op: &LabelOp) -> ProviderResult<()> {
        Ok(())
    }

    /// Trash is local only; mail is never deleted at Primitive.
    async fn move_to_trash(&self, _id: &MessageId) -> ProviderResult<()> {
        Ok(())
    }

    async fn restore_from_trash(&self, _id: &MessageId) -> ProviderResult<()> {
        Ok(())
    }

    async fn send(&self, raw: &[u8], _thread: Option<&ThreadId>) -> ProviderResult<MessageId> {
        let body = send_body(raw, &self.address)?;
        let key = idempotency_key(raw);
        let url = self.url("send-mail");
        let sent: wire::Envelope<wire::SendResult> = self
            .http
            .json(1, Priority::Interactive, |c| c.post(&url).header("Idempotency-Key", &key).json(&body))
            .await
            .map_err(|e| match e {
                // Primitive lists every gate it tried; say what it means.
                ProviderError::Forbidden(m) if m.contains("recipient-scope gates") => {
                    ProviderError::Forbidden(not_allowed(body["to"].as_str().unwrap_or("them")))
                }
                other => other,
            })?;
        if !sent.data.rejected.is_empty() {
            return Err(ProviderError::Forbidden(format!(
                "Primitive did not accept {} ({})",
                sent.data.rejected.join(", "),
                sent.data.status
            )));
        }
        Ok(outbound_id(&sent.data.id))
    }

    async fn fetch_attachment(&self, message: &MessageId, attachment_id: &str) -> ProviderResult<Vec<u8>> {
        let path = if let Some(uuid) = message.as_str().strip_prefix(OUTBOUND) {
            format!("sent-emails/{uuid}/attachments/{attachment_id}")
        } else if let Some(uuid) = message.as_str().strip_prefix(INBOUND) {
            format!("emails/{uuid}/attachments/{attachment_id}")
        } else {
            return Err(ProviderError::NotFound(message.as_str().to_owned()));
        };
        let url = self.url(&path);
        Ok(self.http.bytes(1, Priority::Interactive, |c| c.get(&url)).await?.0)
    }

    /// No drafts at Primitive: drafts stay on the Mac until sent.
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

    fn label_sync(&self) -> provider_api::LabelSync {
        provider_api::LabelSync::Local
    }

    /// Primitive may give a sent message its own Message-ID.
    fn adopts_sent_copies(&self) -> bool {
        true
    }

    /// Labels are local only: the label exists on this Mac.
    async fn create_label(&self, name: &str, color: Option<(&str, &str)>) -> ProviderResult<Label> {
        let slug: String = name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        Ok(Label {
            id: LabelId(format!("Local_{slug}")),
            name: name.to_owned(),
            kind: LabelKind::User,
            color: color.map(|(bg, fg)| LabelColor { background: bg.to_owned(), text: fg.to_owned() }),
            visible: true,
        })
    }
}

/// New mail at once: the change feed's long-poll in place of IMAP IDLE.
/// Fetches go through the provider; listing stays on it too.
pub struct PrimitivePush(pub Arc<PrimitiveProvider>);

#[async_trait]
impl BackfillSource for PrimitivePush {
    async fn fetch(&self, ids: &[MessageId]) -> ProviderResult<Vec<FetchedMessage>> {
        self.0.fetch_messages(ids, Priority::Background).await
    }

    async fn watch(&self, max: Duration) -> ProviderResult<Option<bool>> {
        self.0.wait_for_change(max).await.map(Some)
    }

    fn name(&self) -> &'static str {
        "primitive"
    }
}

/// Creating and verifying Primitive accounts.
pub struct PrimitiveService {
    client: reqwest::Client,
    base: String,
}

impl PrimitiveService {
    pub fn new() -> ProviderResult<Self> {
        Self::with_base(PRIMITIVE_API)
    }

    pub fn with_base(base: &str) -> ProviderResult<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("Kaluta/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .https_only(false) // tests talk to a local mock
            .build()
            .map_err(|e| ProviderError::Network(e.without_url().to_string()))?;
        Ok(Self { client, base: base.trim_end_matches('/').to_owned() })
    }

    async fn call<T: serde::de::DeserializeOwned>(&self, request: reqwest::RequestBuilder) -> ProviderResult<T> {
        let body = self.call_text(request).await?;
        let envelope: wire::Envelope<T> =
            serde_json::from_str(&body).map_err(|e| ProviderError::Decode(e.to_string()))?;
        Ok(envelope.data)
    }

    /// A call's body as text, or its error classified and worded.
    async fn call_text(&self, request: reqwest::RequestBuilder) -> ProviderResult<String> {
        let response = request.send().await.map_err(|e| ProviderError::Network(e.without_url().to_string()))?;
        let status = response.status();
        let body = response.text().await.map_err(|e| ProviderError::Network(e.without_url().to_string()))?;
        if status.is_success() {
            return Ok(body);
        }
        let error = serde_json::from_str::<serde_json::Value>(&body).ok().map(|v| v["error"].clone());
        let code = error.as_ref().and_then(|e| e["code"].as_str().map(str::to_owned)).unwrap_or_default();
        let message = match code.as_str() {
            "mx_conflict" => DOMAIN_RECEIVES_ELSEWHERE.to_owned(),
            "conflict" if status.as_u16() == 409 => DOMAIN_TAKEN.to_owned(),
            _ => error
                .as_ref()
                .and_then(|e| e["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("Primitive answered {}", status.as_u16())),
        };
        Err(match status.as_u16() {
            401 => ProviderError::Unauthorized,
            403 => ProviderError::Forbidden(message),
            404 => ProviderError::NotFound(message),
            429 => ProviderError::RateLimited { retry_after: None },
            s if s >= 500 => ProviderError::Server { status: s, message },
            _ => ProviderError::Invalid(message),
        })
    }
}

/// A domain whose mail already goes elsewhere (Primitive's `mx_conflict`).
pub const DOMAIN_RECEIVES_ELSEWHERE: &str = "That domain already receives mail somewhere else. Use a subdomain for \
     the agent, such as agents.example.com, so your own mail is not affected.";
/// A domain another account has claimed.
pub const DOMAIN_TAKEN: &str = "Another Primitive account has already added that domain.";

fn record_of(r: wire::DnsRecord) -> DnsRecord {
    DnsRecord {
        kind: r.kind,
        fqdn: r.fqdn,
        value: r.value,
        priority: r.priority,
        purpose: r.purpose,
        required: r.required,
        status: r.status,
        message: r.message,
    }
}

fn domain_of(d: wire::Domain) -> MailboxDomain {
    MailboxDomain {
        id: d.id,
        domain: d.domain,
        verified: d.verified,
        records: d.dns_records.into_iter().map(record_of).collect(),
    }
}

fn plan_of(name: String, email: Option<String>, limits: &wire::Limits) -> MailboxPlan {
    let agent = name == "agent";
    MailboxPlan {
        verified: !agent,
        reply_only: agent,
        send_per_hour: limits.send_per_hour as u32,
        send_per_day: limits.send_per_day as u32,
        email,
        name,
    }
}

#[async_trait]
impl MailboxService for PrimitiveService {
    fn name(&self) -> &'static str {
        "primitive"
    }

    fn terms_url(&self) -> &'static str {
        TERMS_URL
    }

    fn code_sender_domain(&self) -> &'static str {
        CODE_SENDER_DOMAIN
    }

    /// Primitive takes no email at sign-up: verification starts later.
    async fn sign_up(
        &self,
        device_name: &str,
        idempotency_key: &str,
        _human_email: Option<&str>,
    ) -> ProviderResult<SignedUp> {
        let account: wire::AgentAccount = self
            .call(
                self.client
                    .post(format!("{}/agent/accounts", self.base))
                    .header("Idempotency-Key", idempotency_key)
                    .json(&json!({ "terms_accepted": true, "device_name": device_name })),
            )
            .await?;
        let address = account
            .address
            .clone()
            .ok_or_else(|| ProviderError::Invalid("Primitive created the account without an address".into()))?;
        Ok(SignedUp {
            api_key: mail_domain::Redacted::new(account.api_key.clone()),
            address,
            plan: plan_of(account.plan.clone(), None, &account.limits),
            inbox_id: None,
        })
    }

    async fn plan(&self, api_key: &str) -> ProviderResult<MailboxPlan> {
        let account: wire::Account =
            self.call(self.client.get(format!("{}/account", self.base)).bearer_auth(api_key)).await?;
        let email = account.email.filter(|e| !e.is_empty() && Some(e) != account.managed_inbox_address.as_ref());
        Ok(plan_of(account.plan, email, &account.limits))
    }

    async fn start_verification(&self, api_key: &str, email: &str) -> ProviderResult<VerificationStarted> {
        let started: wire::ClaimStarted = self
            .call(
                self.client
                    .post(format!("{}/agent/claim/start", self.base))
                    .bearer_auth(api_key)
                    .json(&json!({ "email": email })),
            )
            .await?;
        Ok(VerificationStarted {
            resend_after_secs: started.resend_after_seconds,
            expires_in_secs: started.expires_in_seconds,
        })
    }

    async fn verify(&self, api_key: &str, code: &str) -> ProviderResult<MailboxPlan> {
        let claimed: wire::Claimed = self
            .call(
                self.client
                    .post(format!("{}/agent/claim/verify", self.base))
                    .bearer_auth(api_key)
                    .json(&json!({ "verification_code": code.trim() })),
            )
            .await?;
        Ok(plan_of(claimed.plan, claimed.email, &claimed.limits))
    }

    async fn send_rules(&self, api_key: &str) -> ProviderResult<Vec<SendRule>> {
        let rules: Vec<wire::SendPermission> =
            self.call(self.client.get(format!("{}/send-permissions", self.base)).bearer_auth(api_key)).await?;
        Ok(rules
            .into_iter()
            .filter_map(|r| match r.kind.as_str() {
                "any_recipient" => Some(SendRule::AnyRecipient),
                "managed_zone" => r.zone.map(SendRule::ManagedZone),
                "your_domain" => r.domain.map(SendRule::YourDomain),
                "address" => r.address.map(SendRule::Address),
                _ => None,
            })
            .collect())
    }

    async fn domains(&self, api_key: &str) -> ProviderResult<Vec<MailboxDomain>> {
        let domains: Vec<wire::Domain> =
            self.call(self.client.get(format!("{}/domains", self.base)).bearer_auth(api_key)).await?;
        Ok(domains.into_iter().map(domain_of).collect())
    }

    async fn add_domain(&self, api_key: &str, domain: &str) -> ProviderResult<MailboxDomain> {
        let domain = domain.trim().trim_end_matches('.').to_lowercase();
        let added: wire::Domain = self
            .call(
                self.client
                    .post(format!("{}/domains", self.base))
                    .bearer_auth(api_key)
                    .header("Idempotency-Key", format!("add-domain-{domain}"))
                    .json(&json!({ "domain": domain })),
            )
            .await?;
        Ok(domain_of(added))
    }

    async fn verify_domain(&self, api_key: &str, domain_id: &str) -> ProviderResult<MailboxDomain> {
        let checked: wire::DomainCheck = self
            .call(self.client.post(format!("{}/domains/{domain_id}/verify", self.base)).bearer_auth(api_key))
            .await?;
        // The check carries no name; the listing does.
        let name =
            self.domains(api_key).await?.into_iter().find(|d| d.id == domain_id).map(|d| d.domain).unwrap_or_default();
        Ok(MailboxDomain {
            id: domain_id.to_owned(),
            domain: name,
            verified: checked.verified,
            records: checked.dns_records.into_iter().map(record_of).collect(),
        })
    }

    async fn zone_file(&self, api_key: &str, domain_id: &str) -> ProviderResult<String> {
        self.call_text(self.client.get(format!("{}/domains/{domain_id}/zone-file", self.base)).bearer_auth(api_key))
            .await
    }
}

#[cfg(test)]
mod tests;
