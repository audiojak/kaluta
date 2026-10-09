//! Encryption at rest on a rules server (spec §10.6, oagc-gmn7.7).
//!
//! The server stores a mailbox's snapshots, and the reports its agents
//! file, so that its database alone reads as nothing:
//!
//! - **Snapshots.** Each push is sealed (XChaCha20-Poly1305) with a fresh
//!   random *snapshot key* under a random key id; the mailbox's address,
//!   the version, the key id and the snapshot's `schema_version` are bound
//!   in as associated data.
//! - **Agent keys.** Each agent has a random 256-bit *agent key*, made by
//!   the server while it holds the agent's credential and kept only
//!   wrapped twice: under a key derived from the credential (HKDF-SHA256
//!   from a static token, or from the secret an OAuth grant's tokens
//!   carry), and sealed to the app's public key. The server never stores
//!   the credential, the agent key or the snapshot key in the clear.
//! - **Snapshot key wraps.** The app opens each live agent's key with its
//!   private key and wraps each push's snapshot key under it. A request
//!   unwraps credential → agent key → snapshot key → snapshot, in memory.
//! - **Reports** are sealed by the server to the app's X25519 public key
//!   (an ephemeral key, HKDF-SHA256, XChaCha20-Poly1305), so the server
//!   cannot read a stored report afterwards.
//!
//! This protects data at rest (a leaked database or backup), not from an
//! operator who changes the server's code: the server sees plaintext while
//! it answers a request.
//!
//! Every byte string here is base64 (standard alphabet) on the wire.
//! Sealed boxes are `nonce (24) ‖ ciphertext`; boxes sealed to the app are
//! `ephemeral public key (32) ‖ nonce (24) ‖ ciphertext`.

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

/// The envelope's version: 2 binds the snapshot's `schema_version` to the
/// box (1, from before, did not; such boxes still open, with no schema).
pub const ENCRYPTION_VERSION: u32 = 2;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;

/// Why something could not be opened.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SealError {
    #[error("not base64")]
    Base64,
    #[error("too short to be sealed")]
    Short,
    #[error("the key does not open it, or it was changed")]
    Open,
    #[error("not a key")]
    Key,
    #[error("not UTF-8")]
    Utf8,
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    // The OS's generator; failing it is not something to carry on through.
    getrandom::fill(&mut b).expect("the OS random number generator failed");
    b
}

/// Standard base64.
pub fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Standard base64, read back.
pub fn unb64(s: &str) -> Result<Vec<u8>, SealError> {
    base64::engine::general_purpose::STANDARD.decode(s.trim()).map_err(|_| SealError::Base64)
}

/// A 256-bit secret key, wiped from memory when dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretKey(Zeroizing<[u8; KEY_LEN]>);

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(***)")
    }
}

impl SecretKey {
    pub fn random() -> Self {
        Self(Zeroizing::new(random()))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SealError> {
        let array: [u8; KEY_LEN] = bytes.try_into().map_err(|_| SealError::Key)?;
        Ok(Self(Zeroizing::new(array)))
    }

    pub fn from_base64(s: &str) -> Result<Self, SealError> {
        Self::from_bytes(&Zeroizing::new(unb64(s)?))
    }

    pub fn to_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(b64(self.0.as_ref()))
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(self.0.as_ref().into())
    }
}

/// A new key id: 16 hex digits, not secret.
pub fn new_key_id() -> String {
    random::<8>().iter().map(|b| format!("{b:02x}")).collect()
}

/// Seal `plaintext` under `key` with `ad` bound to it.
fn seal(key: &SecretKey, plaintext: &[u8], ad: &[u8]) -> Vec<u8> {
    let nonce: [u8; NONCE_LEN] = random();
    let ciphertext = key
        .cipher()
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: plaintext, aad: ad })
        // Only fails for a message over 256 GiB.
        .expect("a sealed message fits");
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    out
}

fn open(key: &SecretKey, sealed: &[u8], ad: &[u8]) -> Result<Zeroizing<Vec<u8>>, SealError> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(SealError::Short);
    }
    let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
    key.cipher()
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ciphertext, aad: ad })
        .map(Zeroizing::new)
        .map_err(|_| SealError::Open)
}

