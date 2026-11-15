-- Forwarding rules (spec §14.15, ADR 0021): mail from a sender whose
-- subject starts with a prefix is forwarded to one address, on its own
-- (`auto`) or after asking (`ask`), as the user chose when accepting the
-- rule. Rules are proposed from the user's own forwards; nothing here is
-- written by an agent, and nothing here is synced.
CREATE TABLE forward_rules (
  id               INTEGER PRIMARY KEY AUTOINCREMENT,
  -- An address, or a domain when `sender_is_domain`; lower case.
  sender           TEXT NOT NULL CHECK (sender <> ''),
  sender_is_domain INTEGER NOT NULL DEFAULT 0,
  -- As the user sees it; compared without case. May be empty for an address.
  subject_prefix   TEXT NOT NULL DEFAULT '',
  recipient        TEXT NOT NULL CHECK (recipient <> ''),
  mode             TEXT NOT NULL CHECK (mode IN ('auto', 'ask')),
  enabled          INTEGER NOT NULL DEFAULT 1,
  -- Only mail that arrived after this is forwarded: when the rule was
  -- accepted, or last had its sender, prefix or recipient changed.
  effective_from   INTEGER NOT NULL,
  -- The proposal it was accepted from; NULL for one written by hand.
  proposal_id      INTEGER,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
);

-- What Kaluta proposes, with the forwards that show it. A pattern decided
-- once (accepted or rejected) is not raised again.
CREATE TABLE forward_rule_proposals (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  -- What Teach Kaluta shows.
  statement      TEXT NOT NULL,
  sender         TEXT NOT NULL,
  subject_prefix TEXT NOT NULL DEFAULT '',
  recipient      TEXT NOT NULL,
  -- sender, prefix and recipient, lower case.
  match_key      TEXT NOT NULL UNIQUE,
  status         TEXT NOT NULL CHECK (status IN ('proposed', 'accepted', 'rejected')),
  -- [{"thread_id", "message_id", "subject", "at"}]: the mail forwarded.
  evidence_json  TEXT NOT NULL DEFAULT '[]',
  -- The rule made from it, while accepted.
  rule_id        INTEGER,
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL,
  decided_at     INTEGER
);
CREATE INDEX forward_rule_proposals_by_status ON forward_rule_proposals (status, id);

-- What a rule did with a message: one row per rule and message, which is
-- what keeps a message from being forwarded, or asked about, twice.
CREATE TABLE forward_rule_log (
  rule_id    INTEGER NOT NULL REFERENCES forward_rules (id) ON DELETE CASCADE,
  -- The received message (its provider id).
  message_id TEXT NOT NULL,
  thread_id  TEXT NOT NULL,
  -- Where the rule sent it, or would: a rule's recipient can change later.
  recipient  TEXT NOT NULL,
  outcome    TEXT NOT NULL CHECK (outcome IN ('sent', 'asked', 'dismissed', 'skipped')),
  -- Why it was skipped.
  reason     TEXT,
  at         INTEGER NOT NULL,
  PRIMARY KEY (rule_id, message_id)
) WITHOUT ROWID;
CREATE INDEX forward_rule_log_by_thread ON forward_rule_log (thread_id);
CREATE INDEX forward_rule_log_by_time ON forward_rule_log (rule_id, outcome, at);
