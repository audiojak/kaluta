-- Meeting follow-ups (spec §14.14): meetings read from Granola, Kaluta's
-- verdict on each (does it deserve a follow-up email, and what could it
-- say), and what the user decided. Ids are Granola's.
CREATE TABLE meeting_followups (
  meeting_id        TEXT PRIMARY KEY,
  title             TEXT NOT NULL DEFAULT '',
  -- When it was held, ms since the epoch.
  meeting_at        INTEGER NOT NULL,
  -- [{name, email}] as Granola knows them, the user included.
  participants_json TEXT NOT NULL DEFAULT '[]',
  url               TEXT,
  seen_at           INTEGER NOT NULL,
  -- Kaluta's verdict: NULL until judged.
  judged_at         INTEGER,
  follow_up         INTEGER,
  reason            TEXT NOT NULL DEFAULT '',
  -- [{label, to: [email], subject, body}]
  options_json      TEXT NOT NULL DEFAULT '[]',
  -- Tries that failed (the agent, Granola); given up after three.
  attempts          INTEGER NOT NULL DEFAULT 0,
  -- `sent` (a follow-up was opened to send), `none` (no follow-up).
  decided           TEXT,
  decided_at        INTEGER
) WITHOUT ROWID;
CREATE INDEX meeting_followups_by_time ON meeting_followups (meeting_at);

-- The new workflow kind. SQLite cannot change a CHECK in place, so the
-- sets are copied into a table that allows it, their decisions with
-- them (copied first: dropping the old sets would cascade to them).
CREATE TABLE workflow_sets_new (
  id          INTEGER PRIMARY KEY,
  kind        TEXT NOT NULL
              CHECK (kind IN ('triage', 'replies', 'tasks', 'waiting', 'reading', 'agents', 'teach', 'followups')),
  day         TEXT NOT NULL,
  items_json  TEXT NOT NULL,
  started_at  INTEGER NOT NULL,
  finished_at INTEGER
);
INSERT INTO workflow_sets_new (id, kind, day, items_json, started_at, finished_at)
  SELECT id, kind, day, items_json, started_at, finished_at FROM workflow_sets;
CREATE TABLE workflow_decisions_new (
  set_id     INTEGER NOT NULL REFERENCES workflow_sets_new (id) ON DELETE CASCADE,
  item       TEXT NOT NULL,
  decision   TEXT NOT NULL,
  decided_at INTEGER NOT NULL,
  millis     INTEGER NOT NULL,
  PRIMARY KEY (set_id, item)
) WITHOUT ROWID;
INSERT INTO workflow_decisions_new (set_id, item, decision, decided_at, millis)
  SELECT set_id, item, decision, decided_at, millis FROM workflow_decisions;
DROP TABLE workflow_decisions;
DROP TABLE workflow_sets;
ALTER TABLE workflow_sets_new RENAME TO workflow_sets;
ALTER TABLE workflow_decisions_new RENAME TO workflow_decisions;
CREATE INDEX workflow_sets_by_day ON workflow_sets (day, kind);
CREATE INDEX workflow_decisions_by_time ON workflow_decisions (decided_at);
