//! Connections, pragmas, migrations and the one-writer / N-reader model
//! (spec §6.1, §6.4).
//!
//! - One dedicated writer thread owns the only read-write connection. Writes
//!   are closures sent over a channel and run inside a transaction.
//! - A small pool of read-only connections serves reads. In WAL mode readers
//!   never wait for the writer.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use rusqlite::{Connection, OpenFlags, Transaction};
use tokio::sync::{Semaphore, oneshot};

use crate::error::{StoreError, StoreResult};

/// Migrations in order. `PRAGMA user_version` records how many have run.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/0001_initial.sql"),
    include_str!("../migrations/0002_draft_state.sql"),
    include_str!("../migrations/0003_attachment_data.sql"),
    include_str!("../migrations/0004_routines.sql"),
    include_str!("../migrations/0005_backfill_order.sql"),
    include_str!("../migrations/0006_header_only_index.sql"),
    include_str!("../migrations/0007_undo.sql"),
    include_str!("../migrations/0008_server_drafts.sql"),
    include_str!("../migrations/0009_tasks.sql"),
    include_str!("../migrations/0010_writing_guide.sql"),
    include_str!("../migrations/0011_draft_guide_version.sql"),
    include_str!("../migrations/0012_guide_batch_timing.sql"),
    include_str!("../migrations/0013_ai_compositions.sql"),
    include_str!("../migrations/0014_analysis_runs.sql"),
    include_str!("../migrations/0015_analysis_proposals.sql"),
    include_str!("../migrations/0016_analysis_undo.sql"),
    include_str!("../migrations/0017_facts.sql"),
    include_str!("../migrations/0018_cleanup.sql"),
    include_str!("../migrations/0019_cleanup_progress.sql"),
    include_str!("../migrations/0020_inbox_category_stats.sql"),
    include_str!("../migrations/0021_outbox_claims.sql"),
    include_str!("../migrations/0022_fact_share_with_cloud.sql"),
    include_str!("../migrations/0023_cloud_reports.sql"),
];

pub const READER_COUNT: usize = 4;

/// The migration scripts, in order (tests build old stores from them).
#[cfg(test)]
pub(crate) fn migrations() -> &'static [&'static str] {
    MIGRATIONS
}

pub fn schema_version() -> u32 {
    MIGRATIONS.len() as u32
}

type WriteJob = Box<dyn FnOnce(&mut Connection) + Send>;

/// Handle to an open store. Cheap to clone.
#[derive(Clone)]
pub struct Db {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    writer: Mutex<Option<mpsc::Sender<WriteJob>>>,
    /// [`Db::open_existing`]: the writer is opened by the first write, so a
    /// process that only reads never opens the store for writing.
    lazy_writer: Mutex<bool>,
    readers: Mutex<Vec<Connection>>,
    permits: Semaphore,
}

