//! The server's one SQLite file (spec §10.6): registered mailboxes with
//! their publisher token's hash, the last few published snapshots of each,
//! and the agents connected to them: static agent tokens and OAuth grants,
//! one table (`agent_tokens`, `kind` `token` or `oauth`). For OAuth it also
//! holds registered clients, connect codes, authorization codes and access
//! and refresh tokens, every secret as a hash.
//!
//! One connection behind a mutex, used from blocking tasks: the server's
//! writes are a push now and then, and its reads are small.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params};

/// The file in the data directory.
pub const FILE_NAME: &str = "rules.sqlite3";

/// Snapshots kept per mailbox, newest first.
pub const KEEP_VERSIONS: i64 = 5;

/// Migrations in order; `PRAGMA user_version` records how many have run.
const MIGRATIONS: &[&str] = &[
    "
    CREATE TABLE mailboxes (
        id INTEGER PRIMARY KEY,
        address TEXT NOT NULL UNIQUE,
        publisher_token_hash TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE snapshots (
        mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
        version INTEGER NOT NULL,
        json TEXT NOT NULL,
        published_at INTEGER NOT NULL,
        received_at INTEGER NOT NULL,
        PRIMARY KEY (mailbox_id, version)
    );
    CREATE TABLE agent_tokens (
        id TEXT PRIMARY KEY,
        mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        token_hash TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        revoked_at INTEGER
    );
    CREATE INDEX agent_tokens_by_mailbox ON agent_tokens(mailbox_id);
",
    "
    ALTER TABLE agent_tokens ADD COLUMN kind TEXT NOT NULL DEFAULT 'token';
    ALTER TABLE agent_tokens ADD COLUMN client_id TEXT;
    CREATE TABLE oauth_clients (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        redirect_uris TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE connect_codes (
        id TEXT PRIMARY KEY,
        mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        code_hash TEXT NOT NULL UNIQUE,
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL,
        used_at INTEGER
    );
    CREATE INDEX connect_codes_by_mailbox ON connect_codes(mailbox_id);
    CREATE TABLE oauth_codes (
        code_hash TEXT PRIMARY KEY,
        grant_id TEXT NOT NULL REFERENCES agent_tokens(id) ON DELETE CASCADE,
        client_id TEXT NOT NULL,
        redirect_uri TEXT NOT NULL,
        code_challenge TEXT NOT NULL,
        resource TEXT NOT NULL,
        expires_at INTEGER NOT NULL,
        used_at INTEGER
    );
    CREATE TABLE oauth_tokens (
        token_hash TEXT PRIMARY KEY,
        kind TEXT NOT NULL,
        grant_id TEXT NOT NULL REFERENCES agent_tokens(id) ON DELETE CASCADE,
        resource TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL,
        used_at INTEGER
    );
    CREATE INDEX oauth_tokens_by_grant ON oauth_tokens(grant_id);
",
    "
    ALTER TABLE agent_tokens ADD COLUMN last_used_at INTEGER;
",
    // Reports of what agents sent (`report_send`), waiting for the app to
    // pull them. AUTOINCREMENT: an id is never used twice, so the app's
    // `after` cursor and `ack` never skip or repeat one.
    "
    CREATE TABLE reports (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
        agent_id TEXT NOT NULL,
        received_at INTEGER NOT NULL,
        message_id TEXT,
        recipients TEXT NOT NULL,
        subject TEXT NOT NULL,
        sent_at INTEGER,
        body_markdown TEXT NOT NULL,
        checked_version INTEGER,
        check_version INTEGER,
        guide_check TEXT NOT NULL
    );
    CREATE INDEX reports_by_mailbox ON reports(mailbox_id, id);
    CREATE INDEX reports_by_time ON reports(received_at);
    ALTER TABLE mailboxes ADD COLUMN reports_dropped INTEGER NOT NULL DEFAULT 0;
",
    // Encryption at rest (spec §10.6, oagc-gmn7.7): the app's public key per
    // mailbox; a sealed snapshot's key id and box (its `json` left empty);
    // each agent's key wrapped under its credential and sealed to the app;
    // each push's snapshot key wrapped per agent; a report's sealed box.
    "
    ALTER TABLE mailboxes ADD COLUMN app_key TEXT;
    ALTER TABLE snapshots ADD COLUMN key_id TEXT;
    ALTER TABLE snapshots ADD COLUMN sealed BLOB;
    ALTER TABLE agent_tokens ADD COLUMN key_wrap BLOB;
    ALTER TABLE agent_tokens ADD COLUMN app_seal BLOB;
    CREATE TABLE snapshot_keys (
        mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id) ON DELETE CASCADE,
        key_id TEXT NOT NULL,
        agent_id TEXT NOT NULL REFERENCES agent_tokens(id) ON DELETE CASCADE,
        wrap BLOB NOT NULL,
        PRIMARY KEY (key_id, agent_id)
    );
    CREATE INDEX snapshot_keys_by_agent ON snapshot_keys(agent_id);
    CREATE INDEX snapshot_keys_by_mailbox ON snapshot_keys(mailbox_id);
    ALTER TABLE reports ADD COLUMN sealed BLOB;
",
    // A sealed snapshot's schema, bound to its box (envelope 2); and the
    // database's epoch, a random id made with it, which the app keys the
    // reports it pulled by: a database restored or made again reuses
    // report ids, never its epoch. Upgrading to this compacts the file
    // once (`migrate`).
    "
    ALTER TABLE snapshots ADD COLUMN schema_version INTEGER;
    CREATE TABLE server_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    INSERT INTO server_meta (key, value) VALUES ('epoch', lower(hex(randomblob(16))));
",
];

/// The first schema with everything deleted before overwritten: a database
/// upgraded from before it is compacted once, so what earlier versions
/// deleted (plaintext snapshots, reports, keys) leaves its free pages.
const COMPACTED_FROM: u32 = 6;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("data directory {0}: {1}")]
    Io(PathBuf, std::io::Error),
    #[error("the database is from a newer openagc-rules (schema {0}; this build knows {known})", known = MIGRATIONS.len())]
    TooNew(u32),
    #[error("database task failed: {0}")]
    Task(String),
}

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    /// Open (creating and migrating) the database in `dir`. The directory
    /// is made readable by its owner only, and so is the file.
    pub fn open(dir: &Path) -> Result<Self, DbError> {
        let io = |e| DbError::Io(dir.to_owned(), e);
        if !dir.exists() {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            builder.create(dir).map_err(io)?;
        }
        let path = dir.join(FILE_NAME);
        let mut conn = Connection::open(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(io)?;
        }
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        // What is deleted is overwritten, so a plaintext snapshot replaced by
        // an encrypted one does not linger in free pages.
        conn.pragma_update(None, "secure_delete", true)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&mut conn)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// Run `f` on the connection in a blocking task.
    pub async fn run<T, F>(&self, f: F) -> Result<T, DbError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = conn.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            f(&mut c).map_err(DbError::from)
        })
        .await
        .map_err(|e| DbError::Task(e.to_string()))?
    }

    /// Run `f` on the connection here, for the command line.
    pub fn run_now<T>(&self, f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> Result<T, DbError> {
        let mut c = self.conn.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut c).map_err(DbError::from)
    }
}

