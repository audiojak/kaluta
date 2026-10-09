//! Publishing an agent mailbox's writing guide and shared facts to a rules
//! server (spec §10.6, ADR 0016; `docs/plans/rules-server.md`, step 4), so
//! cloud agents that cannot reach this Mac follow them too.
//!
//! The account directory holds `rules-server.json`: the server, whether the
//! mailbox publishes there, the salt its addresses are hashed with, and the
//! last version pushed, when, and why the last push failed. The publisher
//! token the server answered at registration is in the Keychain as
//! `rules.publish_token.<server>.<account>` (spec §12).
//!
//! What goes is a [`Snapshot`], built as mailbox mode builds the guide: the
//! accepted rules and guidelines with their scope and checks (never their
//! evidence), the confirmed audience groups and the people entries are for
//! as salted hashes, the facts shared with cloud agents (each fact's
//! *Share with cloud agents* switch, [`crate::facts::shares_with_cloud`]),
//! and the mailbox's address, name and what an agent is told about it.
//! Never mail, quotes from it, or keys.
//!
//! A change to the guide or facts marks the mailboxes that publish; a
//! worker pushes each one [`DEBOUNCE`] after its last change (at most
//! [`MAX_WAIT`] after the first), and not at all when what would go is what
//! went last. Each push has a version one above the last, sent with
//! `If-Match` on the version the server holds; a refused push takes the
//! server's version from its answer and pushes again, so versions only go
//! up, across restarts too.
//!
//! With the publisher token the app also pulls the reports cloud agents
//! file about what they sent ([`reports`], oagc-gmn7.6), at each sync and on
//! *Publish Now*, and records them as AI compositions.
//!
//! With the publisher token the app also manages the mailbox's agents on
//! the server (*Connect a Cloud Agent…*, oagc-gmn7.5): it mints agent
//! tokens and one-time connect codes (which a claude.ai connector's OAuth
//! sign-in asks for), lists the agents, static tokens and OAuth grants
//! alike, and revokes them. Tokens and codes are shown once and kept
//! nowhere on this Mac.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, IF_MATCH};
use reqwest::{RequestBuilder, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::Instant;
use writing_guide::Snapshot;

use crate::agent_mailbox::{AgentMeta, AgentService, service_limits};
use crate::facts::{FactStatus, shared};
use crate::guide::{GuideKind, GuideStatus};
use crate::registry::{accounts_dir, scoped};
use crate::secrets::keys;
use crate::{Core, CoreError, CoreEvent, ErrorKind, runtime};

const RECORD_FILE: &str = "rules-server.json";
/// How long after the last change a push waits, so a burst of edits is one
/// version.
pub(crate) const DEBOUNCE: Duration = Duration::from_secs(5);
/// The longest a push waits while changes keep coming.
pub(crate) const MAX_WAIT: Duration = Duration::from_secs(30);
/// When a push that failed for a while (the server unreachable, busy or
/// limiting) is tried again.
pub(crate) const RETRY: Duration = Duration::from_secs(60);
/// Pushes of one version before giving up: refused versions, and one
/// registration again for a server that forgot the mailbox.
const ATTEMPTS: usize = 4;

/// An agent mailbox's publishing, as Settings shows it (spec §10.6).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RulesPublication {
    /// The server, as `https://rules.example.com`.
    pub server_url: String,
    /// Changes are pushed; false once publishing stopped.
    pub enabled: bool,
    /// The last version the server took, if any.
    pub version: Option<i64>,
    /// When, in milliseconds since the Unix epoch.
    pub published_at: Option<i64>,
    /// Why the last push failed, in words; `None` when it did not.
    pub error: Option<String>,
    /// A change waits to be pushed.
    pub pending: bool,
    /// The last version went encrypted (spec §10.6, encryption at rest).
    pub encrypted: bool,
    /// Cloud agents' reports that could not be opened or read: those still
    /// being tried and those given up on since publishing started.
    pub unreadable_reports: u32,
}

/// Whether a rules server keeps snapshots encrypted at rest (spec §10.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RulesEncryption {
    /// It refuses plaintext: every publication there is encrypted.
    Required,
    /// The publisher chooses (on by default).
    Optional,
    /// An older server that cannot store encrypted snapshots.
    Unsupported,
}

/// Exactly what a push sends, for the publish sheet.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RulesPreview {
    pub address: String,
    /// The name it sends as.
    pub name: String,
    /// Accepted rules and guidelines, in the guide's order.
    pub entries: Vec<RulesPreviewEntry>,
    /// The facts shared with cloud agents.
    pub facts: Vec<RulesPreviewFact>,
    /// Confirmed audience groups; their members go as salted hashes.
    pub audiences: Vec<RulesPreviewAudience>,
    /// Facts drafting uses here that stay on this Mac (not shared with
    /// cloud agents).
    pub facts_kept: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RulesPreviewEntry {
    pub kind: GuideKind,
    pub statement: String,
    /// Where it applies, in words, people counted rather than named: "for
    /// Customers; to 2 people". Empty when always.
    pub scope: String,
    /// It carries a check (a banned or required phrase, a length).
    pub has_check: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RulesPreviewFact {
    /// The category's name.
    pub category: String,
    pub label: String,
    /// *Ask before using*: the agent is told to ask first.
    pub ask_before_using: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RulesPreviewAudience {
    pub name: String,
    /// How many addresses and domains, each sent as a salted hash.
    pub members: u32,
}

/// A one-time connect code for a rules server's OAuth sign-in, shown once
/// (spec §10.6): whoever types it on the server's consent page connects an
/// agent, named `name`, to the mailbox.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RulesConnectCode {
    /// As typed: `ABCDE-FGHJK`.
    pub code: String,
    pub name: String,
    /// When it stops working (10 minutes on), in milliseconds since the
    /// Unix epoch.
    pub expires_at: i64,
}

/// A static agent token, shown once.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RulesAgentToken {
    pub id: String,
    pub name: String,
    /// `oagc_agt_…`, for `Authorization: Bearer`.
    pub token: String,
}

/// How an agent reaches a mailbox on a rules server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RulesAgentKind {
    /// A static bearer token (Claude Code, the Agent SDK, scripts).
    Token,
    /// Signed in with OAuth and a connect code (a claude.ai connector, a
    /// cloud routine).
    Connector,
}

/// An agent connected to a mailbox on its rules server.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RulesAgent {
    pub id: String,
    /// The name the user gave it.
    pub name: String,
    pub kind: RulesAgentKind,
    /// A connector's app, as it named itself when it registered ("Claude").
    pub client_name: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub created_at: i64,
    pub revoked_at: Option<i64>,
    /// When the server last let it in (to the minute), if it ever did and
    /// says so.
    pub last_used_at: Option<i64>,
}

