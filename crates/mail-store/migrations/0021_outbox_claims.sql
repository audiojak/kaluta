-- Who is sending an in-flight op, and until when (spec §7.4, outbox
-- claims). The app and a second process (the headless MCP) may both drain
-- one account's outbox: an op is claimed by one drainer, named by
-- `claimed_by`, in the transaction that picks it; `lease_until` is renewed
-- while the call runs. A row whose claimant has gone (its lock file free)
-- or whose lease ran out goes back to pending; a send that comes back so is
-- looked for at the provider before it is sent again.
ALTER TABLE outbox ADD COLUMN claimed_by TEXT;
ALTER TABLE outbox ADD COLUMN lease_until INTEGER;
