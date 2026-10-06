//! The Analysis section's queue (spec §14.10): the proposals waiting, the
//! pairs behind them, deciding them (each decision one undoable change
//! with the guide, ADR 0006), the unseen dot and the metrics.

use std::collections::BTreeMap;

use mail_store::analysis::{self as store, ProposalRow};

use crate::guide::{
    AnalysisStep, GuideChange, GuideEdit, GuideEntryFields, GuideKind, GuideScope, GuideSource, GuideStatus,
};
use crate::{Core, CoreError, CoreEvent, ErrorKind, runtime};

/// `analysis_meta` key: when the user last opened Analysis.
/// When the user last looked at each page's proposals; before pages had
/// their own, one time for both (still read when a page has none).
const LAST_VIEWED: &str = "last_viewed_at";
const LAST_VIEWED_RULES: &str = "last_viewed_rules_at";
const LAST_VIEWED_FACTS: &str = "last_viewed_facts_at";

/// Where proposals are shown (spec §14.10): proposed rules in the Writing
/// Guide, proposed facts in Facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ProposalPage {
    Rules,
    Facts,
}
/// The metrics' window: four weeks.
const WEEK_MS: i64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AnalysisOp {
    Add,
    Edit,
    Rescope,
    Remove,
}

/// A proposed change to the writing guide, as Analysis shows it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AnalysisProposalInfo {
    pub id: i64,
    pub op: AnalysisOp,
    /// The entry an edit, rescope or removal changes.
    pub entry_id: Option<i64>,
    pub category: String,
    pub kind: GuideKind,
    /// What the guide would say (for a removal: what goes).
    pub statement: String,
    pub scope: GuideScope,
    /// The entry as it is now, for before → after.
    pub before_statement: Option<String>,
    pub before_scope: Option<GuideScope>,
    /// An accepted entry an added statement goes against.
    pub contradicts: Option<String>,
    /// Pairs behind it.
    pub support: u32,
    pub watching: bool,
    /// Shown since the user last opened Analysis.
    pub unseen: bool,
    pub shown_at: Option<i64>,
}

/// A proposed fact, category or starter set (spec §14.11).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AnalysisFactProposalInfo {
    pub id: i64,
    /// `fact`, `category` or `starter`.
    pub kind: String,
    pub op: AnalysisOp,
    pub category: String,
    pub category_name: String,
    pub label: String,
    pub value: String,
    /// The fact as it is now, for an alteration.
    pub before_value: Option<String>,
    pub fact_id: Option<i64>,
    pub as_of: Option<i64>,
    /// How freely drafts use it once accepted, unless the user picks
    /// otherwise: the fact's own for an alteration, else its category's.
    pub use_: crate::facts::FactUse,
    /// The words in the user's mail that state it.
    pub quote: String,
    pub message_id: String,
    /// A new category: its name, description and how many facts it takes.
    pub name: String,
    pub description: String,
    pub fact_count: u32,
    /// A starter set's id.
    pub starter: String,
    pub support: u32,
    pub watching: bool,
    pub unseen: bool,
    pub shown_at: Option<i64>,
}

/// Everything waiting in Analysis.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AnalysisQueue {
    /// Shown proposals for the writing guide, newest first.
    pub guide: Vec<AnalysisProposalInfo>,
    /// Shown proposals for facts, newest first.
    pub facts: Vec<AnalysisFactProposalInfo>,
    /// Proposals still short of their threshold.
    pub watching: Vec<AnalysisProposalInfo>,
    /// Learning runs' decisions waiting (spec §14.9, shown here).
    pub learning_decisions: u32,
    /// Something new since the user last looked: the account's dot.
    pub unseen: bool,
    /// New proposed rules (Writing Guide's dot) and facts (Facts' dot).
    pub unseen_rules: bool,
    pub unseen_facts: bool,
}

/// One pair behind a proposal: what the AI wrote and what the user sent.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AnalysisPairInfo {
    pub composition_id: i64,
    pub created_at: i64,
    pub kind: String,
    pub subject: String,
    pub to: Vec<String>,
    pub instruction: String,
    /// `None` once purged (retention).
    pub ai_text: Option<String>,
    pub sent_text: Option<String>,
    /// The words the proposal cites: from the sent text, and what they
    /// replaced in the AI's.
    pub sent_quote: String,
    pub ai_quote: String,
}

/// How much AI drafts get changed (spec §14.10): the median distance per
/// week over four weeks (oldest first; `None` for a week with nothing),
/// and how many were sent as written.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AnalysisMetrics {
    pub weekly_median: Vec<Option<f64>>,
    pub compared: u32,
    pub sent_as_written: u32,
}

/// How drafts that applied an entry fared.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideEntryHealth {
    pub entry_id: i64,
    pub unchanged: u32,
    pub overridden: u32,
}

