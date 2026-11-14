//! Forwarding rules (spec §14.15, ADR 0021): when mail from one sender
//! with one subject prefix is always forwarded to the same address (a
//! receipt to the bookkeeping inbox), Kaluta proposes a rule; an accepted
//! rule forwards what arrives afterwards, on its own or after asking, as
//! the user chose when accepting it.
//!
//! Everything here is code over stored headers: no agent reads the mail,
//! proposes, or decides, and no agent tool reaches a rule. A rule always
//! names a sender, its recipient is fixed, and each message is dealt with
//! once per rule (`forward_rule_log`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use mail_domain::{EmailAddress, MessageId};
use mail_store::forward_rules::{self as store, Evidence, NewProposal, Received, Rule, SentForward};

use crate::events::CoreEvent;
use crate::{Core, CoreError, ErrorKind, runtime};

const DAY_MS: i64 = 24 * 3600 * 1000;
/// How far back the user's forwards are read.
const WATCH_DAYS: i64 = 90;
/// Forwards of the same kind before a rule is proposed.
const MIN_FORWARDS: usize = 3;
const PREFIX_CAP: usize = 60;
/// The most one rule forwards on its own in a day; past it, it asks.
pub(crate) const DAILY_CAP: u32 = 20;
/// The user's forwards are read again at most this often.
const WATCH_EVERY_MS: i64 = 10 * 60 * 1000;

/// What an accepted rule does with a message that matches. Stored by its
/// serde name (`auto`, `ask`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, uniffi::Enum)]
pub enum ForwardMode {
    /// Forwards it, with no one asked.
    #[serde(rename = "auto")]
    Automatic,
    /// Suggests the forward; nothing is sent until the user takes it.
    #[serde(rename = "ask")]
    AskEachTime,
}

impl ForwardMode {
    /// As the store holds it.
    fn stored(self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
    }

    /// From the store; anything unknown asks, the mode that sends nothing.
    fn from_stored(s: &str) -> Self {
        serde_json::from_value(serde_json::Value::String(s.to_owned())).unwrap_or(Self::AskEachTime)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ForwardRuleInfo {
    pub id: i64,
    /// An address, or a domain when `sender_is_domain`.
    pub sender: String,
    pub sender_is_domain: bool,
    pub subject_prefix: String,
    pub recipient: String,
    pub mode: ForwardMode,
    pub enabled: bool,
    /// "Forward mail from … starting “…” to …".
    pub statement: String,
    /// How many it has forwarded, and when last.
    pub sent_count: u32,
    pub last_sent_at: Option<i64>,
}

/// A rule as the user writes or edits it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ForwardRuleEdit {
    pub sender: String,
    pub sender_is_domain: bool,
    pub subject_prefix: String,
    pub recipient: String,
    pub mode: ForwardMode,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ForwardEvidenceInfo {
    pub thread_id: String,
    pub message_id: String,
    pub subject: String,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ForwardRuleProposalInfo {
    pub id: i64,
    pub statement: String,
    pub sender: String,
    pub subject_prefix: String,
    pub recipient: String,
    /// The mail the user forwarded, oldest first.
    pub evidence: Vec<ForwardEvidenceInfo>,
}

/// A rule set to ask, waiting on a message in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ForwardSuggestionInfo {
    pub rule_id: i64,
    pub message_id: String,
    pub recipient: String,
}

/// A message a rule forwarded: the reader's "Forwarded by a rule".
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ForwardMarkInfo {
    pub rule_id: i64,
    pub message_id: String,
    pub recipient: String,
    pub at: i64,
}

// ---- What matches, and what makes a pattern ----

/// A subject without the `Re:` and `Fwd:` in front of it.
pub(crate) fn bare_subject(subject: &str) -> &str {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        match ["re:", "fwd:", "fw:"].iter().find(|p| lower.starts_with(**p)) {
            Some(p) => s = s[p.len()..].trim_start(),
            None => return s,
        }
    }
}

/// The leading words these subjects share (case aside), cut before the
/// first word that holds a digit (an order number, an amount, a date) and
/// kept to [`PREFIX_CAP`] characters. Empty when they share none.
pub(crate) fn shared_prefix(subjects: &[&str]) -> String {
    let split: Vec<Vec<&str>> = subjects.iter().map(|s| bare_subject(s).split_whitespace().collect()).collect();
    let Some(first) = split.first() else { return String::new() };
    let mut words: Vec<&str> = Vec::new();
    for (i, word) in first.iter().enumerate() {
        if word.chars().any(|c| c.is_ascii_digit()) {
            break;
        }
        let lower = word.to_lowercase();
        if !split.iter().all(|s| s.get(i).is_some_and(|w| w.to_lowercase() == lower)) {
            break;
        }
        words.push(word);
    }
    while words.join(" ").chars().count() > PREFIX_CAP {
        words.pop();
    }
    words.join(" ")
}

fn subject_starts(subject: &str, prefix: &str) -> bool {
    bare_subject(subject).to_lowercase().starts_with(&prefix.trim().to_lowercase())
}

/// Whether a rule is for this message: its sender, then its subject.
pub(crate) fn matches(rule: &Rule, from_email: &str, subject: &str) -> bool {
    let from = from_email.trim().to_lowercase();
    let sender = if rule.sender_is_domain {
        from.rsplit_once('@').is_some_and(|(_, domain)| domain == rule.sender)
    } else {
        from == rule.sender
    };
    sender && subject_starts(subject, &rule.subject_prefix)
}

pub(crate) fn statement(sender: &str, is_domain: bool, prefix: &str, recipient: &str) -> String {
    let from = if is_domain { format!("anyone at {sender}") } else { sender.to_owned() };
    if prefix.is_empty() {
        format!("Forward all mail from {from} to {recipient}")
    } else {
        format!("Forward mail from {from} starting “{prefix}” to {recipient}")
    }
}

fn match_key(sender: &str, prefix: &str, recipient: &str) -> String {
    format!("{sender}|{}|{recipient}", prefix.to_lowercase())
}

fn invalid(message: &str) -> CoreError {
    CoreError::new(ErrorKind::InvalidInput, message)
}

fn is_address(s: &str) -> bool {
    !s.chars().any(char::is_whitespace)
        && s.split_once('@').is_some_and(|(name, domain)| {
            !name.is_empty() && !domain.contains('@') && domain.contains('.') && !domain.starts_with('.')
        })
}

/// An edit as it is stored: trimmed, lower case where case means nothing,
/// and refused when it could not be a rule. A rule always names a sender;
/// a whole domain also needs a subject, or it would forward everything a
/// company sends.
fn cleaned(edit: &ForwardRuleEdit) -> Result<ForwardRuleEdit, CoreError> {
    let mut sender = edit.sender.trim().to_lowercase();
    if edit.sender_is_domain {
        sender = sender.trim_start_matches('@').to_owned();
        let plausible = sender.contains('.') && !sender.starts_with('.') && !sender.ends_with('.');
        if !plausible || sender.contains('@') || sender.chars().any(char::is_whitespace) {
            return Err(invalid("Say which domain the mail comes from, like example.com."));
        }
    } else if !is_address(&sender) {
        return Err(invalid("Say which address the mail comes from."));
    }
    let subject_prefix = bare_subject(&edit.subject_prefix).to_owned();
    if subject_prefix.chars().count() > 200 {
        return Err(invalid("That subject is too long to be the start of one."));
    }
    if edit.sender_is_domain && subject_prefix.is_empty() {
        return Err(invalid("A rule for a whole domain needs the start of a subject too."));
    }
    let recipient = edit.recipient.trim().to_lowercase();
    if !is_address(&recipient) {
        return Err(invalid("Say which address to forward to."));
    }
    Ok(ForwardRuleEdit { sender, subject_prefix, recipient, ..edit.clone() })
}

/// The user's forwards of one kind: from one sender, to one address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pattern {
    pub sender: String,
    pub recipient: String,
    pub prefix: String,
    /// The mail forwarded, oldest first.
    pub evidence: Vec<Evidence>,
}

