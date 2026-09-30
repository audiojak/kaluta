//! The writing guide (spec §14.9, ADR 0011): the category taxonomy, the
//! entries the user has decided on, exact undo for every change (ADR 0006),
//! versions of the accepted guide, audience groups and export. Learning
//! (`guide_learn`), rendering (`guide_render`) and checks live beside it.

use std::collections::BTreeSet;

use mail_store::guide::{self as store, EntryRow, GroupRow, Snapshot};
use serde::{Deserialize, Serialize};

use crate::{Core, CoreError, CoreEvent, ErrorKind, runtime};

/// One category of the taxonomy (spec §14.9).
pub struct Category {
    pub id: &'static str,
    pub name: &'static str,
    /// What the processing function looks for, in a line.
    pub looks_for: &'static str,
    /// Sent mail can show it.
    pub learned: bool,
    /// Mail cannot show it (or not all of it): the interview asks.
    pub asked: bool,
}

const fn c(id: &'static str, name: &'static str, looks_for: &'static str, learned: bool, asked: bool) -> Category {
    Category { id, name, looks_for, learned, asked }
}

/// The groups, by letter.
pub const GROUPS: &[(&str, &str)] = &[
    ("A", "Voice and tone"),
    ("B", "Structure"),
    ("C", "Language"),
    ("D", "Audience"),
    ("E", "Message types"),
    ("F", "Content rules"),
    ("G", "Format"),
    ("H", "When unsure"),
];

/// Every category, in order. The processing function checks each batch
/// against all of them, so coverage does not depend on the agent.
pub const CATEGORIES: &[Category] = &[
    c("A1", "Overall voice", "formality, warmth, directness, confidence or hedging", true, false),
    c(
        "A2",
        "Tone by situation",
        "saying no, bad news, apologising, asking favours, chasing, thanking, disagreeing, congratulating",
        true,
        false,
    ),
    c("A3", "Humour, emoji and exclamation marks", "whether, how much and with whom", true, false),
    c(
        "A4",
        "Directness",
        "how plainly requests are stated: 'Can you send this by Friday?' or 'Would you be able to…'",
        true,
        false,
    ),
    c(
        "A5",
        "Uncertainty",
        "hedges such as 'I think', 'My understanding is'; when the user avoids sounding certain",
        true,
        false,
    ),
    c("A6", "Enthusiasm and acknowledgements", "'Great', 'Sounds good', 'Perfect', 'Love it'; how often", true, false),
    c("A7", "Personality markers", "regional words, colloquialisms, lowercase replies", true, false),
    c("B1", "Greeting", "'Hi Ann,', first name only, none in a running thread", true, false),
    c("B2", "Opening line", "straight to the point or a pleasantry; 'hope you're well' or never", true, false),
    c("B3", "Body", "answer first, paragraph length, bullets, numbered lists, bold", true, false),
    c("B4", "Length", "typical length by message type", true, false),
    c("B5", "Closing line", "a next step, 'let me know', or none", true, false),
    c("B6", "Sign-off and name", "'Best,', 'Thanks,', none; the name used", true, false),
    c("B7", "Signature block", "when a signature is included and which", true, true),
    c("B8", "Subject lines", "style for new messages; when a subject is changed", true, false),
    c("B9", "Context", "how much background comes before the point; what recipients are assumed to know", true, false),
    c("B10", "Calls to action", "where the ask goes, explicit deadlines, one clear next step", true, false),
    c("B11", "Questions", "one at a time or several; inline or bulleted; open or specific", true, false),
    c("C1", "Spelling variant", "US or UK spelling", true, false),
    c("C2", "Punctuation", "serial comma, dashes, ellipses, semicolons", true, false),
    c("C3", "Capitalisation", "product names, titles, headings", true, false),
    c("C4", "Contractions", "'I'll' or 'I will'", true, false),
    c("C5", "Numbers, dates, times and money", "forms such as '3pm PT', 'Oct 2', '$5k'", true, false),
    c("C6", "Abbreviations and jargon", "which are used, and with whom", true, false),
    c("C7", "Favoured words and phrases", "phrases the user really uses", true, false),
    c(
        "C8",
        "Things never done",
        "banned words, phrases and clichés; formatting never used; AI habits to avoid",
        true,
        true,
    ),
    c("C9", "Sentence style", "sentence length, fragments, active voice", true, false),
    c("C10", "Languages", "which language to answer in", true, false),
    c(
        "D1",
        "Audience groups",
        "the register used with colleagues, customers, investors, vendors, friends and so on",
        true,
        true,
    ),
    c(
        "D2",
        "Particular people",
        "a nickname, formality or something to remember for one person or domain",
        true,
        false,
    ),
    c("D3", "Forms of address", "first names, titles", true, false),
    c("D4", "First contact or established", "how a first or cold message differs; warm or transactional", true, false),
    c("D5", "Seniority", "writing to someone senior, a peer or a direct report", true, true),
    c("E1", "Replies", "answering inline or in a fresh note", true, false),
    c("E2", "Forwards", "the note on top: 'FYI', a summary, an ask", true, false),
    c("E3", "Introductions", "the pattern for introducing two people", true, false),
    c("E4", "Scheduling", "how times are offered, time zones, calendar links", true, true),
    c("E5", "Follow-ups and chasers", "after how long, how firm, first and later nudges", true, false),
    c("E6", "Declines", "how the user says no", true, false),
    c("E7", "Requests and delegating", "how asks are phrased, deadlines", true, false),
    c("E8", "Status updates and hand-offs", "shape and headings", true, false),
    c("E9", "Thanks and acknowledgements", "one line or more", true, false),
    c("E10", "Recipients", "reply all, who is copied, Cc or Bcc", true, false),
    c("E11", "Attachments and links", "how they are mentioned", true, false),
    c("E12", "Disagreeing and negotiating", "correcting, pushing back, negotiating", true, false),
    c("F1", "Commitments", "dates, prices or terms never promised without the user", false, true),
    c("F2", "Confidentiality", "topics and figures never mentioned, or only to some audiences", false, true),
    c("F3", "Facts about me", "role, company, phone, calendar link, time zone, working hours, pronouns", false, true),
    c("F4", "Never invent", "no made-up facts, names or figures; what to do instead", false, true),
    c("F5", "AI disclosure", "whether a message may say an AI helped", false, true),
    c("F6", "Required wording", "legal or compliance text for some audiences", false, true),
    c("G1", "Plain or rich text", "links as text or URLs, bold, lists", true, false),
    c("G2", "Quoting", "trimming quoted text, answering inline", true, false),
    c("H1", "Missing information", "ask the user, leave a [bracket], or offer options", false, true),
    c("H2", "Conflicts and precedence", "rules beat guidelines; a narrower scope beats a wider one", false, false),
    c("H3", "Model examples", "real sent messages kept as examples of a message type", false, false),
    c("H4", "Draft, send or stay silent", "when only to draft, when to ask first, when not to reply", false, true),
];