fn migrate(conn: &mut Connection) -> Result<(), DbError> {
    let done: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let known = u32::try_from(MIGRATIONS.len()).unwrap_or(u32::MAX);
    if done > known {
        return Err(DbError::TooNew(done));
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(done as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i64::try_from(i + 1).unwrap_or(i64::MAX))?;
        tx.commit()?;
    }
    if done > 0 && done < COMPACTED_FROM {
        compact(conn)?;
    }
    Ok(())
}

/// Copy the write-ahead log into the file and empty it: deleted pages,
/// zeroed in the file (`secure_delete`), leave the log too.
pub fn scrub(c: &Connection) -> rusqlite::Result<()> {
    c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
}

/// Rebuild the file without its free pages, then [`scrub`]: nothing
/// deleted before (even without `secure_delete`) is left in it.
pub fn compact(c: &Connection) -> rusqlite::Result<()> {
    c.execute_batch("VACUUM")?;
    scrub(c)
}

/// The database's epoch (migration 6): a random id made with it.
pub fn epoch(c: &Connection) -> rusqlite::Result<String> {
    c.query_row("SELECT value FROM server_meta WHERE key = 'epoch'", [], |r| r.get(0))
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[derive(Debug, Clone)]
pub struct MailboxRow {
    pub id: i64,
    pub address: String,
    pub publisher_token_hash: String,
}

pub fn mailbox_by_address(c: &Connection, address: &str) -> rusqlite::Result<Option<MailboxRow>> {
    c.query_row("SELECT id, address, publisher_token_hash FROM mailboxes WHERE address = ?1", [address], |r| {
        Ok(MailboxRow { id: r.get(0)?, address: r.get(1)?, publisher_token_hash: r.get(2)? })
    })
    .optional()
}

/// Register `address`; `None` when it already is (the first registration
/// wins).
pub fn insert_mailbox(c: &Connection, address: &str, token_hash: &str, now: i64) -> rusqlite::Result<Option<i64>> {
    let n = c.execute(
        "INSERT INTO mailboxes (address, publisher_token_hash, created_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(address) DO NOTHING",
        params![address, token_hash, now],
    )?;
    Ok((n == 1).then(|| c.last_insert_rowid()))
}

/// Forget a mailbox, its snapshots and its agent tokens.
pub fn delete_mailbox(c: &Connection, id: i64) -> rusqlite::Result<bool> {
    Ok(c.execute("DELETE FROM mailboxes WHERE id = ?1", [id])? == 1)
}

#[derive(Debug, Clone, Default)]
pub struct SnapshotRow {
    pub version: i64,
    /// The snapshot's JSON; empty when it is sealed.
    pub json: String,
    pub published_at: i64,
    /// A sealed snapshot's key id and box (`rules_crypto`).
    pub key_id: Option<String>,
    pub sealed: Option<Vec<u8>>,
    /// A sealed snapshot's schema, bound to its box; `None` for an envelope
    /// 1 box (and plaintext, whose JSON says).
    pub schema_version: Option<u32>,
}

const SNAPSHOT_COLUMNS: &str = "version, json, published_at, key_id, sealed, schema_version";

fn snapshot_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SnapshotRow> {
    Ok(SnapshotRow {
        version: r.get(0)?,
        json: r.get(1)?,
        published_at: r.get(2)?,
        key_id: r.get(3)?,
        sealed: r.get(4)?,
        schema_version: r.get(5)?,
    })
}

pub fn latest_snapshot(c: &Connection, mailbox_id: i64) -> rusqlite::Result<Option<SnapshotRow>> {
    c.query_row(
        &format!("SELECT {SNAPSHOT_COLUMNS} FROM snapshots WHERE mailbox_id = ?1 ORDER BY version DESC LIMIT 1"),
        [mailbox_id],
        snapshot_row,
    )
    .optional()
}

/// The newest snapshot, with its key's wrap for `agent` if it is sealed
/// and wrapped for it. Only ever the newest: an agent the app has not
/// wrapped it for is told so, never served an older version.
pub fn newest_snapshot_for(
    c: &Connection,
    mailbox_id: i64,
    agent: Option<&str>,
) -> rusqlite::Result<Option<(SnapshotRow, Option<Vec<u8>>)>> {
    c.query_row(
        "SELECT s.version, s.json, s.published_at, s.key_id, s.sealed, s.schema_version, k.wrap FROM snapshots s \
         LEFT JOIN snapshot_keys k ON k.key_id = s.key_id AND k.agent_id = ?2 \
         WHERE s.mailbox_id = ?1 ORDER BY s.version DESC LIMIT 1",
        params![mailbox_id, agent.unwrap_or_default()],
        |r| Ok((snapshot_row(r)?, r.get(6)?)),
    )
    .optional()
}

/// The mailbox's public key for sealing to the app, if it pushed one.
pub fn app_key(c: &Connection, mailbox_id: i64) -> rusqlite::Result<Option<String>> {
    Ok(c.query_row("SELECT app_key FROM mailboxes WHERE id = ?1", [mailbox_id], |r| r.get(0)).optional()?.flatten())
}

/// Keep the app's public key. A new one makes every agent's key sealed to
/// the old one useless: those seals go, and each is made again at the
/// agent's next request.
pub fn set_app_key(c: &Connection, mailbox_id: i64, key: &str) -> rusqlite::Result<()> {
    let n =
        c.execute("UPDATE mailboxes SET app_key = ?2 WHERE id = ?1 AND app_key IS NOT ?2", params![mailbox_id, key])?;
    if n > 0 {
        c.execute("UPDATE agent_tokens SET app_seal = NULL WHERE mailbox_id = ?1", [mailbox_id])?;
    }
    Ok(())
}

/// Bytes that may not be stored yet.
pub type MaybeBytes = Option<Vec<u8>>;

/// An agent's key, wrapped under its credential and sealed to the app.
pub fn agent_keys(c: &Connection, agent_id: &str) -> rusqlite::Result<(MaybeBytes, MaybeBytes)> {
    Ok(c.query_row("SELECT key_wrap, app_seal FROM agent_tokens WHERE id = ?1", [agent_id], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })
    .optional()?
    .unwrap_or((None, None)))
}

