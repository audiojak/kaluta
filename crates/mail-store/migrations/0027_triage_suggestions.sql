-- Triage suggestions (spec §14.13): what Kaluta suggests for each new
-- conversation, made ahead by the triage job. A suggestion is for the
-- conversation as it was: once a newer message arrives it no longer
-- counts and the job suggests again.
CREATE TABLE triage_suggestions (
  thread_id   TEXT PRIMARY KEY,
  -- The conversation's latest message (Gmail id) when it was suggested.
  message_id  TEXT NOT NULL,
  kind        TEXT NOT NULL CHECK (kind IN ('reply', 'task', 'archive', 'leave')),
  -- Kaluta's estimate of the time to answer, for a quick reply.
  seconds     INTEGER,
  reason      TEXT NOT NULL,
  -- Up to three answers for a quick reply: [{"label", "text"}].
  answers_json TEXT NOT NULL DEFAULT '[]',
  -- The task to make, for kind 'task': {"title", "category", "due",
  -- "action", "why"}.
  task_json   TEXT,
  agent       TEXT,
  created_at  INTEGER NOT NULL
);
