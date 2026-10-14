//! What makes a conversation in the Inbox likely to matter (spec §14.13,
//! the start screen and Triage by priority): the sender is someone the
//! user writes to, it answers something the user sent, it is to the user
//! alone, it is starred or Gmail marked it important.

use std::collections::HashMap;

use rusqlite::{Connection, params};

use crate::StoreResult;

/// How many sent messages went to each address (to or cc), by lowercased
/// address: who the user knows, and how well.
pub fn written_to(conn: &Connection) -> StoreResult<HashMap<String, u32>> {
    let mut stmt = conn.prepare_cached(
        "SELECT lower(p.email), COUNT(DISTINCT p.message_id)
           FROM participants p JOIN messages m ON m.id = p.message_id
          WHERE m.is_sent_by_me = 1 AND m.is_draft = 0 AND p.role IN ('to', 'cc')
          GROUP BY lower(p.email)",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// One conversation's signals, from its latest message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Signals {
    /// The user wrote earlier in the conversation: this answers them.
    pub answers_you: bool,
    /// The latest message is to one address and copies no one.
    pub to_you_alone: bool,
    pub starred: bool,
    /// Gmail's Important label.
    pub important: bool,
}

/// The signals of the conversation `thread` (Gmail id); its latest
/// message is `message` (Gmail id).
pub fn signals(conn: &Connection, thread: &str, message: &str) -> StoreResult<Signals> {
    let found = conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM messages e WHERE e.thread_id = t.id AND e.is_sent_by_me = 1
                              AND e.is_draft = 0 AND e.internal_date < m.internal_date),
                    (SELECT COUNT(*) FROM participants p WHERE p.message_id = m.id AND p.role = 'to') = 1
                      AND NOT EXISTS (SELECT 1 FROM participants p WHERE p.message_id = m.id AND p.role = 'cc'),
                    EXISTS (SELECT 1 FROM messages s WHERE s.thread_id = t.id AND s.is_starred = 1),
                    EXISTS (SELECT 1 FROM thread_labels i JOIN labels l ON l.id = i.label_id
                             WHERE i.thread_id = t.id AND l.gmail_id = 'IMPORTANT')
               FROM threads t JOIN messages m ON m.thread_id = t.id AND m.gmail_id = ?2
              WHERE t.gmail_id = ?1",
        )?
        .query_row(params![thread, message], |r| {
            Ok(Signals { answers_you: r.get(0)?, to_you_alone: r.get(1)?, starred: r.get(2)?, important: r.get(3)? })
        });
    match found {
        Ok(s) => Ok(s),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Signals::default()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    const NOW: i64 = 1_760_000_000_000;

    #[test]
    fn the_people_written_to_and_a_conversation_that_answers_the_user() {
        let dir = std::env::temp_dir().join(format!("kaluta-priority-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("mail.sqlite")).unwrap();
        crate::demo_workflows::generate(&db, NOW).unwrap();
        let known = db.read_blocking(written_to).unwrap();
        assert!(!known.is_empty(), "the demo's user writes to people");
        assert!(known.keys().all(|k| *k == k.to_lowercase()));
        // A conversation the user wrote in, and someone answered.
        let answered = db
            .read_blocking(|c| {
                Ok(c.query_row(
                    "SELECT t.gmail_id, m.gmail_id FROM threads t
                       JOIN messages m ON m.id = (SELECT id FROM messages WHERE thread_id = t.id AND is_draft = 0
                                                   ORDER BY internal_date DESC, id DESC LIMIT 1)
                      WHERE m.is_sent_by_me = 0
                        AND EXISTS (SELECT 1 FROM messages e WHERE e.thread_id = t.id AND e.is_sent_by_me = 1)
                      LIMIT 1",
                    [],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )?)
            })
            .unwrap();
        let s = db.read_blocking(move |c| signals(c, &answered.0, &answered.1)).unwrap();
        assert!(s.answers_you);
        assert_eq!(db.read_blocking(|c| signals(c, "no-such-thread", "x")).unwrap(), Signals::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
