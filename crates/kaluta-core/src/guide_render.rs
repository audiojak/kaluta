//! Following the writing guide (spec §14.9): the accepted guide rendered
//! as instructions for one message (its recipients' audiences, its type,
//! its language), or in general for a session that has no message yet.
//! Rules and facts are always included, with their scope written out;
//! guidelines only where their scope matches.
//!
//! The rendering and the checks themselves live in `writing-guide`, shared
//! with the rules server (spec §10.6); this module turns the core's
//! records into its types.

use writing_guide::AudienceGroups;
pub use writing_guide::Target;
pub(crate) use writing_guide::with_facts;

use crate::guide::{
    AudienceGroup, AudienceStatus, GuideCheck, GuideCheckKind, GuideEntry, GuideKind, GuideScope, GuideStatus,
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

impl From<&GuideScope> for writing_guide::Scope {
    fn from(s: &GuideScope) -> Self {
        Self {
            groups: s.groups.clone(),
            people: s.people.clone(),
            message_types: s.message_types.clone(),
            languages: s.languages.clone(),
        }
    }
}

impl From<&GuideCheck> for writing_guide::Check {
    fn from(c: &GuideCheck) -> Self {
        let kind = match c.kind {
            GuideCheckKind::BannedPhrase => writing_guide::CheckKind::BannedPhrase,
            GuideCheckKind::RequiredPhrase => writing_guide::CheckKind::RequiredPhrase,
            GuideCheckKind::MaxWords => writing_guide::CheckKind::MaxWords,
        };
        Self { kind, value: c.value.clone() }
    }
}

impl From<&GuideEntry> for writing_guide::Entry {
    fn from(e: &GuideEntry) -> Self {
        let kind = match e.kind {
            GuideKind::Rule => writing_guide::Kind::Rule,
            GuideKind::Guideline => writing_guide::Kind::Guideline,
            GuideKind::Fact => writing_guide::Kind::Fact,
        };
        Self {
            id: e.id,
            category: e.category.clone(),
            kind,
            statement: e.statement.clone(),
            scope: (&e.scope).into(),
            check: e.check.as_ref().map(Into::into),
        }
    }
}

/// The accepted entries, as the guide's renderer and checks take them.
pub(crate) fn accepted(entries: &[GuideEntry]) -> Vec<writing_guide::Entry> {
    entries.iter().filter(|e| e.status == GuideStatus::Accepted).map(Into::into).collect()
}

/// The confirmed audience groups, their members plain.
pub(crate) fn confirmed(groups: &[AudienceGroup]) -> AudienceGroups {
    AudienceGroups::plain(
        groups
            .iter()
            .filter(|g| g.status == AudienceStatus::Confirmed)
            .map(|g| writing_guide::AudienceGroup { name: g.name.clone(), members: g.members.clone() })
            .collect(),
    )
}

/// Whether a guideline's scope matches the message. People and audiences
/// narrow it; a scope naming what the message is not keeps it out.
pub(crate) fn applies(scope: &GuideScope, target: &Target, audiences: &[String]) -> bool {
    writing_guide::applies(&scope.into(), target, audiences)
}

/// The recipients' confirmed audiences, in group order.
pub(crate) fn audiences_of(recipients: &[String], groups: &[AudienceGroup]) -> Vec<String> {
    confirmed(groups).audiences_of(recipients)
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
    writing_guide::render(&accepted(entries), &confirmed(groups), target, examples, version)
}

impl Core {
    /// The guide rendered for a message or (with no target) a session.
    pub(crate) async fn render_guide(&self, target: Option<Target>) -> Result<GuideRendered, CoreError> {
        let entries = self.list_guide_entries(vec![GuideStatus::Accepted]).await?;
        // Facts live in their own store (spec §14.11) and go with the guide.
        let facts = self.fact_lines().await?;
        if entries.is_empty() {
            let version = self.guide_version().await?;
            return Ok(GuideRendered { text: with_facts(String::new(), &facts), version, audiences: vec![] });
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
        Ok(GuideRendered { text: with_facts(text, &facts), version, audiences })
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

/// Run the checks of the entries that apply to this message on an AI
/// draft's own text (never on text the user typed; spec §14.9).
pub(crate) fn check(
    entries: &[GuideEntry],
    groups: &[AudienceGroup],
    target: &Target,
    text: &str,
) -> Vec<GuideCheckFailure> {
    writing_guide::check(&accepted(entries), &confirmed(groups), target, text)
        .into_iter()
        .map(|f| GuideCheckFailure { entry_id: f.entry_id, statement: f.statement, message: f.message })
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
