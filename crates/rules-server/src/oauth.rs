//! The rules server as its own minimal OAuth 2.1 authorization server
//! (spec §10.6, plan `docs/plans/rules-server.md` step 5), for clients that
//! take only a URL: a claude.ai custom connector, which a cloud routine
//! uses, or Claude Code without a header.
//!
//! There are no user accounts and no passwords. The app mints a one-time
//! **connect code** for one agent mailbox (`POST
//! /v1/mailboxes/{address}/connect-codes`, publisher token); the consent
//! page asks for it. Redeeming it makes an **agent grant**: a row beside the
//! static agent tokens (`agent_tokens`, `kind` `oauth`), named as the app
//! named the code, listed and revoked like a token. Access and refresh
//! tokens belong to the grant; revoking it ends them at their next use.
//!
//! - `GET /.well-known/oauth-protected-resource` (RFC 9728), also under
//!   `/mcp`: the resource is `<public URL>/mcp`, the issuer the public URL.
//! - `GET /.well-known/oauth-authorization-server` (RFC 8414).
//! - `POST /oauth/register` (RFC 7591): public clients only
//!   (`token_endpoint_auth_method` `none`); redirect URIs are `https://`, or
//!   `http://` to a loopback address (RFC 8252), matched exactly except for
//!   a loopback port.
//! - `GET` and `POST /oauth/authorize`: the authorization code flow with
//!   PKCE S256 only; the consent page shows the client's name and where it
//!   goes back to and asks for the connect code. Its only cookie carries the
//!   CSRF token. `state` is passed through; `resource` (RFC 8707) must be
//!   this server's.
//! - `POST /oauth/token`: the code for an access token (an hour) and a
//!   refresh token (30 days), with the PKCE verifier; refresh tokens rotate,
//!   and one used twice revokes the grant, as does an authorization code
//!   used twice.
//!
//! Every code and token is random, kept only as its SHA-256 and compared
//! in constant time. Nothing here logs a code, token or verifier.

use std::collections::HashMap;
use std::sync::Mutex;

use axum::extract::{Extension, RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use serde_json::{Value, json};
use url::Url;

use crate::db::{self, AgentTokenRow, AuthCodeRow, ClientRow, OAuthTokenRow, now_ms};
use crate::limit::RateLimiter;
use crate::{AppState, TokenSlot, tokens};

/// The one scope: read the mailbox's published guide and shared facts.
pub const SCOPE: &str = "rules";
/// How long an access token works.
pub const ACCESS_TTL_MS: i64 = 3_600_000;
/// How long a refresh token works (each refresh makes a new one).
pub const REFRESH_TTL_MS: i64 = 30 * 24 * 3_600_000;
/// How long an authorization code may wait to be exchanged.
pub const AUTH_CODE_TTL_MS: i64 = 120_000;
/// How long a connect code works.
pub const CONNECT_CODE_TTL_MS: i64 = 600_000;
/// How long a consent page may stay open.
pub const CONSENT_TTL_MS: i64 = 600_000;
/// Wrong connect codes on one consent page before it closes.
pub const TRIES_PER_CONSENT: u32 = 5;
/// Wrong connect codes from one client before it is locked out…
pub const TRIES_PER_CLIENT: u32 = 5;
/// …for this long.
pub const CLIENT_LOCK_MS: i64 = 3_600_000;
/// Connect codes checked a minute across the whole server, right or
/// wrong: a ceiling on guessing.
pub const WRONG_CODES_PER_MINUTE: u32 = 30;
/// Unused connect codes a mailbox may have at once.
pub const LIVE_CODES_PER_MAILBOX: i64 = 10;
/// Registrations a minute across the whole server.
const REGISTRATIONS_PER_MINUTE: u32 = 30;
/// Consent pages opened a minute across the whole server.
const CONSENTS_PER_MINUTE: u32 = 120;
const MAX_PENDING: usize = 10_000;
const MAX_REDIRECT_URIS: usize = 10;
const MAX_CLIENT_NAME: usize = 100;

/// The server's OAuth side: its addresses, and the consent pages open now.
pub struct OAuth {
    /// `https://rules.example.com`: the issuer.
    pub issuer: String,
    /// `https://rules.example.com/mcp`: the one resource tokens are for.
    pub resource: String,
    secure_cookie: bool,
    pending: Mutex<HashMap<String, Pending>>,
    failures: Mutex<HashMap<String, ClientFailures>>,
    wrong_codes: RateLimiter,
    registrations: RateLimiter,
    consents: RateLimiter,
}

/// A consent page waiting for its connect code.
#[derive(Clone)]
struct Pending {
    client_id: String,
    client_name: String,
    redirect_uri: String,
    state: Option<String>,
    challenge: String,
    csrf_hash: String,
    created_at: i64,
    tries: u32,
}

#[derive(Default)]
struct ClientFailures {
    count: u32,
    since: i64,
    locked_until: Option<i64>,
}

/// The public URL, checked: `https://host[:port]`, or `http://` to a
/// loopback address (a server tried out on one machine). No path, query,
/// fragment or user.
pub fn public_base(input: &str) -> Result<String, String> {
    let url = Url::parse(input.trim()).map_err(|e| format!("{input:?} is not a URL: {e}"))?;
    let host = url.host_str().unwrap_or_default();
    if host.is_empty() {
        return Err(format!("{input:?} has no host"));
    }
    match url.scheme() {
        "https" => {}
        "http" if is_loopback(host) => {}
        _ => return Err(format!("{input:?} must be https:// (plain http:// only to 127.0.0.1, [::1] or localhost)")),
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
        return Err(format!(
            "{input:?} must be the server's origin alone, such as https://rules.example.com (serve it at the root \
             of a host)"
        ));
    }
    Ok(url.origin().ascii_serialization())
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "[::1]" | "::1" | "localhost")
}