fn op(s: &str) -> AnalysisOp {
    match s {
        "edit" => AnalysisOp::Edit,
        "rescope" => AnalysisOp::Rescope,
        "remove" => AnalysisOp::Remove,
        _ => AnalysisOp::Add,
    }
}

fn scope(json: &str) -> GuideScope {
    serde_json::from_str(json).unwrap_or_default()
}

/// The guide edit that accepting a proposal makes; `fields` when the user
/// edited it first. `None` when its entry has gone.
fn edit_for(
    p: &ProposalRow,
    entries: &BTreeMap<i64, crate::guide::GuideEntry>,
    fields: Option<GuideEntryFields>,
) -> Option<GuideEdit> {
    let kind = p.kind.as_deref().and_then(GuideKind::parse).unwrap_or(GuideKind::Guideline);
    let proposed = GuideEntryFields {
        category: p.category.clone(),
        kind,
        statement: p.statement.clone(),
        scope: scope(&p.scope_json),
        check: None,
    };
    match op(&p.op) {
        AnalysisOp::Add => Some(GuideEdit::Add {
            fields: fields.unwrap_or(proposed),
            status: GuideStatus::Accepted,
            source: GuideSource::Learned,
            origin: Some("analysis".into()),
        }),
        AnalysisOp::Edit | AnalysisOp::Rescope => {
            let old = entries.get(&p.entry_id?)?;
            let fields = fields.unwrap_or(GuideEntryFields { check: old.check.clone(), ..proposed });
            Some(GuideEdit::Update { id: old.id, fields })
        }
        AnalysisOp::Remove => {
            let old = entries.get(&p.entry_id?)?;
            Some(GuideEdit::Delete { id: old.id })
        }
    }
}

impl Core {
    async fn fact_proposals(
        &self,
        rows: &[ProposalRow],
        viewed: i64,
    ) -> Result<Vec<AnalysisFactProposalInfo>, CoreError> {
        if rows.is_empty() {
            return Ok(vec![]);
        }
        let categories = self.fact_categories().await?;
        let facts = self.list_facts(vec![crate::facts::FactStatus::Accepted]).await?;
        let mut out: Vec<AnalysisFactProposalInfo> = rows
            .iter()
            .filter_map(|r| {
                let p: crate::analysis_glean::FactPayload = serde_json::from_str(r.payload_json.as_deref()?).ok()?;
                let current = p
                    .fact_id
                    .and_then(|id| facts.iter().find(|f| f.id == id && f.scope == crate::facts::FactScope::Account));
                // A change to a fact that has gone since is moot.
                if p.fact_id.is_some() && current.is_none() {
                    return None;
                }
                let use_ = current.map_or_else(
                    || {
                        categories
                            .iter()
                            .find(|c| c.key == p.category)
                            .map_or(crate::facts::FactUse::Free, |c| c.default_use)
                    },
                    |f| f.use_,
                );
                Some(AnalysisFactProposalInfo {
                    id: r.id,
                    op: op(&r.op),
                    category_name: categories
                        .iter()
                        .find(|c| c.key == p.category)
                        .map_or(p.category.clone(), |c| c.name.clone()),
                    category: p.category,
                    label: p.label,
                    value: p.value,
                    before_value: current.map(|f| f.value.clone()),
                    fact_id: p.fact_id,
                    as_of: p.as_of,
                    use_,
                    quote: p.quote,
                    message_id: p.message_id,
                    name: p.name,
                    description: p.description,
                    fact_count: p.fact_ids.len() as u32,
                    starter: p.starter,
                    kind: p.kind,
                    support: r.support.max(0) as u32,
                    watching: r.status == "watching",
                    unseen: r.shown_at.is_some_and(|at| at > viewed),
                    shown_at: r.shown_at,
                })
            })
            .collect();
        out.sort_by_key(|p| std::cmp::Reverse((p.shown_at, p.id)));
        Ok(out)
    }

    pub(crate) fn analysis_changed(&self) {
        self.account_events().emit(CoreEvent::AnalysisChanged);
    }

    async fn accepted_entries(&self) -> Result<BTreeMap<i64, crate::guide::GuideEntry>, CoreError> {
        Ok(self.list_guide_entries(vec![GuideStatus::Accepted]).await?.into_iter().map(|e| (e.id, e)).collect())
    }

    async fn proposal_rows(&self, ids: Vec<i64>) -> Result<Vec<ProposalRow>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let mut out = Vec::new();
                    for id in ids {
                        out.extend(store::proposal(c, id)?);
                    }
                    Ok(out)
                })
                .await?)
        })
        .await
    }
}

