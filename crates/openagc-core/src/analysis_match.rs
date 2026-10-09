//! Matching AI compositions to the message the user sent (spec §14.10).
//! The rules are a pure function over the waiting records and the sent
//! messages; the core feeds it from the store and saves the result.

use std::collections::{BTreeMap, BTreeSet};

use mail_domain::{MessageId, Millis};
use mail_store::compositions::{self, Composition, Kind, SentCandidate, Status};

use crate::{Core, CoreError, runtime};

/// A record not matched within this long is given up on.
pub const MATCH_WINDOW_MS: Millis = 14 * 24 * 60 * 60 * 1000;
/// A record whose draft is still open is not guessed at for this long.
pub const OPEN_DRAFT_MS: Millis = 24 * 60 * 60 * 1000;
/// Under this word overlap a thread or recipient match is a different
/// message ("Thanks!" after a long draft), not an edit of the draft.
pub const MIN_OVERLAP: f64 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Sent from the draft: the Message-IDs agree.
    SentDraft,
    /// The user's next message in the thread.
    ThreadNext,
    /// The user's next message to one of the recipients.
    RecipientNext,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SentDraft => "sent_draft",
            Self::ThreadNext => "thread_next",
            Self::RecipientNext => "recipient_next",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Matched {
        message_id: String,
        method: Method,
        sent_text: String,
        distance: f64,
    },
    /// Nothing within the window.
    Unmatched,
}

/// The words of a text, lowercased, without punctuation at their ends.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Shared words over all words (Jaccard), 0–1.
pub fn overlap(a: &str, b: &str) -> f64 {
    let a: BTreeSet<String> = words(a).into_iter().collect();
    let b: BTreeSet<String> = words(b).into_iter().collect();
    let union = a.union(&b).count();
    if union == 0 { 1.0 } else { a.intersection(&b).count() as f64 / union as f64 }
}

/// Word edit distance over the longer text's length, 0 (same) to 1.
pub fn distance(a: &str, b: &str) -> f64 {
    let (a, b) = (words(a), words(b));
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 0.0;
    }
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, wa) in a.iter().enumerate() {
        let mut current = vec![i + 1; b.len() + 1];
        for (j, wb) in b.iter().enumerate() {
            let substitute = previous[j] + usize::from(wa != wb);
            current[j + 1] = substitute.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        previous = current;
    }
    previous[b.len()] as f64 / longest as f64
}

