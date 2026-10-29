//! The workflow demo's agent (docs/plans/design-review.md): the scripted
//! fake, plus a few requests it carries out with Kaluta's real tools over
//! the session's socket, as a CLI would through the `kaluta-mcp` shim. So
//! result lists, drafts "Created by Claude", approval cards and routine
//! runs are the real thing, made without a model. Requests it does not
//! recognise get the fake's echo.

use std::path::PathBuf;
use std::sync::Arc;

use agent_api::fake::{FakeAgent, scripted_answer};
use agent_api::{
    AgentEvent, AgentProvider, AgentResult, AgentSession, AgentStatus, EventSink, ProviderId, SessionConfig, TurnInput,
    Usage, shorten, summarize_args,
};
use agent_mcp::{Outcome, ShimClient};
use async_trait::async_trait;
use serde_json::{Value, json};

/// The user's address in the demo mailbox.
const ME: &str = mail_store::demo::ME;

pub(crate) struct DemoAgent(FakeAgent);

impl DemoAgent {
    pub(crate) fn ready(id: ProviderId) -> Self {
        Self(FakeAgent::ready(id))
    }
}

#[async_trait]
impl AgentProvider for DemoAgent {
    fn id(&self) -> ProviderId {
        self.0.id()
    }

    async fn detect(&self) -> AgentStatus {
        self.0.detect().await
    }

    async fn start_session(&self, cfg: SessionConfig, sink: EventSink) -> AgentResult<Box<dyn AgentSession>> {
        let external = format!("demo-{}", cfg.session_id);
        sink.emit(AgentEvent::SessionStarted { external_id: Some(external.clone()) });
        Ok(Box::new(DemoSession {
            sink: sink.clone(),
            external,
            worker: Arc::new(tokio::sync::Mutex::new(Worker {
                sink,
                socket: cfg.mcp.socket_path,
                session: cfg.session_id.0,
                client: None,
                calls: 0,
            })),
            turn: None,
        }))
    }
}

struct DemoSession {
    sink: EventSink,
    external: String,
    worker: Arc<tokio::sync::Mutex<Worker>>,
    turn: Option<tokio::task::JoinHandle<()>>,
}

/// What a turn works with; one turn at a time.
struct Worker {
    sink: EventSink,
    socket: PathBuf,
    session: String,
    client: Option<ShimClient>,
    calls: u32,
}

#[async_trait]
impl AgentSession for DemoSession {
    /// Returns at once, like a CLI taking a prompt: the turn runs on its
    /// own, since a tool call may wait for the user's approval.
    async fn send(&mut self, turn: TurnInput) -> AgentResult<()> {
        self.sink.emit(AgentEvent::TurnStarted);
        let worker = self.worker.clone();
        self.turn = Some(tokio::spawn(async move {
            let mut w = worker.lock().await;
            let text = match scripted_answer(&turn.prompt) {
                Some(text) => text,
                None => w.answer(&turn).await,
            };
            w.sink.emit(AgentEvent::TextDelta { text });
            w.sink.emit(AgentEvent::TurnCompleted {
                usage: Some(Usage { input_tokens: 10, output_tokens: 5, cached_input_tokens: 0 }),
                cost_usd: None,
            });
        }));
        Ok(())
    }

    async fn cancel(&mut self) -> AgentResult<()> {
        if let Some(turn) = self.turn.take() {
            turn.abort();
        }
        self.sink.emit(AgentEvent::TurnFailed { message: "cancelled".into() });
        Ok(())
    }

    fn external_id(&self) -> Option<String> {
        Some(self.external.clone())
    }

    async fn close(&mut self) {
        if let Some(turn) = self.turn.take() {
            turn.abort();
        }
        self.sink.emit(AgentEvent::SessionEnded);
    }
}

/// A thread as a search returns it.
struct Found {
    id: String,
    subject: String,
    sender: String,
    labels: Vec<String>,
}

