//! `openagc-rules`, the rules server for cloud agents (spec §10.6, ADR
//! 0016): it serves agent mailboxes' published writing guides and shared
//! facts to agents that cannot reach the app (a Claude cloud routine, an
//! agent on another machine).
//!
//! - **MCP over Streamable HTTP** at `/mcp` for agents, with an agent
//!   token: `guide_rules` and `facts_lookup`, named, shaped and answered as
//!   in mailbox mode (§10.1), plus the snapshot's version and time;
//!   `check_draft`, the guide's deterministic check; and `report_send`,
//!   which queues what the agent sent for the app ([`reports`]).
//! - **REST** for the app's publishing (publisher token) and read-only
//!   `GET`s for scripts (agent token).
//! - **OAuth 2.1** for clients that take only a URL (a claude.ai custom
//!   connector, which a cloud routine uses): the server is its own minimal
//!   authorization server, and its consent page asks for a one-time connect
//!   code the app minted, never a password ([`oauth`]). On only with a
//!   public URL set.
//!
//! - **Encryption at rest** (`crypto`, `rules_crypto`): sealed
//!   snapshots whose key the app wraps per agent, agent keys wrapped under
//!   each agent's credential, reports sealed to the app. Plaintext only in
//!   memory, during a request.
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
mod crypto;
pub mod db;
pub mod limit;
mod mcp;
pub mod oauth;
pub mod reports;
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
    /// The address agents reach the server at, as `https://rules.example.com`
    /// (an origin, no path): the OAuth issuer, and the base of the `/mcp`
    /// resource tokens are bound to. Unset, OAuth is off.
    pub public_url: Option<String>,
    /// Refuse plaintext snapshots, and reports for a mailbox that never
    /// pushed an encrypted one (the project-hosted server sets it).
    pub require_encryption: bool,
}

/// Why the server cannot start with these settings.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("public URL: {0}")]
    PublicUrl(String),
}

pub(crate) struct Inner {
    pub db: Db,
    pub limiter: RateLimiter,
    pub registration_token_hash: Option<String>,
    pub oauth: Option<oauth::OAuth>,
    pub require_encryption: bool,
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
pub fn app(config: &Config) -> Result<Router, StartError> {
    let oauth = match config.public_url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        Some(url) => Some(oauth::OAuth::new(&oauth::public_base(url).map_err(StartError::PublicUrl)?)),
        None => None,
    };
    let db = Db::open(&config.data_dir)?;
    sweep_reports_hourly(&db);
    let state = AppState(Arc::new(Inner {
        db,
        limiter: RateLimiter::new(config.rate_limit_per_minute),
        registration_token_hash: config.registration_token.as_deref().map(tokens::hash),
        oauth,
        require_encryption: config.require_encryption,
    }));
    Ok(rest::router(state.clone())
        .merge(oauth::router(state.clone()))
        .merge(mcp::router(state))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(axum::middleware::from_fn(log_requests)))
}

/// How often reports the app never pulled are swept once 30 days old.
pub const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);

/// Delete reports past their 30 days (decision 10) now and every
/// [`SWEEP_EVERY`], on the runtime the router is made on, if there is one.
fn sweep_reports_hourly(db: &Db) {
    let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
    let db = db.clone();
    handle.spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        loop {
            tick.tick().await;
            match db.run(|c| db::sweep_reports(c, db::now_ms())).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(deleted = n, "reports past 30 days swept"),
                Err(e) => tracing::warn!(error = %e, "reports not swept"),
            }
        }
    });
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
    /// For a 401 at `/mcp` with OAuth on: where the protected resource
    /// metadata is (RFC 9728 §5.1), so a client can sign in.
    pub resource_metadata: Option<String>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), extra: None, retry_after: None, resource_metadata: None }
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
            let mut value = format!("Bearer realm=\"{REALM}\"");
            if let Some(url) = &self.resource_metadata {
                value.push_str(&format!(", resource_metadata=\"{url}\", scope=\"{}\"", oauth::SCOPE));
            }
            if self.code == "invalid_token" {
                value.push_str(", error=\"invalid_token\"");
            }
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

/// An agent that was let in, by a static token or an OAuth access token:
/// its id (the token's, or the grant's), its mailbox, and its key, opened
/// with its credential for this request (spec §10.6, encryption at rest),
/// once the mailbox has pushed encrypted.
#[derive(Debug, Clone)]
pub(crate) struct AgentAuth {
    pub token_id: String,
    pub mailbox_id: i64,
    pub address: String,
    pub key: Option<rules_crypto::SecretKey>,
}

/// Check an agent's token and take one request from its bucket. OAuth
/// access tokens are bound to the `/mcp` resource: they are taken only
/// where `oauth_ok` (at `/mcp`), and only for this server's resource.
pub(crate) async fn agent_auth(
    state: &AppState,
    headers: &http::HeaderMap,
    slot: Option<&TokenSlot>,
    oauth_ok: bool,
) -> Result<AgentAuth, ApiError> {
    let token = tokens::bearer(headers).ok_or_else(|| ApiError::unauthorized(false))?;
    let presented = tokens::hash(token);
    let (id, mailbox_id, address, secret) = if token.starts_with(tokens::ACCESS_PREFIX) {
        let resource = match &state.oauth {
            Some(o) if oauth_ok => o.resource.clone(),
            _ => return Err(ApiError::unauthorized(true)),
        };
        let now = db::now_ms();
        let found = state
            .db
            .run(move |c| {
                let Some(t) = db::oauth_token(c, &presented)? else { return Ok(None) };
                Ok(db::agent_token(c, &t.grant_id)?.map(|g| (t, g)))
            })
            .await?;
        let Some((t, (grant, address))) = found else { return Err(ApiError::unauthorized(true)) };
        let live = t.kind == db::TOKEN_ACCESS
            && t.expires_at > now
            && grant.kind == db::KIND_OAUTH
            && grant.revoked_at.is_none()
            && tokens::same(&t.resource, &resource);
        if !live {
            return Err(ApiError::unauthorized(true));
        }
        // A token made before grant secrets carries none: the grant gets
        // its key at its next refresh.
        let secret = tokens::grant_secret_of(token).map(crypto::CredentialSecret::new);
        (grant.id, grant.mailbox_id, address, secret)
    } else {
        let id = tokens::agent_token_id(token).ok_or_else(|| ApiError::unauthorized(true))?.to_owned();
        let found = state.db.run({
            let id = id.clone();
            move |c| db::agent_token(c, &id)
        });
        let (row, address) = found.await?.ok_or_else(|| ApiError::unauthorized(true))?;
        if row.kind != db::KIND_TOKEN || row.revoked_at.is_some() || !tokens::same(&presented, &row.token_hash) {
            return Err(ApiError::unauthorized(true));
        }
        (id, row.mailbox_id, address, Some(crypto::CredentialSecret::new(token)))
    };
    if let Some(slot) = slot {
        slot.set(format!("agent:{id}"));
    }
    state.limiter.take(&format!("agent:{id}")).map_err(ApiError::too_many)?;
    // For the app's list ("last used"); a failure here refuses nobody.
    let used = id.clone();
    if let Err(e) = state.db.run(move |c| db::touch_agent(c, &used, db::now_ms())).await {
        tracing::warn!(token = %id, "could not note the agent's use: {e}");
    }
    let key = match secret {
        Some(secret) => {
            let agent = id.clone();
            state.db.run(move |c| crypto::agent_key(c, &agent, mailbox_id, &secret)).await?
        }
        None => None,
    };
    Ok(AgentAuth { token_id: id, mailbox_id, address, key })
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