impl Pattern {
    fn key(&self) -> String {
        match_key(&self.sender, &self.prefix, &self.recipient)
    }

    /// Mail of this kind the user did not forward, among what arrived up
    /// to the last one they did: with any, it is not "always". Later mail
    /// is not counted: the user may not have got to it yet.
    fn left_unforwarded(&self, received: &[Received]) -> bool {
        let forwarded: HashSet<&str> = self.evidence.iter().map(|e| e.message_id.as_str()).collect();
        let last = self.evidence.iter().map(|e| e.at).max().unwrap_or(0);
        received.iter().any(|m| {
            m.at <= last && !forwarded.contains(m.message_id.as_str()) && subject_starts(&m.subject, &self.prefix)
        })
    }
}

/// The patterns in the user's forwards: [`MIN_FORWARDS`] or more from one
/// sender to one address. Forwards a rule sent are its doing, not the
/// user's, and a pattern a rule already covers is not one to propose.
pub(crate) fn patterns(forwards: &[SentForward], by_rules: &HashSet<String>, rules: &[Rule]) -> Vec<Pattern> {
    let mut groups: BTreeMap<(&str, &str), Vec<&SentForward>> = BTreeMap::new();
    for f in forwards {
        let o = &f.original;
        if by_rules.contains(&o.message_id) || o.from_email.is_empty() || o.from_email == f.recipient {
            continue;
        }
        let group = groups.entry((o.from_email.as_str(), f.recipient.as_str())).or_default();
        if !group.iter().any(|g| g.original.message_id == o.message_id) {
            group.push(f);
        }
    }
    let mut found = Vec::new();
    for ((sender, recipient), group) in groups {
        if group.len() < MIN_FORWARDS {
            continue;
        }
        let covered = rules
            .iter()
            .any(|r| r.recipient == recipient && group.iter().all(|f| matches(r, sender, &f.original.subject)));
        if covered {
            continue;
        }
        let subjects: Vec<&str> = group.iter().map(|f| f.original.subject.as_str()).collect();
        let mut evidence: Vec<Evidence> = group
            .iter()
            .map(|f| Evidence {
                thread_id: f.original.thread_id.clone(),
                message_id: f.original.message_id.clone(),
                subject: f.original.subject.clone(),
                at: f.original.at,
            })
            .collect();
        evidence.sort_by_key(|e| e.at);
        found.push(Pattern {
            sender: sender.to_owned(),
            recipient: recipient.to_owned(),
            prefix: shared_prefix(&subjects),
            evidence,
        });
    }
    found
}

fn watched() -> &'static Mutex<HashMap<String, i64>> {
    static WATCHED: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();
    WATCHED.get_or_init(Default::default)
}

// ---- Across the FFI ----

fn proposal_info(p: store::Proposal) -> ForwardRuleProposalInfo {
    ForwardRuleProposalInfo {
        id: p.id,
        statement: p.statement,
        sender: p.sender,
        subject_prefix: p.subject_prefix,
        recipient: p.recipient,
        evidence: p
            .evidence
            .into_iter()
            .map(|e| ForwardEvidenceInfo {
                thread_id: e.thread_id,
                message_id: e.message_id,
                subject: e.subject,
                at: e.at,
            })
            .collect(),
    }
}

fn rule_info(r: Rule, sent: (u32, Option<i64>)) -> ForwardRuleInfo {
    ForwardRuleInfo {
        statement: statement(&r.sender, r.sender_is_domain, &r.subject_prefix, &r.recipient),
        id: r.id,
        sender: r.sender,
        sender_is_domain: r.sender_is_domain,
        subject_prefix: r.subject_prefix,
        recipient: r.recipient,
        mode: ForwardMode::from_stored(&r.mode),
        enabled: r.enabled,
        sent_count: sent.0,
        last_sent_at: sent.1,
    }
}

fn stored(edit: &ForwardRuleEdit, effective_from: i64, proposal_id: Option<i64>) -> Rule {
    Rule {
        id: 0,
        sender: edit.sender.clone(),
        sender_is_domain: edit.sender_is_domain,
        subject_prefix: edit.subject_prefix.clone(),
        recipient: edit.recipient.clone(),
        mode: edit.mode.stored(),
        enabled: edit.enabled,
        effective_from,
        proposal_id,
        created_at: effective_from,
    }
}

impl Core {
    /// An imported mailbox cannot send, so it has no forwarding rules.
    fn refuse_rules_if_archive(&self) -> Result<(), CoreError> {
        match self.effective_account_id() {
            Some(id) if self.is_archive(&id) => Err(invalid(
                "This account is an imported mailbox: it cannot forward mail, so it has no forwarding rules.",
            )),
            _ => Ok(()),
        }
    }

    fn forward_rules_changed(&self) {
        let events = self.account_events();
        events.emit(CoreEvent::ForwardRulesChanged);
        events.emit(CoreEvent::WorkflowsChanged);
    }

