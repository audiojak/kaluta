//! Bearer tokens (spec §10.6). A token is 32 random bytes, base64url, behind
//! a prefix that says what it is. The server keeps only the SHA-256 of the
//! whole token and compares hashes in constant time.
//!
//! - Publisher tokens, `oagc_pub_<secret>`: one per mailbox, checked
//!   against the mailbox named in the path.
//! - Agent tokens, `oagc_agt_<id>_<secret>`: the id (16 hex digits, not
//!   secret) finds the row and names the token in logs and reports.
//! - OAuth access and refresh tokens, `oagc_oat_<secret>` and
//!   `oagc_ort_<secret>`, and authorization codes (`<secret>` alone):
//!   found by their hash.
//! - Connect codes, typed by a person: ten characters from an alphabet
//!   without look-alikes, grouped `ABCDE-FGHJK`; also kept as hashes.

use base64::Engine;
use sha2::{Digest, Sha256};

pub const PUBLISHER_PREFIX: &str = "oagc_pub_";
pub const AGENT_PREFIX: &str = "oagc_agt_";
pub const ACCESS_PREFIX: &str = "oagc_oat_";
pub const REFRESH_PREFIX: &str = "oagc_ort_";
pub const CLIENT_PREFIX: &str = "oagc_cli_";

/// A connect code's characters: digits and capitals without 0, 1, I, L
/// and O, which are read and typed for one another.
pub const CONNECT_ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
/// A connect code's length without its dash: 31^10, about 2^49.
pub const CONNECT_LEN: usize = 10;

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    // The OS's generator; failing it is not something to serve through.
    getrandom::fill(&mut bytes).expect("the OS random number generator failed");
    bytes
}

/// 32 random bytes, base64url: a secret for a token, code or form.
pub fn secret() -> String {
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

/// A new OAuth access token.
pub fn access_token() -> String {
    format!("{ACCESS_PREFIX}{}", secret())
}

/// A new OAuth refresh token.
pub fn refresh_token() -> String {
    format!("{REFRESH_PREFIX}{}", secret())
}

/// A new OAuth client id (public, not secret).
pub fn client_id() -> String {
    format!("{CLIENT_PREFIX}{}{}", new_id(), new_id())
}

/// A new connect code, as shown: `ABCDE-FGHJK`.
pub fn connect_code() -> String {
    let n = u8::try_from(CONNECT_ALPHABET.len()).unwrap_or(u8::MAX);
    // Bytes at or above the largest multiple of the alphabet's size are
    // drawn again, so every character is equally likely.
    let limit = 256 / u16::from(n) * u16::from(n);
    let mut out = String::with_capacity(CONNECT_LEN + 1);
    while out.len() < CONNECT_LEN + 1 {
        for b in random_bytes::<16>() {
            if u16::from(b) >= limit || out.len() == CONNECT_LEN + 1 {
                continue;
            }
            if out.len() == CONNECT_LEN / 2 {
                out.push('-');
            }
            out.push(char::from(CONNECT_ALPHABET[usize::from(b % n)]));
        }
    }
    out
}

/// A connect code as typed, read as stored: upper case, without spaces
/// and dashes. `None` when it cannot be one.
pub fn normalize_connect_code(typed: &str) -> Option<String> {
    let code: String =
        typed.chars().filter(|c| !c.is_whitespace() && *c != '-').map(|c| c.to_ascii_uppercase()).collect();
    (code.len() == CONNECT_LEN && code.bytes().all(|b| CONNECT_ALPHABET.contains(&b))).then_some(code)
}

/// PKCE (RFC 7636): a code verifier is 43 to 128 unreserved characters.
pub fn valid_verifier(v: &str) -> bool {
    (43..=128).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
}

/// An S256 code challenge: base64url SHA-256, 43 characters.
pub fn valid_challenge(c: &str) -> bool {
    c.len() == 43 && c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Whether `verifier` is the one `challenge` was made from (S256), in
/// constant time.
pub fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    let made = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    valid_verifier(verifier) && same(&made, challenge)
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
    fn connect_codes_are_grouped_unambiguous_and_read_back_as_typed() {
        let code = connect_code();
        assert_eq!(code.len(), 11, "{code}");
        assert_eq!(&code[5..6], "-");
        assert!(code.bytes().filter(|b| *b != b'-').all(|b| CONNECT_ALPHABET.contains(&b)), "{code}");
        assert_ne!(code, connect_code());
        let stored = normalize_connect_code(&code).unwrap();
        assert_eq!(normalize_connect_code(&format!(" {} ", code.to_lowercase().replace('-', " "))), Some(stored));
        for bad in ["", "ABCDE-FGHJ", "ABCDE-FGHJKK", "ABCDE-FGHJ0", "ABCDE-FGHJI", "ABCDE+FGHJK"] {
            assert_eq!(normalize_connect_code(bad), None, "{bad}");
        }
        // Every character turns up: nothing in the alphabet is unreachable.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            seen.extend(connect_code().bytes().filter(|b| *b != b'-'));
        }
        assert_eq!(seen.len(), CONNECT_ALPHABET.len());
    }

    #[test]
    fn pkce_s256_as_in_rfc_7636() {
        // RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(valid_verifier(verifier) && valid_challenge(challenge));
        assert!(pkce_matches(verifier, challenge));
        assert!(!pkce_matches("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXK", challenge));
        assert!(!pkce_matches("short", challenge));
        assert!(!valid_challenge("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-c="));
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
