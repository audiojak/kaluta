-- Search by meaning (spec §14.3 amendment 2026-11-18, ADR 0023): a vector
-- per message, computed on the Mac by the app from its subject and
-- snippet. Derived data: rebuilt freely, never sent anywhere.
CREATE TABLE message_embeddings (
  message_id INTEGER PRIMARY KEY REFERENCES messages (id) ON DELETE CASCADE,
  model      TEXT NOT NULL,
  vec        BLOB NOT NULL
);
