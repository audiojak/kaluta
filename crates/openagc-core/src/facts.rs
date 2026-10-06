//! Facts about the user (spec §14.11): the built-in categories, custom
//! ones and starter sets, editing facts and categories as undoable changes
//! (ADR 0006), and what drafting is told. Facts replace the writing
//! guide's F3 category.

use std::collections::BTreeSet;

use mail_store::facts::{self as store, CategoryRow, FactEvidence, FactRow, Snapshot};

use crate::{Core, CoreError, CoreEvent, ErrorKind, runtime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FactUse {
    /// Given to drafting as a fact.
    Free,
    /// Listed with "ask the user before using".
    Ask,
    /// Never given to drafting.
    Never,
}

impl FactUse {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Free => "free",
            Self::Ask => "ask",
            Self::Never => "never",
        }
    }

    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "ask" => Self::Ask,
            "never" => Self::Never,
            _ => Self::Free,
        }
    }
}

/// Where a fact lives (ADR 0012): this account, or every account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FactScope {
    Account,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FactSource {
    You,
    Learned,
    WritingHelp,
}

impl FactSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::You => "you",
            Self::Learned => "learned",
            Self::WritingHelp => "writing_help",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "learned" => Self::Learned,
            "writing_help" => Self::WritingHelp,
            _ => Self::You,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FactStatus {
    Proposed,
    Accepted,
    Rejected,
}

