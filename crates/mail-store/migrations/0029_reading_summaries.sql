-- Weekly reading (spec §14.13): what Kaluta wrote about each conversation
-- the sorting routine filed, made soon after it was sorted, so a bucket
-- opens ready: one line for the list, a short summary for reading. For the
-- conversation's latest message; a newer one is summarised again.
CREATE TABLE reading_summaries (
  thread_id  TEXT PRIMARY KEY,
  message_id TEXT NOT NULL,
  line       TEXT NOT NULL,
  summary    TEXT NOT NULL,
  agent      TEXT,
  created_at INTEGER NOT NULL
);
