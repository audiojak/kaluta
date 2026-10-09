//! Agents outside OpenAGC on an agent mailbox (spec §10.1, mailbox mode):
//! `openagc-mcp --mailbox <address>`, spawned by Claude Code, Codex or a
//! script. With the app running the shim reaches this through the app's
//! socket; with the app closed it runs it in a headless core of its own
//! ([`Core::headless`]). Either way the calls go through the in-app tools:
//! the same permission engine, guide checks, recording (ADR 0013) and the
//! mailbox's *When Agents Send*.
//!
//! Only agent mailboxes are served: the user's own accounts are never
//! reachable by an outside agent.

use std::sync::Arc;

use agent_api::{EventSink, SessionId};
use agent_mcp::{MailboxTool, Outcome};
use permissions::{Scope, Tool};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::agent_mailbox::{AgentSendMode, agent_prompt};
use crate::registry::{AccountKind, IndexEntry, accounts_dir};
use crate::{Core, CoreError, ErrorKind, runtime};

/// Outside agents' session ids start with this; the activity log and the
/// app tell them apart by it.
pub(crate) const SESSION_PREFIX: &str = "outside-";

/// Where the running app writes its socket's path for mailbox mode, under
/// `<data dir>/run/`.
pub const OUTSIDE_SOCKET_FILE: &str = "mcp-socket";

/// What a send queued while the app is closed answers.
pub const QUEUED_MESSAGE: &str = "Queued. It goes out when OpenAGC next opens.";

/// A send that needs the user's approval while the app is closed.
pub(crate) const ASK_WHILE_CLOSED: &str = "This mailbox is set to ask before each send, and OpenAGC is closed, so \
                                           no one can approve it. Nothing was sent or queued. Ask the user to open \
                                           OpenAGC and try again, or to set When Agents Send to Send freely in the \
                                           mailbox's settings.";

/// `claude-code` from an MCP client's name: lowercase letters, digits and
/// dashes, at most 32.
fn client_label(client: &str) -> String {
    let dashed: String =
        client.trim().to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let label: String = dashed.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    let label: String = label.chars().take(32).collect();
    if label.is_empty() { "agent".into() } else { label }
}

/// How the activity log and approvals name a client.
fn client_name(label: &str) -> String {
    match label {
        "claude-code" | "claude" => "Claude Code".into(),
        l if l.starts_with("codex") => "Codex".into(),
        other => other.to_owned(),
    }
}

