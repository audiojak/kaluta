-- Facts about the user (spec §14.11): their own store, replacing the
-- writing guide's F3 entries (moved by a Rust step after this script).
-- Built-in categories are defined in code; this table holds custom ones,
-- and built-ins the user hid.
CREATE TABLE fact_categories (
  key         TEXT PRIMARY KEY,
  name        TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  position    INTEGER NOT NULL DEFAULT 0,
  builtin     INTEGER NOT NULL DEFAULT 0,
  hidden      INTEGER NOT NULL DEFAULT 0,
  default_use TEXT NOT NULL DEFAULT 'free' CHECK (default_use IN ('free', 'ask', 'never')),
  -- The starter set that added it, if one did.
  starter     TEXT,
  created_at  INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE facts (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  category   TEXT NOT NULL,
  label      TEXT NOT NULL,
  value      TEXT NOT NULL,
  use        TEXT NOT NULL DEFAULT 'free' CHECK (use IN ('free', 'ask', 'never')),
  -- When it was true; old ones are flagged for review.
  as_of      INTEGER,
  source     TEXT NOT NULL CHECK (source IN ('you', 'learned', 'writing_help')),
  status     TEXT NOT NULL CHECK (status IN ('proposed', 'accepted', 'rejected')),
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX facts_by_category ON facts (category, label);

CREATE TABLE fact_evidence (
  fact_id    INTEGER NOT NULL REFERENCES facts (id) ON DELETE CASCADE,
  message_id TEXT NOT NULL,
  quote      TEXT NOT NULL,
  added_at   INTEGER NOT NULL,
  PRIMARY KEY (fact_id, message_id)
) WITHOUT ROWID;

-- Every change to facts or categories, for exact undo (ADR 0006).
CREATE TABLE fact_changes (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  reason      TEXT NOT NULL,
  before_json TEXT NOT NULL,
  after_json  TEXT NOT NULL,
  created_at  INTEGER NOT NULL
);
