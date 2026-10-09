//! The writing guide as agents read it (spec §14.9, §14.11, §10.6): the
//! accepted guide rendered as instructions for one message or a whole
//! session, the deterministic check an AI draft is put through (banned and
//! required phrases, length), the facts drafting may use, and the snapshot
//! an agent mailbox publishes to a rules server.
//!
//! Pure and synchronous: no store, no runtime, no UniFFI. `openagc-core`
//! converts its own records into these types and calls the same functions
//! the rules server calls on a published [`Snapshot`], so mailbox mode, the
//! in-app tools and the server answer alike.
//!
//! What goes in is already chosen by the caller: only accepted entries,
//! only confirmed audience groups, and only the facts drafting may see
//! (never *Never share* ones).

mod audience;
mod facts;
mod guide;
mod snapshot;

pub use audience::{AudienceGroup, AudienceGroups, hash_address, is_hash, is_member};
pub use facts::{Fact, fact_line, fact_lines, facts_lookup, with_facts};
pub use guide::{
    Check, CheckFailure, CheckKind, Entry, Kind, Rendered, Scope, Target, applies, check, contains_phrase, fenced,
    render, scope_text,
};
pub use snapshot::{Mailbox, SCHEMA_VERSION, Snapshot, SnapshotError};
