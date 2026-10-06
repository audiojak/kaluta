//! Tool implementations (spec §10.2). Every call passes the session's hard
//! limits and the permission decision before it touches the store.

use std::collections::HashMap;
use std::sync::Arc;

use agent_api::AgentEvent;
use agent_mcp::Outcome;
use mail_domain::{EmailAddress, LabelId, MessageId, ThreadId, ThreadSummary, iso8601_utc};
use mail_store::read;
use permissions::{Decision, ProposedAction, Tool, decide};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::{Core, CoreError};
use mail_store::compositions::Source;
use mail_sync::LocalChange;

fn ids(thread_ids: Vec<String>) -> Vec<ThreadId> {
    thread_ids.into_iter().map(ThreadId).collect()
}

/// Most threads one search returns.
pub(crate) const MAX_SEARCH: u32 = 50;
/// Characters of body per message in `mail_get_thread`.
pub(crate) const MAX_BODY_CHARS: usize = 20_000;
/// Characters of extracted attachment text.
pub(crate) const MAX_ATTACHMENT_CHARS: usize = 100_000;

fn args<T: DeserializeOwned>(arguments: Value) -> Result<T, Outcome> {
    let arguments = if arguments.is_null() { json!({}) } else { arguments };
    serde_json::from_value(arguments).map_err(|e| Outcome::error("invalid_arguments", e.to_string()))
}

fn failed(e: CoreError) -> Outcome {
    match e.kind() {
        crate::ErrorKind::NotFound => Outcome::error("not_found", e.to_string()),
        crate::ErrorKind::InvalidInput => Outcome::error("invalid_arguments", e.to_string()),
        _ => Outcome::error("failed", e.to_string()),
    }
}

/// Thread ids a call names, for the per-call and session caps.
fn thread_ids_of(tool: Tool, arguments: &Value) -> Vec<ThreadId> {
    if tool.risk() == permissions::Risk::ReadOnly {
        return vec![];
    }
    arguments["thread_ids"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).map(ThreadId::new).collect())
        .unwrap_or_default()
}

pub(crate) async fn call(core: &Arc<Core>, session: &str, tool: Tool, arguments: Value) -> Outcome {
    let action =
        ProposedAction { tool, thread_ids: thread_ids_of(tool, &arguments), draft_id: arguments["draft_id"].as_i64() };
    let now = mail_sync::now_millis();
    let checked = core.agents.with_session(session, |s| s.guard.check(&action, now));
    let read_only = core.agents.with_session(session, |s| s.read_only).unwrap_or(false);
    if read_only && tool.risk() != permissions::Risk::ReadOnly {
        return Outcome::error("denied", "this is a preview: nothing may be changed");
    }
    // An archive has no server to send through (spec §7.8): a clear, final
    // answer so the agent stops instead of retrying.
    if matches!(tool, Tool::CreateDraft | Tool::UpdateDraft | Tool::Send | Tool::Forward)
        && core.refuse_if_archive().is_err()
    {
        return Outcome::error("cannot_send", crate::archive::CANNOT_SEND);
    }
    let refusal = match checked {
        None => return Outcome::error("unknown_session", "this agent session has ended"),
        Some(Err(reason)) => Some(reason.to_string()),
        Some(Ok(())) => {
            let policy = core.agents.policy.read().unwrap_or_else(|e| e.into_inner()).clone();
            match decide(&policy, &action) {
                Decision::Deny(reason) => Some(reason.to_string()),
                Decision::Allow => None,
                Decision::RequireApproval => {
                    // An agent mailbox set to send freely (spec §7.9).
                    if matches!(tool, Tool::Send | Tool::Forward) && core.agent_sends_freely() {
                        return send_freely(core, session, tool, arguments).await;
                    }
                    return approve_then_run(core, session, tool, arguments).await;
                }
            }
        }
    };
    let action_id =
        core.record_action(session, tool, &arguments, if refusal.is_some() { "denied" } else { "allowed" }).await;
    let outcome = match refusal {
        Some(reason) => Outcome::error("denied", reason),
        None => run(core, session, tool, arguments).await,
    };
    let state = if matches!(outcome, Outcome::Ok { .. }) { "done" } else { "failed" };
    let state = if refusal_state(&outcome) { "denied" } else { state };
    core.finish_action(action_id, state, Some(super::approvals::outcome_summary(&outcome))).await;
    outcome
}

fn refusal_state(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::Error { code, .. } if code == "denied")
}