/// How agents reach a mailbox's rules server, for *Connect a Cloud Agent…*
/// (spec §10.6).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RulesConnectInfo {
    /// The server as agents reach it: its public URL when its operator set
    /// one, else the address this Mac publishes to.
    pub base_url: String,
    /// Its MCP endpoint, `<base_url>/mcp`: what a claude.ai connector or
    /// `claude mcp add` is given.
    pub mcp_url: String,
    /// It signs agents in with OAuth and connect codes (its operator set
    /// its public URL), which claude.ai connectors and cloud routines need.
    pub oauth: bool,
}

/// `rules-server.json` in the account directory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    server_url: String,
    #[serde(default)]
    enabled: bool,
    /// What addresses are hashed with; new at each registration.
    salt: String,
    /// The last version the server took; the next push is one above.
    #[serde(default)]
    last_version: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    published_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    /// A digest of what the last push sent (versions and times aside), so
    /// a change that does not touch it pushes nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    /// Made for a snapshot of the app (`debug_set_rules_publication`):
    /// never pushed anywhere.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    sample: bool,
    /// Push encrypted (spec §10.6). Unset in records from before
    /// encryption: those encrypt at their next push.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    encrypt: Option<bool>,
    /// The key id of the last version pushed encrypted; `None` when it went
    /// in plaintext.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_id: Option<String>,
    /// Reports given up on unread since publishing started (spec §10.6):
    /// acknowledged so they stop holding back the rest, and counted.
    #[serde(default, skip_serializing_if = "is_zero")]
    unreadable_reports: u32,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Record {
    fn encrypts(&self) -> bool {
        self.encrypt.unwrap_or(true)
    }
}

fn record_path(data_dir: &Path, account_id: &str) -> PathBuf {
    accounts_dir(data_dir).join(account_id).join(RECORD_FILE)
}

fn read_record(path: &Path) -> Option<Record> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn write_record(path: &Path, record: &Record) -> Result<(), CoreError> {
    let storage = |e: std::io::Error| CoreError::new(ErrorKind::Storage, e.to_string());
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(record).map_err(|e| CoreError::new(ErrorKind::Internal, e.to_string()))?;
    std::fs::write(&tmp, bytes).map_err(storage)?;
    std::fs::rename(tmp, path).map_err(storage)
}

/// A rules server's address, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Server {
    base: Url,
    /// `https://rules.example.com`, as stored and shown.
    url: String,
    /// Host, port and path: the Keychain item's `<server>`.
    key: String,
    /// Host and port, for messages.
    host: String,
}

/// Read a server address: `https://` (added when no scheme is given), or
/// plain `http://` to this Mac only (a server run here for trying it out).
/// No user name, password, query or fragment.
pub(crate) fn parse_server(input: &str) -> Result<Server, CoreError> {
    let invalid = |m: &str| CoreError::new(ErrorKind::InvalidInput, m.to_owned());
    let input = input.trim();
    if input.is_empty() {
        return Err(invalid("enter the rules server's address, such as https://rules.example.com"));
    }
    let with_scheme = if input.contains("://") { input.to_owned() } else { format!("https://{input}") };
    let url = Url::parse(&with_scheme).map_err(|_| invalid("that is not a web address"))?;
    let host = url.host_str().unwrap_or_default().to_lowercase();
    if host.is_empty() {
        return Err(invalid("that is not a web address"));
    }
    let loopback = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]");
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        "http" => return Err(invalid("use https://: tokens and your guide must not cross the network in the clear")),
        _ => return Err(invalid("use an https:// address")),
    }
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err(invalid("give the server's address alone, without a name, password, query or #"));
    }
    let path = url.path().trim_end_matches('/').to_owned();
    let host_port = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    let mut base = url.clone();
    base.set_path(&path);
    let shown = format!("{}://{host_port}{path}", url.scheme());
    Ok(Server { base, url: shown, key: format!("{host_port}{path}"), host: host_port })
}

impl Server {
    fn endpoint(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().extend(segments);
        }
        url
    }
}

/// Why a push or registration did not happen, in words for Settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Worth trying again later by itself: unreachable, busy, limiting.
    Transient(String),
    /// Needs the user (or the server's operator).
    Final(String),
}

impl Failure {
    fn message(&self) -> &str {
        match self {
            Self::Transient(m) | Self::Final(m) => m,
        }
    }

    fn into_error(self) -> CoreError {
        match self {
            Self::Transient(m) => CoreError::new(ErrorKind::Network, m),
            Self::Final(m) => CoreError::new(ErrorKind::InvalidInput, m),
        }
    }
}

impl From<CoreError> for Failure {
    fn from(e: CoreError) -> Self {
        Self::Final(e.to_string())
    }
}

/// A server's answer: its status and JSON body (`Null` when none).
struct Answer {
    status: u16,
    body: Value,
}

impl Answer {
    /// The server's own words, or its status.
    fn says(&self) -> String {
        self.body["message"].as_str().map_or_else(|| format!("status {}", self.status), str::to_owned)
    }
}

/// The publisher's state in the core.
#[derive(Default)]
pub(crate) struct RulesState {
    /// The worker's queue: an account changed, or `None` for every
    /// publishing one (a global fact changed).
    tx: OnceLock<mpsc::UnboundedSender<Option<String>>>,
    starting: Mutex<()>,
    /// One push or registration at a time.
    pushing: tokio::sync::Mutex<()>,
    /// One pull of reports at a time.
    pulling: tokio::sync::Mutex<()>,
    /// When each account's reports were last pulled at a sync.
    pulled: Mutex<HashMap<String, std::time::Instant>>,
    /// Reports that did not open or read, by account and the server's key
    /// for them: how many pulls they failed. Each holds back the
    /// acknowledgement until it reads or is given up on.
    report_failures: Mutex<HashMap<(String, String, i64), u32>>,
    /// Record files are read and rewritten under this.
    records: Mutex<()>,
    /// Accounts with a change not pushed yet.
    pending: Mutex<HashSet<String>>,
    client: OnceLock<reqwest::Client>,
    /// Tests: other waits, in milliseconds (0: the defaults).
    debounce_ms: AtomicU64,
    max_wait_ms: AtomicU64,
    retry_ms: AtomicU64,
}

impl RulesState {
    fn wait(field: &AtomicU64, default: Duration) -> Duration {
        match field.load(Ordering::Relaxed) {
            0 => default,
            ms => Duration::from_millis(ms),
        }
    }

    fn set_pending(&self, account: &str, pending: bool) {
        let mut set = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending {
            set.insert(account.to_owned());
        } else {
            set.remove(account);
        }
    }