impl Found {
    fn from(v: &Value) -> Option<Self> {
        Some(Self {
            id: v["thread_id"].as_str()?.to_owned(),
            subject: v["subject"].as_str().unwrap_or_default().to_owned(),
            sender: v["participants"]
                .as_array()
                .and_then(|p| p.iter().filter_map(Value::as_str).find(|a| !a.contains(ME)))
                .unwrap_or_default()
                .to_owned(),
            labels: v["labels"]
                .as_array()
                .map(|l| l.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default(),
        })
    }

    fn name(&self) -> &str {
        self.sender.split(" <").next().unwrap_or(&self.sender)
    }
}

impl Worker {
    async fn answer(&mut self, turn: &TurnInput) -> String {
        let prompt = turn.prompt.as_str();
        let p = prompt.to_lowercase();
        let selected: Vec<String> = turn.context.selected_thread_ids.iter().map(|t| t.0.clone()).collect();
        if prompt.starts_with("You are helping write an email in Kaluta's composer.") {
            if let Some(answer) = self.meeting(prompt).await {
                return answer;
            }
            return writing_help(prompt);
        }
        if prompt.starts_with("Your draft breaks the user's writing guide") {
            return rewrite(prompt);
        }
        if prompt.starts_with("This is an automated, unattended run.") {
            return self.sort(prompt, false).await;
        }
        if prompt.starts_with("This is a preview.") {
            return self.sort(prompt, true).await;
        }
        if p.contains("draft") || p.contains("write a reply") || p.contains("reply to") || p.starts_with("respond") {
            return self.draft_reply(&selected, p.contains("send")).await;
        }
        if (p.contains("need") && p.contains("repl")) || p.contains("waiting on me") || p.contains("owe") {
            return self.needs_reply().await;
        }
        if p.contains("waiting")
            || p.contains("follow up")
            || p.contains("follow-up")
            || p.contains("chase")
            || p.contains("heard back")
            || p.contains("no reply")
        {
            return self.waiting_on_others().await;
        }
        if p.contains("summar") && !selected.is_empty() {
            return self.summarise(&selected[0]).await;
        }
        if (p.contains("new") && (p.contains("since") || p.contains("what's") || p.contains("today")))
            || p.contains("catch me up")
        {
            return self.whats_new().await;
        }
        if p.contains("archive") && !selected.is_empty() {
            return match self.call("mail_archive", json!({ "thread_ids": selected })).await {
                Some(_) => format!("Archived {}.", conversations(selected.len())),
                None => "I couldn't archive those.".into(),
            };
        }
        format!("You said: {prompt}")
    }

    /// Writing help for an email that asks to meet, when the user said no
    /// time (spec §14.9): their Calendar link from the facts, or a question
    /// asking for it. `None` for anything else.
    async fn meeting(&mut self, prompt: &str) -> Option<String> {
        if !asks_to_meet(prompt) {
            return None;
        }
        if let Some(times) = offered_times(prompt) {
            return Some(times);
        }
        let found = self.call("facts_lookup", json!({ "category": "availability" })).await;
        let link = found.as_ref().and_then(|v| v["facts"].as_array()).and_then(|facts| {
            facts.iter().find(|f| f["label"].as_str().is_some_and(|l| l.eq_ignore_ascii_case("Calendar link")))
        });
        Some(meeting_reply(prompt, link.and_then(|f| f["value"].as_str())))
    }

    /// Call one of Kaluta's tools, reporting it as a CLI's stream would.
    async fn call(&mut self, tool: &str, arguments: Value) -> Option<Value> {
        if self.client.is_none() {
            match ShimClient::connect(&self.socket, &self.session).await {
                Ok(c) => self.client = Some(c),
                Err(e) => {
                    tracing::warn!(error = %e, "demo agent cannot reach the tools");
                    return None;
                }
            }
        }
        self.calls += 1;
        let call_id = format!("demo_{}", self.calls);
        self.sink.emit(AgentEvent::ToolCallStarted {
            call_id: call_id.clone(),
            tool: tool.to_owned(),
            args_summary: summarize_args(&arguments),
        });
        let outcome = self.client.as_ref()?.call(tool, arguments).await;
        match outcome {
            Outcome::Ok { text, structured } => {
                self.sink.emit(AgentEvent::ToolCallFinished { call_id, ok: true, summary: shorten(&text, 160) });
                Some(structured.unwrap_or_else(|| serde_json::from_str(&text).unwrap_or(Value::String(text))))
            }
            Outcome::Error { code, message } => {
                self.sink.emit(AgentEvent::ToolCallFinished {
                    call_id,
                    ok: false,
                    summary: shorten(&format!("{code}: {message}"), 160),
                });
                None
            }
        }
    }

    async fn search(&mut self, query: &str, limit: u32) -> Vec<Found> {
        let Some(found) = self.call("mail_search", json!({ "query": query, "limit": limit })).await else {
            return vec![];
        };
        found["threads"].as_array().map(|t| t.iter().filter_map(Found::from).collect()).unwrap_or_default()
    }

    async fn thread(&mut self, id: &str) -> Option<Value> {
        self.call("mail_get_thread", json!({ "thread_id": id })).await
    }

    /// Person-to-person conversations in the Inbox whose last message is
    /// someone else's, asking the user something.
    async fn needs_reply(&mut self) -> String {
        let candidates: Vec<Found> =
            self.search("in:inbox newer_than:7d", 30).await.into_iter().filter(|t| is_person(&t.sender)).collect();
        let mut waiting = Vec::new();
        for t in candidates {
            let asks = self.thread(&t.id).await.is_some_and(|v| {
                let last = v["messages"].as_array().and_then(|m| m.last()).cloned().unwrap_or_default();
                last["sent_by_me"] != true && last["body"].as_str().is_some_and(|b| b.contains('?'))
            });
            if asks {
                waiting.push(t);
            }
        }
        if waiting.is_empty() {
            return "Nothing in your Inbox from the last week is waiting on a reply.".into();
        }
        self.present(&waiting, "Needs a reply").await;
        let lines: Vec<String> = waiting.iter().take(5).map(|t| format!("- {} — {}", t.name(), t.subject)).collect();
        format!(
            "{} {} waiting on you. The newest:\n{}",
            conversations(waiting.len()),
            if waiting.len() == 1 { "is" } else { "are" },
            lines.join("\n")
        )
    }