async fn run(core: &Arc<Core>, session: &str, tool: Tool, arguments: Value) -> Outcome {
    let result = match tool {
        Tool::Search => search(core, session, arguments).await,
        Tool::GetThread => get_thread(core, session, arguments).await,
        Tool::GetMessage => get_message(core, session, arguments).await,
        Tool::ListLabels => list_labels(core).await,
        Tool::GetAttachmentText => attachment_text(core, session, arguments).await,
        Tool::PresentThreads => present_threads(core, session, arguments),
        Tool::FactsLookup => facts_lookup(core, arguments).await,
        Tool::CreateDraft => create_draft(core, session, arguments).await,
        Tool::UpdateDraft => update_draft(core, session, arguments).await,
        Tool::Archive => change_threads(core, arguments, ThreadChange::Archive).await,
        Tool::MarkRead => change_threads(core, arguments, ThreadChange::Read(true)).await,
        Tool::MarkUnread => change_threads(core, arguments, ThreadChange::Read(false)).await,
        Tool::AddLabel => label_threads(core, arguments, true).await,
        Tool::RemoveLabel => label_threads(core, arguments, false).await,
        Tool::CreateLabel => create_label(core, arguments).await,
        // External tools only ever run after approval.
        Tool::Send | Tool::Forward | Tool::Delete => {
            Err(Outcome::error("failed", "external actions run only after approval"))
        }
    };
    result.unwrap_or_else(|e| e)
}

/// What the user approves, prepared before asking: the summary they read
/// and, for sends and forwards, the draft they can review.
struct Proposal {
    summary: String,
    draft_id: Option<i64>,
}

async fn approve_then_run(core: &Arc<Core>, session: &str, tool: Tool, arguments: Value) -> Outcome {
    let proposal = match prepare(core, session, tool, &arguments).await {
        Ok(p) => p,
        Err(outcome) => {
            let id = core.record_action(session, tool, &arguments, "failed").await;
            core.finish_action(id, "failed", Some(super::approvals::outcome_summary(&outcome))).await;
            return outcome;
        }
    };
    let action_id = core.record_action(session, tool, &arguments, "pending").await;
    if let Err(declined) = core.await_approval(session, action_id, tool, proposal.summary, proposal.draft_id).await {
        // A forward the user declined leaves no draft behind.
        if tool == Tool::Forward
            && let Some(draft) = proposal.draft_id
        {
            let _ = core.delete_draft(draft).await;
        }
        return declined;
    }
    let outcome = match tool {
        Tool::Send | Tool::Forward => match proposal.draft_id {
            Some(draft) => match core.clone().send_draft(draft).await {
                Ok(_) => Outcome::json(json!({ "sent": true, "draft_id": draft })),
                Err(e) => failed(e),
            },
            None => Outcome::error("failed", "no draft to send"),
        },
        Tool::Delete => {
            let a: Result<ThreadsArgs, Outcome> = args(arguments);
            match a {
                Ok(a) => {
                    let n = a.thread_ids.len();
                    match core.mutate_unrecorded(LocalChange::Trash { thread_ids: ids(a.thread_ids) }).await {
                        Ok(()) => Outcome::json(json!({ "trashed": n })),
                        Err(e) => failed(e),
                    }
                }
                Err(e) => e,
            }
        }
        // A reversible tool the user asked to approve.
        _ => run(core, session, tool, arguments).await,
    };
    let state = if matches!(outcome, Outcome::Ok { .. }) { "done" } else { "failed" };
    core.finish_action(action_id, state, Some(super::approvals::outcome_summary(&outcome))).await;
    outcome
}

