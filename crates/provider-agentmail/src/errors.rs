//! AgentMail's error answers (`{name, code, message, fix, docs}`), in
//! plain words where the app shows them (spec §7.9). Read through
//! [`provider_api::HttpClient`]'s error hook and by [`crate::AgentMailService`].

use std::time::Duration;

use provider_api::ProviderError;

use crate::wire;

/// A send AgentMail refused because the organisation is not verified yet.
pub const SENDS_ONLY_TO_HUMAN: &str = "AgentMail won't send this yet: until the service account is verified, its \
     agents can write only to the email it was created with. Enter the code AgentMail emailed there to verify it.";
/// Refused for a permission the key lacks; usually: not verified yet.
pub const NOT_VERIFIED_YET: &str =
    "AgentMail allows this only once the service account is verified. Enter the code AgentMail emailed you first.";
/// The username is someone else's.
pub const USERNAME_TAKEN: &str =
    "That address is taken at AgentMail. Give the agent another name, and so another address.";
/// The plan's inboxes are all used.
pub const INBOX_LIMIT: &str = "This AgentMail organisation has as many inboxes as its plan allows (3 on the free \
     plan). Remove one in AgentMail's console, or upgrade the plan there.";
/// The inbox is paused.
pub const INBOX_PAUSED: &str = "This AgentMail inbox is paused, so it cannot send. Resume it in AgentMail's console.";

/// A request over AgentMail's 6 MB limit.
pub fn too_large(bytes: usize) -> String {
    format!(
        "AgentMail takes up to 6 MB per message, attachments included; this one is {:.1} MB. Remove an \
         attachment, or share a link to it instead.",
        bytes as f64 / 1_048_576.0
    )
}

/// An AgentMail error answer as a [`ProviderError`]; `None` when the body
/// is not AgentMail's (the caller's default reading applies).
pub fn agentmail_error(status: u16, body: &str) -> Option<ProviderError> {
    let parsed: wire::ErrorBody = serde_json::from_str(body).ok()?;
    let code = parsed.code.clone().unwrap_or_default();
    let text = words(&parsed);
    Some(match (status, code.as_str()) {
        (_, "message_rejected") if mentions_verification(&parsed) => {
            ProviderError::Forbidden(SENDS_ONLY_TO_HUMAN.into())
        }
        (_, "missing_permission") if mentions_verification(&parsed) => {
            ProviderError::Forbidden(NOT_VERIFIED_YET.into())
        }
        (_, "inbox_paused") => ProviderError::Forbidden(INBOX_PAUSED.into()),
        (_, "resource_taken") => ProviderError::Invalid(USERNAME_TAKEN.into()),
        (_, "not_found") | (404, _) => ProviderError::NotFound(text),
        // Another request with the same Idempotency-Key is still running:
        // retry the same request shortly.
        (_, "conflict") => ProviderError::RateLimited { retry_after: Some(Duration::from_secs(2)) },
        (_, "rate_limit_exceeded") => ProviderError::RateLimited { retry_after: None },
        (413, _) => ProviderError::Invalid(too_large(6 * 1_048_576 + 1)),
        (403, _) => ProviderError::Forbidden(text),
        (s, _) if s >= 500 => ProviderError::Server { status: s, message: text },
        (s, _) if (400..500).contains(&s) => ProviderError::Invalid(text),
        _ => return None,
    })
}

/// Whether the error says the organisation must verify first.
fn mentions_verification(e: &wire::ErrorBody) -> bool {
    let said = format!("{} {}", e.message.as_deref().unwrap_or(""), e.fix.as_deref().unwrap_or(""));
    said.to_ascii_lowercase().contains("verif")
}

/// The message and the fix, as AgentMail words them.
fn words(e: &wire::ErrorBody) -> String {
    let message = e.message.as_deref().map(str::trim).filter(|m| !m.is_empty());
    let fix = e.fix.as_deref().map(str::trim).filter(|f| !f.is_empty());
    let detail = e
        .errors
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x["message"].as_str()).collect::<Vec<_>>().join("; "))
        .filter(|d| !d.is_empty());
    let parts: Vec<&str> = [message, detail.as_deref(), fix].into_iter().flatten().collect();
    if parts.is_empty() {
        "AgentMail did not accept the request".into()
    } else {
        parts.iter().map(|p| p.trim_end_matches('.')).collect::<Vec<_>>().join(". ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agentmail_errors_are_read_and_worded() {
        let rejected = r#"{"name":"MessageRejectedError","code":"message_rejected","message":"Recipient not allowed",
            "fix":"Sending is restricted until verification: POST /v0/agent/verify"}"#;
        assert_eq!(agentmail_error(403, rejected), Some(ProviderError::Forbidden(SENDS_ONLY_TO_HUMAN.into())));
        let blocked = r#"{"code":"message_rejected","message":"Recipient on block list","fix":"Delete the entry"}"#;
        assert_eq!(
            agentmail_error(403, blocked),
            Some(ProviderError::Forbidden("Recipient on block list. Delete the entry".into()))
        );
        let permission = r#"{"code":"missing_permission","message":"Forbidden",
            "fix":"Complete agent verification via POST /v0/agent/verify first"}"#;
        assert_eq!(agentmail_error(403, permission), Some(ProviderError::Forbidden(NOT_VERIFIED_YET.into())));
        let taken = r#"{"name":"IsTakenError","code":"resource_taken","message":"Username taken"}"#;
        assert_eq!(agentmail_error(403, taken), Some(ProviderError::Invalid(USERNAME_TAKEN.into())));
        let invalid = r#"{"name":"ValidationError","code":"validation_error","errors":[{"path":["to"],"message":"bad address"}]}"#;
        assert_eq!(agentmail_error(400, invalid), Some(ProviderError::Invalid("bad address".into())));
        assert!(matches!(
            agentmail_error(503, r#"{"code":"service_unavailable"}"#),
            Some(ProviderError::Server { .. })
        ));
        assert!(matches!(agentmail_error(409, r#"{"code":"conflict"}"#), Some(ProviderError::RateLimited { .. })));
        assert_eq!(agentmail_error(500, "<html>"), None);
    }
}