    fn is_pending(&self, account: &str) -> bool {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).contains(account)
    }

    /// How many of `account`'s reports are failing to read and still tried.
    fn failing_reports(&self, account: &str) -> u32 {
        let failures = self.report_failures.lock().unwrap_or_else(|e| e.into_inner());
        u32::try_from(failures.keys().filter(|(a, _, _)| a == account).count()).unwrap_or(u32::MAX)
    }

    #[cfg(test)]
    pub(crate) fn set_waits(&self, debounce: Duration, max_wait: Duration, retry: Duration) {
        let ms = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX).max(1);
        self.debounce_ms.store(ms(debounce), Ordering::Relaxed);
        self.max_wait_ms.store(ms(max_wait), Ordering::Relaxed);
        self.retry_ms.store(ms(retry), Ordering::Relaxed);
    }
}

fn client(state: &RulesState) -> Result<reqwest::Client, Failure> {
    if let Some(c) = state.client.get() {
        return Ok(c.clone());
    }
    let built = reqwest::Client::builder()
        .user_agent(concat!("Kaluta/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        // A rules server never redirects; following one could carry the
        // token elsewhere.
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| Failure::Final(e.without_url().to_string()))?;
    Ok(state.client.get_or_init(|| built).clone())
}

async fn send(request: RequestBuilder, host: &str) -> Result<Answer, Failure> {
    let response =
        request.send().await.map_err(|e| Failure::Transient(format!("Could not reach {host}: {}", e.without_url())))?;
    let status = response.status().as_u16();
    let body = response.json::<Value>().await.unwrap_or(Value::Null);
    Ok(Answer { status, body })
}

/// A busy or failing server, said in words; tried again later.
fn transient(host: &str, a: &Answer) -> Option<Failure> {
    match a.status {
        429 => Some(Failure::Transient(format!("{host} is limiting requests; trying again shortly"))),
        500..=599 => Some(Failure::Transient(format!("{host} failed ({}); trying again shortly", a.status))),
        _ => None,
    }
}

/// A new salt: 32 random hex digits.
fn new_salt() -> Result<String, CoreError> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| CoreError::new(ErrorKind::Internal, e.to_string()))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// What a cloud agent is told about the mailbox (the snapshot's
/// `mailbox.about`): whose it is, the name and address it sends as, the
/// service it sends through and that service's limits. Unlike mailbox
/// mode's text it never names the user's own email (an unverified
/// AgentMail mailbox writes only there) or any address but the mailbox's
/// own, and says nothing of drafts on this Mac. `others` are the names of
/// the agents sharing its service account.
pub(crate) fn published_about(meta: &AgentMeta, others: &[String], verified: bool) -> String {
    let (service, site) = match meta.service {
        AgentService::Primitive => ("Primitive", "primitive.dev"),
        AgentService::AgentMail => ("AgentMail", "agentmail.to"),
    };
    let unverified = if meta.service == AgentService::AgentMail && !verified {
        " Until its service account is verified, it can write only to the user's own email, the one it was \
         created with; AgentMail refuses anyone else."
    } else {
        ""
    };
    let shared = match others {
        [] => String::new(),
        [one] => format!(" Another agent's mailbox, {one}'s, shares these limits: its sends count against them too."),
        many => format!(
            " The mailboxes of {} other agents ({}) share these limits: their sends count against them too.",
            many.len(),
            many.join(", ")
        ),
    };
    format!(
        "## This is an agent's mailbox\n\nThis mailbox, {address}, belongs to an agent called {name}, not to the \
         user. Mail sent from it goes out as {name} <{address}>. Write as {name}, on the user's behalf. It sends \
         through {service} ({site}) with the mailbox's own key; this server only serves its writing guide and \
         facts.{unverified} {limits}{shared}",
        address = meta.address,
        name = meta.name,
        limits = service_limits(meta.service),
    )
}

/// A digest of what a snapshot says, its version and time aside.
fn digest(snapshot: &Snapshot) -> String {
    let plain = Snapshot { version: 0, published_at: 0, ..snapshot.clone() };
    let mut h = std::collections::hash_map::DefaultHasher::new();
    plain.to_json().unwrap_or_default().hash(&mut h);
    format!("{:016x}", h.finish())
}

/// A scope in words with its people counted, not named.
fn scope_summary(s: &writing_guide::Scope) -> String {
    let mut parts = Vec::new();
    if !s.groups.is_empty() {
        parts.push(format!("for {}", s.groups.join(", ")));
    }
    match s.people.len() {
        0 => {}
        1 => parts.push("to 1 person".to_owned()),
        n => parts.push(format!("to {n} people")),
    }
    if !s.message_types.is_empty() {
        parts.push(format!("in {}", s.message_types.join(", ")));
    }
    if !s.languages.is_empty() {
        parts.push(format!("when writing in {}", s.languages.join(" or ")));
    }
    parts.join("; ")
}

fn preview_of(snapshot: &Snapshot, facts_kept: u32) -> RulesPreview {
    RulesPreview {
        address: snapshot.mailbox.address.clone(),
        name: snapshot.mailbox.name.clone(),
        entries: snapshot
            .entries
            .iter()
            .map(|e| RulesPreviewEntry {
                kind: match e.kind {
                    writing_guide::Kind::Rule => GuideKind::Rule,
                    writing_guide::Kind::Guideline => GuideKind::Guideline,
                    writing_guide::Kind::Fact => GuideKind::Fact,
                },
                statement: e.statement.clone(),
                scope: scope_summary(&e.scope),
                has_check: e.check.is_some(),
            })
            .collect(),
        facts: snapshot
            .facts
            .iter()
            .map(|f| RulesPreviewFact {
                category: f.category.clone(),
                label: f.label.clone(),
                ask_before_using: f.ask_before_using,
            })
            .collect(),
        audiences: snapshot
            .audiences
            .groups
            .iter()
            .map(|g| RulesPreviewAudience {
                name: g.name.clone(),
                members: u32::try_from(g.members.len()).unwrap_or(u32::MAX),
            })
            .collect(),
        facts_kept,
    }
}

/// An RFC 3339 time from the server, in milliseconds.
fn millis(v: &Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|t| t.timestamp_millis())
}

/// An agent's name: 1 to 100 characters on one line.
fn agent_name(name: &str) -> Result<String, CoreError> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.chars().count() > 100 {
        return Err(CoreError::new(ErrorKind::InvalidInput, "give the agent a name of up to 100 characters"));
    }
    Ok(name)
}

fn rules_agent(v: &Value) -> Option<RulesAgent> {
    Some(RulesAgent {
        id: v["id"].as_str()?.to_owned(),
        name: v["name"].as_str()?.to_owned(),
        kind: if v["kind"] == "oauth" { RulesAgentKind::Connector } else { RulesAgentKind::Token },
        client_name: v["client_name"].as_str().map(str::to_owned),
        created_at: millis(&v["created_at"])?,
        revoked_at: millis(&v["revoked_at"]),
        last_used_at: millis(&v["last_used_at"]),
    })
}