/// Send or forward from an agent mailbox that sends freely: no approval,
/// recorded like any send, and what breaks the writing guide is flagged in
/// the activity log and told to the agent (spec §7.9).
async fn send_freely(core: &Arc<Core>, session: &str, tool: Tool, arguments: Value) -> Outcome {
    let proposal = match prepare(core, session, tool, &arguments).await {
        Ok(p) => p,
        Err(outcome) => {
            let id = core.record_action(session, tool, &arguments, "failed").await;
            core.finish_action(id, "failed", Some(super::approvals::outcome_summary(&outcome))).await;
            return outcome;
        }
    };
    let action_id = core.record_action(session, tool, &arguments, "allowed").await;
    let breaches = proposal.summary.split_once(GUIDE_BREACH).map(|(_, b)| b.to_owned());
    let outcome = match proposal.draft_id {
        Some(draft) => match core.clone().send_draft(draft).await {
            Ok(_) => {
                let mut sent = json!({ "sent": true, "draft_id": draft });
                if let Some(breaches) = &breaches {
                    sent["writing_guide_breaches"] = json!(breaches);
                }
                Outcome::json(sent)
            }
            Err(e) => failed(e),
        },
        None => Outcome::error("failed", "no draft to send"),
    };
    // A forward that could not go leaves no draft behind, as when declined.
    if tool == Tool::Forward
        && !matches!(outcome, Outcome::Ok { .. })
        && let Some(draft) = proposal.draft_id
    {
        let _ = core.delete_draft(draft).await;
    }
    let (state, summary) = match &outcome {
        Outcome::Ok { .. } => ("done", proposal.summary),
        _ => ("failed", super::approvals::outcome_summary(&outcome)),
    };
    core.finish_action(action_id, state, Some(summary)).await;
    outcome
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

fn recipients(d: &crate::DraftInfo) -> String {
    let all: Vec<String> = d.to.iter().chain(&d.cc).chain(&d.bcc).map(|a| a.email.clone()).collect();
    if all.is_empty() { "no recipients".into() } else { all.join(", ") }
}

async fn prepare(core: &Arc<Core>, session: &str, tool: Tool, arguments: &Value) -> Result<Proposal, Outcome> {
    match tool {
        Tool::Send => {
            let draft_id = arguments["draft_id"]
                .as_i64()
                .ok_or_else(|| Outcome::error("invalid_arguments", "draft_id is required"))?;
            let d = core
                .get_draft(draft_id)
                .await
                .map_err(failed)?
                .ok_or_else(|| Outcome::error("not_found", "that draft no longer exists"))?;
            if d.to.is_empty() && d.cc.is_empty() && d.bcc.is_empty() {
                return Err(Outcome::error("invalid_arguments", "the draft has no recipients"));
            }
            let warning = guide_warning(core, &d).await;
            Ok(Proposal {
                summary: format!("Send “{}” to {}{warning}", d.subject, recipients(&d)),
                draft_id: Some(draft_id),
            })
        }
        Tool::Forward => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct ForwardArgs {
                message_id: String,
                to: Vec<String>,
                note_markdown: Option<String>,
            }
            let a: ForwardArgs = args(arguments.clone())?;
            let not_found = || Outcome::error("not_found", "no such message");
            let db = core.db().map_err(failed)?;
            let mid = MessageId(a.message_id.clone());
            let m = db
                .read(move |c| read::get_message(c, &mid))
                .await
                .map_err(|e| failed(e.into()))?
                .ok_or_else(not_found)?;
            if !in_scope(core, session, &m.thread_id) {
                return Err(not_found());
            }
            let to = addresses(Some(a.to))?.unwrap_or_default();
            if to.is_empty() {
                return Err(Outcome::error("invalid_arguments", "say who to forward it to"));
            }
            let mut draft = core.forward_draft(a.message_id).await.map_err(failed)?;
            draft.to = to;
            draft.body_html = a.note_markdown.as_deref().map(mail_mime::markdown_to_html).unwrap_or_default();
            let id = core.save_draft(draft.clone()).await.map_err(failed)?;
            core.agents.with_session(session, |s| s.guard.allow_draft(id));
            let warning = guide_warning(core, &draft).await;
            Ok(Proposal {
                summary: format!("Forward “{}” to {}{warning}", m.subject, recipients(&draft)),
                draft_id: Some(id),
            })
        }
        Tool::Delete => {
            let n = arguments["thread_ids"].as_array().map_or(0, Vec::len);
            Ok(Proposal { summary: format!("Move {} to Trash", plural(n, "thread")), draft_id: None })
        }
        other => {
            let n = arguments["thread_ids"].as_array().map_or(0, Vec::len);
            let what = if n > 0 { format!(" on {}", plural(n, "thread")) } else { String::new() };
            Ok(Proposal { summary: format!("{}{what}", other.name()), draft_id: None })
        }
    }
}

fn in_scope(core: &Core, session: &str, thread: &ThreadId) -> bool {
    core.agents.with_session(session, |s| s.guard.scope().allows(thread)).unwrap_or(false)
}

fn address(a: &EmailAddress) -> String {
    match &a.name {
        Some(name) if !name.is_empty() => format!("{name} <{}>", a.email),
        _ => a.email.clone(),
    }
}

async fn label_names(core: &Core) -> Result<HashMap<String, String>, Outcome> {
    let db = core.db().map_err(failed)?;
    let labels = db.read(read::list_labels).await.map_err(|e| failed(e.into()))?;
    Ok(labels.into_iter().map(|l| (l.id.0, l.name)).collect())
}

fn labels_json(ids: &[LabelId], names: &HashMap<String, String>) -> Vec<String> {
    ids.iter().map(|l| names.get(l.as_str()).cloned().unwrap_or_else(|| l.0.clone())).collect()
}