impl OAuth {
    pub fn new(issuer: &str) -> Self {
        Self {
            issuer: issuer.to_owned(),
            resource: format!("{issuer}/mcp"),
            secure_cookie: issuer.starts_with("https://"),
            pending: Mutex::new(HashMap::new()),
            failures: Mutex::new(HashMap::new()),
            wrong_codes: RateLimiter::new(WRONG_CODES_PER_MINUTE),
            registrations: RateLimiter::new(REGISTRATIONS_PER_MINUTE),
            consents: RateLimiter::new(CONSENTS_PER_MINUTE),
        }
    }

    /// Where the protected resource metadata is, for `WWW-Authenticate`.
    pub fn resource_metadata_url(&self) -> String {
        format!("{}/.well-known/oauth-protected-resource", self.issuer)
    }

    fn locked(&self, client_id: &str, now: i64) -> bool {
        let failures = self.failures.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        failures.get(client_id).and_then(|f| f.locked_until).is_some_and(|until| until > now)
    }

    /// Count a wrong connect code against a client; true when that locks it.
    fn count_failure(&self, client_id: &str, now: i64) -> bool {
        let mut failures = self.failures.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if failures.len() > MAX_PENDING {
            failures.retain(|_, f| f.locked_until.is_some_and(|u| u > now) || now - f.since < CLIENT_LOCK_MS);
        }
        let f = failures.entry(client_id.to_owned()).or_default();
        if now - f.since >= CLIENT_LOCK_MS {
            *f = ClientFailures { count: 0, since: now, locked_until: None };
        }
        f.count += 1;
        if f.count >= TRIES_PER_CLIENT {
            f.locked_until = Some(now + CLIENT_LOCK_MS);
            return true;
        }
        false
    }

    fn take_pending(&self, id: &str) -> Option<Pending> {
        self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(id)
    }

    fn put_pending(&self, id: String, p: Pending) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = now_ms();
        pending.retain(|_, p| now - p.created_at < CONSENT_TTL_MS);
        if pending.len() >= MAX_PENDING {
            return false;
        }
        pending.insert(id, p);
        true
    }
}

pub(crate) fn router(state: AppState) -> Router {
    if state.oauth.is_none() {
        return Router::new();
    }
    Router::new()
        .route("/.well-known/oauth-protected-resource", get(resource_metadata))
        .route("/.well-known/oauth-protected-resource/mcp", get(resource_metadata))
        .route("/.well-known/oauth-authorization-server", get(server_metadata))
        .route("/oauth/register", post(register))
        .route("/oauth/authorize", get(authorize).post(consent))
        .route("/oauth/token", post(token))
        .with_state(state)
}

fn oauth(state: &AppState) -> &OAuth {
    // The routes exist only when it is set (above).
    state.oauth.as_ref().expect("OAuth routes are only served with a public URL")
}

/// Public metadata: anyone may read it, from anywhere.
fn public_json(body: Value) -> Response {
    let mut r = Json(body).into_response();
    r.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, HeaderValue::from_static("*"));
    r
}

async fn resource_metadata(State(state): State<AppState>) -> Response {
    let o = oauth(&state);
    public_json(json!({
        "resource": o.resource,
        "authorization_servers": [o.issuer],
        "scopes_supported": [SCOPE],
        "bearer_methods_supported": ["header"],
        "resource_name": "OpenAGC rules server",
    }))
}

async fn server_metadata(State(state): State<AppState>) -> Response {
    let o = oauth(&state);
    let at = |path: &str| format!("{}{path}", o.issuer);
    public_json(json!({
        "issuer": o.issuer,
        "authorization_endpoint": at("/oauth/authorize"),
        "token_endpoint": at("/oauth/token"),
        "registration_endpoint": at("/oauth/register"),
        "scopes_supported": [SCOPE, "offline_access"],
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["none"],
        "code_challenge_methods_supported": ["S256"],
        "authorization_response_iss_parameter_supported": true,
    }))
}

// ---- Registration (RFC 7591) ----

fn registration_error(code: &'static str, description: impl Into<String>) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": code, "error_description": description.into() }))).into_response()
}

/// A redirect URI a client may register: `https://` with a host, or
/// `http://` to a loopback address; no fragment and no user.
pub fn valid_redirect_uri(s: &str) -> bool {
    let Ok(url) = Url::parse(s) else { return false };
    let host = url.host_str().unwrap_or_default();
    let shape = !host.is_empty() && url.fragment().is_none() && url.username().is_empty() && url.password().is_none();
    shape && (url.scheme() == "https" || (url.scheme() == "http" && is_loopback(host)))
}

