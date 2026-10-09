//! The one handler set behind both surfaces (spec §10.6): what
//! `guide_rules`, `facts_lookup` and `check_draft` answer, over MCP and over
//! REST alike, from a mailbox's newest snapshot. The answers are mailbox mode's
//! (§10.1, `docs/mcp.md`) plus the snapshot's `version` and
//! `published_at`, the time the guide is "as of". There is no `send_mode`:
//! a cloud agent sends through the service, not through Kaluta.

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

/// `check_draft`'s arguments: a draft as `mail_send` would take it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckArgs {
    #[serde(default)]
    pub to: Vec<String>,
    pub message_type: Option<String>,
    #[serde(default)]
    pub subject: String,
    pub body_markdown: String,
}

pub const MESSAGE_TYPES: [&str; 3] = ["new", "reply", "forward"];

/// The longest body `check_draft` and `report_send` take: mail services
/// refuse bigger ones anyway.
pub const MAX_BODY_MARKDOWN: usize = 256 * 1024;

fn check_message_type(t: Option<&String>) -> Result<(), String> {
    match t {
        Some(t) if !MESSAGE_TYPES.contains(&t.as_str()) => {
            Err(format!("message_type must be one of new, reply or forward, not {t:?}"))
        }
        _ => Ok(()),
    }
}

/// Why arguments were refused, in words for the agent.
pub fn check_guide_args(a: &GuideArgs) -> Result<(), String> {
    check_message_type(a.message_type.as_ref())
}

/// Why a draft to check was refused, in words for the agent.
pub fn check_draft_args(a: &CheckArgs) -> Result<(), String> {
    check_message_type(a.message_type.as_ref())?;
    if a.body_markdown.len() > MAX_BODY_MARKDOWN {
        return Err(format!("body_markdown is over {} KB", MAX_BODY_MARKDOWN / 1024));
    }
    Ok(())
}

/// A message's type from its subject, as mailbox mode reads a draft's:
/// `Fwd:` a forward, `Re:` a reply, anything else new.
pub fn message_type_of(subject: &str) -> &'static str {
    let s = subject.trim().to_lowercase();
    if s.starts_with("fwd:") || s.starts_with("fw:") {
        "forward"
    } else if s.starts_with("re:") {
        "reply"
    } else {
        "new"
    }
}

/// A Markdown body's text as the guide's check reads it: what a reader
/// sees, without Markdown's marks. Mailbox mode renders the body to HTML
/// and takes the HTML's text; this gives the same words, down to a table's
/// cells running together as they do there. Raw HTML in the Markdown is
/// text, as there.
pub fn markdown_text(markdown: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser, TagEnd};
    let mut out = String::new();
    for event in Parser::new_ext(markdown, Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES) {
        match event {
            Event::Text(t) | Event::Code(t) | Event::Html(t) | Event::InlineHtml(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak | Event::Rule => out.push('\n'),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Item
                | TagEnd::CodeBlock
                | TagEnd::BlockQuote(_)
                | TagEnd::TableHead
                | TagEnd::TableRow,
            ) => out.push('\n'),
            _ => {}
        }
    }
    out.trim().to_owned()
}

/// What a draft breaks in the guide for its recipients and type:
/// `guide_check`'s messages, as mailbox mode's draft tools answer them.
pub fn guide_check(s: &Snapshot, to: &[String], message_type: Option<&str>, subject: &str, body: &str) -> Vec<String> {
    let target = Target {
        recipients: recipients(to),
        message_type: Some(message_type.unwrap_or_else(|| message_type_of(subject)).to_owned()),
        audiences: None,
    };
    s.check(&target, &markdown_text(body)).into_iter().map(|f| f.message).collect()
}

fn recipients(to: &[String]) -> Vec<String> {
    to.iter().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect()
}

/// `check_draft`: what the draft breaks (`guide_check`, empty when
/// nothing), and which snapshot said so.
pub fn check_draft(s: &Snapshot, a: &CheckArgs) -> Value {
    json!({
        "guide_check": guide_check(s, &a.to, a.message_type.as_deref(), &a.subject, &a.body_markdown),
        "guide_version": s.guide_version,
        "version": s.version,
        "published_at": time(s.published_at),
    })
}

/// An instant as agents read it: RFC 3339 in UTC, to the second.
pub fn time(ms: i64) -> Value {
    chrono::DateTime::from_timestamp_millis(ms)
        .map_or(Value::Null, |t| json!(t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)))
}

/// `guide_rules`: the guide for these recipients and this type, whose
/// mailbox it is and the name it sends as.
pub fn guide_rules(s: &Snapshot, a: &GuideArgs) -> Value {
    let target = Target { recipients: recipients(&a.to), message_type: a.message_type.clone(), audiences: None };
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
    fn markdown_reads_as_its_words() {
        assert_eq!(
            markdown_text("Hi **Ann**,\n\nLet's *circle* back.\n\n- one\n- two"),
            "Hi Ann,\nLet's circle back.\none\ntwo"
        );
        assert_eq!(markdown_text("Use <b>bold</b> `code`"), "Use <b>bold</b> code");
        assert_eq!(message_type_of(" Re: plan"), "reply");
        assert_eq!(message_type_of("FWD: plan"), "forward");
        assert_eq!(message_type_of("Plan"), "new");
    }

    #[test]
    fn message_types_are_mailbox_modes() {
        assert!(check_guide_args(&GuideArgs { message_type: Some("reply".into()), ..Default::default() }).is_ok());
        assert!(check_guide_args(&GuideArgs { message_type: Some("memo".into()), ..Default::default() }).is_err());
    }
}
