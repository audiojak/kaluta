//! OAuth with one-time connect codes (spec §10.6), end to end: an
//! in-process server on loopback and a test client doing what a claude.ai
//! custom connector does: discovery from a 401, dynamic registration, the
//! consent page with a connect code from the app, the code for tokens with
//! PKCE, MCP with the access token, refresh with rotation, and revocation.

mod common;

use base64::Engine;
use common::{MAILBOX, Server, call, snapshot, start, start_with};
use reqwest::StatusCode;
use rmcp::ServiceExt;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const CLAUDE_CALLBACK: &str = "https://claude.ai/api/mcp/auth_callback";

/// A browser and client in one: never follows redirects, so each step can
/// be looked at.
fn http() -> reqwest::Client {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap()
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

fn query(url: &str) -> std::collections::HashMap<String, String> {
    url::Url::parse(url).unwrap().query_pairs().into_owned().collect()
}

/// What a test client keeps between steps.
struct Client {
    id: String,
    redirect: String,
    verifier: String,
}

impl Client {
    fn challenge(&self) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()))
    }
}

fn verifier() -> String {
    format!("{}{}", rules_server::tokens::secret(), rules_server::tokens::secret())
}

struct Setup {
    s: Server,
    publisher: String,
    web: reqwest::Client,
}