/// The MCP resource a server's protected resource metadata names, if it is
/// one an agent can be sent to: `https://…/mcp`, or `http://` to this Mac.
fn mcp_resource(v: &Value) -> Option<String> {
    let resource = v["resource"].as_str()?;
    let server = parse_server(resource.strip_suffix("/mcp")?).ok()?;
    (server.endpoint(&["mcp"]).as_str() == resource).then(|| resource.to_owned())
}

impl Core {
    fn rules_record_path(&self, account_id: &str) -> PathBuf {
        record_path(&self.data_path(), account_id)
    }

    fn rules_record(&self, account_id: &str) -> Option<Record> {
        let _guard = self.rules.records.lock().unwrap_or_else(|e| e.into_inner());
        read_record(&self.rules_record_path(account_id))
    }

    /// Change an account's record, if it has one.
    fn update_rules_record(&self, account_id: &str, change: impl FnOnce(&mut Record)) -> Result<(), CoreError> {
        let _guard = self.rules.records.lock().unwrap_or_else(|e| e.into_inner());
        let path = self.rules_record_path(account_id);
        let Some(mut record) = read_record(&path) else { return Ok(()) };
        change(&mut record);
        write_record(&path, &record)
    }

    fn rules_event(&self, account_id: &str) {
        self.events.for_account(Some(account_id.to_owned())).emit(CoreEvent::RulesPublicationChanged);
    }