impl FactStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    fn parse(s: &str) -> Self {
        match s {
            "proposed" => Self::Proposed,
            "rejected" => Self::Rejected,
            _ => Self::Accepted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FactQuote {
    pub message_id: String,
    pub quote: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FactInfo {
    pub id: i64,
    /// A category key: built-in (`work`) or custom.
    pub category: String,
    pub label: String,
    pub value: String,
    pub use_: FactUse,
    /// When it was true, if it ages.
    pub as_of: Option<i64>,
    pub source: FactSource,
    pub status: FactStatus,
    pub evidence: Vec<FactQuote>,
    pub created_at: i64,
    pub updated_at: i64,
    pub scope: FactScope,
    /// A global fact this account has its own fact for (same category and
    /// label): the account's wins.
    pub overridden: bool,
    /// Its `as_of` is old enough that it may no longer be true.
    pub stale: bool,
}

/// A fact dated longer ago than this is flagged for review.
pub const STALE_AFTER_MS: i64 = 180 * 24 * 60 * 60 * 1000;

/// What the user writes or changes on a fact.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FactFields {
    pub category: String,
    pub label: String,
    pub value: String,
    pub use_: FactUse,
    pub as_of: Option<i64>,
}

/// One change in a set applied (and undone) together.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum FactEdit {
    Add { fields: FactFields, status: FactStatus, source: FactSource },
    Update { id: i64, fields: FactFields },
    Decide { id: i64, status: FactStatus },
    Delete { id: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum CategoryEdit {
    /// A custom category; returns its key in the change's categories.
    Add {
        name: String,
        description: String,
    },
    Update {
        key: String,
        name: String,
        description: String,
    },
    /// Built-ins can be hidden, not renamed or deleted.
    Hide {
        key: String,
        hidden: bool,
    },
    /// Custom categories in this order (after the built-ins).
    Reorder {
        keys: Vec<String>,
    },
    /// A custom category; its facts move to Other.
    Delete {
        key: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FactCategoryInfo {
    pub key: String,
    pub name: String,
    pub description: String,
    pub builtin: bool,
    pub hidden: bool,
    pub default_use: FactUse,
    /// Labels it suggests ("Time zone"), for built-ins.
    pub suggested_labels: Vec<String>,
    pub starter: Option<String>,
    /// A custom category in the global store.
    pub global: bool,
}

/// A change applied: its id for undo and redo, and what it touched.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FactChange {
    pub change_id: i64,
    pub facts: Vec<FactInfo>,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum StarterSet {
    Business,
    Freelance,
    Household,
    JobSearch,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StarterSetInfo {
    pub set: StarterSet,
    pub name: String,
    pub categories: Vec<String>,
}

struct Builtin {
    key: &'static str,
    name: &'static str,
    description: &'static str,
    labels: &'static [&'static str],
    default_use: FactUse,
}

/// The categories every account has: the few that fit anyone, personal
/// or for work (spec §14.11).
const BUILTINS: &[Builtin] = &[
    Builtin {
        key: "identity",
        name: "Identity",
        description: "Who the user is: their name and how to refer to them",
        labels: &["Full name", "Preferred name", "Pronouns", "Name pronunciation"],
        default_use: FactUse::Free,
    },
    Builtin {
        key: "contact",
        name: "Contact",
        description: "How to reach the user",
        labels: &["Phone", "Other email addresses", "Mailing address", "Website or profiles"],
        default_use: FactUse::Free,
    },
    Builtin {
        key: "availability",
        name: "Availability",
        description: "When and where the user can be reached or meet",
        labels: &["Time zone", "Usual hours", "Calendar link", "Where I usually am", "Away or travel dates"],
        default_use: FactUse::Free,
    },
    Builtin {
        key: "people",
        name: "People",
        description: "Who the people the user mentions are to them, and how to refer to them",
        labels: &[],
        default_use: FactUse::Ask,
    },
    Builtin {
        key: "work",
        name: "Work",
        description: "The user's occupation, organisation and team",
        labels: &["Occupation or role", "Organisation", "Team"],
        default_use: FactUse::Free,
    },
    Builtin {
        key: "preferences",
        name: "Preferences",
        description: "How the user likes to be reached or to meet, and what to keep in mind when making plans",
        labels: &[],
        default_use: FactUse::Free,
    },
    Builtin {
        key: "other",
        name: "Other",
        description: "Anything that fits nowhere else",
        labels: &[],
        default_use: FactUse::Free,
    },
];

/// (set, its name, its categories: name, description, default use).
type Starter = (StarterSet, &'static str, &'static [(&'static str, &'static str, FactUse)]);

const STARTERS: &[Starter] = &[
    (
        StarterSet::Business,
        "Business",
        &[
            ("Company", "The company: name, what it does, founded, size, offices, website", FactUse::Free),
            ("Products and services", "What the company sells or offers", FactUse::Free),
            ("Customers and markets", "Who the company serves, and where", FactUse::Free),
            ("Pricing and terms", "Prices, discounts, payment and contract terms", FactUse::Ask),
            ("Funding and investors", "Funding rounds, investors and figures", FactUse::Ask),
            ("Policies and support", "Support hours, response times, refunds, compliance", FactUse::Free),
            ("Approved wording", "Boilerplate, taglines and disclaimers to use word for word", FactUse::Free),
            ("Links and resources", "Links worth sharing: decks, docs, booking pages", FactUse::Free),
        ],
    ),
    (
        StarterSet::Freelance,
        "Freelance or consulting",
        &[
            ("Services and rates", "What the user offers and what it costs", FactUse::Ask),
            ("Portfolio and references", "Past work and people who can vouch for it", FactUse::Free),
            ("Availability for new work", "When the user can take on new work", FactUse::Free),
        ],
    ),
    (
        StarterSet::Household,
        "Household",
        &[
            ("Home", "The home: address details and service providers", FactUse::Free),
            ("Family logistics", "School, activities and who does what", FactUse::Ask),
            ("Health providers", "Doctors and other providers, names only", FactUse::Ask),
        ],
    ),
    (
        StarterSet::JobSearch,
        "Job search",
        &[
            ("Experience and skills", "The user's background and strengths", FactUse::Free),
            ("Roles I'm looking for", "The roles, places and terms the user wants", FactUse::Free),
            ("References", "People who will vouch for the user", FactUse::Ask),
        ],
    ),
];

fn builtin(key: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.key == key)
}

fn invalid(message: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::InvalidInput, message)
}

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Lower case, letters and digits only: how names are compared.
fn squash(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Whether two category names are close enough to be the same one
/// ("Contact info" and "Contact").
pub(crate) fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (squash(a), squash(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a == b || (a.len().min(b.len()) >= 5 && (a.starts_with(&b) || b.starts_with(&a)))
}

pub(crate) fn info(f: FactRow, evidence: Vec<FactEvidence>) -> FactInfo {
    FactInfo {
        id: f.id,
        category: f.category,
        label: f.label,
        value: f.value,
        use_: FactUse::parse(&f.use_),
        as_of: f.as_of,
        source: FactSource::parse(&f.source),
        status: FactStatus::parse(&f.status),
        evidence: evidence.into_iter().map(|e| FactQuote { message_id: e.message_id, quote: e.quote }).collect(),
        created_at: f.created_at,
        updated_at: f.updated_at,
        scope: FactScope::Account,
        overridden: false,
        stale: f.as_of.is_some_and(|at| mail_sync::now_millis() - at > STALE_AFTER_MS),
    }
}

/// Every category, built-ins first then custom ones in the user's order.
pub(crate) fn categories_from(rows: &[CategoryRow]) -> Vec<FactCategoryInfo> {
    let mut out: Vec<FactCategoryInfo> = BUILTINS
        .iter()
        .map(|b| FactCategoryInfo {
            key: b.key.into(),
            name: b.name.into(),
            description: b.description.into(),
            builtin: true,
            hidden: rows.iter().any(|r| r.key == b.key && r.hidden),
            default_use: b.default_use,
            suggested_labels: b.labels.iter().map(|l| (*l).to_owned()).collect(),
            starter: None,
            global: false,
        })
        .collect();
    out.extend(rows.iter().filter(|r| !r.builtin).map(|r| FactCategoryInfo {
        key: r.key.clone(),
        name: r.name.clone(),
        description: r.description.clone(),
        builtin: false,
        hidden: r.hidden,
        default_use: FactUse::parse(&r.default_use),
        suggested_labels: vec![],
        starter: r.starter.clone(),
        global: false,
    }));
    out
}

/// A fact as drafting reads it, or `None` for one never shared.
pub(crate) fn prompt_line(f: &FactInfo, category: &str) -> Option<String> {
    let fact = format!("{category} › {}: {}", f.label, f.value);
    match f.use_ {
        FactUse::Free => Some(format!("- {}", crate::guide_ai::fenced(&fact))),
        FactUse::Ask => Some(format!("- {} (ask the user before using this)", crate::guide_ai::fenced(&fact))),
        FactUse::Never => None,
    }
}

impl Core {
    pub(crate) fn facts_changed(&self) {
        self.account_events().emit(CoreEvent::FactsChanged);
    }

    /// The accepted facts drafting may see, each as a prompt line.
    pub(crate) async fn fact_lines(&self) -> Result<Vec<String>, CoreError> {
        let categories = self.fact_categories().await?;
        let name = |key: &str| categories.iter().find(|c| c.key == key).map_or(key.to_owned(), |c| c.name.clone());
        Ok(self
            .list_facts(vec![FactStatus::Accepted])
            .await?
            .iter()
            .filter(|f| !f.overridden)
            .filter_map(|f| prompt_line(f, &name(&f.category)))
            .collect())
    }

    /// The store facts in `scope` live in.
    pub(crate) fn facts_db(&self, scope: FactScope) -> Result<mail_store::Db, CoreError> {
        match scope {
            FactScope::Account => self.db(),
            FactScope::Global => self.global_facts_db(),
        }
    }

    /// The global facts store, opened the first time it is needed (ADR
    /// 0012): `data_dir/global/facts.sqlite`, the account schema.
    pub(crate) fn global_facts_db(&self) -> Result<mail_store::Db, CoreError> {
        let _guard = self.global_facts_lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(db) = self.global_facts.get() {
            return Ok(db.clone());
        }
        let dir = self.data_path().join("global");
        std::fs::create_dir_all(&dir).map_err(|e| CoreError::new(ErrorKind::Storage, e.to_string()))?;
        let db = mail_store::Db::open(&dir.join("facts.sqlite"))?;
        let _ = self.global_facts.set(db.clone());
        Ok(db)
    }

    async fn facts_in(&self, scope: FactScope, statuses: &[FactStatus]) -> Result<Vec<FactInfo>, CoreError> {
        let wanted: Vec<&'static str> = statuses.iter().map(|s| s.as_str()).collect();
        let db = self.facts_db(scope)?;
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let mut out = Vec::new();
                    for f in store::list(c, &wanted)? {
                        let e = store::evidence(c, f.id)?;
                        out.push(FactInfo { scope, ..info(f, e) });
                    }
                    Ok(out)
                })
                .await?)
        })
        .await
    }

