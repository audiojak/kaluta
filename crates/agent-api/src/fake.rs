//! A scripted agent for tests of everything above the adapters.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use crate::{
    AgentEvent, AgentProvider, AgentResult, AgentSession, AgentStatus, EventSink, ProviderId, SessionConfig, TurnInput,
    Usage,
};

/// Replies "You said: <prompt>" to every turn, except Kaluta's task
/// prompts, which get [`task_answer`]'s fixed JSON.
pub struct FakeAgent {
    id: ProviderId,
    status: AgentStatus,
    detections: Arc<AtomicUsize>,
}

impl FakeAgent {
    pub fn ready(id: ProviderId) -> Self {
        Self {
            id,
            status: AgentStatus::Ready { version: "1.0.0 (fake)".into(), path: "/fake/agent".into() },
            detections: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn with_status(id: ProviderId, status: AgentStatus) -> Self {
        Self { id, status, detections: Arc::new(AtomicUsize::new(0)) }
    }

    pub fn detections(&self) -> usize {
        self.detections.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl AgentProvider for FakeAgent {
    fn id(&self) -> ProviderId {
        self.id
    }

    async fn detect(&self) -> AgentStatus {
        self.detections.fetch_add(1, Ordering::Relaxed);
        self.status.clone()
    }

    async fn start_session(&self, cfg: SessionConfig, sink: EventSink) -> AgentResult<Box<dyn AgentSession>> {
        let external = format!("fake-{}", cfg.session_id);
        sink.emit(AgentEvent::SessionStarted { external_id: Some(external.clone()) });
        Ok(Box::new(FakeSession { sink, external }))
    }
}

/// The fake's answer to a Kaluta task prompt (spec §14.8), or `None`
/// for any other prompt: one suggestion per `<email thread_id="…">` block,
/// titled after its subject, alternating between a reply due today and a
/// review with no date, so tests and snapshots see both.
pub fn task_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta task suggestions") {
        return None;
    }
    let today = prompt
        .split("Today is ")
        .nth(1)
        .and_then(|rest| rest.get(..10))
        .map(|d| format!("\"{d}\""))
        .unwrap_or_else(|| "null".into());
    let mut items = Vec::new();
    for (i, block) in prompt.split("<email thread_id=\"").skip(1).enumerate() {
        let Some((id, rest)) = block.split_once('"') else { continue };
        let subject = rest.lines().find_map(|l| l.strip_prefix("Subject: ")).map(str::trim).filter(|s| !s.is_empty());
        let subject = subject.unwrap_or("this email").replace(['"', '\\'], "");
        items.push(if i % 2 == 0 {
            format!(
                r#"{{"thread_id": "{id}", "title": "Reply about {subject}", "category": "Reply", "due": {today}, "action": "reply", "why": "The sender is waiting for an answer."}}"#
            )
        } else {
            format!(
                r#"{{"thread_id": "{id}", "title": "Review {subject}", "category": "Review", "due": null, "action": "none", "why": "Worth reading before it is archived."}}"#
            )
        });
    }
    Some(format!("[{}]", items.join(", ")))
}

/// The fake's answer to a Kaluta writing-guide analysis prompt (spec
/// §14.9), or `None` for any other prompt. For every message it proposes
/// the same few entries, quoting the message's own first words and last
/// line, so the quotes pass the core's check and the proposals merge
/// across batches; and one audience from the first recipient's domain.
pub fn guide_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta writing guide analysis") {
        return None;
    }
    let esc = |s: &str| s.replace(['\\', '"'], "");
    let mut voice = Vec::new();
    let mut signoff = Vec::new();
    let mut domain = None;
    for block in prompt.split("<message id=\"").skip(1) {
        let Some((id, rest)) = block.split_once('"') else { continue };
        let body = rest.split("</message>").next().unwrap_or("");
        if domain.is_none() {
            domain = body
                .lines()
                .find_map(|l| l.strip_prefix("To: "))
                .and_then(|to| to.split('@').nth(1))
                .map(|d| d.trim_end_matches(['>', ',', ' ']).split([',', ' ', '>']).next().unwrap_or("").to_owned())
                .filter(|d| !d.is_empty());
        }
        let text: Vec<&str> =
            body.lines().skip_while(|l| !l.trim().is_empty()).map(str::trim).filter(|l| !l.is_empty()).collect();
        if let Some(first) = text.first() {
            let words: Vec<&str> = first.split_whitespace().take(4).collect();
            voice.push(format!(r#"{{"message_id": "{id}", "quote": "{}"}}"#, esc(&words.join(" "))));
        }
        if let Some(last) = text.last().filter(|l| text.len() > 1 && l.split_whitespace().count() <= 3) {
            signoff.push(format!(r#"{{"message_id": "{id}", "quote": "{}"}}"#, esc(last)));
        }
    }
    let mut proposals = Vec::new();
    if !voice.is_empty() {
        proposals.push(format!(
            r#"{{"category": "A1", "kind": "guideline", "statement": "Keep a friendly, direct tone", "evidence": [{}]}}"#,
            voice.join(", ")
        ));
        proposals.push(format!(
            r#"{{"category": "C8", "kind": "rule", "statement": "Never write 'circle back'", "check": {{"kind": "banned_phrase", "value": "circle back"}}, "evidence": [{}]}}"#,
            voice[0]
        ));
    }
    if !signoff.is_empty() {
        proposals.push(format!(
            r#"{{"category": "B6", "kind": "guideline", "statement": "Sign off with the first name only", "scope": {{"message_types": ["reply"]}}, "evidence": [{}]}}"#,
            signoff.join(", ")
        ));
    }
    let audiences = domain
        .map(|d| {
            format!(r#"{{"name": "Colleagues", "description": "People the user works with", "members": ["@{d}"]}}"#)
        })
        .unwrap_or_default();
    Some(format!(r#"{{"proposals": [{}], "audiences": [{audiences}]}}"#, proposals.join(", ")))
}

/// The fake's answer to a Kaluta writing-guide change prompt: one
/// question adding the request as a guideline, and one removing the first
/// entry in the guide, if there is one.
pub fn change_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta writing guide change") {
        return None;
    }
    let request = prompt.split("<<<\n").nth(1)?.split("\n>>>").next()?.trim().replace(['"', '\\'], "");
    let mut questions = vec![format!(
        r#"{{"question": "Add “{request}” to your guide?", "before": "nothing", "after": "{request}", "edits": [{{"op": "add", "category": "C1", "kind": "rule", "statement": "{request}"}}]}}"#
    )];
    let first = prompt.lines().find_map(|l| l.strip_prefix('#')?.split(' ').next()?.parse::<i64>().ok());
    if let Some(id) = first {
        questions.push(format!(
            r#"{{"question": "Remove the entry it replaces?", "before": "entry {id}", "after": "nothing", "edits": [{{"op": "remove", "id": {id}}}]}}"#
        ));
    }
    Some(format!(r#"{{"questions": [{}]}}"#, questions.join(", ")))
}

/// The fake's answer to a Kaluta writing-guide merge prompt: one
/// decision per category listed, covering its entries.
pub fn merge_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta writing guide merge") {
        return None;
    }
    let mut decisions: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    for line in prompt.lines() {
        if let Some(cat) = line.strip_prefix("Category ").and_then(|l| l.strip_suffix(':')) {
            decisions.push((cat.to_owned(), vec![], vec![]));
        } else if let Some(rest) = line.trim().strip_prefix("mine #") {
            if let (Some(d), Some(id)) = (decisions.last_mut(), rest.split(' ').next()) {
                d.1.push(id.to_owned());
            }
        } else if let Some(rest) = line.trim().strip_prefix("incoming @")
            && let (Some(d), Some(i)) = (decisions.last_mut(), rest.split(' ').next())
        {
            d.2.push(i.to_owned());
        }
    }
    let items: Vec<String> = decisions
        .iter()
        .map(|(cat, mine, incoming)| {
            format!(
                r#"{{"point": "Category {cat}", "summary": "The guides differ on {cat}", "mine": [{}], "incoming": [{}]}}"#,
                mine.join(", "),
                incoming.join(", ")
            )
        })
        .collect();
    Some(format!(r#"{{"decisions": [{}]}}"#, items.join(", ")))
}

/// The fake's answer to a Kaluta analysis comparison prompt: for each
/// pair whose sent text ends on a shorter line than the AI's (fewer
/// characters), "Sign off
/// with the first name only" (a guideline); for each pair whose AI text
/// says "I hope this finds you well" and whose sent text does not, a rule
/// banning it.
pub fn compare_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta analysis compare") {
        return None;
    }
    let esc = |s: &str| s.replace(['\\', '"'], "");
    let block = |text: &str, tag: &str| -> String {
        text.split(&format!("<{tag}>\n"))
            .nth(1)
            .and_then(|r| r.split(&format!("\n</{tag}>")).next())
            .unwrap_or("")
            .to_owned()
    };
    let mut signoff = Vec::new();
    let mut hope = Vec::new();
    for pair in prompt.split("<pair id=\"").skip(1) {
        let Some((id, rest)) = pair.split_once('"') else { continue };
        let (ai, sent) = (block(rest, "ai"), block(rest, "sent"));
        let last = |t: &str| t.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned();
        let (ai_last, sent_last) = (last(&ai), last(&sent));
        if !sent_last.is_empty() && sent_last != ai_last && sent_last.chars().count() < ai_last.chars().count() {
            signoff.push(format!(r#"{{"pair": {id}, "sent": "{}", "ai": "{}"}}"#, esc(&sent_last), esc(&ai_last)));
        }
        let phrase = "I hope this finds you well";
        if ai.contains(phrase) && !sent.contains(phrase) {
            hope.push(format!(r#"{{"pair": {id}, "ai": "{phrase}"}}"#));
        }
    }
    let mut proposals = Vec::new();
    if !signoff.is_empty() {
        proposals.push(format!(
            r#"{{"op": "add", "category": "B6", "kind": "guideline", "statement": "Sign off with the first name only", "evidence": [{}]}}"#,
            signoff.join(", ")
        ));
    }
    if !hope.is_empty() {
        proposals.push(format!(
            r#"{{"op": "add", "category": "C8", "kind": "rule", "statement": "Never write 'I hope this finds you well'", "evidence": [{}]}}"#,
            hope.join(", ")
        ));
    }
    Some(format!(r#"{{"proposals": [{}]}}"#, proposals.join(", ")))
}

/// The fake's answer to a Kaluta fact-gleaning prompt: for each message
/// with a line "I'm <role> at <organisation>.", that role as Work ›
/// Occupation or role, quoting the line; and any line naming a password,
/// so tests see it dropped.
pub fn glean_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta facts glean") {
        return None;
    }
    let esc = |s: &str| s.replace(['\\', '"'], "");
    let mut facts = Vec::new();
    for block in prompt.split("<message id=\"").skip(1) {
        let Some((id, rest)) = block.split_once('"') else { continue };
        let body = rest.split("</message>").next().unwrap_or("");
        for line in body.lines().map(str::trim) {
            if let Some(role) = line.strip_prefix("I'm ").and_then(|l| l.split(" at ").next())
                && line.contains(" at ")
            {
                facts.push(format!(
                    r#"{{"op": "add", "category": "work", "label": "Occupation or role", "value": "{}", "evidence": {{"message_id": "{id}", "quote": "{}"}}}}"#,
                    esc(role),
                    esc(line)
                ));
            }
            if line.to_lowercase().contains("password") {
                facts.push(format!(
                    r#"{{"op": "add", "category": "other", "label": "Wi-Fi password", "value": "{}", "evidence": {{"message_id": "{id}", "quote": "{}"}}}}"#,
                    esc(line),
                    esc(line)
                ));
            }
        }
    }
    Some(format!(r#"{{"facts": [{}], "categories": [], "starter_set": null}}"#, facts.join(", ")))
}

/// The fake's answer to the composer's writing help when the request asks
/// for "facts about me": the questions for facts it does not have (the
/// first turn only; the answers come in a turn of their own).
pub fn writing_help_answer(prompt: &str) -> Option<String> {
    let first = prompt.lines().next()?;
    if !first.starts_with("You are helping write an email in Kaluta's composer.") || !first.contains("facts about me") {
        return None;
    }
    Some(
        r#"{"questions": [{"question": "What is your role?", "category": "work", "label": "Occupation or role"}, {"question": "What does your company do?", "category": "work", "label": "What the company does"}]}"#
            .into(),
    )
}

struct FakeSession {
    sink: EventSink,
    external: String,
}

#[async_trait]
impl AgentSession for FakeSession {
    async fn send(&mut self, turn: TurnInput) -> AgentResult<()> {
        self.sink.emit(AgentEvent::TurnStarted);
        let text = task_answer(&turn.prompt)
            .or_else(|| guide_answer(&turn.prompt))
            .or_else(|| change_answer(&turn.prompt))
            .or_else(|| merge_answer(&turn.prompt))
            .or_else(|| compare_answer(&turn.prompt))
            .or_else(|| glean_answer(&turn.prompt))
            .or_else(|| writing_help_answer(&turn.prompt))
            .unwrap_or_else(|| format!("You said: {}", turn.prompt));
        self.sink.emit(AgentEvent::TextDelta { text });
        self.sink.emit(AgentEvent::TurnCompleted {
            usage: Some(Usage { input_tokens: 10, output_tokens: 5, cached_input_tokens: 0 }),
            cost_usd: None,
        });
        Ok(())
    }

    async fn cancel(&mut self) -> AgentResult<()> {
        self.sink.emit(AgentEvent::TurnFailed { message: "cancelled".into() });
        Ok(())
    }

    fn external_id(&self) -> Option<String> {
        Some(self.external.clone())
    }

    async fn close(&mut self) {
        self.sink.emit(AgentEvent::SessionEnded);
    }
}
