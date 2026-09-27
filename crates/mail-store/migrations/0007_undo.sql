-- Undo for the user's mail actions (spec §14.6a): what each action changed,
-- per message, so undo reverses exactly that. The last 50 are kept.
CREATE TABLE undo_actions (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  kind        TEXT NOT NULL,
  diffs_json  TEXT NOT NULL,
  created_at  INTEGER NOT NULL
);