/// Whether `given` is `registered`: the same string, or for a loopback
/// `http://` URI the same but for the port (RFC 8252 §7.3; Claude Code
/// listens on a port of its own each time).
pub fn redirect_matches(registered: &str, given: &str) -> bool {
    if registered == given {
        return true;
    }
    let (Ok(a), Ok(b)) = (Url::parse(registered), Url::parse(given)) else { return false };
    let loopback = |u: &Url| u.scheme() == "http" && u.host_str().is_some_and(is_loopback);
    loopback(&a)
        && loopback(&b)
        && a.host_str() == b.host_str()
        && a.path() == b.path()
        && a.query() == b.query()
        && b.fragment().is_none()
        && b.username().is_empty()
        && b.password().is_none()
}

async fn register(State(state): State<AppState>, Extension(slot): Extension<TokenSlot>, body: String) -> Response {
    let o = oauth(&state);
    if let Err(wait) = o.registrations.take("register") {
        return crate::ApiError::too_many(wait).into_response();
    }
    let Ok(Value::Object(meta)) = serde_json::from_str::<Value>(&body) else {
        return registration_error("invalid_client_metadata", "the body must be a JSON object");
    };
    let uris: Vec<String> = match meta.get("redirect_uris") {
        Some(Value::Array(a)) if !a.is_empty() && a.len() <= MAX_REDIRECT_URIS => {
            match a.iter().map(|u| u.as_str().map(str::to_owned)).collect::<Option<Vec<_>>>() {
                Some(u) => u,
                None => return registration_error("invalid_redirect_uri", "redirect_uris are strings"),
            }
        }
        _ => {
            return registration_error("invalid_redirect_uri", format!("give 1 to {MAX_REDIRECT_URIS} redirect_uris"));
        }
    };
    if let Some(bad) = uris.iter().find(|u| !valid_redirect_uri(u)) {
        return registration_error(
            "invalid_redirect_uri",
            format!("{bad:?} is not https:// (or http:// to a loopback address), or has a fragment"),
        );
    }
    if let Some(Value::Array(types)) = meta.get("response_types")
        && !types.iter().any(|t| t == "code")
    {
        return registration_error("invalid_client_metadata", "only the code response type is supported");
    }
    if let Some(Value::Array(grants)) = meta.get("grant_types")
        && !grants.iter().any(|g| g == "authorization_code")
    {
        return registration_error("invalid_client_metadata", "only the authorization_code grant is supported");
    }
    // Every client is public: whatever authentication it asked for, it
    // gets none (RFC 7591 §3.2.1 lets the server say what it registered).
    let name: String = meta
        .get("client_name")
        .and_then(Value::as_str)
        .map(|n| n.chars().filter(|c| !c.is_control()).take(MAX_CLIENT_NAME).collect::<String>())
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "An unnamed app".to_owned());
    let row = ClientRow { id: tokens::client_id(), name, redirect_uris: uris, created_at: now_ms() };
    let stored = row.clone();
    let saved = state
        .db
        .run(move |c| {
            db::prune(c, stored.created_at)?;
            db::insert_client(c, &stored)
        })
        .await;
    if let Err(e) = saved {
        return crate::ApiError::from(e).into_response();
    }
    slot.set(format!("client:{}", row.id));
    tracing::info!(client = %row.id, "oauth client registered");
    let body = json!({
        "client_id": row.id,
        "client_id_issued_at": row.created_at / 1000,
        "client_name": row.name,
        "redirect_uris": row.redirect_uris,
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "scope": SCOPE,
    });
    let mut r = (StatusCode::CREATED, Json(body)).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

// ---- Parameters ----

/// Form or query parameters, each at most once (RFC 6749 §3.1).
fn params(raw: &str) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    for (k, v) in url::form_urlencoded::parse(raw.as_bytes()).into_owned() {
        if out.contains_key(&k) {
            return Err(format!("{k} is given more than once"));
        }
        out.insert(k, v);
    }
    Ok(out)
}

/// `resource` (RFC 8707) names this server's `/mcp`: scheme and host in
/// any case, a trailing slash or not.
fn same_resource(given: &str, ours: &str) -> bool {
    let Ok(url) = Url::parse(given) else { return false };
    if url.fragment().is_some() || url.query().is_some() {
        return false;
    }
    let normal = format!("{}{}", url.origin().ascii_serialization(), url.path().trim_end_matches('/'));
    normal == ours
}

// ---- The consent page ----

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

const STYLE: &str = "body{font:16px/1.5 -apple-system,BlinkMacSystemFont,system-ui,sans-serif;max-width:30rem;\
margin:3rem auto;padding:0 1rem;color:#1d1d1f;background:#fff}h1{font-size:1.4rem}\
input{font:1.3rem ui-monospace,SFMono-Regular,Menlo,monospace;letter-spacing:.1em;padding:.4rem .6rem;\
width:100%;box-sizing:border-box;border:1px solid #8e8e93;border-radius:6px;text-transform:uppercase}\
button{font:inherit;padding:.4rem 1rem;margin:1rem .5rem 0 0;border-radius:6px;border:1px solid #8e8e93}\
.go{background:#0a66d8;color:#fff;border-color:#0a66d8}.note{color:#6e6e73;font-size:.9rem}\
.warn{background:#fff4e5;border-radius:6px;padding:.5rem .75rem}.err{color:#c4161c}\
@media(prefers-color-scheme:dark){body{color:#f5f5f7;background:#1d1d1f}input{background:#2c2c2e;color:#f5f5f7}\
.warn{background:#3a2a10}.err{color:#ff6961}.note{color:#a1a1a6}}";

