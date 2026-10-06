-- A guide change made from Analysis also records the proposals it decided
-- (spec §14.10), so undo puts them back with the guide (ADR 0006).
ALTER TABLE guide_changes ADD COLUMN analysis_before TEXT;
ALTER TABLE guide_changes ADD COLUMN analysis_after TEXT;
