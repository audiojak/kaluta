//! An in-process rules server on loopback, a scratch data directory, and
//! the publisher's and agents' calls.

#![allow(dead_code)]

use std::path::PathBuf;

use reqwest::StatusCode;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rules_server::Config;
use serde_json::{Value, json};
use writing_guide::{
    AudienceGroup, AudienceGroups, Check, CheckKind, Entry, Fact, Kind, Mailbox, SCHEMA_VERSION, Scope, Snapshot,
};

pub const MAILBOX: &str = "scout@agents.example";

pub struct Server {
    pub base: String,
    pub dir: PathBuf,
    pub http: reqwest::Client,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub async fn start(rate_limit_per_minute: u32, registration_token: Option<&str>) -> Server {
    start_with(rate_limit_per_minute, registration_token, true).await
}

/// With `oauth`, the server's public URL is its loopback address.
pub async fn start_with(rate_limit_per_minute: u32, registration_token: Option<&str>, oauth: bool) -> Server {
    let dir = std::env::temp_dir().join(format!("openagc-rules-test-{}", rules_server::tokens::new_id()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind loopback");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let config = Config {
        data_dir: dir.clone(),
        rate_limit_per_minute,
        registration_token: registration_token.map(str::to_owned),
        public_url: oauth.then(|| base.clone()),
    };
    let app = rules_server::app(&config).expect("app");
    tokio::spawn(async move { axum::serve(listener, app).await });
    Server { base, dir, http: reqwest::Client::new() }
}

/// The representative guide, as the app would publish it: plain records
/// hashed with a salt.
pub fn snapshot(version: i64) -> Snapshot {
    let entry = |id, category: &str, kind, statement: &str, scope, check| Entry {
        id,
        category: category.into(),
        kind,
        statement: statement.into(),
        scope,
        check,
    };
    let customers = Scope { groups: vec!["Customers".into()], ..Default::default() };
    let ann = Scope { people: vec!["ann@acme.com".into()], ..Default::default() };
    let globex = Scope { people: vec!["@globex.com".into()], ..Default::default() };
    let banned = |v: &str| Some(Check { kind: CheckKind::BannedPhrase, value: v.into() });
    Snapshot {
        schema_version: SCHEMA_VERSION,
        version,
        guide_version: 12,
        published_at: 1_760_000_000_000 + version * 60_000,
        mailbox: Mailbox { address: MAILBOX.into(), name: "Scout".into(), about: "You send as Scout.\n".into() },
        entries: vec![
            entry(1, "B6", Kind::Rule, "Never say 'circle back'", Scope::default(), banned("circle back")),
            entry(2, "A2", Kind::Guideline, "Be formal", customers, None),
            entry(3, "A3", Kind::Guideline, "Call her Annie", ann, None),
            entry(4, "B4", Kind::Rule, "Never mention pricing", globex, banned("pricing")),
        ],
        audiences: AudienceGroups::plain(vec![AudienceGroup {
            name: "Customers".into(),
            members: vec!["@acme.com".into()],
        }]),
        facts: vec![Fact {
            category_key: "work".into(),
            category: "Work".into(),
            label: "Calendar".into(),
            value: "cal.com/scout".into(),
            ask_before_using: false,
        }],
    }
    .hashed("pepper")
}

impl Server {
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub async fn register(&self, address: &str) -> (StatusCode, Value) {
        let r = self.http.post(self.url("/v1/mailboxes")).json(&json!({ "address": address })).send().await.unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }

    /// Register `MAILBOX` and answer its publisher token.
    pub async fn registered(&self) -> String {
        let (status, body) = self.register(MAILBOX).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["publisher_token"].as_str().unwrap().to_owned()
    }

    pub async fn publish(&self, token: &str, if_match: Option<&str>, body: String) -> (StatusCode, Value) {
        let mut req =
            self.http.put(self.url(&format!("/v1/mailboxes/{MAILBOX}/snapshot"))).bearer_auth(token).body(body);
        if let Some(v) = if_match {
            req = req.header("If-Match", v);
        }
        let r = req.send().await.unwrap();
        (r.status(), r.json().await.unwrap_or(Value::Null))
    }

    pub async fn mint(&self, publisher: &str, name: &str) -> (String, String) {
        let r = self
            .http
            .post(self.url(&format!("/v1/mailboxes/{MAILBOX}/agent-tokens")))
            .bearer_auth(publisher)
            .json(&json!({ "name": name }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CREATED);
        let body: Value = r.json().await.unwrap();
        (body["id"].as_str().unwrap().to_owned(), body["token"].as_str().unwrap().to_owned())
    }

    pub async fn get(&self, token: Option<&str>, path: &str) -> reqwest::Response {
        let mut req = self.http.get(self.url(path));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        req.send().await.unwrap()
    }

    pub async fn mcp(&self, token: &str) -> Result<RunningService<rmcp::RoleClient, ()>, String> {
        let config = StreamableHttpClientTransportConfig::with_uri(self.url("/mcp")).auth_header(token);
        ().serve(StreamableHttpClientTransport::from_config(config)).await.map_err(|e| e.to_string())
    }
}

pub async fn call(client: &RunningService<rmcp::RoleClient, ()>, tool: &'static str, args: Value) -> Value {
    let Value::Object(args) = args else { panic!("arguments are an object") };
    let result = client.call_tool(CallToolRequestParams::new(tool).with_arguments(args)).await.expect("tool call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    result.structured_content.expect("a structured answer")
}
