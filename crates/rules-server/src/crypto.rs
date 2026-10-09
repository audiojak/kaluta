//! Encryption at rest on the server's side (spec §10.6, oagc-gmn7.7;
//! the primitives are `rules_crypto`).
//!
//! An agent's credential is the only thing that opens its key: the whole
//! static token, or the secret an OAuth grant's tokens carry after a dot.
//! The server holds the credential only while it answers that agent's
//! request. Then, and only then, it can:
//! - make the agent's key if it has none (and the mailbox has pushed the
//!   app's public key), storing it wrapped under the credential and sealed
//!   to the app, and seal it to the app again after the app's key changed;
//! - unwrap the agent's key, the snapshot key the app wrapped under it,
//!   and the snapshot, all in memory.
//!
//! Nothing unwrapped is written. The keys, credential secrets and snapshot
//! JSON this module holds are wiped when dropped (`zeroize`); copies made
//! on the way by others are not: the HTTP request and its headers, JSON
//! parsing, the MCP layer, the parsed `Snapshot` an answer is rendered
//! from, and whatever the allocator leaves behind. A process memory dump
//! during or soon after a request can hold plaintext.

use axum::http::StatusCode;
use rules_crypto::{self as seal, SecretKey};
use rusqlite::Connection;
use writing_guide::Snapshot;
use zeroize::Zeroizing;

use crate::db;
use crate::{AgentAuth, ApiError, AppState};

/// The secret of an agent's credential, wiped when dropped and never shown.
#[derive(Clone)]
pub(crate) struct CredentialSecret(Zeroizing<String>);

impl CredentialSecret {
    pub fn new(secret: &str) -> Self {
        Self(Zeroizing::new(secret.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for CredentialSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialSecret(***)")
    }
}

/// A new OAuth grant's secret: 32 random bytes, base64url.
pub(crate) fn grant_secret() -> String {
    crate::tokens::secret()
}

/// Whether [`agent_key`] may replace a key the secret does not open.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unopened {
    /// When tokens are issued (the secret is the grant's newest) or a token
    /// minted: a key it does not open is replaced.
    Replace,
    /// At a request: a credential that does not open the key (an access
    /// token from before a refresh rotated the secret) reads nothing and
    /// changes nothing.
    Keep,
}

/// The agent's key, opened with its credential's secret, or made now when
/// it has none and the mailbox has the app's public key. `None` while the
/// mailbox has never pushed encrypted, or when the secret does not open
/// the key and `unopened` keeps it.
pub(crate) fn agent_key(
    c: &Connection,
    agent_id: &str,
    mailbox_id: i64,
    secret: &CredentialSecret,
    unopened: Unopened,
) -> rusqlite::Result<Option<SecretKey>> {
    let credential = seal::credential_key(secret.as_str(), agent_id);
    let app_key = db::app_key(c, mailbox_id)?.and_then(|k| seal::app_public_key(&k).ok());
    let (wrapped, sealed) = db::agent_keys(c, agent_id)?;
    if let Some(key) = wrapped.as_ref().and_then(|w| seal::unwrap_agent_key(&credential, w, agent_id).ok()) {
        if sealed.is_none()
            && let Some(public) = &app_key
            && let Ok(s) = seal::seal_agent_key_for_app(public, &key, agent_id)
        {
            db::set_app_seal(c, agent_id, &s)?;
        }
        return Ok(Some(key));
    }
    if wrapped.is_some() && unopened == Unopened::Keep {
        return Ok(None);
    }
    let Some(public) = app_key else { return Ok(None) };
    let key = SecretKey::random();
    let Ok(for_app) = seal::seal_agent_key_for_app(&public, &key, agent_id) else { return Ok(None) };
    db::set_agent_keys(c, agent_id, &seal::wrap_agent_key(&credential, &key, agent_id), Some(&for_app))?;
    tracing::info!(token = %agent_id, "agent key made");
    Ok(Some(key))
}

/// A grant's refresh rotated its secret: its key, opened with the old
/// secret, is wrapped under the new one (the same key, so the app's wraps
/// of snapshot keys for it stay). False when the old secret opens nothing
/// (no key yet, or tokens from before grant secrets): the new tokens then
/// make a key as a grant's first tokens do.
pub(crate) fn rewrap_agent_key(
    c: &Connection,
    agent_id: &str,
    old: &CredentialSecret,
    new: &CredentialSecret,
) -> rusqlite::Result<bool> {
    let (wrapped, _) = db::agent_keys(c, agent_id)?;
    let Some(wrapped) = wrapped else { return Ok(false) };
    let Ok(key) = seal::unwrap_agent_key(&seal::credential_key(old.as_str(), agent_id), &wrapped, agent_id) else {
        return Ok(false);
    };
    let rewrapped = seal::wrap_agent_key(&seal::credential_key(new.as_str(), agent_id), &key, agent_id);
    db::set_key_wrap(c, agent_id, &rewrapped)?;
    Ok(true)
}

/// Why an agent cannot read the published guide.
pub(crate) enum NotRead {
    /// Nothing was published to the mailbox.
    Unpublished,
    /// The newest snapshot is encrypted and its key is not wrapped for this
    /// agent yet.
    Unreadable,
    /// The newest snapshot was stored in plaintext, and the server now
    /// requires encryption: it serves no plaintext.
    PlaintextRefused,
}

impl NotRead {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unpublished => "not_published",
            Self::Unreadable | Self::PlaintextRefused => "not_readable",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::Unpublished => "nothing has been published to this mailbox yet",
            Self::Unreadable => {
                "the published guide is encrypted and this agent does not have its key yet; Kaluta gives it \
                 the key when it next publishes or syncs this mailbox, while it is open"
            }
            Self::PlaintextRefused => {
                "the published guide was stored unencrypted before this server required encryption, and it serves \
                 nothing unencrypted; Kaluta publishes it again, encrypted, when the mailbox's guide or shared \
                 facts next change (or on Publish Now)"
            }
        }
    }

    pub fn into_error(self) -> ApiError {
        let status = match self {
            Self::Unpublished => StatusCode::NOT_FOUND,
            Self::Unreadable | Self::PlaintextRefused => StatusCode::CONFLICT,
        };
        ApiError::new(status, self.code(), self.message())
    }
}