/// Store an agent's new key (both forms); its wraps of earlier snapshot
/// keys were under the old one and go.
pub fn set_agent_keys(
    c: &Connection,
    agent_id: &str,
    key_wrap: &[u8],
    app_seal: Option<&[u8]>,
) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE agent_tokens SET key_wrap = ?2, app_seal = ?3 WHERE id = ?1 AND revoked_at IS NULL",
        params![agent_id, key_wrap, app_seal],
    )?;
    c.execute("DELETE FROM snapshot_keys WHERE agent_id = ?1", [agent_id])?;
    Ok(())
}

/// An agent's key wrapped under a new credential (a grant's rotated
/// secret): the same key, so its snapshot key wraps and its seal for the
/// app stay.
pub fn set_key_wrap(c: &Connection, agent_id: &str, key_wrap: &[u8]) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE agent_tokens SET key_wrap = ?2 WHERE id = ?1 AND revoked_at IS NULL",
        params![agent_id, key_wrap],
    )?;
    Ok(())
}

pub fn set_app_seal(c: &Connection, agent_id: &str, app_seal: &[u8]) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE agent_tokens SET app_seal = ?2 WHERE id = ?1 AND revoked_at IS NULL AND key_wrap IS NOT NULL",
        params![agent_id, app_seal],
    )?;
    Ok(())
}

/// Store the app's wraps of the snapshot key `key_id` for the mailbox's
/// live agents (others are skipped). How many were stored.
pub fn insert_snapshot_keys(
    c: &Connection,
    mailbox_id: i64,
    key_id: &str,
    wraps: &[(String, Vec<u8>)],
) -> rusqlite::Result<usize> {
    let mut stored = 0;
    for (agent_id, wrap) in wraps {
        stored += c.execute(
            "INSERT OR REPLACE INTO snapshot_keys (mailbox_id, key_id, agent_id, wrap) \
             SELECT ?1, ?2, id, ?4 FROM agent_tokens WHERE id = ?3 AND mailbox_id = ?1 AND revoked_at IS NULL",
            params![mailbox_id, key_id, agent_id, wrap],
        )?;
    }
    Ok(stored)
}

/// Whether the mailbox keeps a snapshot sealed under `key_id`.
pub fn has_key_id(c: &Connection, mailbox_id: i64, key_id: &str) -> rusqlite::Result<bool> {
    c.query_row(
        "SELECT EXISTS (SELECT 1 FROM snapshots WHERE mailbox_id = ?1 AND key_id = ?2)",
        params![mailbox_id, key_id],
        |r| r.get(0),
    )
}

