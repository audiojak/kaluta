//! Recording AI compositions (spec §14.10, ADR 0013): writing help, the
//! agent's draft tools and routines each leave a record of the text the AI
//! wrote, for the daily review to compare with what the user sent.

use mail_store::compositions::{self, Kind, NewComposition, Recipients, Source};
use mail_store::drafts;

use crate::{Core, CoreError, runtime};

/// A recorded AI composition, as Swift sees it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct AiCompositionInfo {
    pub id: i64,
    pub created_at: i64,
    pub source: String,
    pub agent: Option<String>,
    pub kind: String,
    pub draft_id: Option<i64>,
    pub thread_id: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub instruction: String,
    pub ai_text: Option<String>,
    pub guide_version: Option<i64>,
    pub audiences: Vec<String>,
    pub rfc822_message_id: Option<String>,
    pub status: String,
}

impl From<compositions::Composition> for AiCompositionInfo {
    fn from(c: compositions::Composition) -> Self {
        Self {
            id: c.id,
            created_at: c.created_at,
            source: c.source.as_str().into(),
            agent: c.agent,
            kind: c.kind.as_str().into(),
            draft_id: c.draft_id,
            thread_id: c.thread_id,
            to: c.recipients.to,
            cc: c.recipients.cc,
            subject: c.subject,
            instruction: c.instruction,
            ai_text: c.ai_text,
            guide_version: c.guide_version,
            audiences: c.audiences,
            rfc822_message_id: c.rfc822_message_id,
            status: c.status.as_str().into(),
        }
    }
}

/// What an AI wrote, before the draft's addressing is read.
pub(crate) struct AiText {
    pub source: Source,
    pub agent: Option<String>,
    pub instruction: String,
    pub text: String,
    pub html: Option<String>,
    pub guide_version: Option<i64>,
    pub audiences: Vec<String>,
}

impl Core {
    /// Record what an AI wrote into draft `draft_id`. Does nothing until the
    /// account has finished a learning run, and never on an archive.
    /// Returns the record's id when one was written.
    pub(crate) async fn record_composition(&self, draft_id: i64, ai: AiText) -> Result<Option<i64>, CoreError> {
        if self.effective_account_id().is_some_and(|id| self.is_archive(&id)) || ai.text.trim().is_empty() {
            return Ok(None);
        }
        let db = self.db()?;
        runtime::run(async move {
            let now = mail_sync::now_millis();
            Ok(db
                .write(move |tx| {
                    if !compositions::recording(tx)? {
                        return Ok(None);
                    }
                    let Some(d) = drafts::get(tx, draft_id)? else { return Ok(None) };
                    let record = NewComposition {
                        source: ai.source,
                        agent: ai.agent,
                        kind: Kind::of(&d.subject, d.in_reply_to.is_some()),
                        draft_id,
                        thread_id: d.thread_id,
                        in_reply_to: d.in_reply_to,
                        recipients: Recipients::new(
                            d.to.iter().map(|a| a.email.as_str()),
                            d.cc.iter().map(|a| a.email.as_str()),
                        ),
                        subject: d.subject,
                        instruction: ai.instruction,
                        ai_text: ai.text,
                        ai_html: ai.html,
                        guide_version: ai.guide_version,
                        audiences: ai.audiences,
                    };
                    compositions::record(tx, &record, now).map(Some)
                })
                .await?)
        })
        .await
    }

    /// An agent changed a draft's recipients or subject but not its text.
    pub(crate) async fn composition_readdressed(&self, draft_id: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            let now = mail_sync::now_millis();
            Ok(db
                .write(move |tx| {
                    let Some(d) = drafts::get(tx, draft_id)? else { return Ok(()) };
                    let to =
                        Recipients::new(d.to.iter().map(|a| a.email.as_str()), d.cc.iter().map(|a| a.email.as_str()));
                    compositions::update_addressing(tx, draft_id, &to, &d.subject, now)
                })
                .await?)
        })
        .await
    }
}

#[uniffi::export]
impl Core {
    /// Writing help put `ai_text` into draft `draft_id` (spec §14.10).
    /// Called after the draft is saved; a later text for the same draft
    /// replaces it. Nothing is kept before the account's first finished
    /// learning run.
    pub async fn record_writing_help(
        &self,
        draft_id: i64,
        agent: String,
        instruction: String,
        ai_text: String,
        guide_version: Option<i64>,
        audiences: Vec<String>,
    ) -> Result<(), CoreError> {
        let ai = AiText {
            source: Source::WritingHelp,
            agent: Some(agent),
            instruction,
            text: ai_text,
            html: None,
            guide_version,
            audiences,
        };
        self.record_composition(draft_id, ai).await.map(|_| ())
    }

