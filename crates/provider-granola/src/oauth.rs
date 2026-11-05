//! Signing in to Granola's MCP server (MCP authorization, OAuth 2.1): the
//! server's protected-resource metadata names its authorization server;
//! Kaluta registers itself there as a public client (RFC 7591) for a
//! loopback redirect, sends the user's browser to authorize with PKCE
//! (S256), and exchanges the code; the refresh token keeps it signed in.
//! The tokens go to the Keychain as [`Tokens`]' JSON; they are never
//! logged.

use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use provider_api::{ProviderError, ProviderResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// What Kaluta asks for: who the user is, and to stay signed in.
const SCOPE: &str = "openid email offline_access";

/// A signed-in connection: the client Kaluta registered, where to refresh,
/// and the tokens. Stored whole in the Keychain.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub client_id: String,
    pub token_endpoint: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Milliseconds since the epoch; 0 when unknown.
    #[serde(default)]
    pub expires_at: i64,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens").field("client_id", &self.client_id).finish_non_exhaustive()
    }
}

impl Tokens {
    /// Expired, or within a minute of it.
    pub fn stale(&self, now_ms: i64) -> bool {
        self.expires_at > 0 && self.expires_at - 60_000 <= now_ms
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[derive(Deserialize)]
struct ResourceMetadata {
    #[serde(default)]
    authorization_servers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Endpoints {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
}

fn network(e: reqwest::Error) -> ProviderError {
    ProviderError::Network(e.to_string())
}

/// `https://host/path` → `https://host`.
fn origin(url: &str) -> ProviderResult<String> {
    let u = url::Url::parse(url).map_err(|e| ProviderError::Invalid(e.to_string()))?;
    Ok(u.origin().ascii_serialization())
}

async fn get_json<T: for<'de> Deserialize<'de>>(http: &reqwest::Client, url: &str) -> ProviderResult<T> {
    let r = http.get(url).send().await.map_err(network)?;
    if !r.status().is_success() {
        return Err(ProviderError::Invalid(format!("{url} answered {}", r.status())));
    }
    r.json().await.map_err(|e| ProviderError::Decode(e.to_string()))
}

/// The authorization server for the MCP server at `mcp_url`.
pub async fn discover(http: &reqwest::Client, mcp_url: &str) -> ProviderResult<Endpoints> {
    let resource: ResourceMetadata =
        get_json(http, &format!("{}/.well-known/oauth-protected-resource", origin(mcp_url)?)).await?;
    let server = resource
        .authorization_servers
        .first()
        .ok_or_else(|| ProviderError::Invalid("Granola named no authorization server".into()))?;
    get_json(http, &format!("{}/.well-known/oauth-authorization-server", server.trim_end_matches('/'))).await
}

/// Register Kaluta as a public client for `redirect_uri`; its client id.
async fn register(http: &reqwest::Client, endpoint: &str, redirect_uri: &str) -> ProviderResult<String> {
    let body = json!({
        "client_name": "Kaluta",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "scope": SCOPE,
    });
    let r = http.post(endpoint).json(&body).send().await.map_err(network)?;
    if !r.status().is_success() {
        let status = r.status();
        let text = r.text().await.unwrap_or_default();
        return Err(ProviderError::Invalid(format!("Granola refused to register Kaluta ({status}): {text}")));
    }
    let v: Value = r.json().await.map_err(|e| ProviderError::Decode(e.to_string()))?;
    v.get("client_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ProviderError::Invalid("Granola's registration came back without a client id".into()))
}

fn random_token(bytes: usize) -> ProviderResult<String> {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).map_err(|e| ProviderError::Invalid(format!("no secure randomness: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// A sign-in in progress: the browser is to open `url`; the loopback
/// listener waits for its redirect.
pub struct Pending {
    pub url: String,
    redirect_uri: String,
    verifier: String,
    state: String,
    client_id: String,
    token_endpoint: String,
    resource: String,
    listener: TcpListener,
}

/// Start signing in: bind the loopback listener, find Granola's
/// authorization server, register, and build the authorization URL.
pub async fn begin(http: &reqwest::Client, mcp_url: &str) -> ProviderResult<Pending> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| ProviderError::Network(e.to_string()))?;
    let port = listener.local_addr().map_err(|e| ProviderError::Network(e.to_string()))?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let endpoints = discover(http, mcp_url).await?;
    let registration = endpoints
        .registration_endpoint
        .as_deref()
        .ok_or_else(|| ProviderError::Invalid("Granola does not take new clients".into()))?;
    let client_id = register(http, registration, &redirect_uri).await?;
    let (verifier, state) = (random_token(48)?, random_token(24)?);
    let mut url =
        url::Url::parse(&endpoints.authorization_endpoint).map_err(|e| ProviderError::Invalid(e.to_string()))?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", &code_challenge(&verifier))
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", &state)
        .append_pair("resource", mcp_url);
    Ok(Pending {
        url: url.into(),
        redirect_uri,
        verifier,
        state,
        client_id,
        token_endpoint: endpoints.token_endpoint,
        resource: mcp_url.to_owned(),
        listener,
    })
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Kaluta</title>\
<body style=\"font:15px -apple-system,system-ui;margin:4em;text-align:center\">\
<h2>Granola is connected</h2><p>You can close this window and return to Kaluta.</p></body>";

const FAILED_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Kaluta</title>\
<body style=\"font:15px -apple-system,system-ui;margin:4em;text-align:center\">\
<h2>Connecting Granola didn't complete</h2><p>Return to Kaluta and try again.</p></body>";

async fn respond(stream: &mut tokio::net::TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

#[derive(Deserialize)]
struct TokenAnswer {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

async fn token_request(http: &reqwest::Client, endpoint: &str, form: &[(&str, &str)]) -> ProviderResult<TokenAnswer> {
    let r = http.post(endpoint).form(form).send().await.map_err(network)?;
    let status = r.status();
    if status.as_u16() == 400 || status.as_u16() == 401 {
        let text = r.text().await.unwrap_or_default();
        return Err(if text.contains("invalid_grant") || status.as_u16() == 401 {
            ProviderError::Unauthorized
        } else {
            ProviderError::Invalid(format!("Granola refused the sign-in: {text}"))
        });
    }
    if !status.is_success() {
        return Err(ProviderError::Server { status: status.as_u16(), message: r.text().await.unwrap_or_default() });
    }
    r.json().await.map_err(|e| ProviderError::Decode(e.to_string()))
}

impl Pending {
    /// Wait for the browser's redirect, then exchange the code. Requests
    /// that are not the callback are answered and ignored; one with the
    /// wrong `state` ends the sign-in.
    pub async fn finish(self, http: &reqwest::Client, timeout: Duration) -> ProviderResult<Tokens> {
        let deadline = Instant::now() + timeout;
        let code = loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let accepted = tokio::time::timeout(remaining, self.listener.accept())
                .await
                .map_err(|_| ProviderError::Invalid("connecting Granola timed out".into()))?;
            let (mut stream, _) = accepted.map_err(|e| ProviderError::Network(e.to_string()))?;
            let mut buf = vec![0u8; 8192];
            let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let target = request.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
            let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
                respond(&mut stream, 400, FAILED_PAGE).await;
                continue;
            };
            if url.path() != "/callback" {
                respond(&mut stream, 404, "").await;
                continue;
            }
            let param = |name: &str| url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned());
            if param("state").as_deref() != Some(self.state.as_str()) {
                respond(&mut stream, 400, FAILED_PAGE).await;
                return Err(ProviderError::Invalid("Granola's answer did not match this sign-in".into()));
            }
            if let Some(error) = param("error") {
                respond(&mut stream, 200, FAILED_PAGE).await;
                return Err(if error == "access_denied" {
                    ProviderError::Forbidden("access to Granola was not granted".into())
                } else {
                    ProviderError::Invalid(format!("Granola's sign-in failed: {error}"))
                });
            }
            let Some(code) = param("code") else {
                respond(&mut stream, 400, FAILED_PAGE).await;
                return Err(ProviderError::Invalid("Granola's answer had no code".into()));
            };
            respond(&mut stream, 200, DONE_PAGE).await;
            break code;
        };
        let form = [
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("client_id", self.client_id.as_str()),
            ("code_verifier", self.verifier.as_str()),
            ("resource", self.resource.as_str()),
        ];
        let answer = token_request(http, &self.token_endpoint, &form).await?;
        Ok(Tokens {
            client_id: self.client_id,
            token_endpoint: self.token_endpoint,
            access_token: answer.access_token,
            refresh_token: answer.refresh_token,
            expires_at: answer.expires_in.map_or(0, |s| now_ms() + s * 1000),
        })
    }
}

/// New tokens for `tokens`; the refresh token is kept when Granola does
/// not rotate it. `Unauthorized` when the user must connect again.
pub async fn refresh(http: &reqwest::Client, tokens: &Tokens, resource: &str) -> ProviderResult<Tokens> {
    let Some(refresh_token) = tokens.refresh_token.as_deref() else {
        return Err(ProviderError::Unauthorized);
    };
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", tokens.client_id.as_str()),
        ("resource", resource),
    ];
    let answer = token_request(http, &tokens.token_endpoint, &form).await?;
    Ok(Tokens {
        client_id: tokens.client_id.clone(),
        token_endpoint: tokens.token_endpoint.clone(),
        access_token: answer.access_token,
        refresh_token: answer.refresh_token.or_else(|| tokens.refresh_token.clone()),
        expires_at: answer.expires_in.map_or(0, |s| now_ms() + s * 1000),
    })
}