    async fn category_rows(&self, scope: FactScope) -> Result<Vec<CategoryRow>, CoreError> {
        let db = self.facts_db(scope)?;
        runtime::run(async move { Ok(db.read(store::categories).await?) }).await
    }

    /// The categories facts in `scope` can use: for an account, its own
    /// and the global custom ones; for the global store, its own.
    async fn categories_for(&self, scope: FactScope) -> Result<Vec<FactCategoryInfo>, CoreError> {
        let global = self.category_rows(FactScope::Global).await?;
        let mut out = match scope {
            FactScope::Account => categories_from(&self.category_rows(FactScope::Account).await?),
            FactScope::Global => categories_from(&global),
        };
        for c in categories_from(&global).into_iter().filter(|c| !c.builtin) {
            match out.iter_mut().find(|o| o.key == c.key) {
                Some(o) => o.global = true,
                None => out.push(FactCategoryInfo { global: true, ..c }),
            }
        }
        Ok(out)
    }

    /// Apply fact and category edits as one change, recorded for undo.
    async fn apply_facts(
        &self,
        scope: FactScope,
        edits: Vec<FactEdit>,
        category_edits: Vec<CategoryEdit>,
        reason: String,
    ) -> Result<FactChange, CoreError> {
        self.apply_facts_with(scope, edits, category_edits, reason, None).await
    }

