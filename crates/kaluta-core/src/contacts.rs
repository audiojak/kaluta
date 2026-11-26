//! Contacts and contact facts (spec §14.16, ADR 0026): the people the
//! user corresponds with and what is known about each (facts, open topics,
//! commitments and tags). This is the model and the user's own edits;
//! learning from mail is `contacts_learn`. Every edit is one change with
//! its rows before and after, undone and redone exactly (ADR 0006). Per
//! account: nothing here is in the global facts store or published.

use mail_store::contact_facts::{self as store, ContactEvidence, ContactFactRow, PersonRow, Snapshot, TagRow, Touched};
use mail_store::{StoreError, StoreResult, Transaction};

use crate::facts::FactUse;
use crate::{Core, CoreError, CoreEvent, ErrorKind, runtime};

const DAY_MS: i64 = 86_400_000;
/// An open topic with no mail on it for this long is stale.
pub const TOPIC_STALE_MS: i64 = 90 * DAY_MS;
/// An open commitment this long past its due day is stale.
pub const COMMITMENT_STALE_MS: i64 = 30 * DAY_MS;
/// A fact older than this is flagged for review.
pub const FACT_AGED_MS: i64 = 365 * DAY_MS;
/// An address joined to a contact more recently than this is distrusted
/// by the review gate.
pub const NEWLY_JOINED_MS: i64 = 30 * DAY_MS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ContactFactKind {
    Fact,
    Topic,
    Commitment,
    Tag,
}

impl ContactFactKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Topic => "topic",
            Self::Commitment => "commitment",
            Self::Tag => "tag",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "fact" => Self::Fact,
            "topic" => Self::Topic,
            "commitment" => Self::Commitment,
            "tag" => Self::Tag,
            _ => return None,
        })
    }
}

/// What an item concerns, from a fixed list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ContactFactClass {
    Identity,
    Role,
    ContactDetail,
    Preference,
    Personal,
    Topic,
    Money,
    /// Who or where to send something.
    Routing,
    Agreement,
    Deadline,
    Other,
}

impl ContactFactClass {
    pub const ALL: [Self; 11] = [
        Self::Identity,
        Self::Role,
        Self::ContactDetail,
        Self::Preference,
        Self::Personal,
        Self::Topic,
        Self::Money,
        Self::Routing,
        Self::Agreement,
        Self::Deadline,
        Self::Other,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Role => "role",
            Self::ContactDetail => "contact_detail",
            Self::Preference => "preference",
            Self::Personal => "personal",
            Self::Topic => "topic",
            Self::Money => "money",
            Self::Routing => "routing",
            Self::Agreement => "agreement",
            Self::Deadline => "deadline",
            Self::Other => "other",
        }
    }

    /// As stored or as an agent wrote it; anything unknown is `Other`.
    pub(crate) fn parse(s: &str) -> Self {
        let s = s.trim().to_lowercase().replace([' ', '-'], "_");
        Self::ALL.into_iter().find(|c| c.as_str() == s).unwrap_or(Self::Other)
    }

    /// The name shown ("Contact detail").
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Identity => "Identity",
            Self::Role => "Role",
            Self::ContactDetail => "Contact detail",
            Self::Preference => "Preference",
            Self::Personal => "Personal",
            Self::Topic => "Topic",
            Self::Money => "Money",
            Self::Routing => "Routing",
            Self::Agreement => "Agreement",
            Self::Deadline => "Deadline",
            Self::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ContactParty {
    You,
    Them,
}

impl ContactParty {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::You => "you",
            Self::Them => "them",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "you" | "me" | "user" => Some(Self::You),
            "them" | "they" | "contact" => Some(Self::Them),
            _ => None,
        }
    }
}

/// Who said a quote, decided by code from the message's sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum SaidBy {
    You,
    Them,
    /// Another person in the conversation.
    SomeoneElse,
}

impl SaidBy {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::You => "you",
            Self::Them => "them",
            Self::SomeoneElse => "other",
        }
    }

    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "you" => Self::You,
            "them" => Self::Them,
            _ => Self::SomeoneElse,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ContactFactStatus {
    /// Seen once: in no prompt and not on the page.
    Watching,
    /// Held for the user's review.
    Proposed,
    Accepted,
    Rejected,
}

impl ContactFactStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Watching => "watching",
            Self::Proposed => "proposed",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "watching" => Self::Watching,
            "proposed" => Self::Proposed,
            "rejected" => Self::Rejected,
            _ => Self::Accepted,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ContactFactSource {
    /// Written or edited by the user.
    You,
    Learned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum ContactTagSet {
    Relationship,
    Role,
}

impl ContactTagSet {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Relationship => "relationship",
            Self::Role => "role",
        }
    }

    pub(crate) fn parse(s: &str) -> Self {
        if s == "role" { Self::Role } else { Self::Relationship }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactQuote {
    pub message_id: String,
    /// The conversation it was said in, while that message is stored.
    pub thread_id: Option<String>,
    pub quote: String,
    pub said_by: SaidBy,
    pub message_date: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactTagInfo {
    /// Never changes; a rename keeps it.
    pub id: String,
    pub name: String,
    pub set: ContactTagSet,
    pub heading: String,
    pub builtin: bool,
    /// How many contacts carry it (accepted).
    pub contacts: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactFactInfo {
    pub id: i64,
    pub person_id: i64,
    pub kind: ContactFactKind,
    /// Empty for a tag.
    pub label: String,
    /// For a tag, its name as it is now.
    pub value: String,
    /// The tag, for an item of kind tag (`None` when the tag is gone).
    pub tag: Option<ContactTagInfo>,
    pub class: ContactFactClass,
    pub owed_by: Option<ContactParty>,
    pub due: Option<i64>,
    /// Topics and commitments: open or closed.
    pub open: Option<bool>,
    pub use_: FactUse,
    pub as_of: Option<i64>,
    pub source: ContactFactSource,
    /// Held for the user because a rule or the agent called it high risk.
    pub high_risk: bool,
    /// Why it was held.
    pub risk_reason: Option<String>,
    pub status: ContactFactStatus,
    /// A held change to this other item.
    pub changes_id: Option<i64>,
    /// Accepted from mail and not yet seen by the user.
    pub is_new: bool,
    /// An open topic or commitment gone quiet (left out of prompts), or a
    /// fact old enough to check.
    pub stale: bool,
    pub evidence: Vec<ContactQuote>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactAddress {
    pub email: String,
    pub name: Option<String>,
    pub sent_count: i64,
    pub received_count: i64,
    pub last_seen: i64,
    /// When the user joined it to this contact, for a joined address.
    pub joined_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactInfo {
    pub person_id: i64,
    /// The best name known, else the first address.
    pub name: String,
    pub addresses: Vec<ContactAddress>,
    /// Accepted tags, relationships first.
    pub tags: Vec<ContactTagInfo>,
    /// Items held for review.
    pub held: i64,
}

/// A row of the Contacts list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactSummary {
    /// `None` for an address found by search that has no contact yet.
    pub person_id: Option<i64>,
    pub name: String,
    pub email: String,
    pub tags: Vec<ContactTagInfo>,
    pub held: i64,
    /// When something about them last changed, or their last mail.
    pub latest: i64,
}

/// A conversation where the user waits for this contact's answer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactWait {
    pub thread_id: String,
    pub message_id: String,
    pub subject: String,
    pub since: i64,
}

/// A task of the user's on a conversation this contact wrote in.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactTask {
    pub id: i64,
    pub title: String,
    pub done: bool,
    pub thread_id: String,
    pub due_day: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactConversation {
    pub thread_id: String,
    pub subject: String,
    pub latest: i64,
}

/// What a contact's page shows that no agent wrote: found by query from
/// Waiting's and Tasks' own tables each time (nothing is copied).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactLinks {
    pub first_mail: Option<i64>,
    pub latest_mail: Option<i64>,
    /// Messages the user sent them, and they sent.
    pub sent: i64,
    pub received: i64,
    pub waiting: Vec<ContactWait>,
    /// Open first; done ones after.
    pub tasks: Vec<ContactTask>,
    /// The latest five.
    pub recent: Vec<ContactConversation>,
}

/// What the user writes or changes on an item.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactFactFields {
    pub kind: ContactFactKind,
    pub label: String,
    pub value: String,
    pub class: ContactFactClass,
    pub owed_by: Option<ContactParty>,
    pub due: Option<i64>,
    pub use_: FactUse,
    pub as_of: Option<i64>,
}

/// One change in a set applied (and undone) together.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ContactEdit {
    Add {
        person_id: i64,
        fields: ContactFactFields,
    },
    Update {
        id: i64,
        fields: ContactFactFields,
    },
    /// Accept or reject a held item (accepting a held change applies it).
    Decide {
        id: i64,
        accept: bool,
    },
    Delete {
        id: i64,
    },
    SetUse {
        id: i64,
        use_: FactUse,
    },
    /// Close or reopen a topic or commitment.
    SetOpen {
        id: i64,
        open: bool,
    },
    AddTag {
        person_id: i64,
        tag_id: String,
    },
    RemoveTag {
        person_id: i64,
        tag_id: String,
    },
}