pub fn category(id: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.id.eq_ignore_ascii_case(id))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GuideKind {
    Rule,
    Guideline,
    Fact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GuideStatus {
    Proposed,
    Accepted,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum GuideSource {
    Learned,
    You,
    Merged,
}

macro_rules! strings {
    ($t:ty { $($v:ident => $s:literal),* $(,)? }) => {
        impl $t {
            pub(crate) fn as_str(self) -> &'static str { match self { $(Self::$v => $s),* } }
            pub(crate) fn parse(s: &str) -> Option<Self> {
                match s.trim().to_ascii_lowercase().as_str() { $($s => Some(Self::$v),)* _ => None }
            }
        }
    };
}
strings!(GuideKind { Rule => "rule", Guideline => "guideline", Fact => "fact" });
strings!(GuideStatus { Proposed => "proposed", Accepted => "accepted", Rejected => "rejected" });
strings!(GuideSource { Learned => "learned", You => "you", Merged => "merged" });

/// Where an entry applies; empty everywhere means always.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, uniffi::Record)]
#[serde(default)]
pub struct GuideScope {
    /// Audience group names.
    pub groups: Vec<String>,
    /// Addresses or `@domain`s.
    pub people: Vec<String>,
    /// `new`, `reply`, `forward`.
    pub message_types: Vec<String>,
    /// Language names or codes.
    pub languages: Vec<String>,
}

impl GuideScope {
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.people.is_empty() && self.message_types.is_empty() && self.languages.is_empty()
    }

    fn cleaned(mut self) -> Self {
        let clean = |v: &mut Vec<String>, lower: bool| {
            let mut seen = BTreeSet::new();
            v.retain_mut(|s| {
                *s = s.split_whitespace().collect::<Vec<_>>().join(" ");
                if lower {
                    *s = s.to_lowercase();
                }
                !s.is_empty() && seen.insert(s.to_lowercase())
            });
        };
        clean(&mut self.groups, false);
        clean(&mut self.people, true);
        clean(&mut self.message_types, true);
        clean(&mut self.languages, false);
        self.message_types.retain(|t| ["new", "reply", "forward"].contains(&t.as_str()));
        self
    }
}

/// A test the core runs on AI drafts without an agent (spec §14.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum GuideCheckKind {
    /// The draft must not contain the phrase (any case).
    BannedPhrase,
    /// The draft must contain it (a sign-off, required wording).
    RequiredPhrase,
    /// The draft's own text is at most this many words.
    MaxWords,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct GuideCheck {
    pub kind: GuideCheckKind,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideQuote {
    pub message_id: String,
    pub quote: String,
    pub contradicts: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideEntry {
    pub id: i64,
    pub category: String,
    pub kind: GuideKind,
    pub statement: String,
    pub scope: GuideScope,
    pub status: GuideStatus,
    pub source: GuideSource,
    /// Where a merged entry came from.
    pub origin: Option<String>,
    pub check: Option<GuideCheck>,
    /// Analysed messages that support it, and that go against it.
    pub support: u32,
    pub contradict: u32,
    /// For a proposal raised because mail contradicts an accepted entry.
    pub contradiction_of: Option<i64>,
    pub run_id: Option<i64>,
    pub evidence: Vec<GuideQuote>,
    pub created_at: i64,
    pub updated_at: i64,
    pub decided_at: Option<i64>,
}

/// What the user writes or changes on an entry.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideEntryFields {
    pub category: String,
    pub kind: GuideKind,
    pub statement: String,
    pub scope: GuideScope,
    pub check: Option<GuideCheck>,
}

/// One change in a set applied together (and undone together).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum GuideEdit {
    Add {
        fields: GuideEntryFields,
        status: GuideStatus,
        source: GuideSource,
        origin: Option<String>,
    },
    Update {
        id: i64,
        fields: GuideEntryFields,
    },
    /// Accept, reject or reopen as a proposal.
    Decide {
        id: i64,
        status: GuideStatus,
    },
    Delete {
        id: i64,
    },
}

/// A change applied: its id for undo and redo, the guide's version after
/// it, and the entries as they now are (deleted ones left out).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideChange {
    pub change_id: i64,
    pub version: i64,
    pub entries: Vec<GuideEntry>,
}

/// A category with how much of the guide covers it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideCategoryInfo {
    pub id: String,
    pub group: String,
    pub group_name: String,
    pub name: String,
    pub looks_for: String,
    pub learned: bool,
    pub asked: bool,
    pub accepted: u32,
    pub proposed: u32,
    /// Distinct messages behind its accepted entries.
    pub evidence: u32,
}

