//! Granola's sign-in and MCP calls against a wiremock server shaped like
//! Granola's (synthetic meetings, example.* addresses).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use provider_api::ProviderError;
use provider_granola::{GranolaMcp, Tokens, oauth};
use serde_json::json;
use wiremock::matchers::{body_partial_json, body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn discovery(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": format!("{}/mcp", server.uri()), "authorization_servers": [server.uri()]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "authorization_endpoint": format!("{}/oauth2/authorize", server.uri()),
            "token_endpoint": format!("{}/oauth2/token", server.uri()),
            "registration_endpoint": format!("{}/oauth2/register", server.uri()),
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn connecting_registers_kaluta_and_trades_the_code_for_tokens() {
    let server = MockServer::start().await;
    discovery(&server).await;
    Mock::given(method("POST"))
        .and(path("/oauth2/register"))
        .and(body_partial_json(json!({ "client_name": "Kaluta", "token_endpoint_auth_method": "none" })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "client_id": "client-1" })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=the-code"))
        .and(body_string_contains("client_id=client-1"))
        .and(body_string_contains("code_verifier="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "at-1", "refresh_token": "rt-1", "expires_in": 3600, "token_type": "Bearer"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let http = reqwest::Client::new();
    let mcp = format!("{}/mcp", server.uri());
    let pending = oauth::begin(&http, &mcp).await.unwrap();
    let url = url::Url::parse(&pending.url).unwrap();
    let param = |k: &str| url.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned()).unwrap();
    assert!(pending.url.starts_with(&format!("{}/oauth2/authorize?", server.uri())));
    assert_eq!(param("code_challenge_method"), "S256");
    assert_eq!(param("resource"), mcp);
    assert!(param("scope").contains("offline_access"));
    let (redirect, state) = (param("redirect_uri"), param("state"));
    let done = tokio::spawn({
        let http = http.clone();
        async move { pending.finish(&http, Duration::from_secs(10)).await }
    });
    // The browser comes back: first a stray request, then the callback.
    let _ = http.get(redirect.replace("/callback", "/favicon.ico")).send().await;
    let page = http.get(format!("{redirect}?code=the-code&state={state}")).send().await.unwrap();
    assert!(page.text().await.unwrap().contains("Granola is connected"));
    let tokens = done.await.unwrap().unwrap();
    assert_eq!((tokens.client_id.as_str(), tokens.access_token.as_str()), ("client-1", "at-1"));
    assert_eq!(tokens.refresh_token.as_deref(), Some("rt-1"));
    assert!(!format!("{tokens:?}").contains("at-1"), "tokens never show in logs");
}

fn tokens(server: &MockServer) -> Tokens {
    Tokens {
        client_id: "client-1".into(),
        token_endpoint: format!("{}/oauth2/token", server.uri()),
        access_token: "old".into(),
        refresh_token: Some("rt-1".into()),
        expires_at: 0,
    }
}

const LIST: &str = r#"<meetings_data connected_user_email="jo@example.com" count="1">
<meeting id="m-1" title="Pilot kickoff" meeting_at="2026-11-04T11:20:00.000-08:00">
<known_participants>Jo Doe (note creator) (jo@example.com), Sam Rivera (sam@example.org)</known_participants>
</meeting></meetings_data><pagination has_more="false" />"#;

/// The session starts with `initialize`; answers may come as an event
/// stream; a refused token is refreshed once and kept.
#[tokio::test]
async fn tools_are_called_in_a_session_and_a_refused_token_is_refreshed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(header("authorization", "Bearer old"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/oauth2/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=rt-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "access_token": "new", "expires_in": 3600 })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(header("authorization", "Bearer new"))
        .and(body_partial_json(json!({ "method": "initialize" })))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("mcp-session-id", "s-1")
                .set_body_json(json!({ "jsonrpc": "2.0", "id": 1, "result": { "protocolVersion": "2025-06-18" } })),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_partial_json(json!({ "method": "notifications/initialized" })))
        .respond_with(ResponseTemplate::new(202))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(header("mcp-session-id", "s-1"))
        .and(body_partial_json(json!({ "method": "tools/call", "params": { "name": "list_meetings" } })))
        .respond_with(move |req: &wiremock::Request| {
            let id = serde_json::from_slice::<serde_json::Value>(&req.body).unwrap()["id"].clone();
            let message =
                json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": LIST }] } });
            ResponseTemplate::new(200).set_body_raw(format!("event: message\ndata: {message}\n\n"), "text/event-stream")
        })
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .and(body_partial_json(json!({ "method": "tools/call", "params": { "name": "get_meetings" } })))
        .respond_with(move |req: &wiremock::Request| {
            let id = serde_json::from_slice::<serde_json::Value>(&req.body).unwrap()["id"].clone();
            ResponseTemplate::new(200).set_body_json(json!({ "jsonrpc": "2.0", "id": id, "result": {
                "isError": true, "content": [{ "type": "text", "text": "Meeting not found" }] } }))
        })
        .mount(&server)
        .await;

    let kept: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = kept.clone();
    let mcp = GranolaMcp::new(
        reqwest::Client::new(),
        &format!("{}/mcp", server.uri()),
        tokens(&server),
        Some(Arc::new(move |t: &Tokens| sink.lock().unwrap().push(t.access_token.clone()))),
    );
    let page = mcp.list_meetings("2026-10-28", "2026-11-04", None).await.unwrap();
    assert_eq!(page.connected_email.as_deref(), Some("jo@example.com"));
    assert_eq!(page.meetings[0].participants[1].email.as_deref(), Some("sam@example.org"));
    assert_eq!(*kept.lock().unwrap(), vec!["new".to_owned()], "the new token is kept");
    let err = mcp.meeting_notes("nope").await.unwrap_err();
    assert!(matches!(err, ProviderError::Invalid(ref m) if m.contains("Meeting not found")), "{err:?}");
}