fn thread_json(t: &ThreadSummary, names: &HashMap<String, String>) -> Value {
    json!({
        "thread_id": t.id,
        "subject": t.subject,
        "participants": t.participants.iter().map(address).collect::<Vec<_>>(),
        "date": iso8601_utc(t.last_message_at),
        "snippet": t.snippet,
        "labels": labels_json(&t.label_ids, names),
        "unread": t.unread_count > 0,
        "messages": t.message_count,
        "has_attachments": t.has_attachments,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    query: String,
    limit: Option<u32>,
    cursor: Option<String>,
}

async fn search(core: &Arc<Core>, session: &str, arguments: Value) -> Result<Outcome, Outcome> {
    let a: SearchArgs = args(arguments)?;
    let query = if a.query.trim().is_empty() { "in:inbox".to_owned() } else { a.query };
    let limit = a.limit.unwrap_or(20).clamp(1, MAX_SEARCH);
    let page = core.search_threads(query, a.cursor, limit).await.map_err(failed)?;
    let names = label_names(core).await?;
    let db = core.db().map_err(failed)?;
    let mut threads = Vec::with_capacity(page.rows.len());
    for row in page.rows {
        let id = ThreadId(row.id);
        if !in_scope(core, session, &id) {
            continue;
        }
        let lookup = id.clone();
        if let Some(t) = db.read(move |c| read::get_thread_summary(c, &lookup)).await.map_err(|e| failed(e.into()))? {
            threads.push(thread_json(&t, &names));
        }
    }
    Ok(Outcome::json(json!({ "threads": threads, "next_cursor": page.next_cursor })))
}

fn message_json(m: &mail_domain::Message, body: Option<&str>, include_quoted: bool) -> Value {
    let (text, truncated) = match body {
        Some(b) => {
            let b = if include_quoted { b.to_owned() } else { mail_mime::strip_quoted(b) };
            let (t, cut) = mail_mime::truncate_chars(&b, MAX_BODY_CHARS);
            (Some(t), cut)
        }
        None => (None, false),
    };
    json!({
        "message_id": m.id,
        "from": m.from.as_ref().map(address),
        "to": m.to.iter().map(address).collect::<Vec<_>>(),
        "cc": m.cc.iter().map(address).collect::<Vec<_>>(),
        "date": iso8601_utc(m.date),
        "subject": m.subject,
        "unread": !m.is_read,
        "sent_by_me": m.is_sent_by_me,
        "body": text,
        "body_available": body.is_some(),
        "truncated": truncated,
        "attachments": m.attachments.iter().filter(|a| !a.is_inline).map(|a| json!({
            "attachment_id": a.id,
            "filename": a.filename,
            "mime_type": a.mime_type,
            "size": a.size,
        })).collect::<Vec<_>>(),
    })
}

/// How long an agent's read waits for header-only bodies to download.
const ENSURE_BODIES_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Download header-only bodies before a tool reads them (spec §7.4 tiered
/// download). Failure is not the tool's: it answers with
/// `body_available: false` and the bodies are fetched first later.
async fn ensure_bodies(core: &Core, ids: Vec<MessageId>) {
    let Some(service) = core.sync_service() else { return };
    let fetch = service.engine().ensure_bodies(ids.clone());
    match tokio::time::timeout(ENSURE_BODIES_WAIT, fetch).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "agent read: bodies not downloaded");
            let _ = service.prioritize(ids).await;
        }
        Err(_) => {
            tracing::warn!("agent read: bodies still downloading");
            let _ = service.prioritize(ids).await;
        }
    }
}

