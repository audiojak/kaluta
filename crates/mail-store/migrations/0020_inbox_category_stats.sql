-- Inbox threads and unread threads per category tab (spec §14.3), kept by
-- deltas when threads are recomputed, as `label_stats` is: the sidebar's
-- Inbox count (Primary's unread once other tabs have mail) and the tab
-- counts no longer count over the whole Inbox. A thread in several
-- categories counts in the first, in tab order; one in none is Primary
-- ('CATEGORY_PERSONAL').
CREATE TABLE inbox_category_stats (
  category            TEXT PRIMARY KEY,
  thread_count        INTEGER NOT NULL DEFAULT 0,
  unread_thread_count INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;

INSERT INTO inbox_category_stats (category, thread_count, unread_thread_count)
SELECT COALESCE(
         (SELECT cl.gmail_id FROM thread_labels c JOIN labels cl ON cl.id = c.label_id
          WHERE c.thread_id = tl.thread_id
            AND cl.gmail_id IN ('CATEGORY_PROMOTIONS', 'CATEGORY_SOCIAL', 'CATEGORY_UPDATES', 'CATEGORY_FORUMS')
          ORDER BY CASE cl.gmail_id WHEN 'CATEGORY_PROMOTIONS' THEN 0 WHEN 'CATEGORY_SOCIAL' THEN 1
                                    WHEN 'CATEGORY_UPDATES' THEN 2 ELSE 3 END
          LIMIT 1),
         'CATEGORY_PERSONAL'),
       COUNT(*), SUM(t.unread_count > 0)
FROM thread_labels tl JOIN threads t ON t.id = tl.thread_id
WHERE tl.label_id = (SELECT id FROM labels WHERE gmail_id = 'INBOX')
GROUP BY 1;
