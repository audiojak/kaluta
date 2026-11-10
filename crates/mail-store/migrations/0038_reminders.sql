-- Remind me later (spec §14.13, amended 2026-11-09): a conversation kept
-- out of Triage until a time the user chose, then brought back to it. The
-- email stays in the Inbox meanwhile; mail from someone else in the
-- conversation after `set_at` brings it back at once.
CREATE TABLE reminders (
  thread_id TEXT PRIMARY KEY,
  remind_at INTEGER NOT NULL,
  set_at    INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX reminders_by_time ON reminders (remind_at);
