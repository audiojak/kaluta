//! Comparing AI drafts with what the user sent (spec §14.10): the prompt
//! for a batch of pairs, the lenient parser for the agent's answer (every
//! quote must occur in the text it cites), and the merge into proposals
//! that wait in Analysis. Drafts sent as written count for the entries
//! that applied to them. This module never talks to an agent itself.

use std::collections::{BTreeMap, BTreeSet};

use mail_store::analysis::{self as store, EvidenceRow, NewProposal};
use mail_store::compositions::Composition;
use serde_json::Value;

use crate::guide::{AudienceGroup, CATEGORIES, GuideEntry, GuideKind, GuideScope, GuideStatus, category, clean_text};
use crate::guide_ai::{fenced, loose};
use crate::guide_render::{Target, applies, audiences_of};
use crate::{Core, CoreError, runtime};

/// The first line of every comparison prompt: how the fake agent knows one.
pub const COMPARE_MARKER: &str = "OpenAGC analysis compare";
/// Pairs a proposal needs before it shows: a guideline two, a rule three.
pub const GUIDELINE_PAIRS: i64 = 2;
pub const RULE_PAIRS: i64 = 3;
const STATEMENT_CAP: usize = 400;
const QUOTE_CAP: usize = 300;
/// Text of each side of a pair in a prompt, at most.
const TEXT_CAP: usize = 3_000;

/// One pair as the comparison sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    pub id: i64,
    /// `new`, `reply` or `forward`.
    pub kind: String,
    pub audiences: Vec<String>,
    pub instruction: String,
    pub ai: String,
    pub sent: String,
    /// Accepted entries (not facts) that applied to the message.
    pub applied: Vec<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Edit,
    Rescope,
    Remove,
}

impl Op {
    fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Edit => "edit",
            Self::Rescope => "rescope",
            Self::Remove => "remove",
        }
    }
}

/// A change the comparison proposes, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub op: Op,
    /// The entry an edit, rescope or removal is for.
    pub entry_id: Option<i64>,
    pub category: String,
    pub kind: GuideKind,
    pub statement: String,
    pub scope: GuideScope,
    /// An accepted entry an added statement goes against.
    pub contradicts: Option<i64>,
    pub evidence: Vec<EvidenceRow>,
}

impl Change {
    /// What makes two proposals the same one, across pairs and days.
    pub fn key(&self) -> String {
        let entry = self.entry_id.unwrap_or_default();
        match self.op {
            Op::Add => format!("add|{}|{}", self.category, loose(&self.statement)),
            Op::Edit => format!("edit|{entry}|{}|{}", self.kind.as_str(), loose(&self.statement)),
            Op::Rescope => format!("rescope|{entry}|{}", serde_json::to_string(&self.scope).unwrap_or_default()),
            Op::Remove => format!("remove|{entry}"),
        }
    }

    fn threshold(&self) -> i64 {
        if self.kind == GuideKind::Rule { RULE_PAIRS } else { GUIDELINE_PAIRS }
    }
}

/// The accepted entries (rules and guidelines) that applied to a
/// composition: its recipients, type and audience, as drafting saw them.
pub(crate) fn applied_entries(entries: &[GuideEntry], groups: &[AudienceGroup], c: &Composition) -> Vec<i64> {
    let target = Target {
        recipients: c.recipients.to.iter().chain(&c.recipients.cc).cloned().collect(),
        message_type: Some(c.kind.as_str().to_owned()),
        audiences: (!c.audiences.is_empty()).then(|| c.audiences.clone()),
    };
    let audiences = target.audiences.clone().unwrap_or_else(|| audiences_of(&target.recipients, groups));
    entries
        .iter()
        .filter(|e| e.status == GuideStatus::Accepted && e.kind != GuideKind::Fact)
        .filter(|e| applies(&e.scope, &target, &audiences))
        .map(|e| e.id)
        .collect()
}

fn capped(s: &str) -> String {
    s.chars().take(TEXT_CAP).collect()
}

