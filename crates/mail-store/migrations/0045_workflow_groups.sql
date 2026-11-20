-- Triage groups (spec §14.13 amendment 2026-11-19): which part of the set
-- each item came from (due again, worth a look, from people, automated),
-- kept with the set so a resumed set shows the same groups. A JSON object
-- from item key to group; NULL for sets without groups.
ALTER TABLE workflow_sets ADD COLUMN groups_json TEXT;