/// An exported guide, read for import or merging.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideImport {
    pub entries: Vec<GuideEntryFields>,
    pub groups: Vec<AudienceGroup>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GuideVersionInfo {
    pub version: i64,
    pub reason: String,
    pub created_at: i64,
    pub accepted: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AudienceStatus {
    Suggested,
    Confirmed,
    Rejected,
}
strings!(AudienceStatus { Suggested => "suggested", Confirmed => "confirmed", Rejected => "rejected" });

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AudienceGroup {
    pub id: i64,
    pub name: String,
    pub status: AudienceStatus,
    pub description: String,
    /// Addresses or `@domain`s.
    pub members: Vec<String>,
}

const STATEMENT_CAP: usize = 400;

/// Groups suggested to fill the set to `MIN_GROUPS` when the mail showed
/// fewer (spec §14.9), in order, with what each is for.
pub const GAP_GROUPS: &[(&str, &str)] = &[
    ("Colleagues", "People you work with"),
    ("Customers", "People who buy from you or use your product"),
    ("Investors", "Investors and board members"),
    ("Vendors", "Suppliers and service providers"),
    ("Candidates", "People you are hiring"),
    ("Direct reports", "People who report to you"),
    ("Advisers", "Lawyers, accountants and other advisers"),
    ("Friends and family", "Personal mail"),
    ("Strangers", "People you have not written to before"),
];

/// Audience groups the set is filled to.
pub const MIN_GROUPS: usize = 5;

/// Whether `address` belongs to a group with these members.
pub(crate) fn is_member(address: &str, members: &[String]) -> bool {
    let a = address.trim().to_lowercase();
    members.iter().any(|m| {
        let m = m.trim().to_lowercase();
        m.strip_prefix('@').map_or(a == m, |d| a.ends_with(&format!("@{d}")))
    })
}

/// Fill the groups to `MIN_GROUPS` (not counting rejected ones) from the
/// obvious gaps, each *suggested*. Returns how many were added.
pub(crate) fn fill_gaps(tx: &mail_store::Transaction<'_>) -> mail_store::StoreResult<usize> {
    let groups = store::groups(tx)?;
    let live = groups.iter().filter(|g| g.status != "rejected").count();
    let mut added = 0;
    for (name, description) in GAP_GROUPS {
        if live + added >= MIN_GROUPS {
            break;
        }
        if groups.iter().any(|g| g.name.eq_ignore_ascii_case(name)) {
            continue;
        }
        store::save_group(
            tx,
            &GroupRow {
                name: (*name).into(),
                status: "suggested".into(),
                description: (*description).into(),
                position: (groups.len() + added) as i64,
                ..Default::default()
            },
        )?;
        added += 1;
    }
    Ok(added)
}

pub(crate) fn clean_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::InvalidInput, message)
}

/// Checked and tidied fields, or why not.
pub(crate) fn validate(fields: GuideEntryFields) -> Result<GuideEntryFields, CoreError> {
    let cat = category(&fields.category).ok_or_else(|| invalid(format!("{} is not a category", fields.category)))?;
    let statement = clean_text(&fields.statement);
    if statement.is_empty() {
        return Err(invalid("an entry needs a statement"));
    }
    if statement.chars().count() > STATEMENT_CAP {
        return Err(invalid(format!("keep an entry under {STATEMENT_CAP} characters")));
    }
    let check = match fields.check {
        None => None,
        Some(GuideCheck { kind, value }) => {
            let value = clean_text(&value);
            match kind {
                _ if value.is_empty() => None,
                GuideCheckKind::MaxWords if value.parse::<u32>().map_or(true, |n| n == 0) => {
                    return Err(invalid("a word limit is a whole number"));
                }
                _ => Some(GuideCheck { kind, value }),
            }
        }
    };
    Ok(GuideEntryFields {
        category: cat.id.to_owned(),
        kind: fields.kind,
        statement,
        scope: fields.scope.cleaned(),
        check,
    })
}

fn to_row(fields: &GuideEntryFields, row: &mut EntryRow) -> Result<(), CoreError> {
    row.category = fields.category.clone();
    row.kind = fields.kind.as_str().into();
    row.statement = fields.statement.clone();
    row.scope_json = serde_json::to_string(&fields.scope).map_err(|e| invalid(e.to_string()))?;
    row.check_json = match &fields.check {
        Some(c) => Some(serde_json::to_string(c).map_err(|e| invalid(e.to_string()))?),
        None => None,
    };
    Ok(())
}

pub(crate) fn from_snapshot(s: Snapshot) -> GuideEntry {
    let e = s.entry;
    GuideEntry {
        id: e.id,
        category: e.category,
        kind: GuideKind::parse(&e.kind).unwrap_or(GuideKind::Guideline),
        statement: e.statement,
        scope: serde_json::from_str(&e.scope_json).unwrap_or_default(),
        status: GuideStatus::parse(&e.status).unwrap_or(GuideStatus::Proposed),
        source: GuideSource::parse(&e.source).unwrap_or(GuideSource::Learned),
        origin: e.origin,
        check: e.check_json.and_then(|c| serde_json::from_str(&c).ok()),
        support: e.support.max(0) as u32,
        contradict: e.contradict.max(0) as u32,
        contradiction_of: e.contradiction_of,
        run_id: e.run_id,
        evidence: s
            .evidence
            .into_iter()
            .map(|q| GuideQuote { message_id: q.message_id, quote: q.quote, contradicts: q.contradicts })
            .collect(),
        created_at: e.created_at,
        updated_at: e.updated_at,
        decided_at: e.decided_at,
    }
}