    /// Apply fact and category edits, and decide Analysis proposals
    /// (`decide`: ids and their new status), as one change.
    pub(crate) async fn apply_facts_with(
        &self,
        scope: FactScope,
        edits: Vec<FactEdit>,
        category_edits: Vec<CategoryEdit>,
        reason: String,
        decide: Option<(Vec<i64>, &'static str)>,
    ) -> Result<FactChange, CoreError> {
        if edits.is_empty() && category_edits.is_empty() && decide.is_none() {
            return Err(invalid("nothing to change"));
        }
        // Validate before the store.
        let known: Vec<FactCategoryInfo> = self.categories_for(scope).await?;
        for e in &edits {
            if let FactEdit::Add { fields, .. } | FactEdit::Update { fields, .. } = e {
                if clean(&fields.label).is_empty() || fields.value.trim().is_empty() {
                    return Err(invalid("a fact needs a label and a value"));
                }
                let new_categories: Vec<String> = category_edits
                    .iter()
                    .filter_map(|c| match c {
                        CategoryEdit::Add { name, .. } => Some(name.clone()),
                        _ => None,
                    })
                    .collect();
                if !known.iter().any(|c| c.key == fields.category) && !new_categories.contains(&fields.category) {
                    return Err(invalid("choose one of your categories"));
                }
            }
        }
        let db = self.facts_db(scope)?;
        let now = mail_sync::now_millis();
        let change = runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    // Which rows the change touches, so its record has them.
                    let mut fact_ids: Vec<i64> = edits
                        .iter()
                        .filter_map(|e| match e {
                            FactEdit::Update { id, .. } | FactEdit::Decide { id, .. } | FactEdit::Delete { id } => {
                                Some(*id)
                            }
                            FactEdit::Add { .. } => None,
                        })
                        .collect();
                    let mut keys: Vec<String> = Vec::new();
                    let stored = store::categories(tx)?;
                    for c in &category_edits {
                        match c {
                            CategoryEdit::Update { key, .. } | CategoryEdit::Hide { key, .. } => keys.push(key.clone()),
                            CategoryEdit::Delete { key } => {
                                keys.push(key.clone());
                                fact_ids.extend(
                                    store::list(tx, &["proposed", "accepted", "rejected"])?
                                        .into_iter()
                                        .filter(|f| &f.category == key)
                                        .map(|f| f.id),
                                );
                            }
                            CategoryEdit::Reorder { keys: order } => keys.extend(order.iter().cloned()),
                            CategoryEdit::Add { .. } => {}
                        }
                    }
                    let mut before = store::snapshot(tx, &fact_ids, &keys)?;
                    let proposal_snapshots = |tx: &mail_store::Transaction<'_>| -> mail_store::StoreResult<_> {
                        let mut out = Vec::new();
                        for id in decide.iter().flat_map(|(ids, _)| ids) {
                            out.extend(mail_store::analysis::snapshot_proposal(tx, *id)?);
                        }
                        Ok(out)
                    };
                    before.proposals = proposal_snapshots(tx)?;
                    if let Some((ids, status)) = &decide {
                        for id in ids {
                            mail_store::analysis::set_proposal_status(tx, *id, status, now)?;
                        }
                    }
                    let invalid_store = |m: &str| mail_store::StoreError::Invalid(m.to_owned());
                    // Categories first: a fact may go into one made here
                    // (named by its name until it has a key).
                    let mut added: Vec<(String, String)> = Vec::new();
                    for c in &category_edits {
                        match c {
                            CategoryEdit::Add { name, description } => {
                                let name = clean(name);
                                if name.is_empty() {
                                    return Err(invalid_store("a category needs a name"));
                                }
                                let taken = BUILTINS.iter().any(|b| similar(b.name, &name))
                                    || stored.iter().any(|r| !r.builtin && similar(&r.name, &name));
                                if taken {
                                    return Err(invalid_store(&format!("you already have a category like “{name}”")));
                                }
                                let key = format!("c{now}{}", added.len());
                                let position =
                                    stored.iter().map(|r| r.position).max().unwrap_or(0) + 1 + added.len() as i64;
                                store::put_category(
                                    tx,
                                    &CategoryRow {
                                        key: key.clone(),
                                        name: name.clone(),
                                        description: clean(description),
                                        position,
                                        builtin: false,
                                        hidden: false,
                                        default_use: "free".into(),
                                        starter: None,
                                        created_at: now,
                                    },
                                )?;
                                keys.push(key.clone());
                                added.push((name, key));
                            }
                            CategoryEdit::Update { key, name, description } => {
                                let mut row = store::get_category(tx, key)?
                                    .filter(|r| !r.builtin)
                                    .ok_or_else(|| invalid_store("built-in categories cannot be renamed"))?;
                                row.name = clean(name);
                                row.description = clean(description);
                                if row.name.is_empty() {
                                    return Err(invalid_store("a category needs a name"));
                                }
                                store::put_category(tx, &row)?;
                            }
                            CategoryEdit::Hide { key, hidden } => {
                                let row = match store::get_category(tx, key)? {
                                    Some(mut r) => {
                                        r.hidden = *hidden;
                                        r
                                    }
                                    None => {
                                        let b = builtin(key).ok_or_else(|| invalid_store("no such category"))?;
                                        CategoryRow {
                                            key: b.key.into(),
                                            name: b.name.into(),
                                            builtin: true,
                                            hidden: *hidden,
                                            default_use: b.default_use.as_str().into(),
                                            created_at: now,
                                            ..Default::default()
                                        }
                                    }
                                };
                                store::put_category(tx, &row)?;
                            }
                            CategoryEdit::Reorder { keys: order } => {
                                for (i, key) in order.iter().enumerate() {
                                    if let Some(mut r) = store::get_category(tx, key)?.filter(|r| !r.builtin) {
                                        r.position = i as i64;
                                        store::put_category(tx, &r)?;
                                    }
                                }
                            }
                            CategoryEdit::Delete { key } => {
                                if builtin(key).is_some() {
                                    return Err(invalid_store("built-in categories cannot be deleted; hide them"));
                                }
                                for f in store::list(tx, &["proposed", "accepted", "rejected"])? {
                                    if &f.category == key {
                                        store::write(tx, &FactRow { category: "other".into(), updated_at: now, ..f })?;
                                    }
                                }
                                store::delete_category(tx, key)?;
                            }
                        }
                    }
                    let resolve = |category: &str| -> String {
                        added.iter().find(|(name, _)| name == category).map_or(category.to_owned(), |(_, k)| k.clone())
                    };
                    for e in &edits {
                        match e {
                            FactEdit::Add { fields, status, source } => {
                                let id = store::insert(
                                    tx,
                                    &FactRow {
                                        id: 0,
                                        category: resolve(&fields.category),
                                        label: clean(&fields.label),
                                        value: fields.value.trim().to_owned(),
                                        use_: fields.use_.as_str().into(),
                                        as_of: fields.as_of,
                                        source: source.as_str().into(),
                                        status: status.as_str().into(),
                                        created_at: now,
                                        updated_at: now,
                                    },
                                )?;
                                fact_ids.push(id);
                            }
                            FactEdit::Update { id, fields } => {
                                let old =
                                    store::get(tx, *id)?.ok_or_else(|| invalid_store("that fact no longer exists"))?;
                                store::write(
                                    tx,
                                    &FactRow {
                                        category: resolve(&fields.category),
                                        label: clean(&fields.label),
                                        value: fields.value.trim().to_owned(),
                                        use_: fields.use_.as_str().into(),
                                        as_of: fields.as_of,
                                        updated_at: now,
                                        ..old
                                    },
                                )?;
                            }
                            FactEdit::Decide { id, status } => {
                                let old =
                                    store::get(tx, *id)?.ok_or_else(|| invalid_store("that fact no longer exists"))?;
                                store::write(tx, &FactRow { status: status.as_str().into(), updated_at: now, ..old })?;
                            }
                            FactEdit::Delete { id } => store::delete(tx, *id)?,
                        }
                    }
                    // One fact per label in a category.
                    let accepted = store::list(tx, &["accepted"])?;
                    let mut seen = BTreeSet::new();
                    for f in &accepted {
                        if !seen.insert((f.category.clone(), f.label.to_lowercase())) {
                            return Err(invalid_store(&format!("you already have “{}” in that category", f.label)));
                        }
                    }
                    let mut after = store::snapshot(tx, &fact_ids, &keys)?;
                    after.proposals = proposal_snapshots(tx)?;
                    let change_id = store::record_change(tx, &reason, &before, &after, now)?;
                    Ok(FactChange {
                        change_id,
                        facts: after.facts.into_iter().map(|(f, e)| FactInfo { scope, ..info(f, e) }).collect(),
                        categories: keys,
                    })
                })
                .await?)
        })
        .await?;
        self.facts_changed();
        Ok(change)
    }

    async fn replay_facts(&self, scope: FactScope, change_id: i64, undo: bool) -> Result<(), CoreError> {
        let db = self.facts_db(scope)?;
        let now = mail_sync::now_millis();
        let global_side = runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    let (_, before, after) = store::get_change(tx, change_id)?
                        .ok_or_else(|| mail_store::StoreError::Invalid("that change can no longer be undone".into()))?;
                    let (ids, keys) = touched(&before.facts, &after.facts, &before.categories, &after.categories);
                    let to = if undo { &before } else { &after };
                    store::restore(tx, &ids, &keys, to, now)?;
                    // A move between stores: the global side goes back too.
                    let (gids, gkeys) = touched(
                        &before.global_facts,
                        &after.global_facts,
                        &before.global_categories,
                        &after.global_categories,
                    );
                    let global = Snapshot {
                        facts: to.global_facts.clone(),
                        categories: to.global_categories.clone(),
                        ..Default::default()
                    };
                    Ok((gids, gkeys, global))
                })
                .await?)
        })
        .await?;
        let (gids, gkeys, global) = global_side;
        if !gids.is_empty() || !gkeys.is_empty() {
            let gdb = self.global_facts_db()?;
            runtime::run(
                async move { Ok(gdb.write(move |tx| store::restore(tx, &gids, &gkeys, &global, now)).await?) },
            )
            .await?;
        }
        self.facts_changed();
        Ok(())
    }

    /// Move a fact between this account and the global store: one change,
    /// recorded in the account's store with both sides (ADR 0012). Its
    /// custom category goes with it (copied, if the other side lacks it).
    async fn move_fact(&self, id: i64, to: FactScope) -> Result<FactChange, CoreError> {
        let from = if to == FactScope::Global { FactScope::Account } else { FactScope::Global };
        let (src, dst) = (self.facts_db(from)?, self.facts_db(to)?);
        let (fact, quotes, category) = runtime::run(async move {
            Ok(src
                .read(move |c| {
                    let f = store::get(c, id)?
                        .ok_or_else(|| mail_store::StoreError::Invalid("that fact is gone".into()))?;
                    let e = store::evidence(c, id)?;
                    let cat = if builtin(&f.category).is_none() { store::get_category(c, &f.category)? } else { None };
                    Ok((f, e, cat))
                })
                .await?)
        })
        .await?;
        let now = mail_sync::now_millis();
        // The other side first: a failure after it leaves a copy, never a loss.
        let (f2, q2, c2) = (fact.clone(), quotes.clone(), category.clone());
        let (moved, cat_before, cat_after) = runtime::run(async move {
            Ok(dst
                .write(move |tx| {
                    let dup = store::list(tx, &["accepted"])?.into_iter().any(|o| {
                        o.category == f2.category && o.label.eq_ignore_ascii_case(&f2.label) && f2.status == "accepted"
                    });
                    if dup {
                        return Err(mail_store::StoreError::Invalid(format!(
                            "“{}” is there already; change or delete one first",
                            f2.label
                        )));
                    }
                    let cat_before = match &c2 {
                        Some(c) => store::get_category(tx, &c.key)?,
                        None => None,
                    };
                    if let Some(c) = &c2
                        && cat_before.is_none()
                    {
                        store::put_category(tx, c)?;
                    }
                    let new_id = store::insert(tx, &FactRow { id: 0, updated_at: now, ..f2 })?;
                    // Quotes are from one account's mail: the global copy
                    // has none (ADR 0012); the change record keeps them.
                    if to == FactScope::Account {
                        store::add_evidence(tx, new_id, &q2, now)?;
                    }
                    let moved = (store::get(tx, new_id)?.unwrap_or_default(), store::evidence(tx, new_id)?);
                    let cat_after = match &c2 {
                        Some(c) => store::get_category(tx, &c.key)?,
                        None => None,
                    };
                    Ok((moved, cat_before, cat_after))
                })
                .await?)
        })
        .await?;
        let acc = self.db()?;
        let reason = if to == FactScope::Global { "make global" } else { "make this account's only" };
        let (fact2, quotes2, moved2) = (fact.clone(), quotes.clone(), moved.clone());
        let change_id = runtime::run(async move {
            Ok(acc
                .write(move |tx| {
                    // This account's side, and the global side, before and after.
                    let (mut before, mut after) = (Snapshot::default(), Snapshot::default());
                    let gone = (fact2, quotes2);
                    if to == FactScope::Global {
                        before.facts.push(gone.clone());
                        store::delete(tx, gone.0.id)?;
                        after.global_facts.push(moved2);
                        before.global_categories.extend(cat_before);
                        after.global_categories.extend(cat_after);
                    } else {
                        before.global_facts.push(gone);
                        after.facts.push(moved2);
                        before.categories.extend(cat_before);
                        after.categories.extend(cat_after);
                    }
                    store::record_change(tx, reason, &before, &after, now)
                })
                .await?)
        })
        .await?;
        if to == FactScope::Account {
            let gdb = self.global_facts_db()?;
            runtime::run(async move { Ok(gdb.write(move |tx| store::delete(tx, id)).await?) }).await?;
        }
        self.facts_changed();
        Ok(FactChange {
            change_id,
            facts: vec![FactInfo { scope: to, ..info(moved.0, moved.1) }],
            categories: category.map(|c| vec![c.key]).unwrap_or_default(),
        })
    }
}