    /// The agent mailboxes that publish: `account`, if it does, or (with
    /// `None`) every one.
    fn publishing(&self, account: Option<String>) -> Vec<String> {
        let enabled = |id: &str| self.rules_record(id).is_some_and(|r| r.enabled);
        match account {
            Some(id) => enabled(&id).then_some(id).into_iter().collect(),
            None => std::fs::read_dir(accounts_dir(&self.data_path()))
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
                        .filter(|id| enabled(id))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// The guide or facts changed: push what changed for `account` (or
    /// every publishing mailbox), once changes stop for a moment.
    pub(crate) fn rules_changed(&self, account: Option<String>) {
        if self.headless {
            return;
        }
        let targets = self.publishing(account);
        if targets.is_empty() {
            return;
        }
        for target in &targets {
            self.rules.set_pending(target, true);
        }
        if let Some(tx) = self.rules_worker() {
            let _ = tx.send(if targets.len() == 1 { targets.into_iter().next() } else { None });
        }
    }

    /// The publisher's queue, starting the worker the first time.
    fn rules_worker(&self) -> Option<mpsc::UnboundedSender<Option<String>>> {
        if let Some(tx) = self.rules.tx.get() {
            return Some(tx.clone());
        }
        let _guard = self.rules.starting.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = self.rules.tx.get() {
            return Some(tx.clone());
        }
        let me = self.me.upgrade()?;
        let (tx, rx) = mpsc::unbounded_channel();
        runtime::runtime().spawn(worker(Arc::downgrade(&me), rx));
        let _ = self.rules.tx.set(tx.clone());
        Some(tx)
    }

    /// What would be published for `account_id` now, with its version and
    /// time unset and its addresses plain, and how many facts drafting uses
    /// there that stay on this Mac.
    pub(crate) async fn rules_snapshot(&self, account_id: &str) -> Result<(Snapshot, u32), CoreError> {
        let meta = self.agent_meta_or_err(account_id)?;
        self.store_for(account_id).await?;
        scoped(Some(account_id.to_owned()), async {
            let guide = self.list_guide_entries(vec![GuideStatus::Accepted]).await?;
            // F3 entries from before Facts had their own store are not used
            // (spec §14.11): they do not go either.
            let entries: Vec<writing_guide::Entry> =
                crate::guide_render::accepted(&guide).into_iter().filter(|e| e.category != "F3").collect();
            let audiences = crate::guide_render::confirmed(&self.list_audience_groups().await?);
            let guide_version = self.guide_version().await?;
            let categories = self.fact_categories().await?;
            let name = |key: &str| {
                categories.iter().find(|c| c.key == key).map_or_else(|| "Other".to_owned(), |c| c.name.clone())
            };
            let used: Vec<crate::facts::FactInfo> = self
                .list_facts(vec![FactStatus::Accepted])
                .await?
                .into_iter()
                .filter(|f| !f.overridden)
                .filter(|f| !categories.iter().any(|c| c.key == f.category && c.hidden))
                .filter(|f| f.use_ != crate::facts::FactUse::Never)
                .collect();
            let facts: Vec<writing_guide::Fact> =
                used.iter().filter(|f| f.share_with_cloud).filter_map(|f| shared(f, &name(&f.category))).collect();
            let kept = u32::try_from(used.len() - facts.len()).unwrap_or(u32::MAX);
            let verified = self.service_meta(&meta.service_account).is_ok_and(|s| s.verified);
            let about = published_about(&meta, &self.fellow_agents(account_id, &meta), verified);
            let snapshot = Snapshot {
                schema_version: writing_guide::SCHEMA_VERSION,
                version: 0,
                guide_version,
                published_at: 0,
                mailbox: writing_guide::Mailbox {
                    address: meta.address.clone(),
                    name: meta.name.clone(),
                    about: about.trim().to_owned(),
                },
                entries,
                audiences,
                facts,
            };
            Ok((snapshot, kept))
        })
        .await
    }

    /// Register `address` on `server`: the publisher token, answered once.
    async fn rules_register(
        &self,
        server: &Server,
        address: &str,
        registration_token: Option<&str>,
    ) -> Result<String, Failure> {
        let host = &server.host;
        let mut request =
            client(&self.rules)?.post(server.endpoint(&["v1", "mailboxes"])).json(&json!({ "address": address }));
        if let Some(token) = registration_token.map(str::trim).filter(|t| !t.is_empty()) {
            request = request.bearer_auth(token);
        }
        let a = send(request, host).await?;
        if let Some(f) = transient(host, &a) {
            return Err(f);
        }
        match a.status {
            200 | 201 => {
                a.body["publisher_token"].as_str().filter(|t| !t.is_empty()).map(str::to_owned).ok_or_else(|| {
                    Failure::Final(format!("{host} did not answer with a publisher token; is it a rules server?"))
                })
            }
            409 => Err(Failure::Final(format!(
                "{address} is already registered on {host}. If this Mac registered it before and lost its token, \
                 the server's operator can forget it (kaluta-rules forget-mailbox), and then you can publish again."
            ))),
            401 if registration_token.is_some() => {
                Err(Failure::Final(format!("{host} did not accept that registration token.")))
            }
            401 => Err(Failure::Final(format!(
                "{host} asks for its registration token to add a mailbox; its operator has it."
            ))),
            404 | 405 => Err(Failure::Final(format!("{host} does not look like a rules server."))),
            _ => Err(Failure::Final(format!("{host} did not add the mailbox: {}", a.says()))),
        }
    }

    /// Push `account_id`'s snapshot if it publishes and what would go
    /// changed (or always, with `force`). `Ok(true)` when the server took a
    /// new version. The outcome is recorded for Settings either way.
    pub(crate) async fn rules_push(&self, account_id: &str, force: bool) -> Result<bool, Failure> {
        let _guard = self.rules.pushing.lock().await;
        let outcome = self.rules_push_locked(account_id, force).await;
        match &outcome {
            Ok(_) => self.rules.set_pending(account_id, false),
            Err(f) => {
                if matches!(f, Failure::Final(_)) {
                    self.rules.set_pending(account_id, false);
                }
                let message = f.message().to_owned();
                let _ = self.update_rules_record(account_id, |r| r.error = Some(message));
                tracing::warn!(
                    account = account_id,
                    transient = matches!(f, Failure::Transient(_)),
                    "rules push failed"
                );
            }
        }
        self.rules_event(account_id);
        outcome
    }

    async fn rules_push_locked(&self, account_id: &str, force: bool) -> Result<bool, Failure> {
        let Some(mut record) = self.rules_record(account_id).filter(|r| r.enabled && !r.sample) else {
            return Ok(false);
        };
        let server = parse_server(&record.server_url)?;
        let host = server.host.clone();
        let (plain, _) = self.rules_snapshot(account_id).await?;
        let address = plain.mailbox.address.clone();
        let content = digest(&plain);
        let mut encrypt = record.encrypts();
        // A publication from before encryption, or one turned to it, goes
        // encrypted now even if nothing changed.
        let migrating = encrypt && record.key_id.is_none();
        if !force && !migrating && record.error.is_none() && record.content.as_deref() == Some(content.as_str()) {
            return Ok(false);
        }
        let key = keys::rules_publish_token(&server.key, account_id);
        let mut token = match self.secrets.get(key.clone())? {
            Some(t) => t,
            // The Keychain lost it: register again (the server may have
            // forgotten the mailbox too; if not, it says so).
            None => self.rules_register_again(account_id, &server, &address, &key, &mut record).await?,
        };
        let mut registered_again = false;
        let mut expected = (record.last_version > 0).then_some(record.last_version);
        let mut version = record.last_version + 1;
        let mut sealing: Option<encryption::Sealing> = None;
        for _ in 0..ATTEMPTS {
            let published_at = mail_sync::now_millis();
            let snapshot = Snapshot { version, published_at, ..plain.clone() }.hashed(&record.salt);
            let body = if encrypt {
                if sealing.is_none() {
                    sealing = Some(self.rules_sealing(account_id, &server, &address, &token).await?);
                }
                sealing.as_ref().map_or_else(|| Err(Failure::Final("not sealed".into())), |s| s.body(&snapshot))?
            } else {
                snapshot.to_json().map_err(|e| Failure::Final(e.to_string()))?
            };
            let mut request = client(&self.rules)?
                .put(server.endpoint(&["v1", "mailboxes", &address, "snapshot"]))
                .bearer_auth(&token)
                .header(CONTENT_TYPE, "application/json")
                .body(body);
            if let Some(v) = expected {
                request = request.header(IF_MATCH, format!("\"{v}\""));
            }
            let a = send(request, &host).await?;
            if let Some(f) = transient(&host, &a) {
                return Err(f);
            }
            match a.status {
                200 | 201 => {
                    let key_id = sealing.as_ref().map(|s| s.key_id.clone());
                    self.update_rules_record(account_id, |r| {
                        r.last_version = version;
                        r.published_at = Some(published_at);
                        r.error = None;
                        r.content = Some(content.clone());
                        r.salt.clone_from(&record.salt);
                        r.encrypt = Some(encrypt);
                        r.key_id = key_id;
                    })?;
                    tracing::info!(account = account_id, version, encrypted = encrypt, "published to the rules server");
                    return Ok(true);
                }
                // Another version is current (a push this Mac lost track
                // of): push above it, matching it.
                409 | 412 | 428 => {
                    let current = a.body["current_version"].as_i64();
                    expected = current;
                    version = version.max(current.unwrap_or(0) + 1);
                }
                // The server forgot the mailbox: register it again, once. A
                // server answers an address it does not have as it answers a
                // wrong token (401; 404 from older ones), so that nobody
                // learns which are registered: registering again tells.
                404 | 401 if !registered_again => {
                    registered_again = true;
                    token = match self.rules_register_again(account_id, &server, &address, &key, &mut record).await {
                        Ok(t) => t,
                        // Still registered: the token is what it refuses.
                        Err(Failure::Final(_)) if a.status == 401 => return Err(token_refused(&host, &address)),
                        Err(f) => return Err(f),
                    };
                    expected = None;
                    // A new registration has no agents to wrap for.
                    sealing = None;
                }
                401 => return Err(token_refused(&host, &address)),
                // The server requires encryption: this publication encrypts
                // from now on.
                422 if !encrypt && a.body["error"] == "encryption_required" => {
                    encrypt = true;
                    self.update_rules_record(account_id, |r| r.encrypt = Some(true))?;
                }
                // A server from before encryption reads the push as a
                // plaintext snapshot: without its schema, or (the envelope
                // now naming one) without a snapshot's fields.
                422 if encrypt && (a.says().contains("schema_version") || a.says().contains("not a snapshot")) => {
                    return Err(Failure::Final(format!(
                        "{host} runs an older kaluta-rules that cannot keep the guide encrypted. Its operator can \
                         update it; or stop publishing and publish again with encryption off."
                    )));
                }
                422 => return Err(Failure::Final(format!("{host} refused the snapshot: {}", a.says()))),
                _ => return Err(Failure::Final(format!("{host} did not take the snapshot: {}", a.says()))),
            }
        }
        Err(Failure::Final(format!("{host} kept refusing the snapshot's version; try Publish Now")))
    }

    /// Register again (a lost token, a server that forgot the mailbox):
    /// a new token in the Keychain and a new salt.
    async fn rules_register_again(
        &self,
        account_id: &str,
        server: &Server,
        address: &str,
        key: &str,
        record: &mut Record,
    ) -> Result<String, Failure> {
        let token = self.rules_register(server, address, None).await?;
        self.secrets.set(key.to_owned(), token.clone())?;
        record.salt = new_salt()?;
        let salt = record.salt.clone();
        self.update_rules_record(account_id, |r| {
            r.salt = salt;
            r.content = None;
        })?;
        tracing::info!(account = account_id, "registered again on the rules server");
        Ok(token)
    }

    /// An agent mailbox is being removed: ask the server it publishes to
    /// to forget it (best effort: a server that cannot be reached keeps
    /// its copy), and forget the publisher token.
    pub(crate) async fn rules_forget(&self, account_id: &str) {
        let Some(record) = self.rules_record(account_id).filter(|r| !r.sample) else { return };
        let Ok(server) = parse_server(&record.server_url) else { return };
        let key = keys::rules_publish_token(&server.key, account_id);
        if let (Ok(Some(token)), Some(meta), Ok(client)) =
            (self.secrets.get(key.clone()), self.agent_meta(account_id), client(&self.rules))
        {
            let request = client.delete(server.endpoint(&["v1", "mailboxes", &meta.address])).bearer_auth(token);
            let host = server.host.clone();
            let _ = runtime::run(async move { send(request, &host).await.map_err(Failure::into_error) }).await;
        }
        let _ = self.secrets.delete(key);
        self.rules_forget_keys(account_id);
        self.rules.set_pending(account_id, false);
    }

    /// Call the publisher's API for `account_id` on its rules server: the
    /// method, the path under `/v1/mailboxes/<address>/` and a JSON body.
    /// The answer when it is a success; otherwise why not, in words.
    async fn rules_manage(
        &self,
        account_id: &str,
        method: reqwest::Method,
        segments: &[&str],
        body: Option<Value>,
    ) -> Result<Value, CoreError> {
        let record = self.rules_record(account_id).filter(|r| !r.sample).ok_or_else(|| {
            CoreError::new(ErrorKind::InvalidInput, "this mailbox does not publish to a rules server; publish it first")
        })?;
        let server = parse_server(&record.server_url)?;
        let address = self.agent_meta_or_err(account_id)?.address;
        let token = self.secrets.get(keys::rules_publish_token(&server.key, account_id))?.ok_or_else(|| {
            CoreError::new(
                ErrorKind::InvalidInput,
                format!("this Mac has lost its publisher token for {}; Publish Now registers again", server.host),
            )
        })?;
        let mut path = vec!["v1", "mailboxes", address.as_str()];
        path.extend_from_slice(segments);
        let client = client(&self.rules).map_err(Failure::into_error)?;
        let mut request = client.request(method, server.endpoint(&path)).bearer_auth(token);
        if let Some(b) = body {
            request = request.json(&b);
        }
        let host = server.host.clone();
        let a = runtime::run(async move { send(request, &host).await.map_err(Failure::into_error) }).await?;
        let host = &server.host;
        if let Some(f) = transient(host, &a) {
            return Err(f.into_error());
        }
        match a.status {
            200..=299 => Ok(a.body),
            401 => Err(CoreError::new(
                ErrorKind::InvalidInput,
                format!(
                    "{host} does not accept this Mac's publisher token for {address}: it may have forgotten the \
                     mailbox, which Publish Now registers again; if not, its operator can forget it \
                     (kaluta-rules forget-mailbox), and then publish again."
                ),
            )),
            404 if a.body["error"] == "not_registered" => Err(CoreError::new(
                ErrorKind::InvalidInput,
                format!("{host} has forgotten {address}; Publish Now registers it again"),
            )),
            404 if a.body["error"] == "not_found" => {
                Err(CoreError::new(ErrorKind::NotFound, format!("{host} has no such agent for {address}")))
            }
            404 | 405 => Err(CoreError::new(
                ErrorKind::InvalidInput,
                format!("{host} does not offer this; it may run an older kaluta-rules"),
            )),
            _ => Err(CoreError::new(ErrorKind::InvalidInput, format!("{host}: {}", a.says()))),
        }
    }

    fn rules_status(&self, account_id: &str) -> Option<RulesPublication> {
        self.rules_record(account_id).map(|r| RulesPublication {
            server_url: r.server_url,
            enabled: r.enabled,
            version: (r.last_version > 0).then_some(r.last_version),
            published_at: r.published_at,
            error: r.error,
            pending: r.enabled && self.rules.is_pending(account_id),
            encrypted: r.key_id.is_some(),
            unreadable_reports: r.unreadable_reports.saturating_add(self.rules.failing_reports(account_id)),
        })
    }
}

/// A server that refuses this Mac's publisher token for a mailbox it still
/// has.
fn token_refused(host: &str, address: &str) -> Failure {
    Failure::Final(format!(
        "{host} no longer accepts this Mac's publisher token for {address}. The server's operator can forget the \
         mailbox (kaluta-rules forget-mailbox); then publish again."
    ))
}

/// Push each publishing mailbox a moment after its changes stop.
async fn worker(core: Weak<Core>, mut rx: mpsc::UnboundedReceiver<Option<String>>) {
    // Account → (its first change not pushed, when to push).
    let mut due: HashMap<String, (Instant, Instant)> = HashMap::new();
    loop {
        let next = due.values().map(|(_, at)| *at).min();
        let wake = async move {
            match next {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            message = rx.recv() => {
                let Some(target) = message else { return };
                let Some(core) = core.upgrade() else { return };
                let now = Instant::now();
                let debounce = RulesState::wait(&core.rules.debounce_ms, DEBOUNCE);
                let max_wait = RulesState::wait(&core.rules.max_wait_ms, MAX_WAIT);
                for account in core.publishing(target) {
                    core.rules.set_pending(&account, true);
                    let first = due.get(&account).map_or(now, |(first, _)| *first);
                    due.insert(account, (first, (now + debounce).min(first + max_wait)));
                }
            }
            () = wake => {
                let now = Instant::now();
                let ready: Vec<String> =
                    due.iter().filter(|(_, (_, at))| *at <= now).map(|(account, _)| account.clone()).collect();
                for account in ready {
                    due.remove(&account);
                    let Some(core) = core.upgrade() else { return };
                    if let Err(Failure::Transient(_)) = core.rules_push(&account, false).await {
                        let retry = RulesState::wait(&core.rules.retry_ms, RETRY);
                        let at = Instant::now() + retry;
                        due.insert(account, (at, at));
                    }
                }
            }
        }
    }
}

#[uniffi::export]
impl Core {
    /// Exactly what publishing `account_id` (an agent mailbox) to a rules
    /// server sends now, for the publish sheet (spec §10.6).
    pub async fn rules_preview(&self, account_id: String) -> Result<RulesPreview, CoreError> {
        let (snapshot, kept) = self.rules_snapshot(&account_id).await?;
        Ok(preview_of(&snapshot, kept))
    }

    /// Start publishing an agent mailbox to the rules server at
    /// `server_url`: register it there the first time (with the server's
    /// registration token, if it asks for one) and keep the publisher token
    /// in the Keychain, then push now and on every change. A failed first
    /// push is in the status this returns; a failed registration is an
    /// error, and nothing is kept.
    pub async fn rules_publish_start(
        self: Arc<Self>,
        account_id: String,
        server_url: String,
        registration_token: Option<String>,
        encrypt: bool,
    ) -> Result<RulesPublication, CoreError> {
        if self.headless {
            return Err(CoreError::new(ErrorKind::PermissionDenied, "Kaluta publishes; open it"));
        }
        let meta = self.agent_meta_or_err(&account_id)?;
        let server = parse_server(&server_url)?;
        let core = self.clone();
        runtime::run(async move {
            let existing = core.rules_record(&account_id);
            if let Some(r) = existing.as_ref().filter(|r| r.enabled && r.server_url != server.url) {
                return Err(CoreError::new(
                    ErrorKind::InvalidInput,
                    format!("this mailbox publishes to {} already; stop publishing there first", r.server_url),
                ));
            }
            let key = keys::rules_publish_token(&server.key, &account_id);
            let same = existing.as_ref().filter(|r| r.server_url == server.url);
            let has_token = core.secrets.get(key.clone())?.is_some();
            let record = match same {
                Some(r) if has_token => {
                    Record { enabled: true, encrypt: Some(encrypt), unreadable_reports: 0, ..r.clone() }
                }
                _ => {
                    let _guard = core.rules.pushing.lock().await;
                    let token = core
                        .rules_register(&server, &meta.address, registration_token.as_deref())
                        .await
                        .map_err(Failure::into_error)?;
                    core.secrets.set(key, token)?;
                    // A server this mailbox published to before: its token
                    // is no use any more (the server keeps its copy).
                    if let Some(old) = existing.as_ref().filter(|r| r.server_url != server.url)
                        && let Ok(old_server) = parse_server(&old.server_url)
                    {
                        let _ = core.secrets.delete(keys::rules_publish_token(&old_server.key, &account_id));
                    }
                    Record {
                        server_url: server.url.clone(),
                        enabled: true,
                        salt: new_salt()?,
                        // Versions only go up, even on a server that forgot.
                        last_version: same.map_or(0, |r| r.last_version),
                        encrypt: Some(encrypt),
                        ..Record::default()
                    }
                }
            };
            {
                let _guard = core.rules.records.lock().unwrap_or_else(|e| e.into_inner());
                write_record(&core.rules_record_path(&account_id), &record)?;
            }
            tracing::info!(account = account_id.as_str(), "publishing to a rules server");
            let _ = core.rules_push(&account_id, true).await;
            // Later changes go through the worker.
            core.rules_worker();
            core.rules_status(&account_id).ok_or_else(|| CoreError::new(ErrorKind::Internal, "the record is gone"))
        })
        .await
    }

    /// Stop publishing an agent mailbox. With `remove_from_server`, the
    /// server forgets it too (its snapshots and agent tokens), and this Mac
    /// its publisher token; without, agents keep reading the last version.
    pub async fn rules_publish_stop(&self, account_id: String, remove_from_server: bool) -> Result<(), CoreError> {
        let Some(record) = self.rules_record(&account_id) else { return Ok(()) };
        if !remove_from_server {
            self.update_rules_record(&account_id, |r| r.enabled = false)?;
            self.rules.set_pending(&account_id, false);
            self.rules_event(&account_id);
            return Ok(());
        }
        let server = parse_server(&record.server_url)?;
        let key = keys::rules_publish_token(&server.key, &account_id);
        let address = self.agent_meta_or_err(&account_id)?.address;
        if let Some(token) = self.secrets.get(key.clone())? {
            let client = client(&self.rules).map_err(Failure::into_error)?;
            let request = client.delete(server.endpoint(&["v1", "mailboxes", &address])).bearer_auth(token);
            let host = server.host.clone();
            let answer = runtime::run(async move { send(request, &host).await.map_err(Failure::into_error) }).await?;
            // Gone already (404, from older servers) is what was asked for.
            // A 401 is a mailbox gone or a token refused, which the server
            // does not say apart: nothing is removed here unless it is.
            if answer.status == 401 {
                return Err(CoreError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "{} does not accept this Mac's publisher token for {address}: it may have forgotten the \
                         mailbox already. Stop publishing without removing it, or ask the server's operator to \
                         forget it (kaluta-rules forget-mailbox).",
                        server.host
                    ),
                ));
            }
            if !matches!(answer.status, 200 | 204 | 404) {
                return Err(CoreError::new(
                    ErrorKind::Network,
                    format!("{} did not remove the mailbox: {}", server.host, answer.says()),
                ));
            }
        }
        self.secrets.delete(key)?;
        self.rules_forget_keys(&account_id);
        {
            let _guard = self.rules.records.lock().unwrap_or_else(|e| e.into_inner());
            let _ = std::fs::remove_file(self.rules_record_path(&account_id));
        }
        self.rules.set_pending(&account_id, false);
        self.rules_event(&account_id);
        Ok(())
    }

    /// An agent mailbox's publishing, or `None` when it never published.
    pub fn rules_publish_status(&self, account_id: String) -> Option<RulesPublication> {
        self.rules_status(&account_id)
    }

    /// Push now, even when nothing changed (a new version), and say how it
    /// went.
    pub async fn rules_publish_now(&self, account_id: String) -> Result<RulesPublication, CoreError> {
        let enabled = self.rules_record(&account_id).is_some_and(|r| r.enabled);
        if !enabled {
            return Err(CoreError::new(ErrorKind::InvalidInput, "this mailbox does not publish to a rules server"));
        }
        let core = self.me.upgrade().ok_or_else(|| CoreError::new(ErrorKind::Internal, "the core is closing"))?;
        let id = account_id.clone();
        runtime::run(async move {
            let _ = core.rules_push(&id, true).await;
            // And the reports cloud agents filed since the last sync.
            if let Err(f) = core.rules_pull_reports(&id).await {
                tracing::warn!(
                    account = id.as_str(),
                    transient = matches!(f, Failure::Transient(_)),
                    "reports not pulled"
                );
            }
            Ok(())
        })
        .await?;
        self.rules_status(&account_id).ok_or_else(|| CoreError::new(ErrorKind::Internal, "the record is gone"))
    }

    /// At launch: push what changed while Kaluta was closed (or failed
    /// last time) for every mailbox that publishes.
    pub fn resume_rules_publishing(&self) {
        self.rules_changed(None);
    }

    /// How agents reach `account_id`'s rules server, and whether it signs
    /// claude.ai connectors in (OAuth with connect codes): from its
    /// protected resource metadata, which needs no token. A server without
    /// a public URL has none.
    pub async fn rules_connect_info(&self, account_id: String) -> Result<RulesConnectInfo, CoreError> {
        let record = self.rules_record(&account_id).filter(|r| !r.sample).ok_or_else(|| {
            CoreError::new(ErrorKind::InvalidInput, "this mailbox does not publish to a rules server; publish it first")
        })?;
        let server = parse_server(&record.server_url)?;
        let client = client(&self.rules).map_err(Failure::into_error)?;
        let request = client.get(server.endpoint(&[".well-known", "oauth-protected-resource"]));
        let host = server.host.clone();
        let a = runtime::run(async move { send(request, &host).await.map_err(Failure::into_error) }).await?;
        if let Some(f) = transient(&server.host, &a) {
            return Err(f.into_error());
        }
        let resource = (a.status == 200).then(|| mcp_resource(&a.body)).flatten();
        Ok(match resource {
            Some(mcp_url) => {
                RulesConnectInfo { base_url: mcp_url.trim_end_matches("/mcp").to_owned(), mcp_url, oauth: true }
            }
            None => RulesConnectInfo {
                base_url: server.url.clone(),
                mcp_url: server.endpoint(&["mcp"]).to_string(),
                oauth: false,
            },
        })
    }

    /// Whether the rules server at `server_url` keeps snapshots encrypted at
    /// rest, requires it, or is too old to (spec §10.6), for the publish
    /// sheet: `GET /v1/server`, no token.
    pub async fn rules_server_encryption(&self, server_url: String) -> Result<RulesEncryption, CoreError> {
        let server = parse_server(&server_url)?;
        let client = client(&self.rules).map_err(Failure::into_error)?;
        let request = client.get(server.endpoint(&["v1", "server"]));
        let host = server.host.clone();
        let a = runtime::run(async move { send(request, &host).await.map_err(Failure::into_error) }).await?;
        if let Some(f) = transient(&server.host, &a) {
            return Err(f.into_error());
        }
        match (a.status, a.body["encryption"].as_str()) {
            (200, Some("required")) => Ok(RulesEncryption::Required),
            (200, Some(_)) => Ok(RulesEncryption::Optional),
            (404 | 405, _) => Ok(RulesEncryption::Unsupported),
            _ => Err(CoreError::new(ErrorKind::Network, format!("{} does not look like a rules server", server.host))),
        }
    }

    /// A one-time connect code for an agent named `name` (spec §10.6): the
    /// user types it on the rules server's sign-in page when connecting a
    /// claude.ai connector or cloud routine. It works once, for 10 minutes,
    /// and is shown only now. The server must have OAuth on (its operator
    /// set its public URL); if not, the error says so.
    pub async fn rules_connect_code_mint(
        &self,
        account_id: String,
        name: String,
    ) -> Result<RulesConnectCode, CoreError> {
        let name = agent_name(&name)?;
        let a = self
            .rules_manage(&account_id, reqwest::Method::POST, &["connect-codes"], Some(json!({ "name": name })))
            .await?;
        let code = a["code"].as_str().filter(|c| !c.is_empty());
        match (code, millis(&a["expires_at"])) {
            (Some(code), Some(expires_at)) => Ok(RulesConnectCode {
                code: code.to_owned(),
                name: a["name"].as_str().map_or(name, str::to_owned),
                expires_at,
            }),
            _ => Err(CoreError::new(ErrorKind::Network, "the rules server did not answer with a connect code")),
        }
    }

    /// A static agent token for an agent named `name`, for Claude Code, the
    /// Agent SDK or a script (`Authorization: Bearer`). Shown only now.
    pub async fn rules_agent_token_mint(&self, account_id: String, name: String) -> Result<RulesAgentToken, CoreError> {
        let name = agent_name(&name)?;
        let a = self
            .rules_manage(&account_id, reqwest::Method::POST, &["agent-tokens"], Some(json!({ "name": name })))
            .await?;
        // The server made its key at minting: give it the newest snapshot
        // key now, so it reads at once (best effort; the next sync retries).
        if let Some(core) = self.me.upgrade() {
            let account = account_id.clone();
            let _ = runtime::run(async move {
                if let Err(f) = core.rules_rewrap(&account).await {
                    tracing::warn!(account = account.as_str(), error = f.message(), "snapshot key not wrapped");
                }
                Ok::<_, CoreError>(())
            })
            .await;
        }
        match (a["id"].as_str(), a["token"].as_str()) {
            (Some(id), Some(token)) if !token.is_empty() => Ok(RulesAgentToken {
                id: id.to_owned(),
                name: a["name"].as_str().map_or(name, str::to_owned),
                token: token.to_owned(),
            }),
            _ => Err(CoreError::new(ErrorKind::Network, "the rules server did not answer with a token")),
        }
    }

    /// The agents connected to an agent mailbox on its rules server: static
    /// tokens and connectors signed in with a connect code, revoked ones
    /// included, oldest first.
    pub async fn rules_agents(&self, account_id: String) -> Result<Vec<RulesAgent>, CoreError> {
        let a = self.rules_manage(&account_id, reqwest::Method::GET, &["agent-tokens"], None).await?;
        // A connector that signed in since the last push needs the newest
        // snapshot key (spec §10.6); the sheet asks every few seconds.
        self.rules_rewrap_soon(&account_id, &a);
        Ok(a["agent_tokens"].as_array().map(|list| list.iter().filter_map(rules_agent).collect()).unwrap_or_default())
    }

    /// Revoke an agent, a token or a connector: it stops at its next
    /// request. Revoking twice is fine.
    pub async fn rules_agent_revoke(&self, account_id: String, agent_id: String) -> Result<(), CoreError> {
        if agent_id.is_empty() || !agent_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "not an agent id"));
        }
        self.rules_manage(&account_id, reqwest::Method::DELETE, &["agent-tokens", &agent_id], None).await?;
        Ok(())
    }

    /// Snapshots and previews: a publishing record as if pushed, with no
    /// server contacted.
    pub fn debug_set_rules_publication(
        &self,
        account_id: String,
        server_url: String,
        version: i64,
        published_at: i64,
        error: Option<String>,
    ) -> Result<(), CoreError> {
        let server = parse_server(&server_url)?;
        let record = Record {
            server_url: server.url,
            enabled: true,
            salt: new_salt()?,
            last_version: version,
            published_at: Some(published_at),
            error,
            content: None,
            sample: true,
            encrypt: Some(true),
            // Shown as published in plaintext, as the docs' pictures are.
            key_id: None,
            unreadable_reports: 0,
        };
        {
            let _guard = self.rules.records.lock().unwrap_or_else(|e| e.into_inner());
            write_record(&self.rules_record_path(&account_id), &record)?;
        }
        self.rules_event(&account_id);
        Ok(())
    }
}

mod encryption;
mod reports;
pub use reports::{CloudReportInfo, CloudReportMatch};

#[cfg(test)]
mod tests;