/// Match waiting records to sent messages. `sent` is oldest first; `used`
/// holds messages already matched; `text` gives a sent message's own text
/// (quotes and signature stripped), or `None` while it cannot be read.
/// Records left out of the result keep waiting.
pub fn match_records(
    waiting: &[Composition],
    sent: &[SentCandidate],
    used: &BTreeSet<String>,
    now: Millis,
    text: &mut dyn FnMut(&str) -> Option<String>,
) -> Vec<(i64, Outcome)> {
    let mut used = used.clone();
    let mut out: Vec<(i64, Outcome)> = Vec::new();
    let mut decided: BTreeSet<i64> = BTreeSet::new();
    let by_rfc822: BTreeMap<&str, &SentCandidate> =
        sent.iter().filter_map(|s| s.rfc822_message_id.as_deref().map(|id| (id, s))).collect();

    // Exact matches first: they win over any guess another record makes.
    for record in waiting {
        let Some(found) = record.rfc822_message_id.as_deref().and_then(|id| by_rfc822.get(id)) else { continue };
        if used.contains(&found.message_id) {
            continue;
        }
        let Some(sent_text) = text(&found.message_id) else { continue };
        let ai = record.ai_text.as_deref().unwrap_or_default();
        used.insert(found.message_id.clone());
        decided.insert(record.id);
        out.push((
            record.id,
            Outcome::Matched {
                message_id: found.message_id.clone(),
                method: Method::SentDraft,
                distance: distance(ai, &sent_text),
                sent_text,
            },
        ));
    }

    // Then the guesses, newest record first: the most recent record wins a
    // message, and an older one does not move on to a later message.
    let mut rest: Vec<&Composition> = waiting.iter().filter(|r| !decided.contains(&r.id)).collect();
    rest.sort_by_key(|r| std::cmp::Reverse((r.created_at, r.id)));
    for record in rest {
        let ai = record.ai_text.as_deref().unwrap_or_default();
        let after = |s: &&SentCandidate| s.at > record.created_at;
        let mut candidates: Vec<(&SentCandidate, Method)> = Vec::new();
        // A draft sent with a Message-ID waits for that exact copy; one still
        // open waits for its own send, for a day (it may go from Gmail).
        let open = record.draft_id.is_some() && now - record.created_at < OPEN_DRAFT_MS;
        // A cloud agent's report is linked to its send by the report's own
        // matching (Message-ID, or recipient, subject and time), never
        // guessed at here: its agent may send many alike.
        if record.rfc822_message_id.is_none() && !open && !compositions::is_reported(record) {
            if matches!(record.kind, Kind::Reply | Kind::Forward)
                && let Some(thread) = record.thread_id.as_deref()
                && let Some(next) = sent.iter().filter(after).find(|s| s.thread_id == thread)
            {
                candidates.push((next, Method::ThreadNext));
            }
            if matches!(record.kind, Kind::New | Kind::Forward) {
                let to = &record.recipients.to;
                let cc = &record.recipients.cc;
                let has = |s: &SentCandidate, people: &[String]| s.to.iter().chain(&s.cc).any(|a| people.contains(a));
                let next = sent
                    .iter()
                    .filter(after)
                    .find(|s| has(s, to))
                    .or_else(|| sent.iter().filter(after).find(|s| has(s, cc)));
                if let Some(next) = next {
                    candidates.push((next, Method::RecipientNext));
                }
            }
        }
        let mut matched = false;
        for (candidate, method) in candidates {
            if used.contains(&candidate.message_id) {
                continue;
            }
            let Some(sent_text) = text(&candidate.message_id) else { continue };
            if overlap(ai, &sent_text) < MIN_OVERLAP {
                continue;
            }
            used.insert(candidate.message_id.clone());
            out.push((
                record.id,
                Outcome::Matched {
                    message_id: candidate.message_id.clone(),
                    method,
                    distance: distance(ai, &sent_text),
                    sent_text,
                },
            ));
            matched = true;
            break;
        }
        if !matched && now - record.created_at > MATCH_WINDOW_MS {
            out.push((record.id, Outcome::Unmatched));
        }
    }
    out
}

/// What a matching pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MatchSummary {
    pub matched: u32,
    pub unmatched: u32,
    pub waiting: u32,
}

