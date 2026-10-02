//! Merging a writing guide into this account (spec §14.9), from another
//! account in the app or an exported file. Evidence never travels (ADR
//! 0004). Identical entries are skipped; entries on points this guide does
//! not cover are listed to add; where both guides say something different
//! the user's agent writes one high-level decision per point. Facts and
//! content rules (F) are always a decision. The app applies the result as
//! one undoable change (`apply_guide_edits`).

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::Value;

use crate::guide::{AudienceGroup, GuideEntry, GuideEntryFields, GuideStatus, category, clean_text, read_export};
use crate::{Core, CoreError, ErrorKind};

/// The first line of every merge prompt: how the fake agent knows one.
pub const MERGE_MARKER: &str = "OpenAGC writing guide merge";

/// One point where the guides differ: keep this account's entries, take
/// the incoming ones, or keep both with the incoming ones scoped.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideMergeDecision {
    /// "Sign-off".
    pub point: String,
    /// "Best here, Cheers incoming".
    pub summary: String,
    pub mine: Vec<GuideEntry>,
    pub incoming: Vec<GuideEntryFields>,
}

/// What merging would do, for the user to look at before anything changes.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideMergePlan {
    /// Where the entries come from ("john@actual.ai", "Writing Guide.json").
    pub origin: String,
    /// Entries to add: on points this guide does not cover.
    pub additions: Vec<GuideEntryFields>,
    /// Incoming entries already in this guide, skipped.
    pub identical: u32,
    pub decisions: Vec<GuideMergeDecision>,
    /// Incoming audience groups this account does not have.
    pub groups: Vec<AudienceGroup>,
}

/// One category where the guides differ: this account's entries and the
/// incoming ones.
pub(crate) type Conflict = (String, Vec<GuideEntry>, Vec<GuideEntryFields>);

fn same(a: &GuideEntryFields, b: &GuideEntry) -> bool {
    a.category == b.category && mail_store::guide::norm(&a.statement) == mail_store::guide::norm(&b.statement)
}

/// The deterministic part of a plan: identical entries skipped, additions
/// for uncovered points, and the conflicts (by category) left for
/// decisions. F entries are always conflicts.
pub(crate) fn split(
    mine: &[GuideEntry],
    incoming: Vec<GuideEntryFields>,
) -> (Vec<GuideEntryFields>, u32, Vec<Conflict>) {
    let mut additions = Vec::new();
    let mut identical = 0;
    let mut conflicts: Vec<Conflict> = Vec::new();
    for e in incoming {
        if mine.iter().any(|m| same(&e, m)) {
            identical += 1;
            continue;
        }
        let here: Vec<GuideEntry> = mine.iter().filter(|m| m.category == e.category).cloned().collect();
        if here.is_empty() && !e.category.starts_with('F') {
            additions.push(e);
            continue;
        }
        match conflicts.iter_mut().find(|(c, _, _)| *c == e.category) {
            Some((_, _, inc)) => inc.push(e),
            None => conflicts.push((e.category.clone(), here, vec![e])),
        }
    }
    (additions, identical, conflicts)
}

fn fallback(conflicts: Vec<Conflict>) -> Vec<GuideMergeDecision> {
    conflicts
        .into_iter()
        .map(|(cat, mine, incoming)| {
            let name = category(&cat).map(|c| c.name).unwrap_or("");
            let summary = match (mine.first(), incoming.first()) {
                (Some(m), Some(i)) => format!("Here: {} Incoming: {}", m.statement, i.statement),
                (None, Some(i)) => format!("Incoming: {}", i.statement),
                _ => String::new(),
            };
            GuideMergeDecision { point: format!("{cat} {name}"), summary, mine, incoming }
        })
        .collect()
}

pub fn prompt(conflicts: &[Conflict]) -> String {
    let mut lines = Vec::new();
    let mut index = 0;
    for (cat, mine, incoming) in conflicts {
        lines.push(format!("Category {cat}:"));
        for m in mine {
            lines.push(format!("  mine #{} [{}] {}", m.id, m.kind.as_str(), crate::guide_ai::fenced(&m.statement)));
        }
        for i in incoming {
            lines.push(format!("  incoming @{index} [{}] {}", i.kind.as_str(), crate::guide_ai::fenced(&i.statement)));
            index += 1;
        }
    }
    format!(
        "{MERGE_MARKER}\n\n\
         The user is merging another writing guide into theirs. Below are the points where the two differ. \
         Group them into a short set of high-level decisions, one per point of difference, each covering the \
         entries involved, so the user can choose which guide wins on that point.\n\n{}\n\n\
         Answer with only a JSON object:\n\
         {{\"decisions\": [{{\"point\": \"Sign-off\", \"summary\": \"'Best' here, 'Cheers' incoming\", \
         \"mine\": [entry ids], \"incoming\": [incoming numbers]}}]}}\n\
         Every mine and incoming entry above belongs to exactly one decision. Do not use tools.",
        lines.join("\n")
    )
}

