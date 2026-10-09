//! AgentMail (agentmail.to) agent mailboxes (spec §7.9, ADR 0014, ADR 0015).
//!
//! An AgentMail *organisation* is a service account: one key, one human
//! email, one plan. Each agent is an *inbox* in it. [`AgentMailService`]
//! signs up (once per human email: signing up again rotates the key),
//! verifies, adds inboxes and makes inbox-scoped keys;
//! [`AgentMailProvider`] syncs one inbox over the REST API, mapping its
//! string labels onto the store's Gmail-shaped ones ([`labels`]) both ways.
//!
//! - Messages are listed per inbox and fetched as raw MIME (`…/raw` gives
//!   a signed download URL), falling back to the JSON fields.
//! - New mail is found by listing by time with an overlap; label changes
//!   made elsewhere come from the inbox's event list (`label.added`,
//!   `label.removed`, newest first) down to the last event seen. Both
//!   positions are in the sync cursor ([`wire::Cursor`]).
//! - Label changes made in the app go out as `PATCH` (or `batch-update`)
//!   through the outbox. Trash and delete stay on this Mac.
//! - Sends turn the composer's MIME into AgentMail's JSON, with an
//!   `Idempotency-Key` and an `X-OpenAGC-Outbox-Id` header. After an answer
//!   that leaves it unknown whether the mail went (a timeout, a 5xx), the
//!   next attempt first looks for that header among the inbox's recent
//!   messages and takes a match as sent ([`AgentMailProvider::send`]).

mod errors;
pub mod labels;
pub mod push;
mod service;
pub mod wire;
#[cfg(any(test, feature = "fake-ws"))]
pub mod ws_fake;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use futures::stream::{FuturesUnordered, StreamExt};
use mail_domain::{EmailAddress, Label, LabelColor, LabelId, LabelKind, MessageId, Millis, ThreadId, system_labels};
use provider_api::{
    Change, ChangeSet, FetchedAttachment, FetchedBody, FetchedMessage, HttpClient, IdPage, LabelOp, LabelSync,
    ListFilter, MailProvider, PageToken, Priority, Profile, ProviderError, ProviderResult, RateLimiter, RetryPolicy,
    SyncCursor, TokenSource,
};
use serde_json::json;
use sha2::{Digest, Sha256};

pub use errors::{
    INBOX_LIMIT, INBOX_PAUSED, NOT_VERIFIED_YET, SENDS_ONLY_TO_HUMAN, USERNAME_TAKEN, agentmail_error, too_large,
};
pub use labels::{ARCHIVED, labels};
pub use push::{AGENTMAIL_WS, AgentMailPush, AgentMailSocket, SocketConfig, SocketState};
pub use service::{AgentMailService, plan_name, username_of};

pub const AGENTMAIL_API: &str = "https://api.agentmail.to";
pub const TERMS_URL: &str = "https://www.agentmail.to/legal/terms";
/// AgentMail's console; the user signs in with the email the service
/// account was created with.
pub const DASHBOARD_URL: &str = "https://console.agentmail.to";
/// Where AgentMail's verification codes come from, for *Fill Code*. Not
/// confirmed against a real code (hand-check list in the plan).
pub const CODE_SENDER_DOMAIN: &str = "agentmail.to";
/// The domain inboxes take addresses on unless the organisation has its own.
pub const MANAGED_DOMAIN: &str = "agentmail.to";
/// The header naming the outbox entry a send came from.
pub const OUTBOX_HEADER: &str = "X-OpenAGC-Outbox-Id";
/// AgentMail's limit on a send request, body and attachments included.
pub const MAX_REQUEST_BYTES: usize = 6 * 1024 * 1024;

/// Rows per listing page.
const PAGE_SIZE: u32 = 100;
/// Ids per `batch-update`.
const BATCH_SIZE: usize = 50;
/// Concurrent message fetches; the rate limiter paces them.
const FETCH_CONCURRENCY: usize = 6;
/// Listing new mail by time looks back this far past the newest message
/// seen, for mail that shows up late.
const OVERLAP_MS: Millis = 60 * 60 * 1000;
/// Pages read per poll before giving up on reaching the last event seen
/// (the cursor is then treated as expired).
const MAX_PAGES: usize = 20;
/// A send queued longer ago than this may have been tried before (in an
/// earlier run of the app): look for it among sent mail first.
const RETRY_CHECK_AFTER_MS: Millis = 10 * 60 * 1000;
/// Snippet length, as for Gmail.
const SNIPPET_CHARS: usize = 200;

