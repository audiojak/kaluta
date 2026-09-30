//! Learning the writing guide from sent mail (spec §14.9): choosing the
//! sample and preparing each message down to the user's own words, with
//! no agent. The processing function and the background run build on it.

use std::collections::{BTreeMap, BTreeSet};

use mail_domain::MessageId;
use mail_store::guide::{self as store, SentRow};

use crate::{Core, CoreError, ErrorKind, runtime};

/// Messages the learning dialog offers by default.
pub const DEFAULT_SAMPLE: u32 = 1_000;
/// Messages per agent turn.
pub const BATCH_SIZE: usize = 20;
/// Own text shorter than this is not worth analysing ("Thanks!").
const MIN_WORDS: usize = 2;
/// Each message's text in a prompt, at most.
pub const TEXT_CAP: usize = 3_000;
/// `guide_meta` key: the signature block found in the user's mail (B7).
pub const SIGNATURE_KEY: &str = "signature";

/// What the dialog shows before a run: how many sent messages exist, how
/// many a run of this size would analyse, and how many were done before.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideSampleInfo {
    pub sent: u32,
    pub analysed_before: u32,
    /// Messages a run with these settings would analyse.
    pub chosen: u32,
    /// Agent turns that takes (batches of `BATCH_SIZE`).
    pub batches: u32,
}

/// What to leave out of a sample.
#[derive(Debug, Clone, PartialEq, Eq, Default, uniffi::Record)]
pub struct GuideSampleFilter {
    /// Addresses or `@domain`s; a message to any of them is left out.
    pub exclude_people: Vec<String>,
    /// Label ids; a message carrying any of them is left out.
    pub exclude_labels: Vec<String>,
}

/// A sent message down to the user's own words, as the agent reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub message_id: String,
    /// `new`, `reply` or `forward`.
    pub message_type: &'static str,
    pub subject: String,
    /// `Name <address>` for To and Cc.
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub text: String,
    pub words: usize,
}

/// Subjects of mail the user did not write: automatic replies and
/// calendar responses.
fn automatic(subject: &str) -> bool {
    let s = subject.trim().to_lowercase();
    [
        "automatic reply",
        "auto reply",
        "auto-reply",
        "autoreply",
        "out of office",
        "accepted:",
        "declined:",
        "tentative:",
        "tentatively accepted:",
        "invitation:",
        "updated invitation",
        "canceled event",
        "cancelled event",
    ]
    .iter()
    .any(|p| s.starts_with(p))
}

fn matches_person(address: &str, patterns: &[String]) -> bool {
    let a = address.trim().to_lowercase();
    patterns.iter().any(|p| {
        let p = p.trim().to_lowercase();
        if let Some(domain) = p.strip_prefix('@') { a.ends_with(&format!("@{domain}")) } else { a == p }
    })
}

/// Whether the sample keeps this message (before its text is read).
pub(crate) fn keep(row: &SentRow, filter: &GuideSampleFilter) -> bool {
    !automatic(&row.subject)
        && !row.to.iter().chain(&row.cc).any(|(_, e)| matches_person(e, &filter.exclude_people))
        && !row.labels.iter().any(|l| filter.exclude_labels.contains(l))
}

pub(crate) fn message_type(subject: &str, in_reply_to: Option<&str>, body: &str) -> &'static str {
    let s = subject.trim().to_lowercase();
    let forwarded = body.lines().any(|l| l.trim().to_lowercase().starts_with("---------- forwarded message"));
    if s.starts_with("fwd:") || s.starts_with("fw:") || forwarded {
        "forward"
    } else if s.starts_with("re:") || in_reply_to.is_some_and(|r| !r.is_empty()) {
        "reply"
    } else {
        "new"
    }
}

/// The lines of the user's own text, without the quoted conversation, a
/// `-- ` signature, or a "Sent from my …" line.
fn own_lines(body: &str) -> Vec<String> {
    let own = mail_mime::strip_quoted(body);
    let mut lines: Vec<String> = Vec::new();
    for line in own.lines() {
        // RFC 3676: everything after "-- " is the signature.
        if line == "-- " || line == "--" {
            break;
        }
        lines.push(line.trim_end().to_owned());
    }
    while lines.last().is_some_and(|l| l.trim().is_empty() || l.trim().to_lowercase().starts_with("sent from my")) {
        lines.pop();
    }
    lines
}

/// The signature block a batch has in common: the longest run of two to
/// six final lines that at least three messages end with. Short sign-offs
/// ("Thanks,\nJohn") are not a signature: they are B6's evidence.
pub(crate) fn common_signature(texts: &[Vec<String>]) -> Option<Vec<String>> {
    let mut counts: BTreeMap<Vec<String>, usize> = BTreeMap::new();
    for lines in texts {
        let trimmed: Vec<String> = lines.iter().filter(|l| !l.trim().is_empty()).cloned().collect();
        for n in 3..=6.min(trimmed.len().saturating_sub(1)) {
            *counts.entry(trimmed[trimmed.len() - n..].to_vec()).or_default() += 1;
        }
    }
    counts.into_iter().filter(|(_, c)| *c >= 3).max_by_key(|(block, c)| (block.len(), *c)).map(|(block, _)| block)
}

