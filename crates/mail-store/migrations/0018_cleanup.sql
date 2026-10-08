-- Clean Up (spec §14.12): grouping messages by sender, date, size and
-- mailing list, and the Inbox's daily count for the progress card.

-- The mailing-list headers, stored from now on (no refetch of old mail).
-- `list_id` is the id inside List-Id's angle brackets, lower-cased;
-- `list_name` its phrase. The unsubscribe headers are kept as sent
-- (unfolded), for Unsubscribe to parse when it acts.
ALTER TABLE messages ADD COLUMN list_id TEXT;
ALTER TABLE messages ADD COLUMN list_name TEXT;
ALTER TABLE messages ADD COLUMN list_unsubscribe TEXT;
ALTER TABLE messages ADD COLUMN list_unsubscribe_post TEXT;

-- Grouping without reading the messages table: each view's group-by
-- columns as a covering index (the rowid comes with every index), then
-- `date`, so a page of a group's messages, newest first, is chosen from
-- the index before any row is read.
CREATE INDEX messages_by_from_email ON messages (from_email COLLATE NOCASE, from_name, date);
CREATE INDEX messages_by_subject ON messages (subject, date);
CREATE INDEX messages_by_date ON messages (date);
CREATE INDEX messages_by_size ON messages (size_estimate, date);
CREATE INDEX messages_by_list_id ON messages (list_id, list_name, date) WHERE list_id IS NOT NULL;

-- Inbox messages at the start of each day (`YYYY-MM-DD` in the user's
-- calendar), written at the first sync after midnight and on opening.
CREATE TABLE inbox_history (
  day   TEXT PRIMARY KEY,
  count INTEGER NOT NULL
) WITHOUT ROWID;

-- Small settings: the Inbox count when Clean Up was first opened (the
-- progress baseline) and when that was.
CREATE TABLE cleanup_meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) WITHOUT ROWID;