    /// Sent mail from the last month that nobody has answered: the user
    /// wrote last, to a person.
    async fn waiting_on_others(&mut self) -> String {
        let sent = self.search("in:sent older_than:2d newer_than:30d", 30).await;
        let mut waiting = Vec::new();
        for mut t in sent {
            let Some(thread) = self.thread(&t.id).await else { continue };
            let Some(last) = thread["messages"].as_array().and_then(|m| m.last()) else { continue };
            let to = last["to"].as_array().and_then(|to| to.iter().filter_map(Value::as_str).find(|a| is_person(a)));
            if let (true, Some(to)) = (last["sent_by_me"] == true, to) {
                t.sender = to.to_owned();
                waiting.push(t);
            }
        }
        if waiting.is_empty() {
            return "Everyone you wrote to in the last month has answered.".into();
        }
        self.present(&waiting, "Waiting on a reply").await;
        let lines: Vec<String> = waiting.iter().take(5).map(|t| format!("- {} — {}", t.name(), t.subject)).collect();
        format!("You're waiting on {}:\n{}", conversations(waiting.len()), lines.join("\n"))
    }

    async fn whats_new(&mut self) -> String {
        let new = self.search("in:inbox newer_than:1d", 50).await;
        if new.is_empty() {
            return "Nothing new in your Inbox since yesterday.".into();
        }
        let people = new.iter().filter(|t| is_person(&t.sender)).count();
        self.present(&new, "New since yesterday").await;
        format!(
            "{} since yesterday: {} from people, {} automated.",
            conversations(new.len()),
            people,
            new.len() - people
        )
    }

    async fn summarise(&mut self, id: &str) -> String {
        let Some(thread) = self.thread(id).await else { return "I couldn't read that conversation.".into() };
        let subject = thread["subject"].as_str().unwrap_or("This conversation");
        let lines: Vec<String> = thread["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| {
                let who = if m["sent_by_me"] == true {
                    "You".to_owned()
                } else {
                    name(m["from"].as_str().unwrap_or("Someone"))
                };
                format!("- {who}: {}", first_sentence(body_without_greeting(m["body"].as_str().unwrap_or_default())))
            })
            .collect();
        format!("{subject}\n{}", lines.join("\n"))
    }

    /// Draft a reply to the selected conversation, or to the newest one
    /// waiting on the user; with `send`, ask to send it.
    async fn draft_reply(&mut self, selected: &[String], send: bool) -> String {
        let target = match selected.first() {
            Some(id) => Some(id.clone()),
            None => {
                let found = self.search("in:inbox newer_than:7d", 20).await;
                found.into_iter().find(|t| is_person(&t.sender)).map(|t| t.id)
            }
        };
        let Some(id) = target else { return "Select a conversation and I'll draft a reply.".into() };
        let Some(thread) = self.thread(&id).await else { return "I couldn't read that conversation.".into() };
        let Some(theirs) = thread["messages"].as_array().and_then(|m| m.iter().rev().find(|m| m["sent_by_me"] != true))
        else {
            return "There's nothing in that conversation to reply to.".into();
        };
        let to = name(theirs["from"].as_str().unwrap_or_default());
        let body = reply_text(first_name(&to), theirs["body"].as_str().unwrap_or_default(), "");
        let args = json!({ "reply_to_message_id": theirs["message_id"], "body_markdown": body });
        let Some(draft) = self.call("mail_create_draft", args).await else {
            return "I couldn't create the draft.".into();
        };
        let Some(draft_id) = draft["draft_id"].as_i64() else { return "I couldn't create the draft.".into() };
        if !send {
            return format!("I drafted a reply to {to}. It's in Drafts for you to review before sending.");
        }
        match self.call("mail_send", json!({ "draft_id": draft_id })).await {
            Some(_) => format!("Sent your reply to {to}."),
            None => format!("I drafted a reply to {to} but didn't send it. It's in Drafts."),
        }
    }

    async fn present(&mut self, threads: &[Found], title: &str) {
        let ids: Vec<&str> = threads.iter().take(50).map(|t| t.id.as_str()).collect();
        self.call("mail_present_threads", json!({ "thread_ids": ids, "title": title })).await;
    }