/// Each of the mailbox's agents' key sealed to the app, and whether the
/// newest snapshot is readable to it (plaintext, or its key wrapped for it).
pub fn agent_readability(c: &Connection, mailbox_id: i64) -> rusqlite::Result<Vec<(String, MaybeBytes, bool)>> {
    let mut stmt = c.prepare(
        "SELECT t.id, t.app_seal, \
         COALESCE((SELECT s.sealed IS NULL OR EXISTS (SELECT 1 FROM snapshot_keys k \
                   WHERE k.key_id = s.key_id AND k.agent_id = t.id) \
                   FROM snapshots s WHERE s.mailbox_id = t.mailbox_id ORDER BY s.version DESC LIMIT 1), 0) \
         FROM agent_tokens t WHERE t.mailbox_id = ?1",
    )?;
    stmt.query_map([mailbox_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect()
}

/// A mailbox's plaintext snapshots go once it publishes encrypted ones.
/// How many went.
pub fn delete_plain_snapshots(c: &Connection, mailbox_id: i64) -> rusqlite::Result<usize> {
    c.execute("DELETE FROM snapshots WHERE mailbox_id = ?1 AND sealed IS NULL", [mailbox_id])
}

/// The versions kept, newest first.
pub fn versions(c: &Connection, mailbox_id: i64) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = c.prepare("SELECT version FROM snapshots WHERE mailbox_id = ?1 ORDER BY version DESC")?;
    stmt.query_map([mailbox_id], |r| r.get(0))?.collect()
}

/// Store a snapshot and drop all but the newest [`KEEP_VERSIONS`].
pub fn insert_snapshot(c: &Connection, mailbox_id: i64, row: &SnapshotRow, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO snapshots (mailbox_id, version, json, published_at, received_at, key_id, sealed, \
         schema_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![mailbox_id, row.version, row.json, row.published_at, now, row.key_id, row.sealed, row.schema_version],
    )?;
    c.execute(
        "DELETE FROM snapshots WHERE mailbox_id = ?1 AND version NOT IN \
         (SELECT version FROM snapshots WHERE mailbox_id = ?1 ORDER BY version DESC LIMIT ?2)",
        params![mailbox_id, KEEP_VERSIONS],
    )?;
    // Wraps of keys no kept snapshot uses.
    c.execute(
        "DELETE FROM snapshot_keys WHERE mailbox_id = ?1 AND key_id NOT IN \
         (SELECT key_id FROM snapshots WHERE mailbox_id = ?1 AND key_id IS NOT NULL)",
        [mailbox_id],
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AgentTokenRow {
    pub id: String,
    pub mailbox_id: i64,
    pub name: String,
    pub token_hash: String,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
    /// `token` (a static agent token) or `oauth` (a grant made with a
    /// connect code, whose tokens are in `oauth_tokens`).
    pub kind: String,
    /// The OAuth client a grant was made for.
    pub client_id: Option<String>,
}

pub const KIND_TOKEN: &str = "token";
pub const KIND_OAUTH: &str = "oauth";

fn agent_token_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentTokenRow> {
    Ok(AgentTokenRow {
        id: r.get(0)?,
        mailbox_id: r.get(1)?,
        name: r.get(2)?,
        token_hash: r.get(3)?,
        created_at: r.get(4)?,
        revoked_at: r.get(5)?,
        kind: r.get(6)?,
        client_id: r.get(7)?,
    })
}

const AGENT_TOKEN_COLUMNS: &str = "id, mailbox_id, name, token_hash, created_at, revoked_at, kind, client_id";

/// An agent token with the address of its mailbox.
pub fn agent_token(c: &Connection, id: &str) -> rusqlite::Result<Option<(AgentTokenRow, String)>> {
    c.query_row(
        "SELECT t.id, t.mailbox_id, t.name, t.token_hash, t.created_at, t.revoked_at, t.kind, t.client_id, \
         m.address FROM agent_tokens t JOIN mailboxes m ON m.id = t.mailbox_id WHERE t.id = ?1",
        [id],
        |r| Ok((agent_token_row(r)?, r.get(8)?)),
    )
    .optional()
}

pub fn insert_agent_token(c: &Connection, row: &AgentTokenRow) -> rusqlite::Result<()> {
    c.execute(
        &format!("INSERT INTO agent_tokens ({AGENT_TOKEN_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"),
        params![
            row.id,
            row.mailbox_id,
            row.name,
            row.token_hash,
            row.created_at,
            row.revoked_at,
            row.kind,
            row.client_id
        ],
    )?;
    Ok(())
}

/// A mailbox's agent as listed: the row, its OAuth client's name if it is
/// a grant, and when it was last let in (to the minute).
pub type ListedAgent = (AgentTokenRow, Option<String>, Option<i64>);

/// A mailbox's agents (tokens and grants), oldest first.
pub fn agent_tokens(c: &Connection, mailbox_id: i64) -> rusqlite::Result<Vec<ListedAgent>> {
    let mut stmt = c.prepare(
        "SELECT t.id, t.mailbox_id, t.name, t.token_hash, t.created_at, t.revoked_at, t.kind, t.client_id, \
         c.name, t.last_used_at FROM agent_tokens t LEFT JOIN oauth_clients c ON c.id = t.client_id \
         WHERE t.mailbox_id = ?1 ORDER BY t.created_at, t.id",
    )?;
    stmt.query_map([mailbox_id], |r| Ok((agent_token_row(r)?, r.get(8)?, r.get(9)?)))?.collect()
}

/// How stale `last_used_at` may get before a request writes it again.
pub const LAST_USED_GRAIN_MS: i64 = 60_000;

/// Note that an agent was let in at `now`, at most once a minute.
pub fn touch_agent(c: &Connection, id: &str, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE agent_tokens SET last_used_at = ?2 WHERE id = ?1 AND (last_used_at IS NULL OR last_used_at <= ?2 - ?3)",
        params![id, now, LAST_USED_GRAIN_MS],
    )?;
    Ok(())
}

/// Revoke one of the mailbox's agent tokens; false if it has no such
/// token. Revoking twice keeps the first time.
pub fn revoke_agent_token(c: &Connection, mailbox_id: i64, id: &str, now: i64) -> rusqlite::Result<bool> {
    let n = c.execute(
        "UPDATE agent_tokens SET revoked_at = COALESCE(revoked_at, ?3) WHERE mailbox_id = ?1 AND id = ?2",
        params![mailbox_id, id, now],
    )?;
    if n == 1 {
        forget_grant_secrets(c, id)?;
    }
    Ok(n == 1)
}

/// Revoke an agent whatever its mailbox: a refresh token or authorization
/// code used twice.
pub fn revoke_grant(c: &Connection, id: &str, now: i64) -> rusqlite::Result<()> {
    c.execute("UPDATE agent_tokens SET revoked_at = COALESCE(revoked_at, ?2) WHERE id = ?1", params![id, now])?;
    forget_grant_secrets(c, id)
}

/// A revoked grant's codes and tokens are of no use: drop them. (Its row
/// stays, revoked, for the app's list; a token presented later finds
/// nothing.)
fn forget_grant_secrets(c: &Connection, id: &str) -> rusqlite::Result<()> {
    c.execute("DELETE FROM oauth_tokens WHERE grant_id = ?1", [id])?;
    c.execute("DELETE FROM oauth_codes WHERE grant_id = ?1", [id])?;
    // Its key, in both forms, and every snapshot key wrapped for it: no
    // copy of the database made from now on holds anything it can open.
    c.execute("UPDATE agent_tokens SET key_wrap = NULL, app_seal = NULL WHERE id = ?1", [id])?;
    c.execute("DELETE FROM snapshot_keys WHERE agent_id = ?1", [id])?;
    Ok(())
}

/// A client registered with `POST /oauth/register` (RFC 7591).
#[derive(Debug, Clone)]
pub struct ClientRow {
    pub id: String,
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub created_at: i64,
}

pub fn insert_client(c: &Connection, row: &ClientRow) -> rusqlite::Result<()> {
    let uris = serde_json::to_string(&row.redirect_uris).unwrap_or_else(|_| "[]".into());
    c.execute(
        "INSERT INTO oauth_clients (id, name, redirect_uris, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![row.id, row.name, uris, row.created_at],
    )?;
    Ok(())
}

pub fn client(c: &Connection, id: &str) -> rusqlite::Result<Option<ClientRow>> {
    c.query_row("SELECT id, name, redirect_uris, created_at FROM oauth_clients WHERE id = ?1", [id], |r| {
        let uris: String = r.get(2)?;
        Ok(ClientRow {
            id: r.get(0)?,
            name: r.get(1)?,
            redirect_uris: serde_json::from_str(&uris).unwrap_or_default(),
            created_at: r.get(3)?,
        })
    })
    .optional()
}

/// Clients with no grant are kept this long; then they register again.
pub const UNUSED_CLIENT_MS: i64 = 7 * 24 * 3_600_000;
/// Used codes and spent tokens are kept this long past their expiry.
const SPENT_MS: i64 = 24 * 3_600_000;

/// Drop what has expired: connect and authorization codes, tokens, and
/// clients that never connected an agent.
pub fn prune(c: &Connection, now: i64) -> rusqlite::Result<()> {
    c.execute("DELETE FROM connect_codes WHERE expires_at < ?1", [now - SPENT_MS])?;
    c.execute("DELETE FROM oauth_codes WHERE expires_at < ?1", [now - SPENT_MS])?;
    c.execute("DELETE FROM oauth_tokens WHERE expires_at < ?1", [now - SPENT_MS])?;
    c.execute(
        "DELETE FROM oauth_clients WHERE created_at < ?1 AND id NOT IN \
         (SELECT client_id FROM agent_tokens WHERE client_id IS NOT NULL)",
        [now - UNUSED_CLIENT_MS],
    )?;
    Ok(())
}

/// A connect code minted by the publisher.
#[derive(Debug, Clone)]
pub struct ConnectCodeRow {
    pub id: String,
    pub mailbox_id: i64,
    pub name: String,
    pub code_hash: String,
    pub created_at: i64,
    pub expires_at: i64,
}

pub fn insert_connect_code(c: &Connection, row: &ConnectCodeRow) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO connect_codes (id, mailbox_id, name, code_hash, created_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![row.id, row.mailbox_id, row.name, row.code_hash, row.created_at, row.expires_at],
    )?;
    Ok(())
}

/// How many of a mailbox's connect codes are unused and unexpired.
pub fn live_connect_codes(c: &Connection, mailbox_id: i64, now: i64) -> rusqlite::Result<i64> {
    c.query_row(
        "SELECT COUNT(*) FROM connect_codes WHERE mailbox_id = ?1 AND used_at IS NULL AND expires_at > ?2",
        params![mailbox_id, now],
        |r| r.get(0),
    )
}

/// Use a connect code: its mailbox and name, once, if it is unused and
/// unexpired.
pub fn redeem_connect_code(c: &Connection, code_hash: &str, now: i64) -> rusqlite::Result<Option<(i64, String)>> {
    let found = c
        .query_row(
            "SELECT id, mailbox_id, name FROM connect_codes \
             WHERE code_hash = ?1 AND used_at IS NULL AND expires_at > ?2",
            params![code_hash, now],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)),
        )
        .optional()?;
    let Some((id, mailbox_id, name)) = found else { return Ok(None) };
    c.execute("UPDATE connect_codes SET used_at = ?2 WHERE id = ?1", params![id, now])?;
    Ok(Some((mailbox_id, name)))
}

