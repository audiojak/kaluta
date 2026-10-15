-- What kind of email a conversation is (newsletter, receipt, notification,
-- announcement, invitation, promotion), made with its triage suggestion
-- for mail not written by a person (spec §14.13); empty for the rest.
ALTER TABLE triage_suggestions ADD COLUMN category TEXT NOT NULL DEFAULT '';