    /// The sorting routine (spec §11.3): run it, or with `preview` answer
    /// with the classification only.
    async fn sort(&mut self, prompt: &str, preview: bool) -> String {
        let labels = routine_labels(prompt);
        let query = between(prompt, "Search with mail_search: `", "`").unwrap_or("is:important newer_than:1d");
        if !preview {
            let existing: Vec<String> = self
                .call("mail_list_labels", json!({}))
                .await
                .and_then(|v| {
                    v["labels"]
                        .as_array()
                        .map(|l| l.iter().filter_map(|l| l["name"].as_str().map(str::to_owned)).collect())
                })
                .unwrap_or_default();
            for (name, color) in &labels {
                if !existing.iter().any(|e| e.eq_ignore_ascii_case(name)) {
                    self.call("mail_create_label", json!({ "name": name, "color": color })).await;
                }
            }
        }
        let found = self.search(query, 50).await;
        let mut sorted: Vec<(usize, Found)> = Vec::new();
        let mut untouched = 0;
        for t in found {
            let already = t.labels.iter().any(|l| labels.iter().any(|(name, _)| name.eq_ignore_ascii_case(l)));
            if already && preview {
                continue;
            }
            match bucket(&t)
                .filter(|_| !is_person(&t.sender) && !t.labels.iter().any(|l| l == "STARRED" || l == "Starred"))
            {
                Some(b) if b < labels.len() => sorted.push((b, t)),
                _ => untouched += 1,
            }
        }
        if preview {
            let ids = between(prompt, "\"bucket\": one of [", "]").unwrap_or_default();
            let ids: Vec<&str> = ids.split(',').map(|s| s.trim().trim_matches('"')).collect();
            let rows: Vec<Value> = sorted
                .iter()
                .map(|(b, t)| json!({ "thread_id": t.id, "bucket": ids.get(*b), "reason": format!("From {}", t.name()) }))
                .collect();
            return Value::Array(rows).to_string();
        }
        let mut report = Vec::new();
        for (b, (name, _)) in labels.iter().enumerate() {
            let ids: Vec<&str> = sorted.iter().filter(|(x, _)| *x == b).map(|(_, t)| t.id.as_str()).collect();
            if ids.is_empty() {
                continue;
            }
            self.call("mail_add_label", json!({ "thread_ids": ids, "label": name })).await;
            report.push(format!("- {name}: {}", ids.len()));
            if name.contains("Daily") || name.contains("Hiring") {
                for (_, t) in sorted.iter().filter(|(x, _)| *x == b) {
                    report.push(format!("  - {} — {}", t.name(), t.subject));
                }
            }
        }
        let ids: Vec<&str> = sorted.iter().map(|(_, t)| t.id.as_str()).collect();
        if !ids.is_empty() {
            self.call("mail_archive", json!({ "thread_ids": ids })).await;
        }
        if sorted.is_empty() {
            return "Nothing new to sort this run.".into();
        }
        format!("Sorted {}; left {} in the Inbox.\n{}", conversations(sorted.len()), untouched, report.join("\n"))
    }
}

/// The routine prompt's labels and colours, in bucket order.
fn routine_labels(prompt: &str) -> Vec<(String, String)> {
    prompt
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("- `")?;
            let (name, after) = rest.split_once("` — color ")?;
            Some((name.to_owned(), after.trim().to_owned()))
        })
        .collect()
}

/// Which of the template's six buckets automated mail belongs in, by its
/// sender and subject, as a model would read them.
fn bucket(t: &Found) -> Option<usize> {
    let s = format!("{} {}", t.sender, t.subject).to_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| s.contains(w));
    if any(&["alert", "declined", "expire", "outage", "degraded", "action required"]) {
        Some(0)
    } else if any(&["application", "candidate", "applied"]) {
        Some(5)
    } else if any(&["following up", "last message", "sponsor", "quick question", "pitch"]) {
        Some(4)
    } else if any(&["receipt", "statement", "invoice", "renew", "payment"]) {
        Some(3)
    } else if any(&["event", "summit", "webinar", "meetup", "invited", "early-bird"]) {
        Some(2)
    } else if any(&["digest", "newsletter", "updates@", "field notes", "this week", "five things"]) {
        Some(1)
    } else {
        None
    }
}

/// A person, not a service: the demo's people write from first.last.
fn is_person(address: &str) -> bool {
    let email = address.rsplit('<').next().unwrap_or(address).trim_end_matches('>');
    email.split('@').next().is_some_and(|local| local.contains('.')) && !email.contains(ME)
}

fn name(address: &str) -> String {
    address.split(" <").next().unwrap_or(address).trim().to_owned()
}

fn first_name(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}

fn conversations(n: usize) -> String {
    if n == 1 { "1 conversation".into() } else { format!("{n} conversations") }
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    text.split_once(start)?.1.split(end).next()
}

fn body_without_greeting(body: &str) -> &str {
    let body = body.trim_start();
    match body.split_once('\n') {
        Some((first, rest)) if first.trim_end().ends_with(',') && first.len() < 40 => rest.trim_start(),
        _ => body,
    }
}

fn first_sentence(text: &str) -> String {
    let text = text.split("\n\n").next().unwrap_or(text).replace('\n', " ");
    match text.find(['.', '?', '!']) {
        Some(end) => text[..=end].to_owned(),
        None => text,
    }
}