/// An authorization code, by its hash.
#[derive(Debug, Clone)]
pub struct AuthCodeRow {
    pub grant_id: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub resource: String,
    pub expires_at: i64,
    pub used_at: Option<i64>,
}

pub fn insert_auth_code(c: &Connection, code_hash: &str, row: &AuthCodeRow) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO oauth_codes (code_hash, grant_id, client_id, redirect_uri, code_challenge, resource, \
         expires_at, used_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            code_hash,
            row.grant_id,
            row.client_id,
            row.redirect_uri,
            row.code_challenge,
            row.resource,
            row.expires_at,
            row.used_at
        ],
    )?;
    Ok(())
}

pub fn auth_code(c: &Connection, code_hash: &str) -> rusqlite::Result<Option<AuthCodeRow>> {
    c.query_row(
        "SELECT grant_id, client_id, redirect_uri, code_challenge, resource, expires_at, used_at \
         FROM oauth_codes WHERE code_hash = ?1",
        [code_hash],
        |r| {
            Ok(AuthCodeRow {
                grant_id: r.get(0)?,
                client_id: r.get(1)?,
                redirect_uri: r.get(2)?,
                code_challenge: r.get(3)?,
                resource: r.get(4)?,
                expires_at: r.get(5)?,
                used_at: r.get(6)?,
            })
        },
    )
    .optional()
}

