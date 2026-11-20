-- Snippets (spec §14.5 amendment 2026-11-18): saved text the user inserts
-- into a message by typing / at the start of a line. Kept in the global
-- store beside global facts (ADR 0012), so every account has them; the
-- table exists in account stores too, unused, as the schema is shared.
CREATE TABLE snippets (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  body_html  TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
