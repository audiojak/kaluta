//! Search by meaning (spec §14.3 amendment 2026-11-18, ADR 0023): a vector
//! per message from an on-device model, and the conversations nearest a
//! query's vector. Derived data, so no undo record.

use std::collections::HashMap;

use mail_domain::{MessageId, ThreadId};
use rusqlite::{Connection, Transaction, params};

use crate::error::{StoreError, StoreResult};

/// A message still to embed: its id and the text the model reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToEmbed {
    pub message_id: MessageId,
    pub text: String,
}

/// Messages with no vector from `model` yet, newest first.
pub fn pending(conn: &Connection, model: &str, limit: u32) -> StoreResult<Vec<ToEmbed>> {
    let mut stmt = conn.prepare_cached(
        "SELECT m.gmail_id, m.subject, m.snippet FROM messages m
          WHERE m.is_draft = 0
            AND NOT EXISTS (SELECT 1 FROM message_embeddings e WHERE e.message_id = m.id AND e.model = ?1)
          ORDER BY m.internal_date DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![model, limit], |r| {
        let (id, subject, snippet): (String, String, String) = (r.get(0)?, r.get(1)?, r.get(2)?);
        Ok(ToEmbed { message_id: MessageId(id), text: format!("{subject}\n{snippet}").trim().to_owned() })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// How many messages have a vector from `model`, and how many could.
pub fn progress(conn: &Connection, model: &str) -> StoreResult<(u32, u32)> {
    let done: i64 = conn
        .prepare_cached("SELECT COUNT(*) FROM message_embeddings WHERE model = ?1")?
        .query_row([model], |r| r.get(0))?;
    let all: i64 =
        conn.prepare_cached("SELECT COUNT(*) FROM messages WHERE is_draft = 0")?.query_row([], |r| r.get(0))?;
    Ok((done.max(0) as u32, all.max(0) as u32))
}

/// Keep vectors. A message that is gone is skipped; another model's
/// vector for the same message is replaced.
pub fn store(tx: &Transaction, model: &str, vectors: &[(MessageId, Vec<f32>)]) -> StoreResult<usize> {
    let mut kept = 0;
    for (id, vector) in vectors {
        if vector.is_empty() || vector.iter().any(|x| !x.is_finite()) {
            return Err(StoreError::Invalid("not a vector".into()));
        }
        let bytes: Vec<u8> = vector.iter().flat_map(|x| x.to_le_bytes()).collect();
        kept += tx
            .prepare_cached(
                "INSERT INTO message_embeddings (message_id, model, vec)
                 SELECT id, ?2, ?3 FROM messages WHERE gmail_id = ?1
                 ON CONFLICT (message_id) DO UPDATE SET model = excluded.model, vec = excluded.vec",
            )?
            .execute(params![id.as_str(), model, bytes])?;
    }
    Ok(kept)
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na.sqrt() * nb.sqrt()) }
}

/// The conversations whose messages are nearest `query`, best first, at
/// least `min_score` alike; each conversation by its nearest message.
/// Spam and Trash are left out, as search leaves them out.
pub fn nearest(
    conn: &Connection,
    model: &str,
    query: &[f32],
    min_score: f32,
    limit: usize,
) -> StoreResult<Vec<(ThreadId, f32)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.gmail_id, e.vec FROM message_embeddings e
           JOIN messages m ON m.id = e.message_id JOIN threads t ON t.id = m.thread_id
          WHERE e.model = ?1
            AND NOT EXISTS (SELECT 1 FROM thread_labels tl JOIN labels l ON l.id = tl.label_id
                             WHERE tl.thread_id = t.id AND l.gmail_id IN ('SPAM', 'TRASH'))",
    )?;
    let mut best: HashMap<String, f32> = HashMap::new();
    let rows = stmt.query_map([model], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))?;
    for row in rows {
        let (thread, bytes) = row?;
        let vector: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect();
        let score = cosine(query, &vector);
        if score >= min_score {
            let entry = best.entry(thread).or_insert(score);
            *entry = entry.max(score);
        }
    }
    let mut out: Vec<(ThreadId, f32)> = best.into_iter().map(|(t, s)| (ThreadId(t), s)).collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.as_str().cmp(b.0.as_str())));
    out.truncate(limit);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use mail_domain::{EmailAddress, LabelId};

    use super::*;
    use crate::{Db, IncomingMessage, MailWriter};

    struct Store(std::path::PathBuf, Db);

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store_named(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("kaluta-embeddings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store(dir.clone(), Db::open(&dir.join("mail.sqlite")).unwrap())
    }

    fn add(db: &Db, id: &str, thread: &str, subject: &str, label: &str, at: i64) {
        let m = IncomingMessage {
            id: MessageId::new(id),
            thread_id: ThreadId::new(thread),
            from: Some(EmailAddress::new(None, "sam@example.com")),
            subject: subject.into(),
            snippet: format!("about {subject}"),
            date: at,
            internal_date: at,
            label_ids: vec![LabelId::new(label)],
            ..Default::default()
        };
        db.write_blocking(move |tx| {
            let mut w = MailWriter::new(tx);
            w.upsert_message(&m)?;
            w.finish().map(|_| ())
        })
        .unwrap();
    }

    #[test]
    fn the_nearest_conversations_come_first_and_spam_never() {
        let Store(_, db) = &store_named("nearest");
        add(db, "m1", "t-invoice", "Invoice for March", "INBOX", 1);
        add(db, "m2", "t-lunch", "Lunch on Friday", "INBOX", 2);
        add(db, "m3", "t-spam", "Cheap invoices", "SPAM", 3);
        add(db, "m4", "t-invoice", "Re: Invoice for March", "INBOX", 4);
        let todo = db.read_blocking(|c| pending(c, "test", 10)).unwrap();
        assert_eq!(todo.iter().map(|t| t.message_id.as_str()).collect::<Vec<_>>(), ["m4", "m3", "m2", "m1"]);
        assert_eq!(todo[3].text, "Invoice for March\nabout Invoice for March");
        let v = vec![
            (MessageId::new("m1"), vec![1.0, 0.0, 0.1]),
            (MessageId::new("m2"), vec![0.0, 1.0, 0.0]),
            (MessageId::new("m3"), vec![1.0, 0.0, 0.0]),
            (MessageId::new("m4"), vec![0.9, 0.1, 0.0]),
            (MessageId::new("gone"), vec![1.0, 1.0, 1.0]),
        ];
        assert_eq!(db.write_blocking(move |tx| store(tx, "test", &v)).unwrap(), 4, "a message that is gone is skipped");
        assert!(db.read_blocking(|c| pending(c, "test", 10)).unwrap().is_empty());
        assert_eq!(db.read_blocking(|c| progress(c, "test")).unwrap(), (4, 4));
        assert_eq!(db.read_blocking(|c| progress(c, "other")).unwrap(), (0, 4));

        let found = db.read_blocking(|c| nearest(c, "test", &[1.0, 0.0, 0.0], 0.5, 10)).unwrap();
        assert_eq!(
            found.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(),
            ["t-invoice"],
            "lunch is too far; spam never"
        );
        assert!(found[0].1 > 0.99, "by its nearest message");
        assert!(db.read_blocking(|c| nearest(c, "other", &[1.0, 0.0, 0.0], 0.0, 10)).unwrap().is_empty());
        let bad = vec![(MessageId::new("m1"), vec![f32::NAN])];
        assert!(matches!(db.write_blocking(move |tx| store(tx, "test", &bad)).unwrap_err(), StoreError::Invalid(_)));
    }
}
