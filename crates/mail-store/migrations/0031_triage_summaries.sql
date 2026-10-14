-- Kaluta's summary of a long email meant for reading (a newsletter, a
-- digest, a report), made with its triage suggestion (spec §14.13): the
-- Triage workflow shows it in place of the text, the email one key away.
ALTER TABLE triage_suggestions ADD COLUMN summary TEXT NOT NULL DEFAULT '';
