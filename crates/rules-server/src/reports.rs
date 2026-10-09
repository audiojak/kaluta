//! Reports of what agents sent (spec §10.6, decisions 8 and 10): an agent
//! checks its draft with `check_draft`, sends through the mailbox's
//! service, then files `report_send`. The server checks the report's body
//! against the guide again, keeps it, and the app pulls it at sync, records
//! it as an AI composition and acknowledges it, which deletes it here.
//! Reports the app never pulls go after 30 days regardless.
//!
//! A report is the agent's own words about its own send. The server takes
//! it as untrusted text: sized, never rendered, never logged.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::answers::{self, MAX_BODY_MARKDOWN};
use crate::db::{self, ReportRow};
use crate::{AgentAuth, ApiError, AppState};
use axum::http::StatusCode;

/// `report_send`'s arguments.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportArgs {
    /// The sent message's Message-ID, as the service answered it.
    pub message_id: Option<String>,
    pub to: Vec<String>,
    #[serde(default)]
    pub subject: String,
    /// When it was sent (RFC 3339); the server's time when left out.
    pub sent_at: Option<String>,
    pub body_markdown: String,
    /// The snapshot version the draft was checked against (`check_draft`'s
    /// `version`).
    pub checked_version: Option<i64>,
}

/// Recipients per report.
const MAX_RECIPIENTS: usize = 100;
/// An address, a Message-ID or a subject, in characters.
const MAX_FIELD: usize = 998;
/// The most reports one pull answers.
pub const MAX_LIST: i64 = 500;

fn field_ok(s: &str) -> bool {
    s.chars().count() <= MAX_FIELD && !s.chars().any(|c| c.is_control() && c != '\t')
}

/// A report's checked Message-ID (without brackets), recipients and send
/// time.
pub type Checked = (Option<String>, Vec<String>, Option<i64>);

/// A report's arguments checked, or why not in words for the agent.
pub fn validate(a: &ReportArgs) -> Result<Checked, String> {
    let to: Vec<String> = a.to.iter().map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).collect();
    if to.is_empty() {
        return Err("to needs at least one recipient".into());
    }
    if to.len() > MAX_RECIPIENTS {
        return Err(format!("to has more than {MAX_RECIPIENTS} recipients"));
    }
    if !to.iter().all(|t| field_ok(t) && t.contains('@')) {
        return Err("each recipient in to must be an email address".into());
    }
    if !field_ok(&a.subject) {
        return Err(format!("subject must be one line of at most {MAX_FIELD} characters"));
    }
    if a.body_markdown.len() > MAX_BODY_MARKDOWN {
        return Err(format!("body_markdown is over {} KB", MAX_BODY_MARKDOWN / 1024));
    }
    let message_id = a
        .message_id
        .as_deref()
        .map(|m| m.trim().trim_start_matches('<').trim_end_matches('>').trim().to_owned())
        .filter(|m| !m.is_empty());
    if message_id.as_deref().is_some_and(|m| !field_ok(m) || m.chars().any(char::is_whitespace)) {
        return Err("message_id must be the Message-ID the service gave the sent message".into());
    }
    let sent_at = match a.sent_at.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some(s) => Some(
            chrono::DateTime::parse_from_rfc3339(s)
                .map_err(|_| "sent_at must be an RFC 3339 time, such as 2026-10-09T14:30:00Z".to_owned())?
                .timestamp_millis(),
        ),
    };
    if a.checked_version.is_some_and(|v| v < 0) {
        return Err("checked_version is a snapshot version, 1 or more".into());
    }
    Ok((message_id, to, sent_at))
}

/// Arguments refused, in words: 422, or 413 for a body too big.
pub(crate) fn invalid(message: String) -> ApiError {
    if message.contains(" KB") {
        ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large", message)
    } else {
        ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid_arguments", message)
    }
}

