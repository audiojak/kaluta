//! Gleaning facts from mail the user sends (spec §14.11): an optional
//! step of the daily review. The prompt, the parser (quotes verified, and
//! what must never be stored dropped by pattern before a proposal exists),
//! and the merge into Analysis proposals. Received mail is never read.

use std::collections::BTreeMap;

use mail_store::analysis::{self as store, NewProposal, ProposalRow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::facts::{FactCategoryInfo, FactEdit, FactFields, FactInfo, FactScope, FactSource, FactStatus, FactUse};
use crate::guide_ai::{fenced, loose};
use crate::{Core, CoreError, runtime};

/// The first line of every gleaning prompt: how the fake agent knows one.
pub const GLEAN_MARKER: &str = "Kaluta facts glean";
/// Messages read for facts in one review, at most.
pub const GLEAN_CAP: usize = 30;
/// Facts in Other that look alike before a category is proposed.
const CATEGORY_FACTS: usize = 3;
/// Reviews that find facts fitting a starter set before it is proposed.
const STARTER_REVIEWS: i64 = 3;
const TEXT_CAP: usize = 3_000;
const QUOTE_CAP: usize = 300;

/// `analysis_meta` keys.
pub(crate) mod keys {
    /// `off`, `ai` (mail written with AI, the default) or `all`.
    pub const FACTS_FROM: &str = "facts_from";
    /// The run whose facts were gleaned (so a resumed run does not glean twice).
    pub const GLEANED_RUN: &str = "gleaned_run";
    /// With `all`: the newest sent message read so far.
    pub const GLEANED_UNTIL: &str = "gleaned_until";
}

/// Where gleaning reads from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FactsFrom {
    Off,
    MailWrittenWithAi,
    AllMailISend,
}

impl FactsFrom {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::MailWrittenWithAi => "ai",
            Self::AllMailISend => "all",
        }
    }

    pub(crate) fn parse(s: Option<&str>) -> Self {
        match s {
            Some("off") => Self::Off,
            Some("all") => Self::AllMailISend,
            _ => Self::MailWrittenWithAi,
        }
    }
}

/// One sent message's own text, as gleaning reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sent {
    pub message_id: String,
    pub text: String,
}

/// What a fact proposal carries (`analysis_proposals.payload_json`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FactPayload {
    /// `fact`, `category` or `starter`.
    pub kind: String,
    pub category: String,
    pub label: String,
    pub value: String,
    pub as_of: Option<i64>,
    /// The fact an alteration or removal is for.
    pub fact_id: Option<i64>,
    pub message_id: String,
    pub quote: String,
    /// A new category: its name and description, and the facts in Other
    /// it would take.
    pub name: String,
    pub description: String,
    pub fact_ids: Vec<i64>,
    /// A starter set: its id (`business`, `freelance`, `household`, `job_search`).
    pub starter: String,
}

/// A fact the agent found, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    Add(FactPayload),
    Alter(FactPayload),
    Remove(FactPayload),
    Category(FactPayload),
    Starter(FactPayload),
}

/// What must never be stored, even when found (spec §14.11): passwords,
/// card and bank numbers, government ids, health details about others,
/// and anything about a third party beyond who they are to the user.
pub fn never_store(category: &str, label: &str, value: &str) -> bool {
    let text = format!("{label} {value}").to_lowercase();
    let secret =
        ["password", "passcode", "passphrase", " pin ", "pin:", "pin code", "security code", "cvv", "one-time code"];
    if secret.iter().any(|w| format!(" {text} ").contains(w)) {
        return true;
    }
    let words: Vec<&str> = text.split(|c: char| !c.is_alphanumeric()).collect();
    if ["ssn", "sin", "pwd", "pw", "acct"].iter().any(|w| words.contains(w)) {
        return true;
    }
    let ids = [
        "social security",
        "door code",
        "gate code",
        "alarm code",
        "account no",
        "passport",
        "driver's licen",
        "drivers licen",
        "national insurance",
        "tax id",
        "taxpayer",
        "iban",
        "routing number",
        "sort code",
        "account number",
        "card number",
        "credit card",
        "debit card",
    ];
    if ids.iter().any(|w| text.contains(w)) {
        return true;
    }
    // Long runs of digits: card, bank and id numbers (not phones: those
    // have at most 15 digits and are labelled as phones).
    let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
    let l = label.to_lowercase();
    let phone = ["phone", "mobile", "cell", "fax", "whatsapp", "landline", "tel"].iter().any(|w| l.contains(w))
        && digits.len() <= 15;
    if digits.len() >= 9 && !phone {
        let only_number = value.chars().all(|c| c.is_ascii_digit() || " -./".contains(c));
        if only_number || digits.len() >= 13 {
            return true;
        }
    }
    if value.split(|c: char| !c.is_ascii_digit()).any(|run| run.len() == 9 && value.contains('-')) {
        return true;
    }
    if category == "people" {
        let health = [
            "diagnos",
            "illness",
            "disease",
            "cancer",
            "pregnan",
            "medication",
            "surgery",
            "therapy",
            "depress",
            "anxiety",
            "hospital",
            "disabilit",
            "condition",
            "treatment",
        ];
        if health.iter().any(|w| text.contains(w)) {
            return true;
        }
        // Beyond name, role and how the user knows them.
        if value.split_whitespace().count() > 12 {
            return true;
        }
    }
    false
}