/// Remove a known signature from the end of the lines, if it is there.
pub(crate) fn without_signature(lines: &[String], signature: &[String]) -> Vec<String> {
    let body: Vec<&String> = lines.iter().filter(|l| !l.trim().is_empty()).collect();
    if signature.is_empty() || body.len() <= signature.len() {
        return lines.to_vec();
    }
    let tail: Vec<&String> = body[body.len() - signature.len()..].to_vec();
    if tail.iter().zip(signature).all(|(a, b)| a.trim() == b.trim()) {
        // Cut at the signature's first line, counting from the end.
        let first = tail[0];
        if let Some(pos) = lines.iter().rposition(|l| std::ptr::eq(l, first)) {
            let mut kept = lines[..pos].to_vec();
            while kept.last().is_some_and(|l| l.trim().is_empty()) {
                kept.pop();
            }
            return kept;
        }
    }
    lines.to_vec()
}

fn address(name: &Option<String>, email: &str) -> String {
    match name {
        Some(n) if !n.is_empty() && n != email => format!("{n} <{email}>"),
        _ => email.to_owned(),
    }
}

impl Core {
    /// The sample's message ids, newest first: sent mail not analysed
    /// before and kept by the filter, at most `count`.
    pub(crate) async fn guide_sample(&self, count: u32, filter: GuideSampleFilter) -> Result<Vec<String>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let analysed = store::analysed_messages(c)?;
                    let total = store::sent_count(c)?;
                    let rows = store::sent_messages(c, total, &analysed)?;
                    Ok(rows
                        .into_iter()
                        .filter(|r| keep(r, &filter))
                        .take(count as usize)
                        .map(|r| r.message_id)
                        .collect())
                })
                .await?)
        })
        .await
    }

    /// Prepare a batch: download any header-only bodies first (§7.4), then
    /// reduce each message to the user's own text. Messages with too
    /// little of it are left out. The signature found here (or before) is
    /// removed and remembered for B7.
    pub(crate) async fn prepare_batch(&self, ids: Vec<String>) -> Result<Vec<Prepared>, CoreError> {
        if let Some(service) = self.sync_service() {
            let wanted: Vec<MessageId> = ids.iter().map(|i| MessageId(i.clone())).collect();
            for chunk in wanted.chunks(50) {
                if let Err(e) = service.engine().ensure_bodies(chunk.to_vec()).await {
                    tracing::warn!(error = %e, "guide: some bodies not downloaded; those messages use what is stored");
                }
            }
        }
        let db = self.db()?;
        let rows = runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let wanted: BTreeSet<&String> = ids.iter().collect();
                    let total = store::sent_count(c)?;
                    let sent = store::sent_messages(c, total, &BTreeSet::new())?;
                    let mut out = Vec::new();
                    for row in sent.into_iter().filter(|r| wanted.contains(&r.message_id)) {
                        let body =
                            mail_store::read::get_body(c, &MessageId(row.message_id.clone()))?.unwrap_or_default();
                        let text = body
                            .text_plain
                            .filter(|t| !t.trim().is_empty())
                            .or_else(|| body.html_sanitized.map(|h| mail_mime::html_to_text(&h)))
                            .unwrap_or_default();
                        out.push((row, text));
                    }
                    let signature = store::meta(c, SIGNATURE_KEY)?;
                    Ok((out, signature))
                })
                .await?)
        })
        .await?;
        let (rows, known) = rows;
        let texts: Vec<Vec<String>> = rows.iter().map(|(_, text)| own_lines(text)).collect();
        let signature: Vec<String> = match known {
            Some(s) => s.lines().map(str::to_owned).collect(),
            None => {
                let found = common_signature(&texts).unwrap_or_default();
                if !found.is_empty() {
                    let db = self.db()?;
                    let saved = found.join("\n");
                    runtime::run(
                        async move { Ok(db.write(move |tx| store::set_meta(tx, SIGNATURE_KEY, &saved)).await?) },
                    )
                    .await?;
                }
                found
            }
        };
        Ok(rows
            .into_iter()
            .zip(texts)
            .filter_map(|((row, full), lines)| {
                let lines = without_signature(&lines, &signature);
                let text: String = lines.join("\n").trim().chars().take(TEXT_CAP).collect();
                let words = text.split_whitespace().count();
                (words >= MIN_WORDS).then(|| Prepared {
                    message_type: message_type(&row.subject, row.in_reply_to.as_deref(), &full),
                    message_id: row.message_id,
                    subject: row.subject,
                    to: row.to.iter().map(|(n, e)| address(n, e)).collect(),
                    cc: row.cc.iter().map(|(n, e)| address(n, e)).collect(),
                    text,
                    words,
                })
            })
            .collect())
    }
}

#[uniffi::export]
impl Core {
    /// Messages the learning dialog offers by default, and per agent turn.
    pub fn guide_sample_defaults(&self) -> Vec<u32> {
        vec![DEFAULT_SAMPLE, BATCH_SIZE as u32]
    }