/// Read the agent's decisions; entries it left out get a decision of their
/// own per category, so nothing is taken silently.
pub(crate) fn parse(text: &str, conflicts: Vec<Conflict>) -> Option<Vec<GuideMergeDecision>> {
    let value: Value = text.match_indices('{').find_map(|(i, _)| {
        serde_json::Deserializer::from_str(&text[i..])
            .into_iter::<Value>()
            .next()
            .and_then(Result::ok)
            .filter(|v| v.get("decisions").is_some())
    })?;
    let mine_all: Vec<GuideEntry> = conflicts.iter().flat_map(|(_, m, _)| m.clone()).collect();
    let incoming_all: Vec<GuideEntryFields> = conflicts.iter().flat_map(|(_, _, i)| i.clone()).collect();
    let mut used_mine = BTreeSet::new();
    let mut used_incoming = BTreeSet::new();
    let mut out = Vec::new();
    for d in value.get("decisions").and_then(Value::as_array).into_iter().flatten() {
        let ids = |k: &str| -> Vec<i64> {
            d.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_i64).collect()
        };
        let mine: Vec<GuideEntry> = ids("mine")
            .into_iter()
            .filter_map(|id| mine_all.iter().find(|m| m.id == id).cloned())
            .filter(|m| used_mine.insert(m.id))
            .collect();
        let incoming: Vec<GuideEntryFields> = ids("incoming")
            .into_iter()
            .filter(|i| *i >= 0 && (*i as usize) < incoming_all.len() && used_incoming.insert(*i as usize))
            .map(|i| incoming_all[i as usize].clone())
            .collect();
        if incoming.is_empty() {
            continue;
        }
        let text = |k: &str| clean_text(d.get(k).and_then(Value::as_str).unwrap_or(""));
        out.push(GuideMergeDecision { point: text("point"), summary: text("summary"), mine, incoming });
    }
    // Whatever the agent left out is still a decision.
    let left: Vec<Conflict> = conflicts
        .into_iter()
        .map(|(cat, mine, incoming)| {
            let mine: Vec<GuideEntry> = mine.into_iter().filter(|m| !used_mine.contains(&m.id)).collect();
            (cat, mine, incoming)
        })
        .collect();
    let mut offset = 0;
    let mut rest = Vec::new();
    for (cat, mine, incoming) in left {
        let n = incoming.len();
        let unused: Vec<GuideEntryFields> = incoming
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !used_incoming.contains(&(offset + i)))
            .map(|(_, e)| e)
            .collect();
        offset += n;
        if !unused.is_empty() {
            rest.push((cat, mine, unused));
        }
    }
    out.extend(fallback(rest));
    Some(out)
}

#[uniffi::export]
impl Core {
    /// Plan merging a guide into the open account, from another account
    /// (`from_account`) or an exported file (`json`). Nothing changes here.
    /// When the guides differ, `agent` writes the decisions; it must be
    /// ready then.
    pub async fn plan_guide_merge(
        self: Arc<Self>,
        from_account: Option<String>,
        json: Option<String>,
        agent: String,
    ) -> Result<GuideMergePlan, CoreError> {
        let (text, origin) = match (from_account, json) {
            (Some(account), _) => {
                if Some(&account) == self.effective_account_id().as_ref() {
                    return Err(CoreError::new(ErrorKind::InvalidInput, "choose another account to merge from"));
                }
                let core = self.clone();
                let exported =
                    crate::registry::scoped(Some(account.clone()), async move { core.export_guide(true, false).await })
                        .await?;
                let email = self.account_email(&account).unwrap_or(account);
                (exported, email)
            }
            (None, Some(json)) => (json, "an exported guide".to_owned()),
            (None, None) => return Err(CoreError::new(ErrorKind::InvalidInput, "choose a guide to merge")),
        };
        let (incoming, groups) = read_export(&text)?;
        if incoming.is_empty() {
            return Err(CoreError::new(ErrorKind::NotFound, "that guide has no entries"));
        }
        let mine = self.list_guide_entries(vec![GuideStatus::Accepted]).await?;
        let (additions, identical, conflicts) = split(&mine, incoming);
        let decisions = if conflicts.is_empty() {
            vec![]
        } else if mine.is_empty() {
            // Nothing here to weigh against: only facts and content rules,
            // each still a decision.
            fallback(conflicts)
        } else {
            if let Some(why) = self.agent_not_ready(&agent).await? {
                return Err(CoreError::new(ErrorKind::Agent, why));
            }
            let answer = self.ask_agent_hidden(&agent, prompt(&conflicts)).await?;
            parse(&answer, conflicts.clone()).unwrap_or_else(|| fallback(conflicts))
        };
        let have: Vec<String> = self.list_audience_groups().await?.into_iter().map(|g| g.name.to_lowercase()).collect();
        let groups = groups
            .into_iter()
            .filter(|g| !have.contains(&g.name.to_lowercase()))
            .map(|g| AudienceGroup {
                id: 0,
                name: g.name,
                status: crate::guide::AudienceStatus::Confirmed,
                description: g.description,
                members: g.members,
            })
            .collect();
        Ok(GuideMergePlan { origin, additions, identical, decisions, groups })
    }
}