pub fn use_auth_code(c: &Connection, code_hash: &str, now: i64) -> rusqlite::Result<()> {
    c.execute("UPDATE oauth_codes SET used_at = ?2 WHERE code_hash = ?1", params![code_hash, now])?;
    Ok(())
}

pub const TOKEN_ACCESS: &str = "access";
pub const TOKEN_REFRESH: &str = "refresh";

/// An OAuth access or refresh token, by its hash.
#[derive(Debug, Clone)]
pub struct OAuthTokenRow {
    pub kind: String,
    pub grant_id: String,
    pub resource: String,
    pub expires_at: i64,
    pub used_at: Option<i64>,
}

pub fn insert_oauth_token(c: &Connection, token_hash: &str, row: &OAuthTokenRow, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO oauth_tokens (token_hash, kind, grant_id, resource, created_at, expires_at, used_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![token_hash, row.kind, row.grant_id, row.resource, now, row.expires_at, row.used_at],
    )?;
    Ok(())
}

pub fn oauth_token(c: &Connection, token_hash: &str) -> rusqlite::Result<Option<OAuthTokenRow>> {
    c.query_row(
        "SELECT kind, grant_id, resource, expires_at, used_at FROM oauth_tokens WHERE token_hash = ?1",
        [token_hash],
        |r| {
            Ok(OAuthTokenRow {
                kind: r.get(0)?,
                grant_id: r.get(1)?,
                resource: r.get(2)?,
                expires_at: r.get(3)?,
                used_at: r.get(4)?,
            })
        },
    )
    .optional()
}

/// A grant's access tokens end: its refresh made new ones, under a new
/// secret.
pub fn delete_access_tokens(c: &Connection, grant_id: &str) -> rusqlite::Result<usize> {
    c.execute("DELETE FROM oauth_tokens WHERE grant_id = ?1 AND kind = ?2", params![grant_id, TOKEN_ACCESS])
}

/// A refresh token was exchanged: it may not be again.
pub fn use_oauth_token(c: &Connection, token_hash: &str, now: i64) -> rusqlite::Result<()> {
    c.execute("UPDATE oauth_tokens SET used_at = ?2 WHERE token_hash = ?1", params![token_hash, now])?;
    Ok(())
}

/// Reports kept per mailbox until the app pulls them; past this the oldest
/// are dropped, and counted.
pub const MAX_PENDING_REPORTS: i64 = 10_000;
/// Reports the app has not pulled are deleted this long after they came
/// (decision 10).
pub const REPORT_RETENTION_MS: i64 = 30 * 24 * 3_600_000;

/// A report an agent filed with `report_send`: what it says it sent, and
/// what the guide's check made of it then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportRow {
    pub id: i64,
    pub mailbox_id: i64,
    pub agent_id: String,
    pub received_at: i64,
    pub message_id: Option<String>,
    pub recipients: Vec<String>,
    pub subject: String,
    pub sent_at: Option<i64>,
    pub body_markdown: String,
    /// The snapshot version the agent says it checked the draft against.
    pub checked_version: Option<i64>,
    /// The snapshot version the server checked it against when it came.
    pub check_version: Option<i64>,
    /// What that check found (`guide_check`'s messages).
    pub guide_check: Vec<String>,
    /// The Message-ID, recipients, subject, body and check sealed to the app
    /// (the fields above then empty), for a mailbox with a public key.
    pub sealed: Option<Vec<u8>>,
}

/// Store a report, dropping the mailbox's oldest beyond `max_pending`
/// ([`MAX_PENDING_REPORTS`]; counted in `reports_dropped`). Its id.
pub fn insert_report(c: &Connection, r: &ReportRow, max_pending: i64) -> rusqlite::Result<i64> {
    c.execute(
        "INSERT INTO reports (mailbox_id, agent_id, received_at, message_id, recipients, subject, sent_at, \
         body_markdown, checked_version, check_version, guide_check, sealed) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            r.mailbox_id,
            r.agent_id,
            r.received_at,
            r.message_id,
            serde_json::to_string(&r.recipients).unwrap_or_else(|_| "[]".into()),
            r.subject,
            r.sent_at,
            r.body_markdown,
            r.checked_version,
            r.check_version,
            serde_json::to_string(&r.guide_check).unwrap_or_else(|_| "[]".into()),
            r.sealed,
        ],
    )?;
    let id = c.last_insert_rowid();
    let dropped = c.execute(
        "DELETE FROM reports WHERE mailbox_id = ?1 AND id <= \
         (SELECT id FROM reports WHERE mailbox_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)",
        params![r.mailbox_id, max_pending],
    )?;
    if dropped > 0 {
        c.execute(
            "UPDATE mailboxes SET reports_dropped = reports_dropped + ?2 WHERE id = ?1",
            params![r.mailbox_id, i64::try_from(dropped).unwrap_or(i64::MAX)],
        )?;
    }
    Ok(id)
}

/// The mailbox's reports kept in plaintext (filed before it published
/// encrypted), oldest first.
pub fn plain_reports(c: &Connection, mailbox_id: i64) -> rusqlite::Result<Vec<ReportRow>> {
    Ok(reports(c, mailbox_id, 0, i64::MAX)?.into_iter().map(|(r, _, _)| r).filter(|r| r.sealed.is_none()).collect())
}