/// An HTML page that cannot be framed, runs no script, loads nothing and
/// posts its form only here, and to `goes_to` (the origin the answer
/// redirects to, which `form-action` covers in some browsers).
fn page(status: StatusCode, title: &str, body: &str, goes_to: Option<&str>) -> Response {
    let nonce = tokens::secret();
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" \
         content=\"width=device-width, initial-scale=1\"><title>{title}</title><style \
         nonce=\"{nonce}\">{STYLE}</style></head><body>{body}</body></html>",
        title = escape(title),
    );
    let form_action = match goes_to {
        Some(origin) => format!("'self' {origin}"),
        None => "'none'".to_owned(),
    };
    let csp = format!(
        "default-src 'none'; style-src 'nonce-{nonce}'; form-action {form_action}; frame-ancestors 'none'; \
         base-uri 'none'"
    );
    let mut r = (status, html).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    if let Ok(v) = HeaderValue::from_str(&csp) {
        h.insert(header::CONTENT_SECURITY_POLICY, v);
    }
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("same-origin"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    r
}

/// A page saying what went wrong, for when there is nowhere safe to send
/// the browser back to.
fn error_page(status: StatusCode, message: &str) -> Response {
    let body = format!(
        "<h1>Cannot connect</h1><p>{}</p><p class=\"note\">Start again from the app you were connecting.</p>",
        escape(message)
    );
    page(status, "Cannot connect", &body, None)
}

fn origin_of(uri: &str) -> Option<String> {
    Url::parse(uri).ok().map(|u| u.origin().ascii_serialization())
}

fn cookie_name(request: &str) -> String {
    format!("oagc_consent_{}", &request[..request.len().min(16)])
}

fn consent_page(o: &OAuth, request: &str, p: &Pending, error: Option<&str>) -> Response {
    let host = Url::parse(&p.redirect_uri).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
    let loopback = if is_loopback(&host) {
        "<p class=\"warn\">It goes back to a program on this computer. Continue only if you started this from \
         an app you trust, such as Claude Code.</p>"
    } else {
        ""
    };
    let error = error.map(|e| format!("<p class=\"err\" role=\"alert\">{}</p>", escape(e))).unwrap_or_default();
    let body = format!(
        "<h1>Connect an agent</h1>\
         <p>An app that calls itself <strong>{name}</strong> asks to read an agent mailbox's writing guide and the \
         facts you shared with cloud agents, on this rules server ({issuer}). It cannot read or send mail.</p>\
         <p>When you continue, you go back to <strong>{host}</strong>.</p>{loopback}\
         <form method=\"post\" action=\"/oauth/authorize\">\
         <input type=\"hidden\" name=\"request\" value=\"{request}\">\
         <p><label for=\"code\">Connect code from OpenAGC</label></p>\
         <input id=\"code\" name=\"connect_code\" autocomplete=\"one-time-code\" autocapitalize=\"characters\" \
         spellcheck=\"false\" maxlength=\"20\" placeholder=\"ABCDE-FGHJK\" autofocus>{error}\
         <button class=\"go\" name=\"action\" value=\"allow\">Connect</button>\
         <button name=\"action\" value=\"deny\">Cancel</button></form>\
         <p class=\"note\">In OpenAGC, open the agent mailbox's settings and choose Connect a Cloud Agent… to get \
         a code. It works once, for 10 minutes.</p>",
        name = escape(&p.client_name),
        issuer = escape(&o.issuer),
        host = escape(&host),
        request = escape(request),
    );
    page(StatusCode::OK, "Connect an agent", &body, origin_of(&p.redirect_uri).as_deref())
}

/// Send the browser back to the client with these parameters, plus
/// `state` and `iss` (RFC 9207).
fn back_to(o: &OAuth, redirect_uri: &str, state: Option<&str>, pairs: &[(&str, &str)], status: StatusCode) -> Response {
    let Ok(mut url) = Url::parse(redirect_uri) else {
        return error_page(StatusCode::BAD_REQUEST, "the app's return address is not a URL");
    };
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in pairs {
            q.append_pair(k, v);
        }
        if let Some(s) = state {
            q.append_pair("state", s);
        }
        q.append_pair("iss", &o.issuer);
    }
    let mut r = status.into_response();
    let h = r.headers_mut();
    if let Ok(v) = HeaderValue::from_str(url.as_str()) {
        h.insert(header::LOCATION, v);
    }
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    r
}

/// Send the browser back with an error: 302 from the request, 303 from
/// the form (so the browser follows it with a GET).
fn back_with_error(o: &OAuth, redirect_uri: &str, state: Option<&str>, code: &str, why: &str) -> Response {
    back_to(o, redirect_uri, state, &[("error", code), ("error_description", why)], StatusCode::FOUND)
}

