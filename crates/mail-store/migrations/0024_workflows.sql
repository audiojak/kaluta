-- Workflows (spec §14.13): each workflow's set as it was fixed when the
-- user started it, where they are in it and what they decided, so a set
-- resumes later the same day, ends with a score, and the start screen can
-- estimate from the user's own pace. One account per database; items are
-- keys (`thread:<gmail id>`, `task:<id>`), not foreign keys, so a set
-- still says what it held after a thread leaves the store.
CREATE TABLE workflow_sets (
  id          INTEGER PRIMARY KEY,
  kind        TEXT NOT NULL CHECK (kind IN ('triage', 'replies', 'tasks', 'waiting', 'reading', 'agents', 'teach')),
  -- The local day it was started on, `YYYY-MM-DD`.
  day         TEXT NOT NULL,
  -- The item keys in order; a skipped item moves to the end.
  items_json  TEXT NOT NULL,
  started_at  INTEGER NOT NULL,
  finished_at INTEGER
);
CREATE INDEX workflow_sets_by_day ON workflow_sets (day, kind);

CREATE TABLE workflow_decisions (
  set_id     INTEGER NOT NULL REFERENCES workflow_sets (id) ON DELETE CASCADE,
  item       TEXT NOT NULL,
  -- The command that decided it: `archive`, `reply`, `task`, `leave`,
  -- `done`, `later`, `nudge`, `let_go`, `read`, `accept`, `reject`, …
  decision   TEXT NOT NULL,
  decided_at INTEGER NOT NULL,
  -- Time on the item, for the user's pace.
  millis     INTEGER NOT NULL,
  PRIMARY KEY (set_id, item)
) WITHOUT ROWID;
CREATE INDEX workflow_decisions_by_time ON workflow_decisions (decided_at);

-- Small settings: `triage_since`, when the last triage set finished.
CREATE TABLE workflow_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;