    /// Read the user's forwards and bring the proposals up to date: a
    /// pattern that holds is proposed (or its evidence refreshed), one
    /// still waiting that no longer holds is withdrawn. Whether anything
    /// changed.
    pub(crate) async fn watch_forwards(&self) -> Result<bool, CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let changed = runtime::run(async move {
            let since = now - WATCH_DAYS * DAY_MS;
            let (forwards, by_rules, rules, before) = db
                .read(move |c| {
                    Ok((
                        store::sent_forwards(c, since)?,
                        store::rule_sent_messages(c)?,
                        store::rules(c)?,
                        store::proposed(c)?,
                    ))
                })
                .await?;
            let mut holding = Vec::new();
            for p in patterns(&forwards, &by_rules, &rules) {
                let (sender, first) = (p.sender.clone(), p.evidence.first().map_or(since, |e| e.at));
                let received = db.read(move |c| store::received_from(c, &sender, false, first)).await?;
                if !p.left_unforwarded(&received) {
                    holding.push(p);
                }
            }
            let after = db
                .write(move |tx| {
                    let keys: HashSet<String> = holding.iter().map(Pattern::key).collect();
                    for key in store::proposed_keys(tx)?.into_iter().filter(|k| !keys.contains(k)) {
                        store::withdraw_proposed(tx, &key)?;
                    }
                    for p in &holding {
                        store::propose(
                            tx,
                            &NewProposal {
                                statement: &statement(&p.sender, false, &p.prefix, &p.recipient),
                                sender: &p.sender,
                                subject_prefix: &p.prefix,
                                recipient: &p.recipient,
                                match_key: &p.key(),
                                evidence: p.evidence.clone(),
                            },
                            now,
                        )?;
                    }
                    store::proposed(tx)
                })
                .await?;
            Ok(before != after)
        })
        .await?;
        if changed {
            self.account_events().emit(CoreEvent::WorkflowsChanged);
        }
        Ok(changed)
    }

    /// Forward one message to one address as a rule does: the forward as
    /// *Forward* makes it, with the original's attached files, no note,
    /// held for `hold_ms` (`None`: the Undo Send delay). Whether it is
    /// held.
    async fn forward_by_rule(
        &self,
        message_id: &str,
        recipient: &str,
        hold_ms: Option<u64>,
    ) -> Result<bool, CoreError> {
        self.refuse_if_archive()?;
        let db = self.db()?;
        let service = self.sync_service();
        let cache = self.attachment_cache_dir()?;
        let drafts_dir = self.data_path().join("DraftAttachments");
        let (id, to) = (MessageId(message_id.to_owned()), EmailAddress::new(None, recipient));
        let draft_id = runtime::run(async move {
            // The whole message, or no forward: a receipt is its attachment.
            if let Some(service) = &service {
                service.engine().ensure_bodies(vec![id.clone()]).await?;
            }
            let provider = service.as_ref().map(|s| s.engine().provider());
            let mut draft = mail_sync::forward_draft_with_attachments(&db, provider, &cache, &drafts_dir, &id).await?;
            draft.to = vec![to];
            let now = mail_sync::now_millis();
            Ok(db.write(move |tx| mail_store::drafts::save(tx, &draft, now)).await?)
        })
        .await?;
        match self.send_draft_holding(draft_id, hold_ms).await {
            Ok(held) => Ok(held),
            Err(e) => {
                let _ = self.delete_draft(draft_id).await;
                Err(e)
            }
        }
    }

    /// Apply the account's rules to the mail that arrived since each was
    /// accepted: an automatic rule forwards (up to [`DAILY_CAP`] a day,
    /// then asks), one that asks leaves a suggestion. Each message once
    /// per rule. How many were forwarded and how many asked about.
    pub(crate) async fn apply_forward_rules(&self) -> Result<(u32, u32), CoreError> {
        if self.effective_account_id().is_some_and(|id| self.is_archive(&id)) {
            return Ok((0, 0));
        }
        let db = self.db()?;
        let reader = db.clone();
        let rules = runtime::run(async move { Ok(reader.read(store::rules).await?) }).await?;
        let (mut sent, mut asked) = (0, 0);
        for rule in rules.into_iter().filter(|r| r.enabled) {
            let (reader, r) = (db.clone(), rule.clone());
            let waiting: Vec<Received> = runtime::run(async move {
                Ok(reader
                    .read(move |c| {
                        let done = store::logged_messages(c, r.id)?;
                        let received = store::received_from(c, &r.sender, r.sender_is_domain, r.effective_from)?;
                        Ok(received
                            .into_iter()
                            .filter(|m| !done.contains(&m.message_id) && matches(&r, &m.from_email, &m.subject))
                            .collect())
                    })
                    .await?)
            })
            .await?;
            for m in waiting {
                let now = mail_sync::now_millis();
                let (writer, r, message) = (db.clone(), rule.clone(), m.clone());
                // Claimed before it is sent: a crash in between loses one
                // forward rather than sending two.
                let outcome = runtime::run(async move {
                    Ok(writer
                        .write(move |tx| {
                            let automatic = r.mode == "auto" && store::sent_since(tx, r.id, now - DAY_MS)? < DAILY_CAP;
                            let outcome = if automatic { "sent" } else { "asked" };
                            let first = store::log_first(
                                tx,
                                r.id,
                                &message.message_id,
                                &message.thread_id,
                                &r.recipient,
                                outcome,
                                now,
                            )?;
                            Ok(first.then_some(outcome))
                        })
                        .await?)
                })
                .await?;
                match outcome {
                    Some("sent") => match self.forward_by_rule(&m.message_id, &rule.recipient, Some(0)).await {
                        Ok(_) => sent += 1,
                        Err(e) => {
                            tracing::warn!(rule = rule.id, error = %e, "a rule's forward could not be made; trying at the next sync");
                            let (writer, id) = (db.clone(), m.message_id.clone());
                            runtime::run(async move {
                                Ok(writer.write(move |tx| store::log_remove(tx, rule.id, &id)).await?)
                            })
                            .await?;
                        }
                    },
                    Some(_) => asked += 1,
                    None => {}
                }
            }
        }
        if sent + asked > 0 {
            self.forward_rules_changed();
        }
        Ok((sent, asked))
    }

    /// After each sync: the rules are applied to what came in, and now and
    /// then the user's forwards are read for new patterns.
    pub(crate) async fn forward_rules_after_sync(&self, account_id: &str) {
        if self.is_archive(account_id) {
            return;
        }
        if let Err(e) = self.apply_forward_rules().await {
            tracing::warn!(error = %e, "forwarding rules not applied");
        }
        let key = format!("{}/{account_id}", self.data_path().display());
        let now = mail_sync::now_millis();
        let due = {
            let mut watched = watched().lock().unwrap_or_else(|e| e.into_inner());
            let due = watched.get(&key).is_none_or(|last| now - last >= WATCH_EVERY_MS);
            if due {
                watched.insert(key, now);
            }
            due
        };
        if due && let Err(e) = self.watch_forwards().await {
            tracing::warn!(error = %e, "the user's forwards not read for patterns");
        }
    }

    async fn rule_with_counts(&self, id: i64) -> Result<ForwardRuleInfo, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            let found = db.read(move |c| Ok(store::get_rule(c, id)?.map(|r| (r, store::rule_sent(c, id))))).await?;
            match found {
                Some((rule, sent)) => Ok(rule_info(rule, sent?)),
                None => Err(CoreError::new(ErrorKind::NotFound, "that forwarding rule no longer exists")),
            }
        })
        .await
    }
}

