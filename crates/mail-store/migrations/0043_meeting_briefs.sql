-- Meeting prep (spec §14.14 amendment 2026-11-18, ADR 0024): the user's
-- upcoming meetings from Granola's calendar entries, and the brief Kaluta
-- wrote for each from the user's mail.
CREATE TABLE meeting_briefs (
  meeting_id        TEXT PRIMARY KEY,
  title             TEXT NOT NULL,
  meeting_at        INTEGER NOT NULL,
  participants_json TEXT NOT NULL DEFAULT '[]',
  brief             TEXT,
  updated_at        INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX meeting_briefs_by_time ON meeting_briefs (meeting_at);