/// A report's fields replaced by their box sealed to the app.
pub fn seal_report(c: &Connection, id: i64, sealed: &[u8]) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE reports SET message_id = NULL, recipients = '[]', subject = '', body_markdown = '', \
         guide_check = '[]', sealed = ?2 WHERE id = ?1",
        params![id, sealed],
    )?;
    Ok(())
}

/// A report as listed, with its agent's name and kind.
pub type ListedReport = (ReportRow, String, String);

/// A mailbox's reports after `after`, oldest first, at most `limit`.
pub fn reports(c: &Connection, mailbox_id: i64, after: i64, limit: i64) -> rusqlite::Result<Vec<ListedReport>> {
    let mut stmt = c.prepare(
        "SELECT r.id, r.mailbox_id, r.agent_id, r.received_at, r.message_id, r.recipients, r.subject, r.sent_at, \
         r.body_markdown, r.checked_version, r.check_version, r.guide_check, \
         COALESCE(t.name, ''), COALESCE(t.kind, 'token'), r.sealed \
         FROM reports r LEFT JOIN agent_tokens t ON t.id = r.agent_id \
         WHERE r.mailbox_id = ?1 AND r.id > ?2 ORDER BY r.id LIMIT ?3",
    )?;
    stmt.query_map(params![mailbox_id, after, limit], |r| {
        let recipients: String = r.get(5)?;
        let check: String = r.get(11)?;
        Ok((
            ReportRow {
                id: r.get(0)?,
                mailbox_id: r.get(1)?,
                agent_id: r.get(2)?,
                received_at: r.get(3)?,
                message_id: r.get(4)?,
                recipients: serde_json::from_str(&recipients).unwrap_or_default(),
                subject: r.get(6)?,
                sent_at: r.get(7)?,
                body_markdown: r.get(8)?,
                checked_version: r.get(9)?,
                check_version: r.get(10)?,
                guide_check: serde_json::from_str(&check).unwrap_or_default(),
                sealed: r.get(14)?,
            },
            r.get(12)?,
            r.get(13)?,
        ))
    })?
    .collect()
}

/// How many reports wait for the app, and how many were dropped for room.
pub fn report_counts(c: &Connection, mailbox_id: i64) -> rusqlite::Result<(i64, i64)> {
    c.query_row(
        "SELECT (SELECT COUNT(*) FROM reports WHERE mailbox_id = ?1), \
         (SELECT reports_dropped FROM mailboxes WHERE id = ?1)",
        [mailbox_id],
        |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
    )
}

/// The app has the mailbox's reports up to `up_to_id`: delete them. How
/// many went.
pub fn ack_reports(c: &Connection, mailbox_id: i64, up_to_id: i64) -> rusqlite::Result<usize> {
    c.execute("DELETE FROM reports WHERE mailbox_id = ?1 AND id <= ?2", params![mailbox_id, up_to_id])
}

