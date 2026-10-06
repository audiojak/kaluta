-- Every AI composition with its full text, so the daily review can compare
-- it with what the user sent (spec §14.10, ADR 0013).
CREATE TABLE ai_compositions (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at         INTEGER NOT NULL,
  updated_at         INTEGER NOT NULL,
  source             TEXT NOT NULL CHECK (source IN ('writing_help', 'agent', 'routine')),
  agent              TEXT,
  kind               TEXT NOT NULL CHECK (kind IN ('reply', 'forward', 'new')),
  -- The local draft it went into, while that draft exists.
  draft_id           INTEGER,
  thread_id          TEXT,
  in_reply_to        TEXT,
  -- {"to": [...], "cc": [...]}: lowercased addresses.
  recipients_json    TEXT NOT NULL DEFAULT '{"to":[],"cc":[]}',
  subject            TEXT NOT NULL DEFAULT '',
  instruction        TEXT NOT NULL DEFAULT '',
  ai_text            TEXT,
  ai_html            TEXT,
  guide_version      INTEGER,
  audiences_json     TEXT NOT NULL DEFAULT '[]',
  rfc822_message_id  TEXT,
  status             TEXT NOT NULL DEFAULT 'waiting'
                     CHECK (status IN ('waiting', 'matched', 'unmatched', 'discarded', 'reviewed')),
  matched_message_id TEXT,
  match_method       TEXT CHECK (match_method IN ('sent_draft', 'thread_next', 'recipient_next')),
  sent_text          TEXT,
  distance           REAL,
  reviewed_at        INTEGER
);
CREATE INDEX ai_compositions_by_draft ON ai_compositions (draft_id) WHERE draft_id IS NOT NULL;
CREATE INDEX ai_compositions_by_status ON ai_compositions (status, created_at);
CREATE INDEX ai_compositions_by_rfc822 ON ai_compositions (rfc822_message_id) WHERE rfc822_message_id IS NOT NULL;