/// A change to the account's tag list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ContactTagEdit {
    Add {
        name: String,
        set: ContactTagSet,
        /// One of the set's headings, or empty for *Other*.
        heading: String,
    },
    Rename {
        id: String,
        name: String,
    },
    /// Takes the tag off every contact, in the same change.
    Delete {
        id: String,
    },
}

/// A change applied: its id for undo and redo, and whose pages it touched.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ContactChange {
    pub change_id: i64,
    pub person_ids: Vec<i64>,
    /// Items the change added, in order.
    pub added: Vec<i64>,
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::InvalidInput, message)
}

fn refuse(message: impl Into<String>) -> StoreError {
    StoreError::Invalid(message.into())
}

pub(crate) fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Records what a change touches as it goes: each row as it was the first
/// time it is named, before it is written. `finish` records the change.
#[derive(Default)]
pub(crate) struct Recorder {
    touched: Touched,
    before: Snapshot,
    pub(crate) added: Vec<i64>,
}

impl Recorder {
    fn note(&mut self, tx: &Transaction<'_>, part: Touched) -> StoreResult<()> {
        let snap = store::snapshot(tx, &part)?;
        self.before.facts.extend(snap.facts);
        self.before.tags.extend(snap.tags);
        self.before.people.extend(snap.people);
        self.before.addresses.extend(snap.addresses);
        self.before.reads.extend(snap.reads);
        self.touched.fact_ids.extend(part.fact_ids);
        self.touched.tag_ids.extend(part.tag_ids);
        self.touched.person_ids.extend(part.person_ids);
        self.touched.emails.extend(part.emails);
        self.touched.read_person_ids.extend(part.read_person_ids);
        Ok(())
    }

    /// An existing item about to change.
    pub(crate) fn fact(&mut self, tx: &Transaction<'_>, id: i64) -> StoreResult<()> {
        if self.touched.fact_ids.contains(&id) {
            return Ok(());
        }
        self.note(tx, Touched { fact_ids: vec![id], ..Default::default() })
    }

    /// An item the change just inserted.
    pub(crate) fn new_fact(&mut self, id: i64) {
        self.touched.fact_ids.push(id);
        self.added.push(id);
    }

    pub(crate) fn tag(&mut self, tx: &Transaction<'_>, id: &str) -> StoreResult<()> {
        if self.touched.tag_ids.iter().any(|t| t == id) {
            return Ok(());
        }
        self.note(tx, Touched { tag_ids: vec![id.to_owned()], ..Default::default() })
    }

    pub(crate) fn person(&mut self, tx: &Transaction<'_>, id: i64) -> StoreResult<()> {
        if self.touched.person_ids.contains(&id) {
            return Ok(());
        }
        self.note(tx, Touched { person_ids: vec![id], ..Default::default() })
    }

    pub(crate) fn email(&mut self, tx: &Transaction<'_>, email: &str) -> StoreResult<()> {
        if self.touched.emails.iter().any(|e| e.eq_ignore_ascii_case(email)) {
            return Ok(());
        }
        self.note(tx, Touched { emails: vec![email.to_owned()], ..Default::default() })
    }

    pub(crate) fn reads(&mut self, tx: &Transaction<'_>, person_id: i64) -> StoreResult<()> {
        if self.touched.read_person_ids.contains(&person_id) {
            return Ok(());
        }
        self.note(tx, Touched { read_person_ids: vec![person_id], ..Default::default() })
    }

    /// Whether anything was touched.
    pub(crate) fn is_empty(&self) -> bool {
        self.touched == Touched::default()
    }

    /// Record the change; `None` when it changed nothing.
    pub(crate) fn finish(self, tx: &Transaction<'_>, reason: &str, now: i64) -> StoreResult<Option<ContactChange>> {
        let after = store::snapshot(tx, &self.touched)?;
        if after == self.before {
            return Ok(None);
        }
        let mut person_ids: Vec<i64> = self
            .before
            .facts
            .iter()
            .chain(&after.facts)
            .map(|(f, _)| f.person_id)
            .chain(self.touched.person_ids.iter().copied())
            .collect();
        person_ids.sort_unstable();
        person_ids.dedup();
        let change_id = store::record_change(tx, reason, &self.before, &after, now)?;
        Ok(Some(ContactChange { change_id, person_ids, added: self.added }))
    }
}

/// Every key either side of a change names.
fn touched_of(before: &Snapshot, after: &Snapshot) -> Touched {
    let mut t = Touched::default();
    for s in [before, after] {
        t.fact_ids.extend(s.facts.iter().map(|(f, _)| f.id));
        t.tag_ids.extend(s.tags.iter().map(|t| t.id.clone()));
        t.person_ids.extend(s.people.iter().map(|p| p.id));
        t.emails.extend(s.addresses.iter().map(|(e, _, _)| e.to_lowercase()));
        t.read_person_ids.extend(s.reads.iter().map(|(_, p, _)| *p));
    }
    t.fact_ids.sort_unstable();
    t.fact_ids.dedup();
    t.tag_ids.sort();
    t.tag_ids.dedup();
    t.person_ids.sort_unstable();
    t.person_ids.dedup();
    t.emails.sort();
    t.emails.dedup();
    t.read_person_ids.sort_unstable();
    t.read_person_ids.dedup();
    t
}

