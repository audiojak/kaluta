-- Workflow background jobs (spec §14.13): what each job did on each local
-- day, for its per-account daily cap, and how its last attempt went. A job
-- finds its own work in the store, so this and the work itself are all a
-- quit leaves behind: the next launch carries on within the same cap.
CREATE TABLE workflow_job_days (
  job         TEXT NOT NULL,
  -- `YYYY-MM-DD`, local.
  day         TEXT NOT NULL,
  -- Items done that day (conversations suggested, threads checked, ...).
  items       INTEGER NOT NULL DEFAULT 0,
  last_run_at INTEGER NOT NULL,
  -- Why the last attempt stopped, if it did.
  last_error  TEXT,
  PRIMARY KEY (job, day)
);
