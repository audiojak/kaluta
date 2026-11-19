-- Send later (spec §14.5 amendment 2026-11-18, ADR 0022): a draft the user
-- asked to send at a time still to come. It stays a draft, editable and
-- mirrored as one, until then; the app sends it when the time comes, by
-- the ordinary send. Nothing is queued in the outbox meanwhile.
CREATE TABLE scheduled_sends (
  draft_id INTEGER PRIMARY KEY REFERENCES drafts(id) ON DELETE CASCADE,
  send_at  INTEGER NOT NULL,
  set_at   INTEGER NOT NULL
);
CREATE INDEX scheduled_sends_by_time ON scheduled_sends (send_at);