fn form_back_with_error(o: &OAuth, p: &Pending, code: &str, why: &str) -> Response {
    back_to(
        o,
        &p.redirect_uri,
        p.state.as_deref(),
        &[("error", code), ("error_description", why)],
        StatusCode::SEE_OTHER,
    )
}

/// `GET /oauth/authorize`: check the request and show the consent page.
async fn authorize(
    State(state): State<AppState>,
    Extension(slot): Extension<TokenSlot>,
    RawQuery(query): RawQuery,
) -> Response {
    let o = oauth(&state);
    if o.consents.take("authorize").is_err() {
        return error_page(StatusCode::TOO_MANY_REQUESTS, "This server is busy; try again in a minute.");
    }
    let q = match params(query.as_deref().unwrap_or_default()) {
        Ok(q) => q,
        Err(e) => return error_page(StatusCode::BAD_REQUEST, &e),
    };
    // Until the client and its return address check out, errors are shown
    // here, never redirected (RFC 6749 §4.1.2.1).
    let Some(client_id) = q.get("client_id").cloned() else {
        return error_page(StatusCode::BAD_REQUEST, "The app did not say who it is (client_id).");
    };
    let lookup = client_id.clone();
    let client = match state.db.run(move |c| db::client(c, &lookup)).await {
        Ok(Some(c)) => c,
        Ok(None) => return error_page(StatusCode::BAD_REQUEST, "This app is not registered here (unknown client_id)."),
        Err(e) => return crate::ApiError::from(e).into_response(),
    };
    slot.set(format!("client:{}", client.id));
    let redirect_uri = match q.get("redirect_uri") {
        Some(given) if client.redirect_uris.iter().any(|r| redirect_matches(r, given)) => given.clone(),
        Some(_) => {
            return error_page(StatusCode::BAD_REQUEST, "The app's return address is not one it registered.");
        }
        None if client.redirect_uris.len() == 1 => client.redirect_uris[0].clone(),
        None => return error_page(StatusCode::BAD_REQUEST, "The app did not say where to return (redirect_uri)."),
    };
    let st = q.get("state").map(String::as_str);
    let fail = |code: &str, why: &str| back_with_error(o, &redirect_uri, st, code, why);
    if q.get("response_type").map(String::as_str) != Some("code") {
        return fail("unsupported_response_type", "only response_type=code is supported");
    }
    let challenge = match (q.get("code_challenge"), q.get("code_challenge_method").map(String::as_str)) {
        (Some(c), Some("S256")) if tokens::valid_challenge(c) => c.clone(),
        (Some(_), Some("S256")) => return fail("invalid_request", "code_challenge is not an S256 challenge"),
        _ => return fail("invalid_request", "PKCE is required: send code_challenge with code_challenge_method=S256"),
    };
    if let Some(r) = q.get("resource")
        && !same_resource(r, &o.resource)
    {
        return fail("invalid_target", "this server issues tokens only for its own /mcp");
    }
    let now = now_ms();
    if o.locked(&client.id, now) {
        return fail("access_denied", "too many wrong connect codes from this app; try again in an hour");
    }
    let request = tokens::secret();
    let csrf = tokens::secret();
    let pending = Pending {
        client_id: client.id.clone(),
        client_name: client.name.clone(),
        redirect_uri,
        state: q.get("state").cloned(),
        challenge,
        csrf_hash: tokens::hash(&csrf),
        created_at: now,
        tries: 0,
    };
    let mut r = consent_page(o, &request, &pending, None);
    if !o.put_pending(request.clone(), pending) {
        return error_page(StatusCode::SERVICE_UNAVAILABLE, "This server is busy; try again in a few minutes.");
    }
    let secure = if o.secure_cookie { "; Secure" } else { "" };
    let cookie = format!(
        "{}={csrf}; Path=/oauth/authorize; HttpOnly; SameSite=Lax; Max-Age={}{secure}",
        cookie_name(&request),
        CONSENT_TTL_MS / 1000
    );
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
    r
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then_some(v)
    })
}

