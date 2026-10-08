-- Clean Up's progress card (spec §14.12): mail received today is counted
-- by arrival (`internal_date`), every time the card refreshes; without an
-- index that is a scan of the whole mailbox.
CREATE INDEX messages_by_internal_date ON messages (internal_date);
