//! Codex sessions (spec §9.4): a `codex app-server` per Kaluta session,
//! JSON-RPC over stdio (newline-delimited, no `"jsonrpc"` field), one
//! Codex thread, one turn at a time.
//!
//! Per session rather than per launch: the MCP server's `--session`
//! binding is process-level configuration.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_api::process::{Locator, child_path};
use agent_api::{
    AgentError, AgentEvent, AgentModel, AgentResult, AgentSession, EventSink, ModelFallback, SessionConfig, TurnInput,
    Usage, shorten, summarize_args,
};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::oneshot;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Features that would give the agent anything beyond Kaluta's tools.
pub const DISABLED_FEATURES: &[&str] = &[
    "shell_tool",
    "unified_exec",
    "apps",
    "browser_use",
    "computer_use",
    "in_app_browser",
    "image_generation",
    "plugins",
    "multi_agent",
    "hooks",
    "tool_suggest",
    "goals",
    "skill_search",
];

/// Every argument for `codex app-server`. The only place Codex's flags
/// live. Verified against codex-cli 0.145 with `--strict-config`.
pub fn server_args(cfg: &SessionConfig) -> Vec<String> {
    let toml_str = |s: &Path| format!("\"{}\"", s.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""));
    // Replacing the whole table drops the user's own MCP servers.
    let mcp = format!(
        "mcp_servers={{kaluta={{command={},args=[\"--socket\",{},\"--session\",\"{}\"],\
         default_tools_approval_mode=\"auto\",tool_timeout_sec=900,startup_timeout_sec=20}}}}",
        toml_str(&cfg.mcp.shim_path),
        toml_str(&cfg.mcp.socket_path),
        cfg.session_id.as_str().replace('"', ""),
    );
    let mut args: Vec<String> = vec!["app-server".into(), "--listen".into(), "stdio://".into()];
    for c in [mcp.as_str(), "sandbox_mode=\"read-only\"", "approval_policy=\"never\"", "web_search=\"disabled\""] {
        args.extend(["-c".into(), c.into()]);
    }
    for f in DISABLED_FEATURES {
        args.extend(["--disable".into(), (*f).into()]);
    }
    args
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// The provider's last refused model and what was used instead, shared
/// by its sessions: the next one starts on the model that worked.
pub(crate) type Fallback = Arc<Mutex<Option<ModelFallback>>>;

/// Requests to one app-server, and the answers waiting for them.
#[derive(Clone)]
struct Rpc {
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    pending: Pending,
    next_id: Arc<AtomicU64>,
}

impl Rpc {
    fn new(stdin: ChildStdin) -> Self {
        Self {
            stdin: Arc::new(tokio::sync::Mutex::new(stdin)),
            pending: Arc::default(),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        write_line(&self.stdin, &json!({ "id": id, "method": method, "params": params })).await?;
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Codex stopped".into()),
            Err(_) => Err(format!("Codex did not answer {method}")),
        }
    }

    /// A response to one of our requests: hand it to its waiter.
    fn answer(&self, id: u64, msg: &Value) {
        let waiter = self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        if let Some(w) = waiter {
            let result = match msg.get("error") {
                Some(e) => Err(e["message"].as_str().unwrap_or("Codex returned an error").to_owned()),
                None => Ok(msg["result"].clone()),
            };
            let _ = w.send(result);
        }
    }

    /// The models the account can use, every page, hidden ones left out.
    async fn models(&self) -> Result<Vec<AgentModel>, String> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = self.request("model/list", json!({ "cursor": cursor })).await?;
            for m in page["data"].as_array().into_iter().flatten().filter(|m| m["hidden"] != true) {
                let Some(id) = m["model"].as_str().or(m["id"].as_str()).filter(|s| !s.is_empty()) else { continue };
                out.push(AgentModel {
                    id: id.to_owned(),
                    name: m["displayName"].as_str().filter(|s| !s.is_empty()).unwrap_or(id).to_owned(),
                    is_default: m["isDefault"] == true,
                });
            }
            cursor = page["nextCursor"].as_str().map(str::to_owned);
            if cursor.is_none() || out.len() > 200 {
                return Ok(out);
            }
        }
    }
}