/// A short, plausible reply to `original` from the demo's user, Jo.
fn reply_text(to: &str, original: &str, instruction: &str) -> String {
    let o = original.to_lowercase();
    let i = instruction.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| o.contains(w));
    let day = ["monday", "tuesday", "wednesday", "thursday", "friday"]
        .into_iter()
        .filter_map(|d| o.find(d).map(|at| (at, d)))
        .min()
        .map(|(_, d)| capitalise(d));
    // A direction beyond "write a reply" is what the reply says.
    let generic = i.is_empty() || i == "reply" || i.starts_with("write");
    let line = if i.contains("decline") || i.contains("say no") {
        "Thanks for thinking of me, but I'll have to pass this time.".to_owned()
    } else if !generic {
        let said = instruction.trim().trim_end_matches(['.', '!']);
        format!("{}.", capitalise(said))
    } else if has(&["go or no-go", "go for"]) {
        format!("Let's go for {}. I'll finish the pricing page copy today.", day.as_deref().unwrap_or("it"))
    } else if has(&["per-seat", "pricing", "price"]) && has(&["two years", "commit"]) {
        "Yes: for a two-year commitment we can keep this year's per-seat price. I'll send the updated terms before Friday.".into()
    } else if has(&["redlines", "liability"]) {
        "Twelve months of fees for the cap is fine, and so is 60 days' notice for price changes. I'll send our updated draft by Friday.".into()
    } else if has(&["offer", "candidate"]) {
        "Let's go ahead with the offer. I'll check the start date and the equity line this afternoon.".into()
    } else if has(&["sign-off", "approve", "budget"]) {
        "Approved. Thanks for pulling it together.".into()
    } else if has(&["intro", "meet"]) {
        "Thanks for the introduction. Happy to talk; does early next week work?".into()
    } else if has(&["interview", "piece", "article"]) {
        "Happy to talk. Would Wednesday afternoon work for twenty minutes?".into()
    } else if let Some(day) = day {
        format!("{day} works for me. Please send the invite.")
    } else if has(&["dinner", "coffee", "free"]) {
        "I'd love that. Let me know what suits you and I'll make it work.".into()
    } else {
        "Thanks, this is helpful. I'll come back to you with details by the end of the week.".into()
    };
    let hello = if to.is_empty() { "Hi,".to_owned() } else { format!("Hi {to},") };
    format!("{hello}\n\n{line}\n\nJo")
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// The composer's writing help: a reply to the message being answered,
/// or a short new message.
fn writing_help(prompt: &str) -> String {
    let instruction = between(prompt, "The user asks: ", "\n").unwrap_or_default();
    let to = prompt.lines().find_map(|l| l.strip_prefix("To: ")).unwrap_or_default();
    let to = if to.starts_with('(') { String::new() } else { name(to.split(',').next().unwrap_or_default()) };
    // The composer may give only an address: the name is its first part.
    let to = match to.split_once('@') {
        Some((local, _)) => capitalise(local.split(['.', '_', '-']).next().unwrap_or(local)),
        None => to,
    };
    // A change to a draft already written (a workflow's Shorter, Warmer).
    let draft = between(prompt, "The message so far:\n<<<\n", "\n>>>").unwrap_or_default();
    if !draft.trim().is_empty() && instruction.to_lowercase().starts_with("make it") {
        return changed(draft, instruction);
    }
    let original = between(prompt, "The message being answered or forwarded:\n<<<\n", "\n>>>").unwrap_or_default();
    if original.is_empty() {
        let hello = if to.is_empty() { "Hi,".to_owned() } else { format!("Hi {},", first_name(&to)) };
        let ask = instruction.trim_end_matches('.');
        return format!("{hello}\n\nI wanted to reach out about this: {ask}. Let me know what you think.\n\nJo");
    }
    reply_text(first_name(&to), original, instruction)
}

/// The email being answered asks to meet and the user named no time.
fn asks_to_meet(prompt: &str) -> bool {
    let instruction = between(prompt, "The user asks: ", "\n").unwrap_or_default().to_lowercase();
    let Some(original) = between(prompt, "The message being answered or forwarded:\n<<<\n", "\n>>>") else {
        return false;
    };
    let original = original.to_lowercase();
    let asks = ["meet", "talk", "a call", "find a time", "catch up", "schedule"].iter().any(|w| original.contains(w));
    const DAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
    let timed = instruction.chars().any(|c| c.is_ascii_digit()) || DAYS.iter().any(|d| instruction.contains(d));
    asks && !timed && !instruction.starts_with("make it")
}

/// The reply to a request to meet when the app gave the free times from
/// the calendar: the first two, each on a line of its own, as given.
fn offered_times(prompt: &str) -> Option<String> {
    let list = prompt.split_once("The user's calendar is free at these times")?.1;
    let times: Vec<&str> = list.lines().skip(1).map_while(|l| l.strip_prefix("- ")).take(2).collect();
    if times.is_empty() {
        return None;
    }
    let reply = writing_help(prompt);
    let (body, sign_off) = reply.rsplit_once("\n\n").unwrap_or((reply.as_str(), "Jo"));
    let lines: String = times.iter().map(|t| format!("- {t}\n")).collect();
    Some(format!("{body}\n\nWould one of these work?\n{lines}\n{sign_off}"))
}