    /// The signature block found in the user's sent mail (B7), if any: the
    /// interview suggests answers from it.
    pub async fn guide_signature(&self) -> Result<Option<String>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(|c| store::meta(c, SIGNATURE_KEY)).await?) }).await
    }

    /// For the learning dialog: sent messages available, analysed before,
    /// and how many a run of `count` with this filter would analyse.
    pub async fn guide_sample_info(&self, count: u32, filter: GuideSampleFilter) -> Result<GuideSampleInfo, CoreError> {
        if count == 0 {
            return Err(CoreError::new(ErrorKind::InvalidInput, "choose at least one message"));
        }
        let chosen = self.guide_sample(count, filter).await?.len() as u32;
        let db = self.db()?;
        let (sent, analysed) = runtime::run(async move {
            Ok(db.read(|c| Ok((store::sent_count(c)?, store::analysed_messages(c)?.len() as u32))).await?)
        })
        .await?;
        Ok(GuideSampleInfo { sent, analysed_before: analysed, chosen, batches: chosen.div_ceil(BATCH_SIZE as u32) })
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;
    use crate::guide::tests::demo;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_owned).collect()
    }

    #[test]
    fn own_text_loses_quotes_signatures_and_phone_lines() {
        let body = "Sounds good, see you Friday.\n\nJohn\n\nSent from my iPhone\n\nOn Tue, Sep 15, 2026 at 9:00 AM Ann <a@x.com> wrote:\n> Friday?";
        assert_eq!(own_lines(body), lines("Sounds good, see you Friday.\n\nJohn"));
        let dashed = "Yes.\n-- \nJohn Kennedy\nCEO";
        assert_eq!(own_lines(dashed), lines("Yes."));
    }

    #[test]
    fn a_signature_is_what_several_messages_end_with() {
        let sig = "John Kennedy\nCEO, Actual AI\n+1 555 0100";
        let texts: Vec<Vec<String>> = [
            format!("Thanks for this.\n\n{sig}"),
            format!("Can we move it to 3pm?\n\n{sig}"),
            format!("Attached.\n{sig}"),
            "Sure.\n\nJohn".to_owned(),
        ]
        .iter()
        .map(|t| own_lines(t))
        .collect();
        let found = common_signature(&texts).expect("found");
        assert_eq!(found, lines(sig));
        assert_eq!(without_signature(&texts[0], &found), lines("Thanks for this."));
        assert_eq!(without_signature(&texts[3], &found), texts[3], "a short sign-off stays");
        assert!(common_signature(&texts[..2]).is_none(), "two is not a pattern");
    }

    #[test]
    fn message_types_and_automatic_mail() {
        assert_eq!(message_type("Re: Lunch", None, ""), "reply");
        assert_eq!(message_type("Lunch", Some("<a@b>"), ""), "reply");
        assert_eq!(message_type("Fwd: Invoice", None, ""), "forward");
        assert_eq!(message_type("Invoice", None, "FYI\n---------- Forwarded message ---------\nFrom: x"), "forward");
        assert_eq!(message_type("Plan", None, ""), "new");
        assert!(automatic("Automatic reply: away") && automatic("Accepted: Standup"));
        assert!(!automatic("Accepting the offer"));
        let row = SentRow {
            subject: "Hi".into(),
            to: vec![(None, "ann@acme.com".into())],
            labels: vec!["Label_1".into()],
            ..Default::default()
        };
        assert!(keep(&row, &GuideSampleFilter::default()));
        assert!(!keep(&row, &GuideSampleFilter { exclude_people: vec!["@ACME.com".into()], ..Default::default() }));
        assert!(!keep(&row, &GuideSampleFilter { exclude_labels: vec!["Label_1".into()], ..Default::default() }));
    }

    #[test]
    fn the_sample_is_sent_mail_newest_first_and_prepares_to_own_words() {
        let s = demo("sample");
        let core = &s.1;
        block_on(core.debug_seed_demo_mailbox(80)).unwrap();
        let info = block_on(core.guide_sample_info(DEFAULT_SAMPLE, GuideSampleFilter::default())).unwrap();
        assert!(info.sent > 0, "the demo mailbox has sent mail");
        assert!(info.chosen <= info.sent && info.analysed_before == 0);
        assert_eq!(info.batches, info.chosen.div_ceil(20));
        let ids = block_on(core.guide_sample(5, GuideSampleFilter::default())).unwrap();
        assert!(!ids.is_empty() && ids.len() <= 5);
        let prepared = block_on(core.prepare_batch(ids.clone())).unwrap();
        assert!(!prepared.is_empty());
        for p in &prepared {
            assert!(ids.contains(&p.message_id));
            assert!(!p.text.contains("\n>"), "no quoted lines");
            assert!(p.words >= MIN_WORDS && p.text.chars().count() <= TEXT_CAP);
            assert!(["new", "reply", "forward"].contains(&p.message_type));
        }
        assert!(block_on(core.guide_sample_info(0, GuideSampleFilter::default())).is_err());
    }
}
