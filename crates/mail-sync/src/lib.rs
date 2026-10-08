//! Sync engine, outbox and backfill scheduler (spec §7.4).

mod attachments;
mod compose;
mod convert;
mod engine;
mod error;
pub mod import;
mod outbox;
pub mod transport;

pub use attachments::{AttachmentFile, attachment_file, safe_filename};
pub use compose::{draft_for_editing, forward_draft, reply_draft, schedule_draft_sync, send_draft};
pub use convert::to_incoming;
pub use engine::{
    BACKFILL_BATCH, BodyWindow, ExternalLabelChange, INBOX_PHASES, IncrementalReport, KEY_BODY_WINDOW,
    KEY_CLEANUP_HEADERS, KEY_WINDOW, NewMail, Phase, SyncEngine, SyncObserver, SyncPhase, SyncProgress, SyncWindow,
    cleanup_headers_paused, load_every_header_stored, load_waiting_headers_stored, phases_for,
};
pub use error::{SyncError, SyncResult};
pub use mail_store::undo::MessageDiff;
pub use outbox::{
    AppliedChange, DrainReport, LocalChange, MAX_ATTEMPTS, apply_local_change, apply_local_change_recorded, now_millis,
};