struct TurnState {
    thread_id: String,
    turn_id: Option<String>,
    usage: Option<Usage>,
    last_error: Option<String>,
    /// The turn's prompt, for sending again on another model.
    prompt: String,
    /// This turn already moved to another model once.
    retried: bool,
    /// The model the session was asked for (`None`: Codex's own setting).
    requested: Option<String>,
}

pub(crate) struct CodexSession {
    child: Child,
    rpc: Rpc,
    running: Arc<AtomicBool>,
    state: Arc<Mutex<TurnState>>,
    sink: EventSink,
    fallback: Fallback,
}

pub(crate) async fn start(
    locator: &Locator,
    cfg: SessionConfig,
    sink: EventSink,
    fallback: Fallback,
) -> AgentResult<Box<dyn AgentSession>> {
    let binary = locator.find("codex").ok_or(AgentError::NotInstalled("Codex"))?;
    Ok(Box::new(CodexSession::spawn(&binary, cfg, sink, fallback).await?))
}

/// The models the signed-in account can use: a short-lived app-server,
/// asked and stopped.
pub(crate) async fn list_models(locator: &Locator) -> AgentResult<Vec<AgentModel>> {
    let binary = locator.find("codex").ok_or(AgentError::NotInstalled("Codex"))?;
    let mut child = Command::new(binary)
        .args(["app-server", "--listen", "stdio://"])
        .current_dir(std::env::temp_dir())
        .env("PATH", child_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| AgentError::Spawn(e.to_string()))?;
    let rpc = Rpc::new(child.stdin.take().expect("piped stdin"));
    let stdout = child.stdout.take().expect("piped stdout");
    let answers = rpc.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            if let (Some(id), None) = (msg.get("id").and_then(Value::as_u64), msg.get("method")) {
                answers.answer(id, &msg);
            }
        }
        answers.pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
    });
    let listed = async {
        rpc.request(
            "initialize",
            json!({ "clientInfo": { "name": "kaluta", "title": "Kaluta",
                                                            "version": env!("CARGO_PKG_VERSION") } }),
        )
        .await?;
        write_line(&rpc.stdin, &json!({ "method": "initialized" })).await?;
        rpc.models().await
    }
    .await;
    let _ = child.start_kill();
    listed.map_err(AgentError::Protocol)
}

/// The text to show for a Codex error. Codex passes the API's JSON body
/// on; its message is what matters. A refused model says which and where
/// to choose another.
pub fn readable_error(raw: &str, requested: Option<&str>) -> String {
    let message = inner_message(raw);
    match refused_model(&message) {
        Some(model) if requested.is_some() => format!(
            "The model chosen in Kaluta's Settings › Agents, {model}, isn't available to your Codex account. \
             Choose another there."
        ),
        Some(model) => format!(
            "Codex is set to use {model}, which your account can't use. Choose a model in Kaluta's \
             Settings › Agents, or change it in Codex's own settings (~/.codex/config.toml)."
        ),
        None => message,
    }
}

/// The `message` inside a JSON error body, else the text itself.
fn inner_message(raw: &str) -> String {
    raw.find('{')
        .and_then(|i| serde_json::from_str::<Value>(&raw[i..]).ok())
        .and_then(|v| v["error"]["message"].as_str().or(v["message"].as_str()).map(str::to_owned))
        .unwrap_or_else(|| raw.trim().to_owned())
}

/// The model an error says the account cannot use: its quoted name, or
/// "its model" when the error names none. `None`: another error.
pub fn refused_model(message: &str) -> Option<String> {
    let message = inner_message(message);
    let lower = message.to_lowercase();
    let refused = lower.contains("model")
        && ["not supported", "does not exist", "not available", "not found", "do not have access", "don't have access"]
            .iter()
            .any(|p| lower.contains(p));
    if !refused {
        return None;
    }
    let quoted = ['\'', '"', '`'].iter().find_map(|q| {
        let start = message.find(*q)? + 1;
        let len = message[start..].find(*q)?;
        Some(message[start..start + len].to_owned())
    });
    Some(quoted.filter(|n| !n.is_empty() && !n.contains(' ')).unwrap_or_else(|| "its model".to_owned()))
}