/// The reply to a request to meet: with the user's calendar link on a
/// line of its own before the sign-off, or, without one, a question
/// asking for it (never invented times).
fn meeting_reply(prompt: &str, link: Option<&str>) -> String {
    let Some(link) = link else {
        return r#"{"questions": [{"question": "Which times work for you, or what is your calendar link?", "category": "availability", "label": "Calendar link"}]}"#
            .into();
    };
    let reply = writing_help(prompt);
    let (body, sign_off) = reply.rsplit_once("\n\n").unwrap_or((reply.as_str(), "Jo"));
    format!("{body}\n\nPick a time that suits you here:\n{link}\n\n{sign_off}")
}

/// A draft changed as asked ("Make it …"): shorter keeps its greeting,
/// first sentence and sign-off; warmer thanks them first; anything else
/// adds the request as a line before the sign-off.
fn changed(draft: &str, instruction: &str) -> String {
    let lines: Vec<&str> = draft.lines().filter(|l| !l.trim().is_empty()).collect();
    let (Some(first), Some(last)) = (lines.first(), lines.last()) else { return draft.to_owned() };
    let middle = if lines.len() > 2 { &lines[1..lines.len() - 1] } else { &[][..] };
    let ask = instruction.to_lowercase();
    if ask.contains("shorter") {
        let body = middle.join(" ");
        let sentence = body.split_inclusive(['.', '!', '?']).next().unwrap_or(&body).trim().to_owned();
        return format!("{first}\n\n{sentence}\n\n{last}");
    }
    if ask.contains("warmer") {
        return format!("{first}\n\nThanks so much for this!\n\n{}\n\n{last}", middle.join("\n"));
    }
    format!("{first}\n\n{}\n\n({instruction})\n\n{last}", middle.join("\n"))
}