impl Core {
    /// Entries scoped to group `old` are scoped to `new` instead (or lose
    /// the group when `None`), as one undoable change; its id, if any
    /// entry was scoped to `old`.
    async fn rescope_group(&self, old: &str, new: Option<&str>) -> Result<Option<i64>, CoreError> {
        let edits: Vec<GuideEdit> = self
            .list_guide_entries(vec![])
            .await?
            .into_iter()
            .filter(|e| e.scope.groups.iter().any(|g| g.eq_ignore_ascii_case(old)))
            .map(|e| {
                let mut fields = GuideEntryFields {
                    category: e.category,
                    kind: e.kind,
                    statement: e.statement,
                    scope: e.scope,
                    check: e.check,
                };
                fields.scope.groups.retain(|g| !g.eq_ignore_ascii_case(old));
                if let Some(new) = new
                    && !fields.scope.groups.iter().any(|g| g.eq_ignore_ascii_case(new))
                {
                    fields.scope.groups.push(new.to_owned());
                }
                GuideEdit::Update { id: e.id, fields }
            })
            .collect();
        if edits.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.apply_edits(edits, format!("group {old}")).await?.change_id))
    }

    pub(crate) fn guide_changed(&self) {
        self.account_events().emit(CoreEvent::GuideChanged);
    }

    /// Apply edits as one change in one transaction: recorded for exact
    /// undo, and a new version when the accepted guide changed.
    pub(crate) async fn apply_edits(&self, edits: Vec<GuideEdit>, reason: String) -> Result<GuideChange, CoreError> {
        if edits.is_empty() {
            return Err(invalid("nothing to change"));
        }
        // Validate before touching the store.
        let mut checked = Vec::with_capacity(edits.len());
        for edit in edits {
            checked.push(match edit {
                GuideEdit::Add { fields, status, source, origin } => {
                    GuideEdit::Add { fields: validate(fields)?, status, source, origin }
                }
                GuideEdit::Update { id, fields } => GuideEdit::Update { id, fields: validate(fields)? },
                other => other,
            });
        }
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let result = runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    let mut ids: Vec<i64> = Vec::new();
                    let mut before: Vec<Snapshot> = Vec::new();
                    for edit in &checked {
                        let id = match edit {
                            GuideEdit::Add { fields, status, source, origin } => {
                                let mut row = EntryRow {
                                    status: status.as_str().into(),
                                    source: source.as_str().into(),
                                    origin: origin.clone(),
                                    created_at: now,
                                    updated_at: now,
                                    decided_at: (*status != GuideStatus::Proposed).then_some(now),
                                    ..Default::default()
                                };
                                to_row(fields, &mut row).map_err(|e| mail_store::StoreError::Invalid(e.to_string()))?;
                                store::insert_entry(tx, &row)?
                            }
                            GuideEdit::Update { id, .. } | GuideEdit::Decide { id, .. } | GuideEdit::Delete { id } => {
                                // The state before the change's first edit to it.
                                if !ids.contains(id)
                                    && let Some(s) = store::snapshot(tx, *id)?
                                {
                                    before.push(s);
                                }
                                *id
                            }
                        };
                        if !ids.contains(&id) {
                            ids.push(id);
                        }
                        match edit {
                            GuideEdit::Add { .. } => {}
                            GuideEdit::Update { id, fields } => {
                                let mut row = store::get_entry(tx, *id)?.ok_or_else(|| {
                                    mail_store::StoreError::Invalid("that entry no longer exists".into())
                                })?;
                                to_row(fields, &mut row).map_err(|e| mail_store::StoreError::Invalid(e.to_string()))?;
                                row.updated_at = now;
                                store::write_entry(tx, &row)?;
                            }
                            GuideEdit::Decide { id, status } => {
                                let mut row = store::get_entry(tx, *id)?.ok_or_else(|| {
                                    mail_store::StoreError::Invalid("that entry no longer exists".into())
                                })?;
                                row.status = status.as_str().into();
                                row.decided_at = (*status != GuideStatus::Proposed).then_some(now);
                                row.updated_at = now;
                                store::write_entry(tx, &row)?;
                            }
                            GuideEdit::Delete { id } => {
                                store::delete_entry(tx, *id)?;
                            }
                        }
                    }
                    let mut after = Vec::new();
                    for id in &ids {
                        if let Some(s) = store::snapshot(tx, *id)? {
                            after.push(s);
                        }
                    }
                    let change_id = store::record_change(tx, &reason, &before, &after, now)?;
                    let accepted = |v: &[Snapshot]| v.iter().any(|s| s.entry.status == GuideStatus::Accepted.as_str());
                    let version = if accepted(&before) || accepted(&after) {
                        store::record_version(tx, &reason, now)?
                    } else {
                        store::current_version(tx)?
                    };
                    Ok(GuideChange { change_id, version, entries: after.into_iter().map(from_snapshot).collect() })
                })
                .await?)
        })
        .await?;
        self.guide_changed();
        Ok(result)
    }

    /// Put a recorded change's entries back as they were before it (undo)
    /// or after it (redo).
    async fn replay_change(&self, change_id: i64, undo: bool) -> Result<(), CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            db.write(move |tx| {
                let (reason, before, after) = store::get_change(tx, change_id)?
                    .ok_or_else(|| mail_store::StoreError::Invalid("that change can no longer be undone".into()))?;
                let mut ids: Vec<i64> = before.iter().chain(&after).map(|s| s.entry.id).collect();
                ids.sort_unstable();
                ids.dedup();
                store::restore(tx, &ids, if undo { &before } else { &after })?;
                store::record_version(tx, &format!("{} {reason}", if undo { "undo" } else { "redo" }), now)?;
                Ok(())
            })
            .await?;
            Ok(())
        })
        .await?;
        self.guide_changed();
        Ok(())
    }
}

fn group_info(g: GroupRow) -> AudienceGroup {
    AudienceGroup {
        id: g.id,
        name: g.name,
        status: AudienceStatus::parse(&g.status).unwrap_or(AudienceStatus::Suggested),
        description: g.description,
        members: g.members,
    }
}

