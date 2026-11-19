-- Reply-landed alerts (spec §14.7 amendment 2026-11-18): conversations the
-- user asked to be told about when someone answers. One alert, then the
-- watch is gone.
CREATE TABLE watches (
  thread_id TEXT PRIMARY KEY,
  set_at    INTEGER NOT NULL
) WITHOUT ROWID;
