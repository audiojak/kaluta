-- The account's drafts on the server, from drafts.list (spec §14.5
-- amendment 2026-09-28): which message each draft holds now. Gmail's
-- change history leaves drafts out, so they sync through this, and
-- opening a draft for editing needs its draft id.
CREATE TABLE server_drafts (
  gmail_draft_id   TEXT PRIMARY KEY,
  gmail_message_id TEXT NOT NULL
);
CREATE INDEX server_drafts_by_message ON server_drafts (gmail_message_id);