/// The guide as Markdown, accepted entries by category (and the people and
/// groups they name), for reading or sharing.
pub(crate) fn markdown(entries: &[GuideEntry], groups: &[AudienceGroup], with_evidence: bool) -> String {
    let mut out = String::from("# Writing guide\n");
    let confirmed: Vec<&AudienceGroup> = groups.iter().filter(|g| g.status == AudienceStatus::Confirmed).collect();
    if !confirmed.is_empty() {
        out.push_str("\n## Audiences\n\n");
        for g in confirmed {
            out.push_str(&format!("- **{}**", g.name));
            if !g.members.is_empty() {
                out.push_str(&format!(": {}", g.members.join(", ")));
            }
            out.push('\n');
        }
    }
    for (letter, group_name) in GROUPS {
        let in_group: Vec<&GuideEntry> =
            entries.iter().filter(|e| e.status == GuideStatus::Accepted && e.category.starts_with(letter)).collect();
        if in_group.is_empty() {
            continue;
        }
        out.push_str(&format!("\n## {group_name}\n"));
        for cat in CATEGORIES.iter().filter(|c| c.id.starts_with(letter)) {
            let here: Vec<&&GuideEntry> = in_group.iter().filter(|e| e.category == cat.id).collect();
            if here.is_empty() {
                continue;
            }
            out.push_str(&format!("\n### {} {}\n\n", cat.id, cat.name));
            for e in here {
                let kind = match e.kind {
                    GuideKind::Rule => "Rule",
                    GuideKind::Guideline => "Guideline",
                    GuideKind::Fact => "Fact",
                };
                out.push_str(&format!("- **{kind}.** {}", e.statement));
                let scope = scope_text(&e.scope);
                if !scope.is_empty() {
                    out.push_str(&format!(" *({scope})*"));
                }
                out.push('\n');
                if with_evidence {
                    for q in e.evidence.iter().filter(|q| !q.contradicts).take(3) {
                        out.push_str(&format!("  - “{}”\n", q.quote));
                    }
                }
            }
        }
    }
    out
}

pub(crate) fn scope_text(s: &GuideScope) -> String {
    let mut parts = Vec::new();
    if !s.groups.is_empty() {
        parts.push(format!("for {}", s.groups.join(", ")));
    }
    if !s.people.is_empty() {
        parts.push(format!("to {}", s.people.join(", ")));
    }
    if !s.message_types.is_empty() {
        parts.push(format!("in {}", s.message_types.join(", ")));
    }
    if !s.languages.is_empty() {
        // The draft's language is not known ahead: the agent applies these
        // when it writes in one of them.
        parts.push(format!("when writing in {}", s.languages.join(" or ")));
    }
    parts.join("; ")
}

