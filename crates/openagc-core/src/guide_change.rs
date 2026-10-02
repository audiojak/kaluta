//! Changing the writing guide by prompt (spec §14.9): the user's request
//! goes to their agent in a hidden, read-only session with the guide; the
//! answer is a set of questions, each one decision in plain words with the
//! edits it would make. Nothing changes until the user answers; the app
//! applies the ones they accept as one change (`apply_guide_edits`).

use std::sync::Arc;

use serde_json::Value;

use crate::guide::{
    AudienceGroup, CATEGORIES, GuideCheck, GuideCheckKind, GuideEdit, GuideEntry, GuideEntryFields, GuideKind,
    GuideScope, GuideSource, GuideStatus, category, clean_text,
};
use crate::{Core, CoreError, ErrorKind};

/// The first line of every change prompt: how the fake agent knows one.
pub const CHANGE_MARKER: &str = "OpenAGC writing guide change";

/// One decision for the user: what changes, before and after, and the
/// edits that make it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideChangeQuestion {
    pub question: String,
    pub before: String,
    pub after: String,
    pub edits: Vec<GuideEdit>,
}

pub fn prompt(request: &str, entries: &[GuideEntry], groups: &[AudienceGroup]) -> String {
    let categories: String = CATEGORIES.iter().map(|c| format!("{} {}", c.id, c.name)).collect::<Vec<_>>().join("; ");
    let guide: String = entries
        .iter()
        .filter(|e| e.status == GuideStatus::Accepted)
        .map(|e| {
            let scope = serde_json::to_string(&e.scope).unwrap_or_default();
            format!(
                "#{} [{} {}] {} scope={scope}",
                e.id,
                e.category,
                e.kind.as_str(),
                crate::guide_ai::fenced(&e.statement)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let audiences: String = groups.iter().map(|g| crate::guide_ai::fenced(&g.name)).collect::<Vec<_>>().join(", ");
    format!(
        "{CHANGE_MARKER}\n\n\
         The user wants to change their writing guide (the rules and guidelines an assistant follows when it \
         drafts email for them). Work out the changes their request needs and ask them, one decision at a \
         time, before anything changes.\n\n\
         Their request:\n<<<\n{}\n>>>\n\n\
         The guide now:\n{}\n\n\
         Categories: {categories}\n\
         Audience groups: {}\n\n\
         Answer with only a JSON object:\n\
         {{\"questions\": [{{\"question\": \"one decision in plain words\", \"before\": \"what the guide says now \
         (or 'nothing')\", \"after\": \"what it would say\", \"edits\": [\
         {{\"op\": \"add\", \"category\": \"C1\", \"kind\": \"rule\" or \"guideline\" or \"fact\", \"statement\": \
         \"...\", \"scope\": {{\"groups\": [], \"people\": [], \"message_types\": [], \"languages\": []}}, \
         \"check\": null}}, \
         {{\"op\": \"edit\", \"id\": 12, \"statement\": \"...\"}}, \
         {{\"op\": \"rescope\", \"id\": 12, \"scope\": {{...}}}}, \
         {{\"op\": \"remove\", \"id\": 12}}]}}]}}\n\n\
         - Group edits that belong together into one question; keep questions few and clear.\n\
         - Change only what the request asks for.\n\
         - Do not use tools, and do not create, change, send or delete any mail.",
        request.trim(),
        if guide.is_empty() { "(empty)".to_owned() } else { guide },
        if audiences.is_empty() { "(none)".to_owned() } else { audiences },
    )
}

fn scope_of(v: Option<&Value>) -> Option<GuideScope> {
    let v = v?;
    let list = |k: &str| -> Vec<String> {
        v.get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default()
    };
    Some(GuideScope {
        groups: list("groups"),
        people: list("people"),
        message_types: list("message_types"),
        languages: list("languages"),
    })
}

fn check_of(v: Option<&Value>) -> Option<GuideCheck> {
    let v = v?;
    let kind = match v.get("kind")?.as_str()? {
        "banned_phrase" => GuideCheckKind::BannedPhrase,
        "required_phrase" => GuideCheckKind::RequiredPhrase,
        "max_words" => GuideCheckKind::MaxWords,
        _ => return None,
    };
    let value = match v.get("value")? {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    Some(GuideCheck { kind, value })
}

/// Read the agent's answer: questions with their edits. Edits naming an
/// entry that is not in the guide, or a category that does not exist, are
/// dropped; a question left with no edit is dropped.
pub fn parse(text: &str, entries: &[GuideEntry]) -> Option<Vec<GuideChangeQuestion>> {
    let value: Value = text.match_indices('{').find_map(|(i, _)| {
        serde_json::Deserializer::from_str(&text[i..])
            .into_iter::<Value>()
            .next()
            .and_then(Result::ok)
            .filter(|v| v.get("questions").is_some())
    })?;
    let find = |id: i64| entries.iter().find(|e| e.id == id && e.status == GuideStatus::Accepted);
    let mut out = Vec::new();
    for q in value.get("questions").and_then(Value::as_array).into_iter().flatten() {
        let mut edits = Vec::new();
        for e in q.get("edits").and_then(Value::as_array).into_iter().flatten() {
            let op = e.get("op").and_then(Value::as_str).unwrap_or("");
            let id = e.get("id").and_then(Value::as_i64);
            let edit = match (op, id.and_then(find)) {
                ("add", _) => {
                    let Some(cat) = e.get("category").and_then(Value::as_str).and_then(category) else { continue };
                    let statement = clean_text(e.get("statement").and_then(Value::as_str).unwrap_or(""));
                    if statement.is_empty() {
                        continue;
                    }
                    GuideEdit::Add {
                        fields: GuideEntryFields {
                            category: cat.id.into(),
                            kind: e
                                .get("kind")
                                .and_then(Value::as_str)
                                .and_then(GuideKind::parse)
                                .unwrap_or(GuideKind::Guideline),
                            statement,
                            scope: scope_of(e.get("scope")).unwrap_or_default(),
                            check: check_of(e.get("check")),
                        },
                        status: GuideStatus::Accepted,
                        source: GuideSource::You,
                        origin: None,
                    }
                }
                ("edit", Some(old)) | ("rescope", Some(old)) => {
                    let statement =
                        e.get("statement").and_then(Value::as_str).map(clean_text).filter(|s| !s.is_empty());
                    GuideEdit::Update {
                        id: old.id,
                        fields: GuideEntryFields {
                            category: old.category.clone(),
                            kind: e.get("kind").and_then(Value::as_str).and_then(GuideKind::parse).unwrap_or(old.kind),
                            statement: statement.unwrap_or_else(|| old.statement.clone()),
                            scope: scope_of(e.get("scope")).unwrap_or_else(|| old.scope.clone()),
                            check: if e.get("check").is_some() { check_of(e.get("check")) } else { old.check.clone() },
                        },
                    }
                }
                ("remove", Some(old)) => GuideEdit::Delete { id: old.id },
                _ => continue,
            };
            edits.push(edit);
        }
        if edits.is_empty() {
            continue;
        }
        let text = |k: &str| clean_text(q.get(k).and_then(Value::as_str).unwrap_or(""));
        out.push(GuideChangeQuestion {
            question: text("question"),
            before: text("before"),
            after: text("after"),
            edits,
        });
    }
    Some(out)
}

#[uniffi::export]
impl Core {
    /// Ask `agent` how to change the guide for the user's request; returns
    /// the questions to put to the user. Nothing changes here.
    pub async fn propose_guide_change(
        self: Arc<Self>,
        request: String,
        agent: String,
    ) -> Result<Vec<GuideChangeQuestion>, CoreError> {
        let request = request.trim().to_owned();
        if request.is_empty() {
            return Err(CoreError::new(ErrorKind::InvalidInput, "say what to change"));
        }
        if let Some(why) = self.agent_not_ready(&agent).await? {
            return Err(CoreError::new(ErrorKind::Agent, why));
        }
        let entries = self.list_guide_entries(vec![GuideStatus::Accepted]).await?;
        let groups = self.list_audience_groups().await?;
        let answer = self.ask_agent_hidden(&agent, prompt(&request, &entries, &groups)).await?;
        let questions = parse(&answer, &entries)
            .ok_or_else(|| CoreError::new(ErrorKind::Agent, "the agent's answer was not the changes asked for"))?;
        if questions.is_empty() {
            return Err(CoreError::new(ErrorKind::NotFound, "the agent found nothing to change for that request"));
        }
        Ok(questions)
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;
    use crate::guide::tests::demo;

    #[test]
    fn a_request_becomes_questions_and_nothing_changes_until_they_are_applied() {
        let s = demo("change");
        let core = &s.1;
        core.debug_use_fake_agents();
        let made = block_on(core.apply_guide_edits(
            vec![GuideEdit::Add {
                fields: GuideEntryFields {
                    category: "C1".into(),
                    kind: GuideKind::Rule,
                    statement: "Use US spelling".into(),
                    scope: GuideScope::default(),
                    check: None,
                },
                status: GuideStatus::Accepted,
                source: GuideSource::You,
                origin: None,
            }],
            "x".into(),
        ))
        .unwrap();
        let questions = block_on(
            core.clone().propose_guide_change("Use British spelling from now on".into(), "claude-code".into()),
        )
        .unwrap();
        assert_eq!(questions.len(), 2, "the fake agent adds the request and removes the entry it contradicts");
        assert!(matches!(questions[0].edits[0], GuideEdit::Add { .. }));
        assert_eq!(questions[1].edits, [GuideEdit::Delete { id: made.entries[0].id }]);
        assert_eq!(
            block_on(core.list_guide_entries(vec![GuideStatus::Accepted])).unwrap().len(),
            1,
            "nothing changed yet"
        );

        let all: Vec<GuideEdit> = questions.into_iter().flat_map(|q| q.edits).collect();
        let change = block_on(core.apply_guide_edits(all, "change by prompt".into())).unwrap();
        let now = block_on(core.list_guide_entries(vec![GuideStatus::Accepted])).unwrap();
        assert_eq!(now.iter().map(|e| e.statement.as_str()).collect::<Vec<_>>(), ["Use British spelling from now on"]);
        block_on(core.undo_guide_change(change.change_id)).unwrap();
        assert_eq!(
            block_on(core.list_guide_entries(vec![GuideStatus::Accepted])).unwrap()[0].statement,
            "Use US spelling"
        );

        let codex = block_on(core.clone().propose_guide_change("x".into(), "codex".into())).unwrap_err();
        assert_eq!(codex.kind(), ErrorKind::Agent);
        assert!(block_on(core.clone().propose_guide_change("  ".into(), "claude-code".into())).is_err());
    }

    #[test]
    fn edits_to_unknown_entries_or_categories_are_dropped() {
        let answer = r#"{"questions": [
            {"question": "Q1", "before": "b", "after": "a", "edits": [{"op": "remove", "id": 99}, {"op": "add", "category": "Z9", "statement": "x"}]},
            {"question": "Q2", "edits": [{"op": "add", "category": "c4", "kind": "rule", "statement": " Use contractions ", "check": {"kind": "banned_phrase", "value": "I will"}}]}
        ]}"#;
        let qs = parse(answer, &[]).unwrap();
        assert_eq!(qs.len(), 1);
        let GuideEdit::Add { fields, .. } = &qs[0].edits[0] else { panic!() };
        assert_eq!((fields.category.as_str(), fields.statement.as_str()), ("C4", "Use contractions"));
        assert_eq!(fields.check.as_ref().unwrap().value, "I will");
        assert!(parse("no", &[]).is_none());
    }
}
