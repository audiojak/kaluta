//! Primitive REST types: only the fields OpenAGC reads. Every answer is an
//! envelope `{"success": true, "data": ..., "meta": ...}`.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Envelope<T> {
    pub data: T,
    #[serde(default)]
    pub meta: Option<Meta>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Limits {
    #[serde(default)]
    pub send_per_hour: f64,
    #[serde(default)]
    pub send_per_day: f64,
}

/// `POST /agent/accounts`.
#[derive(Debug, Deserialize)]
pub struct AgentAccount {
    pub api_key: String,
    #[serde(default)]
    pub address: Option<String>,
    pub plan: String,
    pub limits: Limits,
}

/// `GET /account`.
#[derive(Debug, Deserialize)]
pub struct Account {
    #[serde(default)]
    pub email: Option<String>,
    pub plan: String,
    pub limits: Limits,
    #[serde(default)]
    pub managed_inbox_address: Option<String>,
}

/// `POST /agent/claim/start`.
#[derive(Debug, Deserialize)]
pub struct ClaimStarted {
    #[serde(default)]
    pub resend_after_seconds: u32,
    #[serde(default)]
    pub expires_in_seconds: u32,
}

/// `POST /agent/claim/verify`.
#[derive(Debug, Deserialize)]
pub struct Claimed {
    pub plan: String,
    #[serde(default)]
    pub email: Option<String>,
    pub limits: Limits,
}

/// A row of `GET /emails`.
#[derive(Debug, Deserialize)]
pub struct InboundRow {
    pub id: String,
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// A row of `GET /sent-emails`.
#[derive(Debug, Deserialize)]
pub struct SentRow {
    pub id: String,
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// `GET /emails/{id}`.
#[derive(Debug, Deserialize)]
pub struct Inbound {
    pub id: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub status: String,
    pub received_at: String,
    #[serde(default)]
    pub raw_size_bytes: Option<u64>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub from_header: Option<String>,
    #[serde(default)]
    pub from_email: Option<String>,
    #[serde(default)]
    pub to_email: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub body_text: Option<String>,
    #[serde(default)]
    pub body_html: Option<String>,
    #[serde(default)]
    pub content_discarded_at: Option<String>,
}

/// `GET /sent-emails/{id}`.
#[derive(Debug, Deserialize)]
pub struct Sent {
    pub id: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub from_header: String,
    #[serde(default)]
    pub to_header: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body_size_bytes: u64,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub email_references: Option<String>,
    #[serde(default)]
    pub body_text: Option<String>,
    #[serde(default)]
    pub body_html: Option<String>,
    #[serde(default)]
    pub cc: Option<Vec<String>>,
    #[serde(default)]
    pub attachments: Vec<SentAttachment>,
}

#[derive(Debug, Deserialize)]
pub struct SentAttachment {
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub size_bytes: u64,
    pub part_index: u32,
}

/// `GET /changes`.
#[derive(Debug, Deserialize)]
pub struct Changes {
    #[serde(default)]
    pub changes: Vec<ChangeRow>,
    pub next_cursor: String,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Debug, Deserialize)]
pub struct ChangeRow {
    pub kind: String,
    #[serde(default)]
    pub email_id: Option<String>,
    #[serde(default)]
    pub sent_email_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// `POST /send-mail`.
#[derive(Debug, Deserialize)]
pub struct SendResult {
    pub id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub rejected: Vec<String>,
}

/// A record a domain needs (`dns_records`).
#[derive(Debug, Deserialize)]
pub struct DnsRecord {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub fqdn: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub priority: Option<u32>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub message: Option<String>,
}

/// A domain from `POST /domains` or `GET /domains`.
#[derive(Debug, Deserialize)]
pub struct Domain {
    pub id: String,
    pub domain: String,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub dns_records: Vec<DnsRecord>,
}

/// `POST /domains/{id}/verify`.
#[derive(Debug, Deserialize)]
pub struct DomainCheck {
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub dns_records: Vec<DnsRecord>,
}
