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

/// The fake's answer to a Kaluta triage prompt (spec §14.13), or `None`
/// for any other prompt. Per `<email thread_id="…">` block: a question
/// gets a quick reply with two answers, mail from a no-reply address is
/// archived, a long email becomes a task (needing a reply, with two
/// options), the rest stay in the Inbox.
pub fn triage_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta triage suggestions") {
        return None;
    }
    let mut items = Vec::new();
    for block in prompt.split("<email thread_id=\"").skip(1) {
        let Some((id, rest)) = block.split_once('"') else { continue };
        let rest = rest.split("</email>").next().unwrap_or(rest);
        let from = rest.lines().find_map(|l| l.strip_prefix("From: ")).unwrap_or_default().to_lowercase();
        let name = from.split(['<', ' ']).next().unwrap_or_default();
        let name = name
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect::<String>() + &name[c.len_utf8()..])
            .unwrap_or_default();
        let body = rest.split_once("\n\n").map(|(_, b)| b).unwrap_or_default();
        let esc = |s: &str| s.replace(['\\', '"'], "");
        // A long one is summarised: its first sentence.
        let summary = if body.len() > 600 {
            let first = body.split_terminator(['.', '!', '?']).next().unwrap_or_default();
            esc(&format!("It says: {}.", first.split_whitespace().collect::<Vec<_>>().join(" ")))
        } else {
            String::new()
        };
        items.push(if from.contains("no-reply") || from.contains("noreply") || from.contains("notifications")
            || from.contains("newsletter") || from.contains("digest")
        {
            // What kind it is, by its address.
            let category =
                if from.contains("newsletter") || from.contains("digest") { "newsletter" } else { "notification" };
            format!(
                r#"{{"thread_id": "{id}", "kind": "archive", "reason": "Nothing is asked of you.", "summary": "{summary}", "category": "{category}"}}"#
            )
        } else if body.len() > 600 {
            format!(
                r#"{{"thread_id": "{id}", "kind": "task", "reason": "It needs more than a quick answer.", "needs_reply": true, "options": ["Agree", "Ask for more time"], "task": {{"title": "Answer {}", "category": "Reply", "due": null, "action": "reply", "why": "A longer answer is needed."}}}}"#,
                esc(&name)
            )
        } else if body.contains('?') {
            format!(
                r#"{{"thread_id": "{id}", "kind": "reply", "seconds": 20, "reason": "{} asks something quick.", "needs_reply": true, "options": ["Yes", "Not this week"], "answers": [{{"label": "Yes", "text": "Hi {},\n\nYes, that works for me.\n\nJo"}}, {{"label": "Not this week", "text": "Hi {},\n\nNot this week, sorry. Could we look at next week?\n\nJo"}}]}}"#,
                esc(&name),
                esc(&name),
                esc(&name)
            )
        } else {
            format!(r#"{{"thread_id": "{id}", "kind": "leave", "reason": "Worth seeing again."}}"#)
        });
    }
    Some(format!(r#"{{"suggestions": [{}]}}"#, items.join(", ")))
}

/// The fake's answer to a Kaluta reply answers prompt (spec §14.13), or
/// `None`: one email per listed answer, to the email's sender; an email
/// asking for "facts about me" gets questions instead, as writing help's do.
pub fn reply_answers_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta reply answers") {
        return None;
    }
    let block = prompt.split("<email thread_id=\"").nth(1).unwrap_or_default();
    if block.contains("facts about me") {
        return Some(
            r#"{"questions": [{"question": "What is your role?", "category": "work", "label": "Role"}]}"#.into(),
        );
    }
    let from = block.lines().find_map(|l| l.strip_prefix("From: ")).unwrap_or_default();
    let name: String = from.split(['<', ' ', '@']).next().unwrap_or_default().replace(['\\', '"'], "");
    let options = prompt
        .split("Answers to write:\n")
        .nth(1)
        .unwrap_or_default()
        .lines()
        .map_while(|l| l.strip_prefix("- "))
        .map(|o| o.replace(['\\', '"'], ""));
    // Free times given: the first two, each on a line of its own.
    let times: String = prompt
        .split_once("free at these times")
        .map(|(_, list)| {
            list.lines().skip(1).map_while(|l| l.strip_prefix("- ")).take(2).map(|t| format!("\\n- {t}")).collect()
        })
        .unwrap_or_default();
    let answers: Vec<String> =
        options.map(|o| format!(r#"{{"label": "{o}", "text": "Hi {name},\n\n{o}.{times}\n\nJo"}}"#)).collect();
    Some(format!(r#"{{"answers": [{}], "missing": []}}"#, answers.join(", ")))
}

/// The fake's answer to a Kaluta waiting check (spec §14.13), or `None`
/// for any other prompt: a sent email with a question in it expects an
/// answer, the rest do not.
pub fn waiting_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta waiting check") {
        return None;
    }
    let mut items = Vec::new();
    for block in prompt.split("<email thread_id=\"").skip(1) {
        let Some((id, rest)) = block.split_once('"') else { continue };
        let body = rest.split("</email>").next().unwrap_or(rest).split_once("\n\n").map(|(_, b)| b).unwrap_or("");
        let (expects, reason) =
            if body.contains('?') { (true, "It asks them something.") } else { (false, "Nothing is asked of them.") };
        items.push(format!(r#"{{"thread_id": "{id}", "expects": {expects}, "reason": "{reason}"}}"#));
    }
    Some(format!(r#"{{"checks": [{}]}}"#, items.join(", ")))
}

/// The fake's answer to a Kaluta nudges prompt (spec §14.13), or `None`:
/// three follow-ups for each email, addressed to its recipient.
pub fn nudge_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta nudges") {
        return None;
    }
    let mut items = Vec::new();
    for block in prompt.split("<email thread_id=\"").skip(1) {
        let Some((id, rest)) = block.split_once('"') else { continue };
        let to = rest.lines().find_map(|l| l.strip_prefix("To: ")).unwrap_or_default();
        let name = to.split([' ', '<', '@']).next().unwrap_or_default().replace(['"', '\\'], "");
        let name = name
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect::<String>() + &name[c.len_utf8()..])
            .unwrap_or_default();
        items.push(format!(
            r#"{{"thread_id": "{id}", "answers": [{{"label": "Gentle nudge", "text": "Hi {name},\n\nJust bringing this back to the top of your inbox. Any news?\n\nJo"}}, {{"label": "Offer a call", "text": "Hi {name},\n\nWould a quick call be easier? I'm free most afternoons this week.\n\nJo"}}, {{"label": "Set a date", "text": "Hi {name},\n\nCould you let me know by Friday? I'd like to settle this before next week.\n\nJo"}}]}}"#
        ));
    }
    Some(format!(r#"{{"nudges": [{}]}}"#, items.join(", ")))
}

/// The fake's answer to a Kaluta reading prompt (spec §14.13), or `None`:
/// the subject as the line, the first sentence of the body as the summary.
pub fn reading_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta reading summaries") {
        return None;
    }
    let esc = |s: &str| s.replace(['\\', '"', '\n'], " ");
    let mut items = Vec::new();
    for block in prompt.split("<email thread_id=\"").skip(1) {
        let Some((id, rest)) = block.split_once('"') else { continue };
        let rest = rest.split("</email>").next().unwrap_or(rest);
        let subject = rest.lines().find_map(|l| l.strip_prefix("Subject: ")).unwrap_or("A filed email");
        let body = rest.split_once("\n\n").map(|(_, b)| b.trim()).unwrap_or("");
        let first = body.split_inclusive(['.', '!', '?']).next().unwrap_or(body);
        items.push(format!(
            r#"{{"thread_id": "{id}", "line": "{}", "summary": "{}"}}"#,
            esc(subject),
            esc(first.trim())
        ));
    }
    Some(format!(r#"{{"summaries": [{}]}}"#, items.join(", ")))
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
/// for "facts about me": the questions for facts it does not have, the
/// first with answers to pick from (the first turn only; the answers
/// come in a turn of their own).
pub fn writing_help_answer(prompt: &str) -> Option<String> {
    let first = prompt.lines().next()?;
    if !first.starts_with("You are helping write an email in Kaluta's composer.") || !first.contains("facts about me") {
        return None;
    }
    Some(
        r#"{"questions": [{"question": "What is your role?", "category": "work", "label": "Occupation or role", "options": ["Founder", "CTO", "Engineer"]}, {"question": "What does your company do?", "category": "work", "label": "What the company does"}]}"#
            .into(),
    )
}

/// The fake's answer to a Kaluta task-match prompt (spec §14.8), or
/// `None` for any other prompt: every listed task whose title shares two
/// words or more (of three letters or more) with the email's subject.
pub fn task_match_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta task match") {
        return None;
    }
    let words = |s: &str| -> Vec<String> {
        s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() >= 3).map(str::to_owned).collect()
    };
    let subject = words(prompt.split("<email>").nth(1)?.lines().find_map(|l| l.strip_prefix("Subject: "))?);
    let mut found = Vec::new();
    for block in prompt.split("<task id=\"").skip(1) {
        let Some((id, rest)) = block.split_once('"') else { continue };
        let title = rest.lines().find_map(|l| l.strip_prefix("Title: ")).unwrap_or_default();
        if words(title).iter().filter(|w| subject.contains(w)).count() >= 2 {
            found.push(format!(r#"{{"task_id": {id}, "why": "The same piece of work."}}"#));
        }
    }
    Some(format!(r#"{{"matches": [{}]}}"#, found.join(", ")))
}

/// The fake's answer to a Kaluta meeting follow-up prompt (spec §14.14),
/// or `None` for any other prompt: a follow-up with two options when the
/// meeting was with anyone else (the first address on its `With:` line),
/// none otherwise.
pub fn followup_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta meeting follow-ups") {
        return None;
    }
    let id = prompt.split("<meeting id=\"").nth(1)?.split('"').next()?.to_owned();
    let with = prompt.lines().find_map(|l| l.strip_prefix("With: ")).unwrap_or_default();
    let email = with.split('<').nth(1).and_then(|r| r.split('>').next());
    let title =
        prompt.lines().find_map(|l| l.strip_prefix("Title: ")).unwrap_or("the meeting").replace(['"', '\\'], "");
    Some(match email {
        Some(email) => {
            let name = with.split(" <").next().unwrap_or("there").split(' ').next().unwrap_or("there");
            format!(
                r#"{{"meeting_id": "{id}", "follow_up": true, "reason": "You agreed to send something after {title}.", "options": [{{"label": "Send what you promised", "to": ["{email}"], "subject": "Following up: {title}", "body": "Hi {name},\n\nThanks for today. As promised, here is what we discussed; the details are attached.\n\nJo"}}, {{"label": "Thank and recap", "to": ["{email}"], "subject": "Thanks, and next steps", "body": "Hi {name},\n\nThanks for the time today. Next steps: I send the proposal by Friday, you share it with your team.\n\nJo"}}]}}"#
            )
        }
        None => format!(r#"{{"meeting_id": "{id}", "follow_up": false, "reason": "No one else was there."}}"#),
    })
}

/// The fake's answer to a Kaluta meeting prep prompt: a brief naming each
/// person it was given, or `None` for any other prompt.
pub fn prep_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta meeting prep") {
        return None;
    }
    let names: Vec<&str> = prompt.split("<person name=\"").skip(1).filter_map(|p| p.split('"').next()).collect();
    let mut lines: Vec<String> = names.iter().map(|n| format!("- {n}: last wrote about the proposal.")).collect();
    lines.push("- You owe: the pricing for 50 seats.".into());
    Some(lines.join("\n"))
}

/// The fake's answer to a Kaluta contact facts prompt (spec §14.16), or
/// `None`. It reads each `<message>` for a few plain phrasings:
/// "I'll …" / "I will …" is a commitment (owed by whoever said it),
/// "Let's discuss …" a topic, "I'm the …" a role (and a tag when the list
/// has one of that name), "as your <tag>" a tag, and "That's done: <label>"
/// closes the known item with that label. A line `AGENT-ANSWER: {…}` in a
/// message is copied into the answer as it stands: a reader that a planted
/// message has talked round completely, for the adversarial tests.
pub fn contact_facts_answer(prompt: &str) -> Option<String> {
    if !prompt.starts_with("Kaluta contact facts") {
        return None;
    }
    let attr = |tag: &str, name: &str| -> Option<String> {
        let at = tag.find(&format!("{name}=\""))? + name.len() + 2;
        Some(tag[at..].split('"').next()?.to_owned())
    };
    let contacts: Vec<String> =
        prompt.split("<contact ").skip(1).filter_map(|b| attr(b.split('>').next()?, "address")).collect();
    // Known items: "[#12] kind …: Label — Value".
    let known: Vec<(i64, String)> = prompt
        .lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("[#")?;
            let (id, rest) = rest.split_once(']')?;
            let label = rest.split_once(": ")?.1.split(" — ").next()?.trim().to_owned();
            Some((id.parse().ok()?, label))
        })
        .collect();
    let tags: Vec<String> = prompt
        .split("Tags you may choose from")
        .nth(1)
        .and_then(|t| t.split("\n\n").next())
        .map(|block| {
            block
                .lines()
                .filter_map(|l| l.split_once(": "))
                .flat_map(|(_, names)| names.split(", ").map(|n| n.trim().to_owned()).collect::<Vec<_>>())
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let cut = |s: &str, n: usize| -> String {
        let s = s.trim().trim_end_matches(['.', '!', '?', ',', ';']).trim();
        let mut c = s.chars();
        let first: String = c.next().map(|f| f.to_uppercase().collect()).unwrap_or_default();
        (first + c.as_str()).chars().take(n).collect()
    };
    let after = |sentence: &str, phrases: &[&str]| -> Option<String> {
        let lower = sentence.to_lowercase();
        phrases.iter().find_map(|p| lower.find(p).map(|at| sentence[at + p.len()..].to_owned()))
    };
    let mut items: Vec<serde_json::Value> = Vec::new();
    for block in prompt.split("<message ").skip(1) {
        let Some((head, rest)) = block.split_once(">\n") else { continue };
        let body = rest.split("</message>").next().unwrap_or(rest);
        let (Some(id), Some(from), Some(by)) = (attr(head, "id"), attr(head, "from"), attr(head, "by")) else {
            continue;
        };
        let date = attr(head, "date").unwrap_or_default();
        for line in body.lines() {
            if let Some(raw) = line.trim().strip_prefix("AGENT-ANSWER:")
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(raw.trim())
            {
                items.push(v);
            }
        }
        let contact = if by == "you" {
            contacts.first().cloned()
        } else {
            contacts.iter().find(|c| c.eq_ignore_ascii_case(&from)).cloned()
        };
        let Some(contact) = contact else { continue };
        for sentence in body.split(['.', '!', '?', '\n']).map(str::trim).filter(|s| !s.is_empty()) {
            if sentence.starts_with("AGENT-ANSWER:") {
                continue;
            }
            let quotes = serde_json::json!([{ "message": id, "quote": sentence }]);
            if let Some(rest) = after(sentence, &["i'll ", "i will "]) {
                items.push(serde_json::json!({
                    "op": "new", "contact": contact, "kind": "commitment", "label": cut(&rest, 60),
                    "value": format!("Said on {date}"), "class": "other",
                    "owed_by": if by == "you" { "you" } else { "them" }, "quotes": quotes,
                }));
            } else if let Some(rest) = after(sentence, &["let's discuss ", "let's talk about ", "can we discuss "]) {
                items.push(serde_json::json!({
                    "op": "new", "contact": contact, "kind": "topic", "label": cut(&rest, 60),
                    "value": format!("Raised on {date}"), "class": "topic", "quotes": quotes,
                }));
            } else if let Some(rest) = after(sentence, &["that's done: "])
                && let Some((known_id, _)) = known.iter().find(|(_, l)| l.eq_ignore_ascii_case(&cut(&rest, 60)))
            {
                items.push(serde_json::json!({ "op": "close", "id": known_id, "quotes": quotes }));
            }
            if by == "you" {
                continue;
            }
            if let Some(rest) = after(sentence, &["i'm the ", "i am the "]) {
                items.push(serde_json::json!({
                    "op": "new", "contact": contact, "kind": "fact", "label": "Role", "value": cut(&rest, 160),
                    "class": "role", "quotes": quotes,
                }));
                let said = rest.to_lowercase();
                if let Some(tag) = tags.iter().find(|t| {
                    let t = t.to_lowercase();
                    said == t || said.starts_with(&format!("{t} ")) || said.starts_with(&format!("{t},"))
                }) {
                    items.push(serde_json::json!({ "op": "tag", "contact": contact, "tag": tag, "quotes": quotes }));
                }
            }
            if let Some(rest) = after(sentence, &["as your "]) {
                let said = rest.to_lowercase();
                if let Some(tag) = tags.iter().find(|t| said.starts_with(&t.to_lowercase())) {
                    items.push(serde_json::json!({ "op": "tag", "contact": contact, "tag": tag, "quotes": quotes }));
                }
            }
        }
    }
    Some(serde_json::json!({ "items": items }).to_string())
}

/// The fake's answer to a deep dive's merge turn (spec §14.16): it joins
/// nothing and closes nothing.
pub fn contact_merge_answer(prompt: &str) -> Option<String> {
    prompt.starts_with("Kaluta contact merge").then(|| r#"{"join": [], "close": []}"#.to_owned())
}

/// The scripted answer to one of Kaluta's own prompts (tasks, the writing
/// guide, analysis, facts, writing help's questions), or `None`.
pub fn scripted_answer(prompt: &str) -> Option<String> {
    task_answer(prompt)
        .or_else(|| task_match_answer(prompt))
        .or_else(|| followup_answer(prompt))
        .or_else(|| prep_answer(prompt))
        .or_else(|| triage_answer(prompt))
        .or_else(|| reply_answers_answer(prompt))
        .or_else(|| waiting_answer(prompt))
        .or_else(|| nudge_answer(prompt))
        .or_else(|| reading_answer(prompt))
        .or_else(|| guide_answer(prompt))
        .or_else(|| change_answer(prompt))
        .or_else(|| merge_answer(prompt))
        .or_else(|| compare_answer(prompt))
        .or_else(|| glean_answer(prompt))
        .or_else(|| writing_help_answer(prompt))
        .or_else(|| contact_facts_answer(prompt))
        .or_else(|| contact_merge_answer(prompt))
}

struct FakeSession {
    sink: EventSink,
    external: String,
}

#[async_trait]
impl AgentSession for FakeSession {
    async fn send(&mut self, turn: TurnInput) -> AgentResult<()> {
        self.sink.emit(AgentEvent::TurnStarted);
        let text = scripted_answer(&turn.prompt).unwrap_or_else(|| format!("You said: {}", turn.prompt));
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