/// Whether a quote states a value: a word of it (three letters or more,
/// or a number) occurs in the quote.
fn states(quote: &str, value: &str) -> bool {
    let quote = loose(quote);
    let words: Vec<String> = value
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3 || w.chars().any(|c| c.is_ascii_digit()))
        .map(str::to_lowercase)
        .collect();
    words.is_empty() || words.iter().any(|w| quote.contains(w.as_str()))
}

fn capped(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// The request for the day's sent mail.
pub fn prompt(sent: &[Sent], categories: &[FactCategoryInfo], facts: &[FactInfo]) -> String {
    let cats: String = categories
        .iter()
        .filter(|c| !c.hidden)
        .map(|c| format!("{} ({}): {}", c.key, fenced(&c.name), fenced(&c.description)))
        .collect::<Vec<_>>()
        .join("\n");
    let known: String = facts
        .iter()
        .filter(|f| f.status == FactStatus::Accepted && f.scope == FactScope::Account)
        .map(|f| format!("#{} [{}] {}: {}", f.id, f.category, fenced(&f.label), fenced(&f.value)))
        .collect::<Vec<_>>()
        .join("\n");
    let messages: String = sent
        .iter()
        .map(|m| format!("<message id=\"{}\">\n{}\n</message>", m.message_id, fenced(&capped(&m.text, TEXT_CAP))))
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "{GLEAN_MARKER}\n\n\
         Below are emails the user sent (their own words only). Find facts about the user and their work that \
         an assistant drafting email for them could use: who they are, how to reach them, when they are \
         available, who the people they mention are to them, their work, their preferences, and anything that \
         fits one of their own categories.\n\n\
         Never propose: passwords or codes; card, bank or account numbers; government ids; health details \
         about anyone else; anything about another person beyond their name, role and how the user knows them.\n\n\
         Categories (key (name): what belongs there):\n{cats}\n\n\
         Facts already known (alter or remove them by #id rather than repeating them):\n{}\n\n\
         {messages}\n\n\
         Answer with only a JSON object, and nothing else:\n\
         {{\"facts\": [{{\"op\": \"add\", \"category\": \"work\", \"label\": \"Team\", \"value\": \"...\", \
         \"as_of\": null or \"YYYY-MM-DD\" for facts that age, \"evidence\": {{\"message_id\": \"...\", \"quote\": \
         \"words from that message\"}}}}, \
         {{\"op\": \"alter\", \"id\": 12, \"value\": \"the new value\", \"evidence\": {{...}}}}, \
         {{\"op\": \"remove\", \"id\": 12, \"evidence\": {{...}}}}], \
         \"categories\": [{{\"name\": \"...\", \"description\": \"...\", \"facts\": [ids of facts in other]}}], \
         \"starter_set\": null or \"business\" or \"freelance\" or \"household\" or \"job_search\"}}\n\n\
         - Only facts the messages state; never guess. Quote the words that state each one.\n\
         - Propose a category only for three or more facts in other that belong together.\n\
         - Name a starter set only when the facts plainly fit it (company facts on a work account: business).\n\
         - Do not use tools, and do not create, change, send or delete any mail.",
        if known.is_empty() { "(none)".to_owned() } else { known },
    )
}

fn date(v: Option<&Value>) -> Option<i64> {
    let s = v?.as_str()?;
    let d = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()?;
    Some(d.and_hms_opt(12, 0, 0)?.and_utc().timestamp_millis())
}