/// The fact ids and category keys on both sides of a change.
fn touched(
    a: &[(FactRow, Vec<FactEvidence>)],
    b: &[(FactRow, Vec<FactEvidence>)],
    ca: &[CategoryRow],
    cb: &[CategoryRow],
) -> (Vec<i64>, Vec<String>) {
    let ids: BTreeSet<i64> = a.iter().chain(b).map(|(f, _)| f.id).collect();
    let keys: BTreeSet<String> = ca.iter().chain(cb).map(|c| c.key.clone()).collect();
    (ids.into_iter().collect(), keys.into_iter().collect())
}

/// What a fact says to an agent's `facts_lookup` call.
pub(crate) fn lookup_json(
    facts: &[FactInfo],
    categories: &[FactCategoryInfo],
    category: Option<&str>,
    query: Option<&str>,
) -> serde_json::Value {
    let want = category.map(str::to_lowercase);
    let q = query.map(str::to_lowercase).filter(|q| !q.trim().is_empty());
    let rows: Vec<serde_json::Value> = facts
        .iter()
        .filter(|f| f.status == FactStatus::Accepted && f.use_ != FactUse::Never && !f.overridden)
        .filter_map(|f| {
            let c = categories.iter().find(|c| c.key == f.category);
            let name = c.map_or(f.category.clone(), |c| c.name.clone());
            if let Some(w) = &want
                && *w != f.category.to_lowercase()
                && *w != name.to_lowercase()
            {
                return None;
            }
            if let Some(q) = &q
                && !format!("{name} {} {}", f.label, f.value).to_lowercase().contains(q.as_str())
            {
                return None;
            }
            Some(serde_json::json!({
                "category": name,
                "label": f.label,
                "value": f.value,
                "ask_before_using": f.use_ == FactUse::Ask,
            }))
        })
        .collect();
    serde_json::json!({ "facts": rows })
}

