-- Waiting on others (spec §14.13). The waiting check's running log: every
-- sent message it read, with its verdict, so none is read twice; a newer
-- message in the conversation is a new row of its own.
CREATE TABLE waiting_checks (
  message_id TEXT PRIMARY KEY,
  thread_id  TEXT NOT NULL,
  checked_at INTEGER NOT NULL,
  -- Whether the message expects an answer.
  expects    INTEGER NOT NULL,
  reason     TEXT NOT NULL,
  agent      TEXT
);
CREATE INDEX waiting_checks_by_day ON waiting_checks (checked_at);

-- Conversations waiting on someone else's answer: the sent message waited
-- on, since when, hidden until (Check again), and how it ended.
CREATE TABLE waiting (
  thread_id   TEXT PRIMARY KEY,
  message_id  TEXT NOT NULL,
  since       INTEGER NOT NULL,
  until       INTEGER,
  state       TEXT NOT NULL CHECK (state IN ('waiting', 'answered', 'let_go')),
  -- Kaluta put the Waiting label on (not the user): Kaluta may take it off.
  labelled    INTEGER NOT NULL DEFAULT 0,
  -- Up to three follow-ups written ahead: [{"label", "text"}].
  nudges_json TEXT NOT NULL DEFAULT '[]',
  updated_at  INTEGER NOT NULL
);
