-- Reports of what cloud agents sent, pulled from the agent mailbox's rules
-- server (spec §10.6, ADR 0016): the agent's own account of its own send,
-- kept as untrusted text. Each is matched to the sent message by its
-- Message-ID, else by recipient, subject and time, and recorded as an AI
-- composition when the account records them (ADR 0013).
CREATE TABLE cloud_reports (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  -- The rules server (its URL) and the report's id there: pulled once.
  server             TEXT NOT NULL,
  server_id          INTEGER NOT NULL,
  agent_id           TEXT NOT NULL,
  agent_name         TEXT NOT NULL,
  agent_kind         TEXT NOT NULL DEFAULT 'token',
  -- When the server took it, and when this Mac pulled it.
  received_at        INTEGER NOT NULL,
  pulled_at          INTEGER NOT NULL,
  -- The Message-ID the agent reported, without its brackets.
  message_id         TEXT,
  to_json            TEXT NOT NULL DEFAULT '[]',
  subject            TEXT NOT NULL DEFAULT '',
  sent_at            INTEGER,
  -- Cleared with the AI texts (retention, ADR 0013).
  body_markdown      TEXT,
  -- The snapshot version the agent checked against, and the one the
  -- server checked the report against, with what that check found.
  checked_version    INTEGER,
  check_version      INTEGER,
  guide_check_json   TEXT NOT NULL DEFAULT '[]',
  composition_id     INTEGER,
  -- The sent message it was matched to (its gmail_id) and how.
  matched_message_id TEXT,
  match_method       TEXT CHECK (match_method IN ('message_id', 'recipient_subject')),
  matched_at         INTEGER,
  UNIQUE (server, server_id)
);
CREATE INDEX cloud_reports_by_time ON cloud_reports (received_at);
CREATE INDEX cloud_reports_unmatched ON cloud_reports (received_at) WHERE matched_message_id IS NULL;
