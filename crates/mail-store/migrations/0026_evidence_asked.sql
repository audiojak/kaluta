-- What the user told Kaluta is evidence (spec §14.13, §14.10 amended): a
-- proposal may cite the words of the user's direction for a draft, beside
-- or instead of the words of their edits.
ALTER TABLE analysis_evidence ADD COLUMN asked_quote TEXT NOT NULL DEFAULT '';
