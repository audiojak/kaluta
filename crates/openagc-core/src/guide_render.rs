//! Following the writing guide (spec §14.9): the accepted guide rendered
//! as instructions for one message (its recipients' audiences, its type,
//! its language), or in general for a session that has no message yet.
//! Rules and facts are always included, with their scope written out;
//! guidelines only where their scope matches.

use crate::guide::{
    AudienceGroup, AudienceStatus, GuideEntry, GuideKind, GuideScope, GuideStatus, is_member, scope_text,
};
use crate::{Core, CoreError};

/// What drafting follows, for one message or in general.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideRendered {
    /// Instructions for the agent; empty when there is no guide yet.
    pub text: String,
    /// The accepted guide's version it was rendered from.
    pub version: i64,
    /// The recipients' confirmed audiences ("Customers").
    pub audiences: Vec<String>,
}

/// The message being drafted, as far as it is known.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Target {
    pub recipients: Vec<String>,
    /// `new`, `reply` or `forward`.
    pub message_type: Option<String>,
    /// Audiences chosen for the draft instead of the recipients' own.
    pub audiences: Option<Vec<String>>,
}

/// Whether a guideline's scope matches the message. People and audiences
/// narrow it; a scope naming what the message is not keeps it out.
pub(crate) fn applies(scope: &GuideScope, target: &Target, audiences: &[String]) -> bool {
    let people = scope.people.is_empty() || target.recipients.iter().any(|r| is_member(r, &scope.people));
    let groups =
        scope.groups.is_empty() || scope.groups.iter().any(|g| audiences.iter().any(|a| a.eq_ignore_ascii_case(g)));
    let types = scope.message_types.is_empty()
        || target.message_type.as_ref().is_some_and(|t| scope.message_types.iter().any(|s| s == t));
    people && groups && types
}

/// A narrower scope first, so it reads as winning: people, then
/// audiences, then message types, then everyone.
fn narrowness(scope: &GuideScope) -> u8 {
    if !scope.people.is_empty() {
        0
    } else if !scope.groups.is_empty() {
        1
    } else if !scope.message_types.is_empty() || !scope.languages.is_empty() {
        2
    } else {
        3
    }
}

/// The recipients' confirmed audiences, in group order.
pub(crate) fn audiences_of(recipients: &[String], groups: &[AudienceGroup]) -> Vec<String> {
    groups
        .iter()
        .filter(|g| g.status == AudienceStatus::Confirmed && recipients.iter().any(|r| is_member(r, &g.members)))
        .map(|g| g.name.clone())
        .collect()
}