#[uniffi::export]
impl Core {
    /// What waits in Analysis (spec §14.10).
    pub async fn analysis_queue(&self) -> Result<AnalysisQueue, CoreError> {
        let entries = self.accepted_entries().await?;
        let learning = self.guide_decisions().await?.len() as u32;
        let db = self.db()?;
        let (rows, (viewed, viewed_facts), learned_at) = runtime::run(async move {
            Ok(db
                .read(|c| {
                    let at = |key| -> Result<Option<i64>, mail_store::StoreError> {
                        Ok(store::meta(c, key)?.and_then(|v| v.parse::<i64>().ok()))
                    };
                    let both = at(LAST_VIEWED)?.unwrap_or(0);
                    let viewed = (at(LAST_VIEWED_RULES)?.unwrap_or(both), at(LAST_VIEWED_FACTS)?.unwrap_or(both));
                    let learned_at = mail_store::guide::runs(c, 1)?
                        .into_iter()
                        .next()
                        .filter(|r| r.status == "done")
                        .and_then(|r| r.finished_at);
                    Ok((store::proposals(c, &["watching", "proposed"])?, viewed, learned_at))
                })
                .await?)
        })
        .await?;
        let mut guide = Vec::new();
        let mut watching = Vec::new();
        let fact_rows: Vec<ProposalRow> =
            rows.iter().filter(|p| p.target == "fact" && p.status == "proposed").cloned().collect();
        for p in rows.into_iter().filter(|p| p.target == "guide" && p.support > 0) {
            let entry = p.entry_id.and_then(|id| entries.get(&id));
            // A change to an entry that has gone since is moot.
            if p.entry_id.is_some() && entry.is_none() {
                continue;
            }
            let info = AnalysisProposalInfo {
                id: p.id,
                op: op(&p.op),
                entry_id: p.entry_id,
                category: p.category.clone(),
                kind: p.kind.as_deref().and_then(GuideKind::parse).unwrap_or(GuideKind::Guideline),
                statement: p.statement.clone(),
                scope: scope(&p.scope_json),
                before_statement: entry.map(|e| e.statement.clone()),
                before_scope: entry.map(|e| e.scope.clone()),
                contradicts: p.contradicts_entry_id.and_then(|id| entries.get(&id)).map(|e| e.statement.clone()),
                support: p.support.max(0) as u32,
                watching: p.status == "watching",
                unseen: p.shown_at.is_some_and(|at| at > viewed),
                shown_at: p.shown_at,
            };
            if info.watching { watching.push(info) } else { guide.push(info) }
        }
        guide.sort_by_key(|p| std::cmp::Reverse((p.shown_at, p.id)));
        watching.sort_by_key(|p| std::cmp::Reverse((p.support, p.id)));
        let facts = self.fact_proposals(&fact_rows, viewed_facts).await?;
        let unseen_rules = guide.iter().any(|p| p.unseen) || (learning > 0 && learned_at.is_some_and(|at| at > viewed));
        let unseen_facts = facts.iter().any(|p| p.unseen);
        Ok(AnalysisQueue {
            guide,
            facts,
            watching,
            learning_decisions: learning,
            unseen: unseen_rules || unseen_facts,
            unseen_rules,
            unseen_facts,
        })
    }

