//! SQLite mail store: schema, migrations, queries, full-text search
//! (spec §6). The store is the only writer of the database; every write
//! that changes messages also maintains the denormalized thread data and
//! search index in the same transaction.

pub mod agents;
pub mod analysis;
pub mod calendar;
pub mod cleanup;
pub mod cloud_reports;
pub mod compositions;
pub mod consistency;
mod db;
pub mod demo;
pub mod demo_workflows;
pub mod drafts;
mod error;
pub mod facts;
pub mod guide;
pub mod outbox;
pub mod priority;
pub mod queue;
pub mod read;
pub mod reading;
pub mod routine_proposals;
pub mod routines;
pub mod search;
pub mod tasks;
pub mod undo;
pub mod waiting;
pub mod workflows;
mod write;

pub use db::{Db, READER_COUNT, schema_version};
pub use error::{StoreError, StoreResult};
pub use read::ThreadPage;
/// The writer's transaction, for callers that compose store functions in one.
pub use rusqlite::{Connection, Transaction};
pub use write::{
    ARCHIVE_LABEL, IncomingAttachment, IncomingMessage, LOCAL_PREFIX, MailWriter, MailboxChange, StoredLabels,
    ThreadChanges,
};