/// Render the guide. `target` None: for a whole session (every guideline,
/// each with its scope); `examples`: the user's own messages to imitate.
pub(crate) fn render(
    entries: &[GuideEntry],
    groups: &[AudienceGroup],
    target: Option<&Target>,
    examples: &[String],
    version: i64,
) -> (String, Vec<String>) {
    let accepted: Vec<&GuideEntry> = entries.iter().filter(|e| e.status == GuideStatus::Accepted).collect();
    if accepted.is_empty() {
        return (String::new(), vec![]);
    }
    let audiences = match target {
        Some(t) => t.audiences.clone().unwrap_or_else(|| audiences_of(&t.recipients, groups)),
        None => vec![],
    };
    let line = |e: &GuideEntry| {
        let scope = scope_text(&e.scope);
        if scope.is_empty() { format!("- {}", e.statement) } else { format!("- {} ({scope})", e.statement) }
    };
    let by_kind = |kind: GuideKind, filter: bool| {
        let mut chosen: Vec<&&GuideEntry> = accepted
            .iter()
            .filter(|e| e.kind == kind)
            .filter(|e| !filter || target.is_none_or(|t| applies(&e.scope, t, &audiences)))
            .collect();
        chosen.sort_by_key(|e| (narrowness(&e.scope), e.category.clone(), e.id));
        chosen.into_iter().map(|e| line(e)).collect::<Vec<_>>()
    };
    let rules = by_kind(GuideKind::Rule, false);
    let facts = by_kind(GuideKind::Fact, false);
    let guidelines = by_kind(GuideKind::Guideline, true);

    let mut out =
        format!("The user's writing guide (version {version}). Follow it in every email you draft or edit for them.\n");
    if let Some(t) = target {
        let mut about = Vec::new();
        if !audiences.is_empty() {
            about.push(format!("written for {}", audiences.join(", ")));
        }
        if let Some(kind) = &t.message_type {
            about.push(format!("a {}", if kind == "new" { "new message" } else { kind }));
        }
        if !about.is_empty() {
            out.push_str(&format!("This message: {}.\n", about.join("; ")));
        }
    }
    if !rules.is_empty() {
        out.push_str(&format!(
            "\nRules (always; a rule with a scope holds where its scope says):\n{}\n",
            rules.join("\n")
        ));
    }
    if !facts.is_empty() {
        out.push_str(&format!("\nFacts about the user you may use:\n{}\n", facts.join("\n")));
    }
    if !guidelines.is_empty() {
        out.push_str(&format!(
            "\nGuidelines ({}; follow them unless the message calls for something else):\n{}\n",
            if target.is_some() { "for this message" } else { "each where its scope says" },
            guidelines.join("\n")
        ));
    }
    out.push_str(
        "\nWhen entries disagree: a rule beats a guideline, and an entry for a person beats one for their \
         audience, which beats one for everyone. This guide never lets you do more than Settings › Permissions \
         allows.\n",
    );
    if !examples.is_empty() {
        out.push_str("\nExamples of how the user writes (imitate the style, not the content):\n");
        for e in examples {
            out.push_str(&format!("<example>\n{}\n</example>\n", crate::guide_ai::fenced(e)));
        }
    }
    (out, audiences)
}

impl Core {
    /// The guide rendered for a message or (with no target) a session.
    pub(crate) async fn render_guide(&self, target: Option<Target>) -> Result<GuideRendered, CoreError> {
        let entries = self.list_guide_entries(vec![GuideStatus::Accepted]).await?;
        if entries.is_empty() {
            return Ok(GuideRendered { text: String::new(), version: self.guide_version().await?, audiences: vec![] });
        }
        let groups = self.list_audience_groups().await?;
        let version = self.guide_version().await?;
        // Up to three of the user's own messages of the same type.
        let mut examples = Vec::new();
        if let Some(kind) = target.as_ref().and_then(|t| t.message_type.clone()) {
            let ids: Vec<String> = self
                .guide_examples()
                .await?
                .into_iter()
                .filter(|p| p[1] == kind)
                .map(|p| p[0].clone())
                .take(3)
                .collect();
            if !ids.is_empty() {
                examples = self.prepare_batch(ids).await?.into_iter().map(|p| p.text).collect();
            }
        }
        let (text, audiences) = render(&entries, &groups, target.as_ref(), &examples, version);
        Ok(GuideRendered { text, version, audiences })
    }
}

/// A check an AI draft failed (spec §14.9).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideCheckFailure {
    pub entry_id: i64,
    pub statement: String,
    /// What is wrong, in a line: "Uses “circle back”, which your rules ban".
    pub message: String,
}

/// Whether `phrase` occurs in `text` as words (any case): "circle back"
/// matches "Let's circle back." but not "encircle backs".
fn contains_phrase(text: &str, phrase: &str) -> bool {
    let words = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_alphanumeric() && c != '\'')
            .filter(|w| !w.is_empty())
            .map(|w| w.to_lowercase())
            .collect()
    };
    let (t, p) = (words(text), words(phrase));
    !p.is_empty() && t.windows(p.len()).any(|w| w == p.as_slice())
}