/// Read the agent's answer. Facts whose quote is not in the message they
/// cite, in unknown categories, or that must never be stored, are
/// dropped. `None` when the answer is not JSON at all.
pub fn parse(text: &str, sent: &[Sent], categories: &[FactCategoryInfo], facts: &[FactInfo]) -> Option<Vec<Found>> {
    let value: Value = text.match_indices('{').find_map(|(i, _)| {
        serde_json::Deserializer::from_str(&text[i..])
            .into_iter::<Value>()
            .next()
            .and_then(Result::ok)
            .filter(|v| v.get("facts").is_some())
    })?;
    let by_id: BTreeMap<&str, &Sent> = sent.iter().map(|m| (m.message_id.as_str(), m)).collect();
    let mine = |id: i64| {
        facts.iter().find(|f| f.id == id && f.scope == FactScope::Account && f.status == FactStatus::Accepted)
    };
    let clean = |v: Option<&Value>, cap: usize| -> String {
        capped(&crate::guide::clean_text(v.and_then(Value::as_str).unwrap_or("")), cap)
    };
    let mut out = Vec::new();
    for f in value.get("facts").and_then(Value::as_array).into_iter().flatten() {
        // Every fact cites the words that state it.
        let e = f.get("evidence");
        let message = e.and_then(|e| e.get("message_id")).and_then(Value::as_str).unwrap_or("");
        let quote = clean(e.and_then(|e| e.get("quote")), QUOTE_CAP);
        let Some(m) = by_id.get(message) else { continue };
        // At least two words, found in the message.
        if quote.split_whitespace().count() < 2 || !loose(&fenced(&m.text)).contains(&loose(&quote)) {
            continue;
        }
        let mut p = FactPayload {
            kind: "fact".into(),
            message_id: message.to_owned(),
            quote,
            as_of: date(f.get("as_of")),
            ..Default::default()
        };
        match f.get("op").and_then(Value::as_str).unwrap_or("add") {
            "add" => {
                let category = f.get("category").and_then(Value::as_str).unwrap_or("");
                let Some(c) = categories.iter().find(|c| c.key == category || c.name.eq_ignore_ascii_case(category))
                else {
                    continue;
                };
                p.category = c.key.clone();
                p.label = clean(f.get("label"), 80);
                p.value = clean(f.get("value"), 400);
                if p.label.is_empty() || p.value.is_empty() || never_store(&p.category, &p.label, &p.value) {
                    continue;
                }
                // The quote must say it: one of the value's words is in it.
                if !states(&p.quote, &p.value) {
                    continue;
                }
                // A fact by that label already: the same is nothing new, a
                // different value is a change to it (spec §14.11).
                if let Some(k) = facts.iter().find(|k| {
                    k.status == FactStatus::Accepted
                        && k.scope == FactScope::Account
                        && k.category == p.category
                        && k.label.to_lowercase() == p.label.to_lowercase()
                }) {
                    if loose(&k.value) != loose(&p.value) {
                        p.label = k.label.clone();
                        p.fact_id = Some(k.id);
                        out.push(Found::Alter(p));
                    }
                    continue;
                }
                // Twice in one answer: once.
                if out.iter().any(|o| {
                    matches!(o, Found::Add(q) if q.category == p.category
                    && q.label.to_lowercase() == p.label.to_lowercase())
                }) {
                    continue;
                }
                out.push(Found::Add(p));
            }
            "alter" => {
                let Some(old) = f.get("id").and_then(Value::as_i64).and_then(mine) else { continue };
                p.value = clean(f.get("value"), 400);
                if p.value.is_empty()
                    || loose(&p.value) == loose(&old.value)
                    || never_store(&old.category, &old.label, &p.value)
                {
                    continue;
                }
                p.category = old.category.clone();
                p.label = old.label.clone();
                p.fact_id = Some(old.id);
                out.push(Found::Alter(p));
            }
            "remove" => {
                let Some(old) = f.get("id").and_then(Value::as_i64).and_then(mine) else { continue };
                p.category = old.category.clone();
                p.label = old.label.clone();
                p.value = old.value.clone();
                p.fact_id = Some(old.id);
                out.push(Found::Remove(p));
            }
            _ => {}
        }
    }
    let in_other: Vec<i64> =
        facts.iter().filter(|f| f.category == "other" && f.scope == FactScope::Account).map(|f| f.id).collect();
    for c in value.get("categories").and_then(Value::as_array).into_iter().flatten() {
        let name = clean(c.get("name"), 60);
        let ids: Vec<i64> = c
            .get("facts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_i64)
            .filter(|id| in_other.contains(id))
            .collect();
        if name.is_empty()
            || ids.len() < CATEGORY_FACTS
            || categories.iter().any(|k| crate::facts::similar(&k.name, &name))
        {
            continue;
        }
        out.push(Found::Category(FactPayload {
            kind: "category".into(),
            name,
            description: clean(c.get("description"), 200),
            fact_ids: ids,
            ..Default::default()
        }));
    }
    if let Some(set) = value.get("starter_set").and_then(Value::as_str)
        && ["business", "freelance", "household", "job_search"].contains(&set)
    {
        out.push(Found::Starter(FactPayload { kind: "starter".into(), starter: set.into(), ..Default::default() }));
    }
    Some(out)
}

fn proposal(found: &Found) -> NewProposal {
    let (op, p, key, threshold) = match found {
        Found::Add(p) => ("add", p, format!("add|{}|{}|{}", p.category, loose(&p.label), loose(&p.value)), 1),
        Found::Alter(p) => ("edit", p, format!("alter|{}|{}", p.fact_id.unwrap_or_default(), loose(&p.value)), 1),
        Found::Remove(p) => ("remove", p, format!("remove|{}", p.fact_id.unwrap_or_default()), 1),
        Found::Category(p) => ("add", p, format!("category|{}", loose(&p.name)), 1),
        Found::Starter(p) => ("add", p, format!("starter|{}", p.starter), STARTER_REVIEWS),
    };
    NewProposal {
        target: "fact".into(),
        op: op.into(),
        entry_id: p.fact_id,
        category: p.category.clone(),
        kind: Some(p.kind.clone()),
        statement: match found {
            Found::Category(p) => p.name.clone(),
            Found::Starter(p) => p.starter.clone(),
            _ => format!("{}: {}", p.label, p.value),
        },
        scope_json: "{}".into(),
        match_key: key,
        contradicts_entry_id: None,
        payload_json: serde_json::to_string(p).ok(),
        threshold,
    }
}

impl Core {
    /// The setting: where facts are learned from.
    pub(crate) async fn facts_from(&self) -> Result<FactsFrom, CoreError> {
        let db = self.db()?;
        let v = runtime::run(async move { Ok(db.read(|c| store::meta(c, keys::FACTS_FROM)).await?) }).await?;
        Ok(FactsFrom::parse(v.as_deref()))
    }

    /// The sent messages a review reads for facts: the AI-matched ones it
    /// reviewed, or (with All mail I send) what the user sent since the
    /// last glean.
    async fn glean_source(&self, run: &mail_store::analysis::RunRow, from: FactsFrom) -> Result<Vec<Sent>, CoreError> {
        let db = self.db()?;
        let started = run.started_at;
        if from == FactsFrom::MailWrittenWithAi {
            return runtime::run(async move {
                Ok(db
                    .read(move |c| {
                        Ok(c.prepare_cached(
                            "SELECT matched_message_id, sent_text FROM ai_compositions
                             WHERE status = 'reviewed' AND reviewed_at >= ?1 AND sent_text IS NOT NULL
                               AND matched_message_id IS NOT NULL
                             ORDER BY reviewed_at, id LIMIT ?2",
                        )?
                        .query_map(rusqlite_params(started), |r| Ok(Sent { message_id: r.get(0)?, text: r.get(1)? }))?
                        .collect::<Result<_, _>>()?)
                    })
                    .await?)
            })
            .await;
        }
        let since = runtime::run(async move { Ok(db.read(|c| store::meta(c, keys::GLEANED_UNTIL)).await?) })
            .await?
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(started - 24 * 60 * 60 * 1000);
        let db = self.db()?;
        let candidates =
            runtime::run(
                async move { Ok(db.read(move |c| mail_store::compositions::sent_since(c, since + 1)).await?) },
            )
            .await?;
        let chosen: Vec<_> = candidates.into_iter().take(GLEAN_CAP).collect();
        let until = chosen.last().map(|m| m.at);
        let texts = self.own_texts(chosen.iter().map(|m| m.message_id.clone()).collect()).await?;
        if let (Some(until), Some(account)) = (until, self.effective_account_id()) {
            self.agents.glean_until.lock().unwrap_or_else(|e| e.into_inner()).insert(account, until);
        }
        Ok(chosen
            .into_iter()
            .filter_map(|m| {
                texts
                    .get(&m.message_id)
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| Sent { message_id: m.message_id, text: t.clone() })
            })
            .collect())
    }

    /// The review's gleaning step, once per run (spec §14.11): read the
    /// day's sent mail for facts, and propose them in Analysis.
    pub(crate) async fn glean_facts(self: &std::sync::Arc<Self>, run: i64, agent: &str) -> Result<(), CoreError> {
        let db = self.db()?;
        let (row, done) = runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let done = store::meta(c, keys::GLEANED_RUN)?.and_then(|v| v.parse::<i64>().ok()) == Some(run);
                    Ok((store::get_run(c, run)?, done))
                })
                .await?)
        })
        .await?;
        let Some(row) = row else { return Ok(()) };
        let from = self.facts_from().await?;
        if done || from == FactsFrom::Off {
            return Ok(());
        }
        let sent = self.glean_source(&row, from).await?;
        if !sent.is_empty() {
            let categories = self.fact_categories().await?;
            let facts = self.list_facts(vec![FactStatus::Accepted]).await?;
            let answer = self.ask_agent_hidden(agent, prompt(&sent, &categories, &facts)).await?;
            match parse(&answer, &sent, &categories, &facts) {
                Some(found) => self.merge_glean(found).await?,
                None => tracing::warn!(run, "fact gleaning skipped: unreadable answer"),
            }
        }
        // Read: only now does "All mail I send" move on past these messages.
        let until = self
            .effective_account_id()
            .and_then(|a| self.agents.glean_until.lock().unwrap_or_else(|e| e.into_inner()).remove(&a));
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    if let Some(until) = until {
                        store::set_meta(tx, keys::GLEANED_UNTIL, &until.to_string())?;
                    }
                    store::set_meta(tx, keys::GLEANED_RUN, &run.to_string())
                })
                .await?)
        })
        .await?;
        Ok(())
    }

    pub(crate) async fn merge_glean(&self, found: Vec<Found>) -> Result<(), CoreError> {
        if found.is_empty() {
            return Ok(());
        }
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    for f in &found {
                        store::upsert_counted(tx, &proposal(f), now)?;
                    }
                    Ok(())
                })
                .await?)
        })
        .await?;
        self.analysis_changed();
        Ok(())
    }

    /// Accept or reject fact proposals (spec §14.11): one change on the
    /// account's facts stack, undone with `undo_fact_change`.
    pub(crate) async fn decide_fact_proposals(
        &self,
        rows: &[ProposalRow],
        accept: bool,
        uses: &std::collections::HashMap<i64, FactUse>,
    ) -> Result<crate::facts::FactChange, CoreError> {
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        let status = if accept { "accepted" } else { "rejected" };
        if !accept {
            return self
                .apply_facts_with(FactScope::Account, vec![], vec![], "analysis reject".into(), Some((ids, status)))
                .await;
        }
        let categories = self.fact_categories().await?;
        let mut edits = Vec::new();
        let mut category_edits = Vec::new();
        let mut starters = Vec::new();
        for r in rows {
            let p: FactPayload =
                r.payload_json.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
            let chosen_use = uses.get(&r.id).copied();
            let default_use = categories.iter().find(|c| c.key == p.category).map_or(FactUse::Free, |c| c.default_use);
            match (p.kind.as_str(), r.op.as_str()) {
                ("fact", "add") => edits.push(FactEdit::Add {
                    fields: FactFields {
                        category: p.category,
                        label: p.label,
                        value: p.value,
                        use_: chosen_use.unwrap_or(default_use),
                        as_of: p.as_of,
                    },
                    status: FactStatus::Accepted,
                    source: FactSource::Learned,
                }),
                ("fact", "edit") => {
                    let Some(id) = p.fact_id else { continue };
                    let facts = self.list_facts(vec![FactStatus::Accepted]).await?;
                    let Some(old) = facts.iter().find(|f| f.id == id && f.scope == FactScope::Account) else {
                        continue;
                    };
                    edits.push(FactEdit::Update {
                        id,
                        fields: FactFields {
                            category: old.category.clone(),
                            label: old.label.clone(),
                            value: p.value,
                            use_: chosen_use.unwrap_or(old.use_),
                            as_of: p.as_of.or(old.as_of),
                        },
                    });
                }
                ("fact", "remove") => edits.extend(p.fact_id.map(|id| FactEdit::Delete { id })),
                ("category", _) => {
                    category_edits
                        .push(crate::facts::CategoryEdit::Add { name: p.name.clone(), description: p.description });
                    let facts = self.list_facts(vec![FactStatus::Accepted]).await?;
                    for id in p.fact_ids {
                        if let Some(f) =
                            facts.iter().find(|f| f.id == id && f.scope == FactScope::Account && f.category == "other")
                        {
                            edits.push(FactEdit::Update {
                                id,
                                fields: FactFields {
                                    category: p.name.clone(),
                                    label: f.label.clone(),
                                    value: f.value.clone(),
                                    use_: f.use_,
                                    as_of: f.as_of,
                                },
                            });
                        }
                    }
                }
                ("starter", _) => starters.push(p.starter),
                _ => {}
            }
        }
        // A starter set's categories are part of the same change.
        let mut starter = None;
        for s in starters {
            let set = match s.as_str() {
                "business" => crate::facts::StarterSet::Business,
                "freelance" => crate::facts::StarterSet::Freelance,
                "household" => crate::facts::StarterSet::Household,
                _ => crate::facts::StarterSet::JobSearch,
            };
            let have = self.fact_categories().await?;
            for (name, description) in crate::facts::starter_categories(set) {
                if !have.iter().any(|c| crate::facts::similar(&c.name, &name)) {
                    category_edits.push(crate::facts::CategoryEdit::Add { name, description });
                }
            }
            starter = Some(set);
        }
        let change = self
            .apply_facts_full(
                FactScope::Account,
                edits,
                category_edits,
                "analysis accept".into(),
                Some((ids, status)),
                starter,
            )
            .await?;
        self.analysis_changed();
        Ok(change)
    }
}

