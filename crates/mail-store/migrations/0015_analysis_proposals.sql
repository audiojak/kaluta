-- What the daily review proposes (spec §14.10): changes to the writing
-- guide (and, later, to facts), merged across pairs and days, with the
-- pairs that show them. Shown once backed by enough pairs; until then
-- 'watching'.
CREATE TABLE analysis_proposals (
  id                   INTEGER PRIMARY KEY AUTOINCREMENT,
  target               TEXT NOT NULL DEFAULT 'guide' CHECK (target IN ('guide', 'fact')),
  op                   TEXT NOT NULL CHECK (op IN ('add', 'edit', 'rescope', 'remove')),
  -- The guide entry an edit, rescope or removal is for.
  entry_id             INTEGER,
  category             TEXT NOT NULL,
  kind                 TEXT,
  statement            TEXT NOT NULL DEFAULT '',
  scope_json           TEXT NOT NULL DEFAULT '{}',
  -- What makes two proposals the same one.
  match_key            TEXT NOT NULL,
  status               TEXT NOT NULL CHECK (status IN ('watching', 'proposed', 'accepted', 'rejected')),
  -- Distinct pairs behind it.
  support              INTEGER NOT NULL DEFAULT 0,
  contradicts_entry_id INTEGER,
  payload_json         TEXT,
  created_at           INTEGER NOT NULL,
  updated_at           INTEGER NOT NULL,
  -- When it crossed the threshold and showed (the unseen dot).
  shown_at             INTEGER,
  decided_at           INTEGER
);
CREATE UNIQUE INDEX analysis_proposals_by_key ON analysis_proposals (target, match_key);
CREATE INDEX analysis_proposals_by_status ON analysis_proposals (status, shown_at);

CREATE TABLE analysis_evidence (
  proposal_id    INTEGER NOT NULL REFERENCES analysis_proposals (id) ON DELETE CASCADE,
  composition_id INTEGER NOT NULL REFERENCES ai_compositions (id) ON DELETE CASCADE,
  -- From what the user sent, and what it replaced in the AI's text.
  sent_quote     TEXT NOT NULL DEFAULT '',
  ai_quote       TEXT NOT NULL DEFAULT '',
  added_at       INTEGER NOT NULL,
  PRIMARY KEY (proposal_id, composition_id)
) WITHOUT ROWID;

-- How drafts that applied an entry fared: sent as written, or changed
-- against it (spec §14.10, per-entry health).
CREATE TABLE analysis_entry_health (
  entry_id   INTEGER PRIMARY KEY,
  unchanged  INTEGER NOT NULL DEFAULT 0,
  overridden INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL
);