/// `report_send`: check the body against the newest snapshot, keep the
/// report for the app, and answer `{"queued": true, …}` with what the check
/// found.
pub(crate) async fn file(state: &AppState, auth: &AgentAuth, a: ReportArgs) -> Result<Value, ApiError> {
    let (message_id, to, sent_at) = validate(&a).map_err(invalid)?;
    let snapshot = crate::rest::latest(state, auth.mailbox_id).await?;
    let (check_version, guide_check) = match &snapshot {
        Some(s) => (Some(s.version), answers::guide_check(s, &to, None, &a.subject, &a.body_markdown)),
        None => (None, vec![]),
    };
    let now = db::now_ms();
    let row = ReportRow {
        id: 0,
        mailbox_id: auth.mailbox_id,
        agent_id: auth.token_id.clone(),
        received_at: now,
        message_id,
        recipients: to,
        subject: a.subject.trim().to_owned(),
        sent_at,
        body_markdown: a.body_markdown,
        checked_version: a.checked_version,
        check_version,
        guide_check: guide_check.clone(),
    };
    let id = state
        .db
        .run(move |c| {
            let tx = c.transaction()?;
            db::sweep_reports(&tx, now)?;
            let id = db::insert_report(&tx, &row, db::MAX_PENDING_REPORTS)?;
            tx.commit()?;
            Ok(id)
        })
        .await?;
    // The report's id and its agent's; never what it says.
    tracing::info!(mailbox = auth.mailbox_id, token = %auth.token_id, report = id, "report queued");
    Ok(json!({
        "queued": true,
        "report_id": id,
        "guide_check": guide_check,
        "version": check_version,
        "message": "Queued for OpenAGC, which records it the next time it syncs this mailbox.",
    }))
}

/// One report as the app pulls it.
fn report_json(r: &ReportRow, agent_name: &str, agent_kind: &str) -> Value {
    json!({
        "id": r.id,
        "agent_id": r.agent_id,
        "agent_name": agent_name,
        "agent_kind": agent_kind,
        "received_at": answers::time(r.received_at),
        "message_id": r.message_id,
        "to": r.recipients,
        "subject": r.subject,
        "sent_at": r.sent_at.map_or(Value::Null, answers::time),
        "body_markdown": r.body_markdown,
        "checked_version": r.checked_version,
        "check": { "version": r.check_version, "guide_check": r.guide_check },
    })
}

/// The mailbox's reports after `after`, oldest first: `{"reports",
/// "pending", "dropped", "more"}`.
pub(crate) async fn list(state: &AppState, mailbox_id: i64, after: i64, limit: i64) -> Result<Value, ApiError> {
    let limit = limit.clamp(1, MAX_LIST);
    let now = db::now_ms();
    let (rows, (pending, dropped)) = state
        .db
        .run(move |c| {
            db::sweep_reports(c, now)?;
            Ok((db::reports(c, mailbox_id, after, limit + 1)?, db::report_counts(c, mailbox_id)?))
        })
        .await?;
    let more = i64::try_from(rows.len()).unwrap_or(i64::MAX) > limit;
    let reports: Vec<Value> =
        rows.iter().take(usize::try_from(limit).unwrap_or(usize::MAX)).map(|(r, n, k)| report_json(r, n, k)).collect();
    Ok(json!({ "reports": reports, "pending": pending, "dropped": dropped, "more": more }))
}

/// The app has recorded the reports up to `up_to_id`: they go.
pub(crate) async fn ack(state: &AppState, mailbox_id: i64, up_to_id: i64) -> Result<Value, ApiError> {
    let deleted = state.db.run(move |c| db::ack_reports(c, mailbox_id, up_to_id)).await?;
    tracing::info!(mailbox = mailbox_id, deleted, "reports acknowledged");
    Ok(json!({ "deleted": deleted }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> ReportArgs {
        ReportArgs {
            message_id: Some(" <abc@mail.example> ".into()),
            to: vec!["ann@acme.com".into()],
            subject: "Plan".into(),
            sent_at: Some("2026-10-09T14:30:00Z".into()),
            body_markdown: "Hi".into(),
            checked_version: Some(3),
        }
    }

    #[test]
    fn reports_are_checked_and_their_message_id_unwrapped() {
        let (id, to, at) = validate(&args()).unwrap();
        assert_eq!(id.as_deref(), Some("abc@mail.example"));
        assert_eq!(to, ["ann@acme.com"]);
        assert_eq!(at, Some(1_791_556_200_000));
        for bad in [
            ReportArgs { to: vec![], ..args() },
            ReportArgs { to: vec!["not an address".into()], ..args() },
            ReportArgs { subject: "two\nlines".into(), ..args() },
            ReportArgs { sent_at: Some("yesterday".into()), ..args() },
            ReportArgs { message_id: Some("a b@c".into()), ..args() },
            ReportArgs { body_markdown: "x".repeat(MAX_BODY_MARKDOWN + 1), ..args() },
            ReportArgs { checked_version: Some(-1), ..args() },
        ] {
            assert!(validate(&bad).is_err(), "{bad:?}");
        }
        let big =
            invalid(validate(&ReportArgs { body_markdown: "x".repeat(MAX_BODY_MARKDOWN + 1), ..args() }).unwrap_err());
        assert_eq!(big.status, StatusCode::PAYLOAD_TOO_LARGE);
    }
}
