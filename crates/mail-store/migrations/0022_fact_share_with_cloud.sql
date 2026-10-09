-- A fact's *Share with cloud agents* switch (spec §10.6, §14.11): whether
-- an agent mailbox that publishes to a rules server includes it. NULL is
-- the default, which follows the fact's use and store (decision 4 of the
-- rules-server plan): on for an account's *Use freely* facts, off for *Ask
-- before using* and for global facts; *Never share* facts never go.
ALTER TABLE facts ADD COLUMN share_with_cloud INTEGER;