/// The prefix of every domain-separation label. It keeps the project's
/// name from before it was Kaluta: it is a protocol constant, bound into
/// every sealed snapshot, report and wrapped key on a server, and changing
/// it would make all of them unreadable.
const LABEL_PREFIX: &str = "openagc-rules/v1/";

/// Associated data: a label and fields, each ended by a zero byte.
fn context(label: &str, fields: &[&str]) -> Vec<u8> {
    let mut out = format!("{LABEL_PREFIX}{label}").into_bytes();
    for f in fields {
        out.push(0);
        out.extend_from_slice(f.as_bytes());
    }
    out
}

fn snapshot_ad(address: &str, version: i64, key_id: &str, schema_version: Option<u32>) -> Vec<u8> {
    let address = address.trim().to_lowercase();
    match schema_version {
        Some(schema) => context("snapshot/2", &[&address, &version.to_string(), key_id, &schema.to_string()]),
        // Envelope 1, from before the schema was bound.
        None => context("snapshot", &[&address, &version.to_string(), key_id]),
    }
}

/// A snapshot's JSON sealed under `key` for `address` at `version`, with
/// its `schema_version` bound to it.
pub fn seal_snapshot(
    key: &SecretKey,
    key_id: &str,
    address: &str,
    version: i64,
    schema_version: u32,
    json: &str,
) -> Vec<u8> {
    seal(key, json.as_bytes(), &snapshot_ad(address, version, key_id, Some(schema_version)))
}

/// A sealed snapshot's JSON, if `key` opens it for this address, version,
/// key id and schema (`None` for an envelope 1 box, which bound none).
pub fn open_snapshot(
    key: &SecretKey,
    key_id: &str,
    address: &str,
    version: i64,
    schema_version: Option<u32>,
    sealed: &[u8],
) -> Result<Zeroizing<String>, SealError> {
    let bytes = open(key, sealed, &snapshot_ad(address, version, key_id, schema_version))?;
    let text = std::str::from_utf8(&bytes).map_err(|_| SealError::Utf8)?;
    Ok(Zeroizing::new(text.to_owned()))
}

/// The key an agent's credential wraps its agent key under: HKDF-SHA256
/// of the credential's secret (a whole static token, or an OAuth grant's
/// secret), for that agent.
pub fn credential_key(secret: &str, agent_id: &str) -> SecretKey {
    // The salt is LABEL_PREFIX + "credential": a protocol constant too.
    let hk = Hkdf::<Sha256>::new(Some(b"openagc-rules/v1/credential"), secret.as_bytes());
    let mut okm = Zeroizing::new([0u8; KEY_LEN]);
    // 32 bytes is far below HKDF-SHA256's limit.
    hk.expand(agent_id.as_bytes(), okm.as_mut()).expect("32 bytes of HKDF output");
    SecretKey(okm)
}

/// An agent key wrapped under its credential's key.
pub fn wrap_agent_key(credential: &SecretKey, agent_key: &SecretKey, agent_id: &str) -> Vec<u8> {
    seal(credential, agent_key.0.as_ref(), &context("agent-key", &[agent_id]))
}

pub fn unwrap_agent_key(credential: &SecretKey, wrapped: &[u8], agent_id: &str) -> Result<SecretKey, SealError> {
    SecretKey::from_bytes(&open(credential, wrapped, &context("agent-key", &[agent_id]))?)
}

/// A push's snapshot key wrapped under an agent's key.
pub fn wrap_snapshot_key(agent_key: &SecretKey, snapshot_key: &SecretKey, agent_id: &str, key_id: &str) -> Vec<u8> {
    seal(agent_key, snapshot_key.0.as_ref(), &context("snapshot-key", &[agent_id, key_id]))
}

pub fn unwrap_snapshot_key(
    agent_key: &SecretKey,
    wrapped: &[u8],
    agent_id: &str,
    key_id: &str,
) -> Result<SecretKey, SealError> {
    SecretKey::from_bytes(&open(agent_key, wrapped, &context("snapshot-key", &[agent_id, key_id]))?)
}

