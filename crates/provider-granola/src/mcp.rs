//! Granola's MCP server over Streamable HTTP: JSON-RPC posted to one URL,
//! answered as JSON or as a server-sent event stream; a session id from
//! `initialize` goes with each later request. Only `tools/call` is used,
//! on Granola's read-only meeting tools. A rejected token is refreshed
//! once, and the new tokens are handed to `on_tokens` to be kept.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use provider_api::{ProviderError, ProviderResult};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::meetings::{MeetingPage, parse_meetings};
use crate::oauth::{self, Tokens, now_ms};

const PROTOCOL: &str = "2025-06-18";
const SESSION_HEADER: &str = "mcp-session-id";

/// Told when the tokens change (a refresh), to keep them.
pub type TokensChanged = Arc<dyn Fn(&Tokens) + Send + Sync>;

pub struct GranolaMcp {
    http: reqwest::Client,
    url: String,
    tokens: Mutex<Tokens>,
    session: Mutex<Option<String>>,
    next_id: AtomicU64,
    on_tokens: Option<TokensChanged>,
}

enum Sent {
    Answer(Value),
    /// The token was refused.
    Unauthorized,
    /// The session is gone (404 with a session id): start another.
    SessionGone,
}

impl GranolaMcp {
    pub fn new(http: reqwest::Client, url: &str, tokens: Tokens, on_tokens: Option<TokensChanged>) -> Self {
        Self {
            http,
            url: url.to_owned(),
            tokens: Mutex::new(tokens),
            session: Mutex::new(None),
            next_id: AtomicU64::new(1),
            on_tokens,
        }
    }

    async fn token(&self) -> ProviderResult<String> {
        let mut tokens = self.tokens.lock().await;
        if tokens.stale(now_ms()) {
            *tokens = oauth::refresh(&self.http, &tokens, &self.url).await?;
            if let Some(cb) = &self.on_tokens {
                cb(&tokens);
            }
        }
        Ok(tokens.access_token.clone())
    }

    async fn refresh_now(&self) -> ProviderResult<()> {
        let mut tokens = self.tokens.lock().await;
        *tokens = oauth::refresh(&self.http, &tokens, &self.url).await?;
        if let Some(cb) = &self.on_tokens {
            cb(&tokens);
        }
        Ok(())
    }

    /// Post one JSON-RPC message; the answer to `id` when it has one.
    async fn post(&self, body: &Value, id: Option<u64>) -> ProviderResult<Sent> {
        let token = self.token().await?;
        let session = self.session.lock().await.clone();
        let mut req = self
            .http
            .post(&self.url)
            .bearer_auth(token)
            .header("accept", "application/json, text/event-stream")
            .header("mcp-protocol-version", PROTOCOL)
            .json(body);
        if let Some(s) = &session {
            req = req.header(SESSION_HEADER, s);
        }
        let r = req.send().await.map_err(|e| ProviderError::Network(e.to_string()))?;
        let status = r.status().as_u16();
        if status == 401 {
            return Ok(Sent::Unauthorized);
        }
        if status == 404 && session.is_some() {
            return Ok(Sent::SessionGone);
        }
        if status == 429 {
            let after = r
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(std::time::Duration::from_secs);
            return Err(ProviderError::RateLimited { retry_after: after });
        }
        if status >= 500 {
            return Err(ProviderError::Server { status, message: r.text().await.unwrap_or_default() });
        }
        if !(200..300).contains(&status) {
            return Err(ProviderError::Invalid(format!(
                "Granola answered {status}: {}",
                r.text().await.unwrap_or_default()
            )));
        }
        if let Some(s) = r.headers().get(SESSION_HEADER).and_then(|v| v.to_str().ok()) {
            *self.session.lock().await = Some(s.to_owned());
        }
        let Some(id) = id else { return Ok(Sent::Answer(Value::Null)) };
        let stream = r
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/event-stream"));
        let text = r.text().await.map_err(|e| ProviderError::Decode(e.to_string()))?;
        let from_stream = || {
            text.lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
                .find(|m| m.get("id").and_then(Value::as_u64) == Some(id))
        };
        // An event stream whatever the header says, if it reads as one.
        let answer =
            if stream { from_stream() } else { serde_json::from_str::<Value>(&text).ok().or_else(from_stream) };
        let answer = answer.ok_or_else(|| ProviderError::Decode("Granola's answer could not be read".into()))?;
        if let Some(error) = answer.get("error") {
            let message = error.get("message").and_then(Value::as_str).unwrap_or("an error");
            return Err(ProviderError::Invalid(format!("Granola: {message}")));
        }
        Ok(Sent::Answer(answer.get("result").cloned().unwrap_or(Value::Null)))
    }