fn tag_info(t: &TagRow, counts: &[(String, i64)]) -> ContactTagInfo {
    ContactTagInfo {
        id: t.id.clone(),
        name: t.name.clone(),
        set: ContactTagSet::parse(&t.tag_set),
        heading: t.heading.clone(),
        builtin: t.builtin,
        contacts: counts.iter().find(|(id, _)| id == &t.id).map_or(0, |(_, n)| *n),
    }
}

/// Whether an item has gone quiet (topics, commitments) or old (facts).
pub(crate) fn is_stale(f: &ContactFactRow, now: i64) -> bool {
    let open = f.state.as_deref() == Some("open");
    match f.kind.as_str() {
        "topic" => open && f.as_of.unwrap_or(f.updated_at) < now - TOPIC_STALE_MS,
        "commitment" => open && f.due.is_some_and(|d| d < now - COMMITMENT_STALE_MS),
        "fact" => f.as_of.is_some_and(|a| a < now - FACT_AGED_MS),
        _ => false,
    }
}

pub(crate) fn fact_info(
    f: ContactFactRow,
    evidence: Vec<ContactEvidence>,
    tags: &[TagRow],
    counts: &[(String, i64)],
    now: i64,
) -> ContactFactInfo {
    let stale = is_stale(&f, now);
    let tag = (f.kind == "tag").then(|| tags.iter().find(|t| t.id == f.value).map(|t| tag_info(t, counts))).flatten();
    ContactFactInfo {
        id: f.id,
        person_id: f.person_id,
        kind: ContactFactKind::parse(&f.kind).unwrap_or(ContactFactKind::Fact),
        value: tag.as_ref().map_or(f.value, |t| t.name.clone()),
        label: f.label,
        tag,
        class: ContactFactClass::parse(&f.class),
        owed_by: f.owed_by.as_deref().and_then(ContactParty::parse),
        due: f.due,
        open: f.state.as_deref().map(|s| s == "open"),
        use_: FactUse::parse(&f.use_),
        as_of: f.as_of,
        source: if f.source == "you" { ContactFactSource::You } else { ContactFactSource::Learned },
        high_risk: f.risk == "high",
        risk_reason: f.risk_reason,
        status: ContactFactStatus::parse(&f.status),
        changes_id: f.changes_id,
        is_new: f.status == "accepted" && !f.seen && f.source == "learned",
        stale,
        evidence: evidence
            .into_iter()
            .map(|e| ContactQuote {
                message_id: e.message_id,
                thread_id: None,
                quote: e.quote,
                said_by: SaidBy::parse(&e.said_by),
                message_date: e.message_date,
            })
            .collect(),
        created_at: f.created_at,
        updated_at: f.updated_at,
    }
}

/// Say which conversation each quote was said in.
pub(crate) fn with_threads(conn: &mail_store::Connection, mut info: ContactFactInfo) -> StoreResult<ContactFactInfo> {
    for quote in &mut info.evidence {
        quote.thread_id = mail_store::contact_learn::thread_of(conn, &quote.message_id)?;
    }
    Ok(info)
}

/// A person's accepted tags, relationships first, in the list's order.
pub(crate) fn tags_of(
    conn: &mail_store::Connection,
    person_id: i64,
    tags: &[TagRow],
    counts: &[(String, i64)],
) -> StoreResult<Vec<ContactTagInfo>> {
    let have: Vec<String> = store::list_for_person(conn, person_id, &["accepted"])?
        .into_iter()
        .filter(|f| f.kind == "tag")
        .map(|f| f.value)
        .collect();
    Ok(tags.iter().filter(|t| have.contains(&t.id)).map(|t| tag_info(t, counts)).collect())
}

pub(crate) fn contact_info(conn: &mail_store::Connection, person_id: i64) -> StoreResult<Option<ContactInfo>> {
    let Some(person) = store::person(conn, person_id)? else { return Ok(None) };
    let addresses = store::addresses(conn, person_id)?;
    let name = person
        .name
        .filter(|n| !n.trim().is_empty())
        .or_else(|| addresses.iter().find_map(|a| a.name.clone().filter(|n| !n.trim().is_empty())))
        .or_else(|| addresses.first().map(|a| a.email.clone()))
        .unwrap_or_default();
    let tags = store::tags(conn)?;
    let counts = store::tag_counts(conn)?;
    let held = store::list_for_person(conn, person_id, &["proposed"])?.len() as i64;
    Ok(Some(ContactInfo {
        person_id,
        name,
        addresses: addresses
            .into_iter()
            .map(|a| ContactAddress {
                email: a.email,
                name: a.name,
                sent_count: a.sent_count,
                received_count: a.received_count,
                last_seen: a.last_seen,
                joined_at: a.person_joined_at,
            })
            .collect(),
        tags: tags_of(conn, person_id, &tags, &counts)?,
        held,
    }))
}

fn row_from(person_id: i64, fields: &ContactFactFields, now: i64) -> StoreResult<ContactFactRow> {
    if fields.kind == ContactFactKind::Tag {
        return Err(refuse("add a tag from the tag list"));
    }
    let commitment = fields.kind == ContactFactKind::Commitment;
    let has_state = matches!(fields.kind, ContactFactKind::Topic | ContactFactKind::Commitment);
    if commitment && fields.owed_by.is_none() {
        return Err(refuse("say who owes it: you or them"));
    }
    Ok(ContactFactRow {
        person_id,
        kind: fields.kind.as_str().into(),
        label: clean(&fields.label),
        value: clean(&fields.value),
        class: fields.class.as_str().into(),
        owed_by: commitment.then(|| fields.owed_by.map(|p| p.as_str().to_owned())).flatten(),
        due: commitment.then_some(fields.due).flatten(),
        state: has_state.then(|| "open".to_owned()),
        use_: fields.use_.as_str().into(),
        as_of: fields.as_of,
        source: "you".into(),
        risk: "low".into(),
        status: "accepted".into(),
        seen: true,
        created_at: now,
        updated_at: now,
        ..Default::default()
    })
}

fn existing(tx: &Transaction<'_>, id: i64) -> StoreResult<ContactFactRow> {
    store::get(tx, id)?.ok_or_else(|| refuse("that item no longer exists"))
}

/// Refuse a second live item with the label, with words the user can read.
fn refuse_twin(tx: &Transaction<'_>, row: &ContactFactRow) -> StoreResult<()> {
    match store::find(tx, row.person_id, &row.kind, &row.label)? {
        Some(other) if other.id != row.id => Err(refuse(format!("this contact already has “{}”", other.label))),
        _ => Ok(()),
    }
}