    /// The user looked at a page's proposals: its dot clears.
    pub async fn analysis_seen(&self, page: ProposalPage) -> Result<(), CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis().to_string();
        let key = match page {
            ProposalPage::Rules => LAST_VIEWED_RULES,
            ProposalPage::Facts => LAST_VIEWED_FACTS,
        };
        // No event: the app asks after it has re-read the queue.
        runtime::run(async move { Ok(db.write(move |tx| store::set_meta(tx, key, &now)).await?) }).await
    }

    /// The pairs behind a proposal, oldest first.
    pub async fn analysis_pairs(&self, proposal_id: i64) -> Result<Vec<AnalysisPairInfo>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let mut out = Vec::new();
                    for e in store::evidence(c, proposal_id)? {
                        let Some(r) = mail_store::compositions::get(c, e.composition_id)? else { continue };
                        out.push(AnalysisPairInfo {
                            composition_id: r.id,
                            created_at: r.created_at,
                            kind: r.kind.as_str().into(),
                            subject: r.subject,
                            to: r.recipients.to,
                            instruction: r.instruction,
                            ai_text: r.ai_text,
                            sent_text: r.sent_text,
                            sent_quote: e.sent_quote,
                            ai_quote: e.ai_quote,
                        });
                    }
                    Ok(out)
                })
                .await?)
        })
        .await
    }

    /// Accept or reject proposals: one change, undoable with the guide's
    /// (`undo_guide_change`). Accepting edits the guide and makes a new
    /// version; rejecting means it is not proposed again.
    pub async fn decide_analysis_proposals(&self, ids: Vec<i64>, accept: bool) -> Result<GuideChange, CoreError> {
        if ids.is_empty() {
            return Err(CoreError::new(ErrorKind::InvalidInput, "nothing to decide"));
        }
        // Only open guide proposals: a double Return or a stale list does
        // not accept one twice.
        let rows: Vec<ProposalRow> = self
            .proposal_rows(ids)
            .await?
            .into_iter()
            .filter(|p| p.target == "guide" && matches!(p.status.as_str(), "proposed" | "watching"))
            .collect();
        let entries = self.accepted_entries().await?;
        let mut edits = Vec::new();
        let mut decided = Vec::new();
        // Changes to one entry are made together, on the entry as it is
        // now: an edit sets its statement and kind, a rescope its scope,
        // and a removal wins over both.
        let mut per_entry: BTreeMap<i64, Option<GuideEntryFields>> = BTreeMap::new();
        for p in &rows {
            if accept {
                match (op(&p.op), p.entry_id) {
                    (AnalysisOp::Add, _) => edits.extend(edit_for(p, &entries, None)),
                    (o, Some(id)) => {
                        let Some(current) = entries.get(&id) else { continue };
                        let slot = per_entry.entry(id).or_insert_with(|| {
                            Some(GuideEntryFields {
                                category: current.category.clone(),
                                kind: current.kind,
                                statement: current.statement.clone(),
                                scope: current.scope.clone(),
                                check: current.check.clone(),
                            })
                        });
                        match (o, slot.as_mut()) {
                            (AnalysisOp::Remove, _) => *slot = None,
                            (AnalysisOp::Edit, Some(f)) => {
                                f.statement = p.statement.clone();
                                f.kind = p.kind.as_deref().and_then(GuideKind::parse).unwrap_or(f.kind);
                            }
                            (AnalysisOp::Rescope, Some(f)) => f.scope = scope(&p.scope_json),
                            _ => {}
                        }
                    }
                    // Its entry has gone: nothing to accept.
                    _ => continue,
                }
            }
            decided.push(p.id);
        }
        for (id, fields) in per_entry {
            edits.push(match fields {
                Some(fields) => GuideEdit::Update { id, fields },
                None => GuideEdit::Delete { id },
            });
        }
        if decided.is_empty() {
            return Err(CoreError::new(ErrorKind::NotFound, "the entry this changes is no longer in your guide"));
        }
        let status = if accept { "accepted" } else { "rejected" };
        let change = self
            .apply_edits_with(
                edits,
                format!("analysis {}", if accept { "accept" } else { "reject" }),
                Some(AnalysisStep::Decide { ids: decided, status }),
            )
            .await?;
        self.analysis_changed();
        Ok(change)
    }

    /// Accept or reject fact proposals: one change on the account's facts
    /// stack (`undo_fact_change`). `uses` says, by proposal id, how freely
    /// drafts may use an accepted fact; one not named keeps its default.
    pub async fn decide_fact_analysis_proposals(
        &self,
        ids: Vec<i64>,
        accept: bool,
        uses: std::collections::HashMap<i64, crate::facts::FactUse>,
    ) -> Result<crate::facts::FactChange, CoreError> {
        let rows: Vec<ProposalRow> =
            self.proposal_rows(ids).await?.into_iter().filter(|r| r.target == "fact").collect();
        if rows.is_empty() {
            return Err(CoreError::new(ErrorKind::InvalidInput, "nothing to decide"));
        }
        self.decide_fact_proposals(&rows, accept, &uses).await
    }

    /// Accept a proposal as the user edited it.
    pub async fn accept_analysis_proposal_edited(
        &self,
        id: i64,
        fields: GuideEntryFields,
    ) -> Result<GuideChange, CoreError> {
        let rows = self.proposal_rows(vec![id]).await?;
        let p = rows.first().ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that proposal is gone"))?;
        let entries = self.accepted_entries().await?;
        let edit = edit_for(p, &entries, Some(fields))
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "the entry this changes is no longer in your guide"))?;
        let change = self
            .apply_edits_with(
                vec![edit],
                "analysis accept edited".into(),
                Some(AnalysisStep::Decide { ids: vec![id], status: "accepted" }),
            )
            .await?;
        self.analysis_changed();
        Ok(change)
    }

    /// "Ignore Edits to This Message": the pair no longer counts for any
    /// open proposal. Undoable.
    pub async fn ignore_analysis_pair(&self, composition_id: i64) -> Result<GuideChange, CoreError> {
        let change = self
            .apply_edits_with(vec![], "analysis ignore pair".into(), Some(AnalysisStep::IgnorePair(composition_id)))
            .await?;
        self.analysis_changed();
        Ok(change)
    }

    /// How much AI drafts got changed over the last four weeks.
    pub async fn analysis_metrics(&self) -> Result<AnalysisMetrics, CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let rows: Vec<(i64, f64)> = runtime::run(async move {
            Ok(db
                .read(move |c| {
                    Ok(c.prepare_cached(
                        "SELECT created_at, distance FROM ai_compositions
                         WHERE distance IS NOT NULL AND created_at >= ?1",
                    )?
                    .query_map([now - 4 * WEEK_MS], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<_, _>>()?)
                })
                .await?)
        })
        .await?;
        let mut weeks: Vec<Vec<f64>> = vec![vec![]; 4];
        for (at, d) in &rows {
            let ago = ((now - at) / WEEK_MS).clamp(0, 3) as usize;
            weeks[3 - ago].push(*d);
        }
        let median = |v: &mut Vec<f64>| -> Option<f64> {
            if v.is_empty() {
                return None;
            }
            v.sort_by(|a, b| a.total_cmp(b));
            let n = v.len();
            Some(if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 })
        };
        let unchanged = rows.iter().filter(|(_, d)| *d <= store::UNCHANGED).count() as u32;
        Ok(AnalysisMetrics {
            weekly_median: weeks.iter_mut().map(median).collect(),
            compared: rows.len() as u32,
            sent_as_written: unchanged,
        })
    }

    /// Snapshots and previews: proposed facts as a review would leave them
    /// (a new fact, a changed one, a new category), for facts the account
    /// already has ("Occupation or role" gets a new value when there is one).
    pub async fn debug_seed_fact_proposals(&self) -> Result<(), CoreError> {
        use crate::analysis_glean::{FactPayload, Found};
        let facts = self.list_facts(vec![crate::facts::FactStatus::Accepted]).await?;
        let quote = |q: &str| FactPayload { message_id: "demo".into(), quote: q.into(), ..Default::default() };
        let mut found = vec![
            Found::Add(FactPayload {
                kind: "fact".into(),
                category: "availability".into(),
                label: "Working hours".into(),
                value: "9 to 5 Pacific, weekdays".into(),
                ..quote("I'm around 9 to 5 Pacific on weekdays if you want to talk.")
            }),
            Found::Add(FactPayload {
                kind: "fact".into(),
                category: "work".into(),
                label: "Company".into(),
                value: "Northwind Labs".into(),
                ..quote("We started Northwind Labs two years ago.")
            }),
            Found::Category(FactPayload {
                kind: "category".into(),
                category: "other".into(),
                name: "Speaking".into(),
                description: "Talks the user gives and the topics they speak on".into(),
                ..quote("Happy to give the talk on hiring again in March.")
            }),
        ];
        if let Some(role) = facts.iter().find(|f| f.label == "Occupation or role") {
            found.push(Found::Alter(FactPayload {
                kind: "fact".into(),
                category: role.category.clone(),
                label: role.label.clone(),
                value: "Founder and CEO".into(),
                fact_id: Some(role.id),
                ..quote("As founder and CEO, I can sign off on this.")
            }));
        }
        self.merge_glean(found).await
    }

    /// Snapshots and previews: a few reviewed pairs and the proposals they
    /// back, on an account that has learned (the demo mailbox's).
    pub async fn debug_seed_analysis(&self) -> Result<(), CoreError> {
        use mail_store::analysis::{EvidenceRow, NewProposal};
        use mail_store::compositions::{self, Kind, NewComposition, Recipients, Source};
        let entries: Vec<crate::guide::GuideEntry> = self.accepted_entries().await?.into_values().collect();
        let first = entries.iter().find(|e| e.kind == GuideKind::Guideline).map(|e| (e.id, e.category.clone()));
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    let samples = [
                        ("Hi Priya,\nThank you so much for sending this over. I hope this finds you well.\nBest regards,\nJohn", "Hi Priya,\nThanks for sending this over.\nJ", 0.42),
                        ("Hi Sam,\nI wanted to follow up regarding the proposal.\nBest regards,\nJohn", "Sam, following up on the proposal.\nJ", 0.51),
                        ("Hello Alex,\nThat works perfectly for me.\nBest regards,\nJohn", "Alex, that works.\nJ", 0.38),
                        ("Hi Dana,\nHappy to help with that.\nBest,\nJohn", "Hi Dana,\nHappy to help with that.\nBest,\nJohn", 0.0),
                    ];
                    let mut pairs = Vec::new();
                    for (i, (ai, sent, distance)) in samples.iter().enumerate() {
                        let at = now - (i as i64 + 1) * 86_400_000;
                        let id = compositions::record(
                            tx,
                            &NewComposition {
                                source: Source::WritingHelp,
                                agent: Some("claude-code".into()),
                                kind: Kind::Reply,
                                draft_id: -(i as i64) - 1000,
                                thread_id: None,
                                in_reply_to: None,
                                recipients: Recipients::new(["priya@example.com"], []),
                                subject: "Re: The proposal".into(),
                                instruction: "Write a reply".into(),
                                ai_text: (*ai).into(),
                                ai_html: None,
                                guide_version: None,
                                audiences: vec![],
                            },
                            at,
                        )?;
                        compositions::draft_gone(tx, -(i as i64) - 1000, true, at)?;
                        compositions::set_matched(tx, id, &format!("seed-{id}"), "sent_draft", sent, *distance, at)?;
                        store::mark_reviewed(tx, &[id], at)?;
                        pairs.push(id);
                    }
                    let evidence = |ids: &[i64], sent: &str, ai: &str| -> Vec<EvidenceRow> {
                        ids.iter()
                            .map(|id| EvidenceRow { composition_id: *id, sent_quote: sent.into(), ai_quote: ai.into() })
                            .collect()
                    };
                    let add = |tx: &mail_store::Transaction<'_>, op: &str, entry: Option<i64>, category: &str, kind: &str,
                               statement: &str, ev: Vec<EvidenceRow>, threshold: i64| {
                        store::upsert_proposal(
                            tx,
                            &NewProposal {
                                target: "guide".into(),
                                op: op.into(),
                                entry_id: entry,
                                category: category.into(),
                                kind: Some(kind.into()),
                                statement: statement.into(),
                                scope_json: "{}".into(),
                                match_key: format!("seed|{op}|{statement}"),
                                threshold,
                                ..Default::default()
                            },
                            &ev,
                            now,
                        )
                    };
                    add(tx, "add", None, "B6", "guideline", "Sign off with just 'J'", evidence(&pairs[..3], "J", "Best regards,\nJohn"), 2)?;
                    add(tx, "add", None, "C8", "rule", "Never write 'I hope this finds you well'", evidence(&pairs[..1], "Thanks for sending this over.", "I hope this finds you well."), 3)?;
                    if let Some((id, category)) = first {
                        add(tx, "edit", Some(id), &category, "guideline", "Keep replies to two or three short sentences", evidence(&pairs[1..3], "following up on the proposal.", "I wanted to follow up regarding the proposal."), 2)?;
                        store::add_health(tx, id, 1, 2, now)?;
                    }
                    Ok(())
                })
                .await?)
        })
        .await?;
        self.analysis_changed();
        Ok(())
    }

    /// Per entry: drafts that applied it sent as written, and changed
    /// against it (the guide shows it on each entry).
    pub async fn guide_entry_health(&self) -> Result<Vec<GuideEntryHealth>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(store::health)
                .await?
                .into_iter()
                .map(|(entry_id, unchanged, overridden)| GuideEntryHealth {
                    entry_id,
                    unchanged: unchanged.max(0) as u32,
                    overridden: overridden.max(0) as u32,
                })
                .collect())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use mail_store::analysis::{EvidenceRow, NewProposal};
    use mail_store::compositions::{self, Kind, NewComposition, Recipients, Source};

    use super::*;
    use crate::analysis_run::tests::{learned, no_schedule, rt};

    fn scratch(name: &str) -> crate::guide::tests::Scratch {
        let s = crate::guide::tests::demo(name);
        s.1.debug_use_fake_agents();
        no_schedule(&s.1);
        learned(&s.1);
        s
    }

    fn pair(core: &Core, distance: f64, ago: i64) -> i64 {
        let db = core.db().unwrap();
        let at = mail_sync::now_millis() - ago;
        rt(db.write(move |tx| {
            let id = compositions::record(
                tx,
                &NewComposition {
                    source: Source::Agent,
                    agent: None,
                    kind: Kind::Reply,
                    draft_id: -at,
                    thread_id: None,
                    in_reply_to: None,
                    recipients: Recipients::new(["ann@example.com"], []),
                    subject: "Friday".into(),
                    instruction: "Say yes".into(),
                    ai_text: "Hi Ann,\nBest regards,\nJohn".into(),
                    ai_html: None,
                    guide_version: None,
                    audiences: vec![],
                },
                at,
            )?;
            compositions::draft_gone(tx, -at, true, at)?;
            compositions::set_matched(tx, id, &format!("m{id}"), "thread_next", "Hi Ann,\nJ", distance, at)?;
            Ok(id)
        }))
        .unwrap()
    }

    fn propose(core: &Core, op: &str, entry: Option<i64>, statement: &str, pairs: &[i64]) -> i64 {
        let db = core.db().unwrap();
        let (op, statement, pairs) = (op.to_owned(), statement.to_owned(), pairs.to_vec());
        rt(db.write(move |tx| {
            let p = NewProposal {
                target: "guide".into(),
                op: op.clone(),
                entry_id: entry,
                category: "B6".into(),
                kind: Some("guideline".into()),
                statement: statement.clone(),
                scope_json: "{}".into(),
                match_key: format!("{op}|{statement}"),
                threshold: 2,
                ..Default::default()
            };
            let evidence: Vec<EvidenceRow> = pairs
                .iter()
                .map(|id| EvidenceRow { composition_id: *id, sent_quote: "J".into(), ai_quote: "Best regards".into() })
                .collect();
            Ok(store::upsert_proposal(tx, &p, &evidence, mail_sync::now_millis())?.unwrap())
        }))
        .unwrap()
    }

    fn entry(core: &Core, statement: &str) -> i64 {
        block_on(core.apply_guide_edits(
            vec![GuideEdit::Add {
                fields: GuideEntryFields {
                    category: "B6".into(),
                    kind: GuideKind::Guideline,
                    statement: statement.into(),
                    scope: GuideScope::default(),
                    check: None,
                },
                status: GuideStatus::Accepted,
                source: GuideSource::You,
                origin: None,
            }],
            "test".into(),
        ))
        .unwrap()
        .entries[0]
            .id
    }

    #[test]
    fn the_queue_shows_proposals_and_its_dot_clears_when_seen() {
        let s = scratch("queue-seen");
        let core = &s.1;
        let (a, b) = (pair(core, 0.3, 0), pair(core, 0.3, 0));
        let shown = propose(core, "add", None, "Sign off with 'J'", &[a, b]);
        let quiet = propose(core, "add", None, "Open with 'Hi'", &[a]);
        let q = block_on(core.analysis_queue()).unwrap();
        assert_eq!(q.guide.iter().map(|p| p.id).collect::<Vec<_>>(), vec![shown]);
        assert_eq!(q.watching.iter().map(|p| p.id).collect::<Vec<_>>(), vec![quiet]);
        assert!(q.unseen && q.unseen_rules && !q.unseen_facts && q.guide[0].unseen);
        // Facts is another page: looking there leaves the rules' dot.
        block_on(core.analysis_seen(ProposalPage::Facts)).unwrap();
        assert!(block_on(core.analysis_queue()).unwrap().unseen_rules);
        block_on(core.analysis_seen(ProposalPage::Rules)).unwrap();
        let q = block_on(core.analysis_queue()).unwrap();
        assert!(!q.unseen && !q.unseen_rules && !q.guide[0].unseen, "opening the Writing Guide clears the dot");
        let pairs = block_on(core.analysis_pairs(shown)).unwrap();
        assert_eq!(pairs.len(), 2);
        assert_eq!((pairs[0].sent_text.as_deref(), pairs[0].sent_quote.as_str()), (Some("Hi Ann,\nJ"), "J"));
    }

    #[test]
    fn accepting_changes_the_guide_and_undo_puts_both_back() {
        let s = scratch("queue-accept");
        let core = &s.1;
        let (a, b) = (pair(core, 0.3, 0), pair(core, 0.3, 0));
        let old = entry(core, "Sign off with 'Best regards, John'");
        let edit = propose(core, "edit", Some(old), "Sign off with 'J'", &[a, b]);
        let q = block_on(core.analysis_queue()).unwrap();
        assert_eq!(q.guide[0].before_statement.as_deref(), Some("Sign off with 'Best regards, John'"));
        let version = block_on(core.guide_version()).unwrap();
        let change = block_on(core.decide_analysis_proposals(vec![edit], true)).unwrap();
        assert!(change.version > version, "a new guide version");
        let now = block_on(core.guide_entry(old)).unwrap().unwrap();
        assert_eq!(now.statement, "Sign off with 'J'");
        assert!(block_on(core.analysis_queue()).unwrap().guide.is_empty());

        block_on(core.undo_guide_change(change.change_id)).unwrap();
        assert_eq!(block_on(core.guide_entry(old)).unwrap().unwrap().statement, "Sign off with 'Best regards, John'");
        let q = block_on(core.analysis_queue()).unwrap();
        assert_eq!(q.guide.iter().map(|p| p.id).collect::<Vec<_>>(), vec![edit], "back in the queue");
        block_on(core.redo_guide_change(change.change_id)).unwrap();
        assert_eq!(block_on(core.guide_entry(old)).unwrap().unwrap().statement, "Sign off with 'J'");
    }

    #[test]
    fn accept_all_is_one_change_and_reject_is_undoable() {
        let s = scratch("queue-all");
        let core = &s.1;
        let (a, b) = (pair(core, 0.3, 0), pair(core, 0.3, 0));
        let one = propose(core, "add", None, "Sign off with 'J'", &[a, b]);
        let two = propose(core, "add", None, "Open with 'Hi'", &[a, b]);
        let change = block_on(core.decide_analysis_proposals(vec![one, two], true)).unwrap();
        assert_eq!(change.entries.len(), 2);
        assert!(change.entries.iter().all(|e| e.origin.as_deref() == Some("analysis")));
        block_on(core.undo_guide_change(change.change_id)).unwrap();
        assert_eq!(block_on(core.analysis_queue()).unwrap().guide.len(), 2, "one undo puts both back");
        assert!(block_on(core.list_guide_entries(vec![GuideStatus::Accepted])).unwrap().is_empty());

        let rejected = block_on(core.decide_analysis_proposals(vec![one], false)).unwrap();
        assert_eq!(block_on(core.analysis_queue()).unwrap().guide.len(), 1);
        block_on(core.undo_guide_change(rejected.change_id)).unwrap();
        assert_eq!(block_on(core.analysis_queue()).unwrap().guide.len(), 2);
    }

    #[test]
    fn an_ignored_pair_stops_counting_and_undo_counts_it_again() {
        let s = scratch("queue-ignore");
        let core = &s.1;
        let (a, b) = (pair(core, 0.3, 0), pair(core, 0.3, 0));
        let p = propose(core, "add", None, "Sign off with 'J'", &[a, b]);
        let change = block_on(core.ignore_analysis_pair(a)).unwrap();
        let q = block_on(core.analysis_queue()).unwrap();
        assert_eq!(q.guide[0].support, 1);
        block_on(core.ignore_analysis_pair(b)).unwrap();
        assert!(block_on(core.analysis_queue()).unwrap().guide.is_empty(), "nothing left behind it");
        block_on(core.undo_guide_change(change.change_id + 1)).unwrap();
        block_on(core.undo_guide_change(change.change_id)).unwrap();
        let q = block_on(core.analysis_queue()).unwrap();
        assert_eq!((q.guide[0].id, q.guide[0].support), (p, 2));
    }

    #[test]
    fn undoing_an_ignore_keeps_evidence_added_since() {
        let s = scratch("queue-ignore-later");
        let core = &s.1;
        let (a, b, c) = (pair(core, 0.3, 0), pair(core, 0.3, 0), pair(core, 0.3, 0));
        let p = propose(core, "add", None, "Sign off with 'J'", &[a, b]);
        let change = block_on(core.ignore_analysis_pair(a)).unwrap();
        propose(core, "add", None, "Sign off with 'J'", &[c]);
        block_on(core.undo_guide_change(change.change_id)).unwrap();
        let db = core.db().unwrap();
        let mut ids: Vec<i64> =
            rt(db.read(move |x| store::evidence(x, p))).unwrap().into_iter().map(|e| e.composition_id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![a, b, c], "the ignored pair is back and the later one stays");
        assert_eq!(block_on(core.analysis_queue()).unwrap().guide[0].support, 3);
    }

    #[test]
    fn accepting_a_removal_and_an_edit_of_one_entry_together_removes_it() {
        let s = scratch("queue-same-entry");
        let core = &s.1;
        let (a, b) = (pair(core, 0.3, 0), pair(core, 0.3, 0));
        let old = entry(core, "Sign off with 'Best regards, John'");
        let edit = propose(core, "edit", Some(old), "Sign off with 'J'", &[a, b]);
        let remove = propose(core, "remove", Some(old), "Sign off with 'Best regards, John'", &[a, b]);
        block_on(core.decide_analysis_proposals(vec![remove, edit], true)).unwrap();
        assert!(block_on(core.guide_entry(old)).unwrap().is_none(), "the removal wins");
        assert!(block_on(core.analysis_queue()).unwrap().guide.is_empty(), "both are decided");
        // Accepting again does nothing: they are decided.
        assert!(block_on(core.decide_analysis_proposals(vec![edit], true)).is_err());
    }

    #[test]
    fn old_texts_are_purged_and_the_metrics_stay() {
        let s = scratch("queue-purge");
        let core = &s.1;
        let day = 24 * 60 * 60 * 1000;
        let old = pair(core, 0.3, 10 * day);
        let fresh = pair(core, 0.2, day);
        let now = mail_sync::now_millis();
        let db = core.db().unwrap();
        rt(db.write(move |tx| {
            store::mark_reviewed(tx, &[old], now - 40 * day)?;
            store::mark_reviewed(tx, &[fresh], now - day)
        }))
        .unwrap();
        let before = block_on(core.analysis_metrics()).unwrap();
        assert_eq!(rt(core.purge_compositions(now)).unwrap(), 1, "older than the 30 days kept");
        let get = |id: i64| rt(db.read(move |c| compositions::get(c, id))).unwrap().unwrap();
        assert_eq!((get(old).ai_text, get(old).sent_text, get(old).distance), (None, None, Some(0.3)));
        assert!(get(fresh).ai_text.is_some());
        assert_eq!(block_on(core.analysis_metrics()).unwrap(), before, "the metrics need only the distance");
    }

    #[test]
    fn metrics_take_the_median_change_by_week() {
        let s = scratch("queue-metrics");
        let core = &s.1;
        let day = 24 * 60 * 60 * 1000;
        pair(core, 0.2, day);
        pair(core, 0.4, 2 * day);
        pair(core, 0.0, 3 * day);
        pair(core, 0.6, 10 * day);
        let m = block_on(core.analysis_metrics()).unwrap();
        assert_eq!(m.weekly_median, vec![None, None, Some(0.6), Some(0.2)]);
        assert_eq!((m.compared, m.sent_as_written), (4, 1));
    }
}

#[cfg(test)]
mod seed_tests {
    use futures::executor::block_on;

    use crate::analysis_run::tests::{learned, no_schedule};

    #[test]
    fn the_seeded_proposals_can_be_decided() {
        let s = crate::guide::tests::demo("queue-seed");
        let core = &s.1;
        core.debug_use_fake_agents();
        no_schedule(core);
        learned(core);
        block_on(core.debug_seed_analysis()).unwrap();
        let q = block_on(core.analysis_queue()).unwrap();
        assert!(!q.guide.is_empty());
        let ids = q.guide.iter().map(|p| p.id).collect();
        block_on(core.decide_analysis_proposals(ids, true)).unwrap();
    }
}