/// The newest snapshot, opened in memory, or why this agent cannot read
/// it. Never an older version: one the app has not wrapped for this agent
/// is [`NotRead::Unreadable`].
pub(crate) async fn snapshot_for(state: &AppState, auth: &AgentAuth) -> Result<Result<Snapshot, NotRead>, ApiError> {
    let (mailbox_id, agent_id, address) = (auth.mailbox_id, auth.token_id.clone(), auth.address.clone());
    let key = auth.key.clone();
    let require_encryption = state.require_encryption;
    let opened = state
        .db
        .run(move |c| {
            let Some((row, wrap)) = db::newest_snapshot_for(c, mailbox_id, Some(agent_id.as_str()))? else {
                return Ok(Err(NotRead::Unpublished));
            };
            let json = match (&row.sealed, &row.key_id, wrap, &key) {
                (None, _, _, _) if require_encryption => return Ok(Err(NotRead::PlaintextRefused)),
                (None, _, _, _) => Ok(Zeroizing::new(row.json)),
                (Some(_), _, None, _) | (Some(_), _, _, None) => return Ok(Err(NotRead::Unreadable)),
                (Some(sealed), Some(key_id), Some(wrap), Some(agent_key)) => {
                    seal::unwrap_snapshot_key(agent_key, &wrap, &agent_id, key_id)
                        .and_then(|k| {
                            seal::open_snapshot(&k, key_id, &address, row.version, row.schema_version, sealed)
                        })
                        .map_err(|e| e.to_string())
                }
                (Some(_), None, _, _) => Err("a sealed snapshot without its key id".to_owned()),
            };
            Ok(Ok(json))
        })
        .await?;
    let json = match opened {
        Err(not_read) => return Ok(Err(not_read)),
        Ok(Ok(json)) => json,
        // The app's wrap or the box is damaged: the agent cannot read it.
        Ok(Err(e)) => {
            tracing::warn!(token = %auth.token_id, error = %e, "a snapshot did not open");
            return Ok(Err(NotRead::Unreadable));
        }
    };
    Snapshot::from_json(&json).map(Ok).map_err(|e| {
        // serde's message can quote what the snapshot says: only its kind.
        tracing::error!(token = %auth.token_id, error = snapshot_error_kind(&e), "a published snapshot did not read");
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "the server failed; see its log")
    })
}

/// Why a snapshot did not read, in words that quote nothing from it.
pub(crate) fn snapshot_error_kind(e: &writing_guide::SnapshotError) -> String {
    use writing_guide::SnapshotError as E;
    match e {
        E::Malformed(e) => format!("malformed JSON ({:?}, line {} column {})", e.classify(), e.line(), e.column()),
        E::UnsupportedSchema(v) => format!("unsupported schema_version {v}"),
        E::MissingSchema => "no schema_version".to_owned(),
        E::PlainAddresses => "plain addresses".to_owned(),
    }
}

/// The newest snapshot, or why the agent cannot read one, as an error.
pub(crate) async fn published(state: &AppState, auth: &AgentAuth) -> Result<Snapshot, ApiError> {
    snapshot_for(state, auth).await?.map_err(NotRead::into_error)
}
