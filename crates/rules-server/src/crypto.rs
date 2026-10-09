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
//! Nothing unwrapped is written, and the keys and plaintext held here are
//! wiped when dropped.

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

/// The agent's key, opened with its credential's secret, or made now when
/// it has none and the mailbox has the app's public key. A key the secret
/// no longer opens (a grant whose tokens were made before grant secrets)
/// is replaced. `None` while the mailbox has never pushed encrypted.
pub(crate) fn agent_key(
    c: &Connection,
    agent_id: &str,
    mailbox_id: i64,
    secret: &CredentialSecret,
) -> rusqlite::Result<Option<SecretKey>> {
    let credential = seal::credential_key(secret.as_str(), agent_id);
    let app_key = db::app_key(c, mailbox_id)?.and_then(|k| seal::app_public_key(&k).ok());
    let (wrapped, sealed) = db::agent_keys(c, agent_id)?;
    if let Some(key) = wrapped.and_then(|w| seal::unwrap_agent_key(&credential, &w, agent_id).ok()) {
        if sealed.is_none()
            && let Some(public) = &app_key
            && let Ok(s) = seal::seal_agent_key_for_app(public, &key, agent_id)
        {
            db::set_app_seal(c, agent_id, &s)?;
        }
        return Ok(Some(key));
    }
    let Some(public) = app_key else { return Ok(None) };
    let key = SecretKey::random();
    let Ok(for_app) = seal::seal_agent_key_for_app(&public, &key, agent_id) else { return Ok(None) };
    db::set_agent_keys(c, agent_id, &seal::wrap_agent_key(&credential, &key, agent_id), Some(&for_app))?;
    tracing::info!(token = %agent_id, "agent key made");
    Ok(Some(key))
}

/// Why an agent cannot read the published guide.
pub(crate) enum NotRead {
    /// Nothing was published to the mailbox.
    Unpublished,
    /// The newest snapshot is encrypted and its key is not wrapped for this
    /// agent yet.
    Unreadable,
}

impl NotRead {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unpublished => "not_published",
            Self::Unreadable => "not_readable",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::Unpublished => "nothing has been published to this mailbox yet",
            Self::Unreadable => {
                "the published guide is encrypted and this agent does not have its key yet; OpenAGC gives it \
                 the key when it next publishes or syncs this mailbox, while it is open"
            }
        }
    }

    pub fn into_error(self) -> ApiError {
        let status = match self {
            Self::Unpublished => StatusCode::NOT_FOUND,
            Self::Unreadable => StatusCode::CONFLICT,
        };
        ApiError::new(status, self.code(), self.message())
    }
}

/// The newest snapshot this agent can read, opened in memory.
pub(crate) async fn snapshot_for(state: &AppState, auth: &AgentAuth) -> Result<Result<Snapshot, NotRead>, ApiError> {
    let (mailbox_id, agent_id, address) = (auth.mailbox_id, auth.token_id.clone(), auth.address.clone());
    let key = auth.key.clone();
    let opened = state
        .db
        .run(move |c| {
            let row = db::readable_snapshot(c, mailbox_id, key.as_ref().map(|_| agent_id.as_str()))?;
            let Some((row, wrap)) = row else {
                let any = db::latest_snapshot(c, mailbox_id)?.is_some();
                return Ok(Err(if any { NotRead::Unreadable } else { NotRead::Unpublished }));
            };
            let json = match (&row.sealed, &row.key_id, wrap, &key) {
                (None, _, _, _) => Ok(Zeroizing::new(row.json)),
                (Some(sealed), Some(key_id), Some(wrap), Some(agent_key)) => {
                    seal::unwrap_snapshot_key(agent_key, &wrap, &agent_id, key_id)
                        .and_then(|k| seal::open_snapshot(&k, key_id, &address, row.version, sealed))
                        .map_err(|e| e.to_string())
                }
                _ => Err("a sealed snapshot without its key".to_owned()),
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
    Snapshot::from_json(&json).map(Ok).map_err(ApiError::internal)
}

/// The newest snapshot, or why the agent cannot read one, as an error.
pub(crate) async fn published(state: &AppState, auth: &AgentAuth) -> Result<Snapshot, ApiError> {
    snapshot_for(state, auth).await?.map_err(NotRead::into_error)
}
