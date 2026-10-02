-- How long a learning run's batches took, to estimate the time left
-- (spec §14.9): batches timed and their total milliseconds.
ALTER TABLE guide_runs ADD COLUMN timed_batches INTEGER NOT NULL DEFAULT 0;
ALTER TABLE guide_runs ADD COLUMN timed_ms INTEGER NOT NULL DEFAULT 0;
