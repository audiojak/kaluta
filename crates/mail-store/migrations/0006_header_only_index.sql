-- Tiered download (spec §7.4 amendment 2026-09-27): search asks Gmail too
-- when header-only mail exists, so that question must be cheap.
CREATE INDEX messages_header_only ON messages (id) WHERE body_state = 'metadata';