impl Db {
    /// Open (creating if needed) and migrate the database at `path`.
    pub fn open(path: &Path) -> StoreResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let writer = open_writer(path)?;
        let db = Self::with_readers(path, false)?;
        db.start_writer(writer)?;
        Ok(db)
    }

    /// Open a store another process owns, without creating or migrating it
    /// (the headless MCP, spec §10.1): refused unless its schema is exactly
    /// this build's. Reads use read-only connections; the first write opens
    /// the writer (no migration), so a reader never opens it for writing.
    pub fn open_existing(path: &Path) -> StoreResult<Self> {
        if !path.is_file() {
            return Err(StoreError::NotFound(format!("no store at {}", path.display())));
        }
        let db = Self::with_readers(path, true)?;
        db.read_blocking(check_version)?;
        Ok(db)
    }

    /// Whether this handle has opened the store for writing.
    pub fn writer_opened(&self) -> bool {
        self.inner.writer.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    fn with_readers(path: &Path, lazy_writer: bool) -> StoreResult<Self> {
        let mut readers = Vec::with_capacity(READER_COUNT);
        for _ in 0..READER_COUNT {
            let conn = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI,
            )?;
            configure(&conn, false)?;
            readers.push(conn);
        }
        Ok(Self {
            inner: Arc::new(Inner {
                path: path.to_owned(),
                writer: Mutex::new(None),
                lazy_writer: Mutex::new(lazy_writer),
                readers: Mutex::new(readers),
                permits: Semaphore::new(READER_COUNT),
            }),
        })
    }

    /// Run the writer connection on its own thread.
    fn start_writer(&self, writer: Connection) -> StoreResult<()> {
        let (tx, rx) = mpsc::channel::<WriteJob>();
        thread::Builder::new()
            .name("openagc-store-writer".into())
            .spawn(move || {
                let mut conn = writer;
                while let Ok(job) = rx.recv() {
                    job(&mut conn);
                }
            })
            .map_err(|e| StoreError::Io(e.to_string()))?;
        *self.inner.writer.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        Ok(())
    }

    /// Whether both handles are the same open store.
    pub fn same_store(&self, other: &Db) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Run `f` in a write transaction on the writer thread. Commits if `f`
    /// returns `Ok`, rolls back otherwise.
    pub async fn write<T, F>(&self, f: F) -> StoreResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> StoreResult<T> + Send + 'static,
    {
        let rx = self.submit(f)?;
        rx.await.map_err(|_| StoreError::Closed)?
    }

    /// Blocking variant of [`Db::write`] for non-async callers and tests.
    /// Must not be called from inside a tokio runtime.
    pub fn write_blocking<T, F>(&self, f: F) -> StoreResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> StoreResult<T> + Send + 'static,
    {
        let rx = self.submit(f)?;
        rx.blocking_recv().map_err(|_| StoreError::Closed)?
    }

    fn submit<T, F>(&self, f: F) -> StoreResult<oneshot::Receiver<StoreResult<T>>>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> StoreResult<T> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let job: WriteJob = Box::new(move |conn| {
            let result = (|| {
                // The write lock up front: another process writing the same
                // store (the headless MCP, spec §7.4) makes a deferred
                // transaction that read first fail at its first write with
                // SQLITE_BUSY at once, past the busy timeout. Taken at BEGIN,
                // the lock is waited for.
                let txn = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let value = f(&txn)?;
                txn.commit()?;
                Ok(value)
            })();
            let _ = tx.send(result);
        });
        let mut lazy = self.inner.lazy_writer.lock().unwrap_or_else(|e| e.into_inner());
        if *lazy {
            // The first write of an `open_existing` store: the writer, with
            // the schema checked again (it may have changed since opening).
            let conn = Connection::open_with_flags(&self.inner.path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
            configure(&conn, true)?;
            check_version(&conn)?;
            self.start_writer(conn)?;
            *lazy = false;
        }
        drop(lazy);
        let guard = self.inner.writer.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().ok_or(StoreError::Closed)?.send(job).map_err(|_| StoreError::Closed)?;
        Ok(rx)
    }

    /// Run `f` on a pooled read-only connection, off the async executor.
    pub async fn read<T, F>(&self, f: F) -> StoreResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> StoreResult<T> + Send + 'static,
    {
        let _permit = self.inner.permits.acquire().await.map_err(|_| StoreError::Closed)?;
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || with_reader(&inner, f))
            .await
            .map_err(|e| StoreError::Io(format!("read task failed: {e}")))?
    }

    /// Blocking variant of [`Db::read`].
    pub fn read_blocking<T, F>(&self, f: F) -> StoreResult<T>
    where
        F: FnOnce(&Connection) -> StoreResult<T>,
    {
        with_reader(&self.inner, f)
    }

    /// Stop accepting writes. Queued writes still run.
    pub fn close(&self) {
        *self.inner.lazy_writer.lock().unwrap_or_else(|e| e.into_inner()) = false;
        self.inner.writer.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

fn with_reader<T>(inner: &Inner, f: impl FnOnce(&Connection) -> StoreResult<T>) -> StoreResult<T> {
    let conn = inner.readers.lock().unwrap_or_else(|e| e.into_inner()).pop();
    // All pooled readers busy (only possible for blocking callers, which
    // skip the semaphore): open a temporary one rather than wait.
    let conn = match conn {
        Some(c) => c,
        None => {
            let c = Connection::open_with_flags(&inner.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            configure(&c, false)?;
            return f(&c);
        }
    };
    let result = f(&conn);
    inner.readers.lock().unwrap_or_else(|e| e.into_inner()).push(conn);
    result
}

/// The writer connection, configured and migrated. Another opener setting
/// up the same store (switching it to WAL, migrating) can answer "busy"
/// without waiting; try again for a few seconds.
fn open_writer(path: &Path) -> StoreResult<Connection> {
    let busy = |e: &StoreError| {
        matches!(e, StoreError::Sqlite(rusqlite::Error::SqliteFailure(f, _))
            if matches!(f.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
    };
    let mut attempt = 0;
    loop {
        let result = Connection::open(path).map_err(StoreError::from).and_then(|mut conn| {
            configure(&conn, true)?;
            migrate(&mut conn)?;
            Ok(conn)
        });
        match result {
            Err(e) if busy(&e) && attempt < 50 => {
                attempt += 1;
                thread::sleep(std::time::Duration::from_millis(100));
            }
            other => return other,
        }
    }
}

/// How long a write waits for another process's write to finish before
/// failing. Writes are short (a sync batch is well under a second); this
/// is far past any of them, so a second writer is waited for, never
/// reported to the user.
const WRITER_BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn configure(conn: &Connection, writer: bool) -> StoreResult<()> {
    conn.busy_timeout(if writer { WRITER_BUSY_TIMEOUT } else { std::time::Duration::from_secs(5) })?;
    if writer {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "cache_size", -65_536)?; // 64 MB
    } else {
        conn.pragma_update(None, "cache_size", -16_384)?; // 16 MB
    }
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "mmap_size", 256 * 1024 * 1024)?;
    conn.set_prepared_statement_cache_capacity(128);
    Ok(())
}

/// Refuse a store whose schema is not this build's: an opener that may not
/// migrate cannot read an older one correctly, nor a newer one at all.
fn check_version(conn: &Connection) -> StoreResult<()> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let target = schema_version();
    match current.cmp(&target) {
        std::cmp::Ordering::Equal => Ok(()),
        std::cmp::Ordering::Less => Err(StoreError::Version(format!(
            "this mailbox's store is from an older OpenAGC (schema v{current}; this build reads v{target}); \
             open OpenAGC once to update it"
        ))),
        std::cmp::Ordering::Greater => Err(StoreError::Version(format!(
            "this mailbox's store is from a newer OpenAGC (schema v{current}; this build reads v{target}); \
             update OpenAGC"
        ))),
    }
}

fn migrate(conn: &mut Connection) -> StoreResult<()> {
    let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let target = schema_version();
    if current > target {
        return Err(StoreError::Migration(format!(
            "database schema v{current} is newer than this build (v{target}); update OpenAGC"
        )));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let version = i as u32 + 1;
        // The write lock first, then the version again: two openers of the
        // same store at launch both read the old version, and the second
        // then failed on what the first had just added.
        let txn = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let now: u32 = txn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if now >= version {
            continue;
        }
        txn.execute_batch(sql).map_err(|e| StoreError::Migration(format!("v{version}: {e}")))?;
        // Steps SQL cannot do, in the same transaction.
        if version == crate::facts::MIGRATION_VERSION {
            crate::facts::move_guide_facts(&txn)
                .map_err(|e| StoreError::Migration(format!("v{version}: moving facts: {e}")))?;
        }
        // user_version cannot be bound as a parameter.
        txn.execute_batch(&format!("PRAGMA user_version = {version}"))?;
        txn.commit()?;
        tracing::info!(version, "store migrated");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn temp_db_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("openagc-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("mail.sqlite")
    }

    #[test]
    fn two_openers_at_once_both_migrate_cleanly() {
        let dir = std::env::temp_dir().join(format!("openagc-store-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("mail.sqlite");
        for _ in 0..5 {
            let _ = std::fs::remove_dir_all(&dir);
            let handles: Vec<_> = (0..4)
                .map(|_| {
                    let path = path.clone();
                    std::thread::spawn(move || Db::open(&path).map(|_| ()))
                })
                .collect();
            for h in handles {
                h.join().unwrap().unwrap();
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opens_migrates_and_reports_version() {
        let db = Db::open(&temp_db_path("migrate")).unwrap();
        let version: u32 = db.read_blocking(|c| Ok(c.pragma_query_value(None, "user_version", |r| r.get(0))?)).unwrap();
        assert_eq!(version, schema_version());
        let mode: String = db.read_blocking(|c| Ok(c.pragma_query_value(None, "journal_mode", |r| r.get(0))?)).unwrap();
        assert_eq!(mode, "wal");
        // Seeded virtual label.
        let n: i64 = db
            .read_blocking(|c| {
                Ok(c.query_row("SELECT COUNT(*) FROM labels WHERE gmail_id = '@archive'", [], |r| r.get(0))?)
            })
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn reopening_does_not_rerun_migrations() {
        let path = temp_db_path("reopen");
        Db::open(&path).unwrap().close();
        let db = Db::open(&path).unwrap();
        let n: i64 = db.read_blocking(|c| Ok(c.query_row("SELECT COUNT(*) FROM labels", [], |r| r.get(0))?)).unwrap();
        assert_eq!(n, 1, "seed row inserted once");
    }

    #[test]
    fn upgrading_counts_the_inbox_tabs_of_the_mail_already_stored() {
        use mail_domain::{LabelId, MessageId, ThreadId};
        let path = temp_db_path("tab-counts");
        let db = Db::open(&path).unwrap();
        db.write_blocking(|tx| {
            let mut w = crate::MailWriter::new(tx);
            for (id, labels) in [
                ("a", &["INBOX", "UNREAD"][..]),
                ("b", &["INBOX", "CATEGORY_FORUMS", "CATEGORY_SOCIAL", "UNREAD"]),
                ("c", &["INBOX", "CATEGORY_SOCIAL"]),
                ("d", &["CATEGORY_SOCIAL", "UNREAD"]),
            ] {
                w.upsert_message(&crate::IncomingMessage {
                    id: MessageId::new(id),
                    thread_id: ThreadId::new(id),
                    label_ids: labels.iter().map(|l| LabelId::new(*l)).collect(),
                    ..Default::default()
                })?;
            }
            w.finish()
        })
        .unwrap();
        db.close();
        // The store as it was before the counts were kept.
        let c = Connection::open(&path).unwrap();
        c.execute_batch(
            "DROP TABLE inbox_category_stats;
             ALTER TABLE outbox DROP COLUMN claimed_by; ALTER TABLE outbox DROP COLUMN lease_until;
             ALTER TABLE facts DROP COLUMN share_with_cloud;
             DROP TABLE cloud_reports;
             PRAGMA user_version = 19",
        )
        .unwrap();
        drop(c);
        let db = Db::open(&path).unwrap();
        let tabs = db.read_blocking(crate::read::stored_inbox_categories).unwrap();
        let tabs: Vec<_> = tabs.iter().map(|t| (t.id.as_str(), t.total, t.unread)).collect();
        assert_eq!(
            tabs,
            [
                ("CATEGORY_PERSONAL", 1, 1),
                ("CATEGORY_PROMOTIONS", 0, 0),
                ("CATEGORY_SOCIAL", 2, 1),
                ("CATEGORY_UPDATES", 0, 0),
                ("CATEGORY_FORUMS", 0, 0),
            ]
        );
        assert_eq!(db.read_blocking(crate::consistency::check).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn newer_schema_is_refused() {
        let path = temp_db_path("newer");
        Db::open(&path).unwrap().close();
        let c = Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA user_version = 999").unwrap();
        drop(c);
        let err = Db::open(&path).err().unwrap();
        assert!(matches!(err, StoreError::Migration(_)), "{err}");
    }

    #[test]
    fn an_existing_store_opens_without_migrating_and_reads_never_open_the_writer() {
        let path = temp_db_path("existing");
        assert!(matches!(Db::open_existing(&path), Err(StoreError::NotFound(_))), "never created");
        assert!(!path.exists());
        let owner = Db::open(&path).unwrap();
        owner.write_blocking(|t| Ok(t.execute("INSERT INTO sync_state (key, value) VALUES ('k', 'v')", [])?)).unwrap();
        owner.close();
        drop(owner);

        let other = Db::open_existing(&path).unwrap();
        let read = |db: &Db| -> String {
            db.read_blocking(|c| Ok(c.query_row("SELECT value FROM sync_state WHERE key = 'k'", [], |r| r.get(0))?))
                .unwrap()
        };
        assert_eq!(read(&other), "v");
        assert!(!other.writer_opened(), "reading never opens the store for writing");
        other.write_blocking(|t| Ok(t.execute("UPDATE sync_state SET value = 'w' WHERE key = 'k'", [])?)).unwrap();
        assert!(other.writer_opened());
        assert_eq!(read(&other), "w");
        other.close();
        assert!(matches!(
            other.write_blocking(|t| Ok(t.execute("DELETE FROM sync_state", [])?)),
            Err(StoreError::Closed)
        ));
    }

    #[test]
    fn an_existing_store_of_another_schema_is_refused_in_words() {
        for (version, says) in [(3, "older OpenAGC"), (999, "newer OpenAGC")] {
            let path = temp_db_path(&format!("existing-v{version}"));
            Db::open(&path).unwrap().close();
            let c = Connection::open(&path).unwrap();
            c.execute_batch(&format!("PRAGMA user_version = {version}")).unwrap();
            drop(c);
            let err = Db::open_existing(&path).err().unwrap();
            assert!(matches!(err, StoreError::Version(_)), "{err}");
            assert!(err.to_string().contains(says), "{err}");
            let still: u32 =
                Connection::open(&path).unwrap().pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
            assert_eq!(still, version, "nothing was migrated");
        }
    }

    #[test]
    fn failed_write_rolls_back() {
        let db = Db::open(&temp_db_path("rollback")).unwrap();
        let r: StoreResult<()> = db.write_blocking(|t| {
            t.execute("INSERT INTO sync_state (key, value) VALUES ('k', 'v')", [])?;
            Err(StoreError::NotFound("forced".into()))
        });
        assert!(r.is_err());
        let n: i64 =
            db.read_blocking(|c| Ok(c.query_row("SELECT COUNT(*) FROM sync_state", [], |r| r.get(0))?)).unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn readers_see_committed_writes_and_run_concurrently() {
        let db = Db::open(&temp_db_path("concurrent")).unwrap();
        db.write(|t| {
            t.execute("INSERT INTO sync_state (key, value) VALUES ('history_id', '42')", [])?;
            Ok(())
        })
        .await
        .unwrap();
        let reads = (0..16).map(|_| {
            let db = db.clone();
            tokio::spawn(async move {
                db.read(|c| {
                    Ok(c.query_row("SELECT value FROM sync_state WHERE key = 'history_id'", [], |r| {
                        r.get::<_, String>(0)
                    })?)
                })
                .await
            })
        });
        for r in reads {
            assert_eq!(r.await.unwrap().unwrap(), "42");
        }
    }

    #[test]
    fn fts5_contentless_delete_and_trigram_are_available() {
        let db = Db::open(&temp_db_path("fts")).unwrap();
        db.write_blocking(|t| {
            t.execute("INSERT INTO messages_fts (rowid, subject, body) VALUES (7, 'Quarterly report', 'numbers')", [])?;
            t.execute("DELETE FROM messages_fts WHERE rowid = 7", [])?;
            t.execute("INSERT INTO contacts (email, name) VALUES ('johnny@example.com', 'Johnny')", [])?;
            Ok(())
        })
        .unwrap();
        let (fts, tri): (i64, i64) = db
            .read_blocking(|c| {
                let fts =
                    c.query_row("SELECT COUNT(*) FROM messages_fts WHERE messages_fts MATCH 'quarterly'", [], |r| {
                        r.get(0)
                    })?;
                let tri =
                    c.query_row("SELECT COUNT(*) FROM contacts_fts WHERE contacts_fts MATCH 'ohn'", [], |r| r.get(0))?;
                Ok((fts, tri))
            })
            .unwrap();
        assert_eq!(fts, 0, "row deleted by rowid");
        assert_eq!(tri, 1, "trigram substring match");
    }
}