#[uniffi::export]
impl Core {
    /// The account's facts in any of `statuses`, then the global ones
    /// (ADR 0012), with evidence. A global fact the account has its own
    /// fact for (same category and label) is marked overridden.
    pub async fn list_facts(&self, statuses: Vec<FactStatus>) -> Result<Vec<FactInfo>, CoreError> {
        let mine = self.facts_in(FactScope::Account, &statuses).await?;
        let mut global = self.facts_in(FactScope::Global, &statuses).await?;
        for g in &mut global {
            g.overridden = g.status == FactStatus::Accepted
                && mine.iter().any(|m| {
                    m.status == FactStatus::Accepted
                        && m.category == g.category
                        && m.label.eq_ignore_ascii_case(&g.label)
                });
        }
        Ok(mine.into_iter().chain(global).collect())
    }

    /// The global facts only (Settings › Facts).
    pub async fn list_global_facts(&self, statuses: Vec<FactStatus>) -> Result<Vec<FactInfo>, CoreError> {
        self.facts_in(FactScope::Global, &statuses).await
    }

    /// Every category: the built-ins, then the user's own in their order,
    /// with the global custom ones.
    pub async fn fact_categories(&self) -> Result<Vec<FactCategoryInfo>, CoreError> {
        self.categories_for(FactScope::Account).await
    }

    /// The global store's categories (Settings › Facts).
    pub async fn global_fact_categories(&self) -> Result<Vec<FactCategoryInfo>, CoreError> {
        self.categories_for(FactScope::Global).await
    }

    /// Change global facts (Settings › Facts); one change, recorded in the
    /// global store (`undo_global_fact_change`).
    pub async fn apply_global_fact_edits(&self, edits: Vec<FactEdit>, reason: String) -> Result<FactChange, CoreError> {
        self.apply_facts(FactScope::Global, edits, vec![], reason).await
    }

    pub async fn edit_global_fact_categories(&self, edits: Vec<CategoryEdit>) -> Result<FactChange, CoreError> {
        self.apply_facts(FactScope::Global, vec![], edits, "categories".into()).await
    }

    pub async fn undo_global_fact_change(&self, change_id: i64) -> Result<(), CoreError> {
        self.replay_facts(FactScope::Global, change_id, true).await
    }

    pub async fn redo_global_fact_change(&self, change_id: i64) -> Result<(), CoreError> {
        self.replay_facts(FactScope::Global, change_id, false).await
    }

    /// Make an account fact global: every account uses it. Undoable on the
    /// account's stack (`undo_fact_change`).
    pub async fn make_fact_global(&self, id: i64) -> Result<FactChange, CoreError> {
        self.move_fact(id, FactScope::Global).await
    }

    /// Make a global fact this account's only. Undoable on the account's
    /// stack.
    pub async fn make_fact_local(&self, global_id: i64) -> Result<FactChange, CoreError> {
        self.move_fact(global_id, FactScope::Account).await
    }

    /// The starter sets and the categories each adds.
    pub fn fact_starter_sets(&self) -> Vec<StarterSetInfo> {
        STARTERS
            .iter()
            .map(|(set, name, cats)| StarterSetInfo {
                set: *set,
                name: (*name).into(),
                categories: cats.iter().map(|c| c.0.to_owned()).collect(),
            })
            .collect()
    }

    /// A category close to `name`, if there is one: offered instead of a
    /// new one.
    pub async fn similar_fact_category(&self, name: String) -> Result<Option<FactCategoryInfo>, CoreError> {
        Ok(self.fact_categories().await?.into_iter().find(|c| similar(&c.name, &name)))
    }

    /// Change facts; one change, undoable (`undo_fact_change`).
    pub async fn apply_fact_edits(&self, edits: Vec<FactEdit>, reason: String) -> Result<FactChange, CoreError> {
        self.apply_facts(FactScope::Account, edits, vec![], reason).await
    }

    /// Change categories; one change, undoable. Deleting a custom
    /// category moves its facts to Other.
    pub async fn edit_fact_categories(&self, edits: Vec<CategoryEdit>) -> Result<FactChange, CoreError> {
        self.apply_facts(FactScope::Account, vec![], edits, "categories".into()).await
    }