async fn setup() -> Setup {
    let s = start(0, None).await;
    let publisher = s.registered().await;
    let (status, _) = s.publish(&publisher, None, snapshot(1).to_json().unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    Setup { s, publisher, web: http() }
}

impl Setup {
    /// The app mints a connect code for an agent it names.
    async fn connect_code(&self, name: &str) -> String {
        let r = self
            .s
            .http
            .post(self.s.url(&format!("/v1/mailboxes/{MAILBOX}/connect-codes")))
            .bearer_auth(&self.publisher)
            .json(&json!({ "name": name }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CREATED);
        let body: Value = r.json().await.unwrap();
        assert_eq!(body["name"], name);
        assert!(body["expires_at"].is_string());
        body["code"].as_str().unwrap().to_owned()
    }

    async fn register(&self, name: &str, redirect: &str) -> Client {
        let r = self
            .web
            .post(self.s.url("/oauth/register"))
            .json(&json!({
                "client_name": name,
                "redirect_uris": [redirect],
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CREATED);
        let body: Value = r.json().await.unwrap();
        assert_eq!(body["token_endpoint_auth_method"], "none");
        Client {
            id: body["client_id"].as_str().unwrap().to_owned(),
            redirect: redirect.to_owned(),
            verifier: verifier(),
        }
    }

    fn authorize_url(&self, c: &Client, redirect: &str, extra: &[(&str, &str)]) -> String {
        let challenge = c.challenge();
        let resource = self.s.url("/mcp");
        let mut pairs = vec![
            ("response_type", "code"),
            ("client_id", c.id.as_str()),
            ("redirect_uri", redirect),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", "st-123"),
            ("scope", "rules offline_access"),
            ("resource", resource.as_str()),
        ];
        pairs.retain(|(k, _)| !extra.iter().any(|(e, _)| e == k));
        pairs.extend(extra.iter().filter(|(_, v)| !v.is_empty()).copied());
        format!("{}?{}", self.s.url("/oauth/authorize"), form(&pairs))
    }

    /// Open the consent page: its form's request id and CSRF cookie.
    async fn consent_page(&self, c: &Client) -> (String, String, String) {
        let r = self.web.get(self.authorize_url(c, &c.redirect, &[])).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::OK, "{:?}", r.headers().get("location"));
        let h = r.headers().clone();
        let csp = h["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("frame-ancestors 'none'") && csp.contains("default-src 'none'"), "{csp}");
        assert_eq!(h["x-frame-options"], "DENY");
        assert_eq!(h["cache-control"], "no-store");
        let cookie = h["set-cookie"].to_str().unwrap();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Lax"), "{cookie}");
        let cookie = cookie.split(';').next().unwrap().to_owned();
        let html = r.text().await.unwrap();
        let start = html.find("name=\"request\" value=\"").unwrap() + "name=\"request\" value=\"".len();
        let request = html[start..].split('"').next().unwrap().to_owned();
        (request, cookie, html)
    }

    async fn submit(&self, request: &str, cookie: &str, code: &str) -> reqwest::Response {
        self.web
            .post(self.s.url("/oauth/authorize"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Cookie", cookie)
            .header("Origin", &self.s.base)
            .body(form(&[("request", request), ("connect_code", code), ("action", "allow")]))
            .send()
            .await
            .unwrap()
    }

    /// Consent with `code`: the authorization code it redirects with.
    async fn authorize(&self, c: &Client, code: &str) -> String {
        let (request, cookie, _) = self.consent_page(c).await;
        let r = self.submit(&request, &cookie, code).await;
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        let location = r.headers()["location"].to_str().unwrap().to_owned();
        assert!(location.starts_with(&format!("{}?", c.redirect)), "{location}");
        let q = query(&location);
        assert_eq!(q["state"], "st-123");
        assert_eq!(q["iss"], self.s.base);
        q["code"].clone()
    }

    async fn token(&self, pairs: &[(&str, &str)]) -> (StatusCode, Value) {
        let r = self
            .web
            .post(self.s.url("/oauth/token"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form(pairs))
            .send()
            .await
            .unwrap();
        let status = r.status();
        if status == StatusCode::OK {
            assert_eq!(r.headers()["cache-control"], "no-store");
        }
        (status, r.json().await.unwrap())
    }

    async fn exchange(&self, c: &Client, code: &str) -> (StatusCode, Value) {
        let resource = self.s.url("/mcp");
        self.token(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &c.redirect),
            ("client_id", &c.id),
            ("code_verifier", &c.verifier),
            ("resource", &resource),
        ])
        .await
    }

    async fn refresh(&self, c: &Client, refresh: &str) -> (StatusCode, Value) {
        self.token(&[("grant_type", "refresh_token"), ("refresh_token", refresh), ("client_id", &c.id)]).await
    }

    async fn mcp_status(&self, token: &str) -> StatusCode {
        self.web
            .post(self.s.url("/mcp"))
            .bearer_auth(token)
            .header("Accept", "application/json, text/event-stream")
            .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))
            .send()
            .await
            .unwrap()
            .status()
    }

    async fn agents(&self) -> Vec<Value> {
        let r = self
            .s
            .http
            .get(self.s.url(&format!("/v1/mailboxes/{MAILBOX}/agent-tokens")))
            .bearer_auth(&self.publisher)
            .send()
            .await
            .unwrap();
        let body: Value = r.json().await.unwrap();
        body["agent_tokens"].as_array().unwrap().clone()
    }

    /// Connect a client fully: its access and refresh tokens.
    async fn connected(&self, name: &str) -> (Client, String, String) {
        let c = self.register("Claude", CLAUDE_CALLBACK).await;
        let code = self.authorize(&c, &self.connect_code(name).await).await;
        let (status, tokens) = self.exchange(&c, &code).await;
        assert_eq!(status, StatusCode::OK, "{tokens}");
        let access = tokens["access_token"].as_str().unwrap().to_owned();
        let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
        (c, access, refresh)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_connector_discovers_registers_consents_with_a_connect_code_and_reads_until_revoked() {
    let t = setup().await;
    let base = t.s.base.clone();

    // Discovery from the 401, as a claude.ai connector does it.
    let r = t.web.post(t.s.url("/mcp")).json(&json!({})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let challenge = r.headers()["www-authenticate"].to_str().unwrap().to_owned();
    let metadata_url = challenge.split("resource_metadata=\"").nth(1).unwrap().split('"').next().unwrap().to_owned();
    let prm: Value = t.web.get(&metadata_url).send().await.unwrap().json().await.unwrap();
    assert_eq!(prm["resource"], format!("{base}/mcp"));
    assert_eq!(prm["authorization_servers"], json!([base]));
    let inserted: Value =
        t.web.get(t.s.url("/.well-known/oauth-protected-resource/mcp")).send().await.unwrap().json().await.unwrap();
    assert_eq!(inserted, prm, "also at the path-inserted address");
    let issuer = prm["authorization_servers"][0].as_str().unwrap();
    let asm: Value = t
        .web
        .get(format!("{issuer}/.well-known/oauth-authorization-server"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(asm["issuer"], base);
    assert_eq!(asm["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(asm["token_endpoint_auth_methods_supported"], json!(["none"]));
    assert_eq!(asm["registration_endpoint"], format!("{base}/oauth/register"));

    // Registration, the consent page (escaped, framed by nothing), the code.
    let c = t.register("Claude <script>", CLAUDE_CALLBACK).await;
    let (request, cookie, html) = t.consent_page(&c).await;
    assert!(html.contains("Claude &lt;script&gt;") && !html.contains("<script>"), "escaped");
    assert!(html.contains("<strong>claude.ai</strong>"), "the redirect host is shown");
    let connect = t.connect_code("Weekly outreach routine").await;
    assert_eq!(connect.len(), 11);
    // Typed in lower case with a space: still the code.
    let r = t.submit(&request, &cookie, &format!(" {}", connect.to_lowercase().replace('-', " "))).await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    let location = r.headers()["location"].to_str().unwrap().to_owned();
    let set_cookie = r.headers()["set-cookie"].to_str().unwrap().to_owned();
    assert!(set_cookie.contains("Max-Age=0"), "the CSRF cookie is cleared");
    let q = query(&location);
    assert_eq!((q["state"].as_str(), q["iss"].as_str()), ("st-123", base.as_str()));

    let (status, tokens) = t.exchange(&c, &q["code"]).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    assert_eq!(tokens["token_type"], "Bearer");
    assert_eq!(tokens["expires_in"], 3600);
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();

    // MCP with the access token: the same answers as with a static token.
    let config = StreamableHttpClientTransportConfig::with_uri(t.s.url("/mcp")).auth_header(access.clone());
    let client = ().serve(StreamableHttpClientTransport::from_config(config)).await.expect("connect with OAuth");
    let answer = call(&client, "guide_rules", json!({ "to": ["ann@acme.com"] })).await;
    assert!(answer["writing_guide"].as_str().unwrap().contains("Call her Annie"));
    // The token is for /mcp only.
    let r = t.s.get(Some(&access), &format!("/v1/m/{MAILBOX}/guide")).await;
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "audience-bound to /mcp");

    // The app sees it beside its tokens, named as the code was.
    let (_, static_token) = t.s.mint(&t.publisher, "A script").await;
    let agents = t.agents().await;
    assert_eq!(agents.len(), 2);
    assert_eq!(agents[0]["kind"], "oauth");
    assert_eq!(agents[0]["name"], "Weekly outreach routine");
    assert_eq!(agents[0]["client_name"], "Claude <script>");
    assert_eq!(agents[1]["kind"], "token");
    let grant = agents[0]["id"].as_str().unwrap().to_owned();

    // Refresh rotates: a new pair, and the old refresh token is spent.
    let (status, second) = t.refresh(&c, &refresh).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let access2 = second["access_token"].as_str().unwrap().to_owned();
    assert_ne!(access2, access);
    assert_ne!(second["refresh_token"], tokens["refresh_token"]);
    assert_eq!(t.mcp_status(&access2).await, StatusCode::OK);

    // The app revokes it: tokens and refresh stop at once; the static
    // token is untouched.
    let r =
        t.s.http
            .delete(t.s.url(&format!("/v1/mailboxes/{MAILBOX}/agent-tokens/{grant}")))
            .bearer_auth(&t.publisher)
            .send()
            .await
            .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert_eq!(t.mcp_status(&access2).await, StatusCode::UNAUTHORIZED);
    assert!(client.list_all_tools().await.is_err(), "an open client is cut off");
    let (status, body) = t.refresh(&c, second["refresh_token"].as_str().unwrap()).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
    assert_eq!(t.mcp_status(&static_token).await, StatusCode::OK);
    assert!(t.agents().await[0]["revoked_at"].is_string());

    // The connect code worked once.
    let c2 = t.register("Claude", CLAUDE_CALLBACK).await;
    let (request, cookie, _) = t.consent_page(&c2).await;
    let r = t.submit(&request, &cookie, &connect).await;
    assert_eq!(r.status(), StatusCode::OK, "shown again, with an error");
    assert!(r.text().await.unwrap().contains("That code is wrong, used or expired"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refresh_token_used_twice_revokes_the_agent() {
    let t = setup().await;
    let (c, access, refresh) = t.connected("Routine").await;
    // Each token for its own use only.
    assert_eq!(t.mcp_status(&refresh).await, StatusCode::UNAUTHORIZED, "a refresh token is no access token");
    let (status, _) = t.refresh(&c, &access).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "nor an access token a refresh token");
    let other = t.register("Other", CLAUDE_CALLBACK).await;
    let (status, _) = t.refresh(&other, &refresh).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "another client cannot refresh it");
    let (status, second) = t.refresh(&c, &refresh).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = t.refresh(&c, &refresh).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
    for token in [access.as_str(), second["access_token"].as_str().unwrap()] {
        assert_eq!(t.mcp_status(token).await, StatusCode::UNAUTHORIZED, "every token of the grant");
    }
    let (status, _) = t.refresh(&c, second["refresh_token"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "the rotated one too");
    assert!(t.agents().await[0]["revoked_at"].is_string(), "the app sees it revoked");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_authorization_code_works_once_with_its_verifier_and_return_address() {
    let t = setup().await;

    // Replayed: the second exchange is refused and the first's tokens die.
    let c = t.register("Claude", CLAUDE_CALLBACK).await;
    let code = t.authorize(&c, &t.connect_code("One").await).await;
    let (status, tokens) = t.exchange(&c, &code).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = t.exchange(&c, &code).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
    assert_eq!(t.mcp_status(tokens["access_token"].as_str().unwrap()).await, StatusCode::UNAUTHORIZED);

    // A verifier that is not the challenge's: refused, and the code spent.
    let c = t.register("Claude", CLAUDE_CALLBACK).await;
    let code = t.authorize(&c, &t.connect_code("Two").await).await;
    let wrong = Client { id: c.id.clone(), redirect: c.redirect.clone(), verifier: verifier() };
    let (status, body) = t.exchange(&wrong, &code).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")), "{body}");
    let (status, _) = t.exchange(&c, &code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "spent");

    // Another return address at the token endpoint, or another client.
    let c = t.register("Claude", CLAUDE_CALLBACK).await;
    let code = t.authorize(&c, &t.connect_code("Three").await).await;
    let elsewhere =
        Client { id: c.id.clone(), redirect: "https://claude.ai/other".into(), verifier: c.verifier.clone() };
    let (status, body) = t.exchange(&elsewhere, &code).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("invalid_grant")));
    let c = t.register("Claude", CLAUDE_CALLBACK).await;
    let code = t.authorize(&c, &t.connect_code("Four").await).await;
    let other = t.register("Other", CLAUDE_CALLBACK).await;
    let thief = Client { id: other.id, redirect: c.redirect.clone(), verifier: c.verifier.clone() };
    let (status, _) = t.exchange(&thief, &code).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "issued to another client");

    // Unknown client and grant type.
    let (status, body) = t.token(&[("grant_type", "authorization_code"), ("client_id", "nobody")]).await;
    assert_eq!((status, body["error"].as_str()), (StatusCode::UNAUTHORIZED, Some("invalid_client")));
    let (status, body) = t.token(&[("grant_type", "password"), ("client_id", &c.id)]).await;
    assert_eq!(body["error"], "unsupported_grant_type", "{status}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_authorization_request_is_checked_before_and_after_the_return_address() {
    let t = setup().await;
    let c = t.register("Claude", CLAUDE_CALLBACK).await;

    // A return address it did not register: shown here, never redirected.
    for redirect in ["https://evil.example/cb", "https://claude.ai/api/mcp/auth_callback/x"] {
        let r = t.web.get(t.authorize_url(&c, redirect, &[])).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
        assert!(r.headers().get("location").is_none(), "no open redirect");
    }
    let unknown = Client { id: "oagc_cli_nobody".into(), redirect: CLAUDE_CALLBACK.into(), verifier: verifier() };
    let r = t.web.get(t.authorize_url(&unknown, CLAUDE_CALLBACK, &[])).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(r.headers().get("location").is_none());

    // Then errors are shown too, never sent to the return address: anyone
    // can register a client with any https:// address, and a redirect
    // before the user has used the page would make this an open
    // redirector (RFC 9700 §4.11.2).
    let refused = async |extra: &[(&str, &str)]| {
        let r = t.web.get(t.authorize_url(&c, CLAUDE_CALLBACK, extra)).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{extra:?}");
        assert!(r.headers().get("location").is_none(), "no redirect: {extra:?}");
        let html = r.text().await.unwrap();
        html.split("cannot be used (").nth(1).unwrap().split(')').next().unwrap().to_owned()
    };
    assert_eq!(refused(&[("code_challenge", "")]).await, "invalid_request", "PKCE is required");
    assert_eq!(refused(&[("code_challenge_method", "plain")]).await, "invalid_request", "S256 only");
    assert_eq!(refused(&[("response_type", "token")]).await, "unsupported_response_type");
    assert_eq!(refused(&[("resource", "https://other.example/mcp")]).await, "invalid_target");

    // Registration refuses what could send codes in the clear or nowhere.
    for redirect in ["http://claude.ai/cb", "https://claude.ai/cb#frag", "javascript:alert(1)"] {
        let r =
            t.web.post(t.s.url("/oauth/register")).json(&json!({ "redirect_uris": [redirect] })).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST, "{redirect}");
        let body: Value = r.json().await.unwrap();
        assert_eq!(body["error"], "invalid_redirect_uri");
    }

    // A loopback client (Claude Code) on a port of its own each time.
    let local = t.register("Claude Code", "http://127.0.0.1/callback").await;
    let local = Client { redirect: "http://127.0.0.1:43117/callback".into(), ..local };
    let (_, _, html) = t.consent_page(&local).await;
    assert!(html.contains("a program on this computer"), "a loopback warning");
    let code = t.authorize(&local, &t.connect_code("Claude Code").await).await;
    let (status, body) = t.exchange(&local, &code).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_connect_codes_lock_the_page_then_the_client_and_expired_codes_fail() {
    let t = setup().await;
    let c = t.register("Claude", CLAUDE_CALLBACK).await;
    let (request, cookie, _) = t.consent_page(&c).await;
    for left in (1..5).rev() {
        let r = t.submit(&request, &cookie, "ABCDE-FGHJK").await;
        assert_eq!(r.status(), StatusCode::OK);
        let html = r.text().await.unwrap();
        assert!(html.contains(&format!("{left} tr")), "{left} left: {html}");
    }
    let r = t.submit(&request, &cookie, "ABCDE-FGHJK").await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(query(r.headers()["location"].to_str().unwrap())["error"], "access_denied");

    // The client is locked from here: even the right code does not get it
    // in now, and the page says so rather than redirecting.
    let good = t.connect_code("Routine").await;
    let r = t.web.get(t.authorize_url(&c, CLAUDE_CALLBACK, &[])).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert!(r.headers().get("location").is_none());
    // Another client still can, with that code.
    let other = t.register("Claude", CLAUDE_CALLBACK).await;
    t.authorize(&other, &good).await;

    // An expired code is refused like a wrong one.
    let late = t.connect_code("Late").await;
    let db = rules_server::Db::open(&t.s.dir).unwrap();
    db.run_now(|c| c.execute("UPDATE connect_codes SET expires_at = 0 WHERE used_at IS NULL", [])).unwrap();
    let c = t.register("Claude", CLAUDE_CALLBACK).await;
    let (request, cookie, _) = t.consent_page(&c).await;
    let r = t.submit(&request, &cookie, &late).await;
    assert_eq!(r.status(), StatusCode::OK);
    assert!(r.text().await.unwrap().contains("wrong, used or expired"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_consent_form_needs_its_own_page_and_cookie() {
    let t = setup().await;
    let c = t.register("Claude", CLAUDE_CALLBACK).await;
    let code = t.connect_code("Routine").await;

    // No cookie (a form posted from another site): refused.
    let (request, _, _) = t.consent_page(&c).await;
    let r = t.submit(&request, "oagc_consent_x=y", &code).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    // Its cookie, but posted from another origin: refused.
    let (request, cookie, _) = t.consent_page(&c).await;
    let r = t
        .web
        .post(t.s.url("/oauth/authorize"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Cookie", &cookie)
        .header("Origin", "https://evil.example")
        .body(form(&[("request", &request), ("connect_code", &code), ("action", "allow")]))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    // Another page's cookie: refused.
    let (first, first_cookie, _) = t.consent_page(&c).await;
    let (_, second_cookie, _) = t.consent_page(&c).await;
    let swapped = format!("{}={}", first_cookie.split('=').next().unwrap(), second_cookie.split('=').nth(1).unwrap());
    let r = t.submit(&first, &swapped, &code).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);

    // The code was never spent by those: it still works, and Cancel says no.
    let (request, cookie, _) = t.consent_page(&c).await;
    let r = t
        .web
        .post(t.s.url("/oauth/authorize"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Cookie", &cookie)
        .body(form(&[("request", &request), ("action", "deny")]))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(query(r.headers()["location"].to_str().unwrap())["error"], "access_denied");
    t.authorize(&c, &code).await;
    assert_eq!(t.agents().await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_public_url_oauth_is_off_and_tokens_work_as_before() {
    let s = start_with(0, None, false).await;
    let publisher = s.registered().await;
    for path in ["/.well-known/oauth-protected-resource", "/.well-known/oauth-authorization-server", "/oauth/register"]
    {
        assert_eq!(s.get(None, path).await.status(), StatusCode::NOT_FOUND, "{path}");
    }
    let r = s
        .http
        .post(s.url(&format!("/v1/mailboxes/{MAILBOX}/connect-codes")))
        .bearer_auth(&publisher)
        .json(&json!({ "name": "Routine" }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["error"], "oauth_off");
    let r = s.http.post(s.url("/mcp")).json(&json!({})).send().await.unwrap();
    assert_eq!(r.headers()["www-authenticate"], "Bearer realm=\"kaluta-rules\"");
    let (_, agent) = s.mint(&publisher, "Script").await;
    assert!(s.mcp(&agent).await.is_ok());
}

// Encryption at rest (spec §10.6, oagc-gmn7.7): a grant's tokens carry its
// secret, which its key is wrapped under; refreshing keeps it.
#[tokio::test(flavor = "multi_thread")]
async fn a_connectors_tokens_carry_its_grant_secret_and_read_encrypted_snapshots() {
    use rules_crypto as seal;
    let t = setup().await;
    // Connected while the mailbox was plaintext: no key yet.
    let (c, access, refresh) = t.connected("Weekly outreach routine").await;
    let secret = rules_server::tokens::grant_secret_of(&access).expect("a grant secret").to_owned();
    assert_eq!(rules_server::tokens::grant_secret_of(&refresh), Some(secret.as_str()));
    let grant = t.agents().await[0].clone();
    let grant_id = grant["id"].as_str().unwrap().to_owned();

    let mut app = common::SealingApp::new();
    let (status, body) = t.s.push_sealed(&t.publisher, &mut app, &snapshot(2), Some("1")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let client = connect(&t.s, &access).await;
    let refused = client
        .call_tool(rmcp::model::CallToolRequestParams::new("guide_rules").with_arguments(serde_json::Map::new()))
        .await
        .unwrap();
    assert_eq!(refused.is_error, Some(true), "its key is made now; the app has not wrapped for it yet");
    assert_eq!(t.s.rewrap(&t.publisher, &app).await, 1);
    assert_eq!(call(&client, "guide_rules", json!({})).await["version"], 2);

    // Refreshed: new tokens under a new secret, reading at once (the same
    // key, rewrapped: the app's wrap for it stays).
    let wrap_before: Vec<u8> =
        t.s.backup().query_row("SELECT key_wrap FROM agent_tokens WHERE id = ?1", [&grant_id], |r| r.get(0)).unwrap();
    let (status, tokens) = t.refresh(&c, &refresh).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    let access2 = tokens["access_token"].as_str().unwrap();
    let refresh2 = tokens["refresh_token"].as_str().unwrap();
    let secret2 = rules_server::tokens::grant_secret_of(access2).expect("a grant secret").to_owned();
    assert_ne!(secret2, secret, "rotated at the refresh");
    assert_eq!(rules_server::tokens::grant_secret_of(refresh2), Some(secret2.as_str()));
    assert_eq!(call(&connect(&t.s, access2).await, "guide_rules", json!({})).await["version"], 2);
    // The access token from before ends; a leaked copy of it opens nothing
    // in the database from now on.
    assert_eq!(t.mcp_status(&access).await, StatusCode::UNAUTHORIZED);
    let stored = t.s.stored_bytes();
    for kept in [secret.as_str(), secret2.as_str(), access2, &access, &refresh] {
        assert!(!common::contains(&stored, kept), "the database holds a secret");
    }
    assert!(
        !stored.windows(wrap_before.len()).any(|w| w == wrap_before.as_slice()),
        "the wrap under the old secret is gone from the file and its log"
    );
    let key_wrap: Vec<u8> =
        t.s.backup().query_row("SELECT key_wrap FROM agent_tokens WHERE id = ?1", [&grant_id], |r| r.get(0)).unwrap();
    let key = seal::unwrap_agent_key(&seal::credential_key(&secret2, &grant_id), &key_wrap, &grant_id).unwrap();
    assert!(seal::unwrap_agent_key(&seal::credential_key(&secret, &grant_id), &key_wrap, &grant_id).is_err());
    let old_key = seal::unwrap_agent_key(&seal::credential_key(&secret, &grant_id), &wrap_before, &grant_id).unwrap();
    assert_eq!(key, old_key, "the same agent key");
    // Reuse detection still holds: the spent refresh token revokes later.
    let (status, again) = t.refresh(&c, refresh2).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    let (status, _) = t.refresh(&c, refresh2).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(t.mcp_status(again["access_token"].as_str().unwrap()).await, StatusCode::UNAUTHORIZED);

    // A connector that signs in after the mailbox is encrypted has its key
    // from its first tokens, before it makes any request.
    let (_, later, _) = t.connected("Daily digest").await;
    let agents = t.agents().await;
    let new = agents.iter().find(|a| a["name"] == "Daily digest").unwrap();
    assert!(new["agent_key"].is_string());
    assert_eq!(t.s.rewrap(&t.publisher, &app).await, 1);
    assert_eq!(call(&connect(&t.s, &later).await, "guide_rules", json!({})).await["version"], 2);

    // Revoked: its key and wraps go.
    let r =
        t.s.http
            .delete(t.s.url(&format!("/v1/mailboxes/{MAILBOX}/agent-tokens/{grant_id}")))
            .bearer_auth(&t.publisher)
            .send()
            .await
            .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    let left: (Option<Vec<u8>>, i64) = t
        .s
        .backup()
        .query_row(
            "SELECT key_wrap, (SELECT COUNT(*) FROM snapshot_keys WHERE agent_id = ?1) FROM agent_tokens WHERE id = ?1",
            [&grant_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(left, (None, 0));
}

async fn connect(s: &Server, token: &str) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
    let config = StreamableHttpClientTransportConfig::with_uri(s.url("/mcp")).auth_header(token);
    ().serve(StreamableHttpClientTransport::from_config(config)).await.expect("connect")
}

/// Behind a proxy the server trusts, `X-Forwarded-For` names the client.
fn from(r: reqwest::RequestBuilder, address: &str) -> reqwest::RequestBuilder {
    r.header("X-Forwarded-For", address)
}

#[tokio::test(flavor = "multi_thread")]
async fn one_address_cannot_block_sign_in_for_everyone_else() {
    let s = common::start_trusting(0, &["127.0.0.1"]).await;
    let publisher = s.registered().await;
    assert_eq!(s.publish(&publisher, None, snapshot(1).to_json().unwrap()).await.0, StatusCode::OK);
    let t = Setup { s, publisher, web: http() };
    const STRANGER: &str = "203.0.113.9";
    const USER: &str = "198.51.100.7";
    let register = async |address: &str| {
        from(t.web.post(t.s.url("/oauth/register")), address)
            .json(&json!({ "client_name": "Claude", "redirect_uris": [CLAUDE_CALLBACK] }))
            .send()
            .await
            .unwrap()
    };

    // Registrations: the stranger's run out; the user's do not.
    let mut stranger_clients = vec![];
    for _ in 0..rules_server::oauth::REGISTRATIONS_PER_ADDRESS_PER_MINUTE {
        let r = register(STRANGER).await;
        assert_eq!(r.status(), StatusCode::CREATED);
        let id = r.json::<Value>().await.unwrap()["client_id"].as_str().unwrap().to_owned();
        stranger_clients.push(Client { id, redirect: CLAUDE_CALLBACK.into(), verifier: verifier() });
    }
    assert_eq!(register(STRANGER).await.status(), StatusCode::TOO_MANY_REQUESTS);
    let r = register(USER).await;
    assert_eq!(r.status(), StatusCode::CREATED, "another address registers");
    let user = Client {
        id: r.json::<Value>().await.unwrap()["client_id"].as_str().unwrap().to_owned(),
        redirect: CLAUDE_CALLBACK.into(),
        verifier: verifier(),
    };

    // The stranger opens the user's client's page (its id is public) and
    // tries codes until the client is locked: locked for the stranger only.
    let page = async |c: &Client, address: &str| {
        let r = from(t.web.get(t.authorize_url(c, &c.redirect, &[])), address).send().await.unwrap();
        let status = r.status();
        let cookie = r.headers().get("set-cookie").map(|v| v.to_str().unwrap().split(';').next().unwrap().to_owned());
        let html = r.text().await.unwrap();
        let request = html.split("name=\"request\" value=\"").nth(1).map(|r| r.split('"').next().unwrap().to_owned());
        (status, request, cookie)
    };
    let submit = async |request: &str, cookie: &str, code: &str, address: &str| {
        from(t.web.post(t.s.url("/oauth/authorize")), address)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Cookie", cookie)
            .header("Origin", &t.s.base)
            .body(form(&[("request", request), ("connect_code", code), ("action", "allow")]))
            .send()
            .await
            .unwrap()
    };
    let (_, request, cookie) = page(&user, STRANGER).await;
    let (request, cookie) = (request.unwrap(), cookie.unwrap());
    for _ in 0..rules_server::oauth::TRIES_PER_CLIENT {
        let _ = submit(&request, &cookie, "ABCDE-FGHJK", STRANGER).await;
    }
    assert_eq!(page(&user, STRANGER).await.0, StatusCode::FORBIDDEN, "locked for the stranger");
    let code = t.connect_code("Weekly outreach routine").await;
    let (status, request, cookie) = page(&user, USER).await;
    assert_eq!(status, StatusCode::OK, "not for the user");
    let r = submit(&request.unwrap(), &cookie.unwrap(), &code, USER).await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert!(query(r.headers()["location"].to_str().unwrap()).contains_key("code"), "the user connects");

    // Codes tried from one address run out; the user's next try does not.
    let mut refused = false;
    for mine in &stranger_clients[..3] {
        // Fewer than the lockout's tries on each client.
        let (_, request, cookie) = page(mine, STRANGER).await;
        let (request, cookie) = (request.unwrap(), cookie.unwrap());
        for _ in 0..4 {
            let html = submit(&request, &cookie, "ABCDE-FGHJK", STRANGER).await.text().await.unwrap();
            refused |= html.contains("Too many codes were tried from here");
        }
    }
    assert!(refused, "at most {} codes a minute from one address", rules_server::oauth::CODES_PER_ADDRESS_PER_MINUTE);
    let code = t.connect_code("Daily digest").await;
    let second = t.register("Claude", CLAUDE_CALLBACK).await;
    let (_, request, cookie) = page(&second, USER).await;
    let r = submit(&request.unwrap(), &cookie.unwrap(), &code, USER).await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER, "another address still signs in");

    // Consent pages from one address run out too, not everyone's.
    let mut busy = false;
    for _ in 0..rules_server::oauth::CONSENTS_PER_ADDRESS_PER_MINUTE {
        busy |= page(&stranger_clients[3], STRANGER).await.0 == StatusCode::TOO_MANY_REQUESTS;
    }
    assert!(busy);
    assert_eq!(page(&second, USER).await.0, StatusCode::OK);
}