/// AgentMail does not publish its request limit ("generous", 429 with
/// `Retry-After` past it): five a second, shared by an organisation's
/// agents as they share the key.
pub fn rate_limiter() -> Arc<RateLimiter> {
    Arc::new(RateLimiter::new(300, 30))
}

/// One synced agent: an inbox of an AgentMail organisation.
pub struct AgentMailProvider {
    http: HttpClient,
    /// Sends: never repeated by the client itself (see [`Self::send`]).
    send_http: HttpClient,
    /// Signed download URLs take no key.
    download: reqwest::Client,
    base: String,
    inbox_id: String,
    address: String,
    /// Outbox ids whose send ended without a clear answer, this run.
    unsure: Mutex<HashSet<String>>,
}

impl AgentMailProvider {
    pub fn new(tokens: Arc<dyn TokenSource>, address: &str, inbox_id: &str) -> ProviderResult<Self> {
        Self::for_inbox(tokens, address, inbox_id, rate_limiter(), RetryPolicy::default(), AGENTMAIL_API)
    }

    /// One inbox of an organisation whose agents share `limiter`.
    pub fn for_inbox(
        tokens: Arc<dyn TokenSource>,
        address: &str,
        inbox_id: &str,
        limiter: Arc<RateLimiter>,
        retry: RetryPolicy,
        base: &str,
    ) -> ProviderResult<Self> {
        let hook: provider_api::http::ErrorHook = Arc::new(agentmail_error);
        let http = HttpClient::new(tokens, limiter, retry)?.with_error_hook(hook);
        let send_http = http.clone().with_retry(RetryPolicy { max_attempts: 1, ..retry });
        let download = reqwest::Client::builder()
            .user_agent(concat!("OpenAGC/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(|e| ProviderError::Network(e.without_url().to_string()))?;
        Ok(Self {
            http,
            send_http,
            download,
            base: base.trim_end_matches('/').to_owned(),
            inbox_id: inbox_id.to_owned(),
            address: address.to_owned(),
            unsure: Mutex::new(HashSet::new()),
        })
    }

    /// Tests: give up on a send after `timeout`.
    pub fn with_send_timeout(mut self, timeout: Duration) -> ProviderResult<Self> {
        self.send_http = self.send_http.with_timeout(timeout)?;
        Ok(self)
    }

    /// `…/v0/inboxes/{inbox}/<segments>`, each segment escaped.
    fn url(&self, segments: &[&str]) -> String {
        let mut url = format!("{}/v0/inboxes/{}", self.base, escape(&self.inbox_id));
        for s in segments {
            url.push('/');
            url.push_str(&escape(s));
        }
        url
    }

    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, String)],
        priority: Priority,
    ) -> ProviderResult<T> {
        let url = self.url(segments);
        self.http.json(1, priority, |c| c.get(&url).query(query)).await
    }

    /// Bytes behind a signed download URL.
    async fn download(&self, url: &str) -> ProviderResult<Vec<u8>> {
        let response =
            self.download.get(url).send().await.map_err(|e| ProviderError::Network(e.without_url().to_string()))?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            return Err(match status {
                403 | 404 | 410 => ProviderError::NotFound(format!("download answered {status}")),
                s if s >= 500 => ProviderError::Server { status: s, message: "download failed".into() },
                s => ProviderError::Invalid(format!("download answered {s}")),
            });
        }
        Ok(response.bytes().await.map_err(|e| ProviderError::Network(e.without_url().to_string()))?.to_vec())
    }

    /// The raw MIME of a message; `None` when AgentMail has none to give.
    async fn raw(&self, id: &str, priority: Priority) -> ProviderResult<Option<Vec<u8>>> {
        let link: wire::Download = match self.get(&["messages", id, "raw"], &[], priority).await {
            Ok(link) => link,
            Err(ProviderError::NotFound(_) | ProviderError::Forbidden(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        match self.download(&link.download_url).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(ProviderError::NotFound(_) | ProviderError::Invalid(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// A message with its labels and raw form; `None` when it is gone.
    async fn fetch_one(&self, id: &MessageId, priority: Priority) -> ProviderResult<Option<FetchedMessage>> {
        let record: wire::Message = match self.get(&["messages", id.as_str()], &[], priority).await {
            Ok(m) => m,
            Err(ProviderError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let raw = self.raw(id.as_str(), priority).await?;
        Ok(Some(message_of(&record, raw.as_deref())))
    }

    /// One page of the inbox's messages, newest first.
    async fn list(
        &self,
        query: &[(&str, String)],
        page: Option<String>,
        priority: Priority,
    ) -> ProviderResult<wire::MessagePage> {
        let mut query = query.to_vec();
        query.push(("limit", PAGE_SIZE.to_string()));
        if let Some(token) = page {
            query.push(("page_token", token));
        }
        self.get(&["messages"], &query, priority).await
    }

    /// Every message from `after` on, newest first. More than
    /// [`MAX_PAGES`] pages is [`ProviderError::CursorExpired`]: the listing
    /// is newest first, so stopping there would report only the newest and
    /// move the cursor past the rest, which no later poll would list. A
    /// resync lists them all instead.
    async fn list_since(&self, after: Millis, priority: Priority) -> ProviderResult<Vec<wire::Message>> {
        let mut out = Vec::new();
        let mut page = None;
        for _ in 0..MAX_PAGES {
            let got = self.list(&[("after", rfc3339(after))], page, priority).await?;
            out.extend(got.messages);
            match got.next_page_token.filter(|t| !t.is_empty()) {
                Some(next) => page = Some(next),
                None => return Ok(out),
            }
        }
        tracing::info!(pages = MAX_PAGES, "more new mail than a poll reads; resyncing");
        Err(ProviderError::CursorExpired)
    }

    /// Whether a message is in the Inbox at AgentMail now ([`labels::to_local`]
    /// of its labels); `false` when it is gone.
    async fn in_inbox(&self, id: &str) -> ProviderResult<bool> {
        match self.get::<wire::Message>(&["messages", id], &[], Priority::Background).await {
            Ok(m) => Ok(labels::to_local(&m.labels).iter().any(|l| l.as_str() == system_labels::INBOX)),
            Err(ProviderError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Label events newer than `cursor`'s, oldest first, and the newest
    /// event seen. [`ProviderError::CursorExpired`] when the last event
    /// seen was not reached within [`MAX_PAGES`].
    async fn events_since(
        &self,
        cursor: &wire::Cursor,
    ) -> ProviderResult<(Vec<wire::Event>, Option<(String, Millis)>)> {
        let mut newer = Vec::new();
        let mut page: Option<String> = None;
        let mut newest = None;
        for _ in 0..MAX_PAGES {
            let mut query = vec![("limit", PAGE_SIZE.to_string())];
            if let Some(token) = page.take() {
                query.push(("page_token", token));
            }
            let got: wire::EventPage = self.get(&["events"], &query, Priority::Background).await?;
            for e in got.events {
                let at = e.event_at.as_deref().and_then(parse_time).unwrap_or(0);
                if newest.is_none() {
                    newest = Some((e.event_id.clone(), at));
                }
                let reached = cursor.event.as_deref() == Some(e.event_id.as_str())
                    || (cursor.event_at > 0 && at > 0 && at < cursor.event_at);
                if reached {
                    newer.reverse();
                    return Ok((newer, newest));
                }
                newer.push(e);
            }
            match got.next_page_token.filter(|t| !t.is_empty()) {
                Some(next) => page = Some(next),
                None => {
                    // The whole list is newer than the cursor (or the event
                    // seen last is gone): apply it all, oldest first.
                    newer.reverse();
                    return Ok((newer, newest));
                }
            }
        }
        Err(ProviderError::CursorExpired)
    }

    /// The message an earlier attempt of this send left at AgentMail, by
    /// its [`OUTBOX_HEADER`]; `None` when there is none.
    async fn find_sent(&self, outbox_id: &str, subject: &str, queued: Millis) -> ProviderResult<Option<String>> {
        // Not sent before it was queued; the overlap allows for clocks.
        let after = queued.min(now_millis()) - OVERLAP_MS;
        let mut candidates = Vec::new();
        let mut page = None;
        for _ in 0..3 {
            let got = self.list(&[("after", rfc3339(after))], page, Priority::Interactive).await?;
            for m in got.messages {
                match &m.headers {
                    Some(h) if header(h, OUTBOX_HEADER).is_some() => {
                        if header(h, OUTBOX_HEADER) == Some(outbox_id) {
                            return Ok(Some(m.message_id));
                        }
                    }
                    // The listing left the headers out: the message itself
                    // has them, for the ones that could be this send.
                    _ if m.subject.as_deref().unwrap_or("").trim() == subject.trim()
                        && m.labels.iter().all(|l| !l.eq_ignore_ascii_case("received")) =>
                    {
                        candidates.push(m.message_id);
                    }
                    _ => {}
                }
            }
            match got.next_page_token.filter(|t| !t.is_empty()) {
                Some(next) => page = Some(next),
                None => break,
            }
        }
        for id in candidates.into_iter().take(10) {
            let full: wire::Message = match self.get(&["messages", &id], &[], Priority::Interactive).await {
                Ok(m) => m,
                Err(ProviderError::NotFound(_)) => continue,
                Err(e) => return Err(e),
            };
            if full.headers.as_ref().and_then(|h| header(h, OUTBOX_HEADER)) == Some(outbox_id) {
                return Ok(Some(full.message_id));
            }
        }
        Ok(None)
    }

    async fn post_send(&self, url: &str, body: &serde_json::Value, key: &str) -> ProviderResult<wire::Sent> {
        self.send_http.json(1, Priority::Interactive, |c| c.post(url).header("Idempotency-Key", key).json(body)).await
    }

    fn unsure(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.unsure.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A path segment, escaped (`@`, `+` and `=` stay as they are).
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~@!$&'()*+,;=:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn header<'a>(headers: &'a HashMap<String, String>, name: &str) -> Option<&'a str> {
    headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.trim())
}

fn rfc3339(millis: Millis) -> String {
    chrono::DateTime::from_timestamp_millis(millis)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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

fn strip_angles(s: &str) -> String {
    s.trim().trim_start_matches('<').trim_end_matches('>').to_owned()
}

fn angled(id: &str) -> String {
    format!("<{}>", strip_angles(id))
}

/// When a message was sent or received, in milliseconds.
fn time_of(m: &wire::Message) -> Millis {
    m.timestamp.as_deref().or(m.created_at.as_deref()).and_then(parse_time).unwrap_or(0)
}

/// A message from its record and raw MIME, or from the record's own
/// fields when there is no raw form.
fn message_of(record: &wire::Message, raw: Option<&[u8]>) -> FetchedMessage {
    let id = MessageId(record.message_id.clone());
    let thread_id = ThreadId(record.thread_id.clone());
    let label_ids = labels::to_local(&record.labels);
    let at = time_of(record);
    if let Some(parsed) = raw.and_then(|r| mail_mime::parse(r).ok()) {
        let h = parsed.headers;
        let snippet = snippet(parsed.text.as_deref().or(parsed.html.as_deref()).unwrap_or(""));
        return FetchedMessage {
            id,
            thread_id,
            label_ids,
            snippet,
            internal_date: at,
            size_estimate: record.size.unwrap_or(raw.map_or(0, |r| r.len() as u64)),
            message_id_header: h.message_id,
            in_reply_to: h.in_reply_to,
            references: h.references,
            from: h.from,
            to: h.to,
            cc: h.cc,
            bcc: h.bcc,
            reply_to: h.reply_to,
            subject: h.subject,
            date: h.date.or(Some(at)),
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
        };
    }
    let joined = |v: &[String]| v.join(", ");
    let (to, cc, bcc, reply_to) =
        (joined(&record.to), joined(&record.cc), joined(&record.bcc), joined(&record.reply_to));
    let h = mail_mime::parse_headers([
        ("From", record.from.as_deref().unwrap_or_default()),
        ("To", to.as_str()),
        ("Cc", cc.as_str()),
        ("Bcc", bcc.as_str()),
        ("Reply-To", reply_to.as_str()),
    ]);
    let message_id_header = record
        .headers
        .as_ref()
        .and_then(|hs| header(hs, "Message-ID"))
        .map(strip_angles)
        .or_else(|| record.message_id.contains('@').then(|| strip_angles(&record.message_id)));
    let text = record.text.clone();
    FetchedMessage {
        id,
        thread_id,
        label_ids,
        snippet: record
            .preview
            .clone()
            .map(|p| snippet(&p))
            .unwrap_or_else(|| snippet(text.as_deref().or(record.html.as_deref()).unwrap_or(""))),
        internal_date: at,
        size_estimate: record.size.unwrap_or(0),
        message_id_header,
        in_reply_to: record.in_reply_to.as_deref().map(strip_angles),
        references: record.references.iter().map(|r| strip_angles(r)).collect(),
        from: h.from,
        to: h.to,
        cc: h.cc,
        bcc: h.bcc,
        reply_to: h.reply_to,
        subject: record.subject.clone().unwrap_or_default(),
        date: Some(at),
        list: Default::default(),
        body: Some(FetchedBody {
            text,
            html: record.html.clone(),
            attachments: record
                .attachments
                .iter()
                .map(|a| FetchedAttachment {
                    part_id: Some(a.attachment_id.clone()),
                    attachment_id: Some(a.attachment_id.clone()),
                    filename: a.filename.clone().unwrap_or_else(|| "attachment".into()),
                    mime_type: a.content_type.clone().unwrap_or_else(|| "application/octet-stream".into()),
                    size: a.size,
                    content_id: a.content_id.as_deref().map(strip_angles),
                    is_inline: a.content_disposition.as_deref() == Some("inline"),
                    data: None,
                })
                .collect(),
        }),
    }
}

/// What a listing asks AgentMail for, and what it keeps of the answer.
#[derive(Debug, Default, PartialEq, Eq)]
struct Listing {
    query: Vec<(&'static str, String)>,
    /// A store label each kept message must have.
    need: Option<&'static str>,
}

/// AgentMail's listing for a filter. `None`: nothing at AgentMail matches
/// (Trash, which is the Mac's, or a search it cannot run).
fn plan_listing(filter: &ListFilter, now: Millis) -> Option<Listing> {
    let mut listing = Listing::default();
    for label in &filter.label_ids {
        match label.as_str() {
            system_labels::INBOX => listing.need = Some(system_labels::INBOX),
            system_labels::SENT => listing.need = Some(system_labels::SENT),
            system_labels::UNREAD => listing.query.push(("labels", "unread".into())),
            system_labels::STARRED => listing.query.push(("labels", "starred".into())),
            system_labels::SPAM => {
                listing.query.push(("labels", "spam".into()));
                listing.query.push(("include_spam", "true".into()));
                listing.need = Some(system_labels::SPAM);
            }
            other if other.chars().all(|c| c.is_ascii_uppercase() || c == '_') => return None,
            user => listing.query.push(("labels", user.to_owned())),
        }
    }
    if let Some(query) = filter.query.as_deref() {
        for term in query.split_whitespace() {
            if term == "is:unread" {
                listing.query.push(("labels", "unread".into()));
            } else {
                let days = term.strip_prefix("newer_than:").and_then(|d| d.strip_suffix('d'))?;
                let days: i64 = days.parse().ok()?;
                listing.query.push(("after", rfc3339(now - days * 86_400_000)));
            }
        }
    }
    Some(listing)
}

/// An address as AgentMail takes it: `Name <email>` or the email.
fn address(a: &EmailAddress) -> String {
    match a.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => format!("\"{}\" <{}>", name.replace(['"', '\\'], ""), a.email),
        None => a.email.clone(),
    }
}

/// One send's outbox id: its Message-ID, which the composer made and the
/// outbox keeps across retries; else a digest of its bytes.
fn outbox_id(parsed: &mail_mime::ParsedMessage, raw: &[u8]) -> String {
    match parsed.headers.message_id.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        Some(id) => id.to_owned(),
        None => digest(raw),
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// The JSON AgentMail sends from, for the composer's MIME message. A reply
/// (`reply` true) leaves the subject and threading headers to AgentMail.
fn send_body(parsed: &mail_mime::ParsedMessage, outbox_id: &str, reply: bool) -> ProviderResult<serde_json::Value> {
    let h = &parsed.headers;
    if h.to.is_empty() && h.cc.is_empty() && h.bcc.is_empty() {
        return Err(ProviderError::Invalid("a message needs at least one recipient".into()));
    }
    let list = |v: &[EmailAddress]| v.iter().map(address).collect::<Vec<_>>();
    let mut headers = serde_json::Map::new();
    headers.insert(OUTBOX_HEADER.into(), json!(outbox_id));
    let mut body = json!({});
    if !h.to.is_empty() {
        body["to"] = json!(list(&h.to));
    }
    if !h.cc.is_empty() {
        body["cc"] = json!(list(&h.cc));
    }
    if !h.bcc.is_empty() {
        body["bcc"] = json!(list(&h.bcc));
    }
    if !h.reply_to.is_empty() {
        body["reply_to"] = json!(list(&h.reply_to));
    }
    if reply {
        body["reply_all"] = json!(false);
    } else {
        body["subject"] = json!(h.subject);
        if let Some(parent) = &h.in_reply_to {
            headers.insert("In-Reply-To".into(), json!(angled(parent)));
        }
        if !h.references.is_empty() {
            headers.insert(
                "References".into(),
                json!(h.references.iter().map(|r| angled(r)).collect::<Vec<_>>().join(" ")),
            );
        }
    }
    if let Some(text) = &parsed.text {
        body["text"] = json!(text);
    }
    if let Some(html) = &parsed.html {
        body["html"] = json!(html);
    }
    if parsed.text.is_none() && parsed.html.is_none() {
        body["text"] = json!("");
    }
    if !parsed.attachments.is_empty() {
        let engine = base64::engine::general_purpose::STANDARD;
        body["attachments"] = json!(
            parsed
                .attachments
                .iter()
                .map(|a| {
                    let mut part = json!({
                        "filename": a.filename,
                        "content_type": a.mime_type,
                        "content_disposition": if a.is_inline { "inline" } else { "attachment" },
                        "content": engine.encode(&a.data),
                    });
                    if let Some(cid) = &a.content_id {
                        part["content_id"] = json!(cid);
                    }
                    part
                })
                .collect::<Vec<_>>()
        );
    }
    body["headers"] = serde_json::Value::Object(headers);
    let size = serde_json::to_vec(&body).map(|b| b.len()).unwrap_or(0);
    if size > MAX_REQUEST_BYTES {
        return Err(ProviderError::Invalid(too_large(size)));
    }
    Ok(body)
}

/// Whether a failed send may still have gone out.
fn ambiguous(e: &ProviderError) -> bool {
    matches!(e, ProviderError::Network(_) | ProviderError::Server { .. } | ProviderError::Decode(_))
}

fn parse_cursor(cursor: &SyncCursor) -> ProviderResult<wire::Cursor> {
    serde_json::from_str(&cursor.0).map_err(|_| ProviderError::CursorExpired)
}

fn cursor_text(cursor: &wire::Cursor) -> SyncCursor {
    SyncCursor(serde_json::to_string(cursor).unwrap_or_default())
}

/// The messages near the newest one, already reported.
fn seen_near(messages: &[wire::Message], at: Millis) -> Vec<String> {
    let mut seen: Vec<String> =
        messages.iter().filter(|m| time_of(m) >= at - OVERLAP_MS).map(|m| m.message_id.clone()).collect();
    seen.sort();
    seen.dedup();
    seen
}

#[async_trait]
impl MailProvider for AgentMailProvider {
    async fn profile(&self) -> ProviderResult<Profile> {
        // The positions first, so nothing arriving while the first listing
        // runs is missed (spec §7.4).
        let page = self.list(&[], None, Priority::Interactive).await?;
        let at = page.messages.iter().map(time_of).max().unwrap_or_else(now_millis);
        let events: wire::EventPage = self.get(&["events"], &[("limit", "1".into())], Priority::Interactive).await?;
        let newest = events.events.first();
        let cursor = wire::Cursor {
            at,
            seen: seen_near(&page.messages, at),
            event: newest.map(|e| e.event_id.clone()),
            event_at: newest.and_then(|e| e.event_at.as_deref()).and_then(parse_time).unwrap_or(0),
        };
        Ok(Profile { email: self.address.clone(), messages_total: 0, cursor: cursor_text(&cursor) })
    }

    async fn list_labels(&self) -> ProviderResult<Vec<Label>> {
        Ok(labels())
    }

    async fn list_message_ids(&self, filter: &ListFilter, page: Option<PageToken>) -> ProviderResult<IdPage> {
        let Some(listing) = plan_listing(filter, now_millis()) else {
            return Ok(IdPage { ids: vec![], next: None, estimated_total: Some(0) });
        };
        let got = self.list(&listing.query, page.map(|p| p.0), Priority::Background).await?;
        let ids = got
            .messages
            .into_iter()
            .filter(|m| listing.need.is_none_or(|need| labels::to_local(&m.labels).iter().any(|l| l.as_str() == need)))
            .map(|m| (MessageId(m.message_id), ThreadId(m.thread_id)))
            .collect();
        Ok(IdPage { ids, next: got.next_page_token.filter(|t| !t.is_empty()).map(PageToken), estimated_total: None })
    }

    async fn fetch_messages(&self, ids: &[MessageId], priority: Priority) -> ProviderResult<Vec<FetchedMessage>> {
        let mut out = Vec::with_capacity(ids.len());
        let mut pending = ids.iter();
        let mut running = FuturesUnordered::new();
        loop {
            while running.len() < FETCH_CONCURRENCY {
                match pending.next() {
                    Some(id) => running.push(self.fetch_one(id, priority)),
                    None => break,
                }
            }
            match running.next().await {
                Some(result) => out.extend(result?),
                None => return Ok(out),
            }
        }
    }

    /// New mail by listing from the newest time seen (less an overlap),
    /// and label changes made elsewhere from the event list.
    async fn changes_since(&self, cursor: &SyncCursor) -> ProviderResult<ChangeSet> {
        let mut position = parse_cursor(cursor)?;
        let mut changes = Vec::new();

        let listed = self.list_since(position.at - OVERLAP_MS, Priority::Interactive).await?;
        let seen: HashSet<&str> = position.seen.iter().map(String::as_str).collect();
        for m in listed.iter().rev() {
            if !seen.contains(m.message_id.as_str()) {
                changes.push(Change::MessageAdded {
                    id: MessageId(m.message_id.clone()),
                    thread_id: ThreadId(m.thread_id.clone()),
                    label_ids: labels::to_local(&m.labels),
                });
            }
        }
        let at = listed.iter().map(time_of).max().unwrap_or(0).max(position.at);
        let mut still_seen = seen_near(&listed, at);
        if at == position.at {
            // Nothing newer: what was reported before is still near the
            // newest, listed now or not (a page limit).
            still_seen.extend(position.seen.iter().cloned());
            still_seen.sort();
            still_seen.dedup();
        }

        let (events, newest) = self.events_since(&position).await?;
        for e in events {
            let added = matches!(e.event_type.as_str(), "label.added" | "label_added");
            let removed = matches!(e.event_type.as_str(), "label.removed" | "label_removed");
            let (Some(message), Some(label)) = (e.message_id.as_deref(), e.label.as_deref()) else { continue };
            if !added && !removed {
                continue;
            }
            let Some((plus, minus)) = labels::event_change(label, added) else { continue };
            let id = MessageId(message.to_owned());
            if !plus.is_empty() {
                changes.push(Change::LabelsAdded { id: id.clone(), label_ids: plus });
            }
            if !minus.is_empty() {
                changes.push(Change::LabelsRemoved { id: id.clone(), label_ids: minus });
            }
            // Out of Spam or Trash: back in the Inbox if AgentMail has it there.
            if removed && labels::leaves_the_inbox(label) && self.in_inbox(message).await? {
                changes.push(Change::LabelsAdded { id, label_ids: vec![LabelId::new(system_labels::INBOX)] });
            }
        }

        position.at = at;
        position.seen = still_seen;
        if let Some((event, event_at)) = newest {
            position.event = Some(event);
            position.event_at = event_at;
        }
        Ok(ChangeSet { changes, cursor: cursor_text(&position) })
    }

    /// Read state, archive, stars and labels go to AgentMail; trash and
    /// spam stay on this Mac.
    async fn modify_labels(&self, op: &LabelOp) -> ProviderResult<()> {
        let (add, remove) = labels::to_server(&op.add, &op.remove);
        if add.is_empty() && remove.is_empty() {
            return Ok(());
        }
        let mut ids: Vec<&str> = op.message_ids.iter().map(MessageId::as_str).collect();
        ids.sort_unstable();
        ids.dedup();
        if let [one] = ids.as_slice() {
            let url = self.url(&["messages", one]);
            let body = json!({ "add_labels": add, "remove_labels": remove });
            return self.http.empty(1, Priority::Interactive, |c| c.patch(&url).json(&body)).await;
        }
        let url = self.url(&["messages", "batch-update"]);
        for chunk in ids.chunks(BATCH_SIZE) {
            let body = json!({ "message_ids": chunk, "add_labels": add, "remove_labels": remove });
            self.http.empty(1, Priority::Interactive, |c| c.post(&url).json(&body)).await?;
        }
        Ok(())
    }

    /// Trash is local only; mail is never trashed or deleted at AgentMail.
    async fn move_to_trash(&self, _id: &MessageId) -> ProviderResult<()> {
        Ok(())
    }

    async fn restore_from_trash(&self, _id: &MessageId) -> ProviderResult<()> {
        Ok(())
    }

    /// Sends the composer's message as AgentMail's JSON: a reply through
    /// `…/{parent}/reply` (so AgentMail threads it), else `…/send`.
    ///
    /// There is no sending twice. Each send carries an `Idempotency-Key`
    /// (AgentMail keeps it 24 hours) and the [`OUTBOX_HEADER`]. The client
    /// does not repeat a send itself; when an attempt ends without a clear
    /// answer (a timeout, a 5xx), or the message was queued long enough
    /// ago that an earlier run may have tried it, the next attempt first
    /// looks for the header among the inbox's recent mail and, found,
    /// takes that message as the send.
    async fn send(&self, raw: &[u8], _thread: Option<&ThreadId>) -> ProviderResult<MessageId> {
        let parsed = mail_mime::parse(raw).map_err(|e| ProviderError::Invalid(e.to_string()))?;
        let outbox = outbox_id(&parsed, raw);
        let key = digest(outbox.as_bytes());
        let queued = parsed.headers.date.unwrap_or_else(now_millis);
        let check = self.unsure().contains(&outbox) || now_millis() - queued > RETRY_CHECK_AFTER_MS;
        if check && let Some(id) = self.find_sent(&outbox, &parsed.headers.subject, queued).await? {
            tracing::info!("an earlier attempt of this send went out; not sending again");
            self.unsure().remove(&outbox);
            return Ok(MessageId(id));
        }
        let result = match parsed.headers.in_reply_to.as_deref() {
            Some(parent) => {
                let body = send_body(&parsed, &outbox, true)?;
                match self.post_send(&self.url(&["messages", &angled(parent), "reply"]), &body, &key).await {
                    // AgentMail does not know the parent by that id: a new
                    // message with threading headers instead.
                    Err(ProviderError::NotFound(_)) => {
                        let body = send_body(&parsed, &outbox, false)?;
                        self.post_send(&self.url(&["messages", "send"]), &body, &format!("{key}-s")).await
                    }
                    other => other,
                }
            }
            None => {
                let body = send_body(&parsed, &outbox, false)?;
                self.post_send(&self.url(&["messages", "send"]), &body, &key).await
            }
        };
        match result {
            Ok(sent) => {
                self.unsure().remove(&outbox);
                Ok(MessageId(sent.message_id))
            }
            Err(e) => {
                if ambiguous(&e) {
                    tracing::warn!(error = %e, "send ended without a clear answer; checking before any retry");
                    self.unsure().insert(outbox);
                }
                Err(e)
            }
        }
    }

    /// The message an earlier attempt left, by its [`OUTBOX_HEADER`]
    /// (asked by the outbox when a send was tried before, or left in
    /// flight by a process that died, which this one's memory of unsure
    /// sends cannot know about).
    async fn already_sent(&self, raw: &[u8]) -> ProviderResult<Option<MessageId>> {
        let parsed = mail_mime::parse(raw).map_err(|e| ProviderError::Invalid(e.to_string()))?;
        let outbox = outbox_id(&parsed, raw);
        let queued = parsed.headers.date.unwrap_or_else(now_millis);
        let found = self.find_sent(&outbox, &parsed.headers.subject, queued).await?;
        if found.is_some() {
            self.unsure().remove(&outbox);
        }
        Ok(found.map(MessageId))
    }

    async fn fetch_attachment(&self, message: &MessageId, attachment_id: &str) -> ProviderResult<Vec<u8>> {
        let link: wire::Download =
            self.get(&["messages", message.as_str(), "attachments", attachment_id], &[], Priority::Interactive).await?;
        self.download(&link.download_url).await
    }

    /// Drafts stay on this Mac until sent (AgentMail's drafts are not used).
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

    /// A user label is a name at AgentMail: nothing to create there until a
    /// message carries it.
    async fn create_label(&self, name: &str, color: Option<(&str, &str)>) -> ProviderResult<Label> {
        let name = name.trim();
        if name.is_empty() || labels::is_reserved(name) {
            return Err(ProviderError::Invalid(format!(
                "AgentMail keeps the label “{name}” for itself; choose another"
            )));
        }
        Ok(Label {
            id: LabelId::new(name),
            name: name.to_owned(),
            kind: LabelKind::User,
            color: color.map(|(bg, fg)| LabelColor { background: bg.to_owned(), text: fg.to_owned() }),
            visible: true,
        })
    }

    /// Labels sync both ways: a message fetched again (a resync, a
    /// refetch) takes AgentMail's read state, Inbox, stars and user labels,
    /// and keeps what is only this Mac's ([`labels::merge_refetched`]).
    /// There is no label listing at AgentMail, so user labels in the store
    /// are kept when the list is refreshed.
    fn label_sync(&self) -> LabelSync {
        LabelSync::Both(labels::merge_refetched)
    }

    /// AgentMail gives a sent message its own id.
    fn adopts_sent_copies(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests;