/// `POST /oauth/authorize`: the consent form, with its connect code.
async fn consent(
    State(state): State<AppState>,
    Extension(slot): Extension<TokenSlot>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let o = oauth(&state);
    let form = match params(&body) {
        Ok(f) => f,
        Err(e) => return error_page(StatusCode::BAD_REQUEST, &e),
    };
    let request = form.get("request").cloned().unwrap_or_default();
    // A form posted from another site carries neither our Origin nor the
    // cookie (SameSite=Lax); either is enough to refuse it.
    let from_here = headers.get(header::ORIGIN).is_none_or(|v| v.to_str().is_ok_and(|v| v == o.issuer));
    let Some(mut p) = o.take_pending(&request) else {
        return error_page(StatusCode::BAD_REQUEST, "This page has expired or was already used.");
    };
    let csrf_ok =
        cookie(&headers, &cookie_name(&request)).is_some_and(|c| tokens::same(&tokens::hash(c), &p.csrf_hash));
    if !from_here || !csrf_ok {
        tracing::warn!(client = %p.client_id, "consent form refused: not posted from its own page");
        return error_page(StatusCode::FORBIDDEN, "This form was not sent from its own page.");
    }
    slot.set(format!("client:{}", p.client_id));
    let now = now_ms();
    if now - p.created_at >= CONSENT_TTL_MS {
        return form_back_with_error(o, &p, "access_denied", "the consent page expired");
    }
    let clear = format!("{}=; Path=/oauth/authorize; HttpOnly; SameSite=Lax; Max-Age=0", cookie_name(&request));
    let with_clear = |mut r: Response| {
        if let Ok(v) = HeaderValue::from_str(&clear) {
            r.headers_mut().append(header::SET_COOKIE, v);
        }
        r
    };
    if form.get("action").map(String::as_str) == Some("deny") {
        return with_clear(form_back_with_error(o, &p, "access_denied", "the user said no"));
    }
    if o.locked(&p.client_id, now) {
        return with_clear(form_back_with_error(
            o,
            &p,
            "access_denied",
            "too many wrong connect codes from this app; try again in an hour",
        ));
    }
    if o.wrong_codes.take("check").is_err() {
        // Not counted against anyone: the server as a whole is being tried.
        let r = consent_page(
            o,
            &request,
            &p,
            Some("Too many codes were tried on this server just now. Wait a minute and try again."),
        );
        o.put_pending(request, p);
        return r;
    }
    let typed = form.get("connect_code").map(String::as_str).unwrap_or_default();
    let redeemed = match tokens::normalize_connect_code(typed) {
        Some(code) => {
            let code_hash = tokens::hash(&code);
            let auth_code = tokens::secret();
            let row = AuthCodeRow {
                grant_id: tokens::new_id(),
                client_id: p.client_id.clone(),
                redirect_uri: p.redirect_uri.clone(),
                code_challenge: p.challenge.clone(),
                resource: o.resource.clone(),
                expires_at: now + AUTH_CODE_TTL_MS,
                used_at: None,
            };
            let auth_hash = tokens::hash(&auth_code);
            let made = state
                .db
                .run(move |c| {
                    let tx = c.transaction()?;
                    let Some((mailbox_id, name)) = db::redeem_connect_code(&tx, &code_hash, now)? else {
                        return Ok(None);
                    };
                    db::insert_agent_token(
                        &tx,
                        &AgentTokenRow {
                            id: row.grant_id.clone(),
                            mailbox_id,
                            name,
                            token_hash: String::new(),
                            created_at: now,
                            revoked_at: None,
                            kind: db::KIND_OAUTH.into(),
                            client_id: Some(row.client_id.clone()),
                        },
                    )?;
                    db::insert_auth_code(&tx, &auth_hash, &row)?;
                    tx.commit()?;
                    Ok(Some((mailbox_id, row.grant_id)))
                })
                .await;
            match made {
                Ok(m) => m.map(|m| (m, auth_code)),
                Err(e) => return crate::ApiError::from(e).into_response(),
            }
        }
        None => None,
    };
    let Some(((mailbox_id, grant_id), auth_code)) = redeemed else {
        p.tries += 1;
        let locked = o.count_failure(&p.client_id, now);
        tracing::info!(client = %p.client_id, tries = p.tries, locked, "wrong connect code");
        if locked || p.tries >= TRIES_PER_CONSENT {
            return with_clear(form_back_with_error(o, &p, "access_denied", "too many wrong connect codes"));
        }
        let left = TRIES_PER_CONSENT - p.tries;
        let message = format!(
            "That code is wrong, used or expired. Check it in OpenAGC, or make a new one. {left} {} left.",
            if left == 1 { "try" } else { "tries" }
        );
        let r = consent_page(o, &request, &p, Some(&message));
        o.put_pending(request, p);
        return r;
    };
    tracing::info!(mailbox = mailbox_id, grant = %grant_id, client = %p.client_id, "agent connected with a connect code");
    with_clear(back_to(o, &p.redirect_uri, p.state.as_deref(), &[("code", &auth_code)], StatusCode::SEE_OTHER))
}

// ---- Tokens ----

fn token_error(status: StatusCode, code: &'static str, description: &str) -> Response {
    let mut r = (status, Json(json!({ "error": code, "error_description": description }))).into_response();
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    if status == StatusCode::UNAUTHORIZED {
        h.insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"openagc-rules\""));
    }
    r
}

fn invalid_grant(description: &str) -> Response {
    token_error(StatusCode::BAD_REQUEST, "invalid_grant", description)
}

/// The client id of a token request: in the form, or as HTTP Basic's user
/// (with an empty password) for clients that send it there.
fn client_of(form: &HashMap<String, String>, headers: &HeaderMap) -> Option<String> {
    if let Some(id) = form.get("client_id") {
        return Some(id.clone());
    }
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, b64) = value.trim().split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (user, _) = decoded.split_once(':')?;
    url::form_urlencoded::parse(format!("u={user}").as_bytes()).next().map(|(_, v)| v.into_owned())
}