/// Apply one edit inside a change.
pub(crate) fn apply_edit(tx: &Transaction<'_>, rec: &mut Recorder, edit: &ContactEdit, now: i64) -> StoreResult<()> {
    match edit {
        ContactEdit::Add { person_id, fields } => {
            if store::person(tx, *person_id)?.is_none() {
                return Err(refuse("that contact no longer exists"));
            }
            let row = row_from(*person_id, fields, now)?;
            store::check(&row)?;
            refuse_twin(tx, &row)?;
            let id = store::insert(tx, &row)?;
            rec.new_fact(id);
        }
        ContactEdit::Update { id, fields } => {
            rec.fact(tx, *id)?;
            let old = existing(tx, *id)?;
            if old.kind == "tag" {
                return Err(refuse("a tag is added or removed, not edited"));
            }
            let new = row_from(old.person_id, fields, now)?;
            // What the user edits is theirs; its state, status and age stay.
            let row = ContactFactRow {
                id: old.id,
                state: if new.state.is_some() { old.state.clone().or(new.state.clone()) } else { None },
                status: old.status.clone(),
                changes_id: old.changes_id,
                risk: old.risk.clone(),
                risk_reason: old.risk_reason.clone(),
                created_at: old.created_at,
                ..new
            };
            store::check(&row)?;
            if row.changes_id.is_none() && row.status != "rejected" {
                refuse_twin(tx, &row)?;
            }
            store::write(tx, &row)?;
        }
        ContactEdit::Decide { id, accept } => {
            rec.fact(tx, *id)?;
            let old = existing(tx, *id)?;
            if !accept {
                store::write(tx, &ContactFactRow { status: "rejected".into(), updated_at: now, ..old })?;
                return Ok(());
            }
            match old.changes_id {
                // A held change: the item it names takes its words, and the
                // proposal is gone (its quotes go with the item).
                Some(target_id) => {
                    rec.fact(tx, target_id)?;
                    let target = existing(tx, target_id)?;
                    let quotes = store::evidence(tx, old.id)?;
                    store::delete(tx, old.id)?;
                    let changed = ContactFactRow {
                        label: old.label,
                        value: old.value,
                        class: old.class,
                        owed_by: old.owed_by,
                        due: old.due,
                        state: old.state,
                        as_of: old.as_of.or(target.as_of),
                        updated_at: now,
                        ..target
                    };
                    refuse_twin(tx, &changed)?;
                    store::write(tx, &changed)?;
                    store::add_evidence(tx, target_id, &quotes, now)?;
                }
                None => {
                    let row = ContactFactRow { status: "accepted".into(), seen: true, updated_at: now, ..old };
                    if row.kind != "tag" {
                        refuse_twin(tx, &row)?;
                    }
                    store::write(tx, &row)?;
                }
            }
        }
        ContactEdit::Delete { id } => {
            rec.fact(tx, *id)?;
            existing(tx, *id)?;
            // Held changes to it go with it (the store cascades); record them.
            for other in store::list(tx, &["proposed", "watching", "rejected"])? {
                if other.changes_id == Some(*id) {
                    rec.fact(tx, other.id)?;
                }
            }
            store::delete(tx, *id)?;
        }
        ContactEdit::SetUse { id, use_ } => {
            rec.fact(tx, *id)?;
            let old = existing(tx, *id)?;
            store::write(tx, &ContactFactRow { use_: use_.as_str().into(), updated_at: now, ..old })?;
        }
        ContactEdit::SetOpen { id, open } => {
            rec.fact(tx, *id)?;
            let old = existing(tx, *id)?;
            if !matches!(old.kind.as_str(), "topic" | "commitment") {
                return Err(refuse("only topics and commitments are closed"));
            }
            let state = if *open { "open" } else { "closed" };
            // Reopening counts as mail on it: it is not stale at once.
            let as_of = if *open { Some(now) } else { old.as_of };
            store::write(tx, &ContactFactRow { state: Some(state.into()), as_of, updated_at: now, ..old })?;
        }
        ContactEdit::AddTag { person_id, tag_id } => {
            if store::person(tx, *person_id)?.is_none() {
                return Err(refuse("that contact no longer exists"));
            }
            if store::get_tag(tx, tag_id)?.is_none() {
                return Err(refuse("that tag is not in your list"));
            }
            match store::find(tx, *person_id, "tag", tag_id)? {
                Some(have) if have.status == "accepted" => {}
                // One suggested from mail: the user's choice settles it.
                Some(held) => {
                    rec.fact(tx, held.id)?;
                    store::write(
                        tx,
                        &ContactFactRow { status: "accepted".into(), seen: true, updated_at: now, ..held },
                    )?;
                }
                None => {
                    let id = store::insert(
                        tx,
                        &ContactFactRow {
                            person_id: *person_id,
                            kind: "tag".into(),
                            value: tag_id.clone(),
                            class: "other".into(),
                            use_: "free".into(),
                            source: "you".into(),
                            risk: "low".into(),
                            status: "accepted".into(),
                            seen: true,
                            created_at: now,
                            updated_at: now,
                            ..Default::default()
                        },
                    )?;
                    rec.new_fact(id);
                }
            }
        }
        ContactEdit::RemoveTag { person_id, tag_id } => {
            if let Some(item) = store::find(tx, *person_id, "tag", tag_id)? {
                rec.fact(tx, item.id)?;
                store::delete(tx, item.id)?;
            }
        }
    }
    Ok(())
}

fn apply_tag_edit(tx: &Transaction<'_>, rec: &mut Recorder, edit: &ContactTagEdit, now: i64) -> StoreResult<()> {
    let taken = |tx: &Transaction<'_>, set: &str, name: &str, but: &str| -> StoreResult<bool> {
        Ok(store::tags(tx)?
            .iter()
            .any(|t| t.tag_set == set && t.id != but && store::tag_key(&t.name) == store::tag_key(name)))
    };
    match edit {
        ContactTagEdit::Add { name, set, heading } => {
            let name = clean(name);
            if taken(tx, set.as_str(), &name, "")? {
                return Err(refuse(format!("you already have a tag “{name}”")));
            }
            let all = store::tags(tx)?;
            // An id not in use, even within the same millisecond.
            let mut id = format!("user.{now}");
            while all.iter().any(|t| t.id == id) || rec.touched.tag_ids.contains(&id) {
                id.push('x');
            }
            let heading = clean(heading);
            store::put_tag(
                tx,
                &TagRow {
                    id: id.clone(),
                    name,
                    tag_set: set.as_str().into(),
                    heading: if heading.is_empty() { "Other".into() } else { heading },
                    vcard_type: None,
                    builtin: false,
                    position: all.iter().map(|t| t.position).max().unwrap_or(0) + 1,
                    created_at: now,
                },
            )?;
            rec.tag(tx, &id)?;
            // It did not exist before: undo removes it.
            rec.before.tags.retain(|t| t.id != id);
        }
        ContactTagEdit::Rename { id, name } => {
            rec.tag(tx, id)?;
            let old = store::get_tag(tx, id)?.ok_or_else(|| refuse("that tag no longer exists"))?;
            let name = clean(name);
            if taken(tx, &old.tag_set, &name, id)? {
                return Err(refuse(format!("you already have a tag “{name}”")));
            }
            store::put_tag(tx, &TagRow { name, ..old })?;
        }
        ContactTagEdit::Delete { id } => {
            rec.tag(tx, id)?;
            store::get_tag(tx, id)?.ok_or_else(|| refuse("that tag no longer exists"))?;
            for item in store::items_with_tag(tx, id)? {
                rec.fact(tx, item)?;
                store::delete(tx, item)?;
            }
            store::delete_tag(tx, id)?;
        }
    }
    Ok(())
}

