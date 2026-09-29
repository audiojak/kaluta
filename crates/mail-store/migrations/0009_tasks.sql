-- Tasks made from email (spec §14.8). One account per database. Threads
-- and messages are referenced by their Gmail ids, not foreign keys: a task
-- outlives a thread leaving the local store (trimmed, or deleted on the
-- server) and still says what it was about.
CREATE TABLE tasks (
  id           INTEGER PRIMARY KEY,
  thread_id    TEXT NOT NULL,
  message_id   TEXT,
  title        TEXT NOT NULL,
  notes        TEXT NOT NULL DEFAULT '',
  category     TEXT NOT NULL,
  -- A day, `YYYY-MM-DD` in the user's calendar, or none.
  due_day      TEXT,
  action       TEXT NOT NULL DEFAULT 'none' CHECK (action IN ('reply', 'reply_all', 'forward', 'none')),
  status       TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'done')),
  source       TEXT NOT NULL CHECK (source IN ('ai', 'you')),
  why          TEXT NOT NULL DEFAULT '',
  created_at   INTEGER NOT NULL,
  completed_at INTEGER
);
CREATE INDEX tasks_by_thread ON tasks (thread_id);
CREATE INDEX tasks_by_status ON tasks (status, due_day);

-- The account's categories, in order; editable in Settings › Tasks.
CREATE TABLE task_categories (
  name     TEXT PRIMARY KEY COLLATE NOCASE,
  position INTEGER NOT NULL
) WITHOUT ROWID;
INSERT INTO task_categories (name, position) VALUES
  ('Reply', 0), ('Decide', 1), ('Gather Info', 2), ('Schedule', 3),
  ('Review', 4), ('Admin', 5), ('Follow Up', 6);

-- Small settings: the id of the account's `Task` label once found or made.
CREATE TABLE task_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;
