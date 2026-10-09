//! Bearer tokens (spec §10.6). A token is 32 random bytes, base64url, behind
//! a prefix that says what it is. The server keeps only the SHA-256 of the
//! whole token and compares hashes in constant time.
//!
//! - Publisher tokens, `oagc_pub_<secret>`: one per mailbox, checked
//!   against the mailbox named in the path.
//! - Agent tokens, `oagc_agt_<id>_<secret>`: the id (16 hex digits, not
//!   secret) finds the row and names the token in logs and reports.

use base64::Engine;
use sha2::{Digest, Sha256};

pub const PUBLISHER_PREFIX: &str = "oagc_pub_";
pub const AGENT_PREFIX: &str = "oagc_agt_";

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    // The OS's generator; failing it is not something to serve through.
    getrandom::fill(&mut bytes).expect("the OS random number generator failed");
    bytes
}

fn secret() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes::<32>())
}

/// A new agent token id: 16 hex digits.
pub fn new_id() -> String {
    random_bytes::<8>().iter().map(|b| format!("{b:02x}")).collect()
}

/// A new publisher token.
pub fn publisher_token() -> String {
    format!("{PUBLISHER_PREFIX}{}", secret())
}

/// A new agent token with this id.
pub fn agent_token(id: &str) -> String {
    format!("{AGENT_PREFIX}{id}_{}", secret())
}

/// The id of an agent token, if it has an agent token's shape.
pub fn agent_token_id(token: &str) -> Option<&str> {
    let rest = token.strip_prefix(AGENT_PREFIX)?;
    let (id, secret) = rest.split_once('_')?;
    (id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()) && !secret.is_empty()).then_some(id)
}

/// What the server stores for a token: lower-case hex SHA-256.
pub fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether two hashes are equal, in time that does not depend on where
/// they differ.
pub fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The token of an `Authorization: Bearer <token>` header.
pub fn bearer(headers: &http::HeaderMap) -> Option<&str> {
    let value = headers.get(http::header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.trim().split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_have_their_shape_and_are_unique() {
        let p = publisher_token();
        assert!(p.starts_with(PUBLISHER_PREFIX) && p.len() == PUBLISHER_PREFIX.len() + 43);
        assert_ne!(p, publisher_token());
        let id = new_id();
        let a = agent_token(&id);
        assert_eq!(agent_token_id(&a), Some(id.as_str()));
        assert_eq!(agent_token_id(&p), None);
        assert_eq!(agent_token_id("oagc_agt_short_x"), None);
        assert_eq!(agent_token_id(&format!("{AGENT_PREFIX}{id}_")), None);
    }

    #[test]
    fn hashes_compare_in_full() {
        let h = hash("oagc_pub_x");
        assert_eq!(h.len(), 64);
        assert!(same(&h, &hash("oagc_pub_x")));
        assert!(!same(&h, &hash("oagc_pub_y")));
        assert!(!same(&h, &h[..63]));
    }

    #[test]
    fn bearer_reads_the_scheme_in_any_case() {
        let mut headers = http::HeaderMap::new();
        assert_eq!(bearer(&headers), None);
        headers.insert(http::header::AUTHORIZATION, "bearer  abc ".parse().unwrap());
        assert_eq!(bearer(&headers), Some("abc"));
        headers.insert(http::header::AUTHORIZATION, "Basic abc".parse().unwrap());
        assert_eq!(bearer(&headers), None);
        headers.insert(http::header::AUTHORIZATION, "Bearer ".parse().unwrap());
        assert_eq!(bearer(&headers), None);
    }
}