impl Core {
    pub(crate) fn contacts_changed(&self) {
        self.account_events().emit(CoreEvent::ContactsChanged);
    }

    /// Run `f` as one recorded change to contacts. `Ok(None)` when it
    /// changed nothing.
    pub(crate) async fn contact_change<F>(&self, reason: String, f: F) -> Result<Option<ContactChange>, CoreError>
    where
        F: FnOnce(&Transaction<'_>, &mut Recorder, i64) -> StoreResult<()> + Send + 'static,
    {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        let change = runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    let mut rec = Recorder::default();
                    f(tx, &mut rec, now)?;
                    if rec.is_empty() {
                        return Ok(None);
                    }
                    rec.finish(tx, &reason, now)
                })
                .await?)
        })
        .await?;
        if change.is_some() {
            self.contacts_changed();
        }
        Ok(change)
    }

    async fn replay_contacts(&self, change_id: i64, undo: bool) -> Result<(), CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    let (_, before, after) = store::get_change(tx, change_id)?
                        .ok_or_else(|| refuse("that change can no longer be undone"))?;
                    let touched = touched_of(&before, &after);
                    store::restore(tx, &touched, if undo { &before } else { &after }, now)
                })
                .await?)
        })
        .await?;
        self.contacts_changed();
        Ok(())
    }
}

fn nothing_changed() -> CoreError {
    invalid("nothing to change")
}