#[uniffi::export]
impl Core {
    /// Where the daily review learns facts from (spec §14.11).
    pub async fn analysis_facts_from(&self) -> Result<FactsFrom, CoreError> {
        self.facts_from().await
    }

    pub async fn set_analysis_facts_from(&self, from: FactsFrom) -> Result<(), CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.write(move |tx| store::set_meta(tx, keys::FACTS_FROM, from.as_str())).await?) })
            .await
    }
}

/// The one parameter of the reviewed-since query, and the cap.
fn rusqlite_params(started: i64) -> [i64; 2] {
    [started, GLEAN_CAP as i64]
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use mail_store::compositions::{self, Kind, NewComposition, Recipients, Source};

    use super::*;
    use crate::analysis_run::tests::{learned, no_schedule, rt, wait_done};

    #[test]
    fn a_quote_must_state_the_value() {
        assert!(states("I'm the CTO at Acme.", "CTO at Acme"));
        assert!(states("Call me on 415 555 0100", "+1 415 555 0100"));
        assert!(!states("Thanks for the note", "CTO at Acme"));
    }

    #[test]
    fn what_must_never_be_stored_is_dropped() {
        assert!(never_store("other", "Wi-Fi password", "hunter2"));
        assert!(never_store("contact", "Card", "4111 1111 1111 1111"));
        assert!(never_store("other", "Bank", "IBAN GB29 NWBK 6016 1331 9268 19"));
        assert!(never_store("identity", "SSN", "123-45-6789"));
        assert!(never_store("identity", "ID", "123-45-6789"));
        assert!(never_store("people", "Sam", "My brother, recovering from surgery"));
        assert!(never_store(
            "people",
            "Sam",
            "My brother who lives in Ohio with his two kids and works nights at the plant near the river"
        ));
        assert!(!never_store("contact", "Phone", "+1 415 555 0100"));
        assert!(!never_store("contact", "Mobile", "415-555-0100"));
        assert!(never_store("contact", "Phone", "4111 1111 1111 1111 1"), "a card number is not a phone");
        assert!(!never_store("other", "Lessons", "Classes at the Mission"), "ssn only as a word");
        assert!(never_store("other", "Gate code", "4410"));
        assert!(!never_store("people", "Sam", "My assistant"));
        assert!(!never_store("work", "Occupation or role", "CEO of Actual AI"));
    }

    fn sent(text: &str) -> Vec<Sent> {
        vec![Sent { message_id: "m1".into(), text: text.into() }]
    }

    fn categories() -> Vec<FactCategoryInfo> {
        crate::facts::categories_from(&[])
    }

    #[test]
    fn facts_need_a_quote_from_the_message_and_a_known_category() {
        let mail = sent("Hi Ann,\nI'm the CTO at Acme. My calendar: https://cal.com/j\nJ");
        let answer = r#"{"facts": [
            {"op": "add", "category": "work", "label": "Occupation or role", "value": "CTO at Acme", "evidence": {"message_id": "m1", "quote": "I'm the CTO at Acme."}},
            {"op": "add", "category": "Availability", "label": "Calendar link", "value": "https://cal.com/j", "as_of": "2026-10-01", "evidence": {"message_id": "m1", "quote": "My calendar: https://cal.com/j"}},
            {"op": "add", "category": "work", "label": "Team", "value": "Mail", "evidence": {"message_id": "m1", "quote": "not in it"}},
            {"op": "add", "category": "hobbies", "label": "Golf", "value": "yes", "evidence": {"message_id": "m1", "quote": "Hi Ann"}},
            {"op": "add", "category": "other", "label": "Password", "value": "x", "evidence": {"message_id": "m1", "quote": "Hi Ann"}},
            {"op": "add", "category": "work", "label": "Org", "value": "Acme", "evidence": {"message_id": "m9", "quote": "Hi"}}
        ], "categories": [{"name": "Hobbies", "facts": [1, 2]}], "starter_set": "business"}"#;
        let found = parse(answer, &mail, &categories(), &[]).unwrap();
        assert_eq!(found.len(), 3, "{found:#?}");
        let Found::Add(calendar) = &found[1] else { panic!() };
        assert_eq!((calendar.category.as_str(), calendar.as_of.is_some()), ("availability", true));
        assert!(matches!(&found[2], Found::Starter(p) if p.starter == "business"));
        assert!(parse("no", &mail, &categories(), &[]).is_none());
    }

    fn known(id: i64, category: &str, label: &str, value: &str) -> FactInfo {
        FactInfo {
            id,
            category: category.into(),
            label: label.into(),
            value: value.into(),
            use_: FactUse::Free,
            as_of: None,
            source: FactSource::You,
            status: FactStatus::Accepted,
            evidence: vec![],
            created_at: 0,
            updated_at: 0,
            scope: FactScope::Account,
            overridden: false,
            stale: false,
            share_with_cloud: true,
        }
    }

    #[test]
    fn a_new_value_for_a_known_label_is_a_change_not_a_second_fact() {
        let mail = sent("Our team is now Calendar.");
        let facts = vec![known(7, "work", "Team", "Mail")];
        let answer = r#"{"facts": [
            {"op": "add", "category": "work", "label": "team", "value": "Calendar", "evidence": {"message_id": "m1", "quote": "Our team is now Calendar"}},
            {"op": "add", "category": "work", "label": "Team", "value": "Mail", "evidence": {"message_id": "m1", "quote": "Our team"}}
        ]}"#;
        let found = parse(answer, &mail, &categories(), &facts).unwrap();
        assert_eq!(found.len(), 1);
        assert!(
            matches!(&found[0], Found::Alter(p) if p.fact_id == Some(7) && p.value == "Calendar" && p.label == "Team")
        );
    }

    #[test]
    fn known_facts_are_altered_or_removed_and_alike_ones_get_a_category() {
        let mail = sent("We moved to Eastern time last week.");
        let facts = vec![
            known(1, "availability", "Time zone", "Pacific"),
            known(2, "other", "Boat", "Sloop"),
            known(3, "other", "Marina", "Pier 39"),
            known(4, "other", "Sailing club", "SFYC"),
        ];
        let answer = r#"{"facts": [
            {"op": "alter", "id": 1, "value": "Eastern", "evidence": {"message_id": "m1", "quote": "moved to Eastern time"}},
            {"op": "alter", "id": 1, "value": "pacific", "evidence": {"message_id": "m1", "quote": "moved"}},
            {"op": "remove", "id": 2, "evidence": {"message_id": "m1", "quote": "We moved"}},
            {"op": "add", "category": "availability", "label": "Time zone", "value": "Pacific", "evidence": {"message_id": "m1", "quote": "We moved"}}
        ], "categories": [{"name": "Sailing", "description": "The user's boat", "facts": [2, 3, 4]}, {"name": "Contact info", "facts": [2, 3, 4]}]}"#;
        let found = parse(answer, &mail, &categories(), &facts).unwrap();
        assert!(matches!(&found[0], Found::Alter(p) if p.fact_id == Some(1) && p.value == "Eastern"));
        assert!(matches!(&found[1], Found::Remove(p) if p.fact_id == Some(2)));
        assert!(matches!(&found[2], Found::Category(p) if p.name == "Sailing" && p.fact_ids == vec![2, 3, 4]));
        assert_eq!(found.len(), 3, "no change, a known fact again, and a name like Contact are dropped: {found:#?}");
    }

    fn scratch(name: &str) -> crate::guide::tests::Scratch {
        let s = crate::guide::tests::demo(name);
        s.1.debug_use_fake_agents();
        no_schedule(&s.1);
        learned(&s.1);
        s
    }

    fn pair(core: &Core, sent: &str) {
        let db = core.db().unwrap();
        let sent = sent.to_owned();
        let at = mail_sync::now_millis() - 1000;
        rt(db.write(move |tx| {
            let id = compositions::record(
                tx,
                &NewComposition {
                    source: Source::Agent,
                    agent: None,
                    kind: Kind::Reply,
                    draft_id: -1,
                    thread_id: None,
                    in_reply_to: None,
                    recipients: Recipients::new(["ann@example.com"], []),
                    subject: "Intro".into(),
                    instruction: String::new(),
                    ai_text: "Hello Ann, I work at Acme.".into(),
                    ai_html: None,
                    guide_version: None,
                    audiences: vec![],
                },
                at,
            )?;
            compositions::draft_gone(tx, -1, true, at)?;
            compositions::set_matched(tx, id, "sent-1", "sent_draft", &sent, 0.4, at)
        }))
        .unwrap();
    }

    #[test]
    fn the_review_gleans_facts_from_ai_mail_and_accepting_one_is_undoable() {
        let s = scratch("glean-ai");
        let core = &s.1;
        pair(core, "Hi Ann,\nI'm the CTO at Acme.\nOur wifi password is hunter2.\nJ");
        rt(core.clone().start_analysis_run(None)).unwrap();
        wait_done(core);
        let q = block_on(core.analysis_queue()).unwrap();
        assert_eq!(q.facts.len(), 1, "the password was never proposed: {:#?}", q.facts);
        let p = &q.facts[0];
        assert_eq!(
            (p.category.as_str(), p.label.as_str(), p.value.as_str()),
            ("work", "Occupation or role", "the CTO")
        );
        assert_eq!((p.category_name.as_str(), p.quote.as_str()), ("Work", "I'm the CTO at Acme."));
        assert!(q.unseen);
        assert_eq!(p.use_, FactUse::Free, "the category's default");

        // Accepted as "ask first": the user's choice, in the same change.
        let change =
            block_on(core.decide_fact_analysis_proposals(vec![p.id], true, [(p.id, FactUse::Ask)].into())).unwrap();
        let facts = block_on(core.list_facts(vec![FactStatus::Accepted])).unwrap();
        assert_eq!(
            (facts[0].value.as_str(), facts[0].source, facts[0].use_),
            ("the CTO", FactSource::Learned, FactUse::Ask)
        );
        assert!(block_on(core.analysis_queue()).unwrap().facts.is_empty());
        block_on(core.undo_fact_change(change.change_id)).unwrap();
        assert!(block_on(core.list_facts(vec![FactStatus::Accepted])).unwrap().is_empty());
        assert_eq!(block_on(core.analysis_queue()).unwrap().facts.len(), 1, "back in the queue");
        // Rejected: not proposed again.
        block_on(core.decide_fact_analysis_proposals(vec![p.id], false, Default::default())).unwrap();
        pair(core, "Hi Bo,\nI'm the CTO at Acme.\nJ");
        rt(core.clone().start_analysis_run(None)).unwrap();
        wait_done(core);
        assert!(block_on(core.analysis_queue()).unwrap().facts.is_empty());
    }

    #[test]
    fn off_reads_nothing_and_all_mail_moves_on_through_sent() {
        let s = scratch("glean-settings");
        let core = &s.1;
        block_on(core.set_analysis_facts_from(FactsFrom::Off)).unwrap();
        pair(core, "Hi Ann,\nI'm the CTO at Acme.\nJ");
        rt(core.clone().start_analysis_run(None)).unwrap();
        wait_done(core);
        assert!(block_on(core.analysis_queue()).unwrap().facts.is_empty());
        assert_eq!(block_on(core.analysis_facts_from()).unwrap(), FactsFrom::Off);

        block_on(core.debug_seed_demo_mailbox(40)).unwrap();
        block_on(core.set_analysis_facts_from(FactsFrom::AllMailISend)).unwrap();
        let db = core.db().unwrap();
        rt(db.write(|tx| store::set_meta(tx, keys::GLEANED_UNTIL, "0"))).unwrap();
        rt(core.clone().start_analysis_run(None)).unwrap();
        wait_done(core);
        let until = rt(db.read(|c| store::meta(c, keys::GLEANED_UNTIL))).unwrap().unwrap();
        assert!(until.parse::<i64>().unwrap() > 0, "it read the user's sent mail and remembers where it got to");
    }

    #[test]
    fn dated_facts_go_stale() {
        let s = scratch("glean-stale");
        let core = &s.1;
        let old = mail_sync::now_millis() - 200 * 24 * 60 * 60 * 1000;
        block_on(core.apply_fact_edits(
            vec![FactEdit::Add {
                fields: FactFields {
                    category: "availability".into(),
                    label: "Away or travel dates".into(),
                    value: "In Lisbon".into(),
                    use_: FactUse::Free,
                    as_of: Some(old),
                },
                status: FactStatus::Accepted,
                source: FactSource::You,
            }],
            "add".into(),
        ))
        .unwrap();
        assert!(block_on(core.list_facts(vec![FactStatus::Accepted])).unwrap()[0].stale);
    }
}
