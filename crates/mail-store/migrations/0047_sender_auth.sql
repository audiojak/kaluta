-- What the receiving server concluded about each message's sender (spec
-- §14.16): 'pass', 'fail' or 'none', from the Authentication-Results
-- header the provider's own server added. NULL (mail synced before this,
-- imported mailboxes, agent mailboxes) reads as 'none'. Not backfilled.
ALTER TABLE messages ADD COLUMN sender_auth TEXT CHECK (sender_auth IN ('pass', 'fail', 'none'));
