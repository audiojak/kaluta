//! What an agent mailbox publishes to a rules server (spec §10.6): its
//! accepted guide entries with their scope and checks, its confirmed
//! audience groups with every member hashed, and the facts shared with
//! cloud agents. Never evidence, examples or mail.
//!
//! The JSON is the publish format. `schema_version` names its shape: a
//! change that an older reader would misread raises it, and a reader
//! refuses a version it does not know.

use serde::{Deserialize, Serialize};

use crate::audience::AudienceGroups;
use crate::facts::{Fact, fact_lines, facts_lookup, with_facts};
use crate::guide::{CheckFailure, Entry, Rendered, Target, check, render};

/// The snapshot shape this crate reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// Whose guide it is.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Mailbox {
    /// The agent mailbox's address.
    pub address: String,
    /// The name it sends as.
    pub name: String,
    /// What the agent is told about the mailbox and its service's limits,
    /// as rendered by the app.
    #[serde(default)]
    pub about: String,
}

/// One published version of an agent mailbox's guide and shared facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// [`SCHEMA_VERSION`] when written.
    pub schema_version: u32,
    /// The snapshot's version: only goes up, one per push.
    pub version: i64,
    /// The accepted guide's version the entries are from.
    pub guide_version: i64,
    /// When the app published it, in milliseconds since the Unix epoch.
    pub published_at: i64,
    pub mailbox: Mailbox,
    /// Accepted rules and guidelines, with scope and checks.
    #[serde(default)]
    pub entries: Vec<Entry>,
    /// Confirmed audience groups, every member hashed.
    #[serde(default)]
    pub audiences: AudienceGroups,
    /// The facts shared with cloud agents.
    #[serde(default)]
    pub facts: Vec<Fact>,
}

/// Why a published snapshot was refused.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("not a snapshot: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("snapshot schema_version {0} is not one this reader knows (it knows {SCHEMA_VERSION})")]
    UnsupportedSchema(u64),
    #[error("snapshot has no schema_version")]
    MissingSchema,
    #[error("audience groups list addresses: members must be published as salted hashes")]
    PlainAddresses,
}

impl Snapshot {
    /// Read a published snapshot, refusing an unknown schema and audience
    /// members that are not hashed.
    pub fn from_json(json: &str) -> Result<Self, SnapshotError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        match value.get("schema_version").map(serde_json::Value::as_u64) {
            None => return Err(SnapshotError::MissingSchema),
            Some(Some(v)) if v == u64::from(SCHEMA_VERSION) => {}
            Some(Some(v)) => return Err(SnapshotError::UnsupportedSchema(v)),
            Some(None) => return Err(SnapshotError::MissingSchema),
        }
        let snapshot: Self = serde_json::from_value(value)?;
        if snapshot.audiences.salt.is_none() && snapshot.audiences.groups.iter().any(|g| !g.members.is_empty()) {
            return Err(SnapshotError::PlainAddresses);
        }
        Ok(snapshot)
    }

    /// The publish format.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// The guide for a message (or, with no target, a session), with the
    /// shared facts after it: `guide_rules`' `writing_guide`.
    pub fn guide(&self, target: Option<&Target>) -> Rendered {
        let (text, audiences) = render(&self.entries, &self.audiences, target, &[], self.guide_version);
        Rendered { text: with_facts(text, &fact_lines(&self.facts)), version: self.guide_version, audiences }
    }

    /// What a draft's own text breaks: `check_draft`'s `guide_check`.
    pub fn check(&self, target: &Target, text: &str) -> Vec<CheckFailure> {
        check(&self.entries, &self.audiences, target, text)
    }

    /// The answer to `facts_lookup`, from the shared facts.
    pub fn facts_lookup(&self, category: Option<&str>, query: Option<&str>) -> serde_json::Value {
        facts_lookup(&self.facts, category, query)
    }
}