#[uniffi::export]
impl Core {
    /// The account's forwarding rules, oldest first.
    pub async fn forward_rules(&self) -> Result<Vec<ForwardRuleInfo>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(|c| {
                    let mut out = Vec::new();
                    for r in store::rules(c)? {
                        let sent = store::rule_sent(c, r.id)?;
                        out.push(rule_info(r, sent));
                    }
                    Ok(out)
                })
                .await?)
        })
        .await
    }

    /// Write a rule by hand (`id` `None`) or change one. Changing who it
    /// is from, its subject or where it sends, or turning it back on,
    /// makes it apply only to mail that arrives from now.
    pub async fn save_forward_rule(
        &self,
        id: Option<i64>,
        edit: ForwardRuleEdit,
    ) -> Result<ForwardRuleInfo, CoreError> {
        self.refuse_rules_if_archive()?;
        let edit = cleaned(&edit)?;
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let saved = runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    let mut rule = stored(&edit, now, None);
                    if let Some(id) = id {
                        let Some(old) = store::get_rule(tx, id)? else { return Ok(None) };
                        let same = old.sender == rule.sender
                            && old.sender_is_domain == rule.sender_is_domain
                            && old.subject_prefix.to_lowercase() == rule.subject_prefix.to_lowercase()
                            && old.recipient == rule.recipient
                            && (old.enabled || !rule.enabled);
                        rule.id = id;
                        rule.proposal_id = old.proposal_id;
                        if same {
                            rule.effective_from = old.effective_from;
                        }
                    }
                    Ok(Some(store::save_rule(tx, &rule, now)?))
                })
                .await?)
        })
        .await?
        .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that forwarding rule no longer exists"))?;
        self.forward_rules_changed();
        self.rule_with_counts(saved).await
    }

    /// Remove a rule. What it forwarded stays sent; a proposal it came
    /// from stays accepted, so the same pattern is not proposed again.
    pub async fn delete_forward_rule(&self, id: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.write(move |tx| store::delete_rule(tx, id)).await?) }).await?;
        self.forward_rules_changed();
        Ok(())
    }

    /// The forwarding rules Kaluta proposes, waiting in Teach Kaluta.
    pub async fn forward_rule_proposals(&self) -> Result<Vec<ForwardRuleProposalInfo>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(store::proposed).await?.into_iter().map(proposal_info).collect()) }).await
    }

    /// Accept a proposal as the rule given: the proposal's own sender,
    /// subject and recipient or the user's edit of them, and the mode the
    /// user chose (there is no default: accepting asks). The rule applies
    /// to mail that arrives from now.
    pub async fn accept_forward_rule_proposal(
        &self,
        id: i64,
        edit: ForwardRuleEdit,
    ) -> Result<ForwardRuleInfo, CoreError> {
        self.refuse_rules_if_archive()?;
        let edit = cleaned(&edit)?;
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let rule = runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    if !store::get_proposal(tx, id)?.is_some_and(|p| p.status == "proposed") {
                        return Ok(None);
                    }
                    let rule = store::save_rule(tx, &stored(&edit, now, Some(id)), now)?;
                    store::set_proposal_status(tx, id, "accepted", Some(rule), now)?;
                    Ok(Some(rule))
                })
                .await?)
        })
        .await?
        .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that rule is no longer proposed"))?;
        self.forward_rules_changed();
        self.rule_with_counts(rule).await
    }

    /// Reject a proposal: the same pattern is not proposed again.
    pub async fn reject_forward_rule_proposal(&self, id: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db.write(move |tx| store::set_proposal_status(tx, id, "rejected", None, now)).await?)
        })
        .await?;
        self.forward_rules_changed();
        Ok(())
    }

    /// Undo of a decision: proposed again, and the rule an acceptance made
    /// is removed. What it forwarded meanwhile stays sent.
    pub async fn reopen_forward_rule_proposal(&self, id: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    let Some(p) = store::get_proposal(tx, id)? else { return Ok(()) };
                    if let Some(rule) = p.rule_id {
                        store::delete_rule(tx, rule)?;
                    }
                    store::set_proposal_status(tx, id, "proposed", None, now)
                })
                .await?)
        })
        .await?;
        self.forward_rules_changed();
        Ok(())
    }

    /// The forward a rule set to ask suggests for a conversation, if one
    /// is waiting.
    pub async fn forward_suggestion(&self, thread_id: String) -> Result<Option<ForwardSuggestionInfo>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let enabled: HashSet<i64> =
                        store::rules(c)?.into_iter().filter(|r| r.enabled).map(|r| r.id).collect();
                    Ok(store::log_for_thread(c, &thread_id)?
                        .into_iter()
                        .rev()
                        .find(|l| l.outcome == "asked" && enabled.contains(&l.rule_id))
                        .map(|l| ForwardSuggestionInfo {
                            rule_id: l.rule_id,
                            message_id: l.message_id,
                            recipient: l.recipient,
                        }))
                })
                .await?)
        })
        .await
    }

    /// The conversations with a forward suggested and waiting.
    pub async fn forward_suggestion_threads(&self) -> Result<Vec<String>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(store::asked_threads).await?) }).await
    }

    /// Take a suggestion: the forward is sent at once (the user just asked
    /// for it by name; there is no Undo Send wait). Refused when it is no
    /// longer waiting, so two answers cannot send two.
    pub async fn take_forward_suggestion(&self, rule_id: i64, message_id: String) -> Result<bool, CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let (writer, id) = (db.clone(), message_id.clone());
        let recipient = runtime::run(async move {
            Ok(writer
                .write(move |tx| {
                    let Some(waiting) = store::logged_one(tx, rule_id, &id)? else { return Ok(None) };
                    let asked = store::log_resolve(tx, rule_id, &id, "asked", "sent", None, now)?;
                    Ok(asked.then_some(waiting.recipient))
                })
                .await?)
        })
        .await?
        .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that forward is no longer waiting"))?;
        match self.forward_by_rule(&message_id, &recipient, Some(0)).await {
            Ok(held) => {
                self.forward_rules_changed();
                Ok(held)
            }
            Err(e) => {
                // Not sent: it waits again.
                runtime::run(async move {
                    Ok(db
                        .write(move |tx| store::log_resolve(tx, rule_id, &message_id, "sent", "asked", None, now))
                        .await?)
                })
                .await?;
                Err(e)
            }
        }
    }

    /// *Not this one*: the suggestion goes, and is not made again.
    pub async fn dismiss_forward_suggestion(&self, rule_id: i64, message_id: String) -> Result<(), CoreError> {
        self.resolve_suggestion(rule_id, message_id, "asked", "dismissed").await
    }

    /// Undo of *Not this one*: the suggestion waits again.
    pub async fn restore_forward_suggestion(&self, rule_id: i64, message_id: String) -> Result<(), CoreError> {
        self.resolve_suggestion(rule_id, message_id, "dismissed", "asked").await
    }

    /// What rules forwarded in a conversation, oldest first: the reader's
    /// "Forwarded by a rule · address".
    pub async fn forward_marks(&self, thread_id: String) -> Result<Vec<ForwardMarkInfo>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| store::log_for_thread(c, &thread_id))
                .await?
                .into_iter()
                .filter(|l| l.outcome == "sent")
                .map(|l| ForwardMarkInfo {
                    rule_id: l.rule_id,
                    message_id: l.message_id,
                    recipient: l.recipient,
                    at: l.at,
                })
                .collect())
        })
        .await
    }

    /// Read the user's forwards for patterns now (the app asks when Teach
    /// Kaluta opens, so a proposal does not wait on the next sync).
    pub async fn refresh_forward_rule_proposals(&self) -> Result<(), CoreError> {
        self.watch_forwards().await.map(|_| ())
    }
}