    /// Add a starter set's categories (those the account does not have
    /// already); one change, undoable.
    pub async fn add_fact_starter_set(&self, set: StarterSet) -> Result<FactChange, CoreError> {
        let have = self.fact_categories().await?;
        let (_, name, cats) =
            STARTERS.iter().find(|(s, ..)| *s == set).ok_or_else(|| invalid("no such starter set"))?;
        let edits: Vec<CategoryEdit> = cats
            .iter()
            .filter(|(n, ..)| !have.iter().any(|c| similar(&c.name, n)))
            .map(|(n, d, _)| CategoryEdit::Add { name: (*n).into(), description: (*d).into() })
            .collect();
        if edits.is_empty() {
            return Err(invalid(format!("you already have the {name} categories")));
        }
        let change = self.apply_facts(FactScope::Account, vec![], edits, format!("starter set {name}")).await?;
        // Mark them as the set's, with its sensitive ones asking first.
        let db = self.db()?;
        let keys = change.categories.clone();
        let set_name = (*name).to_owned();
        let defaults: Vec<(&str, FactUse)> = cats.iter().map(|(n, _, u)| (*n, *u)).collect();
        let defaults: Vec<(String, FactUse)> = defaults.into_iter().map(|(n, u)| (n.to_owned(), u)).collect();
        runtime::run(async move {
            Ok(db
                .write(move |tx| {
                    for key in &keys {
                        if let Some(mut r) = store::get_category(tx, key)? {
                            r.starter = Some(set_name.clone());
                            if let Some((_, u)) = defaults.iter().find(|(n, _)| *n == r.name) {
                                r.default_use = u.as_str().into();
                            }
                            store::put_category(tx, &r)?;
                        }
                    }
                    Ok(())
                })
                .await?)
        })
        .await?;
        Ok(change)
    }

    pub async fn undo_fact_change(&self, change_id: i64) -> Result<(), CoreError> {
        self.replay_facts(FactScope::Account, change_id, true).await
    }

