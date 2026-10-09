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
    let dir = std::env::temp_dir().join(format!("kaluta-rules-test-{}", rules_server::tokens::new_id()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind loopback");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let config = Config {
        data_dir: dir.clone(),
        rate_limit_per_minute,
        registration_token: registration_token.map(str::to_owned),
        public_url: oauth.then(|| base.clone()),
        require_encryption: false,
        trusted_proxies: vec![],
    };
    let app = rules_server::app(&config).expect("app");
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await
    });
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

/// With OAuth on, behind the proxies `trusted` (whose `X-Forwarded-For`
/// then names the client).
pub async fn start_trusting(rate_limit_per_minute: u32, trusted: &[&str]) -> Server {
    let dir = std::env::temp_dir().join(format!("kaluta-rules-test-{}", rules_server::tokens::new_id()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind loopback");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let config = Config {
        data_dir: dir.clone(),
        rate_limit_per_minute,
        registration_token: None,
        public_url: Some(base.clone()),
        require_encryption: false,
        trusted_proxies: trusted.iter().map(|t| (*t).to_owned()).collect(),
    };
    let app = rules_server::app(&config).expect("app");
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await
    });
    Server { base, dir, http: reqwest::Client::new() }
}

/// With `require_encryption` set as given (and no OAuth).
pub async fn start_requiring(require_encryption: bool) -> Server {
    let dir = std::env::temp_dir().join(format!("kaluta-rules-test-{}", rules_server::tokens::new_id()));
    start_in(dir, require_encryption).await
}

/// Another server on `dir`, as when the operator restarts it with other
/// settings: keep the first alive while this one is used.
pub async fn start_in(dir: PathBuf, require_encryption: bool) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind loopback");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let config = Config {
        data_dir: dir.clone(),
        rate_limit_per_minute: 0,
        registration_token: None,
        public_url: Some(base.clone()),
        require_encryption,
        trusted_proxies: vec![],
    };
    let app = rules_server::app(&config).expect("app");
    tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await
    });
    Server { base, dir, http: reqwest::Client::new() }
}

/// What the app does to publish encrypted (spec §10.6): its own key pair,
/// and the newest snapshot key with its id.
pub struct SealingApp {
    pub key: rules_crypto::AppKey,
    pub snapshot_key: Option<(rules_crypto::SecretKey, String)>,
}

impl SealingApp {
    pub fn new() -> Self {
        Self { key: rules_crypto::AppKey::generate(), snapshot_key: None }
    }

    /// Wraps of `key` for each live agent whose key the server sealed to
    /// this app, as the app makes them: `only_unreadable` leaves out agents
    /// that can read the newest version already.
    pub fn wraps(
        &self,
        agents: &[Value],
        key: &rules_crypto::SecretKey,
        key_id: &str,
        only_unreadable: bool,
    ) -> Vec<rules_crypto::KeyWrap> {
        use rules_crypto as seal;
        agents
            .iter()
            .filter(|a| a["revoked_at"].is_null() && !(only_unreadable && a["readable"] == true))
            .filter_map(|a| {
                let id = a["id"].as_str()?;
                let sealed = seal::unb64(a["agent_key"].as_str()?).ok()?;
                let agent = seal::open_agent_key_for_app(&self.key, &sealed, id).ok()?;
                Some(seal::KeyWrap {
                    agent_id: id.to_owned(),
                    wrap: seal::b64(&seal::wrap_snapshot_key(&agent, key, id, key_id)),
                })
            })
            .collect()
    }
}

impl Server {
    pub async fn agents(&self, publisher: &str) -> Value {
        let r = self.get(Some(publisher), &format!("/v1/mailboxes/{MAILBOX}/agent-tokens")).await;
        assert_eq!(r.status(), StatusCode::OK);
        r.json().await.unwrap()
    }

    /// Push `snapshot` encrypted under a fresh key wrapped for the live
    /// agents, as the app does.
    pub async fn push_sealed(
        &self,
        publisher: &str,
        app: &mut SealingApp,
        snapshot: &Snapshot,
        if_match: Option<&str>,
    ) -> (StatusCode, Value) {
        use rules_crypto as seal;
        let agents = self.agents(publisher).await;
        let key = seal::SecretKey::random();
        let key_id = seal::new_key_id();
        let json = snapshot.to_json().unwrap();
        let body = seal::SealedSnapshot {
            encryption: seal::ENCRYPTION_VERSION,
            key_id: key_id.clone(),
            version: snapshot.version,
            published_at: snapshot.published_at,
            schema_version: snapshot.schema_version,
            address: MAILBOX.into(),
            ciphertext: seal::b64(&seal::seal_snapshot(
                &key,
                &key_id,
                MAILBOX,
                snapshot.version,
                snapshot.schema_version,
                &json,
            )),
            app_key: app.key.public_base64(),
            wraps: app.wraps(agents["agent_tokens"].as_array().unwrap(), &key, &key_id, false),
        };
        let answer = self.publish(publisher, if_match, serde_json::to_string(&body).unwrap()).await;
        if answer.0 == StatusCode::OK {
            app.snapshot_key = Some((key, key_id));
        }
        answer
    }

    /// Wrap the newest snapshot key for agents that cannot read it yet, as
    /// the app does when it sees them in the list. How many were stored.
    pub async fn rewrap(&self, publisher: &str, app: &SealingApp) -> i64 {
        let agents = self.agents(publisher).await;
        let (key, key_id) = app.snapshot_key.as_ref().expect("pushed");
        assert_eq!(agents["key_id"], key_id.as_str());
        let wraps = app.wraps(agents["agent_tokens"].as_array().unwrap(), key, key_id, true);
        let r = self
            .http
            .post(self.url(&format!("/v1/mailboxes/{MAILBOX}/snapshot/keys")))
            .bearer_auth(publisher)
            .json(&json!({ "key_id": key_id, "wraps": wraps }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        r.json::<Value>().await.unwrap()["stored"].as_i64().unwrap()
    }

    /// Every byte the server has written: its database and write-ahead log.
    pub fn stored_bytes(&self) -> Vec<u8> {
        let mut all = Vec::new();
        for name in [rules_server::db::FILE_NAME.to_owned(), format!("{}-wal", rules_server::db::FILE_NAME)] {
            all.extend(std::fs::read(self.dir.join(name)).unwrap_or_default());
        }
        all
    }

    /// A copy of the database as it stands, as a backup would be.
    pub fn backup(&self) -> rusqlite::Connection {
        let to = self.dir.join(format!("backup-{}.sqlite3", rules_server::tokens::new_id()));
        let c = rusqlite::Connection::open(self.dir.join(rules_server::db::FILE_NAME)).unwrap();
        c.execute("VACUUM INTO ?1", [to.to_string_lossy()]).unwrap();
        rusqlite::Connection::open(to).unwrap()
    }
}

/// Whether `haystack` holds `needle` anywhere.
pub fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle.as_bytes())
}
