-- The writing guide (spec §14.9, ADR 0011), per account. Messages are
-- referenced by Gmail id, not foreign key, like tasks: evidence outlives a
-- message leaving the local store.

-- One rule, guideline or fact. `norm` is the statement lower-cased with
-- spaces collapsed, so the same proposal from two batches merges and a
-- rejected one is not proposed again.
CREATE TABLE guide_entries (
  id               INTEGER PRIMARY KEY,
  category         TEXT NOT NULL,
  kind             TEXT NOT NULL CHECK (kind IN ('rule', 'guideline', 'fact')),
  statement        TEXT NOT NULL,
  norm             TEXT NOT NULL,
  scope_json       TEXT NOT NULL DEFAULT '{}',
  status           TEXT NOT NULL CHECK (status IN ('proposed', 'accepted', 'rejected')),
  source           TEXT NOT NULL CHECK (source IN ('learned', 'you', 'merged')),
  origin           TEXT,
  check_json       TEXT,
  support          INTEGER NOT NULL DEFAULT 0,
  contradict       INTEGER NOT NULL DEFAULT 0,
  -- A proposal that mail contradicts an accepted entry: that entry's id.
  contradiction_of INTEGER,
  -- The run that proposed it; proposals wait until that run finishes.
  run_id           INTEGER,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL,
  decided_at       INTEGER
);
CREATE INDEX guide_entries_by_status ON guide_entries (status, category);
CREATE INDEX guide_entries_by_norm ON guide_entries (category, norm);

-- Quotes from the user's own sent mail behind an entry.
CREATE TABLE guide_evidence (
  entry_id    INTEGER NOT NULL REFERENCES guide_entries (id) ON DELETE CASCADE,
  message_id  TEXT NOT NULL,
  quote       TEXT NOT NULL,
  contradicts INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (entry_id, message_id, quote)
) WITHOUT ROWID;

-- Learning runs, batch by batch, so a run survives quitting.
CREATE TABLE guide_runs (
  id          INTEGER PRIMARY KEY,
  kind        TEXT NOT NULL,
  focus       TEXT,
  status      TEXT NOT NULL CHECK (status IN ('running', 'paused', 'done', 'cancelled', 'failed')),
  batch_size  INTEGER NOT NULL,
  total       INTEGER NOT NULL DEFAULT 0,
  done        INTEGER NOT NULL DEFAULT 0,
  agent       TEXT,
  error       TEXT,
  started_at  INTEGER NOT NULL,
  finished_at INTEGER
);

-- Which messages a run covers, in batches; a message is analysed once.
CREATE TABLE guide_run_messages (
  run_id     INTEGER NOT NULL REFERENCES guide_runs (id) ON DELETE CASCADE,
  message_id TEXT NOT NULL,
  batch      INTEGER NOT NULL,
  -- The message's place in the run: newest first, as sampled.
  position   INTEGER NOT NULL,
  done       INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (run_id, message_id)
) WITHOUT ROWID;
CREATE INDEX guide_run_messages_by_message ON guide_run_messages (message_id);

-- Real sent messages kept as models for a message type (H3).
CREATE TABLE guide_examples (
  message_id   TEXT PRIMARY KEY,
  message_type TEXT NOT NULL,
  added_at     INTEGER NOT NULL
) WITHOUT ROWID;

-- Audience groups (D1): inferred and confirmed, or suggested to fill gaps.
CREATE TABLE audience_groups (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE COLLATE NOCASE,
  status      TEXT NOT NULL CHECK (status IN ('suggested', 'confirmed', 'rejected')),
  description TEXT NOT NULL DEFAULT '',
  position    INTEGER NOT NULL DEFAULT 0
);

-- Who is in a group: an address, or `@domain`.
CREATE TABLE audience_members (
  group_id INTEGER NOT NULL REFERENCES audience_groups (id) ON DELETE CASCADE,
  pattern  TEXT NOT NULL COLLATE NOCASE,
  PRIMARY KEY (group_id, pattern)
) WITHOUT ROWID;

-- Every change to entries, as the rows before and after, for exact undo
-- (ADR 0006): undo writes `before` back, redo writes `after`.
CREATE TABLE guide_changes (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  reason      TEXT NOT NULL,
  before_json TEXT NOT NULL,
  after_json  TEXT NOT NULL,
  created_at  INTEGER NOT NULL
);

-- The accepted guide after each change that touched it; drafts record the
-- version they were written under.
CREATE TABLE guide_versions (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  reason        TEXT NOT NULL,
  snapshot_json TEXT NOT NULL,
  created_at    INTEGER NOT NULL
);

-- Small settings (the last run's time, the signature seen, …).
CREATE TABLE guide_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;