    async fn initialize(&self) -> ProviderResult<()> {
        *self.session.lock().await = None;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let init = json!({
            "jsonrpc": "2.0", "id": id, "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL,
                "capabilities": {},
                "clientInfo": { "name": "Kaluta", "version": env!("CARGO_PKG_VERSION") },
            },
        });
        match self.post(&init, Some(id)).await? {
            Sent::Answer(_) => {}
            Sent::Unauthorized => return Err(ProviderError::Unauthorized),
            Sent::SessionGone => return Err(ProviderError::Invalid("Granola refused a new session".into())),
        }
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        self.post(&note, None).await.map(|_| ())
    }

    /// A request in the session, starting one first if need be; a refused
    /// token is refreshed and a lost session started again, once each.
    async fn request(&self, method: &str, params: Value) -> ProviderResult<Value> {
        if self.session.lock().await.is_none() {
            match self.initialize().await {
                Err(ProviderError::Unauthorized) => {
                    self.refresh_now().await?;
                    self.initialize().await?;
                }
                other => other?,
            }
        }
        let (mut refreshed, mut restarted) = (false, false);
        loop {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
            match self.post(&body, Some(id)).await? {
                Sent::Answer(v) => return Ok(v),
                Sent::Unauthorized if !refreshed => {
                    refreshed = true;
                    self.refresh_now().await?;
                }
                Sent::SessionGone if !restarted => {
                    restarted = true;
                    self.initialize().await?;
                }
                Sent::Unauthorized => return Err(ProviderError::Unauthorized),
                Sent::SessionGone => return Err(ProviderError::Invalid("Granola keeps losing the session".into())),
            }
        }
    }

    /// Call a tool; its text. A tool's own error (`isError`) is an error.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> ProviderResult<String> {
        let result = self.request("tools/call", json!({ "name": name, "arguments": arguments })).await?;
        let text = result
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n")
            })
            .unwrap_or_default();
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(ProviderError::Invalid(format!("Granola: {}", text.trim())));
        }
        Ok(text)
    }

    /// The meetings the user took part in or captured between two local
    /// days (`YYYY-MM-DD`, inclusive), newest first, a page at a time.
    pub async fn list_meetings(
        &self,
        from_day: &str,
        to_day: &str,
        cursor: Option<&str>,
    ) -> ProviderResult<MeetingPage> {
        let mut args = json!({
            "time_range": "custom",
            "from_date": from_day,
            "to_date": to_day,
            "involvement": { "captured_by_me": true, "listed_as_participant": true },
            "limit": 50,
            "max_participants": 25,
        });
        if let Some(c) = cursor {
            args["cursor"] = json!(c);
        }
        Ok(parse_meetings(&self.call_tool("list_meetings", args).await?))
    }

    /// A meeting's notes, summary and attendees, as Granola writes them.
    pub async fn meeting_notes(&self, id: &str) -> ProviderResult<String> {
        self.call_tool("get_meetings", json!({ "meeting_ids": [id], "attachments": "none", "max_participants": 25 }))
            .await
    }

    /// Up to `max_chars` of a meeting's transcript, as Granola gives it
    /// (it says so when there is none).
    pub async fn transcript(&self, id: &str, max_chars: u32) -> ProviderResult<String> {
        self.call_tool("get_meeting_transcript", json!({ "meeting_id": id, "max_chars": max_chars.max(1000) })).await
    }
}