#[uniffi::export]
impl Core {
    /// The contact an address belongs to, made now if it has none (opening
    /// a page is a first need). Only people have contacts.
    pub async fn contact_for_address(&self, email: String) -> Result<ContactInfo, CoreError> {
        let email = email.trim().to_owned();
        let own = self.own_addresses().await.unwrap_or_default();
        if own.iter().any(|a| a.eq_ignore_ascii_case(&email)) {
            return Err(invalid("that is your own address"));
        }
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    if !store::is_person(tx, &email)? {
                        return Err(refuse("contacts are people: this address is a list or an automated sender"));
                    }
                    let id = store::ensure_person(tx, &email, now)?;
                    contact_info(tx, id)?.ok_or_else(|| refuse("that contact no longer exists"))
                })
                .await?)
        })
        .await
    }

    pub async fn contact(&self, person_id: i64) -> Result<ContactInfo, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| contact_info(c, person_id)?.ok_or_else(|| refuse("that contact no longer exists")))
                .await?)
        })
        .await
    }

    /// The Contacts list: contacts with items or something held, most
    /// recent first, having every tag in `tag_ids`. With `search`, also
    /// any person's address that matches by name or address.
    pub async fn list_contacts(&self, tag_ids: Vec<String>, search: String) -> Result<Vec<ContactSummary>, CoreError> {
        let db = self.db()?;
        let own = self.own_addresses().await.unwrap_or_default();
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let tags = store::tags(c)?;
                    let counts = store::tag_counts(c)?;
                    let needle = search.trim().to_lowercase();
                    let mut out: Vec<ContactSummary> = Vec::new();
                    for (person_id, held, latest) in store::people_with_items(c)? {
                        let Some(info) = contact_info(c, person_id)? else { continue };
                        if !tag_ids.iter().all(|t| info.tags.iter().any(|have| &have.id == t)) {
                            continue;
                        }
                        let matches = needle.is_empty()
                            || info.name.to_lowercase().contains(&needle)
                            || info.addresses.iter().any(|a| a.email.to_lowercase().contains(&needle));
                        if !matches {
                            continue;
                        }
                        out.push(ContactSummary {
                            person_id: Some(person_id),
                            name: info.name,
                            email: info.addresses.first().map(|a| a.email.clone()).unwrap_or_default(),
                            tags: info.tags,
                            held,
                            latest,
                        });
                    }
                    // Anyone else found by name or address, when no tag narrows the list.
                    if !needle.is_empty() && tag_ids.is_empty() {
                        for found in mail_store::read::suggest_contacts(c, &needle, 20)? {
                            let Some(row) = store::address_row(c, &found.email)? else { continue };
                            let listed = out.iter().any(|o| {
                                o.email.eq_ignore_ascii_case(&row.email)
                                    || (row.person_id.is_some() && o.person_id == row.person_id)
                            });
                            if listed
                                || own.iter().any(|a| a.eq_ignore_ascii_case(&row.email))
                                || !store::is_person(c, &row.email)?
                            {
                                continue;
                            }
                            let person_tags = match row.person_id {
                                Some(id) => tags_of(c, id, &tags, &counts)?,
                                None => Vec::new(),
                            };
                            out.push(ContactSummary {
                                person_id: row.person_id,
                                name: row
                                    .name
                                    .clone()
                                    .filter(|n| !n.trim().is_empty())
                                    .unwrap_or_else(|| row.email.clone()),
                                email: row.email,
                                tags: person_tags,
                                held: 0,
                                latest: row.last_seen,
                            });
                        }
                    }
                    Ok(out)
                })
                .await?)
        })
        .await
    }

    /// A contact's items in any of `statuses`, newest first.
    pub async fn contact_facts(
        &self,
        person_id: i64,
        statuses: Vec<ContactFactStatus>,
    ) -> Result<Vec<ContactFactInfo>, CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let statuses: Vec<&str> = statuses.iter().map(|s| s.as_str()).collect();
                    let tags = store::tags(c)?;
                    let counts = store::tag_counts(c)?;
                    let mut out = Vec::new();
                    for f in store::list_for_person(c, person_id, &statuses)? {
                        let quotes = store::evidence(c, f.id)?;
                        out.push(with_threads(c, fact_info(f, quotes, &tags, &counts, now))?);
                    }
                    Ok(out)
                })
                .await?)
        })
        .await
    }

    /// Every item held for review, across contacts, oldest first.
    pub async fn held_contact_facts(&self) -> Result<Vec<ContactFactInfo>, CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let tags = store::tags(c)?;
                    let counts = store::tag_counts(c)?;
                    let mut out = Vec::new();
                    for f in store::list(c, &["proposed"])? {
                        let quotes = store::evidence(c, f.id)?;
                        out.push(with_threads(c, fact_info(f, quotes, &tags, &counts, now))?);
                    }
                    out.sort_by_key(|f| (f.created_at, f.id));
                    Ok(out)
                })
                .await?)
        })
        .await
    }

    /// Every accepted item of every contact (What Kaluta Knows' People
    /// tab), by contact.
    pub async fn accepted_contact_facts(&self) -> Result<Vec<ContactFactInfo>, CoreError> {
        let db = self.db()?;
        let now = mail_sync::now_millis();
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let tags = store::tags(c)?;
                    let counts = store::tag_counts(c)?;
                    let mut out = Vec::new();
                    for f in store::list(c, &["accepted"])? {
                        let quotes = store::evidence(c, f.id)?;
                        out.push(fact_info(f, quotes, &tags, &counts, now));
                    }
                    Ok(out)
                })
                .await?)
        })
        .await
    }

    /// How many items wait for review (the sidebar's count).
    pub async fn held_contact_fact_count(&self) -> Result<i64, CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.read(store::held_count).await?) }).await
    }

    /// The account's tag list, relationships then roles, with how many
    /// contacts carry each.
    pub async fn contact_tags(&self) -> Result<Vec<ContactTagInfo>, CoreError> {
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(|c| {
                    let counts = store::tag_counts(c)?;
                    Ok(store::tags(c)?.iter().map(|t| tag_info(t, &counts)).collect())
                })
                .await?)
        })
        .await
    }

    /// The names a search's `tag:` operators ask for that are not tags on
    /// the account's list (spec §8): the list says "No tag named …". Empty
    /// when every one is a tag, or the query does not parse.
    pub async fn unknown_search_tags(&self, query: String) -> Result<Vec<String>, CoreError> {
        let Ok(expr) = mail_store::search::parse(&query) else { return Ok(vec![]) };
        let asked = mail_store::search::tag_names(&expr);
        if asked.is_empty() {
            return Ok(vec![]);
        }
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let mut unknown = Vec::new();
                    for name in asked {
                        if store::tags_named(c, &name)?.is_empty() && !unknown.contains(&name) {
                            unknown.push(name);
                        }
                    }
                    Ok(unknown)
                })
                .await?)
        })
        .await
    }

    /// Change a contact's items; one change, undoable
    /// (`undo_contact_change`).
    pub async fn apply_contact_edits(
        &self,
        edits: Vec<ContactEdit>,
        reason: String,
    ) -> Result<ContactChange, CoreError> {
        if edits.is_empty() {
            return Err(nothing_changed());
        }
        self.contact_change(reason, move |tx, rec, now| {
            for e in &edits {
                apply_edit(tx, rec, e, now)?;
            }
            Ok(())
        })
        .await?
        .ok_or_else(nothing_changed)
    }

    /// Change the account's tag list; one change, undoable.
    pub async fn edit_contact_tags(&self, edits: Vec<ContactTagEdit>) -> Result<ContactChange, CoreError> {
        if edits.is_empty() {
            return Err(nothing_changed());
        }
        self.contact_change("tags".into(), move |tx, rec, now| {
            for e in &edits {
                apply_tag_edit(tx, rec, e, now)?;
            }
            Ok(())
        })
        .await?
        .ok_or_else(nothing_changed)
    }

    /// *Same Person As…*: `person_id`'s addresses and items join `into`,
    /// in one undoable change. Kaluta never does this on its own.
    pub async fn join_contacts(&self, person_id: i64, into: i64) -> Result<ContactChange, CoreError> {
        if person_id == into {
            return Err(invalid("choose another contact"));
        }
        self.contact_change("join".into(), move |tx, rec, now| {
            let gone = store::person(tx, person_id)?.ok_or_else(|| refuse("that contact no longer exists"))?;
            let kept = store::person(tx, into)?.ok_or_else(|| refuse("that contact no longer exists"))?;
            rec.person(tx, gone.id)?;
            rec.person(tx, kept.id)?;
            rec.reads(tx, gone.id)?;
            rec.reads(tx, kept.id)?;
            for a in store::addresses(tx, gone.id)? {
                rec.email(tx, &a.email)?;
                store::set_address_person(tx, &a.email, Some(kept.id), Some(now))?;
            }
            // Items move; where both have one with the label (or the tag),
            // the newer stays.
            for item in store::list_for_person(tx, gone.id, &["watching", "proposed", "accepted", "rejected"])? {
                rec.fact(tx, item.id)?;
                let key = if item.kind == "tag" { item.value.clone() } else { item.label.clone() };
                let twin = (item.status != "rejected" && item.changes_id.is_none())
                    .then(|| store::find(tx, kept.id, &item.kind, &key))
                    .transpose()?
                    .flatten();
                if let Some(twin) = twin {
                    rec.fact(tx, twin.id)?;
                    let newer = |f: &ContactFactRow| f.as_of.unwrap_or(f.updated_at);
                    if newer(&item) > newer(&twin) {
                        store::delete(tx, twin.id)?;
                    } else {
                        store::delete(tx, item.id)?;
                        continue;
                    }
                }
                store::write(tx, &ContactFactRow { person_id: kept.id, ..item })?;
            }
            store::move_reads(tx, gone.id, kept.id)?;
            if kept.name.is_none() && gone.name.is_some() {
                store::put_person(tx, &PersonRow { name: gone.name.clone(), ..kept })?;
            }
            store::delete_person(tx, gone.id)?;
            Ok(())
        })
        .await?
        .ok_or_else(nothing_changed)
    }

    /// *Forget This Contact*: their items, the quotes and the read log go,
    /// in one undoable change. The mail is untouched.
    pub async fn forget_contact(&self, person_id: i64) -> Result<ContactChange, CoreError> {
        self.contact_change("forget".into(), move |tx, rec, _now| {
            store::person(tx, person_id)?.ok_or_else(|| refuse("that contact no longer exists"))?;
            rec.person(tx, person_id)?;
            rec.reads(tx, person_id)?;
            for item in store::list_for_person(tx, person_id, &["watching", "proposed", "accepted", "rejected"])? {
                rec.fact(tx, item.id)?;
                store::delete(tx, item.id)?;
            }
            store::delete_reads(tx, person_id)?;
            Ok(())
        })
        .await?
        .ok_or_else(|| invalid("there is nothing to forget about this contact"))
    }

    /// The page's waits, tasks and recent conversations, and how much mail
    /// there is with the contact. No agent is involved.
    pub async fn contact_links(&self, person_id: i64) -> Result<ContactLinks, CoreError> {
        use mail_store::contact_page as page;
        let db = self.db()?;
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let emails: Vec<String> = store::addresses(c, person_id)?.into_iter().map(|a| a.email).collect();
                    let stats = page::mail_stats(c, &emails)?;
                    Ok(ContactLinks {
                        first_mail: stats.first,
                        latest_mail: stats.latest,
                        sent: stats.sent,
                        received: stats.received,
                        waiting: page::waiting_on(c, &emails)?
                            .into_iter()
                            .map(|w| ContactWait {
                                thread_id: w.thread_id,
                                message_id: w.message_id,
                                subject: w.subject,
                                since: w.since,
                            })
                            .collect(),
                        tasks: page::tasks_from(c, &emails)?
                            .into_iter()
                            .map(|t| ContactTask {
                                id: t.id,
                                title: t.title,
                                done: t.done,
                                thread_id: t.thread_id,
                                due_day: t.due_day,
                            })
                            .collect(),
                        recent: page::recent_conversations(c, &emails, 5)?
                            .into_iter()
                            .map(|r| ContactConversation {
                                thread_id: r.thread_id,
                                subject: r.subject,
                                latest: r.latest,
                            })
                            .collect(),
                    })
                })
                .await?)
        })
        .await
    }

    /// The user has seen the contact's page: nothing on it is *New* now.
    pub async fn mark_contact_seen(&self, person_id: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        let changed =
            runtime::run(async move { Ok(db.write(move |tx| store::mark_seen(tx, person_id)).await?) }).await?;
        if changed > 0 {
            self.contacts_changed();
        }
        Ok(())
    }

    pub async fn undo_contact_change(&self, change_id: i64) -> Result<(), CoreError> {
        self.replay_contacts(change_id, true).await
    }

    pub async fn redo_contact_change(&self, change_id: i64) -> Result<(), CoreError> {
        self.replay_contacts(change_id, false).await
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use futures::executor::block_on;

    use super::*;

    pub(crate) fn fields(kind: ContactFactKind, label: &str, value: &str) -> ContactFactFields {
        ContactFactFields {
            kind,
            label: label.into(),
            value: value.into(),
            class: ContactFactClass::Other,
            owed_by: (kind == ContactFactKind::Commitment).then_some(ContactParty::You),
            due: None,
            use_: FactUse::Free,
            as_of: None,
        }
    }

    pub(crate) fn person(core: &Core, email: &str) -> i64 {
        block_on(core.contact_for_address(email.into())).unwrap().person_id
    }

    fn add(core: &Core, person_id: i64, f: ContactFactFields) -> ContactChange {
        block_on(core.apply_contact_edits(vec![ContactEdit::Add { person_id, fields: f }], "add".into())).unwrap()
    }

    fn all(core: &Core, person_id: i64) -> Vec<ContactFactInfo> {
        block_on(core.contact_facts(
            person_id,
            vec![
                ContactFactStatus::Watching,
                ContactFactStatus::Proposed,
                ContactFactStatus::Accepted,
                ContactFactStatus::Rejected,
            ],
        ))
        .unwrap()
    }

    #[test]
    fn every_edit_undoes_and_redoes_exactly() {
        let s = crate::guide::tests::demo("contacts-edit");
        let core = &s.1;
        let reese = person(core, "reese@example.com");
        assert_eq!(person(core, "Reese@Example.com"), reese);

        let added = add(core, reese, fields(ContactFactKind::Topic, "Renewal pricing for 2027", "Waiting on numbers"));
        let id = added.added[0];
        assert_eq!(added.person_ids, vec![reese]);
        let dup = block_on(core.apply_contact_edits(
            vec![ContactEdit::Add {
                person_id: reese,
                fields: fields(ContactFactKind::Topic, "renewal PRICING for 2027", "x"),
            }],
            "add".into(),
        ))
        .unwrap_err();
        assert_eq!(dup.kind(), ErrorKind::InvalidInput, "one item per label within a contact and kind");
        let long = block_on(core.apply_contact_edits(
            vec![ContactEdit::Add {
                person_id: reese,
                fields: fields(ContactFactKind::Fact, "Note", &"x".repeat(161)),
            }],
            "add".into(),
        ))
        .unwrap_err();
        assert_eq!(long.kind(), ErrorKind::InvalidInput, "an over-long value is refused, not cut");

        let before = all(core, reese);
        assert_eq!((before[0].open, before[0].source, before[0].is_new), (Some(true), ContactFactSource::You, false));
        let edits = [
            ContactEdit::Update { id, fields: fields(ContactFactKind::Topic, "Renewal pricing", "They sent numbers") },
            ContactEdit::SetUse { id, use_: FactUse::Ask },
            ContactEdit::SetOpen { id, open: false },
            ContactEdit::SetOpen { id, open: true },
            ContactEdit::Delete { id },
        ];
        // Each edit alone: undo gives the state before it, redo the state after.
        for edit in edits {
            let was = all(core, reese);
            let change = block_on(core.apply_contact_edits(vec![edit.clone()], "edit".into())).unwrap();
            let is = all(core, reese);
            assert_ne!(was, is, "{edit:?}");
            block_on(core.undo_contact_change(change.change_id)).unwrap();
            assert_eq!(all(core, reese), was, "undo {edit:?}");
            block_on(core.redo_contact_change(change.change_id)).unwrap();
            assert_eq!(all(core, reese), is, "redo {edit:?}");
        }
        assert!(all(core, reese).is_empty());
        // Undoing the add of an item removes it; redo brings back the same id.
        let again = add(core, reese, fields(ContactFactKind::Commitment, "Send the revised contract", "By Friday"));
        block_on(core.undo_contact_change(again.change_id)).unwrap();
        assert!(all(core, reese).is_empty());
        block_on(core.redo_contact_change(again.change_id)).unwrap();
        let back = all(core, reese);
        assert_eq!((back[0].id, back[0].owed_by), (again.added[0], Some(ContactParty::You)));
    }

    #[test]
    fn tags_come_from_the_list_and_the_list_is_the_users() {
        let s = crate::guide::tests::demo("contacts-tags");
        let core = &s.1;
        let reese = person(core, "reese@example.com");
        let ada = person(core, "ada@example.com");
        let tag = |person_id: i64, tag_id: &str| ContactEdit::AddTag { person_id, tag_id: tag_id.into() };
        block_on(core.apply_contact_edits(
            vec![
                tag(reese, "rel.customer"),
                tag(reese, "role.vp"),
                tag(reese, "role.engineering"),
                tag(ada, "rel.customer"),
            ],
            "tag".into(),
        ))
        .unwrap();
        // Adding one twice changes nothing; one not on the list is refused.
        assert!(block_on(core.apply_contact_edits(vec![tag(reese, "rel.customer")], "tag".into())).is_err());
        assert!(block_on(core.apply_contact_edits(vec![tag(reese, "rel.wizard")], "tag".into())).is_err());
        let info = block_on(core.contact(reese)).unwrap();
        let names: Vec<&str> = info.tags.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Customer", "VP", "Engineering"], "relationships first, in the list's order");
        assert_eq!(info.tags[0].contacts, 2);

        // The Contacts list filters by every tag given.
        let listed = |tags: &[&str]| {
            block_on(core.list_contacts(tags.iter().map(|t| t.to_string()).collect(), String::new()))
                .unwrap()
                .into_iter()
                .filter_map(|c| c.person_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(listed(&["rel.customer"]).len(), 2);
        assert_eq!(listed(&["rel.customer", "role.engineering"]), vec![reese]);
        assert!(listed(&["rel.vendor"]).is_empty());

        // The user's own tag, renamed (one row; contacts show the new name), then deleted.
        let made = block_on(core.edit_contact_tags(vec![ContactTagEdit::Add {
            name: "Design  partner".into(),
            set: ContactTagSet::Relationship,
            heading: String::new(),
        }]))
        .unwrap();
        let mine = block_on(core.contact_tags()).unwrap().into_iter().find(|t| t.name == "Design partner").unwrap();
        assert!(!mine.builtin && mine.heading == "Other");
        let twin = block_on(core.edit_contact_tags(vec![ContactTagEdit::Add {
            name: "design-partner".into(),
            set: ContactTagSet::Relationship,
            heading: String::new(),
        }]));
        assert!(twin.is_err(), "a name is unique within its set");
        block_on(core.apply_contact_edits(vec![tag(ada, &mine.id)], "tag".into())).unwrap();
        block_on(
            core.edit_contact_tags(vec![ContactTagEdit::Rename { id: mine.id.clone(), name: "Launch partner".into() }]),
        )
        .unwrap();
        assert!(
            block_on(core.contact(ada)).unwrap().tags.iter().any(|t| t.name == "Launch partner" && t.id == mine.id)
        );
        let deleted = block_on(core.edit_contact_tags(vec![ContactTagEdit::Delete { id: mine.id.clone() }])).unwrap();
        assert_eq!(block_on(core.contact(ada)).unwrap().tags.len(), 1, "deleting a tag takes it off every contact");
        block_on(core.undo_contact_change(deleted.change_id)).unwrap();
        assert!(block_on(core.contact(ada)).unwrap().tags.iter().any(|t| t.id == mine.id), "and undo puts it back on");
        block_on(core.redo_contact_change(deleted.change_id)).unwrap();
        block_on(core.undo_contact_change(made.change_id)).ok();
        // A built-in can be renamed and deleted too.
        block_on(
            core.edit_contact_tags(vec![ContactTagEdit::Rename { id: "rel.customer".into(), name: "Client".into() }]),
        )
        .unwrap();
        assert_eq!(block_on(core.contact(reese)).unwrap().tags[0].name, "Client");
        // Removing a tag by hand.
        block_on(core.apply_contact_edits(
            vec![ContactEdit::RemoveTag { person_id: reese, tag_id: "role.vp".into() }],
            "tag".into(),
        ))
        .unwrap();
        assert_eq!(block_on(core.contact(reese)).unwrap().tags.len(), 2);
    }

    #[test]
    fn joining_and_forgetting_undo_exactly() {
        let s = crate::guide::tests::demo("contacts-join");
        let core = &s.1;
        let work = person(core, "reese@work.example");
        let home = person(core, "reese@home.example");
        add(core, work, fields(ContactFactKind::Fact, "Role", "CTO at Acme"));
        add(core, work, fields(ContactFactKind::Topic, "Renewal", "Open"));
        add(core, home, fields(ContactFactKind::Fact, "Role", "Founder of a side project"));
        add(core, home, fields(ContactFactKind::Fact, "Prefers", "Short emails"));
        block_on(core.apply_contact_edits(
            vec![
                ContactEdit::AddTag { person_id: work, tag_id: "rel.customer".into() },
                ContactEdit::AddTag { person_id: home, tag_id: "rel.customer".into() },
                ContactEdit::AddTag { person_id: home, tag_id: "rel.friend".into() },
            ],
            "tag".into(),
        ))
        .unwrap();
        let state = |core: &Core| {
            (
                block_on(core.contact(work)).ok().map(|c| (c.addresses.len(), c.tags.len())),
                block_on(core.contact(home)).ok().map(|c| (c.addresses.len(), c.tags.len())),
                all(core, work),
                all(core, home),
            )
        };
        let apart = state(core);

        let joined = block_on(core.join_contacts(home, work)).unwrap();
        let one = block_on(core.contact(work)).unwrap();
        assert_eq!(one.addresses.len(), 2);
        assert!(one.addresses.iter().any(|a| a.email == "reese@home.example" && a.joined_at.is_some()));
        assert!(block_on(core.contact(home)).is_err(), "the joined contact is gone");
        assert_eq!(person(core, "reese@home.example"), work, "either address finds the page");
        let items = all(core, work);
        assert_eq!(items.iter().filter(|f| f.label == "Role").count(), 1, "one item per label after the join");
        assert_eq!(items.iter().filter(|f| f.kind == ContactFactKind::Tag).count(), 2);
        assert_eq!(items.len(), 5);
        let together = state(core);

        block_on(core.undo_contact_change(joined.change_id)).unwrap();
        assert_eq!(state(core), apart, "undoing a join puts both contacts back exactly");
        block_on(core.redo_contact_change(joined.change_id)).unwrap();
        assert_eq!(state(core), together);

        let forgot = block_on(core.forget_contact(work)).unwrap();
        assert!(all(core, work).is_empty());
        assert_eq!(block_on(core.contact(work)).unwrap().addresses.len(), 2, "the contact and the mail stay");
        block_on(core.undo_contact_change(forgot.change_id)).unwrap();
        assert_eq!(state(core), together, "undoing a forget puts everything back");
        assert!(block_on(core.forget_contact(work)).is_ok());
        assert!(block_on(core.forget_contact(work)).is_err(), "nothing left to forget");
    }

    #[test]
    fn a_search_says_which_of_its_tags_are_not_on_the_list() {
        let s = crate::guide::tests::demo("contacts-search-tags");
        let core = &s.1;
        let unknown = |q: &str| block_on(core.unknown_search_tags(q.into())).unwrap();
        assert!(unknown("tag:customer has:attachment").is_empty());
        assert!(unknown("tag:services-client tag:rel.customer tag:\"board member\"").is_empty());
        assert_eq!(
            unknown("invoice tag:wizard OR -tag:Wizard tag:vp"),
            vec!["wizard".to_string(), "Wizard".to_string()]
        );
        assert!(unknown("from:alex").is_empty());
        assert!(unknown("(tag:").is_empty(), "a query that does not parse names nothing");
    }

    #[test]
    fn automated_senders_and_lists_get_no_contact() {
        let s = crate::guide::tests::demo("contacts-people");
        let core = &s.1;
        for address in
            ["no-reply@shop.example", "notifications@github.example", "MAILER-DAEMON@example.com", "not an address"]
        {
            let err = block_on(core.contact_for_address(address.into())).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "{address}");
        }
        let db = core.db().unwrap();
        db.write_blocking(|tx| {
            tx.execute("INSERT INTO threads (gmail_id) VALUES ('t-list')", [])?;
            let thread = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO messages (thread_id, gmail_id, from_email, date, internal_date, list_id)
                 VALUES (?1, 'm-list', 'news@letters.example', 1, 1, 'news.letters.example')",
                [thread],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO contacts (email, received_count) VALUES ('news@letters.example', 1)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(block_on(core.contact_for_address("news@letters.example".into())).is_err(), "a list is not a person");
        // A list address the user has written to is a person after all.
        db.write_blocking(|tx| {
            tx.execute("UPDATE contacts SET sent_count = 1 WHERE email = 'news@letters.example'", [])?;
            Ok(())
        })
        .unwrap();
        assert!(block_on(core.contact_for_address("news@letters.example".into())).is_ok());
    }
}