/// Delete reports older than [`REPORT_RETENTION_MS`], pulled or not. How
/// many went.
pub fn sweep_reports(c: &Connection, now: i64) -> rusqlite::Result<usize> {
    c.execute("DELETE FROM reports WHERE received_at < ?1", [now - REPORT_RETENTION_MS])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("openagc-rules-db-{}", crate::tokens::new_id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn opens_migrates_once_and_refuses_a_newer_schema() {
        let dir = scratch();
        let db = Db::open(&dir).unwrap();
        let id = db.run_now(|c| insert_mailbox(c, "a@x.com", "h", 1)).unwrap().unwrap();
        assert_eq!(db.run_now(|c| insert_mailbox(c, "a@x.com", "other", 2)).unwrap(), None);
        drop(db);
        let db = Db::open(&dir).unwrap();
        assert_eq!(db.run_now(|c| mailbox_by_address(c, "a@x.com")).unwrap().unwrap().id, id);
        db.run_now(|c| c.pragma_update(None, "user_version", 99)).unwrap();
        drop(db);
        assert!(matches!(Db::open(&dir), Err(DbError::TooNew(99))));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&dir.join(FILE_NAME)), 0o600);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_database_from_before_oauth_keeps_its_tokens_as_static_ones() {
        let dir = scratch();
        std::fs::create_dir_all(&dir).unwrap();
        {
            let c = Connection::open(dir.join(FILE_NAME)).unwrap();
            c.execute_batch(MIGRATIONS[0]).unwrap();
            c.pragma_update(None, "user_version", 1).unwrap();
            c.execute(
                "INSERT INTO mailboxes (address, publisher_token_hash, created_at) VALUES ('a@x.com', 'h', 1)",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO agent_tokens (id, mailbox_id, name, token_hash, created_at) \
                 VALUES ('0123456789abcdef', 1, 'Old', 'h', 1)",
                [],
            )
            .unwrap();
        }
        let db = Db::open(&dir).unwrap();
        let (row, _) = db.run_now(|c| agent_token(c, "0123456789abcdef")).unwrap().unwrap();
        assert_eq!((row.kind.as_str(), row.client_id), (KIND_TOKEN, None));
        assert_eq!(
            db.run_now(|c| c.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))).unwrap(),
            u32::try_from(MIGRATIONS.len()).unwrap()
        );
        let listed = db.run_now(|c| agent_tokens(c, 1)).unwrap();
        assert_eq!(listed[0].2, None, "never used, as far as the server knows");
        db.run_now(|c| touch_agent(c, "0123456789abcdef", 100_000)).unwrap();
        db.run_now(|c| touch_agent(c, "0123456789abcdef", 130_000)).unwrap();
        assert_eq!(db.run_now(|c| agent_tokens(c, 1)).unwrap()[0].2, Some(100_000), "once a minute at most");
        db.run_now(|c| touch_agent(c, "0123456789abcdef", 160_000)).unwrap();
        assert_eq!(db.run_now(|c| agent_tokens(c, 1)).unwrap()[0].2, Some(160_000));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_database_from_before_secure_delete_is_compacted_once_on_upgrade() {
        let dir = scratch();
        std::fs::create_dir_all(&dir).unwrap();
        let marker = "plain-snapshot-marker-5e1a";
        let bytes = |dir: &Path| -> Vec<u8> {
            let mut all = std::fs::read(dir.join(FILE_NAME)).unwrap();
            all.extend(std::fs::read(dir.join(format!("{FILE_NAME}-wal"))).unwrap_or_default());
            all
        };
        let holds = |b: &[u8]| b.windows(marker.len()).any(|w| w == marker.as_bytes());
        {
            // Schema 5, as a server from before this fix left it: a
            // plaintext snapshot deleted without secure_delete.
            let mut c = Connection::open(dir.join(FILE_NAME)).unwrap();
            c.pragma_update(None, "secure_delete", false).unwrap();
            let tx = c.transaction().unwrap();
            for sql in &MIGRATIONS[..5] {
                tx.execute_batch(sql).unwrap();
            }
            tx.pragma_update(None, "user_version", 5).unwrap();
            tx.commit().unwrap();
            c.execute(
                "INSERT INTO mailboxes (address, publisher_token_hash, created_at) VALUES ('a@x.com', 'h', 1)",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO snapshots (mailbox_id, version, json, published_at, received_at) VALUES (1, 1, ?1, 1, 1)",
                [format!("{{\"fact\":\"{marker}\"}}").repeat(50)],
            )
            .unwrap();
            c.execute("DELETE FROM snapshots", []).unwrap();
        }
        assert!(holds(&bytes(&dir)), "the deleted plaintext lingers in a free page");
        let db = Db::open(&dir).unwrap();
        assert!(!holds(&bytes(&dir)), "compacted on upgrade");
        let made = db.run_now(|c| epoch(c)).unwrap();
        assert_eq!(made.len(), 32);
        drop(db);
        assert_eq!(Db::open(&dir).unwrap().run_now(|c| epoch(c)).unwrap(), made, "made once");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keeps_the_newest_versions_and_forgets_a_mailbox_whole() {
        let dir = scratch();
        let db = Db::open(&dir).unwrap();
        db.run_now(|c| {
            let id = insert_mailbox(c, "a@x.com", "h", 1)?.unwrap();
            for v in 1..=7 {
                insert_snapshot(
                    c,
                    id,
                    &SnapshotRow { version: v, json: "{}".into(), published_at: v, ..Default::default() },
                    v,
                )?;
            }
            assert_eq!(versions(c, id)?, [7, 6, 5, 4, 3]);
            assert_eq!(latest_snapshot(c, id)?.unwrap().version, 7);
            let t = AgentTokenRow {
                id: "0123456789abcdef".into(),
                mailbox_id: id,
                name: "Routine".into(),
                token_hash: "h".into(),
                created_at: 1,
                revoked_at: None,
                kind: KIND_TOKEN.into(),
                client_id: None,
            };
            insert_agent_token(c, &t)?;
            assert!(revoke_agent_token(c, id, &t.id, 5)?);
            assert!(revoke_agent_token(c, id, &t.id, 9)?);
            assert_eq!(agent_token(c, &t.id)?.unwrap().0.revoked_at, Some(5));
            assert!(!revoke_agent_token(c, id + 1, &t.id, 9)?, "another mailbox's token");
            assert!(delete_mailbox(c, id)?);
            assert!(versions(c, id)?.is_empty());
            assert!(agent_token(c, &t.id)?.is_none());
            Ok(())
        })
        .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_are_kept_in_order_dropped_past_the_cap_acked_and_swept() {
        let dir = scratch();
        let db = Db::open(&dir).unwrap();
        db.run_now(|c| {
            let id = insert_mailbox(c, "a@x.com", "h", 1)?.unwrap();
            let report = |at: i64| ReportRow {
                id: 0,
                mailbox_id: id,
                agent_id: "0123456789abcdef".into(),
                received_at: at,
                message_id: Some("m@x".into()),
                recipients: vec!["ann@acme.com".into()],
                subject: "Hi".into(),
                sent_at: None,
                body_markdown: "Hello".into(),
                checked_version: Some(3),
                check_version: Some(3),
                guide_check: vec!["Uses “circle back”, which your rules ban".into()],
                sealed: None,
            };
            let ids: Vec<i64> = (1..=5).map(|at| insert_report(c, &report(at), 3)).collect::<Result<_, _>>()?;
            let listed = reports(c, id, 0, 100)?;
            let kept: Vec<i64> = listed.iter().map(|(r, _, _)| r.id).collect();
            assert_eq!(kept, ids[2..], "the oldest went for room");
            assert_eq!(report_counts(c, id)?, (3, 2));
            assert_eq!(listed[0].0.guide_check.len(), 1);
            assert_eq!(listed[0].1, "", "an agent the server no longer has is named by its id alone");
            assert_eq!(reports(c, id, ids[3], 100)?.len(), 1, "after a cursor");
            assert_eq!(ack_reports(c, id, ids[3])?, 2);
            assert_eq!(report_counts(c, id)?.0, 1);
            assert_eq!(sweep_reports(c, 5 + REPORT_RETENTION_MS)?, 0, "not yet 30 days");
            assert_eq!(sweep_reports(c, 6 + REPORT_RETENTION_MS)?, 1);
            insert_report(c, &report(9), 3)?;
            assert!(delete_mailbox(c, id)?);
            assert_eq!(c.query_row("SELECT COUNT(*) FROM reports", [], |r| r.get::<_, i64>(0))?, 0);
            Ok(())
        })
        .unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