/// What a token request came to, decided in one transaction.
enum Exchange {
    Issued {
        grant_id: String,
        access: String,
        refresh: String,
    },
    Refused(&'static str),
    /// A code or refresh token used twice: the grant is revoked.
    Replayed(String),
}

/// Store a new access and refresh token for a grant, both carrying the
/// grant's secret, and make the grant's key if it has none (spec §10.6,
/// encryption at rest): this is the one moment the server holds the
/// secret without a request from the agent.
fn issue(
    c: &rusqlite::Connection,
    grant_id: &str,
    resource: &str,
    now: i64,
    secret: &str,
) -> rusqlite::Result<(String, String)> {
    let access = tokens::access_token(secret);
    let refresh = tokens::refresh_token(secret);
    if let Some((grant, _)) = db::agent_token(c, grant_id)? {
        crate::crypto::agent_key(c, grant_id, grant.mailbox_id, &crate::crypto::CredentialSecret::new(secret))?;
    }
    let row = |kind: &str, ttl| OAuthTokenRow {
        kind: kind.to_owned(),
        grant_id: grant_id.to_owned(),
        resource: resource.to_owned(),
        expires_at: now + ttl,
        used_at: None,
    };
    db::insert_oauth_token(c, &tokens::hash(&access), &row(db::TOKEN_ACCESS, ACCESS_TTL_MS), now)?;
    db::insert_oauth_token(c, &tokens::hash(&refresh), &row(db::TOKEN_REFRESH, REFRESH_TTL_MS), now)?;
    Ok((access, refresh))
}

/// The grant behind a code or token, if it is live and the client's.
fn live_grant(c: &rusqlite::Connection, grant_id: &str, client_id: &str) -> rusqlite::Result<bool> {
    Ok(db::agent_token(c, grant_id)?.is_some_and(|(g, _)| {
        g.kind == db::KIND_OAUTH && g.revoked_at.is_none() && g.client_id.as_deref() == Some(client_id)
    }))
}

async fn token(
    State(state): State<AppState>,
    Extension(slot): Extension<TokenSlot>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let o = oauth(&state);
    let form = match params(&body) {
        Ok(f) => f,
        Err(e) => return token_error(StatusCode::BAD_REQUEST, "invalid_request", &e),
    };
    let Some(client_id) = client_of(&form, &headers) else {
        return token_error(StatusCode::UNAUTHORIZED, "invalid_client", "send client_id");
    };
    if let Err(wait) = state.limiter.take(&format!("client:{client_id}")) {
        return crate::ApiError::too_many(wait).into_response();
    }
    let lookup = client_id.clone();
    match state.db.run(move |c| db::client(c, &lookup)).await {
        Ok(Some(_)) => slot.set(format!("client:{client_id}")),
        Ok(None) => {
            return token_error(StatusCode::UNAUTHORIZED, "invalid_client", "this client is not registered here");
        }
        Err(e) => return crate::ApiError::from(e).into_response(),
    }
    if let Some(r) = form.get("resource")
        && !same_resource(r, &o.resource)
    {
        return token_error(
            StatusCode::BAD_REQUEST,
            "invalid_target",
            "this server issues tokens only for its own /mcp",
        );
    }
    let resource = o.resource.clone();
    let now = now_ms();
    let exchange = match form.get("grant_type").map(String::as_str) {
        Some("authorization_code") => {
            let (Some(code), Some(verifier)) = (form.get("code"), form.get("code_verifier")) else {
                return token_error(StatusCode::BAD_REQUEST, "invalid_request", "send code and code_verifier");
            };
            let code_hash = tokens::hash(code);
            let verifier = verifier.clone();
            let redirect_uri = form.get("redirect_uri").cloned();
            state
                .db
                .run(move |c| {
                    let tx = c.transaction()?;
                    let Some(row) = db::auth_code(&tx, &code_hash)? else {
                        return Ok(Exchange::Refused("unknown code"));
                    };
                    if row.used_at.is_some() {
                        db::revoke_grant(&tx, &row.grant_id, now)?;
                        tx.commit()?;
                        return Ok(Exchange::Replayed(row.grant_id));
                    }
                    // Any check that fails spends the code: it was made
                    // for one exchange.
                    db::use_auth_code(&tx, &code_hash, now)?;
                    let outcome = if row.expires_at <= now {
                        Exchange::Refused("the code expired")
                    } else if row.client_id != client_id {
                        Exchange::Refused("the code was issued to another client")
                    } else if redirect_uri.as_deref() != Some(row.redirect_uri.as_str()) {
                        Exchange::Refused("redirect_uri is not the one the code was issued for")
                    } else if !tokens::pkce_matches(&verifier, &row.code_challenge) {
                        Exchange::Refused("code_verifier does not match the code_challenge")
                    } else if !tokens::same(&row.resource, &resource) || !live_grant(&tx, &row.grant_id, &client_id)? {
                        Exchange::Refused("the grant was revoked")
                    } else {
                        // The grant's secret is made here, with its first tokens.
                        let secret = zeroize::Zeroizing::new(crate::crypto::grant_secret());
                        let (access, refresh) = issue(&tx, &row.grant_id, &resource, now, &secret)?;
                        Exchange::Issued { grant_id: row.grant_id, access, refresh }
                    };
                    tx.commit()?;
                    Ok(outcome)
                })
                .await
        }
        Some("refresh_token") => {
            let Some(refresh) = form.get("refresh_token") else {
                return token_error(StatusCode::BAD_REQUEST, "invalid_request", "send refresh_token");
            };
            let token_hash = tokens::hash(refresh);
            // Refreshing keeps the grant's secret; a token from before grant
            // secrets gets one now (and the grant a new key).
            let secret = zeroize::Zeroizing::new(
                tokens::grant_secret_of(refresh).map_or_else(crate::crypto::grant_secret, str::to_owned),
            );
            state
                .db
                .run(move |c| {
                    let tx = c.transaction()?;
                    let Some(row) = db::oauth_token(&tx, &token_hash)?.filter(|t| t.kind == db::TOKEN_REFRESH) else {
                        return Ok(Exchange::Refused("unknown refresh token"));
                    };
                    if row.used_at.is_some() {
                        // Rotated already: someone else holds a copy.
                        db::revoke_grant(&tx, &row.grant_id, now)?;
                        tx.commit()?;
                        return Ok(Exchange::Replayed(row.grant_id));
                    }
                    let outcome = if row.expires_at <= now {
                        Exchange::Refused("the refresh token expired")
                    } else if !tokens::same(&row.resource, &resource) || !live_grant(&tx, &row.grant_id, &client_id)? {
                        Exchange::Refused("the grant was revoked or is another client's")
                    } else {
                        db::use_oauth_token(&tx, &token_hash, now)?;
                        let (access, refresh) = issue(&tx, &row.grant_id, &resource, now, &secret)?;
                        Exchange::Issued { grant_id: row.grant_id, access, refresh }
                    };
                    tx.commit()?;
                    Ok(outcome)
                })
                .await
        }
        Some(_) => {
            return token_error(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                "grant_type is authorization_code or refresh_token",
            );
        }
        None => return token_error(StatusCode::BAD_REQUEST, "invalid_request", "send grant_type"),
    };
    match exchange {
        Ok(Exchange::Issued { grant_id, access, refresh }) => {
            tracing::info!(grant = %grant_id, "oauth tokens issued");
            let body = json!({
                "access_token": access,
                "token_type": "Bearer",
                "expires_in": ACCESS_TTL_MS / 1000,
                "refresh_token": refresh,
                "scope": SCOPE,
            });
            let mut r = Json(body).into_response();
            let h = r.headers_mut();
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            h.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
            r
        }
        Ok(Exchange::Refused(why)) => invalid_grant(why),
        Ok(Exchange::Replayed(grant_id)) => {
            tracing::warn!(grant = %grant_id, "a code or refresh token was used twice; the agent is revoked");
            invalid_grant("this was used already; the connection is revoked, and the user must connect again")
        }
        Err(e) => crate::ApiError::from(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_url_is_an_https_origin_or_loopback() {
        assert_eq!(public_base("https://Rules.Example.com/").unwrap(), "https://rules.example.com");
        assert_eq!(public_base("https://rules.example.com:8443").unwrap(), "https://rules.example.com:8443");
        assert_eq!(public_base("http://127.0.0.1:8787").unwrap(), "http://127.0.0.1:8787");
        for bad in
            ["rules.example.com", "http://rules.example.com", "https://x.com/rules", "https://x.com/?a", "ftp://x"]
        {
            assert!(public_base(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn redirect_uris_are_https_or_loopback_and_match_exactly_but_a_loopback_port() {
        for ok in ["https://claude.ai/api/mcp/auth_callback", "http://localhost/callback", "http://127.0.0.1:9/cb"] {
            assert!(valid_redirect_uri(ok), "{ok}");
        }
        for bad in ["http://claude.ai/cb", "https://claude.ai/cb#x", "cursor://cb", "https://u:p@x.com/cb", "/cb", ""] {
            assert!(!valid_redirect_uri(bad), "{bad}");
        }
        let claude = "https://claude.ai/api/mcp/auth_callback";
        assert!(redirect_matches(claude, claude));
        for other in [
            "https://claude.ai/api/mcp/auth_callback/",
            "https://claude.ai/api/mcp/auth_callback?x=1",
            "https://Claude.ai/api/mcp/auth_callback",
            "https://claude.ai:443/api/mcp/auth_callback",
            "https://evil.example/api/mcp/auth_callback",
        ] {
            assert!(!redirect_matches(claude, other), "{other}");
        }
        assert!(redirect_matches("http://localhost/callback", "http://localhost:3118/callback"));
        assert!(redirect_matches("http://127.0.0.1/callback", "http://127.0.0.1:50000/callback"));
        assert!(!redirect_matches("http://127.0.0.1/callback", "http://localhost:50000/callback"));
        assert!(!redirect_matches("http://127.0.0.1/callback", "http://127.0.0.1:50000/other"));
    }

    #[test]
    fn resources_compare_by_origin_and_path() {
        let ours = "https://rules.example.com/mcp";
        for same in [ours, "https://RULES.example.com/mcp/", "HTTPS://rules.example.com/mcp"] {
            assert!(same_resource(same, ours), "{same}");
        }
        for other in ["https://rules.example.com", "https://rules.example.com/mcp#a", "https://other.com/mcp", "x"] {
            assert!(!same_resource(other, ours), "{other}");
        }
    }

    #[test]
    fn everything_shown_is_escaped() {
        assert_eq!(
            escape("<script>\"a\" & 'b'</script>"),
            "&lt;script&gt;&quot;a&quot; &amp; &#39;b&#39;&lt;/script&gt;"
        );
    }
}
