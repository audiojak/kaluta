//! `openagc-rules`, the rules server for cloud agents (spec §10.6, ADR
//! 0016): it serves agent mailboxes' published writing guides and shared
//! facts to agents that cannot reach the app (a Claude cloud routine, an
//! agent on another machine).
//!
//! - **MCP over Streamable HTTP** at `/mcp` for agents, with an agent
//!   token: `guide_rules` and `facts_lookup`, named, shaped and answered as
//!   in mailbox mode (§10.1), plus the snapshot's version and time.
//! - **REST** for the app's publishing (publisher token) and read-only
//!   `GET`s for scripts (agent token).
//!
//! The app is the source of truth and the only writer: it registers a
//! mailbox, pushes full snapshots whose version only goes up, and mints
//! and revokes agent tokens. The server holds copies, hashes of tokens and
//! nothing that acts: no mail, no service key, no OAuth token. It never
//! logs a token or what a snapshot says; request logs name the token's id.
//!
//! It depends on `writing-guide` for the guide's renderers and the
//! snapshot format, and never on `openagc-core` (`cargo xtask check-deps`).

pub mod answers;
pub mod db;
pub mod limit;
mod mcp;
mod rest;
pub mod tokens;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use axum::Router;
use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

pub use db::{Db, DbError};
use limit::RateLimiter;

/// The largest request body: a snapshot is a few kilobytes.
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// The realm in `WWW-Authenticate`.
const REALM: &str = "openagc-rules";

/// How the server runs; read from flags and the environment by `main`.
#[derive(Debug, Clone)]
pub struct Config {
    /// Where the SQLite file lives.
    pub data_dir: PathBuf,
    /// Requests per minute per token (and for registrations, all together);
    /// 0 turns limiting off.
    pub rate_limit_per_minute: u32,
    /// When set, registering a mailbox needs `Authorization: Bearer` with
    /// it: a server reachable by strangers stays closed to their mailboxes.
    pub registration_token: Option<String>,
}

pub(crate) struct Inner {
    pub db: Db,
    pub limiter: RateLimiter,
    pub registration_token_hash: Option<String>,
}

/// What every handler shares.
#[derive(Clone)]
pub(crate) struct AppState(pub(crate) Arc<Inner>);

impl std::ops::Deref for AppState {
    type Target = Inner;
    fn deref(&self) -> &Inner {
        &self.0
    }
}

/// The router for these settings, with its database opened (created and
/// migrated if need be).
pub fn app(config: &Config) -> Result<Router, DbError> {
    let db = Db::open(&config.data_dir)?;
    let state = AppState(Arc::new(Inner {
        db,
        limiter: RateLimiter::new(config.rate_limit_per_minute),
        registration_token_hash: config.registration_token.as_deref().map(tokens::hash),
    }));
    Ok(rest::router(state.clone())
        .merge(mcp::router(state))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(axum::middleware::from_fn(log_requests)))
}

/// Which token a request was made with, for its log line: filled in by
/// whichever check accepted it. Never the token itself.
#[derive(Clone, Default)]
pub(crate) struct TokenSlot(Arc<OnceLock<String>>);

impl TokenSlot {
    pub fn set(&self, who: String) {
        let _ = self.0.set(who);
    }
}

/// One line per request: method, route pattern (never the path, which
/// holds an address, or the query, which holds recipients), status, time
/// and the token's id.
async fn log_requests(mut req: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = req.method().clone();
    let route = req.extensions().get::<MatchedPath>().map_or_else(|| "-".to_owned(), |p| p.as_str().to_owned());
    let slot = TokenSlot::default();
    req.extensions_mut().insert(slot.clone());
    let response = next.run(req).await;
    let token = slot.0.get().map_or("-", String::as_str);
    let status = response.status().as_u16();
    let ms = started.elapsed().as_millis();
    if route == "/healthz" {
        tracing::debug!(%method, route, status, ms, token, "request");
    } else {
        tracing::info!(%method, route, status, ms, token, "request");
    }
    response
}