/// The app's X25519 key pair for one mailbox: agents' keys and reports are
/// sealed to its public half; the private half stays in the Keychain.
pub struct AppKey(x25519_dalek::StaticSecret);

impl std::fmt::Debug for AppKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppKey(public {})", self.public_base64())
    }
}

impl AppKey {
    pub fn generate() -> Self {
        Self(x25519_dalek::StaticSecret::from(*Zeroizing::new(random::<KEY_LEN>())))
    }

    pub fn from_base64(s: &str) -> Result<Self, SealError> {
        let bytes = Zeroizing::new(unb64(s)?);
        let array: Zeroizing<[u8; KEY_LEN]> = Zeroizing::new(bytes.as_slice().try_into().map_err(|_| SealError::Key)?);
        Ok(Self(x25519_dalek::StaticSecret::from(*array)))
    }

    /// The private key, for the Keychain.
    pub fn to_base64(&self) -> Zeroizing<String> {
        Zeroizing::new(b64(&self.0.to_bytes()))
    }

    pub fn public_base64(&self) -> String {
        b64(x25519_dalek::PublicKey::from(&self.0).as_bytes())
    }
}

/// A public key as published, checked.
pub fn app_public_key(s: &str) -> Result<[u8; KEY_LEN], SealError> {
    unb64(s)?.as_slice().try_into().map_err(|_| SealError::Key)
}

fn sealed_box_key(
    shared: &[u8; KEY_LEN],
    ephemeral: &[u8; KEY_LEN],
    recipient: &[u8; KEY_LEN],
    ctx: &[u8],
) -> SecretKey {
    let mut salt = [0u8; 2 * KEY_LEN];
    salt[..KEY_LEN].copy_from_slice(ephemeral);
    salt[KEY_LEN..].copy_from_slice(recipient);
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared);
    let mut okm = Zeroizing::new([0u8; KEY_LEN]);
    hk.expand(ctx, okm.as_mut()).expect("32 bytes of HKDF output");
    SecretKey(okm)
}

/// `plaintext` sealed to the app's public key, readable only with its
/// private key. `ctx` names what it is and is bound to it.
pub fn seal_for_app(app_public: &[u8; KEY_LEN], plaintext: &[u8], ctx: &[u8]) -> Result<Vec<u8>, SealError> {
    let ephemeral = x25519_dalek::StaticSecret::from(*Zeroizing::new(random::<KEY_LEN>()));
    let ephemeral_public = x25519_dalek::PublicKey::from(&ephemeral);
    let recipient = x25519_dalek::PublicKey::from(*app_public);
    let shared = ephemeral.diffie_hellman(&recipient);
    if !shared.was_contributory() {
        return Err(SealError::Key);
    }
    let key = sealed_box_key(shared.as_bytes(), ephemeral_public.as_bytes(), app_public, ctx);
    let mut out = ephemeral_public.as_bytes().to_vec();
    out.extend(seal(&key, plaintext, ctx));
    Ok(out)
}

/// What [`seal_for_app`] sealed, opened with the app's private key.
pub fn open_for_app(app: &AppKey, sealed: &[u8], ctx: &[u8]) -> Result<Zeroizing<Vec<u8>>, SealError> {
    if sealed.len() < KEY_LEN + NONCE_LEN + TAG_LEN {
        return Err(SealError::Short);
    }
    let (ephemeral, rest) = sealed.split_at(KEY_LEN);
    let ephemeral: [u8; KEY_LEN] = ephemeral.try_into().map_err(|_| SealError::Short)?;
    let shared = app.0.diffie_hellman(&x25519_dalek::PublicKey::from(ephemeral));
    if !shared.was_contributory() {
        return Err(SealError::Open);
    }
    let public = *x25519_dalek::PublicKey::from(&app.0).as_bytes();
    let key = sealed_box_key(shared.as_bytes(), &ephemeral, &public, ctx);
    open(&key, rest, ctx)
}

