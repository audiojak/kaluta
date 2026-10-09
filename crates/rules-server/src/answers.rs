//! The one handler set behind both surfaces (spec §10.6): what
//! `guide_rules` and `facts_lookup` answer, over MCP and over REST alike,
//! from a mailbox's newest snapshot. The answers are mailbox mode's
//! (§10.1, `docs/mcp.md`) plus the snapshot's `version` and
//! `published_at`, the time the guide is "as of". There is no `send_mode`:
//! a cloud agent sends through the service, not through OpenAGC.

use serde::Deserialize;
use serde_json::{Value, json};
use writing_guide::{Snapshot, Target};

/// `guide_rules`' arguments, as in mailbox mode.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuideArgs {
    #[serde(default)]
    pub to: Vec<String>,
    pub message_type: Option<String>,
}

/// `facts_lookup`'s arguments, as in mailbox mode.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactsArgs {
    pub category: Option<String>,
    pub query: Option<String>,
}

pub const MESSAGE_TYPES: [&str; 3] = ["new", "reply", "forward"];

/// Why arguments were refused, in words for the agent.
pub fn check_guide_args(a: &GuideArgs) -> Result<(), String> {
    match &a.message_type {
        Some(t) if !MESSAGE_TYPES.contains(&t.as_str()) => {
            Err(format!("message_type must be one of new, reply or forward, not {t:?}"))
        }
        _ => Ok(()),
    }
}

/// An instant as agents read it: RFC 3339 in UTC, to the second.
pub fn time(ms: i64) -> Value {
    chrono::DateTime::from_timestamp_millis(ms)
        .map_or(Value::Null, |t| json!(t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)))
}

/// `guide_rules`: the guide for these recipients and this type, whose
/// mailbox it is and the name it sends as.
pub fn guide_rules(s: &Snapshot, a: &GuideArgs) -> Value {
    let target = Target {
        recipients: a.to.iter().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect(),
        message_type: a.message_type.clone(),
        audiences: None,
    };
    let guide = s.guide(Some(&target));
    json!({
        "mailbox": s.mailbox.address,
        "sends_as": format!("{} <{}>", s.mailbox.name, s.mailbox.address),
        "about": s.mailbox.about.trim(),
        "writing_guide": guide.text,
        "guide_version": guide.version,
        "version": s.version,
        "published_at": time(s.published_at),
    })
}

/// `facts_lookup`: the matching facts shared with cloud agents.
pub fn facts_lookup(s: &Snapshot, a: &FactsArgs) -> Value {
    let mut answer = s.facts_lookup(a.category.as_deref(), a.query.as_deref());
    answer["version"] = json!(s.version);
    answer["published_at"] = time(s.published_at);
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_utc_to_the_second() {
        assert_eq!(time(1_760_000_000_123), json!("2025-10-09T08:53:20Z"));
    }

    #[test]
    fn message_types_are_mailbox_modes() {
        assert!(check_guide_args(&GuideArgs { message_type: Some("reply".into()), ..Default::default() }).is_ok());
        assert!(check_guide_args(&GuideArgs { message_type: Some("memo".into()), ..Default::default() }).is_err());
    }
}
