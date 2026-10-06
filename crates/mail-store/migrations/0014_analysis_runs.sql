-- The daily review (spec §14.10): one run per account per calendar day,
-- or on demand, comparing matched AI compositions in batches.
CREATE TABLE analysis_runs (
  id            INTEGER PRIMARY KEY,
  -- The local calendar day it ran for, YYYY-MM-DD.
  day           TEXT NOT NULL,
  trigger       TEXT NOT NULL CHECK (trigger IN ('daily', 'manual')),
  status        TEXT NOT NULL CHECK (status IN ('running', 'paused', 'done', 'cancelled', 'failed')),
  agent         TEXT,
  error         TEXT,
  -- What matching found at the start of the run.
  matched       INTEGER NOT NULL DEFAULT 0,
  unmatched     INTEGER NOT NULL DEFAULT 0,
  -- Pairs sent as written (no agent needed) and pairs to compare.
  unchanged     INTEGER NOT NULL DEFAULT 0,
  batch_size    INTEGER NOT NULL,
  total         INTEGER NOT NULL DEFAULT 0,
  done          INTEGER NOT NULL DEFAULT 0,
  timed_batches INTEGER NOT NULL DEFAULT 0,
  timed_ms      INTEGER NOT NULL DEFAULT 0,
  started_at    INTEGER NOT NULL,
  finished_at   INTEGER
);
CREATE INDEX analysis_runs_by_day ON analysis_runs (day);

CREATE TABLE analysis_run_pairs (
  run_id         INTEGER NOT NULL REFERENCES analysis_runs (id) ON DELETE CASCADE,
  composition_id INTEGER NOT NULL REFERENCES ai_compositions (id) ON DELETE CASCADE,
  batch          INTEGER NOT NULL,
  done           INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (run_id, composition_id)
) WITHOUT ROWID;

-- Settings and state of Analysis for the account: last_viewed_at,
-- daily_review, facts_from, pairs_per_day, keep_days, notify.
CREATE TABLE analysis_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;