fn random_hex() -> String {
    let mut b = [0u8; 6];
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuideArgs {
    #[serde(default)]
    to: Vec<String>,
    message_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SendArgs {
    to: Vec<String>,
    #[serde(default)]
    cc: Vec<String>,
    subject: String,
    body_markdown: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyArgs {
    message_id: String,
    body_markdown: String,
    #[serde(default)]
    reply_all: bool,
}

fn args<T: serde::de::DeserializeOwned>(arguments: Value) -> Result<T, Outcome> {
    let arguments = if arguments.is_null() { json!({}) } else { arguments };
    serde_json::from_value(arguments).map_err(|e| Outcome::error("invalid_arguments", e.to_string()))
}

impl Core {
    /// The account of the agent mailbox at `address`. Anything else — one
    /// of the user's own accounts, an imported mailbox, an unknown address —
    /// is refused: outside agents get agent mailboxes only. Reads files
    /// only; no store is opened.
    pub fn outside_mailbox_account(&self, address: &str) -> Result<String, CoreError> {
        let wanted = address.trim().to_lowercase();
        let data_dir = self.data_path();
        // The index as the app wrote it; never rebuilt here.
        let index: Option<Vec<IndexEntry>> = std::fs::read(accounts_dir(&data_dir).join("index.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let agent = crate::agent_mailbox::service_account::agents_on_disk(&data_dir).into_iter().find(|(_, m)| {
            m.address.eq_ignore_ascii_case(&wanted)
                || m.managed_address.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(&wanted))
        });
        let listed = |id: &str| index.as_ref().is_none_or(|entries| entries.iter().any(|e| e.id == id));
        match agent {
            Some((id, _)) if listed(&id) => Ok(id),
            _ => {
                let own = index
                    .iter()
                    .flatten()
                    .find(|e| e.email.eq_ignore_ascii_case(&wanted) && e.kind != AccountKind::Agent);
                let message = match own {
                    Some(_) => format!(
                        "{address} is one of your own accounts, not an agent mailbox. Agents outside OpenAGC can \
                         use only agent mailboxes."
                    ),
                    None => format!(
                        "There is no agent mailbox {address} in OpenAGC (looked in {}). Create one in OpenAGC with \
                         Accounts › Create an Agent Mailbox…",
                        data_dir.display()
                    ),
                };
                Err(CoreError::new(ErrorKind::PermissionDenied, message))
            }
        }
    }

    /// Open an outside agent's session on the agent mailbox at `address`
    /// for the MCP client `client`. Returns the session id.
    pub async fn open_outside_session(self: &Arc<Self>, address: &str, client: &str) -> Result<String, CoreError> {
        let account = self.outside_mailbox_account(address)?;
        // Opens the store: in the headless core, read-only and only if its
        // schema is this build's.
        self.store_for(&account).await?;
        let label = client_label(client);
        let id = format!("{SESSION_PREFIX}{label}-{}", random_hex());
        // The app shows an outside agent's approvals (§10.4) through the
        // agent event stream; the headless core has no one to show them to.
        let sink = (!self.headless).then(|| EventSink::new(SessionId(id.clone()), self.agent_runtime().tx.clone()));
        self.agents.bind_account(&id, account.clone());
        self.agents.register(&id, Scope::Mailbox, sink);
        self.agents.with_session(&id, |s| {
            s.agent = Some(format!("outside:{label}"));
            s.last_prompt = format!("Written by {} outside OpenAGC, through openagc-mcp.", client_name(&label));
        });
        // The activity log needs the session's row. The headless core writes
        // it with the first send, so reading leaves the store untouched.
        if !self.headless {
            crate::registry::scoped(Some(account), self.record_outside_session(&id)).await?;
        }
        tracing::info!(session = %id, "outside agent connected");
        Ok(id)
    }

    async fn record_outside_session(&self, session: &str) -> Result<(), CoreError> {
        let db = self.db()?;
        let provider = self.agents.with_session(session, |s| s.agent.clone()).flatten().unwrap_or_default();
        let (uuid, now) = (session.to_owned(), mail_sync::now_millis());
        runtime::run(async move {
            Ok(db.write(move |tx| mail_store::agents::start_session(tx, &uuid, &provider, now)).await?)
        })
        .await
    }

    /// The outside agent disconnected: what it waits on is declined.
    pub async fn close_outside_session(self: &Arc<Self>, session: &str) {
        let account = self.agents.session_account(session);
        self.agents.approvals.reject_session(session);
        self.agents.unregister(session);
        crate::registry::scoped(account, async {
            let Ok(db) = self.db() else { return };
            // The headless core wrote a row only if it sent something.
            if self.headless && !db.writer_opened() {
                return;
            }
            let (uuid, now) = (session.to_owned(), mail_sync::now_millis());
            let _ = runtime::run(async move {
                Ok::<_, CoreError>(db.write(move |tx| mail_store::agents::end_session(tx, &uuid, now)).await?)
            })
            .await;
        })
        .await;
    }

    /// "Claude Code outside OpenAGC, as writer@…": who an outside session's
    /// proposals come from; `None` for the app's own sessions.
    pub(crate) fn outside_label(&self, session: &str) -> Option<String> {
        let rest = session.strip_prefix(SESSION_PREFIX)?;
        let label = rest.rsplit_once('-').map_or(rest, |(label, _)| label);
        let address = self.agents.session_account(session).and_then(|a| self.agent_meta(&a)).map(|m| m.address);
        Some(match address {
            Some(address) => format!("{} outside OpenAGC, as {address}", client_name(label)),
            None => format!("{} outside OpenAGC", client_name(label)),
        })
    }

    /// One mailbox-mode call.
    pub async fn call_outside(self: &Arc<Self>, session: &str, tool: MailboxTool, arguments: Value) -> Outcome {
        let Some(account) = self.agents.session_account(session).filter(|_| self.agents.has(session)) else {
            return Outcome::error("unknown_session", "this agent session has ended");
        };
        crate::registry::scoped(Some(account), async {
            match tool {
                MailboxTool::GuideRules => self.outside_guide(session, arguments).await,
                MailboxTool::FactsLookup => self.outside_read(session, Tool::FactsLookup, arguments).await,
                MailboxTool::Search => self.outside_read(session, Tool::Search, arguments).await,
                MailboxTool::GetThread => self.outside_read(session, Tool::GetThread, arguments).await,
                MailboxTool::Send => match args::<SendArgs>(arguments) {
                    Ok(a) => {
                        let draft = json!({ "to": a.to, "cc": a.cc, "subject": a.subject,
                                            "body_markdown": a.body_markdown });
                        self.outside_send(session, tool, draft).await
                    }
                    Err(e) => e,
                },
                MailboxTool::Reply => match args::<ReplyArgs>(arguments) {
                    Ok(a) => {
                        let draft = json!({ "reply_to_message_id": a.message_id, "reply_all": a.reply_all,
                                            "body_markdown": a.body_markdown });
                        self.outside_send(session, tool, draft).await
                    }
                    Err(e) => e,
                },
            }
        })
        .await
    }

    /// A read: the in-app tool, with its permission checks and audit.
    async fn outside_read(self: &Arc<Self>, session: &str, tool: Tool, arguments: Value) -> Outcome {
        crate::agents::tools::call(self, session, tool, arguments).await
    }

    /// `guide_rules`: the writing guide for the recipients and type, the
    /// mailbox's identity and limits, and how it sends.
    async fn outside_guide(self: &Arc<Self>, session: &str, arguments: Value) -> Outcome {
        let a: GuideArgs = match args(arguments.clone()) {
            Ok(a) => a,
            Err(e) => return e,
        };
        let Some(account) = self.effective_account_id() else {
            return Outcome::error("unknown_session", "this agent session has ended");
        };
        let Some(meta) = self.agent_meta(&account) else {
            return Outcome::error("not_found", "this agent mailbox was removed");
        };
        let action = self.record_named_action(session, "guide_rules", "read_only", &arguments, "allowed").await;
        let target = crate::guide_render::Target {
            recipients: a.to.iter().map(|t| t.trim().to_lowercase()).collect(),
            message_type: a.message_type,
            audiences: None,
        };
        let outcome = match self.render_guide(Some(target)).await {
            Ok(guide) => {
                let limits = self.agent_limits_text(&meta);
                let about = agent_prompt(&meta, &self.fellow_agents(&account, &meta), &limits);
                Outcome::json(json!({
                    "mailbox": meta.address,
                    "sends_as": format!("{} <{}>", meta.name, meta.address),
                    "about": about.trim(),
                    "send_mode": match meta.send_mode {
                        AgentSendMode::Freely => "send_freely",
                        AgentSendMode::Ask => "ask_before_each_send",
                    },
                    "writing_guide": guide.text,
                    "guide_version": guide.version,
                }))
            }
            Err(e) => Outcome::error("failed", e.to_string()),
        };
        let state = if matches!(outcome, Outcome::Ok { .. }) { "done" } else { "failed" };
        self.finish_action(action, state, Some(crate::agents::outcome_summary(&outcome))).await;
        outcome
    }

    /// `mail_send` / `mail_reply`: the in-app draft and send tools in one
    /// call. The draft is written and checked against the guide (what it
    /// breaks comes back as `guide_check`), recorded as AI-written (ADR
    /// 0013), then sent as the mailbox's setting says: at once, or once the
    /// user approves. With the app closed the send is queued in the
    /// outbox for the app; asking is refused, as no one can answer.
    async fn outside_send(self: &Arc<Self>, session: &str, tool: MailboxTool, draft: Value) -> Outcome {
        if self.headless && !self.agent_sends_freely() {
            return Outcome::error("needs_openagc", ASK_WHILE_CLOSED);
        }
        if self.headless
            && let Err(e) = self.record_outside_session(session).await
        {
            return Outcome::error("failed", e.to_string());
        }
        let created = crate::agents::tools::call(self, session, Tool::CreateDraft, draft).await;
        let (draft_id, guide_check) = match &created {
            Outcome::Ok { structured: Some(v), .. } => match v["draft_id"].as_i64() {
                Some(id) => (id, v.get("guide_check").cloned().unwrap_or_else(|| json!([]))),
                None => return Outcome::error("failed", "the draft was not saved"),
            },
            _ => return created,
        };
        let sent = crate::agents::tools::call(self, session, Tool::Send, json!({ "draft_id": draft_id })).await;
        match sent {
            Outcome::Ok { structured: Some(Value::Object(mut v)), .. } => {
                v.insert("guide_check".into(), guide_check);
                if self.headless {
                    v.remove("sent");
                    v.insert("queued".into(), json!(true));
                    v.insert("message".into(), json!(QUEUED_MESSAGE));
                }
                tracing::info!(session, tool = tool.name(), queued = self.headless, "outside agent sent");
                Outcome::json(Value::Object(v))
            }
            Outcome::Ok { .. } => sent,
            Outcome::Error { code, message } => {
                // Declined, refused or failed: the one-call send leaves no
                // draft behind in the mailbox.
                let _ = self.delete_draft(draft_id).await;
                Outcome::Error { code, message }
            }
        }
    }
}

#[uniffi::export]
impl Core {
    /// Mailbox mode while the app runs (spec §10.1): bind the agent socket
    /// now and say where it is, so `openagc-mcp --mailbox` started by an
    /// outside agent goes through this core. Called once at launch.
    pub fn serve_outside_agents(self: Arc<Self>) -> Result<(), CoreError> {
        if self.headless {
            return Ok(());
        }
        let socket = self.mcp_socket_path()?;
        let dir = self.data_path().join("run");
        let storage = |e: std::io::Error| CoreError::new(ErrorKind::Storage, e.to_string());
        std::fs::create_dir_all(&dir).map_err(storage)?;
        let tmp = dir.join(format!("{OUTSIDE_SOCKET_FILE}.tmp"));
        std::fs::write(&tmp, socket.to_string_lossy().as_bytes()).map_err(storage)?;
        std::fs::rename(&tmp, dir.join(OUTSIDE_SOCKET_FILE)).map_err(storage)
    }
}

/// The running app's socket for mailbox mode, if it said where it is.
pub fn outside_socket(data_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let text = std::fs::read_to_string(data_dir.join("run").join(OUTSIDE_SOCKET_FILE)).ok()?;
    let path = text.trim();
    (!path.is_empty()).then(|| std::path::PathBuf::from(path))
}

#[cfg(test)]
mod tests;