/// The account's default model, else any other than the refused one.
fn replacement(models: &[AgentModel], refused: &str) -> Option<String> {
    models
        .iter()
        .find(|m| m.is_default && m.id != refused)
        .or_else(|| models.iter().find(|m| m.id != refused))
        .map(|m| m.id.clone())
}

/// The turn's model was refused: the same prompt again on the account's
/// default, once. Remembered, so the provider's next sessions start on it.
async fn retry_on_default(
    rpc: &Rpc,
    state: &Mutex<TurnState>,
    fallback: &Fallback,
    refused: String,
) -> Result<Value, String> {
    let models = rpc.models().await?;
    let used = replacement(&models, &refused).ok_or_else(|| "Codex offers no other model".to_owned())?;
    let (thread, prompt, requested) = {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        s.retried = true;
        s.usage = None;
        s.last_error = None;
        s.turn_id = None;
        (s.thread_id.clone(), s.prompt.clone(), s.requested.clone())
    };
    tracing::warn!(refused = %refused, used = %used, "Codex refused its model; using the account's default");
    *fallback.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(ModelFallback { requested, refused, used: used.clone() });
    rpc.request(
        "turn/start",
        json!({ "threadId": thread, "input": [{ "type": "text", "text": prompt }], "model": used }),
    )
    .await
}

async fn write_line(stdin: &tokio::sync::Mutex<ChildStdin>, message: &Value) -> Result<(), String> {
    let mut line = message.to_string();
    line.push('\n');
    let mut w = stdin.lock().await;
    w.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
    w.flush().await.map_err(|e| e.to_string())
}

impl CodexSession {
    async fn spawn(binary: &PathBuf, cfg: SessionConfig, sink: EventSink, fallback: Fallback) -> AgentResult<Self> {
        let mut child = Command::new(binary)
            .args(server_args(&cfg))
            .current_dir(&cfg.working_dir)
            .env("PATH", child_path())
            .envs(cfg.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| AgentError::Spawn(e.to_string()))?;
        let rpc = Rpc::new(child.stdin.take().expect("piped stdin"));
        let stdout = child.stdout.take().expect("piped stdout");
        let running = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(TurnState {
            thread_id: String::new(),
            turn_id: None,
            usage: None,
            last_error: None,
            prompt: String::new(),
            retried: false,
            requested: cfg.model.clone(),
        }));

        let reader = Reader {
            rpc: rpc.clone(),
            running: running.clone(),
            state: state.clone(),
            sink: sink.clone(),
            fallback: fallback.clone(),
        };
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                reader.handle(&line).await;
            }
            reader.closed();
        });

        let session = Self { child, rpc, running, state, sink, fallback };
        session
            .request(
                "initialize",
                json!({
                    "clientInfo": { "name": "kaluta", "title": "Kaluta", "version": env!("CARGO_PKG_VERSION") },
                    "capabilities": { "experimentalApi": true },
                }),
            )
            .await
            .map_err(AgentError::Protocol)?;
        write_line(&session.rpc.stdin, &json!({ "method": "initialized" })).await.map_err(AgentError::Protocol)?;

        let instructions = std::fs::read_to_string(&cfg.system_prompt_file).ok();
        let mut params = json!({
            "cwd": cfg.working_dir,
            "sandbox": "read-only",
            "approvalPolicy": "never",
        });
        if let Some(text) = instructions {
            params["developerInstructions"] = json!(text);
        }
        // A model refused earlier this launch: straight to the one that worked.
        let remembered = session
            .fallback
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .filter(|f| f.requested == cfg.model)
            .map(|f| f.used);
        if let Some(model) = remembered.or_else(|| cfg.model.clone()) {
            params["model"] = json!(model);
        }
        let (method, params) = match &cfg.resume {
            Some(thread) => {
                params["threadId"] = json!(thread);
                ("thread/resume", params)
            }
            None => {
                params["ephemeral"] = json!(false);
                ("thread/start", params)
            }
        };
        let result = session.request(method, params).await.map_err(AgentError::Protocol)?;
        let thread_id = result["thread"]["id"]
            .as_str()
            .ok_or_else(|| AgentError::Protocol("Codex did not return a thread id".into()))?
            .to_owned();
        session.state.lock().unwrap_or_else(|e| e.into_inner()).thread_id = thread_id.clone();
        session.sink.emit(AgentEvent::SessionStarted { external_id: Some(thread_id) });
        Ok(session)
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.rpc.request(method, params).await
    }

    fn thread_id(&self) -> String {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).thread_id.clone()
    }
}

