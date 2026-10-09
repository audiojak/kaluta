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
];

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
    Ok(())
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

#[derive(Debug, Clone)]
pub struct SnapshotRow {
    pub version: i64,
    pub json: String,
    pub published_at: i64,
}

pub fn latest_snapshot(c: &Connection, mailbox_id: i64) -> rusqlite::Result<Option<SnapshotRow>> {
    c.query_row(
        "SELECT version, json, published_at FROM snapshots WHERE mailbox_id = ?1 ORDER BY version DESC LIMIT 1",
        [mailbox_id],
        |r| Ok(SnapshotRow { version: r.get(0)?, json: r.get(1)?, published_at: r.get(2)? }),
    )
    .optional()
}

/// The versions kept, newest first.
pub fn versions(c: &Connection, mailbox_id: i64) -> rusqlite::Result<Vec<i64>> {
    let mut stmt = c.prepare("SELECT version FROM snapshots WHERE mailbox_id = ?1 ORDER BY version DESC")?;
    stmt.query_map([mailbox_id], |r| r.get(0))?.collect()
}

/// Store a snapshot and drop all but the newest [`KEEP_VERSIONS`].
pub fn insert_snapshot(c: &Connection, mailbox_id: i64, row: &SnapshotRow, now: i64) -> rusqlite::Result<()> {
    c.execute(
        "INSERT INTO snapshots (mailbox_id, version, json, published_at, received_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![mailbox_id, row.version, row.json, row.published_at, now],
    )?;
    c.execute(
        "DELETE FROM snapshots WHERE mailbox_id = ?1 AND version NOT IN \
         (SELECT version FROM snapshots WHERE mailbox_id = ?1 ORDER BY version DESC LIMIT ?2)",
        params![mailbox_id, KEEP_VERSIONS],
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

/// A refresh token was exchanged: it may not be again.
pub fn use_oauth_token(c: &Connection, token_hash: &str, now: i64) -> rusqlite::Result<()> {
    c.execute("UPDATE oauth_tokens SET used_at = ?2 WHERE token_hash = ?1", params![token_hash, now])?;
    Ok(())
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
        assert_eq!(db.run_now(|c| c.pragma_query_value(None, "user_version", |r| r.get::<_, u32>(0))).unwrap(), 3);
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
    fn keeps_the_newest_versions_and_forgets_a_mailbox_whole() {
        let dir = scratch();
        let db = Db::open(&dir).unwrap();
        db.run_now(|c| {
            let id = insert_mailbox(c, "a@x.com", "h", 1)?.unwrap();
            for v in 1..=7 {
                insert_snapshot(c, id, &SnapshotRow { version: v, json: "{}".into(), published_at: v }, v)?;
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
}