impl Core {
    /// Match the account's waiting records to what the user sent (spec
    /// §14.10). Bodies stored as headers only are downloaded first.
    pub(crate) async fn match_compositions(&self, now: Millis) -> Result<MatchSummary, CoreError> {
        let db = self.db()?;
        let (waiting, sent, used) = runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let waiting = compositions::list(c, Status::Waiting, u32::MAX)?;
                    let since = waiting.iter().map(|r| r.created_at).min().unwrap_or(now);
                    Ok((waiting, compositions::sent_since(c, since)?, compositions::matched_messages(c)?))
                })
                .await?)
        })
        .await?;
        if waiting.is_empty() {
            return Ok(MatchSummary::default());
        }
        if let Some(service) = self.sync_service() {
            let ids: Vec<MessageId> = sent.iter().map(|s| MessageId(s.message_id.clone())).collect();
            for chunk in ids.chunks(50) {
                if let Err(e) = service.engine().ensure_bodies(chunk.to_vec()).await {
                    tracing::warn!(error = %e, "analysis: some sent bodies not downloaded");
                }
            }
        }
        let texts = self.own_texts(sent.iter().map(|s| s.message_id.clone()).collect()).await?;
        let mut text = |id: &str| texts.get(id).cloned();
        let outcomes = match_records(&waiting, &sent, &used, now, &mut text);
        let mut summary = MatchSummary { waiting: waiting.len() as u32, ..Default::default() };
        for (_, o) in &outcomes {
            summary.waiting -= 1;
            match o {
                Outcome::Matched { .. } => summary.matched += 1,
                Outcome::Unmatched => summary.unmatched += 1,
            }
        }
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    for (id, outcome) in &outcomes {
                        match outcome {
                            Outcome::Matched { message_id, method, sent_text, distance } => compositions::set_matched(
                                tx,
                                *id,
                                message_id,
                                method.as_str(),
                                sent_text,
                                *distance,
                                now,
                            )?,
                            Outcome::Unmatched => compositions::set_status(tx, *id, Status::Unmatched, now)?,
                        }
                    }
                    Ok(())
                })
                .await?)
        })
        .await?;
        Ok(summary)
    }

    /// Sent messages down to the user's own words, as learning prepares
    /// them, by message id. Messages with no body yet are left out.
    pub(crate) async fn own_texts(&self, ids: Vec<String>) -> Result<BTreeMap<String, String>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let signature: Vec<String> = mail_store::guide::meta(c, crate::guide_learn::SIGNATURE_KEY)?
                        .map(|s| s.lines().map(str::to_owned).collect())
                        .unwrap_or_default();
                    let mut out = BTreeMap::new();
                    for id in ids {
                        let Some(body) = mail_store::read::get_body(c, &MessageId(id.clone()))? else { continue };
                        let (_, own) = crate::guide_learn::body_texts(body);
                        let lines = crate::guide_learn::own_lines(&own);
                        let lines = crate::guide_learn::without_signature(&lines, &signature);
                        out.insert(id, lines.join("\n").trim().to_owned());
                    }
                    Ok(out)
                })
                .await?)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use mail_store::compositions::{Recipients, Source};

    use super::*;

    const DAY: Millis = 24 * 60 * 60 * 1000;

    fn record(id: i64, kind: Kind, at: Millis, thread: Option<&str>, to: &[&str], text: &str) -> Composition {
        Composition {
            id,
            created_at: at,
            updated_at: at,
            source: Source::WritingHelp,
            agent: None,
            kind,
            draft_id: None,
            thread_id: thread.map(str::to_owned),
            in_reply_to: None,
            recipients: Recipients::new(to.iter().copied(), []),
            subject: String::new(),
            instruction: String::new(),
            ai_text: Some(text.to_owned()),
            ai_html: None,
            guide_version: None,
            audiences: vec![],
            rfc822_message_id: None,
            status: Status::Waiting,
            matched_message_id: None,
            match_method: None,
            sent_text: None,
            distance: None,
            reviewed_at: None,
        }
    }

    fn sent(id: &str, thread: &str, at: Millis, to: &[&str], rfc822: Option<&str>) -> SentCandidate {
        SentCandidate {
            message_id: id.into(),
            thread_id: thread.into(),
            rfc822_message_id: rfc822.map(str::to_owned),
            at,
            to: to.iter().map(|s| s.to_string()).collect(),
            cc: vec![],
        }
    }

    const AI: &str = "Hi Ann, Friday at noon works well for me. See you then.";
    const EDITED: &str = "Hi Ann, Friday at noon works. See you then!";

    fn run(
        waiting: &[Composition],
        sent: &[SentCandidate],
        texts: &[(&str, &str)],
        now: Millis,
    ) -> Vec<(i64, Outcome)> {
        let texts: BTreeMap<String, String> = texts.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        match_records(waiting, sent, &BTreeSet::new(), now, &mut |id| texts.get(id).cloned())
    }

    fn method(o: &Outcome) -> Option<(&str, Method)> {
        match o {
            Outcome::Matched { message_id, method, .. } => Some((message_id.as_str(), *method)),
            Outcome::Unmatched => None,
        }
    }

    #[test]
    fn each_rule_in_turn() {
        let mut exact = record(1, Kind::Reply, 100, Some("t1"), &["ann@x.com"], AI);
        exact.rfc822_message_id = Some("abc@openagc".into());
        let reply = record(2, Kind::Reply, 100, Some("t2"), &["bo@x.com"], AI);
        let new = record(3, Kind::New, 100, None, &["cy@x.com"], AI);
        let forward = record(4, Kind::Forward, 100, Some("t4"), &["di@x.com"], AI);
        let sent = [
            // Earlier than every record: never a match.
            sent("m0", "t2", 50, &["bo@x.com"], None),
            sent("m1", "t1", 200, &["ann@x.com"], Some("abc@openagc")),
            sent("m2", "t2", 200, &["bo@x.com"], None),
            sent("m3", "t9", 300, &["cy@x.com"], None),
            // The forward's thread gives nothing; its recipient does.
            sent("m4", "t8", 300, &["di@x.com"], None),
        ];
        let texts = [("m0", EDITED), ("m1", EDITED), ("m2", EDITED), ("m3", EDITED), ("m4", EDITED)];
        let out = run(&[exact, reply, new, forward], &sent, &texts, 400);
        let got: BTreeMap<i64, Option<(&str, Method)>> = out.iter().map(|(id, o)| (*id, method(o))).collect();
        assert_eq!(got[&1], Some(("m1", Method::SentDraft)));
        assert_eq!(got[&2], Some(("m2", Method::ThreadNext)));
        assert_eq!(got[&3], Some(("m3", Method::RecipientNext)));
        assert_eq!(got[&4], Some(("m4", Method::RecipientNext)));
    }

    #[test]
    fn a_cloud_agents_report_is_paired_by_message_id_only() {
        let mut guessed = record(1, Kind::New, 100, None, &["ann@x.com"], AI);
        guessed.source = Source::Agent;
        guessed.agent = Some("cloud:Weekly outreach routine".into());
        let mut exact = guessed.clone();
        exact.id = 2;
        exact.rfc822_message_id = Some("sent-2@agents.example".into());
        let sent = [
            sent("m1", "t1", 200, &["ann@x.com"], Some("other@agents.example")),
            sent("m2", "t2", 300, &["ann@x.com"], Some("sent-2@agents.example")),
        ];
        let out = run(&[guessed, exact], &sent, &[("m1", AI), ("m2", AI)], 400);
        let got: Vec<(i64, Option<(&str, Method)>)> = out.iter().map(|(id, o)| (*id, method(o))).collect();
        assert_eq!(got, [(2, Some(("m2", Method::SentDraft)))], "the one without a Message-ID is not guessed at");
    }

    #[test]
    fn to_comes_before_cc_and_a_reply_never_matches_by_recipient() {
        let mut new = record(1, Kind::New, 100, None, &["ann@x.com"], AI);
        new.recipients.cc = vec!["bo@x.com".into()];
        let reply = record(2, Kind::Reply, 100, Some("t1"), &["cy@x.com"], AI);
        let sent = [
            sent("m1", "t5", 200, &["bo@x.com"], None),
            sent("m2", "t6", 300, &["ann@x.com"], None),
            sent("m3", "t7", 300, &["cy@x.com"], None),
        ];
        let out = run(&[new, reply], &sent, &[("m1", EDITED), ("m2", EDITED), ("m3", EDITED)], 400);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(method(&out[0].1), Some(("m2", Method::RecipientNext)));
    }

    #[test]
    fn one_message_matches_one_record_and_the_newest_wins() {
        let older = record(1, Kind::Reply, 100, Some("t1"), &["ann@x.com"], AI);
        let newer = record(2, Kind::Reply, 150, Some("t1"), &["ann@x.com"], AI);
        let sent = [sent("m1", "t1", 200, &["ann@x.com"], None), sent("m2", "t1", 300, &["ann@x.com"], None)];
        let out = run(&[older, newer], &sent, &[("m1", EDITED), ("m2", EDITED)], 400);
        assert_eq!(out, vec![(2, out[0].1.clone())]);
        assert_eq!(method(&out[0].1), Some(("m1", Method::ThreadNext)));

        // A message already matched on an earlier day is not taken again.
        let again = record(3, Kind::Reply, 100, Some("t1"), &["ann@x.com"], AI);
        let used: BTreeSet<String> = ["m1".to_owned()].into();
        let out = match_records(&[again], &sent, &used, 400, &mut |_| Some(EDITED.to_owned()));
        assert!(out.is_empty(), "it does not move on to a later message: {out:?}");
    }

    #[test]
    fn an_unrelated_message_is_not_an_edit_and_old_records_expire() {
        let reply = record(1, Kind::Reply, 100, Some("t1"), &["ann@x.com"], AI);
        let thanks = [sent("m1", "t1", 200, &["ann@x.com"], None)];
        assert!(run(std::slice::from_ref(&reply), &thanks, &[("m1", "Thanks!")], 400).is_empty());
        let out = run(&[reply], &thanks, &[("m1", "Thanks!")], 100 + 15 * DAY);
        assert_eq!(out, vec![(1, Outcome::Unmatched)]);
        // Exact matches need no overlap: the user rewrote the draft itself.
        let mut exact = record(2, Kind::New, 100, None, &["ann@x.com"], AI);
        exact.rfc822_message_id = Some("abc".into());
        let exact_sent = [sent("m2", "t2", 200, &["ann@x.com"], Some("abc"))];
        let out = run(&[exact], &exact_sent, &[("m2", "Thanks!")], 400);
        assert_eq!(method(&out[0].1), Some(("m2", Method::SentDraft)));
    }

    #[test]
    fn a_draft_still_open_is_not_guessed_at_for_a_day() {
        let mut open = record(1, Kind::Reply, 100, Some("t1"), &["ann@x.com"], AI);
        open.draft_id = Some(9);
        let sent = [sent("m1", "t1", 200, &["ann@x.com"], None)];
        assert!(
            run(std::slice::from_ref(&open), &sent, &[("m1", EDITED)], 400).is_empty(),
            "it waits for its own send"
        );
        let later = run(&[open], &sent, &[("m1", EDITED)], 100 + OPEN_DRAFT_MS + 1);
        assert_eq!(method(&later[0].1), Some(("m1", Method::ThreadNext)), "sent some other way, after a day");
    }

    #[test]
    fn a_body_not_downloaded_yet_waits() {
        let reply = record(1, Kind::Reply, 100, Some("t1"), &["ann@x.com"], AI);
        let sent = [sent("m1", "t1", 200, &["ann@x.com"], None)];
        assert!(run(&[reply], &sent, &[], 400).is_empty());
    }

    #[test]
    fn distances() {
        assert_eq!(distance(AI, AI), 0.0);
        assert_eq!(distance("a b c d", "a b c e"), 0.25);
        assert_eq!(distance("a b", ""), 1.0);
        assert_eq!(distance("Hello, Ann.", "hello ann"), 0.0, "case and punctuation are not edits");
        let d = distance(AI, EDITED);
        assert!(d > 0.05 && d < 0.5, "{d}");
        assert!(overlap(AI, "Thanks!") < MIN_OVERLAP);
        assert!(overlap(AI, EDITED) > 0.5);
    }

    #[test]
    fn a_draft_sent_from_the_app_is_matched_to_its_copy_without_the_quote() {
        use futures::executor::block_on;
        use permissions::{Scope, Tool};

        let s = crate::guide::tests::demo("match-sent");
        let core = &s.1;
        block_on(core.debug_seed_demo_mailbox(40)).unwrap();
        let db = core.db().unwrap();
        crate::runtime::runtime()
            .block_on(db.write(|tx| {
                let run = mail_store::guide::create_run(tx, "learn", None, None, &[], 20, 1)?;
                mail_store::guide::set_run_status(tx, run, "done", None, 2)
            }))
            .unwrap();
        let thread = block_on(core.list_threads("INBOX".into(), None, 10)).unwrap().rows[0].id.clone();
        let parent = block_on(core.get_thread(thread)).unwrap().unwrap().messages.last().unwrap().id.clone();
        core.agents.register("s1", Scope::Mailbox, None);
        let out = crate::runtime::runtime().block_on(crate::agents::tools_call_for_tests(
            core,
            "s1",
            Tool::CreateDraft,
            serde_json::json!({"reply_to_message_id": parent, "body_markdown": AI}),
        ));
        let agent_mcp::Outcome::Ok { structured: Some(value), .. } = out else { panic!("{out:?}") };
        let draft_id = value["draft_id"].as_i64().unwrap();
        let mut draft = block_on(core.get_draft(draft_id)).unwrap().unwrap();
        draft.body_html =
            draft.body_html.replacen(&mail_mime::markdown_to_html(AI), &mail_mime::markdown_to_html(EDITED), 1);
        block_on(core.save_draft(draft)).unwrap();
        block_on(core.send_draft(draft_id)).unwrap();

        let summary = block_on(core.match_compositions(mail_sync::now_millis())).unwrap();
        assert_eq!(summary, MatchSummary { matched: 1, unmatched: 0, waiting: 0 });
        let r = &block_on(core.ai_compositions(5)).unwrap()[0];
        assert_eq!(r.status, "matched");
        let stored = crate::runtime::runtime().block_on(db.read(|c| compositions::recent(c, 1))).unwrap().remove(0);
        assert_eq!(stored.match_method.as_deref(), Some("sent_draft"));
        assert_eq!(stored.sent_text.as_deref(), Some(EDITED), "the quoted original is not the user's text");
        assert!(stored.distance.unwrap() > 0.05);
        // A second pass finds nothing new.
        assert_eq!(block_on(core.match_compositions(mail_sync::now_millis())).unwrap(), MatchSummary::default());
    }
}
