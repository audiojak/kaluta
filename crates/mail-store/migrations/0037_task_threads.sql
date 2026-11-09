-- More than one conversation per task (spec §14.8, amended 2026-11-09):
-- reminders and follow-ups about the same piece of work, added to a task
-- that already exists instead of making another. The task's own thread
-- stays in `tasks.thread_id`; these are the others.
CREATE TABLE task_threads (
  task_id   INTEGER NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
  thread_id TEXT NOT NULL,
  added_at  INTEGER NOT NULL,
  PRIMARY KEY (task_id, thread_id)
) WITHOUT ROWID;
CREATE INDEX task_threads_by_thread ON task_threads (thread_id);