/// What an agent key sealed to the app is bound to.
pub fn agent_key_for_app_context(agent_id: &str) -> Vec<u8> {
    context("agent-key-for-app", &[agent_id])
}

/// What a report sealed to the app is bound to.
pub fn report_context(address: &str, agent_id: &str) -> Vec<u8> {
    context("report", &[&address.trim().to_lowercase(), agent_id])
}

/// An agent key sealed to the app.
pub fn seal_agent_key_for_app(
    app_public: &[u8; KEY_LEN],
    agent_key: &SecretKey,
    agent_id: &str,
) -> Result<Vec<u8>, SealError> {
    seal_for_app(app_public, agent_key.0.as_ref(), &agent_key_for_app_context(agent_id))
}

/// An agent key the server sealed to the app, opened.
pub fn open_agent_key_for_app(app: &AppKey, sealed: &[u8], agent_id: &str) -> Result<SecretKey, SealError> {
    SecretKey::from_bytes(&open_for_app(app, sealed, &agent_key_for_app_context(agent_id))?)
}

/// One agent's wrap of a push's snapshot key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyWrap {
    pub agent_id: String,
    /// [`wrap_snapshot_key`], base64.
    pub wrap: String,
}

/// An encrypted push, as the app sends it (`PUT …/snapshot`): the snapshot
/// sealed under a fresh key, that key wrapped for each live agent, and the
/// app's public key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedSnapshot {
    /// [`ENCRYPTION_VERSION`].
    pub encryption: u32,
    pub key_id: String,
    pub version: i64,
    pub published_at: i64,
    /// The sealed snapshot's `schema_version`, bound to the box: the server
    /// refuses one it cannot read before storing anything (0 when absent,
    /// as in envelope 1).
    #[serde(default)]
    pub schema_version: u32,
    /// The mailbox's address.
    pub address: String,
    /// [`seal_snapshot`], base64.
    pub ciphertext: String,
    /// The app's X25519 public key for this mailbox, base64.
    pub app_key: String,
    #[serde(default)]
    pub wraps: Vec<KeyWrap>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_labels_keep_their_bytes_from_before_the_rename() {
        // Sealed data on every server is bound to these; see LABEL_PREFIX.
        assert_eq!(context("snapshot", &["a"]), b"openagc-rules/v1/snapshot\0a");
        // HKDF-SHA256, salt "openagc-rules/v1/credential", computed outside Rust.
        assert_eq!(
            &*credential_key("oagc_agt_pin_secret", "agent-1").to_base64(),
            "ctEvayjhxEj6sGeQVYlwusEfgxZ5/AXJeB9fEDZRRFY="
        );
    }

    #[test]
    fn a_snapshot_opens_only_with_its_key_address_version_and_key_id() {
        let key = SecretKey::random();
        let sealed = seal_snapshot(&key, "k1", "Scout@Agents.example", 3, 1, "{\"marker\":\"zebra\"}");
        assert!(!sealed.windows(5).any(|w| w == b"zebra"));
        let open = |key: &SecretKey, key_id, address, version, schema, sealed: &[u8]| {
            open_snapshot(key, key_id, address, version, schema, sealed)
        };
        assert_eq!(&*open(&key, "k1", "scout@agents.example", 3, Some(1), &sealed).unwrap(), "{\"marker\":\"zebra\"}");
        assert_eq!(open(&SecretKey::random(), "k1", "scout@agents.example", 3, Some(1), &sealed), Err(SealError::Open));
        assert_eq!(open(&key, "k1", "scout@agents.example", 4, Some(1), &sealed), Err(SealError::Open));
        assert_eq!(open(&key, "k2", "scout@agents.example", 3, Some(1), &sealed), Err(SealError::Open));
        assert_eq!(open(&key, "k1", "writer@agents.example", 3, Some(1), &sealed), Err(SealError::Open));
        // The schema is bound: a box declared as another schema does not open.
        assert_eq!(open(&key, "k1", "scout@agents.example", 3, Some(2), &sealed), Err(SealError::Open));
        assert_eq!(open(&key, "k1", "scout@agents.example", 3, None, &sealed), Err(SealError::Open));
        let mut changed = sealed.clone();
        *changed.last_mut().unwrap() ^= 1;
        assert_eq!(open(&key, "k1", "scout@agents.example", 3, Some(1), &changed), Err(SealError::Open));
        assert_eq!(open(&key, "k1", "scout@agents.example", 3, Some(1), &sealed[..10]), Err(SealError::Short));
        // An envelope 1 box, sealed before the schema was bound, still opens.
        let old = seal(&key, b"{}", &snapshot_ad("scout@agents.example", 3, "k1", None));
        assert_eq!(&*open(&key, "k1", "scout@agents.example", 3, None, &old).unwrap(), "{}");
    }

    #[test]
    fn keys_unwrap_along_the_chain_and_only_with_the_right_credential() {
        let token = "oagc_agt_0123456789abcdef_secret";
        let agent = SecretKey::random();
        let snapshot = SecretKey::random();
        let cred = credential_key(token, "0123456789abcdef");
        assert_eq!(cred, credential_key(token, "0123456789abcdef"), "derived, not random");
        assert_ne!(cred, credential_key(token, "fedcba9876543210"), "per agent");
        assert_ne!(cred, credential_key("oagc_agt_0123456789abcdef_other", "0123456789abcdef"));
        let wrapped = wrap_agent_key(&cred, &agent, "0123456789abcdef");
        assert_eq!(unwrap_agent_key(&cred, &wrapped, "0123456789abcdef").unwrap(), agent);
        let wrong = credential_key("oagc_agt_0123456789abcdef_guess", "0123456789abcdef");
        assert_eq!(unwrap_agent_key(&wrong, &wrapped, "0123456789abcdef"), Err(SealError::Open));
        assert_eq!(unwrap_agent_key(&cred, &wrapped, "fedcba9876543210"), Err(SealError::Open));
        let sk = wrap_snapshot_key(&agent, &snapshot, "0123456789abcdef", "k1");
        assert_eq!(unwrap_snapshot_key(&agent, &sk, "0123456789abcdef", "k1").unwrap(), snapshot);
        assert_eq!(unwrap_snapshot_key(&agent, &sk, "0123456789abcdef", "k2"), Err(SealError::Open));
        assert_eq!(format!("{agent:?}"), "SecretKey(***)");
    }

    #[test]
    fn boxes_sealed_to_the_app_open_only_with_its_private_key_and_context() {
        let app = AppKey::generate();
        let public = app_public_key(&app.public_base64()).unwrap();
        let ctx = report_context("scout@agents.example", "0123456789abcdef");
        let sealed = seal_for_app(&public, b"the body: zebra", &ctx).unwrap();
        assert_ne!(sealed, seal_for_app(&public, b"the body: zebra", &ctx).unwrap(), "a fresh key each time");
        assert!(!sealed.windows(5).any(|w| w == b"zebra"));
        assert_eq!(&**open_for_app(&app, &sealed, &ctx).unwrap(), b"the body: zebra");
        assert_eq!(open_for_app(&AppKey::generate(), &sealed, &ctx), Err(SealError::Open));
        let other = report_context("scout@agents.example", "fedcba9876543210");
        assert_eq!(open_for_app(&app, &sealed, &other), Err(SealError::Open));
        // The private key round-trips through the Keychain's text.
        let again = AppKey::from_base64(&app.to_base64()).unwrap();
        assert_eq!(again.public_base64(), app.public_base64());
        assert_eq!(&**open_for_app(&again, &sealed, &ctx).unwrap(), b"the body: zebra");
        // An agent key, sealed to the app.
        let agent = SecretKey::random();
        let sealed = seal_agent_key_for_app(&public, &agent, "a1").unwrap();
        assert_eq!(open_agent_key_for_app(&app, &sealed, "a1").unwrap(), agent);
        assert!(open_agent_key_for_app(&app, &sealed, "a2").is_err());
        // A low-order public key is refused rather than sealed to.
        assert_eq!(seal_for_app(&[0u8; 32], b"x", &ctx), Err(SealError::Key));
    }
}