    /// Recorded AI compositions, newest first.
    pub async fn ai_compositions(&self, limit: u32) -> Result<Vec<AiCompositionInfo>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db.read(move |c| compositions::recent(c, limit)).await?.into_iter().map(Into::into).collect())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::executor::block_on;
    use permissions::{Scope, Tool};
    use serde_json::json;

    use crate::Core;

    fn learned(core: &Core) {
        let db = core.db().unwrap();
        block_on(db.write(|tx| {
            let run = mail_store::guide::create_run(tx, "learn", None, Some("claude-code"), &[], 20, 1)?;
            mail_store::guide::set_run_status(tx, run, "done", None, 2)
        }))
        .unwrap();
    }

    fn create(core: &Arc<Core>, session: &str, body: &str) -> i64 {
        let out = crate::runtime::runtime().block_on(crate::agents::tools_call_for_tests(
            core,
            session,
            Tool::CreateDraft,
            json!({"to": ["Ann <Ann@Example.com>"], "cc": ["bo@example.com"], "subject": "Plan", "body_markdown": body}),
        ));
        let agent_mcp::Outcome::Ok { structured: Some(value), .. } = out else { panic!("{out:?}") };
        value["draft_id"].as_i64().unwrap()
    }

    #[test]
    fn nothing_is_recorded_before_the_first_learning_run() {
        let s = crate::guide::tests::demo("compositions-gate");
        let core = &s.1;
        core.agents.register("s1", Scope::Mailbox, None);
        let draft = create(core, "s1", "Hi Ann");
        block_on(core.record_writing_help(draft, "claude-code".into(), "say hi".into(), "Hi".into(), None, vec![]))
            .unwrap();
        assert!(block_on(core.ai_compositions(10)).unwrap().is_empty());
    }

    #[test]
    fn an_agent_draft_is_recorded_kept_through_edits_and_linked_when_sent() {
        let s = crate::guide::tests::demo("compositions-agent");
        let core = &s.1;
        learned(core);
        core.agents.register("s1", Scope::Mailbox, None);
        core.agents.with_session("s1", |s| {
            s.agent = Some("codex".into());
            s.last_prompt = "Draft a plan for Ann".into();
        });
        let draft = create(core, "s1", "Hi **Ann**, here is the plan.");
        let records = block_on(core.ai_compositions(10)).unwrap();
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!((r.source.as_str(), r.agent.as_deref(), r.kind.as_str()), ("agent", Some("codex"), "new"));
        assert_eq!(r.ai_text.as_deref(), Some("Hi Ann, here is the plan."));
        assert_eq!(
            (r.to.clone(), r.cc.clone()),
            (vec!["ann@example.com".to_owned()], vec!["bo@example.com".to_owned()])
        );
        assert_eq!(r.instruction, "Draft a plan for Ann");
        assert_eq!(r.draft_id, Some(draft));

        // The user's own edits do not touch the AI's text.
        let mut d = block_on(core.get_draft(draft)).unwrap().unwrap();
        d.body_html = "<p>Hi Ann, the plan is below. John</p>".into();
        block_on(core.save_draft(d)).unwrap();
        let r = &block_on(core.ai_compositions(10)).unwrap()[0];
        assert_eq!(r.ai_text.as_deref(), Some("Hi Ann, here is the plan."));
        // A rewrite by the agent replaces it.
        let out = crate::runtime::runtime().block_on(crate::agents::tools_call_for_tests(
            core,
            "s1",
            Tool::UpdateDraft,
            json!({"draft_id": draft, "body_markdown": "Hi Ann, the plan."}),
        ));
        assert!(matches!(out, agent_mcp::Outcome::Ok { .. }), "{out:?}");
        let r = &block_on(core.ai_compositions(10)).unwrap()[0];
        assert_eq!(r.ai_text.as_deref(), Some("Hi Ann, the plan."));

        // Sent (the demo account sends at once): the record keeps the
        // Message-ID and lets go of the draft.
        assert!(!block_on(core.send_draft(draft)).unwrap());
        let r = &block_on(core.ai_compositions(10)).unwrap()[0];
        assert_eq!((r.status.as_str(), r.draft_id), ("waiting", None));
        let id = r.rfc822_message_id.clone().expect("the Message-ID it was sent with");
        let sent = block_on(core.list_threads("SENT".into(), None, 50)).unwrap().rows;
        assert!(sent.iter().any(|t| t.subject == "Plan"));
        assert!(id.contains(".openagc@"), "{id}");
    }

    #[test]
    fn writing_help_records_and_a_discarded_draft_is_marked() {
        let s = crate::guide::tests::demo("compositions-help");
        let core = &s.1;
        learned(core);
        core.agents.register("s1", Scope::Mailbox, None);
        let draft = create(core, "s1", "Hello");
        block_on(core.record_writing_help(
            draft,
            "claude-code".into(),
            "make it warmer".into(),
            "Hello Ann, lovely to hear from you.".into(),
            Some(4),
            vec!["Friends".into()],
        ))
        .unwrap();
        let records = block_on(core.ai_compositions(10)).unwrap();
        assert_eq!(records.len(), 1, "the same draft keeps one record");
        let r = &records[0];
        assert_eq!((r.source.as_str(), r.instruction.as_str()), ("writing_help", "make it warmer"));
        assert_eq!((r.guide_version, r.audiences.clone()), (Some(4), vec!["Friends".to_owned()]));
        block_on(core.delete_draft(draft)).unwrap();
        let r = &block_on(core.ai_compositions(10)).unwrap()[0];
        assert_eq!((r.status.as_str(), r.draft_id), ("discarded", None));
    }
}