#[async_trait]
impl AgentSession for CodexSession {
    async fn send(&mut self, turn: TurnInput) -> AgentResult<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(AgentError::Busy);
        }
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            s.usage = None;
            s.last_error = None;
            s.turn_id = None;
            s.prompt = turn.full_prompt();
            s.retried = false;
        }
        self.sink.emit(AgentEvent::TurnStarted);
        let params = json!({
            "threadId": self.thread_id(),
            "input": [{ "type": "text", "text": turn.full_prompt() }],
        });
        let mut started = self.request("turn/start", params).await;
        // Refused when the turn starts: the account's default instead.
        if let Err(e) = &started
            && let Some(refused) = refused_model(e)
        {
            started = retry_on_default(&self.rpc, &self.state, &self.fallback, refused).await.map_err(|_| e.clone());
        }
        match started {
            Ok(result) => {
                if let Some(id) = result["turn"]["id"].as_str() {
                    self.state.lock().unwrap_or_else(|e| e.into_inner()).turn_id = Some(id.to_owned());
                }
                Ok(())
            }
            Err(e) => {
                self.running.store(false, Ordering::SeqCst);
                let requested = self.state.lock().unwrap_or_else(|e| e.into_inner()).requested.clone();
                let message = readable_error(&e, requested.as_deref());
                self.sink.emit(AgentEvent::TurnFailed { message: message.clone() });
                Err(AgentError::Protocol(message))
            }
        }
    }

    async fn cancel(&mut self) -> AgentResult<()> {
        if !self.running.load(Ordering::SeqCst) {
            return Ok(());
        }
        let turn_id = self.state.lock().unwrap_or_else(|e| e.into_inner()).turn_id.clone();
        if let Some(turn_id) = turn_id {
            self.request("turn/interrupt", json!({ "threadId": self.thread_id(), "turnId": turn_id }))
                .await
                .map_err(AgentError::Protocol)?;
        }
        Ok(())
    }

    fn external_id(&self) -> Option<String> {
        let id = self.thread_id();
        (!id.is_empty()).then_some(id)
    }

    async fn close(&mut self) {
        let _ = self.cancel().await;
        let _ = self.child.start_kill();
        self.sink.emit(AgentEvent::SessionEnded);
    }
}

struct Reader {
    rpc: Rpc,
    running: Arc<AtomicBool>,
    state: Arc<Mutex<TurnState>>,
    sink: EventSink,
    fallback: Fallback,
}

impl Reader {
    async fn handle(&self, line: &str) {
        let Ok(msg) = serde_json::from_str::<Value>(line) else { return };
        let method = msg["method"].as_str();
        match (msg.get("id"), method) {
            // A response to one of our requests.
            (Some(id), None) => {
                let Some(id) = id.as_u64() else { return };
                self.rpc.answer(id, &msg);
            }
            // A request from Codex (approvals, elicitations): Kaluta's own
            // permission engine gates inside the tools, so none are granted.
            (Some(id), Some(method)) => {
                tracing::warn!(method, "refused a Codex server request");
                let reply = json!({ "id": id, "error": { "code": -32601, "message": "Kaluta does not allow this" } });
                let _ = write_line(&self.rpc.stdin, &reply).await;
            }
            (None, Some(method)) => self.notification(method, &msg["params"]),
            (None, None) => {}
        }
    }