/// Run the checks of the entries that apply to this message on an AI
/// draft's own text (never on text the user typed; spec §14.9).
pub(crate) fn check(
    entries: &[GuideEntry],
    groups: &[AudienceGroup],
    target: &Target,
    text: &str,
) -> Vec<GuideCheckFailure> {
    let audiences = target.audiences.clone().unwrap_or_else(|| audiences_of(&target.recipients, groups));
    let words = text.split_whitespace().count();
    entries
        .iter()
        .filter(|e| e.status == GuideStatus::Accepted && applies(&e.scope, target, &audiences))
        // Which language a draft is in is not known here: checks scoped to
        // a language are left to the agent, which sees them in the guide.
        .filter(|e| e.scope.languages.is_empty())
        .filter_map(|e| {
            let c = e.check.as_ref()?;
            let message = match c.kind {
                crate::guide::GuideCheckKind::BannedPhrase if contains_phrase(text, &c.value) => {
                    format!("Uses “{}”, which your rules ban", c.value)
                }
                crate::guide::GuideCheckKind::RequiredPhrase if !contains_phrase(text, &c.value) => {
                    format!("Leaves out “{}”, which your rules require", c.value)
                }
                crate::guide::GuideCheckKind::MaxWords => {
                    let limit: usize = c.value.parse().ok()?;
                    if words <= limit {
                        return None;
                    }
                    format!("Is {words} words; your guide says at most {limit}")
                }
                _ => return None,
            };
            Some(GuideCheckFailure { entry_id: e.id, statement: e.statement.clone(), message })
        })
        .collect()
}

impl Core {
    /// Check an AI draft's own text against the guide for its message.
    pub(crate) async fn check_against_guide(
        &self,
        target: Target,
        text: &str,
    ) -> Result<Vec<GuideCheckFailure>, CoreError> {
        let entries = self.list_guide_entries(vec![GuideStatus::Accepted]).await?;
        if entries.iter().all(|e| e.check.is_none()) {
            return Ok(vec![]);
        }
        let groups = self.list_audience_groups().await?;
        Ok(check(&entries, &groups, &target, text))
    }

    /// Record the guide version a draft was written under.
    pub(crate) async fn record_draft_guide(&self, draft_id: i64, version: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        crate::runtime::run(async move {
            Ok(db.write(move |tx| mail_store::guide::set_draft_guide_version(tx, draft_id, version)).await?)
        })
        .await
    }
}

#[uniffi::export]
impl Core {
    /// The guide version an AI draft was written under, if it was.
    pub async fn draft_guide_version(&self, draft_id: i64) -> Result<Option<i64>, CoreError> {
        let db = self.db()?;
        crate::runtime::run(
            async move { Ok(db.read(move |c| mail_store::guide::draft_guide_version(c, draft_id)).await?) },
        )
        .await
    }

    /// Writing help wrote this draft's body under guide `version`.
    pub async fn set_draft_guide_version(&self, draft_id: i64, version: i64) -> Result<(), CoreError> {
        self.record_draft_guide(draft_id, version).await
    }

    /// Check a draft an AI wrote (its own text, without the quoted
    /// original) against the guide for its recipients and type.
    pub async fn check_guide_draft(
        &self,
        text: String,
        recipients: Vec<String>,
        message_type: Option<String>,
        audiences: Option<Vec<String>>,
    ) -> Result<Vec<GuideCheckFailure>, CoreError> {
        self.check_against_guide(Target { recipients, message_type, audiences }, &text).await
    }