/// The request for one batch of pairs.
pub fn prompt(pairs: &[Pair], entries: &[GuideEntry]) -> String {
    let categories: String = CATEGORIES
        .iter()
        .filter(|c| !c.id.starts_with('F'))
        .map(|c| format!("{} {}", c.id, c.name))
        .collect::<Vec<_>>()
        .join("; ");
    let guide: String = entries
        .iter()
        .filter(|e| e.status == GuideStatus::Accepted && e.kind != GuideKind::Fact)
        .map(|e| {
            let scope = serde_json::to_string(&e.scope).unwrap_or_default();
            format!("#{} [{} {}] {} scope={scope}", e.id, e.category, e.kind.as_str(), fenced(&e.statement))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let blocks: String = pairs
        .iter()
        .map(|p| {
            let applied = p.applied.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(", ");
            format!(
                "<pair id=\"{}\" type=\"{}\">\nAudience: {}\nThe user asked: {}\nGuide entries that applied: {}\n\
                 <ai>\n{}\n</ai>\n<sent>\n{}\n</sent>\n</pair>",
                p.id,
                p.kind,
                if p.audiences.is_empty() { "(none)".to_owned() } else { fenced(&p.audiences.join(", ")) },
                if p.instruction.trim().is_empty() {
                    "(nothing recorded)".to_owned()
                } else {
                    fenced(&capped(&p.instruction))
                },
                if applied.is_empty() { "(none)".to_owned() } else { applied },
                fenced(&capped(&p.ai)),
                fenced(&capped(&p.sent)),
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "{COMPARE_MARKER}\n\n\
         Each pair below is an email an assistant drafted for the user (<ai>) and what the user actually sent \
         after editing it (<sent>). The assistant followed the user's writing guide. Find where the user's edits \
         show the guide is wrong or missing something about their style, and propose changes to the guide.\n\n\
         Only style counts: tone, length, greetings and sign-offs, wording, structure, punctuation, formatting. \
         A change of substance (a different date, a new plan, a fact the assistant could not know, a paragraph \
         about something else) is not style: leave it out.\n\n\
         The guide now (rules and guidelines, by #id):\n{}\n\n\
         Categories: {categories}\n\n\
         {blocks}\n\n\
         Answer with only a JSON object, and nothing else:\n\
         {{\"proposals\": [{{\"op\": \"add\", \"category\": \"B6\", \"kind\": \"rule\" or \"guideline\", \
         \"statement\": \"...\", \"scope\": {{\"groups\": [], \"people\": [], \"message_types\": [], \
         \"languages\": []}}, \"contradicts\": null or an entry id, \"evidence\": [{{\"pair\": 12, \"sent\": \
         \"words from the sent text\", \"ai\": \"what they replaced in the AI text\"}}]}}, \
         {{\"op\": \"edit\", \"id\": 7, \"statement\": \"...\", \"kind\": \"guideline\", \"evidence\": [...]}}, \
         {{\"op\": \"rescope\", \"id\": 7, \"scope\": {{...}}, \"evidence\": [...]}}, \
         {{\"op\": \"remove\", \"id\": 7, \"evidence\": [...]}}]}}\n\n\
         - A statement is one sentence, an instruction to someone drafting for the user.\n\
         - Edit, rescope or remove an entry by its #id when the user's edits go against it; add an entry for \
         what the guide does not cover.\n\
         - Quote exactly: \"sent\" must be words from that pair's sent text and \"ai\" words from its AI text.\n\
         - Propose nothing rather than guess; one edit is not a habit.\n\
         - Do not use tools, and do not create, change, send or delete any mail.",
        if guide.is_empty() { "(empty)".to_owned() } else { guide },
    )
}

fn scope_of(v: Option<&Value>) -> Option<GuideScope> {
    let v = v?.as_object()?;
    let list = |k: &str| -> Vec<String> {
        v.get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(clean_text).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default()
    };
    Some(GuideScope {
        groups: list("groups"),
        people: list("people"),
        message_types: list("message_types")
            .into_iter()
            .filter(|t| ["new", "reply", "forward"].contains(&t.as_str()))
            .collect(),
        languages: list("languages"),
    })
}

/// A quote that occurs in the text it cites (as the prompt showed it).
fn verified(quote: Option<&str>, text: &str) -> Option<String> {
    let quote = clean_text(quote?);
    let quote: String = quote.chars().take(QUOTE_CAP).collect();
    if quote.is_empty() {
        return None;
    }
    loose(&fenced(text)).contains(&loose(&quote)).then_some(quote)
}

/// Read the agent's answer. Unknown categories and entries, and evidence
/// for pairs not in the batch or quoting words the texts do not have, are
/// dropped; a change left with no evidence is dropped. `None` when the
/// answer is not JSON at all.
pub fn parse(text: &str, pairs: &[Pair], entries: &[GuideEntry]) -> Option<Vec<Change>> {
    let value: Value = text.match_indices('{').find_map(|(i, _)| {
        serde_json::Deserializer::from_str(&text[i..])
            .into_iter::<Value>()
            .next()
            .and_then(Result::ok)
            .filter(|v| v.get("proposals").is_some())
    })?;
    let accepted =
        |id: i64| entries.iter().find(|e| e.id == id && e.status == GuideStatus::Accepted && e.kind != GuideKind::Fact);
    let by_id: BTreeMap<i64, &Pair> = pairs.iter().map(|p| (p.id, p)).collect();
    let mut out = Vec::new();
    for p in value.get("proposals").and_then(Value::as_array).into_iter().flatten() {
        let mut evidence = Vec::new();
        let mut seen = BTreeSet::new();
        for e in p.get("evidence").and_then(Value::as_array).into_iter().flatten() {
            let Some(pair) = e.get("pair").and_then(Value::as_i64).and_then(|id| by_id.get(&id)) else { continue };
            let sent_given = e.get("sent").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty());
            let ai_given = e.get("ai").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty());
            let sent = verified(e.get("sent").and_then(Value::as_str), &pair.sent);
            let ai = verified(e.get("ai").and_then(Value::as_str), &pair.ai);
            // Every quote given must be found; at least one is needed.
            if (sent_given && sent.is_none()) || (ai_given && ai.is_none()) || (sent.is_none() && ai.is_none()) {
                continue;
            }
            if seen.insert(pair.id) {
                evidence.push(EvidenceRow {
                    composition_id: pair.id,
                    sent_quote: sent.unwrap_or_default(),
                    ai_quote: ai.unwrap_or_default(),
                });
            }
        }
        if evidence.is_empty() {
            continue;
        }
        let statement: String =
            clean_text(p.get("statement").and_then(Value::as_str).unwrap_or("")).chars().take(STATEMENT_CAP).collect();
        let kind = p.get("kind").and_then(Value::as_str).and_then(GuideKind::parse).filter(|k| *k != GuideKind::Fact);
        let entry = p.get("id").and_then(Value::as_i64).and_then(accepted);
        let change = match (p.get("op").and_then(Value::as_str).unwrap_or("add"), entry) {
            ("add", _) => {
                let Some(cat) = p.get("category").and_then(Value::as_str).and_then(category) else { continue };
                if statement.is_empty() || cat.id.starts_with('F') {
                    continue;
                }
                Change {
                    op: Op::Add,
                    entry_id: None,
                    category: cat.id.into(),
                    kind: kind.unwrap_or(GuideKind::Guideline),
                    statement,
                    scope: scope_of(p.get("scope")).unwrap_or_default(),
                    contradicts: p.get("contradicts").and_then(Value::as_i64).and_then(accepted).map(|e| e.id),
                    evidence,
                }
            }
            ("edit", Some(old)) => {
                if statement.is_empty()
                    || loose(&statement) == loose(&old.statement) && kind.is_none_or(|k| k == old.kind)
                {
                    continue;
                }
                Change {
                    op: Op::Edit,
                    entry_id: Some(old.id),
                    category: old.category.clone(),
                    kind: kind.unwrap_or(old.kind),
                    statement: if statement.is_empty() { old.statement.clone() } else { statement },
                    scope: old.scope.clone(),
                    contradicts: None,
                    evidence,
                }
            }
            ("rescope", Some(old)) => {
                let Some(scope) = scope_of(p.get("scope")).filter(|s| *s != old.scope) else { continue };
                Change {
                    op: Op::Rescope,
                    entry_id: Some(old.id),
                    category: old.category.clone(),
                    kind: old.kind,
                    statement: old.statement.clone(),
                    scope,
                    contradicts: None,
                    evidence,
                }
            }
            ("remove", Some(old)) => Change {
                op: Op::Remove,
                entry_id: Some(old.id),
                category: old.category.clone(),
                kind: old.kind,
                statement: old.statement.clone(),
                scope: old.scope.clone(),
                contradicts: None,
                evidence,
            },
            _ => continue,
        };
        out.push(change);
    }
    Some(out)
}

fn new_proposal(c: &Change) -> NewProposal {
    NewProposal {
        target: "guide".into(),
        op: c.op.as_str().into(),
        entry_id: c.entry_id,
        category: c.category.clone(),
        kind: Some(c.kind.as_str().into()),
        statement: c.statement.clone(),
        scope_json: serde_json::to_string(&c.scope).unwrap_or_else(|_| "{}".into()),
        match_key: c.key(),
        contradicts_entry_id: c.contradicts,
        payload_json: None,
        threshold: c.threshold(),
    }
}

impl Core {
    /// The pairs of a batch, ready for the prompt; pairs whose texts have
    /// gone (purged) are left out.
    pub(crate) async fn compare_pairs(&self, ids: &[i64]) -> Result<(Vec<Pair>, Vec<GuideEntry>), CoreError> {
        let entries = self.list_guide_entries(vec![GuideStatus::Accepted]).await?;
        let groups = self.list_audience_groups().await?;
        let db = self.db()?;
        let ids = ids.to_vec();
        let records = runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let mut out = Vec::new();
                    for id in ids {
                        out.extend(mail_store::compositions::get(c, id)?);
                    }
                    Ok(out)
                })
                .await?)
        })
        .await?;
        let pairs = records
            .into_iter()
            .filter_map(|c| {
                let applied = applied_entries(&entries, &groups, &c);
                Some(Pair {
                    id: c.id,
                    kind: c.kind.as_str().into(),
                    audiences: c.audiences.clone(),
                    instruction: c.instruction.clone(),
                    ai: c.ai_text.clone()?,
                    sent: c.sent_text.clone()?,
                    applied,
                })
            })
            .collect();
        Ok((pairs, entries))
    }

    /// Merge what the comparison found into the proposals: one per change,
    /// its evidence growing across pairs and days. Each pair counts once
    /// against each entry it changed; an entry gone against in three pairs
    /// is proposed for removal. Returns `false` when the answer could not
    /// be read.
    pub(crate) async fn merge_compare(
        &self,
        pairs: &[Pair],
        entries: &[GuideEntry],
        answer: &str,
    ) -> Result<bool, CoreError> {
        let Some(changes) = parse(answer, pairs, entries) else { return Ok(false) };
        let kinds: BTreeMap<i64, (GuideKind, String, String, GuideScope)> = entries
            .iter()
            .map(|e| (e.id, (e.kind, e.category.clone(), e.statement.clone(), e.scope.clone())))
            .collect();
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    let mut against: BTreeSet<(i64, i64)> = BTreeSet::new();
                    for change in &changes {
                        store::upsert_proposal(tx, &new_proposal(change), &change.evidence, now)?;
                        if let Some(entry) = change.entry_id.or(change.contradicts) {
                            against.extend(change.evidence.iter().map(|e| (entry, e.composition_id)));
                        }
                    }
                    let mut entries_against: BTreeSet<i64> = BTreeSet::new();
                    for (entry, _) in &against {
                        store::add_health(tx, *entry, 0, 1, now)?;
                        entries_against.insert(*entry);
                    }
                    for entry in entries_against {
                        let pairs = store::pairs_against(tx, entry)?;
                        if pairs.len() as i64 >= RULE_PAIRS
                            && let Some((kind, category, statement, scope)) = kinds.get(&entry)
                        {
                            let remove = Change {
                                op: Op::Remove,
                                entry_id: Some(entry),
                                category: category.clone(),
                                kind: *kind,
                                statement: statement.clone(),
                                scope: scope.clone(),
                                contradicts: None,
                                evidence: pairs,
                            };
                            store::upsert_proposal(tx, &new_proposal(&remove), &remove.evidence, now)?;
                        }
                    }
                    Ok(())
                })
                .await?)
        })
        .await?;
        Ok(true)
    }

    /// Drafts sent as written support the entries that applied to them.
    pub(crate) async fn reinforce_unchanged(&self, ids: &[i64], now: mail_domain::Millis) -> Result<(), CoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        let (pairs, _) = self.compare_pairs(ids).await?;
        let applied: Vec<(i64, Vec<i64>)> = pairs.into_iter().map(|p| (p.id, p.applied)).collect();
        let ids = ids.to_vec();
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    for (_, entries) in &applied {
                        for entry in entries {
                            store::add_health(tx, *entry, 1, 0, now)?;
                        }
                    }
                    store::mark_reviewed(tx, &ids, now)
                })
                .await?)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guide::{GuideSource, GuideStatus};

    fn entry(id: i64, category: &str, kind: GuideKind, statement: &str) -> GuideEntry {
        GuideEntry {
            id,
            category: category.into(),
            kind,
            statement: statement.into(),
            scope: GuideScope::default(),
            status: GuideStatus::Accepted,
            source: GuideSource::You,
            origin: None,
            check: None,
            support: 0,
            contradict: 0,
            contradiction_of: None,
            run_id: None,
            evidence: vec![],
            created_at: 0,
            updated_at: 0,
            decided_at: None,
        }
    }

    fn pair(id: i64, ai: &str, sent: &str) -> Pair {
        Pair {
            id,
            kind: "reply".into(),
            audiences: vec![],
            instruction: "Write a reply".into(),
            ai: ai.into(),
            sent: sent.into(),
            applied: vec![7],
        }
    }

    #[test]
    fn the_prompt_fences_mail_and_lists_what_applied() {
        let entries = [entry(7, "B6", GuideKind::Guideline, "Sign off with 'Best, John'")];
        let p = prompt(&[pair(3, "Hi <b>Ann</b>\nBest, John", "Hi Ann\nJ")], &entries);
        assert!(p.starts_with(COMPARE_MARKER));
        assert!(p.contains("#7 [B6 guideline] Sign off with 'Best, John'"));
        assert!(p.contains("<pair id=\"3\" type=\"reply\">"));
        assert!(p.contains("Guide entries that applied: #7"));
        assert!(p.contains("Hi ‹b>Ann‹/b>"), "mail cannot close or fake a block");
    }

    #[test]
    fn quotes_must_occur_in_the_texts_they_cite() {
        let entries = [entry(7, "B6", GuideKind::Guideline, "Sign off with 'Best, John'")];
        let pairs = [pair(3, "Thanks Ann.\nBest, John", "Thanks Ann.\nJ"), pair(4, "Hello there", "Hi there")];
        let answer = r#"Here you go: {"proposals": [
            {"op": "edit", "id": 7, "statement": "Sign off with 'J'", "evidence": [
                {"pair": 3, "sent": "J", "ai": "Best, John"},
                {"pair": 4, "sent": "made up", "ai": "Hello"},
                {"pair": 99, "sent": "Hi"}]},
            {"op": "add", "category": "B1", "statement": "Open with 'Hi'", "evidence": [{"pair": 4, "sent": "Hi there"}]},
            {"op": "add", "category": "Z9", "statement": "Unknown", "evidence": [{"pair": 4, "sent": "Hi"}]},
            {"op": "add", "category": "F3", "statement": "My role: CEO", "evidence": [{"pair": 4, "sent": "Hi"}]},
            {"op": "remove", "id": 12, "evidence": [{"pair": 3, "sent": "J"}]},
            {"op": "add", "category": "A1", "statement": "Be warm", "evidence": [{"pair": 3, "sent": "nowhere"}]}
        ]}"#;
        let changes = parse(answer, &pairs, &entries).unwrap();
        assert_eq!(changes.len(), 2, "{changes:#?}");
        assert_eq!(changes[0].op, Op::Edit);
        assert_eq!(changes[0].evidence.len(), 1, "only the verified pair");
        assert_eq!(changes[0].evidence[0].ai_quote, "Best, John");
        assert_eq!(changes[0].key(), "edit|7|guideline|sign off with 'j'");
        assert_eq!((changes[1].op, changes[1].category.as_str()), (Op::Add, "B1"));
        assert!(parse("not json", &pairs, &entries).is_none());
    }

    #[test]
    fn keys_merge_the_same_change_however_it_is_written() {
        let e = vec![EvidenceRow::default()];
        let a = Change {
            op: Op::Add,
            entry_id: None,
            category: "B1".into(),
            kind: GuideKind::Guideline,
            statement: "Open with  'Hi'".into(),
            scope: GuideScope::default(),
            contradicts: None,
            evidence: e.clone(),
        };
        let b = Change { statement: "open with 'hi'".into(), ..a.clone() };
        assert_eq!(a.key(), b.key());
        assert_eq!(a.threshold(), GUIDELINE_PAIRS);
        assert_eq!(Change { kind: GuideKind::Rule, ..a }.threshold(), RULE_PAIRS);
    }

    mod runs {
        use std::sync::Arc;

        use futures::executor::block_on;
        use mail_store::compositions::{self, Kind, NewComposition, Recipients, Source};

        use super::super::*;
        use crate::analysis_run::tests::{learned, no_schedule, rt, wait_done};
        use crate::guide::{GuideEdit, GuideEntryFields, GuideSource};

        const AI: &str = "Hi Ann,\nI hope this finds you well. Friday works for me.\nBest regards,\nJohn";
        const SENT: &str = "Hi Ann,\nFriday works for me.\nJ";

        fn scratch(name: &str) -> crate::guide::tests::Scratch {
            let s = crate::guide::tests::demo(name);
            s.1.debug_use_fake_agents();
            no_schedule(&s.1);
            learned(&s.1);
            s
        }

        /// A composition already matched to what was sent.
        fn pair(core: &Core, ai: &str, sent: &str, distance: f64) -> i64 {
            let (ai, sent) = (ai.to_owned(), sent.to_owned());
            let db = core.db().unwrap();
            let at = mail_sync::now_millis();
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
                        subject: "Friday".into(),
                        instruction: "Say Friday works".into(),
                        ai_text: ai,
                        ai_html: None,
                        guide_version: None,
                        audiences: vec![],
                    },
                    at,
                )?;
                compositions::draft_gone(tx, -1, true, at)?;
                compositions::set_matched(tx, id, &format!("m{id}"), "thread_next", &sent, distance, at)?;
                Ok(id)
            }))
            .unwrap()
        }

        fn review(core: &Arc<Core>) {
            rt(core.clone().start_analysis_run(None)).unwrap();
            wait_done(core);
        }

        fn by_statement(core: &Core, statement: &str) -> Option<store::ProposalRow> {
            let db = core.db().unwrap();
            let all = rt(db.read(|c| store::proposals(c, &["watching", "proposed", "accepted", "rejected"]))).unwrap();
            all.into_iter().find(|p| p.statement == statement)
        }

        #[test]
        fn edits_become_proposals_that_show_at_their_threshold_and_grow_across_days() {
            let s = scratch("compare-threshold");
            let core = &s.1;
            pair(core, AI, SENT, 0.4);
            review(core);
            let signoff = by_statement(core, "Sign off with the first name only").expect("a proposal");
            assert_eq!((signoff.status.as_str(), signoff.support), ("watching", 1), "one pair is not a habit");
            let hope = by_statement(core, "Never write 'I hope this finds you well'").unwrap();
            assert_eq!((hope.kind.as_deref(), hope.status.as_str()), (Some("rule"), "watching"));

            // The next review: the same proposals, with more evidence.
            pair(core, AI, SENT, 0.4);
            review(core);
            let signoff = by_statement(core, "Sign off with the first name only").unwrap();
            assert_eq!((signoff.status.as_str(), signoff.support), ("proposed", 2), "a guideline shows at two");
            assert!(signoff.shown_at.is_some());
            assert_eq!(by_statement(core, "Never write 'I hope this finds you well'").unwrap().status, "watching");
            pair(core, AI, SENT, 0.4);
            review(core);
            let hope = by_statement(core, "Never write 'I hope this finds you well'").unwrap();
            assert_eq!((hope.status.as_str(), hope.support), ("proposed", 3), "a rule shows at three");
            let db = core.db().unwrap();
            let evidence = rt(db.read(move |c| store::evidence(c, hope.id))).unwrap();
            assert!(evidence.iter().all(|e| e.ai_quote == "I hope this finds you well" && e.sent_quote.is_empty()));
        }

        #[test]
        fn a_rejected_proposal_is_not_raised_again() {
            let s = scratch("compare-rejected");
            let core = &s.1;
            pair(core, AI, SENT, 0.4);
            pair(core, AI, SENT, 0.4);
            review(core);
            let p = by_statement(core, "Sign off with the first name only").unwrap();
            let db = core.db().unwrap();
            rt(db.write(move |tx| store::set_proposal_status(tx, p.id, "rejected", 1))).unwrap();
            pair(core, AI, SENT, 0.4);
            review(core);
            let after = by_statement(core, "Sign off with the first name only").unwrap();
            assert_eq!((after.status.as_str(), after.support), ("rejected", 2));
        }

        #[test]
        fn drafts_sent_as_written_count_for_what_applied_and_overridden_entries_are_proposed_for_removal() {
            let s = scratch("compare-health");
            let core = &s.1;
            let change = block_on(core.apply_guide_edits(
                vec![GuideEdit::Add {
                    fields: GuideEntryFields {
                        category: "B6".into(),
                        kind: GuideKind::Guideline,
                        statement: "Sign off with 'Best regards, John'".into(),
                        scope: GuideScope::default(),
                        check: None,
                    },
                    status: GuideStatus::Accepted,
                    source: GuideSource::You,
                    origin: None,
                }],
                "test".into(),
            ))
            .unwrap();
            let entry = change.entries[0].id;
            pair(core, AI, AI, 0.0);
            review(core);
            let db = core.db().unwrap();
            assert_eq!(rt(db.read(store::health)).unwrap(), vec![(entry, 1, 0)], "sent as written: support");

            // Three pairs that each edit the entry: it is proposed for removal.
            let ids: Vec<i64> = (0..3).map(|_| pair(core, AI, SENT, 0.4)).collect();
            let (pairs, entries) = block_on(core.compare_pairs(&ids)).unwrap();
            assert!(pairs.iter().all(|p| p.applied == vec![entry]));
            for p in &pairs {
                let answer = format!(
                    r#"{{"proposals": [{{"op": "edit", "id": {entry}, "statement": "Sign off with 'J'", "evidence": [{{"pair": {}, "sent": "J", "ai": "Best regards"}}]}}]}}"#,
                    p.id
                );
                assert!(block_on(core.merge_compare(std::slice::from_ref(p), &entries, &answer)).unwrap());
            }
            assert_eq!(rt(db.read(store::health)).unwrap(), vec![(entry, 1, 3)]);
            let all = rt(db.read(|c| store::proposals(c, &["watching", "proposed"]))).unwrap();
            let edit = all.iter().find(|p| p.op == "edit").unwrap();
            assert_eq!((edit.status.as_str(), edit.support), ("proposed", 3));
            let remove = all.iter().find(|p| p.op == "remove").expect("proposed for removal");
            assert_eq!((remove.entry_id, remove.status.as_str(), remove.support), (Some(entry), "proposed", 3));
        }
    }
}
