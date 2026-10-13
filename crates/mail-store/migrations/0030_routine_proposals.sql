-- Changes Kaluta proposes to a sorting routine (spec §14.13, §14.10
-- amended): when the user moves filed mail back to the Inbox, Kaluta
-- proposes leaving such mail alone. Nothing changes until it is accepted
-- in Teach Kaluta. The evidence is the conversations moved back.
CREATE TABLE routine_proposals (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  -- The routine it changes (its uuid); NULL until the account has one.
  routine_id    TEXT,
  op            TEXT NOT NULL CHECK (op IN ('leave_alone')),
  -- What Teach Kaluta shows: "Leave mail from news@example.com in the Inbox".
  statement     TEXT NOT NULL,
  -- The line added to the routine's leave-alone rules when accepted.
  rule          TEXT NOT NULL,
  match_key     TEXT NOT NULL UNIQUE,
  status        TEXT NOT NULL CHECK (status IN ('proposed', 'accepted', 'rejected')),
  -- [{"thread_id", "subject", "bucket", "at"}]
  evidence_json TEXT NOT NULL DEFAULT '[]',
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL,
  decided_at    INTEGER
);
