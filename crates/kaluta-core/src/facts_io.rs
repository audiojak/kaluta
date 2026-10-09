//! Exporting facts and merging them into another account (spec §14.11):
//! Markdown to read or share, JSON to merge, with custom categories.

use serde::{Deserialize, Serialize};

use crate::analysis_glean::FactPayload;
use crate::facts::{CategoryEdit, FactEdit, FactFields, FactScope, FactSource, FactStatus, FactUse, similar};
use crate::{Core, CoreError, ErrorKind, runtime};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ExportedCategory {
    key: String,
    name: String,
    description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ExportedFact {
    category: String,
    label: String,
    value: String,
    #[serde(rename = "use")]
    use_: String,
    as_of: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Exported {
    /// Files exported before the project was named Kaluta say `openagc_facts`.
    #[serde(alias = "openagc_facts")]
    kaluta_facts: u32,
    categories: Vec<ExportedCategory>,
    facts: Vec<ExportedFact>,
}

/// What a merge did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FactMergeResult {
    /// Facts added; one change, undone with `undo_fact_change` (0 when none).
    pub change_id: i64,
    pub added: u32,
    /// Facts that differ from this account's: proposals in Analysis.
    pub proposed: u32,
    /// The same here already.
    pub skipped: u32,
    pub categories_added: u32,
}

#[uniffi::export]
impl Core {
    /// This account's accepted facts (not the global ones) as Markdown (to
    /// read or share) or JSON (to merge into another account).
    pub async fn export_facts(&self, json: bool) -> Result<String, CoreError> {
        let facts: Vec<_> = self
            .list_facts(vec![FactStatus::Accepted])
            .await?
            .into_iter()
            .filter(|f| f.scope == FactScope::Account)
            .collect();
        let categories = self.fact_categories().await?;
        if json {
            let used: Vec<&String> = facts.iter().map(|f| &f.category).collect();
            let out = Exported {
                kaluta_facts: 1,
                categories: categories
                    .iter()
                    .filter(|c| !c.builtin && used.contains(&&c.key))
                    .map(|c| ExportedCategory {
                        key: c.key.clone(),
                        name: c.name.clone(),
                        description: c.description.clone(),
                    })
                    .collect(),
                facts: facts
                    .iter()
                    .map(|f| ExportedFact {
                        category: f.category.clone(),
                        label: f.label.clone(),
                        value: f.value.clone(),
                        use_: f.use_.as_str().into(),
                        as_of: f.as_of,
                    })
                    .collect(),
            };
            return serde_json::to_string_pretty(&out).map_err(|e| CoreError::new(ErrorKind::Internal, e.to_string()));
        }
        let mut md = String::from("# Facts\n");
        for c in &categories {
            let mine: Vec<_> = facts.iter().filter(|f| f.category == c.key).collect();
            if mine.is_empty() {
                continue;
            }
            md.push_str(&format!("\n## {}\n\n", c.name));
            if !c.builtin && !c.description.is_empty() {
                md.push_str(&format!("{}\n\n", c.description));
            }
            for f in mine {
                let note = match f.use_ {
                    FactUse::Free => "",
                    FactUse::Ask => " (ask before using)",
                    FactUse::Never => " (never share)",
                };
                md.push_str(&format!("- **{}:** {}{note}\n", f.label, f.value));
            }
        }
        Ok(md)
    }

    /// Merge exported facts (JSON) into this account: missing custom
    /// categories are created (a close name maps to the one there is),
    /// new facts added as one undoable change, and a fact whose value
    /// differs from this account's becomes a proposal in Analysis.
    pub async fn merge_facts(&self, json: String) -> Result<FactMergeResult, CoreError> {
        let incoming: Exported = serde_json::from_str(&json)
            .map_err(|_| CoreError::new(ErrorKind::InvalidInput, "that file is not a Kaluta facts export"))?;
        let have = self.fact_categories().await?;
        let mine: Vec<_> = self
            .list_facts(vec![FactStatus::Accepted])
            .await?
            .into_iter()
            .filter(|f| f.scope == FactScope::Account)
            .collect();
        // Where each incoming category goes: a key here, or a name to make.
        let mut category_edits = Vec::new();
        let target = |key: &str| -> String {
            if have.iter().any(|c| c.key == key && c.builtin) {
                return key.to_owned();
            }
            match incoming.categories.iter().find(|c| c.key == key) {
                Some(c) => have.iter().find(|h| similar(&h.name, &c.name)).map_or(c.name.clone(), |h| h.key.clone()),
                None => "other".into(),
            }
        };
        for c in &incoming.categories {
            if !have.iter().any(|h| similar(&h.name, &c.name)) && incoming.facts.iter().any(|f| f.category == c.key) {
                category_edits.push(CategoryEdit::Add { name: c.name.clone(), description: c.description.clone() });
            }
        }
        let mut edits = Vec::new();
        let mut proposals = Vec::new();
        let mut skipped = 0;
        for f in &incoming.facts {
            let category = target(&f.category);
            match mine.iter().find(|m| m.category == category && m.label.eq_ignore_ascii_case(&f.label)) {
                Some(m) if m.value.trim() == f.value.trim() => skipped += 1,
                Some(m) => proposals.push(FactPayload {
                    kind: "fact".into(),
                    category: m.category.clone(),
                    label: m.label.clone(),
                    value: f.value.clone(),
                    as_of: f.as_of,
                    fact_id: Some(m.id),
                    quote: "From another account's facts".into(),
                    ..Default::default()
                }),
                None => edits.push(FactEdit::Add {
                    fields: FactFields {
                        category,
                        label: f.label.clone(),
                        value: f.value.clone(),
                        use_: FactUse::parse(&f.use_),
                        as_of: f.as_of,
                    },
                    status: FactStatus::Accepted,
                    source: FactSource::You,
                }),
            }
        }
        let (added, categories_added) = (edits.len() as u32, category_edits.len() as u32);
        let change_id = if edits.is_empty() && category_edits.is_empty() {
            0
        } else {
            self.apply_facts_with(FactScope::Account, edits, category_edits, "merge facts".into(), None)
                .await?
                .change_id
        };
        let proposed = proposals.len() as u32;
        if !proposals.is_empty() {
            let db = self.db()?;
            let now = mail_sync::now_millis();
            runtime::run(async move {
                Ok(db
                    .write(move |tx| {
                        for p in &proposals {
                            let new = mail_store::analysis::NewProposal {
                                target: "fact".into(),
                                op: "edit".into(),
                                entry_id: p.fact_id,
                                category: p.category.clone(),
                                kind: Some("fact".into()),
                                statement: format!("{}: {}", p.label, p.value),
                                scope_json: "{}".into(),
                                match_key: format!(
                                    "alter|{}|{}",
                                    p.fact_id.unwrap_or_default(),
                                    crate::guide_ai::loose(&p.value)
                                ),
                                contradicts_entry_id: None,
                                payload_json: serde_json::to_string(p).ok(),
                                threshold: 1,
                            };
                            mail_store::analysis::upsert_counted(tx, &new, now)?;
                        }
                        Ok(())
                    })
                    .await?)
            })
            .await?;
            self.analysis_changed();
        }
        Ok(FactMergeResult { change_id, added, proposed, skipped, categories_added })
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;

    fn add(core: &Core, category: &str, label: &str, value: &str) {
        block_on(core.apply_fact_edits(
            vec![FactEdit::Add {
                fields: FactFields {
                    category: category.into(),
                    label: label.into(),
                    value: value.into(),
                    use_: FactUse::Free,
                    as_of: None,
                },
                status: FactStatus::Accepted,
                source: FactSource::You,
            }],
            "add".into(),
        ))
        .unwrap();
    }

    #[test]
    fn facts_export_and_merge_into_another_account() {
        let from = crate::guide::tests::demo("facts-export");
        let core = &from.1;
        let made = block_on(core.edit_fact_categories(vec![CategoryEdit::Add {
            name: "Properties".into(),
            description: "Homes I'm selling".into(),
        }]))
        .unwrap();
        add(core, &made.categories[0], "Elm Street", "3 bedrooms");
        add(core, "work", "Occupation or role", "CEO");
        add(core, "availability", "Time zone", "Pacific");
        let md = block_on(core.export_facts(false)).unwrap();
        assert!(md.contains("## Work\n\n- **Occupation or role:** CEO"), "{md}");
        assert!(md.contains("## Properties\n\nHomes I'm selling"), "{md}");
        let json = block_on(core.export_facts(true)).unwrap();

        let to = crate::guide::tests::demo("facts-import");
        let other = &to.1;
        add(other, "work", "Occupation or role", "CTO");
        add(other, "availability", "Time zone", "Pacific");
        let result = block_on(other.merge_facts(json.clone())).unwrap();
        assert_eq!((result.added, result.proposed, result.skipped, result.categories_added), (1, 1, 1, 1));
        let facts = block_on(other.list_facts(vec![FactStatus::Accepted])).unwrap();
        let cats = block_on(other.fact_categories()).unwrap();
        let properties = cats.iter().find(|c| c.name == "Properties").expect("the category came with it");
        assert!(facts.iter().any(|f| f.category == properties.key && f.value == "3 bedrooms"));
        assert_eq!(
            facts.iter().find(|f| f.label == "Occupation or role").unwrap().value,
            "CTO",
            "a difference is a proposal"
        );
        let q = block_on(other.analysis_queue()).unwrap();
        assert_eq!(q.facts.len(), 1);
        assert_eq!((q.facts[0].value.as_str(), q.facts[0].before_value.as_deref()), ("CEO", Some("CTO")));
        block_on(other.undo_fact_change(result.change_id)).unwrap();
        assert_eq!(block_on(other.list_facts(vec![FactStatus::Accepted])).unwrap().len(), 2);

        // A file exported before the project was named Kaluta merges too.
        assert!(json.contains("\"kaluta_facts\": 1"), "{json}");
        let old = block_on(other.merge_facts(json.replace("\"kaluta_facts\"", "\"openagc_facts\""))).unwrap();
        assert_eq!(old.added, 1);
        assert!(block_on(other.merge_facts("{}".into())).is_err());
    }
}