impl Core {
    /// An account's address, for saying where merged entries came from.
    fn account_email(&self, account: &str) -> Option<String> {
        let index = std::fs::read_to_string(
            std::path::PathBuf::from(&self.config.data_dir).join("accounts").join("index.json"),
        )
        .ok()?;
        let v: Value = serde_json::from_str(&index).ok()?;
        v.as_array()?
            .iter()
            .find(|a| a.get("id").and_then(Value::as_str) == Some(account))?
            .get("email")?
            .as_str()
            .map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;
    use crate::guide::tests::demo;
    use crate::guide::{GuideEdit, GuideKind, GuideScope, GuideSource};

    fn f(cat: &str, statement: &str) -> GuideEntryFields {
        GuideEntryFields {
            category: cat.into(),
            kind: GuideKind::Guideline,
            statement: statement.into(),
            scope: GuideScope::default(),
            check: None,
        }
    }

    fn add(core: &Core, entries: Vec<GuideEntryFields>) {
        block_on(
            core.apply_guide_edits(
                entries
                    .into_iter()
                    .map(|fields| GuideEdit::Add {
                        fields,
                        status: GuideStatus::Accepted,
                        source: GuideSource::You,
                        origin: None,
                    })
                    .collect(),
                "x".into(),
            ),
        )
        .unwrap();
    }

    #[test]
    fn a_plan_skips_identical_adds_uncovered_points_and_asks_about_the_rest() {
        let s = demo("merge-plan");
        let core = &s.1;
        core.debug_use_fake_agents();
        add(core, vec![f("B6", "Sign off with 'Best'"), f("C1", "Use US spelling")]);
        let incoming = r#"{"format": "openagc-writing-guide", "version": 1,
            "entries": [
              {"category": "C1", "kind": "rule", "statement": "use US spelling."},
              {"category": "B6", "kind": "guideline", "statement": "Sign off with 'Cheers'"},
              {"category": "A1", "kind": "guideline", "statement": "Be warm"},
              {"category": "F3", "kind": "fact", "statement": "My phone: 555 0100"}
            ],
            "groups": [{"name": "Investors", "members": ["@fund.com"]}]}"#;
        let plan = block_on(core.clone().plan_guide_merge(None, Some(incoming.into()), "claude-code".into())).unwrap();
        assert_eq!(plan.identical, 1);
        assert_eq!(plan.additions.iter().map(|a| a.statement.as_str()).collect::<Vec<_>>(), ["Be warm"]);
        let points: Vec<usize> = plan.decisions.iter().map(|d| d.incoming.len()).collect();
        assert_eq!(points.iter().sum::<usize>(), 2, "the sign-off and the fact are decisions");
        let signoff = plan.decisions.iter().find(|d| d.incoming[0].category == "B6").unwrap();
        assert_eq!(signoff.mine[0].statement, "Sign off with 'Best'");
        assert!(plan.decisions.iter().any(|d| d.incoming[0].category == "F3"), "facts are never taken silently");
        assert_eq!(plan.groups[0].name, "Investors");
        assert_eq!(block_on(core.list_guide_entries(vec![])).unwrap().len(), 2, "nothing changed yet");
    }

    #[test]
    fn into_an_empty_guide_everything_but_content_rules_is_simply_added() {
        let s = demo("merge-empty");
        let core = &s.1;
        let incoming = r#"{"format": "openagc-writing-guide", "version": 1, "entries": [
            {"category": "A1", "kind": "guideline", "statement": "Be warm"},
            {"category": "F4", "kind": "rule", "statement": "Never invent figures"}]}"#;
        // No agent needed: nothing here to weigh against.
        let plan = block_on(core.clone().plan_guide_merge(None, Some(incoming.into()), "codex".into())).unwrap();
        assert_eq!(plan.additions.len(), 1);
        assert_eq!(plan.decisions.len(), 1);
        assert!(plan.decisions[0].mine.is_empty());
        assert!(block_on(core.clone().plan_guide_merge(None, Some("{}".into()), "codex".into())).is_err());
    }

    #[test]
    fn decisions_the_agent_leaves_out_are_still_asked() {
        let mine = vec![];
        let conflicts = vec![
            ("B6".to_owned(), mine.clone(), vec![f("B6", "Cheers")]),
            ("F3".to_owned(), mine, vec![f("F3", "Phone")]),
        ];
        let answer =
            r#"{"decisions": [{"point": "Sign-off", "summary": "Cheers incoming", "mine": [], "incoming": [0]}]}"#;
        let decisions = parse(answer, conflicts).unwrap();
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].point, "Sign-off");
        assert_eq!(decisions[1].incoming[0].statement, "Phone");
    }
}
