//! AgentMail REST types: only the fields Kaluta reads. Answers are plain
//! JSON objects; errors are `{name, code, message, fix, docs}`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// `POST /v0/agent/sign-up`. No `Debug`: it holds the API key.
#[derive(Deserialize)]
pub struct SignUp {
    pub inbox_id: String,
    pub api_key: String,
}

/// `POST /v0/agent/verify`.
#[derive(Debug, Deserialize)]
pub struct Verified {
    #[serde(default)]
    pub verified: bool,
}

/// `POST /v0/agent/human`.
#[derive(Debug, Deserialize)]
pub struct HumanAttached {}

/// `GET /v0/organizations`.
#[derive(Debug, Deserialize)]
pub struct Organization {
    #[serde(default)]
    pub inbox_limit: Option<u32>,
}

/// `POST /v0/inboxes`.
#[derive(Debug, Deserialize)]
pub struct Inbox {
    pub inbox_id: String,
    #[serde(default)]
    pub email: Option<String>,
}

/// `POST /v0/inboxes/{id}/api-keys`. No `Debug`: it holds the key.
#[derive(Deserialize)]
pub struct ApiKey {
    pub api_key: String,
}

/// `GET /v0/inboxes/{id}/messages`.
#[derive(Debug, Deserialize)]
pub struct MessagePage {
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

/// A listing row, or `GET …/messages/{id}` (which adds the bodies).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Message {
    pub message_id: String,
    pub thread_id: String,
    #[serde(default)]
    pub labels: Vec<String>,
    /// When it was sent or received.
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    #[serde(default)]
    pub reply_to: Vec<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub preview: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub html: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Attachment {
    pub attachment_id: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub content_disposition: Option<String>,
    #[serde(default)]
    pub content_id: Option<String>,
}

/// `…/raw` and `…/attachments/{id}`: a signed URL to download from.
#[derive(Debug, Deserialize)]
pub struct Download {
    pub download_url: String,
}

/// `GET /v0/inboxes/{id}/events`, newest first.
#[derive(Debug, Deserialize)]
pub struct EventPage {
    #[serde(default)]
    pub events: Vec<Event>,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

/// One event. Only label events matter here, but the list may hold others
/// (`message.received`, …) without a top-level message id or label: those
/// fields are optional so one such event does not fail the whole page.
#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub event_id: String,
    /// `label.added` or `label.removed` (`label_added` / `label_removed`
    /// before 2026-04), or another kind, which is skipped.
    #[serde(default)]
    pub event_type: String,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub event_at: Option<String>,
}

/// `…/messages/send` and `…/{id}/reply`.
#[derive(Debug, Deserialize)]
pub struct Sent {
    pub message_id: String,
}

/// An error answer.
#[derive(Debug, Default, Deserialize)]
pub struct ErrorBody {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub fix: Option<String>,
    #[serde(default)]
    pub errors: Option<serde_json::Value>,
}

/// Where change polling resumes, kept as JSON in the sync cursor: the
/// newest message time seen and the messages seen near it (listing by time
/// overlaps, so mail that shows up late is not missed), and the newest
/// label event seen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    /// Milliseconds: the newest `timestamp` seen.
    #[serde(default)]
    pub at: i64,
    /// Messages already reported at or after `at` less the overlap.
    #[serde(default)]
    pub seen: Vec<String>,
    /// The newest label event seen.
    #[serde(default)]
    pub event: Option<String>,
    /// And its time, in milliseconds.
    #[serde(default)]
    pub event_at: i64,
}