/// The guide as JSON for import and merging: entries without evidence
/// (quotes are another account's mail, ADR 0004) and the groups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExportFile {
    pub format: String,
    pub version: u32,
    pub entries: Vec<ExportEntry>,
    #[serde(default)]
    pub groups: Vec<ExportGroup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExportEntry {
    pub category: String,
    pub kind: String,
    pub statement: String,
    #[serde(default)]
    pub scope: GuideScope,
    #[serde(default)]
    pub check: Option<GuideCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExportGroup {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub members: Vec<String>,
}

pub(crate) const EXPORT_FORMAT: &str = "openagc-writing-guide";

#[uniffi::export]
impl Core {
    /// Every category with how much of the guide covers it.
    pub async fn guide_categories(&self) -> Result<Vec<GuideCategoryInfo>, CoreError> {
        let entries = self.list_guide_entries(vec![]).await?;
        Ok(CATEGORIES
            .iter()
            .map(|cat| {
                let here = entries.iter().filter(|e| e.category == cat.id);
                let accepted: Vec<&GuideEntry> = here.clone().filter(|e| e.status == GuideStatus::Accepted).collect();
                let messages: BTreeSet<&str> = accepted
                    .iter()
                    .flat_map(|e| e.evidence.iter().filter(|q| !q.contradicts).map(|q| q.message_id.as_str()))
                    .collect();
                let group = &cat.id[..1];
                GuideCategoryInfo {
                    id: cat.id.into(),
                    group: group.into(),
                    group_name: GROUPS.iter().find(|(l, _)| *l == group).map(|(_, n)| (*n).into()).unwrap_or_default(),
                    name: cat.name.into(),
                    looks_for: cat.looks_for.into(),
                    learned: cat.learned,
                    asked: cat.asked,
                    accepted: accepted.len() as u32,
                    proposed: here.filter(|e| e.status == GuideStatus::Proposed).count() as u32,
                    evidence: messages.len() as u32,
                }
            })
            .collect())
    }

    /// Entries with any of `statuses` (every entry when empty), with their
    /// evidence, by category.
    pub async fn list_guide_entries(&self, statuses: Vec<GuideStatus>) -> Result<Vec<GuideEntry>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let wanted: Vec<&str> = statuses.iter().map(|s| s.as_str()).collect();
                    let mut out = Vec::new();
                    for row in store::list_entries(c, &wanted)? {
                        let evidence = store::evidence(c, row.id)?;
                        out.push(from_snapshot(Snapshot { entry: row, evidence }));
                    }
                    Ok(out)
                })
                .await?)
        })
        .await
    }

    pub async fn guide_entry(&self, id: i64) -> Result<Option<GuideEntry>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(move |c| store::snapshot(c, id)).await?.map(from_snapshot)) }).await
    }

    /// Apply edits as one change (for Undo, one step): the user's own
    /// edits, decisions, interview answers, changes by prompt and merges.
    pub async fn apply_guide_edits(&self, edits: Vec<GuideEdit>, reason: String) -> Result<GuideChange, CoreError> {
        self.apply_edits(edits, reason).await
    }

    /// Undo a change exactly: its entries go back to how they were.
    pub async fn undo_guide_change(&self, change_id: i64) -> Result<(), CoreError> {
        self.replay_change(change_id, true).await
    }

    pub async fn redo_guide_change(&self, change_id: i64) -> Result<(), CoreError> {
        self.replay_change(change_id, false).await
    }

    /// The accepted guide's current version (0 before any).
    pub async fn guide_version(&self) -> Result<i64, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(store::current_version).await?) }).await
    }

    pub async fn guide_versions(&self, limit: u32) -> Result<Vec<GuideVersionInfo>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| store::versions(c, limit))
                .await?
                .into_iter()
                .map(|(version, reason, created_at, n)| GuideVersionInfo {
                    version,
                    reason,
                    created_at,
                    accepted: n as u32,
                })
                .collect())
        })
        .await
    }

    pub async fn list_audience_groups(&self) -> Result<Vec<AudienceGroup>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(store::groups).await?.into_iter().map(group_info).collect()) }).await
    }

    /// Add or update a group by name; its members are replaced.
    pub async fn save_audience_group(&self, group: AudienceGroup) -> Result<Vec<AudienceGroup>, CoreError> {
        let name = clean_text(&group.name);
        if name.is_empty() {
            return Err(invalid("a group needs a name"));
        }
        let db = self.db()?;
        let row = GroupRow {
            id: group.id,
            name,
            status: group.status.as_str().into(),
            description: clean_text(&group.description),
            position: 0,
            members: group.members.iter().map(|m| clean_text(m)).filter(|m| !m.is_empty()).collect(),
        };
        let groups = runtime::run(async move {
            db.write(move |tx| store::save_group(tx, &row).map(|_| ())).await?;
            Ok(db.read(store::groups).await?.into_iter().map(group_info).collect())
        })
        .await?;
        self.guide_changed();
        Ok(groups)
    }

    /// Rename a group; entries scoped to it follow (one undoable change).
    pub async fn rename_audience_group(&self, id: i64, name: String) -> Result<Vec<AudienceGroup>, CoreError> {
        let name = clean_text(&name);
        if name.is_empty() {
            return Err(invalid("a group needs a name"));
        }
        let groups = self.list_audience_groups().await?;
        let old =
            groups.iter().find(|g| g.id == id).ok_or_else(|| CoreError::new(ErrorKind::NotFound, "no such group"))?;
        if groups.iter().any(|g| g.id != id && g.name.eq_ignore_ascii_case(&name)) {
            return Err(invalid(format!("there is already a group called {name}; merge them instead")));
        }
        let old_name = old.name.clone();
        self.rescope_group(&old_name, Some(&name)).await?;
        let db = self.db()?;
        runtime::run(async move { Ok(db.write(move |tx| store::rename_group(tx, id, &name)).await?) }).await?;
        self.guide_changed();
        self.list_audience_groups().await
    }

    /// Merge `from` into `into`: its members join, entries scoped to it are
    /// scoped to `into`, and `from` goes. Returns the change that re-scoped
    /// the entries, for the app's Undo (with the two groups as they were).
    pub async fn merge_audience_groups(&self, into: i64, from: i64) -> Result<Option<i64>, CoreError> {
        let groups = self.list_audience_groups().await?;
        let find = |id| groups.iter().find(|g: &&AudienceGroup| g.id == id).cloned();
        let (Some(target), Some(source)) = (find(into), find(from)) else {
            return Err(CoreError::new(ErrorKind::NotFound, "no such group"));
        };
        if into == from {
            return Err(invalid("a group cannot merge into itself"));
        }
        let change = self.rescope_group(&source.name, Some(&target.name)).await?;
        let mut members = target.members.clone();
        for m in source.members {
            if !members.contains(&m) {
                members.push(m);
            }
        }
        self.save_audience_group(AudienceGroup { members, ..target }).await?;
        self.delete_audience_group(from).await?;
        Ok(change)
    }

    /// Suggest groups for the obvious gaps until there are five.
    pub async fn fill_audience_groups(&self) -> Result<Vec<AudienceGroup>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.write(fill_gaps).await?) }).await?;
        self.guide_changed();
        self.list_audience_groups().await
    }

    /// The confirmed groups these recipients belong to, in group order
    /// (what scopes guidelines for a message).
    pub async fn audience_for(&self, addresses: Vec<String>) -> Result<Vec<String>, CoreError> {
        Ok(self
            .list_audience_groups()
            .await?
            .into_iter()
            .filter(|g| g.status == AudienceStatus::Confirmed)
            .filter(|g| addresses.iter().any(|a| is_member(a, &g.members)))
            .map(|g| g.name)
            .collect())
    }

    pub async fn delete_audience_group(&self, id: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.write(move |tx| store::delete_group(tx, id).map(|_| ())).await?) }).await?;
        self.guide_changed();
        Ok(())
    }

    /// Keep a sent message as a model of its type (H3), or with `None`
    /// stop keeping it.
    pub async fn set_guide_example(&self, message_id: String, message_type: Option<String>) -> Result<(), CoreError> {
        if let Some(t) = &message_type
            && !["new", "reply", "forward"].contains(&t.as_str())
        {
            return Err(invalid("a model example is a new message, a reply or a forward"));
        }
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db.write(move |tx| store::set_example(tx, &message_id, message_type.as_deref(), now)).await?)
        })
        .await?;
        self.guide_changed();
        Ok(())
    }

    pub async fn guide_examples(&self) -> Result<Vec<Vec<String>>, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(store::examples).await?.into_iter().map(|(m, t)| vec![m, t]).collect()) })
            .await
    }

    /// Read an exported guide (JSON) without changing anything: its entries
    /// and groups, for a look before importing or merging.
    pub fn read_guide_export(&self, json: String) -> Result<GuideImport, CoreError> {
        let (entries, groups) = read_export(&json)?;
        Ok(GuideImport {
            entries,
            groups: groups
                .into_iter()
                .map(|g| AudienceGroup {
                    id: 0,
                    name: g.name,
                    status: AudienceStatus::Suggested,
                    description: g.description,
                    members: g.members,
                })
                .collect(),
        })
    }

    /// The accepted guide as Markdown (to read or share) or JSON (to import
    /// or merge into another account; never with evidence).
    pub async fn export_guide(&self, json: bool, with_evidence: bool) -> Result<String, CoreError> {
        let entries = self.list_guide_entries(vec![GuideStatus::Accepted]).await?;
        let groups = self.list_audience_groups().await?;
        if !json {
            return Ok(markdown(&entries, &groups, with_evidence));
        }
        let file = ExportFile {
            format: EXPORT_FORMAT.into(),
            version: 1,
            entries: entries
                .into_iter()
                .map(|e| ExportEntry {
                    category: e.category,
                    kind: e.kind.as_str().into(),
                    statement: e.statement,
                    scope: e.scope,
                    check: e.check,
                })
                .collect(),
            groups: groups
                .into_iter()
                .filter(|g| g.status == AudienceStatus::Confirmed)
                .map(|g| ExportGroup { name: g.name, description: g.description, members: g.members })
                .collect(),
        };
        serde_json::to_string_pretty(&file).map_err(|e| invalid(e.to_string()))
    }
}