/// An error answered as `{"error": code, "message": …}` plus any extra
/// fields, with the headers its status calls for.
#[derive(Debug)]
pub(crate) struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub extra: Option<Value>,
    pub retry_after: Option<u64>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), extra: None, retry_after: None }
    }

    pub fn with(mut self, extra: Value) -> Self {
        self.extra = Some(extra);
        self
    }

    /// No token, or one that is unknown, revoked or for something else.
    pub fn unauthorized(given: bool) -> Self {
        let message =
            if given { "the token is unknown or was revoked" } else { "send a token as Authorization: Bearer <token>" };
        Self::new(StatusCode::UNAUTHORIZED, if given { "invalid_token" } else { "missing_token" }, message)
    }

    pub fn too_many(wait: std::time::Duration) -> Self {
        let secs = wait.as_secs() + 1;
        let mut e =
            Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", format!("too many requests; wait {secs} s"));
        e.retry_after = Some(secs);
        e
    }

    pub fn internal(e: impl std::fmt::Display) -> Self {
        // The message stays in the log, where it names no token or content.
        tracing::error!(error = %e, "request failed");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "the server failed; see its log")
    }
}

impl From<DbError> for ApiError {
    fn from(e: DbError) -> Self {
        Self::internal(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "error": self.code, "message": self.message });
        if let (Some(Value::Object(extra)), Value::Object(map)) = (self.extra, &mut body) {
            map.extend(extra);
        }
        let mut response = (self.status, axum::Json(body)).into_response();
        let headers = response.headers_mut();
        if self.status == StatusCode::UNAUTHORIZED {
            let value = if self.code == "invalid_token" {
                format!("Bearer realm=\"{REALM}\", error=\"invalid_token\"")
            } else {
                format!("Bearer realm=\"{REALM}\"")
            };
            if let Ok(v) = HeaderValue::from_str(&value) {
                headers.insert(header::WWW_AUTHENTICATE, v);
            }
        }
        if let Some(secs) = self.retry_after {
            headers.insert(header::RETRY_AFTER, HeaderValue::from(secs));
        }
        response
    }
}

/// An agent token that was accepted: which token, and its mailbox.
#[derive(Debug, Clone)]
pub(crate) struct AgentAuth {
    pub token_id: String,
    pub mailbox_id: i64,
    pub address: String,
}

/// Check an agent token and take one request from its bucket.
pub(crate) async fn agent_auth(
    state: &AppState,
    headers: &http::HeaderMap,
    slot: Option<&TokenSlot>,
) -> Result<AgentAuth, ApiError> {
    let token = tokens::bearer(headers).ok_or_else(|| ApiError::unauthorized(false))?;
    let id = tokens::agent_token_id(token).ok_or_else(|| ApiError::unauthorized(true))?.to_owned();
    let presented = tokens::hash(token);
    let found = state.db.run({
        let id = id.clone();
        move |c| db::agent_token(c, &id)
    });
    let (row, address) = found.await?.ok_or_else(|| ApiError::unauthorized(true))?;
    if row.revoked_at.is_some() || !tokens::same(&presented, &row.token_hash) {
        return Err(ApiError::unauthorized(true));
    }
    if let Some(slot) = slot {
        slot.set(format!("agent:{id}"));
    }
    state.limiter.take(&format!("agent:{id}")).map_err(ApiError::too_many)?;
    Ok(AgentAuth { token_id: id, mailbox_id: row.mailbox_id, address })
}

/// A mailbox address as stored: trimmed and lower-cased, and shaped like
/// an address.
pub(crate) fn normalize_address(s: &str) -> Option<String> {
    let a = s.trim().to_lowercase();
    let (local, domain) = a.split_once('@')?;
    let bad = |c: char| c.is_whitespace() || c.is_control() || "/?#@\\\"<>,;".contains(c);
    let ok = (3..=254).contains(&a.len())
        && !local.is_empty()
        && !domain.is_empty()
        && !local.chars().any(bad)
        && !domain.chars().any(bad);
    ok.then_some(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_trimmed_lowercased_and_checked() {
        assert_eq!(normalize_address(" Scout@Agents.Example ").as_deref(), Some("scout@agents.example"));
        for bad in ["", "scout", "@x.com", "a@", "a b@x.com", "a@b@c.com", "a/b@x.com", "a@x.com?q"] {
            assert_eq!(normalize_address(bad), None, "{bad}");
        }
    }
}