    /// The guide for a message being drafted (spec §14.9): its recipients
    /// (addresses), its type (`new`, `reply`, `forward`), and optionally
    /// the audiences to write for instead of the recipients' own (drafting
    /// with an audience).
    pub async fn guide_for_message(
        &self,
        recipients: Vec<String>,
        message_type: Option<String>,
        audiences: Option<Vec<String>>,
    ) -> Result<GuideRendered, CoreError> {
        self.render_guide(Some(Target { recipients, message_type, audiences })).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guide::GuideSource;

    fn entry(id: i64, kind: GuideKind, statement: &str, scope: GuideScope) -> GuideEntry {
        GuideEntry {
            id,
            category: "A1".into(),
            kind,
            statement: statement.into(),
            scope,
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

    fn groups() -> Vec<AudienceGroup> {
        vec![AudienceGroup {
            id: 1,
            name: "Customers".into(),
            status: AudienceStatus::Confirmed,
            description: String::new(),
            members: vec!["@acme.com".into()],
        }]
    }

    fn guide() -> Vec<GuideEntry> {
        let customers = GuideScope { groups: vec!["Customers".into()], ..Default::default() };
        let replies = GuideScope { message_types: vec!["reply".into()], ..Default::default() };
        let ann = GuideScope { people: vec!["ann@acme.com".into()], ..Default::default() };
        vec![
            entry(1, GuideKind::Rule, "Never promise delivery dates", GuideScope::default()),
            entry(2, GuideKind::Guideline, "Be warm and brief", GuideScope::default()),
            entry(3, GuideKind::Guideline, "Be formal", customers.clone()),
            entry(4, GuideKind::Guideline, "Answer in the first line", replies),
            entry(5, GuideKind::Guideline, "Call her Annie", ann),
            entry(6, GuideKind::Fact, "My calendar link: cal.com/john", GuideScope::default()),
            entry(7, GuideKind::Rule, "Include the support address", customers),
        ]
    }

    #[test]
    fn language_scoped_entries_are_left_to_the_agent() {
        let french = GuideScope { languages: vec!["French".into()], ..Default::default() };
        let mut e = entry(1, GuideKind::Rule, "Never write 'Salut'", french);
        e.check =
            Some(crate::guide::GuideCheck { kind: crate::guide::GuideCheckKind::BannedPhrase, value: "Salut".into() });
        let t = Target { recipients: vec![], message_type: Some("new".into()), audiences: None };
        let (text, _) = render(std::slice::from_ref(&e), &[], Some(&t), &[], 1);
        assert!(text.contains("Never write 'Salut' (when writing in French)"), "{text}");
        assert!(check(&[e], &[], &t, "Salut Ann").is_empty(), "the draft's language is not known here");
    }

    #[test]
    fn a_message_gets_rules_facts_and_the_guidelines_that_fit_it() {
        let t = Target { recipients: vec!["Ann@acme.com".into()], message_type: Some("reply".into()), audiences: None };
        let (text, audiences) = render(&guide(), &groups(), Some(&t), &[], 7);
        assert_eq!(audiences, ["Customers"]);
        assert!(text.contains("version 7") && text.contains("written for Customers; a reply"));
        for s in [
            "Never promise delivery dates",
            "cal.com/john",
            "Be formal",
            "Answer in the first line",
            "Call her Annie",
            "Be warm and brief",
        ] {
            assert!(text.contains(s), "{s} missing");
        }
        assert!(text.contains("Include the support address (for Customers)"), "a scoped rule shows its scope");
        let annie = text.find("Call her Annie").unwrap();
        let formal = text.find("Be formal").unwrap();
        let warm = text.find("Be warm and brief").unwrap();
        assert!(annie < formal && formal < warm, "narrow scopes first");

        let colleague =
            Target { recipients: vec!["bob@actual.ai".into()], message_type: Some("new".into()), audiences: None };
        let (text, audiences) = render(&guide(), &groups(), Some(&colleague), &["Hi Bob,\nSure.".into()], 7);
        assert!(audiences.is_empty());
        assert!(!text.contains("Be formal") && !text.contains("Answer in the first line") && !text.contains("Annie"));
        assert!(text.contains("Never promise delivery dates"), "rules are always there");
        assert!(text.contains("<example>\nHi Bob,\nSure.\n</example>"));

        // Drafting for an audience chosen by the user.
        let chosen = Target { audiences: Some(vec!["Customers".into()]), ..colleague };
        assert!(render(&guide(), &groups(), Some(&chosen), &[], 7).0.contains("Be formal"));
    }

    #[test]
    fn checks_catch_banned_and_missing_phrases_and_length_where_they_apply() {
        use crate::guide::{GuideCheck, GuideCheckKind};
        let with = |id, kind, value: &str, scope| GuideEntry {
            check: Some(GuideCheck { kind, value: value.into() }),
            ..entry(id, GuideKind::Rule, "rule", scope)
        };
        let customers = GuideScope { groups: vec!["Customers".into()], ..Default::default() };
        let entries = vec![
            with(1, GuideCheckKind::BannedPhrase, "circle back", GuideScope::default()),
            with(2, GuideCheckKind::RequiredPhrase, "support@acme.com", customers),
            with(3, GuideCheckKind::MaxWords, "8", GuideScope::default()),
        ];
        let to_customer =
            Target { recipients: vec!["ann@acme.com".into()], message_type: Some("reply".into()), audiences: None };
        let failures = check(&entries, &groups(), &to_customer, "Let's Circle back next week about the plan, Ann.");
        let ids: Vec<i64> = failures.iter().map(|f| f.entry_id).collect();
        assert_eq!(ids, [1, 2, 3]);
        assert_eq!(failures[0].message, "Uses “circle back”, which your rules ban");
        assert!(failures[2].message.starts_with("Is 9 words"));
        let to_colleague = Target { recipients: vec!["bob@actual.ai".into()], ..to_customer.clone() };
        assert!(check(&entries, &groups(), &to_colleague, "Sounds good.").is_empty(), "scoped checks stay in scope");
        assert!(check(&entries, &groups(), &to_colleague, "The encircle backstory").is_empty(), "whole words only");
    }

    #[test]
    fn a_session_gets_every_entry_with_its_scope_and_no_guide_means_nothing() {
        let (text, _) = render(&guide(), &groups(), None, &[], 3);
        assert!(text.contains("Be formal (for Customers)") && text.contains("Answer in the first line (in reply)"));
        assert!(render(&[], &groups(), None, &[], 0).0.is_empty());
        let mut proposed = guide();
        proposed.iter_mut().for_each(|e| e.status = GuideStatus::Proposed);
        assert!(render(&proposed, &groups(), None, &[], 0).0.is_empty(), "only accepted entries");
    }

    #[test]
    fn sessions_and_draft_tools_carry_the_guide_and_drafts_record_its_version() {
        use futures::executor::block_on;

        use crate::guide::{GuideEdit, GuideEntryFields};

        let s = crate::guide::tests::demo("render-live");
        let core = &s.1;
        core.debug_use_fake_agents();
        block_on(core.apply_guide_edits(
            vec![GuideEdit::Add {
                fields: GuideEntryFields {
                    category: "B6".into(),
                    kind: GuideKind::Rule,
                    statement: "Sign off with 'John'".into(),
                    scope: GuideScope::default(),
                    check: Some(crate::guide::GuideCheck {
                        kind: crate::guide::GuideCheckKind::BannedPhrase,
                        value: "circle back".into(),
                    }),
                },
                status: GuideStatus::Accepted,
                source: GuideSource::You,
                origin: None,
            }],
            "test".into(),
        ))
        .unwrap();
        let session = block_on(core.clone().start_agent_session("claude-code".into(), None, None)).unwrap();
        let prompt_file =
            std::path::PathBuf::from(&core.config.data_dir).join("agents").join(&session).join("system-prompt.md");
        let prompt = std::fs::read_to_string(prompt_file).expect("a per-session prompt with the guide");
        assert!(prompt.contains("## Writing guide") && prompt.contains("Sign off with 'John'"));

        let out = crate::runtime::runtime().block_on(crate::agents::tools_call_for_tests(
            core,
            &session,
            permissions::Tool::CreateDraft,
            serde_json::json!({"to": ["ann@example.com"], "subject": "Plan", "body_markdown": "Hi Ann, let's circle back."}),
        ));
        let agent_mcp::Outcome::Ok { structured: Some(value), .. } = out else { panic!("draft not created: {out:?}") };
        assert!(value["writing_guide"].as_str().unwrap().contains("Sign off with 'John'"));
        assert_eq!(value["guide_check"][0], "Uses “circle back”, which your rules ban");
        let draft = value["draft_id"].as_i64().unwrap();
        assert_eq!(block_on(core.draft_guide_version(draft)).unwrap(), Some(1));
        let _ = block_on(core.clone().close_agent_session(session));
    }
}