async fn body_text(core: &Core, id: &MessageId) -> Result<Option<String>, Outcome> {
    let db = core.db().map_err(failed)?;
    let id = id.clone();
    let body = db.read(move |c| read::get_body(c, &id)).await.map_err(|e| failed(e.into()))?;
    Ok(body.and_then(|b| b.text_plain))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadArgs {
    thread_id: String,
}

async fn get_thread(core: &Arc<Core>, session: &str, arguments: Value) -> Result<Outcome, Outcome> {
    let a: ThreadArgs = args(arguments)?;
    let id = ThreadId(a.thread_id);
    let not_found = || Outcome::error("not_found", "no such thread");
    if !in_scope(core, session, &id) {
        return Err(not_found());
    }
    let db = core.db().map_err(failed)?;
    let lookup = id.clone();
    let (_, messages) =
        db.read(move |c| read::get_thread(c, &lookup)).await.map_err(|e| failed(e.into()))?.ok_or_else(not_found)?;
    let header_only: Vec<MessageId> =
        messages.iter().filter(|m| m.body_state != mail_domain::BodyState::Full).map(|m| m.id.clone()).collect();
    let messages = if header_only.is_empty() {
        messages
    } else {
        ensure_bodies(core, header_only).await;
        let lookup = id.clone();
        db.read(move |c| read::get_thread(c, &lookup)).await.map_err(|e| failed(e.into()))?.ok_or_else(not_found)?.1
    };
    let lookup = id.clone();
    let summary = db
        .read(move |c| read::get_thread_summary(c, &lookup))
        .await
        .map_err(|e| failed(e.into()))?
        .ok_or_else(not_found)?;
    let names = label_names(core).await?;
    let mut out = Vec::with_capacity(messages.len());
    for m in &messages {
        let body = body_text(core, &m.id).await?;
        out.push(message_json(m, body.as_deref(), false));
    }
    Ok(Outcome::json(json!({
        "thread_id": summary.id,
        "subject": summary.subject,
        "labels": labels_json(&summary.label_ids, &names),
        "messages": out,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageArgs {
    message_id: String,
    #[serde(default)]
    include_quoted: bool,
}

async fn get_message(core: &Arc<Core>, session: &str, arguments: Value) -> Result<Outcome, Outcome> {
    let a: MessageArgs = args(arguments)?;
    let id = MessageId(a.message_id);
    let not_found = || Outcome::error("not_found", "no such message");
    let db = core.db().map_err(failed)?;
    let lookup = id.clone();
    let m =
        db.read(move |c| read::get_message(c, &lookup)).await.map_err(|e| failed(e.into()))?.ok_or_else(not_found)?;
    if !in_scope(core, session, &m.thread_id) {
        return Err(not_found());
    }
    let m = if m.body_state == mail_domain::BodyState::Full {
        m
    } else {
        ensure_bodies(core, vec![id.clone()]).await;
        let lookup = id.clone();
        db.read(move |c| read::get_message(c, &lookup)).await.map_err(|e| failed(e.into()))?.ok_or_else(not_found)?
    };
    let body = body_text(core, &id).await?;
    let mut value = message_json(&m, body.as_deref(), a.include_quoted);
    value["thread_id"] = json!(m.thread_id);
    Ok(Outcome::json(value))
}

async fn list_labels(core: &Arc<Core>) -> Result<Outcome, Outcome> {
    let mailboxes = core.list_mailboxes().await.map_err(failed)?;
    let labels: Vec<Value> = mailboxes
        .iter()
        .filter(|m| m.label_id.is_some())
        .map(|m| {
            json!({
                "label_id": m.label_id,
                "name": m.name,
                "system": m.kind != crate::ffi::MailboxKind::Label,
                "unread": m.unread_count,
                "total": m.total_count,
            })
        })
        .collect();
    Ok(Outcome::json(json!({ "labels": labels })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachmentArgs {
    message_id: String,
    attachment_id: String,
}

async fn attachment_text(core: &Arc<Core>, session: &str, arguments: Value) -> Result<Outcome, Outcome> {
    let a: AttachmentArgs = args(arguments)?;
    let not_found = || Outcome::error("not_found", "no such attachment on that message");
    let row: i64 = a.attachment_id.parse().map_err(|_| not_found())?;
    let db = core.db().map_err(failed)?;
    // A header-only message has no attachment rows yet (tiered download).
    let named = MessageId(a.message_id.clone());
    let lookup = named.clone();
    let message = db.read(move |c| read::get_message(c, &lookup)).await.map_err(|e| failed(e.into()))?;
    if let Some(m) = message
        && m.body_state != mail_domain::BodyState::Full
        && in_scope(core, session, &m.thread_id)
    {
        ensure_bodies(core, vec![named]).await;
    }
    let source = db.read(move |c| read::attachment_source(c, row)).await.map_err(|e| failed(e.into()))?;
    // The attachment must belong to the message named, and be in scope.
    let source = source.filter(|s| s.message_id.as_str() == a.message_id).ok_or_else(not_found)?;
    let message_id = source.message_id.clone();
    let m = db.read(move |c| read::get_message(c, &message_id)).await.map_err(|e| failed(e.into()))?;
    if !m.is_some_and(|m| in_scope(core, session, &m.thread_id)) {
        return Err(not_found());
    }
    let file = core.attachment_file(a.attachment_id).await.map_err(failed)?;
    let text = if mail_mime::is_pdf(&file.mime_type, &file.filename) {
        let extractor = core.agents.text.read().unwrap_or_else(|e| e.into_inner()).clone();
        let extractor = extractor.ok_or_else(|| Outcome::error("unsupported", "PDF text is not available"))?;
        let path = file.path.clone();
        tokio::task::spawn_blocking(move || extractor.pdf_text(path))
            .await
            .map_err(|e| Outcome::error("failed", e.to_string()))?
            .unwrap_or_default()
    } else {
        let path = file.path.clone();
        let bytes = tokio::fs::read(&path).await.map_err(|e| Outcome::error("failed", e.to_string()))?;
        mail_mime::extract_attachment_text(&bytes, &file.mime_type, &file.filename)
            .map_err(|e| Outcome::error("unsupported", e.to_string()))?
    };
    let (text, truncated) = mail_mime::truncate_chars(&text, MAX_ATTACHMENT_CHARS);
    Ok(Outcome::json(json!({
        "filename": file.filename,
        "mime_type": file.mime_type,
        "text": text,
        "truncated": truncated,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PresentArgs {
    thread_ids: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    title: Option<String>,
}

fn present_threads(core: &Arc<Core>, session: &str, arguments: Value) -> Result<Outcome, Outcome> {
    let a: PresentArgs = args(arguments)?;
    let ids: Vec<ThreadId> = a.thread_ids.into_iter().map(ThreadId).filter(|t| in_scope(core, session, t)).collect();
    let shown = ids.len();
    core.agents.with_session(session, |s| {
        if let Some(sink) = &s.sink {
            sink.emit(AgentEvent::ResultsAvailable { thread_ids: ids });
        }
    });
    Ok(Outcome::json(json!({ "shown": shown })))
}

// MARK: Changing the mailbox (Reversible)

/// "Name <a@b.c>" or "a@b.c".
fn parse_address(text: &str) -> Result<crate::ffi::AddressInfo, Outcome> {
    let t = text.trim();
    let (name, email) = match (t.rfind('<'), t.rfind('>')) {
        (Some(open), Some(close)) if open < close => {
            let name = t[..open].trim().trim_matches('"').trim();
            (if name.is_empty() { None } else { Some(name.to_owned()) }, t[open + 1..close].trim().to_owned())
        }
        _ => (None, t.to_owned()),
    };
    let valid = email.matches('@').count() == 1
        && !email.starts_with('@')
        && !email.ends_with('@')
        && !email.chars().any(|c| c.is_whitespace() || matches!(c, '<' | '>' | ',' | ';' | '"'));
    if !valid {
        return Err(Outcome::error("invalid_arguments", format!("{text:?} is not an email address")));
    }
    Ok(crate::ffi::AddressInfo { name, email })
}

fn addresses(list: Option<Vec<String>>) -> Result<Option<Vec<crate::ffi::AddressInfo>>, Outcome> {
    list.map(|l| l.iter().map(|a| parse_address(a)).collect()).transpose()
}

/// The draft's own text (not the quoted original), its type and its
/// recipients, as the writing guide's checks see it.
fn draft_target(d: &crate::DraftInfo) -> (crate::guide_render::Target, String) {
    let subject = d.subject.trim().to_lowercase();
    let kind = if subject.starts_with("fwd:") || subject.starts_with("fw:") {
        "forward"
    } else if d.in_reply_to_message_id.is_some() {
        "reply"
    } else {
        "new"
    };
    let target = crate::guide_render::Target {
        recipients: d.to.iter().chain(&d.cc).map(|a| a.email.clone()).collect(),
        message_type: Some(kind.into()),
        audiences: None,
    };
    (target, mail_mime::html_to_text(&d.body_html))
}

/// " · Breaks your writing guide: …" for an approval, or nothing.
/// Where a proposal's summary says what the draft breaks.
const GUIDE_BREACH: &str = " · Breaks your writing guide: ";

async fn guide_warning(core: &Arc<Core>, d: &crate::DraftInfo) -> String {
    let (target, text) = draft_target(d);
    match core.check_against_guide(target, &text).await {
        Ok(failures) if !failures.is_empty() => {
            format!("{GUIDE_BREACH}{}", failures.iter().map(|f| f.message.clone()).collect::<Vec<_>>().join("; "))
        }
        _ => String::new(),
    }
}

/// A draft tool's result with the writing guide for the draft's
/// recipients and type (spec §14.9), so a long conversation keeps it; the
/// draft records the guide version it was written under.
async fn draft_with_guide(core: &Arc<Core>, d: &crate::DraftInfo) -> Value {
    let mut value = draft_json(d);
    let (target, text) = draft_target(d);
    if let Ok(guide) = core.render_guide(Some(target.clone())).await
        && !guide.text.is_empty()
    {
        value["writing_guide"] = json!(guide.text);
        // What the draft breaks, so the agent can fix it before asking to send.
        if let Ok(failures) = core.check_against_guide(target, &text).await
            && !failures.is_empty()
        {
            value["guide_check"] = json!(failures.iter().map(|f| f.message.clone()).collect::<Vec<_>>());
        }
        if let Err(e) = core.record_draft_guide(d.id, guide.version).await {
            tracing::warn!(error = %e, "draft guide version not recorded");
        }
    }
    value
}

/// Keep what the agent wrote into a draft (spec §14.10): its own text, not
/// the quoted original, with the prompt it was answering.
async fn record_body(core: &Arc<Core>, session: &str, draft_id: i64, html: &str) {
    let routine = core.agents.routine_sessions.lock().unwrap_or_else(|e| e.into_inner()).contains_key(session);
    let (agent, prompt) =
        core.agents.with_session(session, |s| (s.agent.clone(), s.last_prompt.clone())).unwrap_or_default();
    let version = core.draft_guide_version(draft_id).await.ok().flatten();
    let ai = crate::compositions::AiText {
        source: if routine { Source::Routine } else { Source::Agent },
        agent,
        instruction: prompt,
        text: mail_mime::html_to_text(html).trim().to_owned(),
        html: Some(html.to_owned()),
        guide_version: version,
        audiences: vec![],
    };
    if let Err(e) = core.record_composition(draft_id, ai).await {
        tracing::warn!(error = %e, "AI composition not recorded");
    }
}

fn draft_json(d: &crate::DraftInfo) -> Value {
    json!({
        "draft_id": d.id,
        "subject": d.subject,
        "to": d.to.iter().map(|a| a.email.clone()).collect::<Vec<_>>(),
        "cc": d.cc.iter().map(|a| a.email.clone()).collect::<Vec<_>>(),
        "reply_to_message_id": d.in_reply_to_message_id,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactsLookupArgs {
    category: Option<String>,
    query: Option<String>,
}

/// The user's facts drafting may use (spec §14.11); never-share facts are
/// left out.
async fn facts_lookup(core: &Arc<Core>, arguments: Value) -> Result<Outcome, Outcome> {
    let a: FactsLookupArgs = args(arguments)?;
    let facts = core.list_facts(vec![crate::facts::FactStatus::Accepted]).await.map_err(failed)?;
    let categories = core.fact_categories().await.map_err(failed)?;
    Ok(Outcome::json(crate::facts::lookup_json(&facts, &categories, a.category.as_deref(), a.query.as_deref())))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateDraftArgs {
    reply_to_message_id: Option<String>,
    #[serde(default)]
    reply_all: bool,
    to: Option<Vec<String>>,
    cc: Option<Vec<String>>,
    subject: Option<String>,
    body_markdown: String,
}

async fn create_draft(core: &Arc<Core>, session: &str, arguments: Value) -> Result<Outcome, Outcome> {
    let a: CreateDraftArgs = args(arguments)?;
    let mut draft = match &a.reply_to_message_id {
        Some(id) => {
            let not_found = || Outcome::error("not_found", "no such message");
            let db = core.db().map_err(failed)?;
            let mid = MessageId(id.clone());
            let m = db
                .read(move |c| read::get_message(c, &mid))
                .await
                .map_err(|e| failed(e.into()))?
                .ok_or_else(not_found)?;
            if !in_scope(core, session, &m.thread_id) {
                return Err(not_found());
            }
            core.reply_draft(id.clone(), a.reply_all).await.map_err(failed)?
        }
        None => crate::DraftInfo {
            id: 0,
            thread_id: None,
            in_reply_to_message_id: None,
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: String::new(),
            body_html: String::new(),
            quoted_html: String::new(),
            attachments: vec![],
            status: crate::DraftStatus::Editing,
            error: None,
            updated_at: 0,
        },
    };
    if let Some(to) = addresses(a.to)? {
        draft.to = to;
    }
    if let Some(cc) = addresses(a.cc)? {
        draft.cc = cc;
    }
    if let Some(subject) = a.subject {
        draft.subject = subject;
    }
    draft.body_html = mail_mime::markdown_to_html(&a.body_markdown);
    let quote = draft.quoted_html.clone();
    let id = core.save_draft(draft.clone()).await.map_err(failed)?;
    draft.id = id;
    core.agents.with_session(session, |s| {
        s.guard.allow_draft(id);
        s.draft_quotes.insert(id, quote);
    });
    let value = draft_with_guide(core, &draft).await;
    record_body(core, session, id, &draft.body_html).await;
    Ok(Outcome::json(value))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateDraftArgs {
    draft_id: i64,
    to: Option<Vec<String>>,
    cc: Option<Vec<String>>,
    subject: Option<String>,
    body_markdown: Option<String>,
}

async fn update_draft(core: &Arc<Core>, session: &str, arguments: Value) -> Result<Outcome, Outcome> {
    let a: UpdateDraftArgs = args(arguments)?;
    let not_yours = || Outcome::error("denied", "only drafts created in this conversation can be changed");
    let quote = core
        .agents
        .with_session(session, |s| {
            s.guard.owns_draft(a.draft_id).then(|| s.draft_quotes.get(&a.draft_id).cloned().unwrap_or_default())
        })
        .flatten()
        .ok_or_else(not_yours)?;
    if core.agents.approvals.is_frozen(core.effective_account_id().as_deref(), a.draft_id) {
        return Err(Outcome::error("denied", "the user is reviewing this draft; wait for their answer"));
    }
    let mut draft = core
        .get_draft(a.draft_id)
        .await
        .map_err(failed)?
        .ok_or_else(|| Outcome::error("not_found", "that draft no longer exists"))?;
    if let Some(to) = addresses(a.to)? {
        draft.to = to;
    }
    if let Some(cc) = addresses(a.cc)? {
        draft.cc = cc;
    }
    if let Some(subject) = a.subject {
        draft.subject = subject;
    }
    if let Some(body) = &a.body_markdown {
        // The stored body includes the quote; rebuild it around the new text.
        draft.body_html = mail_mime::markdown_to_html(body);
        draft.quoted_html = quote;
    }
    core.save_draft(draft.clone()).await.map_err(failed)?;
    let value = draft_with_guide(core, &draft).await;
    match &a.body_markdown {
        Some(_) => record_body(core, session, a.draft_id, &draft.body_html).await,
        None => {
            if let Err(e) = core.composition_readdressed(a.draft_id).await {
                tracing::warn!(error = %e, "AI composition not updated");
            }
        }
    }
    Ok(Outcome::json(value))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ThreadsArgs {
    thread_ids: Vec<String>,
}

enum ThreadChange {
    Archive,
    Read(bool),
}

async fn change_threads(core: &Arc<Core>, arguments: Value, change: ThreadChange) -> Result<Outcome, Outcome> {
    let a: ThreadsArgs = args(arguments)?;
    let n = a.thread_ids.len();
    let (done, key) = match change {
        ThreadChange::Archive => (core.mutate_unrecorded(LocalChange::archive(ids(a.thread_ids))).await, "archived"),
        ThreadChange::Read(read) => (
            core.mutate_unrecorded(LocalChange::set_read(ids(a.thread_ids), read)).await,
            if read { "marked_read" } else { "marked_unread" },
        ),
    };
    done.map_err(failed)?;
    Ok(Outcome::json(json!({ key: n })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LabelArgs {
    thread_ids: Vec<String>,
    label: String,
}

async fn label_threads(core: &Arc<Core>, arguments: Value, add: bool) -> Result<Outcome, Outcome> {
    let a: LabelArgs = args(arguments)?;
    let db = core.db().map_err(failed)?;
    let labels = db.read(read::list_labels).await.map_err(|e| failed(e.into()))?;
    let wanted = a.label.trim();
    // System labels are refused by name whether or not this account has them.
    let system = mail_domain::system_labels::PROTECTED.iter().chain(&["INBOX", "UNREAD", "STARRED", "IMPORTANT"]);
    if system.clone().any(|s| s.eq_ignore_ascii_case(wanted)) {
        return Err(Outcome::error(
            "invalid_arguments",
            format!("{wanted} is a system label; only user labels can be set here"),
        ));
    }
    let label = labels
        .iter()
        .find(|l| l.id.as_str() == wanted)
        .or_else(|| labels.iter().find(|l| l.name.eq_ignore_ascii_case(wanted)))
        .ok_or_else(|| {
            Outcome::error("not_found", format!("there is no label {wanted:?}; create it with mail_create_label"))
        })?;
    if label.kind != mail_domain::LabelKind::User {
        return Err(Outcome::error(
            "invalid_arguments",
            format!("{} is a system label; only user labels can be set here", label.name),
        ));
    }
    let n = a.thread_ids.len();
    let (plus, minus) = if add { (vec![label.id.0.clone()], vec![]) } else { (vec![], vec![label.id.0.clone()]) };
    let change = LocalChange::Labels {
        thread_ids: ids(a.thread_ids),
        add: plus.into_iter().map(LabelId).collect(),
        remove: minus.into_iter().map(LabelId).collect(),
    };
    core.mutate_unrecorded(change).await.map_err(failed)?;
    Ok(Outcome::json(json!({ "label": label.name, if add { "labeled" } else { "unlabeled" }: n })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateLabelArgs {
    name: String,
    color: Option<String>,
}

async fn create_label(core: &Arc<Core>, arguments: Value) -> Result<Outcome, Outcome> {
    let a: CreateLabelArgs = args(arguments)?;
    if let Some(c) = &a.color
        && !(c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|ch| ch.is_ascii_hexdigit()))
    {
        return Err(Outcome::error("invalid_arguments", "color must look like #rrggbb"));
    }
    let label = core.create_label(a.name, a.color).await.map_err(failed)?;
    Ok(Outcome::json(json!({ "label_id": label.id, "name": label.name })))
}