impl Core {
    async fn resolve_suggestion(
        &self,
        rule_id: i64,
        message_id: String,
        from: &'static str,
        to: &'static str,
    ) -> Result<(), CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db.write(move |tx| store::log_resolve(tx, rule_id, &message_id, from, to, None, now)).await?)
        })
        .await?;
        self.forward_rules_changed();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::executor::block_on;
    use mail_domain::{LabelId, ThreadId};
    use mail_store::{IncomingMessage, MailWriter};
    use provider_api::{FetchedAttachment, FetchedBody, FetchedMessage};

    use super::*;
    use crate::{CoreConfig, EventListener};

    struct Noop;
    impl EventListener for Noop {
        fn on_event(&self, _: Option<String>, _: CoreEvent) {}
    }

    const SHOP: &str = "receipts@shop.example";
    const BOOKS: &str = "receipts@books.example";

    fn core(name: &str) -> Arc<Core> {
        let dir = std::env::temp_dir().join(format!("kaluta-core-forward-rules-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let core = Core::new(
            CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
            Arc::new(crate::secrets::MemorySecrets::default()),
            Arc::new(Noop),
        )
        .unwrap();
        block_on(core.clone().open_account("acct".into())).unwrap();
        core
    }

    fn wait_until(mut done: impl FnMut() -> bool) -> bool {
        for _ in 0..250 {
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        false
    }

    fn rule(sender: &str, prefix: &str) -> Rule {
        Rule {
            id: 1,
            sender: sender.into(),
            sender_is_domain: !sender.contains('@'),
            subject_prefix: prefix.into(),
            recipient: BOOKS.into(),
            mode: "auto".into(),
            enabled: true,
            effective_from: 0,
            proposal_id: None,
            created_at: 0,
        }
    }

    fn edit(mode: ForwardMode) -> ForwardRuleEdit {
        ForwardRuleEdit {
            sender: SHOP.into(),
            sender_is_domain: false,
            subject_prefix: "Your receipt from Shop".into(),
            recipient: BOOKS.into(),
            mode,
            enabled: true,
        }
    }

    #[test]
    fn subjects_are_compared_bare_and_a_prefix_stops_at_what_varies() {
        assert_eq!(bare_subject("  Re: FWD: fw: Your receipt "), "Your receipt");
        assert_eq!(
            shared_prefix(&[
                "Your receipt from Shop #1042",
                "your Receipt from Shop #1043",
                "Fwd: Your receipt from Shop"
            ]),
            "Your receipt from Shop"
        );
        assert_eq!(shared_prefix(&["Receipt 2026-11-01", "Receipt 2026-11-02"]), "Receipt");
        assert_eq!(shared_prefix(&["Invoice ready", "Your receipt"]), "", "nothing shared");
        assert_eq!(shared_prefix(&["#1042 paid", "#1043 paid"]), "", "a number first");
        let long = format!("{} tail", "word ".repeat(20).trim());
        assert!(shared_prefix(&[&long, &long]).chars().count() <= PREFIX_CAP);
    }

    #[test]
    fn a_rule_is_for_its_sender_and_subject_and_never_a_subject_alone() {
        let by_address = rule(SHOP, "Your receipt");
        assert!(matches(&by_address, "Receipts@Shop.example", "Re: your receipt from Shop #7"));
        assert!(!matches(&by_address, SHOP, "Password reset"));
        assert!(
            !matches(&by_address, "attacker@evil.example", "Your receipt from Shop #7"),
            "the subject is not enough"
        );
        assert!(!matches(&by_address, "receipts@shop.example.evil.example", "Your receipt"));
        let by_domain = rule("shop.example", "Your receipt");
        assert!(matches(&by_domain, "billing@shop.example", "Your receipt"));
        assert!(!matches(&by_domain, "billing@notshop.example", "Your receipt"), "the domain exactly");
        assert!(!matches(&by_domain, "billing@pay.shop.example", "Your receipt"), "not its subdomains");
        assert!(matches(&rule(SHOP, ""), SHOP, "Anything at all"));
    }

    #[test]
    fn a_mode_is_stored_by_name_and_an_unknown_one_asks() {
        assert_eq!((ForwardMode::Automatic.stored(), ForwardMode::AskEachTime.stored()), ("auto".into(), "ask".into()));
        assert_eq!(ForwardMode::from_stored("auto"), ForwardMode::Automatic);
        assert_eq!(ForwardMode::from_stored("ask"), ForwardMode::AskEachTime);
        assert_eq!(ForwardMode::from_stored("anything else"), ForwardMode::AskEachTime, "never automatic by accident");
    }

    #[test]
    fn a_rule_must_name_a_sender_and_a_real_recipient() {
        let ok = cleaned(&ForwardRuleEdit {
            sender: "  Receipts@Shop.example ".into(),
            subject_prefix: " Re: Your receipt ".into(),
            recipient: " Receipts@Books.example".into(),
            ..edit(ForwardMode::AskEachTime)
        })
        .unwrap();
        assert_eq!(
            (ok.sender.as_str(), ok.subject_prefix.as_str(), ok.recipient.as_str()),
            (SHOP, "Your receipt", BOOKS)
        );
        let bad = |change: fn(&mut ForwardRuleEdit)| {
            let mut e = edit(ForwardMode::Automatic);
            change(&mut e);
            cleaned(&e).unwrap_err().kind()
        };
        assert_eq!(bad(|e| e.sender = String::new()), ErrorKind::InvalidInput);
        assert_eq!(bad(|e| e.sender = "shop.example".into()), ErrorKind::InvalidInput, "an address unless a domain");
        assert_eq!(bad(|e| e.recipient = "books".into()), ErrorKind::InvalidInput);
        assert_eq!(bad(|e| e.recipient = "a@b.example, c@d.example".into()), ErrorKind::InvalidInput, "one address");
        assert_eq!(
            bad(|e| {
                e.sender = "shop.example".into();
                e.sender_is_domain = true;
                e.subject_prefix = String::new();
            }),
            ErrorKind::InvalidInput,
            "a whole domain needs a subject"
        );
        let domain = cleaned(&ForwardRuleEdit {
            sender: "@Shop.example".into(),
            sender_is_domain: true,
            ..edit(ForwardMode::Automatic)
        })
        .unwrap();
        assert_eq!(domain.sender, "shop.example");
    }

    /// A received message and, when `forwarded`, the user's forward of it.
    fn receipt(n: i64, at: i64, from: &str, subject: &str, forwarded: Option<&str>) -> Vec<IncomingMessage> {
        let mail = |id: String, at: i64, from: &str, to: &str, subject: String, label: &str| IncomingMessage {
            id: mail_domain::MessageId::new(&id),
            thread_id: ThreadId::new(format!("t{n}")),
            from: Some(EmailAddress::new(None, from)),
            to: vec![EmailAddress::new(None, to)],
            subject,
            date: at,
            internal_date: at,
            label_ids: vec![LabelId::new(label)],
            ..Default::default()
        };
        let mut out = vec![mail(format!("r{n}"), at, from, "me@example.com", subject.to_owned(), "INBOX")];
        if let Some(to) = forwarded {
            out.push(mail(format!("f{n}"), at + 60_000, "me@example.com", to, format!("Fwd: {subject}"), "SENT"));
        }
        out
    }

    fn put(core: &Arc<Core>, messages: Vec<IncomingMessage>) {
        let db = core.db().unwrap();
        block_on(db.write(move |tx| {
            let mut w = MailWriter::new(tx);
            for m in &messages {
                w.upsert_message(m)?;
            }
            w.finish()
        }))
        .unwrap();
    }

    #[test]
    fn three_forwards_with_none_left_out_are_proposed_and_accepting_makes_the_rule() {
        let core = core("propose");
        let now = mail_sync::now_millis();
        let day = |n: i64| now - (10 - n) * DAY_MS;
        let subject = |n: i64| format!("Your receipt from Shop #{}", 100 + n);
        put(&core, receipt(1, day(1), SHOP, &subject(1), Some(BOOKS)));
        put(&core, receipt(2, day(2), SHOP, &subject(2), Some(BOOKS)));
        assert!(!block_on(core.watch_forwards()).unwrap(), "two are not a pattern");
        assert!(block_on(core.forward_rule_proposals()).unwrap().is_empty());

        put(&core, receipt(3, day(3), SHOP, &subject(3), Some(BOOKS)));
        // Other mail from the shop is not a receipt, and a receipt that
        // came after the last forward may not have been got to yet.
        put(&core, receipt(4, day(2), SHOP, "Your order has shipped", None));
        put(&core, receipt(5, day(5), SHOP, &subject(5), None));
        assert!(block_on(core.watch_forwards()).unwrap());
        let proposals = block_on(core.forward_rule_proposals()).unwrap();
        assert_eq!(proposals.len(), 1);
        let p = &proposals[0];
        assert_eq!(
            (p.sender.as_str(), p.subject_prefix.as_str(), p.recipient.as_str()),
            (SHOP, "Your receipt from Shop", BOOKS)
        );
        assert_eq!(p.statement, format!("Forward mail from {SHOP} starting “Your receipt from Shop” to {BOOKS}"));
        assert_eq!(p.evidence.iter().map(|e| e.message_id.as_str()).collect::<Vec<_>>(), ["r1", "r2", "r3"]);
        assert!(!block_on(core.watch_forwards()).unwrap(), "nothing new, nothing changed");
        let teach = core.db().unwrap().read_blocking(mail_store::workflows::teach_items).unwrap();
        assert!(teach.contains(&format!("forward:{}", p.id)), "it waits in Teach Kaluta");

        // One the user did not forward, before the last they did: not "always".
        put(&core, receipt(6, day(2) + 1, SHOP, &subject(6), None));
        assert!(block_on(core.watch_forwards()).unwrap());
        assert!(block_on(core.forward_rule_proposals()).unwrap().is_empty(), "withdrawn");
        // Forwarded after all: it holds again.
        put(&core, receipt(6, day(2) + 1, SHOP, &subject(6), Some(BOOKS)));
        assert!(block_on(core.watch_forwards()).unwrap());
        let p = block_on(core.forward_rule_proposals()).unwrap().remove(0);
        assert_eq!(p.evidence.len(), 4);

        // Accepting takes a mode; the rule is for mail from now on.
        let accepted = block_on(core.accept_forward_rule_proposal(p.id, edit(ForwardMode::AskEachTime))).unwrap();
        assert_eq!((accepted.mode, accepted.enabled, accepted.sent_count), (ForwardMode::AskEachTime, true, 0));
        assert!(block_on(core.forward_rule_proposals()).unwrap().is_empty());
        assert_eq!(block_on(core.forward_rules()).unwrap(), vec![accepted.clone()]);
        assert_eq!(block_on(core.apply_forward_rules()).unwrap(), (0, 0), "nothing that arrived before it");
        assert!(!block_on(core.watch_forwards()).unwrap(), "a pattern a rule covers is not proposed again");
        let again = block_on(core.accept_forward_rule_proposal(p.id, edit(ForwardMode::Automatic))).unwrap_err();
        assert_eq!(again.kind(), ErrorKind::NotFound, "decided once");

        // ⌘Z: proposed again, the rule gone.
        block_on(core.reopen_forward_rule_proposal(p.id)).unwrap();
        assert!(block_on(core.forward_rules()).unwrap().is_empty());
        assert_eq!(block_on(core.forward_rule_proposals()).unwrap().len(), 1);
        // Rejected: never raised again.
        block_on(core.reject_forward_rule_proposal(p.id)).unwrap();
        assert!(!block_on(core.watch_forwards()).unwrap());
        assert!(block_on(core.forward_rule_proposals()).unwrap().is_empty());
    }

    #[test]
    fn forwards_to_different_people_or_by_a_rule_are_no_pattern() {
        let sent = |n: i64, to: &str| SentForward {
            forward_id: format!("f{n}"),
            forwarded_at: n,
            recipient: to.into(),
            original: Received {
                message_id: format!("r{n}"),
                thread_id: format!("t{n}"),
                from_email: SHOP.into(),
                subject: format!("Your receipt #{n}"),
                at: n,
            },
        };
        let none = HashSet::new();
        let spread = [sent(1, "a@x.example"), sent(2, "b@x.example"), sent(3, "c@x.example")];
        assert!(patterns(&spread, &none, &[]).is_empty());
        let same = [sent(1, BOOKS), sent(2, BOOKS), sent(3, BOOKS), sent(3, BOOKS)];
        let found = patterns(&same, &none, &[]);
        assert_eq!((found.len(), found[0].evidence.len(), found[0].prefix.as_str()), (1, 3, "Your receipt"));
        let by_rule: HashSet<String> = ["r3".to_owned()].into();
        assert!(patterns(&same, &by_rule, &[]).is_empty(), "the rule's forward is not the user's");
        assert!(patterns(&same, &none, &[rule(SHOP, "your receipt")]).is_empty(), "already a rule");
        assert_eq!(patterns(&same, &none, &[rule(SHOP, "Invoice")]).len(), 1, "a rule for other mail");
    }

    /// A received message with its text and, perhaps, a file.
    fn arrived(id: &str, at: i64, from: &str, subject: &str, attachment: Option<&str>) -> IncomingMessage {
        IncomingMessage {
            id: mail_domain::MessageId::new(id),
            thread_id: ThreadId::new(format!("t-{id}")),
            from: Some(EmailAddress::new(None, from)),
            to: vec![EmailAddress::new(None, "me@example.com")],
            subject: subject.into(),
            date: at,
            internal_date: at,
            label_ids: vec![LabelId::new("INBOX")],
            body: Some(mail_domain::Body {
                text_plain: Some("Thanks for your order.".into()),
                html_sanitized: None,
                has_remote_images: false,
            }),
            attachments: attachment
                .map(|name| mail_store::IncomingAttachment {
                    part_id: Some("1".into()),
                    filename: name.into(),
                    mime_type: "application/pdf".into(),
                    size: 8,
                    data: Some(b"%PDF-1.4".to_vec()),
                    ..Default::default()
                })
                .into_iter()
                .collect(),
            ..Default::default()
        }
    }

    /// Who each forward in a conversation went to.
    fn forwards(core: &Arc<Core>, thread: &str) -> Vec<String> {
        let detail = block_on(core.get_thread(thread.into())).unwrap().expect("the conversation");
        detail
            .messages
            .into_iter()
            .filter(|m| m.subject.starts_with("Fwd:"))
            .flat_map(|m| m.to.into_iter().map(|a| a.email))
            .collect()
    }

    /// The rules themselves, on mail put in the store: no sync runs here.
    #[test]
    fn a_rule_asks_or_forwards_new_mail_once() {
        let core = core("apply");
        let now = mail_sync::now_millis();

        // Mail that arrived before the rule is never forwarded by it.
        put(&core, vec![arrived("old", now - 3_600_000, SHOP, "Your receipt from Shop #1", None)]);
        let rule = block_on(core.save_forward_rule(None, edit(ForwardMode::AskEachTime))).unwrap();
        put(
            &core,
            vec![
                arrived("new", now + 60_000, SHOP, "Your receipt from Shop #2", Some("receipt.pdf")),
                // The same subject from someone else, and other mail from the shop.
                arrived("forged", now + 60_000, "attacker@evil.example", "Your receipt from Shop #3", None),
                arrived("other", now + 60_000, SHOP, "Password reset", None),
            ],
        );
        assert_eq!(block_on(core.apply_forward_rules()).unwrap(), (0, 1), "asked about one, sent nothing");
        for thread in ["t-old", "t-forged", "t-other"] {
            assert_eq!(block_on(core.forward_suggestion(thread.into())).unwrap(), None, "{thread}");
        }
        assert_eq!(block_on(core.forward_suggestion_threads()).unwrap(), ["t-new"]);
        assert!(forwards(&core, "t-new").is_empty(), "asking sends nothing");

        // Not this one, and back; then taken: sent once, with the file.
        let s = block_on(core.forward_suggestion("t-new".into())).unwrap().unwrap();
        assert_eq!((s.rule_id, s.message_id.as_str(), s.recipient.as_str()), (rule.id, "new", BOOKS));
        block_on(core.dismiss_forward_suggestion(s.rule_id, s.message_id.clone())).unwrap();
        assert_eq!(block_on(core.forward_suggestion("t-new".into())).unwrap(), None);
        block_on(core.restore_forward_suggestion(s.rule_id, s.message_id.clone())).unwrap();
        block_on(core.take_forward_suggestion(s.rule_id, s.message_id.clone())).unwrap();
        // (The stored copy of sent mail lists no files; that the file goes
        // too is checked where a provider receives it, below.)
        assert_eq!(forwards(&core, "t-new"), vec![BOOKS.to_owned()], "to the rule's address, once");
        let twice = block_on(core.take_forward_suggestion(s.rule_id, s.message_id)).unwrap_err();
        assert_eq!(twice.kind(), ErrorKind::NotFound, "one answer");
        assert_eq!(block_on(core.forward_marks("t-new".into())).unwrap().len(), 1);
        assert_eq!(block_on(core.forward_rules()).unwrap()[0].sent_count, 1);

        // Made automatic: the next one goes with no one asked, and only once.
        let auto = block_on(core.save_forward_rule(Some(rule.id), edit(ForwardMode::Automatic))).unwrap();
        assert_eq!(auto.mode, ForwardMode::Automatic);
        put(&core, vec![arrived("next", now + 120_000, SHOP, "Your receipt from Shop #4", None)]);
        assert_eq!(block_on(core.apply_forward_rules()).unwrap(), (1, 0), "forwarded on its own");
        assert_eq!(block_on(core.apply_forward_rules()).unwrap(), (0, 0), "never twice");
        assert_eq!(forwards(&core, "t-next"), vec![BOOKS.to_owned()]);
        for thread in ["t-old", "t-forged", "t-other"] {
            assert!(forwards(&core, thread).is_empty(), "{thread}");
        }
        // The rule's own forwards are not evidence for another rule.
        assert!(!block_on(core.watch_forwards()).unwrap());
        assert!(block_on(core.forward_rule_proposals()).unwrap().is_empty());

        // Paused: nothing. Turned on again: only what arrives from then.
        let paused = ForwardRuleEdit { enabled: false, ..edit(ForwardMode::Automatic) };
        block_on(core.save_forward_rule(Some(rule.id), paused)).unwrap();
        put(&core, vec![arrived("paused", mail_sync::now_millis(), SHOP, "Your receipt from Shop #5", None)]);
        assert_eq!(block_on(core.apply_forward_rules()).unwrap(), (0, 0));
        std::thread::sleep(std::time::Duration::from_millis(5));
        block_on(core.save_forward_rule(Some(rule.id), edit(ForwardMode::Automatic))).unwrap();
        put(&core, vec![arrived("missed", mail_sync::now_millis() - 1_000, SHOP, "Your receipt from Shop #6", None)]);
        assert_eq!(block_on(core.apply_forward_rules()).unwrap(), (0, 0), "what came while it was off stays");
        block_on(core.delete_forward_rule(rule.id)).unwrap();
        assert!(block_on(core.forward_rules()).unwrap().is_empty());
        assert!(block_on(core.forward_marks("t-new".into())).unwrap().is_empty(), "its marks go with it");
    }

    /// The one place that needs a sync and a provider: mail it delivers is
    /// put to the rules when the round ends, with nothing else asking, and
    /// what an automatic rule sends reaches the provider with the
    /// original's file.
    #[test]
    fn a_sync_puts_new_mail_to_the_rules_and_the_forward_carries_the_file() {
        let core = core("after-sync");
        let fake = Arc::new(provider_api::fake::FakeProvider::new("me@example.com", 1_790_000_000_000, 50));
        core.start_sync_with(fake.clone()).unwrap();
        assert!(wait_until(|| block_on(core.account_address()).is_ok_and(|a| a == "me@example.com")));
        block_on(core.save_forward_rule(None, edit(ForwardMode::Automatic))).unwrap();
        fake.set_attachment_bytes("att-new", b"%PDF-1.4".to_vec());
        fake.deliver(FetchedMessage {
            id: mail_domain::MessageId::new("new"),
            thread_id: ThreadId::new("t-new"),
            label_ids: vec![LabelId::new("INBOX")],
            internal_date: mail_sync::now_millis() + 60_000,
            from: Some(EmailAddress::new(None, SHOP)),
            subject: "Your receipt from Shop #2".into(),
            body: Some(FetchedBody {
                text: Some("Thanks.".into()),
                html: None,
                attachments: vec![FetchedAttachment {
                    part_id: Some("1".into()),
                    attachment_id: Some("att-new".into()),
                    filename: "receipt.pdf".into(),
                    mime_type: "application/pdf".into(),
                    size: 8,
                    content_id: None,
                    is_inline: false,
                    data: None,
                }],
            }),
            ..Default::default()
        });
        core.sync_now();
        let sent = || fake.message(&mail_domain::MessageId::new("sent1")).and_then(|m| m.body).and_then(|b| b.text);
        assert!(wait_until(|| sent().is_some()), "forwarded when the round ended");
        let raw = sent().unwrap();
        assert!(raw.contains(BOOKS) && raw.contains("Fwd: Your receipt from Shop #2"), "{raw}");
        assert!(raw.contains("receipt.pdf"), "the attachment goes too");
        core.sync_now();
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(fake.message(&mail_domain::MessageId::new("sent2")).is_none(), "once");
        core.stop_sync();
    }

    /// ADR 0021: rules are out of agents' reach. No tool an agent can call,
    /// in a session or as a mailbox's outside agent, reads, makes or
    /// changes one; a forward an agent asks for is still its own tool,
    /// gated as before.
    #[test]
    fn no_agent_tool_reaches_a_forwarding_rule() {
        for tool in permissions::Tool::ALL {
            let name = tool.name();
            assert!(!name.contains("rule") || name.contains("guide"), "{name} must not be about forwarding rules");
        }
        assert_eq!(permissions::Tool::Forward.risk(), permissions::Risk::External, "an agent's forward still asks");
        let named = [
            "forward_rules",
            "mail_forward_rules",
            "save_forward_rule",
            "forward_rule_create",
            "accept_forward_rule_proposal",
            "settings_set",
        ];
        for name in named {
            assert!(permissions::Tool::from_name(name).is_none(), "{name}");
            assert!(agent_mcp::catalog::MailboxTool::from_name(name).is_none(), "{name}");
        }
    }

    #[test]
    fn an_automatic_rule_asks_once_it_has_forwarded_its_days_worth() {
        let core = core("cap");
        let now = mail_sync::now_millis();
        let rule = block_on(core.save_forward_rule(None, edit(ForwardMode::Automatic))).unwrap();
        let db = core.db().unwrap();
        block_on(db.write(move |tx| {
            for n in 0..DAILY_CAP {
                store::log_first(tx, rule.id, &format!("m{n}"), &format!("t{n}"), BOOKS, "sent", now)?;
            }
            Ok(())
        }))
        .unwrap();
        put(&core, receipt(99, now + 60_000, SHOP, "Your receipt from Shop #99", None));
        assert_eq!(block_on(core.apply_forward_rules()).unwrap(), (0, 1), "asked, not sent");
        assert!(block_on(core.forward_suggestion("t99".into())).unwrap().is_some());
    }
}