/// Writing help's rewrite for the guide's checks: the draft, minus the
/// lines that break them.
fn rewrite(prompt: &str) -> String {
    let banned: Vec<String> = prompt
        .lines()
        .filter_map(|l| l.strip_prefix("- "))
        .filter_map(|l| between(l, "'", "'").map(str::to_lowercase))
        .collect();
    let draft = between(prompt, "<<<\n", "\n>>>").unwrap_or_default();
    draft.lines().filter(|l| !banned.iter().any(|b| l.to_lowercase().contains(b))).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(sender: &str, subject: &str) -> Found {
        Found { id: "t".into(), subject: subject.into(), sender: sender.into(), labels: vec![] }
    }

    use std::sync::Arc;
    use std::time::Duration;

    use futures::executor::block_on;

    use crate::agents::{AgentEventInfo as E, PromptContextInfo};
    use crate::{Core, CoreConfig, CoreEvent};

    #[derive(Default)]
    struct Recorder(std::sync::Mutex<Vec<CoreEvent>>);
    impl crate::EventListener for Recorder {
        fn on_event(&self, _account: Option<String>, event: CoreEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    impl Recorder {
        /// The session's events so far, once `done` holds (10 s at most).
        fn wait(&self, session: &str, done: impl Fn(&[E]) -> bool) -> Vec<E> {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                let seen: Vec<E> = self
                    .0
                    .lock()
                    .unwrap()
                    .iter()
                    .filter_map(|e| match e {
                        CoreEvent::AgentEvents { session_id, events } if session_id == session => Some(events.clone()),
                        _ => None,
                    })
                    .flatten()
                    .collect();
                if done(&seen) {
                    return seen;
                }
                assert!(std::time::Instant::now() < deadline, "timed out: {seen:?}");
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    fn demo_core(name: &str) -> (Arc<Core>, Arc<Recorder>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("kaluta-demo-agent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let recorder = Arc::new(Recorder::default());
        let core = Core::new(
            CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
            Arc::new(crate::secrets::MemorySecrets::default()),
            recorder.clone(),
        )
        .unwrap();
        core.debug_use_demo_agents();
        block_on(core.clone().open_account("demo".into())).unwrap();
        block_on(core.debug_seed_workflow_demo(100)).unwrap();
        (core, recorder, dir)
    }

    fn ask(core: &Arc<Core>, prompt: &str, selected: Vec<String>) -> String {
        let session = block_on(core.clone().start_agent_session("claude-code".into(), None, None)).unwrap();
        let context = PromptContextInfo { selected_thread_ids: selected, ..PromptContextInfo::default() };
        block_on(core.clone().send_agent_prompt(session.clone(), prompt.into(), context)).unwrap();
        session
    }

    #[test]
    fn it_lists_what_needs_a_reply_and_what_waits_on_others() {
        let (core, recorder, dir) = demo_core("lists");
        let session = ask(&core, "Which need a reply?", vec![]);
        let seen = recorder.wait(&session, |s| s.iter().any(|e| matches!(e, E::TurnCompleted { .. })));
        let shown = seen.iter().find_map(|e| match e {
            E::ResultsAvailable { thread_ids, title } => Some((thread_ids.clone(), title.clone())),
            _ => None,
        });
        let (ids, title) = shown.expect("a results list");
        assert_eq!(title.as_deref(), Some("Needs a reply"));
        assert!(ids.contains(&"demowf-renewal".to_owned()), "{ids:?}");
        assert!(!ids.iter().any(|id| id == "demowf-outage"), "no automated mail");
        assert!(!ids.iter().any(|id| id == "demowf-standup" || id == "demowf-thanks"), "nothing asked: {ids:?}");
        assert!(seen.iter().any(|e| matches!(e, E::ToolCallStarted { tool, .. } if tool == "mail_search")));

        let session = ask(&core, "Who haven't I heard back from?", vec![]);
        let seen = recorder.wait(&session, |s| s.iter().any(|e| matches!(e, E::TurnCompleted { .. })));
        let ids = seen
            .iter()
            .find_map(|e| match e {
                E::ResultsAvailable { thread_ids, .. } => Some(thread_ids.clone()),
                _ => None,
            })
            .expect("a results list");
        for waiting in ["demowf-pilot", "demowf-ip", "demowf-quote", "demowf-references"] {
            assert!(ids.contains(&waiting.to_owned()), "{waiting} in {ids:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn it_drafts_a_reply_and_asks_to_send_it() {
        let (core, recorder, dir) = demo_core("draft");
        let session = ask(&core, "Draft a reply and send it", vec!["demowf-board-prep".into()]);
        let seen = recorder.wait(&session, |s| s.iter().any(|e| matches!(e, E::ActionProposed { .. })));
        let (action, draft) = seen
            .iter()
            .find_map(|e| match e {
                E::ActionProposed { action_id, tool, draft_id, .. } if tool == "mail_send" => {
                    Some((*action_id, *draft_id))
                }
                _ => None,
            })
            .unwrap();
        let draft = block_on(core.get_draft(draft.unwrap())).unwrap().unwrap();
        assert!(draft.body_html.contains("Thursday works for me"), "{}", draft.body_html);
        core.resolve_agent_action(action, false).unwrap();
        let seen = recorder.wait(&session, |s| s.iter().any(|e| matches!(e, E::TurnCompleted { .. })));
        assert!(seen.iter().any(|e| matches!(e, E::TextDelta { text } if text.contains("didn't send"))), "{seen:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sorting_routine_runs_for_real_and_can_be_undone() {
        let (core, _recorder, dir) = demo_core("routine");
        let routine = block_on(core.create_routine_from_template("local".into())).unwrap();
        block_on(core.clone().run_routine_now(routine.id.clone())).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let run = loop {
            let runs = block_on(core.list_routine_runs(routine.id.clone(), 5)).unwrap();
            if let Some(run) = runs.into_iter().find(|r| r.status != "running") {
                break run;
            }
            assert!(std::time::Instant::now() < deadline, "the run never finished");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert_eq!(run.status, "succeeded", "{run:?}");
        assert!(run.thread_count >= 6, "{run:?}");
        assert!(run.report_text.as_deref().unwrap_or_default().contains("Marked Important/1-Daily"), "{run:?}");
        let inbox = block_on(core.search_threads("in:inbox is:important newer_than:1d".into(), None, 50)).unwrap();
        assert!(inbox.rows.iter().all(|t| !t.id.ends_with("-outage")), "sorted out of the Inbox");
        let restored = block_on(core.undo_routine_run(run.run_id)).unwrap();
        assert_eq!(restored, run.thread_count);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn people_and_services_are_told_apart() {
        assert!(is_person("Morgan Okafor <morgan.okafor@example.com>"));
        assert!(!is_person("Billing <billing@example.net>"));
        assert!(!is_person("Me <me@example.com>"));
    }

    #[test]
    fn automated_mail_lands_in_the_templates_buckets() {
        let b = |s: &str, sub: &str| bucket(&found(s, sub));
        assert_eq!(b("Billing <billing@example.net>", "Your payment was declined"), Some(0));
        assert_eq!(b("Status Alerts <alerts@example.net>", "Degraded performance in us-east"), Some(0));
        assert_eq!(b("The Digest <digest@example.org>", "This week in design systems"), Some(1));
        assert_eq!(b("Events <events@example.org>", "You're invited: Founders Meetup"), Some(2));
        assert_eq!(b("Billing <billing@example.net>", "Your receipt from Example Cloud"), Some(3));
        assert_eq!(b("Pat at Example Leads <pat@example.net>", "Just following up on my last note"), Some(4));
        assert_eq!(b("Careers <careers@example.com>", "New application for Product Designer"), Some(5));
    }

    #[test]
    fn the_routine_prompt_is_read_for_its_labels_and_query() {
        let prompt = "Search with mail_search: `is:important newer_than:1d -in:sent`\n\n- `Marked Important/1-Daily` — color #fb4c2f\n- `Marked Important/2-Weekly-Newsletters` — color #4a86e8\n";
        assert_eq!(
            routine_labels(prompt),
            vec![
                ("Marked Important/1-Daily".to_owned(), "#fb4c2f".to_owned()),
                ("Marked Important/2-Weekly-Newsletters".to_owned(), "#4a86e8".to_owned())
            ]
        );
        assert_eq!(between(prompt, "Search with mail_search: `", "`"), Some("is:important newer_than:1d -in:sent"));
    }

    #[test]
    fn writing_help_answers_the_message() {
        let prompt = "You are helping write an email in Kaluta's composer. The user asks: Reply\n\nAnswer with only the text.\n\nFrom: Me <me@example.com>\nTo: Sam Rivera <sam.rivera@example.com>\nSubject: Re: Board prep\n\nThe message so far:\n<<<\n\n>>>\n\nThe message being answered or forwarded:\n<<<\nHi Jo,\n\nCan you do Thursday at 2pm for the board prep?\n\nSam\n>>>";
        assert_eq!(writing_help(prompt), "Hi Sam,\n\nThursday works for me. Please send the invite.\n\nJo");
    }

    #[test]
    fn a_request_to_meet_is_answered_with_the_calendar_link_or_a_question() {
        let ask = |said: &str, email: &str| {
            format!(
                "You are helping write an email in Kaluta's composer. The user asks: {said}\n\nTo: Sam Rivera <sam@example.com>\n\n\
                 The message being answered or forwarded:\n<<<\nHi Jo,\n\n{email}\n\nSam\n>>>"
            )
        };
        let meet = ask("Sure, happy to", "When can we talk this week?");
        assert!(asks_to_meet(&meet));
        assert!(!asks_to_meet(&ask("Yes, Thursday at 2", "When can we talk this week?")), "a time was named");
        assert!(!asks_to_meet(&ask("Sure", "Can you send the deck?")), "not about meeting");
        assert!(!asks_to_meet(&ask("Make it shorter", "When can we talk?")));
        let with = meeting_reply(&meet, Some("https://cal.com/jo"));
        assert!(with.starts_with("Hi Sam,"), "{with}");
        assert!(with.contains("\nhttps://cal.com/jo\n\nJo") || with.ends_with("https://cal.com/jo\n\nJo"), "{with}");
        // Free times from the calendar: the first two, as given.
        let timed = format!(
            "{meet}\n\nThe user's calendar is free at these times (America/Los_Angeles):\n- Thu Oct 30, 2–2:30 pm\n\
             - Fri Oct 31, 9–9:30 am\n- Mon Nov 2, 9–9:30 am\nWhen the reply agrees to meet…"
        );
        let offered = offered_times(&timed).unwrap();
        assert!(offered.contains("\n- Thu Oct 30, 2–2:30 pm\n- Fri Oct 31, 9–9:30 am\n"), "{offered}");
        assert!(!offered.contains("Mon Nov 2") && offered.ends_with("Jo"));
        assert!(offered_times(&meet).is_none());
        let without = meeting_reply(&meet, None);
        let q: Value = serde_json::from_str(&without).unwrap();
        assert_eq!(q["questions"][0]["label"], "Calendar link");
        assert_eq!(q["questions"][0]["category"], "availability");
    }

    #[test]
    fn writing_help_greets_by_name_when_given_only_an_address() {
        let prompt = "You are helping write an email in Kaluta's composer. The user asks: Reply\n\nTo: sam.rivera@example.com\n\nThe message being answered or forwarded:\n<<<\nCan you do Thursday?\n>>>";
        assert!(writing_help(prompt).starts_with("Hi Sam,\n"), "{}", writing_help(prompt));
    }

    #[test]
    fn a_reply_says_what_the_user_said() {
        let text =
            reply_text("Quinn", "Are you free for dinner next Wednesday or Thursday?", "Thursday works, my place at 7");
        assert_eq!(text, "Hi Quinn,\n\nThursday works, my place at 7.\n\nJo");
        assert!(reply_text("Quinn", "Dinner Wednesday?", "Write a reply").contains("Wednesday works"));
    }

    #[test]
    fn a_change_to_a_draft_is_made_to_that_draft() {
        let draft = "Hi Sam,\nThursday works for me. Please send the invite.\nJo";
        assert_eq!(changed(draft, "Make it shorter"), "Hi Sam,\n\nThursday works for me.\n\nJo");
        assert!(changed(draft, "Make it warmer").contains("Thanks so much"));
        let prompt = format!(
            "You are helping write an email in Kaluta's composer. The user asks: Make it shorter\n\nTo: sam@example.com\n\n\
             The message so far:\n<<<\n{draft}\n>>>"
        );
        assert_eq!(writing_help(&prompt), "Hi Sam,\n\nThursday works for me.\n\nJo");
    }

    #[test]
    fn a_rewrite_drops_what_the_guide_bans() {
        let prompt = "Your draft breaks the user's writing guide:\n- Never write 'circle back'\nRewrite it.\n\nYour draft:\n<<<\nHi Sam,\nLet's circle back next week.\nJo\n>>>";
        assert_eq!(rewrite(prompt), "Hi Sam,\nJo");
    }
}