/// Read an exported guide: its entries as fields (unknown categories and
/// empty statements skipped) and its groups.
pub(crate) fn read_export(json: &str) -> Result<(Vec<GuideEntryFields>, Vec<ExportGroup>), CoreError> {
    let file: ExportFile =
        serde_json::from_str(json).map_err(|_| invalid("that file is not an exported OpenAGC writing guide"))?;
    if file.format != EXPORT_FORMAT {
        return Err(invalid("that file is not an exported OpenAGC writing guide"));
    }
    let entries = file
        .entries
        .into_iter()
        .filter_map(|e| {
            validate(GuideEntryFields {
                category: e.category,
                kind: GuideKind::parse(&e.kind).unwrap_or(GuideKind::Guideline),
                statement: e.statement,
                scope: e.scope,
                check: e.check,
            })
            .ok()
        })
        .collect();
    Ok((entries, file.groups))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use futures::executor::block_on;

    use super::*;
    use crate::{CoreConfig, EventListener};

    struct Noop;
    impl EventListener for Noop {
        fn on_event(&self, _: Option<String>, _: CoreEvent) {}
    }

    pub(crate) struct Scratch(pub std::path::PathBuf, pub Arc<Core>);
    impl Drop for Scratch {
        fn drop(&mut self) {
            self.1.stop_sync();
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(crate) fn demo(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("openagc-core-guide-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let core = Core::new(
            CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
            Arc::new(crate::secrets::MemorySecrets::default()),
            Arc::new(Noop),
        )
        .unwrap();
        block_on(core.clone().open_account("demo".into())).unwrap();
        Scratch(dir, core)
    }

    fn fields(category: &str, statement: &str) -> GuideEntryFields {
        GuideEntryFields {
            category: category.into(),
            kind: GuideKind::Guideline,
            statement: statement.into(),
            scope: GuideScope::default(),
            check: None,
        }
    }

    #[test]
    fn the_taxonomy_is_complete_and_unique() {
        assert_eq!(CATEGORIES.len(), 57);
        let ids: BTreeSet<&str> = CATEGORIES.iter().map(|c| c.id).collect();
        assert_eq!(ids.len(), CATEGORIES.len());
        for (letter, _) in GROUPS {
            assert!(CATEGORIES.iter().any(|c| c.id.starts_with(letter)));
        }
        assert!(category("b6").is_some() && category("Z1").is_none());
        assert!(CATEGORIES.iter().filter(|c| c.id.starts_with('F')).all(|c| c.asked && !c.learned));
    }

    #[test]
    fn edits_apply_as_one_change_and_undo_exactly() {
        let s = demo("edits");
        let core = &s.1;
        let first = block_on(core.apply_guide_edits(
            vec![
                GuideEdit::Add {
                    fields: fields("b6", "  Sign off with 'John'.  "),
                    status: GuideStatus::Accepted,
                    source: GuideSource::You,
                    origin: None,
                },
                GuideEdit::Add {
                    fields: GuideEntryFields {
                        check: Some(GuideCheck { kind: GuideCheckKind::BannedPhrase, value: " circle back ".into() }),
                        kind: GuideKind::Rule,
                        ..fields("C8", "Never write 'circle back'")
                    },
                    status: GuideStatus::Proposed,
                    source: GuideSource::Learned,
                    origin: None,
                },
            ],
            "interview".into(),
        ))
        .unwrap();
        assert_eq!(first.entries.len(), 2);
        assert_eq!(first.entries[0].category, "B6");
        assert_eq!(first.entries[0].statement, "Sign off with 'John'.");
        assert_eq!(first.entries[1].check.as_ref().unwrap().value, "circle back");
        assert_eq!(first.version, 1, "the accepted guide changed");
        let rule = first.entries[1].id;

        let decided =
            block_on(core.apply_guide_edits(
                vec![GuideEdit::Decide { id: rule, status: GuideStatus::Accepted }],
                "decide".into(),
            ))
            .unwrap();
        assert_eq!(decided.version, 2);
        assert_eq!(block_on(core.list_guide_entries(vec![GuideStatus::Accepted])).unwrap().len(), 2);

        block_on(core.undo_guide_change(decided.change_id)).unwrap();
        assert_eq!(block_on(core.guide_entry(rule)).unwrap().unwrap().status, GuideStatus::Proposed);
        block_on(core.redo_guide_change(decided.change_id)).unwrap();
        assert_eq!(block_on(core.guide_entry(rule)).unwrap().unwrap().status, GuideStatus::Accepted);

        // Undoing the first change removes both entries it added.
        block_on(core.undo_guide_change(first.change_id)).unwrap();
        assert!(block_on(core.list_guide_entries(vec![])).unwrap().is_empty());
        block_on(core.redo_guide_change(first.change_id)).unwrap();
        assert_eq!(block_on(core.list_guide_entries(vec![])).unwrap().len(), 2);

        let cats = block_on(core.guide_categories()).unwrap();
        assert_eq!(cats.len(), 57);
        let b6 = cats.iter().find(|c| c.id == "B6").unwrap();
        assert_eq!((b6.accepted, b6.group_name.as_str()), (1, "Structure"));
    }

    #[test]
    fn bad_edits_change_nothing() {
        let s = demo("invalid");
        let core = &s.1;
        let bad = vec![
            GuideEdit::Add {
                fields: fields("A1", "Warm and direct"),
                status: GuideStatus::Accepted,
                source: GuideSource::You,
                origin: None,
            },
            GuideEdit::Add {
                fields: fields("Z9", "Nope"),
                status: GuideStatus::Accepted,
                source: GuideSource::You,
                origin: None,
            },
        ];
        assert_eq!(block_on(core.apply_guide_edits(bad, "x".into())).unwrap_err().kind(), ErrorKind::InvalidInput);
        assert!(block_on(core.list_guide_entries(vec![])).unwrap().is_empty(), "validated before writing");
        let too_long = GuideEdit::Add {
            fields: fields("A1", &"word ".repeat(100)),
            status: GuideStatus::Accepted,
            source: GuideSource::You,
            origin: None,
        };
        assert!(block_on(core.apply_guide_edits(vec![too_long], "x".into())).is_err());
        let words = GuideEdit::Add {
            fields: GuideEntryFields {
                check: Some(GuideCheck { kind: GuideCheckKind::MaxWords, value: "many".into() }),
                ..fields("B4", "Keep replies short")
            },
            status: GuideStatus::Accepted,
            source: GuideSource::You,
            origin: None,
        };
        assert!(block_on(core.apply_guide_edits(vec![words], "x".into())).is_err());
        assert!(block_on(core.apply_guide_edits(vec![GuideEdit::Delete { id: 99 }], "x".into())).is_ok());
    }

    #[test]
    fn the_guide_exports_and_reads_back_without_evidence() {
        let s = demo("export");
        let core = &s.1;
        block_on(core.save_audience_group(AudienceGroup {
            id: 0,
            name: "Customers".into(),
            status: AudienceStatus::Confirmed,
            description: "People who buy from us".into(),
            members: vec!["@acme.com".into()],
        }))
        .unwrap();
        block_on(core.apply_guide_edits(
            vec![GuideEdit::Add {
                fields: GuideEntryFields {
                    scope: GuideScope { groups: vec!["Customers".into()], ..Default::default() },
                    ..fields("A1", "Be warm but brief")
                },
                status: GuideStatus::Accepted,
                source: GuideSource::You,
                origin: None,
            }],
            "you".into(),
        ))
        .unwrap();
        let md = block_on(core.export_guide(false, false)).unwrap();
        assert!(md.contains("## Voice and tone") && md.contains("### A1 Overall voice"));
        assert!(md.contains("- **Guideline.** Be warm but brief *(for Customers)*"));
        assert!(md.contains("**Customers**: @acme.com"));
        let json = block_on(core.export_guide(true, true)).unwrap();
        assert!(!json.contains("evidence"));
        let read = core.read_guide_export(json).unwrap();
        assert_eq!(read.entries[0].statement, "Be warm but brief");
        assert_eq!(read.groups[0].name, "Customers");
        assert!(read_export("{\"format\":\"other\",\"version\":1,\"entries\":[]}").is_err());
        assert!(read_export("not json").is_err());
    }

    #[test]
    fn groups_fill_to_five_rename_merge_and_scope_messages() {
        let s = demo("groups");
        let core = &s.1;
        let group = |name: &str, status, members: &[&str]| AudienceGroup {
            id: 0,
            name: name.into(),
            status,
            description: String::new(),
            members: members.iter().map(|m| (*m).to_owned()).collect(),
        };
        block_on(core.save_audience_group(group("Team", AudienceStatus::Confirmed, &["@actual.ai"]))).unwrap();
        block_on(core.save_audience_group(group("Customers", AudienceStatus::Rejected, &[]))).unwrap();
        let filled = block_on(core.fill_audience_groups()).unwrap();
        let live: Vec<&AudienceGroup> = filled.iter().filter(|g| g.status != AudienceStatus::Rejected).collect();
        assert_eq!(live.len(), MIN_GROUPS);
        assert!(live.iter().skip(1).all(|g| g.status == AudienceStatus::Suggested));
        assert!(!live.iter().any(|g| g.name == "Customers"), "a rejected group is not suggested again");
        assert_eq!(block_on(core.fill_audience_groups()).unwrap().len(), filled.len(), "nothing more to fill");

        // An entry scoped to Team follows a rename and a merge.
        block_on(core.apply_guide_edits(
            vec![GuideEdit::Add {
                fields: GuideEntryFields {
                    scope: GuideScope { groups: vec!["Team".into()], ..Default::default() },
                    ..fields("A1", "Be brief with the team")
                },
                status: GuideStatus::Accepted,
                source: GuideSource::You,
                origin: None,
            }],
            "x".into(),
        ))
        .unwrap();
        let team = filled.iter().find(|g| g.name == "Team").unwrap().id;
        let groups = block_on(core.rename_audience_group(team, "Coworkers".into())).unwrap();
        assert!(groups.iter().any(|g| g.name == "Coworkers" && g.id == team));
        let entry = &block_on(core.list_guide_entries(vec![GuideStatus::Accepted])).unwrap()[0];
        assert_eq!(entry.scope.groups, ["Coworkers"]);
        let other = groups.iter().find(|g| g.name == "Vendors").unwrap().id;
        block_on(core.save_audience_group(group("Vendors", AudienceStatus::Confirmed, &["ann@x.com"]))).unwrap();
        assert!(block_on(core.rename_audience_group(other, "coworkers".into())).is_err(), "names are unique");
        assert_eq!(block_on(core.merge_audience_groups(team, other)).unwrap(), None, "no entry was scoped to Vendors");
        let merged = block_on(core.list_audience_groups()).unwrap();
        let colleagues = merged.iter().find(|g| g.id == team).unwrap();
        assert_eq!(colleagues.members, ["@actual.ai", "ann@x.com"]);
        assert!(!merged.iter().any(|g| g.id == other));

        assert_eq!(block_on(core.audience_for(vec!["Bob@Actual.ai".into()])).unwrap(), ["Coworkers"]);
        assert!(block_on(core.audience_for(vec!["x@elsewhere.com".into()])).unwrap().is_empty());
    }

    #[test]
    fn examples_are_kept_by_type() {
        let s = demo("examples");
        let core = &s.1;
        block_on(core.set_guide_example("m1".into(), Some("reply".into()))).unwrap();
        assert!(block_on(core.set_guide_example("m2".into(), Some("memo".into()))).is_err());
        assert_eq!(block_on(core.guide_examples()).unwrap(), vec![vec!["m1".to_owned(), "reply".to_owned()]]);
        block_on(core.set_guide_example("m1".into(), None)).unwrap();
        assert!(block_on(core.guide_examples()).unwrap().is_empty());
    }
}
