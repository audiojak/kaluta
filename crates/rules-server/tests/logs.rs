//! What the server logs: one line per request naming the token's id, and
//! never a token, an address it was asked about or what a snapshot says.
//! Its own test binary, as it installs the process's subscriber.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use common::{MAILBOX, call, snapshot, start};
use serde_json::json;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn logs_name_token_ids_and_nothing_secret() {
    let captured = Captured::default();
    let writer = captured.clone();
    // As `openagc-rules` sets it, with the server's own lines at debug.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn,rules_server=debug"))
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .init();

    let s = start(0, None).await;
    let publisher = s.registered().await;
    let (status, _) = s.publish(&publisher, None, snapshot(1).to_json().unwrap()).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let (id, agent) = s.mint(&publisher, "Routine").await;
    let r = s.get(Some(&agent), &format!("/v1/m/{MAILBOX}/guide?to=bea@globex.com")).await;
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let client = s.mcp(&agent).await.unwrap();
    call(&client, "guide_rules", json!({ "to": ["ann@acme.com"] })).await;
    let _ = s.get(Some("oagc_agt_0123456789abcdef_guess"), "/v1/m/x@y.z/facts").await;
    let oauth_secrets = oauth_flow(&s, &publisher).await;
    let leaked = unreadable_sealed_snapshot(&s, &publisher, &agent).await;

    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("route=\"/oauth/token\"") && log.contains("agent connected with a connect code"), "{log}");
    for secret in &oauth_secrets {
        assert!(!log.contains(secret.as_str()), "an OAuth secret was logged: {log}");
    }
    assert!(log.contains(&format!("token=\"agent:{id}\"")), "{log}");
    assert!(log.contains("route=\"/v1/m/{address}/guide\"") && log.contains("route=\"/mcp\""), "{log}");
    assert!(log.contains(&format!("tool call token={id} tool=\"guide_rules\"")), "{log}");
    assert!(log.contains("status=401"), "{log}");
    for secret in [publisher.as_str(), agent.as_str(), "oagc_agt_0123456789abcdef_guess"] {
        assert!(!log.contains(secret), "a token was logged: {log}");
    }
    assert!(log.contains("a published snapshot did not read"), "{log}");
    for content in ["bea@globex.com", "ann@acme.com", "circle back", "cal.com/scout", "Scout", MAILBOX, leaked] {
        assert!(!log.contains(content), "{content:?} was logged: {log}");
    }
}

/// A sealed snapshot that opens but does not read (a broken app build):
/// what serde would say about it quotes the value it choked on. The value.
async fn unreadable_sealed_snapshot(s: &common::Server, publisher: &str, agent: &str) -> &'static str {
    use rules_crypto as seal;
    const LEAKY: &str = "leaky-value-7781";
    let mut app = common::SealingApp::new();
    let (status, _) = s.push_sealed(publisher, &mut app, &snapshot(2), Some("1")).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    // The agent's key is made at its request; the app can then wrap for it.
    let _ = s.get(Some(agent), &format!("/v1/m/{MAILBOX}/guide")).await;
    let agents = s.agents(publisher).await;
    let key = seal::SecretKey::random();
    let key_id = seal::new_key_id();
    let json = format!("{{\"schema_version\":1,\"version\":\"{LEAKY}\"}}");
    let body = seal::SealedSnapshot {
        encryption: seal::ENCRYPTION_VERSION,
        key_id: key_id.clone(),
        version: 3,
        published_at: 3,
        schema_version: 1,
        address: MAILBOX.into(),
        ciphertext: seal::b64(&seal::seal_snapshot(&key, &key_id, MAILBOX, 3, 1, &json)),
        app_key: app.key.public_base64(),
        wraps: app.wraps(agents["agent_tokens"].as_array().unwrap(), &key, &key_id, false),
    };
    assert!(!body.wraps.is_empty());
    let (status, _) = s.publish(publisher, Some("2"), serde_json::to_string(&body).unwrap()).await;
    assert_eq!(status, reqwest::StatusCode::OK);
    let r = s.get(Some(agent), &format!("/v1/m/{MAILBOX}/guide")).await;
    assert_eq!(r.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);
    LEAKY
}

/// A connector's whole flow, with one wrong code: every secret it saw
/// (connect codes, the state, the PKCE verifier and challenge, the
/// authorization code, the tokens, the CSRF cookie).
async fn oauth_flow(s: &common::Server, publisher: &str) -> Vec<String> {
    use base64::Engine;
    use sha2::Digest;
    let web = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let form = |pairs: &[(&str, &str)]| {
        let mut f = url::form_urlencoded::Serializer::new(String::new());
        for (k, v) in pairs {
            f.append_pair(k, v);
        }
        f.finish()
    };
    let r = s
        .http
        .post(s.url(&format!("/v1/mailboxes/{MAILBOX}/connect-codes")))
        .bearer_auth(publisher)
        .json(&json!({ "name": "Routine" }))
        .send()
        .await
        .unwrap();
    let code = r.json::<serde_json::Value>().await.unwrap()["code"].as_str().unwrap().to_owned();
    let redirect = "https://claude.ai/api/mcp/auth_callback";
    let r = web.post(s.url("/oauth/register")).json(&json!({ "redirect_uris": [redirect] })).send().await.unwrap();
    let client = r.json::<serde_json::Value>().await.unwrap()["client_id"].as_str().unwrap().to_owned();
    let verifier = format!("{}{}", rules_server::tokens::secret(), rules_server::tokens::secret());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(&verifier));
    let state = "state-SECRET-4711";
    let query = form(&[
        ("response_type", "code"),
        ("client_id", &client),
        ("redirect_uri", redirect),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
    ]);
    let r = web.get(format!("{}?{query}", s.url("/oauth/authorize"))).send().await.unwrap();
    let cookie = r.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_owned();
    let html = r.text().await.unwrap();
    let marker = "name=\"request\" value=\"";
    let at = html.find(marker).unwrap() + marker.len();
    let request = html[at..].split('"').next().unwrap().to_owned();
    let mut auth_code = String::new();
    for typed in ["ZZZZZ-ZZZZZ", code.as_str()] {
        let r = web
            .post(s.url("/oauth/authorize"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Cookie", &cookie)
            .body(form(&[("request", &request), ("connect_code", typed), ("action", "allow")]))
            .send()
            .await
            .unwrap();
        if let Some(location) = r.headers().get("location") {
            let url = url::Url::parse(location.to_str().unwrap()).unwrap();
            auth_code = url.query_pairs().find(|(k, _)| k == "code").unwrap().1.into_owned();
        }
    }
    let r = web
        .post(s.url("/oauth/token"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form(&[
            ("grant_type", "authorization_code"),
            ("code", &auth_code),
            ("redirect_uri", redirect),
            ("client_id", &client),
            ("code_verifier", &verifier),
        ]))
        .send()
        .await
        .unwrap();
    let tokens: serde_json::Value = r.json().await.unwrap();
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    let refresh = tokens["refresh_token"].as_str().unwrap().to_owned();
    let mcp = s.mcp(&access).await.unwrap();
    call(&mcp, "facts_lookup", json!({})).await;
    vec![
        code.clone(),
        code.replace('-', ""),
        "ZZZZZ".into(),
        state.into(),
        verifier,
        challenge,
        auth_code,
        access,
        refresh,
        cookie.split('=').nth(1).unwrap().to_owned(),
        request,
    ]
}