    pub async fn redo_fact_change(&self, change_id: i64) -> Result<(), CoreError> {
        self.replay_facts(FactScope::Account, change_id, false).await
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;

    use super::*;

    fn fields(category: &str, label: &str, value: &str, use_: FactUse) -> FactFields {
        FactFields { category: category.into(), label: label.into(), value: value.into(), use_, as_of: None }
    }

    fn add(core: &Core, f: FactFields) -> FactChange {
        block_on(core.apply_fact_edits(
            vec![FactEdit::Add { fields: f, status: FactStatus::Accepted, source: FactSource::You }],
            "add".into(),
        ))
        .unwrap()
    }

    #[test]
    fn facts_are_added_kept_unique_and_undone() {
        let s = crate::guide::tests::demo("facts-edit");
        let core = &s.1;
        let change = add(core, fields("work", "Team", "Mail", FactUse::Free));
        let id = change.facts[0].id;
        let dup = block_on(core.apply_fact_edits(
            vec![FactEdit::Add {
                fields: fields("work", "team", "Other", FactUse::Free),
                status: FactStatus::Accepted,
                source: FactSource::You,
            }],
            "add".into(),
        ))
        .unwrap_err();
        assert_eq!(dup.kind(), ErrorKind::InvalidInput, "one fact per label in a category");
        assert!(
            block_on(core.apply_fact_edits(
                vec![FactEdit::Add {
                    fields: fields("nowhere", "x", "y", FactUse::Free),
                    status: FactStatus::Accepted,
                    source: FactSource::You
                }],
                "add".into()
            ))
            .is_err()
        );
        let edit = block_on(core.apply_fact_edits(
            vec![FactEdit::Update { id, fields: fields("work", "Team", "Mail and Calendar", FactUse::Ask) }],
            "edit".into(),
        ))
        .unwrap();
        block_on(core.undo_fact_change(edit.change_id)).unwrap();
        let now = block_on(core.list_facts(vec![FactStatus::Accepted])).unwrap();
        assert_eq!((now[0].value.as_str(), now[0].use_), ("Mail", FactUse::Free));
        block_on(core.undo_fact_change(change.change_id)).unwrap();
        assert!(block_on(core.list_facts(vec![FactStatus::Accepted])).unwrap().is_empty());
        block_on(core.redo_fact_change(change.change_id)).unwrap();
        assert_eq!(block_on(core.list_facts(vec![FactStatus::Accepted])).unwrap()[0].id, id);
    }

    #[test]
    fn custom_categories_and_starter_sets() {
        let s = crate::guide::tests::demo("facts-categories");
        let core = &s.1;
        let cats = block_on(core.fact_categories()).unwrap();
        assert_eq!(
            cats.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
            ["identity", "contact", "availability", "people", "work", "preferences", "other"]
        );
        assert_eq!(cats.iter().find(|c| c.key == "people").unwrap().default_use, FactUse::Ask);
        let near = block_on(core.similar_fact_category("Contact info".into())).unwrap();
        assert_eq!(near.map(|c| c.key), Some("contact".into()), "a close name offers the one there is");
        assert!(
            block_on(core.edit_fact_categories(vec![CategoryEdit::Add {
                name: "contact info".into(),
                description: String::new()
            }]))
            .is_err()
        );

        let made = block_on(core.edit_fact_categories(vec![CategoryEdit::Add {
            name: "Properties".into(),
            description: "Properties I'm currently selling".into(),
        }]))
        .unwrap();
        let key = made.categories[0].clone();
        let fact = add(core, fields(&key, "Elm Street", "3 bedrooms", FactUse::Free)).facts[0].id;
        let gone = block_on(core.edit_fact_categories(vec![CategoryEdit::Delete { key: key.clone() }])).unwrap();
        let moved = block_on(core.list_facts(vec![FactStatus::Accepted])).unwrap();
        assert_eq!((moved[0].id, moved[0].category.as_str()), (fact, "other"), "its facts move to Other");
        block_on(core.undo_fact_change(gone.change_id)).unwrap();
        assert_eq!(block_on(core.list_facts(vec![FactStatus::Accepted])).unwrap()[0].category, key);
        assert!(block_on(core.fact_categories()).unwrap().iter().any(|c| c.key == key));
        assert!(block_on(core.edit_fact_categories(vec![CategoryEdit::Delete { key: "work".into() }])).is_err());
        block_on(core.edit_fact_categories(vec![CategoryEdit::Hide { key: "work".into(), hidden: true }])).unwrap();
        assert!(block_on(core.fact_categories()).unwrap().iter().find(|c| c.key == "work").unwrap().hidden);

        let business = block_on(core.add_fact_starter_set(StarterSet::Business)).unwrap();
        assert_eq!(business.categories.len(), 8);
        let cats = block_on(core.fact_categories()).unwrap();
        let pricing = cats.iter().find(|c| c.name == "Pricing and terms").unwrap();
        assert_eq!((pricing.default_use, pricing.starter.as_deref()), (FactUse::Ask, Some("Business")));
        assert!(block_on(core.add_fact_starter_set(StarterSet::Business)).is_err(), "nothing new to add");
    }

    #[test]
    fn drafting_sees_facts_by_their_use() {
        let s = crate::guide::tests::demo("facts-render");
        let core = &s.1;
        add(core, fields("work", "Occupation or role", "CEO", FactUse::Free));
        add(core, fields("people", "Sam", "My assistant", FactUse::Ask));
        add(core, fields("contact", "Mailing address", "1 Main St", FactUse::Never));
        let lines = block_on(core.fact_lines()).unwrap();
        assert_eq!(
            lines,
            vec!["- People › Sam: My assistant (ask the user before using this)", "- Work › Occupation or role: CEO"]
        );
        let facts = block_on(core.list_facts(vec![FactStatus::Accepted])).unwrap();
        let cats = block_on(core.fact_categories()).unwrap();
        let all = lookup_json(&facts, &cats, None, None);
        assert_eq!(all["facts"].as_array().unwrap().len(), 2, "never-share facts are not given out");
        let work = lookup_json(&facts, &cats, Some("Work"), None);
        assert_eq!(work["facts"][0]["value"], "CEO");
        assert_eq!(lookup_json(&facts, &cats, None, Some("assistant"))["facts"][0]["ask_before_using"], true);
    }

    #[test]
    fn a_fact_made_global_moves_and_undo_brings_it_back() {
        let s = crate::guide::tests::demo("facts-global");
        let core = &s.1;
        let made = block_on(core.edit_fact_categories(vec![CategoryEdit::Add {
            name: "Company".into(),
            description: "The company".into(),
        }]))
        .unwrap();
        let key = made.categories[0].clone();
        let id = add(core, fields(&key, "Name", "Actual AI", FactUse::Free)).facts[0].id;

        let change = block_on(core.make_fact_global(id)).unwrap();
        assert!(s.0.join("global").join("facts.sqlite").exists(), "the global store, in the data directory");
        let all = block_on(core.list_facts(vec![FactStatus::Accepted])).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!((all[0].scope, all[0].value.as_str()), (FactScope::Global, "Actual AI"));
        let gid = all[0].id;
        let global_cats = block_on(core.global_fact_categories()).unwrap();
        assert!(global_cats.iter().any(|c| c.key == key && c.global), "its custom category went with it");
        assert_eq!(block_on(core.fact_lines()).unwrap(), vec!["- Company › Name: Actual AI"]);
        assert!(all[0].evidence.is_empty(), "no account's quotes in the global store");

        block_on(core.undo_fact_change(change.change_id)).unwrap();
        let all = block_on(core.list_facts(vec![FactStatus::Accepted])).unwrap();
        assert_eq!(all.iter().map(|f| (f.id, f.scope)).collect::<Vec<_>>(), vec![(id, FactScope::Account)]);
        assert!(block_on(core.list_global_facts(vec![FactStatus::Accepted])).unwrap().is_empty());
        assert!(!block_on(core.global_fact_categories()).unwrap().iter().any(|c| c.key == key));
        block_on(core.redo_fact_change(change.change_id)).unwrap();
        assert_eq!(block_on(core.list_global_facts(vec![FactStatus::Accepted])).unwrap()[0].id, gid);

        // And back to this account only.
        let local = block_on(core.make_fact_local(gid)).unwrap();
        assert_eq!(local.facts[0].scope, FactScope::Account);
        assert!(block_on(core.list_global_facts(vec![FactStatus::Accepted])).unwrap().is_empty());
        block_on(core.undo_fact_change(local.change_id)).unwrap();
        assert_eq!(block_on(core.list_global_facts(vec![FactStatus::Accepted])).unwrap().len(), 1);
    }

    #[test]
    fn an_account_fact_overrides_the_global_one() {
        let s = crate::guide::tests::demo("facts-override");
        let core = &s.1;
        block_on(core.apply_global_fact_edits(
            vec![FactEdit::Add {
                fields: fields("availability", "Time zone", "Pacific", FactUse::Free),
                status: FactStatus::Accepted,
                source: FactSource::You,
            }],
            "add".into(),
        ))
        .unwrap();
        assert_eq!(block_on(core.fact_lines()).unwrap(), vec!["- Availability › Time zone: Pacific"]);
        add(core, fields("availability", "time zone", "Eastern", FactUse::Free));
        let all = block_on(core.list_facts(vec![FactStatus::Accepted])).unwrap();
        let global = all.iter().find(|f| f.scope == FactScope::Global).unwrap();
        assert!(global.overridden);
        assert_eq!(block_on(core.fact_lines()).unwrap(), vec!["- Availability › time zone: Eastern"]);
        let cats = block_on(core.fact_categories()).unwrap();
        assert_eq!(lookup_json(&all, &cats, Some("availability"), None)["facts"].as_array().unwrap().len(), 1);
        // Moving it up would make two: refused.
        let mine = all.iter().find(|f| f.scope == FactScope::Account).unwrap().id;
        assert!(block_on(core.make_fact_global(mine)).is_err());
    }
}