    fn notification(&self, method: &str, p: &Value) {
        {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if p["threadId"].as_str().is_some_and(|t| !s.thread_id.is_empty() && t != s.thread_id) {
                return;
            }
        }
        match method {
            "item/agentMessage/delta" => {
                if let Some(text) = p["delta"].as_str().filter(|t| !t.is_empty()) {
                    self.sink.emit(AgentEvent::TextDelta { text: text.to_owned() });
                }
            }
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                if let Some(text) = p["delta"].as_str().filter(|t| !t.is_empty()) {
                    self.sink.emit(AgentEvent::ThinkingDelta { text: text.to_owned() });
                }
            }
            "item/started" if p["item"]["type"] == "mcpToolCall" => {
                let item = &p["item"];
                self.sink.emit(AgentEvent::ToolCallStarted {
                    call_id: item["id"].as_str().unwrap_or_default().to_owned(),
                    tool: item["tool"].as_str().unwrap_or_default().to_owned(),
                    args_summary: summarize_args(&item["arguments"]),
                });
            }
            "item/completed" if p["item"]["type"] == "mcpToolCall" => {
                let item = &p["item"];
                let ok = item["status"] == "completed" && item["error"].is_null();
                let summary = if ok {
                    item["result"]["content"]
                        .as_array()
                        .map(|parts| parts.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join(" "))
                        .unwrap_or_default()
                } else {
                    item["error"]["message"].as_str().unwrap_or("failed").to_owned()
                };
                self.sink.emit(AgentEvent::ToolCallFinished {
                    call_id: item["id"].as_str().unwrap_or_default().to_owned(),
                    ok,
                    summary: shorten(&summary, 160),
                });
            }
            "thread/tokenUsage/updated" => {
                let last = &p["tokenUsage"]["last"];
                self.state.lock().unwrap_or_else(|e| e.into_inner()).usage = Some(Usage {
                    input_tokens: last["inputTokens"].as_u64().unwrap_or(0),
                    output_tokens: last["outputTokens"].as_u64().unwrap_or(0),
                    cached_input_tokens: last["cachedInputTokens"].as_u64().unwrap_or(0),
                });
            }
            "error" if p["willRetry"] == false => {
                let message = p["error"]["message"].as_str().unwrap_or("Codex reported an error").to_owned();
                self.state.lock().unwrap_or_else(|e| e.into_inner()).last_error = Some(message);
            }
            "turn/completed" => {
                let turn = &p["turn"];
                let (usage, error, retried, requested) = {
                    let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    (s.usage, s.last_error.clone(), s.retried, s.requested.clone())
                };
                let event = match turn["status"].as_str() {
                    Some("completed") => AgentEvent::TurnCompleted { usage, cost_usd: None },
                    Some("interrupted") => AgentEvent::TurnFailed { message: "The turn was stopped.".into() },
                    _ => {
                        let raw = turn["error"]["message"]
                            .as_str()
                            .map(str::to_owned)
                            .or(error)
                            .unwrap_or_else(|| "Codex stopped with an error.".into());
                        // The model was refused: the same turn again on the
                        // account's default, once; the turn goes on meanwhile.
                        if !retried && let Some(refused) = refused_model(&raw) {
                            self.retry(refused, raw);
                            return;
                        }
                        AgentEvent::TurnFailed { message: readable_error(&raw, requested.as_deref()) }
                    }
                };
                self.running.store(false, Ordering::SeqCst);
                self.sink.emit(event);
            }
            _ => {}
        }
    }

    fn retry(&self, refused: String, raw: String) {
        let (rpc, state, fallback, sink, running) =
            (self.rpc.clone(), self.state.clone(), self.fallback.clone(), self.sink.clone(), self.running.clone());
        tokio::spawn(async move {
            match retry_on_default(&rpc, &state, &fallback, refused).await {
                Ok(result) => {
                    if let Some(id) = result["turn"]["id"].as_str() {
                        state.lock().unwrap_or_else(|e| e.into_inner()).turn_id = Some(id.to_owned());
                    }
                }
                Err(_) => {
                    let requested = state.lock().unwrap_or_else(|e| e.into_inner()).requested.clone();
                    running.store(false, Ordering::SeqCst);
                    sink.emit(AgentEvent::TurnFailed { message: readable_error(&raw, requested.as_deref()) });
                }
            }
        });
    }

    /// The app-server exited: fail what is waiting.
    fn closed(&self) {
        self.rpc.pending.lock().unwrap_or_else(|e| e.into_inner()).clear();
        if self.running.swap(false, Ordering::SeqCst) {
            self.sink.emit(AgentEvent::TurnFailed { message: "Codex stopped unexpectedly.".into() });
        }
    }
}
