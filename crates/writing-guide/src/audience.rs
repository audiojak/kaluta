//! Audience groups (spec §14.9) and who belongs to them. In the app a
//! group lists addresses and `@domain`s; in a published snapshot (spec
//! §10.6) each member is a salted hash, and a recipient is hashed the same
//! way before matching, so the server never holds the addresses.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A confirmed audience group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudienceGroup {
    /// "Customers".
    pub name: String,
    /// Addresses or `@domain`s; in a hashed set, their [`hash_address`]es.
    #[serde(default)]
    pub members: Vec<String>,
}

/// The confirmed audience groups, their members plain or hashed.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AudienceGroups {
    /// Set when every member is a salted hash (a published snapshot); the
    /// salt the members were hashed with.
    #[serde(default)]
    pub salt: Option<String>,
    #[serde(default)]
    pub groups: Vec<AudienceGroup>,
}

/// Whether `address` belongs to a group with these (plain) members: the
/// address itself, or an `@domain` it is at. Any case.
pub fn is_member(address: &str, members: &[String]) -> bool {
    let a = address.trim().to_lowercase();
    members.iter().any(|m| {
        let m = m.trim().to_lowercase();
        m.strip_prefix('@').map_or(a == m, |d| a.ends_with(&format!("@{d}")))
    })
}

/// An address or `@domain` as published (decision 5 of the rules-server
/// plan): lower-case hex of SHA-256 over the salt, a zero byte, and the
/// trimmed, lower-cased address.
pub fn hash_address(salt: &str, address: &str) -> String {
    let mut h = Sha256::new();
    h.update(salt.as_bytes());
    h.update([0u8]);
    h.update(address.trim().to_lowercase().as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

impl AudienceGroups {
    /// Groups with plain members, as the app holds them.
    pub fn plain(groups: Vec<AudienceGroup>) -> Self {
        Self { salt: None, groups }
    }

    /// The same groups with every member hashed with `salt`, for
    /// publishing. Groups already hashed are returned as they are.
    pub fn hashed(&self, salt: &str) -> Self {
        if self.salt.is_some() {
            return self.clone();
        }
        let groups = self
            .groups
            .iter()
            .map(|g| AudienceGroup {
                name: g.name.clone(),
                members: g.members.iter().map(|m| hash_address(salt, m)).collect(),
            })
            .collect();
        Self { salt: Some(salt.to_owned()), groups }
    }

    /// Whether `address` belongs to `group`.
    pub fn contains(&self, group: &AudienceGroup, address: &str) -> bool {
        match &self.salt {
            None => is_member(address, &group.members),
            Some(salt) => {
                let a = address.trim().to_lowercase();
                let mut keys = vec![hash_address(salt, &a)];
                if let Some((_, domain)) = a.rsplit_once('@') {
                    keys.push(hash_address(salt, &format!("@{domain}")));
                }
                group.members.iter().any(|m| keys.iter().any(|k| m.eq_ignore_ascii_case(k)))
            }
        }
    }

    /// The recipients' audiences, in group order.
    pub fn audiences_of(&self, recipients: &[String]) -> Vec<String> {
        self.groups.iter().filter(|g| recipients.iter().any(|r| self.contains(g, r))).map(|g| g.name.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups() -> AudienceGroups {
        AudienceGroups::plain(vec![
            AudienceGroup { name: "Customers".into(), members: vec!["@Acme.com".into(), "bea@globex.com".into()] },
            AudienceGroup { name: "Investors".into(), members: vec!["vc@fund.com".into()] },
        ])
    }

    #[test]
    fn members_are_addresses_or_domains_in_any_case() {
        assert!(is_member(" Ann@ACME.com ", &["@acme.com".into()]));
        assert!(!is_member("ann@notacme.com", &["@acme.com".into()]));
        assert!(!is_member("ann@sub.acme.com", &["@acme.com".into()]), "a domain is not its subdomains");
        assert!(is_member("bea@globex.com", &["BEA@globex.com".into()]));
    }

    #[test]
    fn the_hash_is_pinned_salted_and_case_blind() {
        // Pinned: the app publishes and the server matches with this.
        // printf 'salt\0ann@acme.com' | shasum -a 256
        assert_eq!(
            hash_address("salt", " Ann@Acme.com "),
            "d4a3e5a9ff66d259519535ab209101bbdd9d7bdec49d1875127b7bc2825c9a21"
        );
        assert_eq!(
            hash_address("salt", "@ACME.com"),
            "02110e50a2daac25c221f38f92c37a907fe1c2212fe2de2693b7ea588b1c0a71"
        );
        assert_ne!(hash_address("salt", "ann@acme.com"), hash_address("pepper", "ann@acme.com"));
    }

    #[test]
    fn hashed_groups_match_as_plain_ones_do() {
        let plain = groups();
        let hashed = plain.hashed("s3cret");
        assert_eq!(hashed.salt.as_deref(), Some("s3cret"));
        assert!(hashed.groups.iter().flat_map(|g| &g.members).all(|m| m.len() == 64 && !m.contains('@')));
        assert_eq!(hashed.hashed("other"), hashed, "hashing twice changes nothing");
        for to in [
            vec!["ann@acme.com".to_owned()],
            vec!["Bea@Globex.com".into(), "vc@fund.com".into()],
            vec!["ann@sub.acme.com".into()],
            vec!["nobody@example.com".into()],
            vec![],
        ] {
            assert_eq!(hashed.audiences_of(&to), plain.audiences_of(&to), "{to:?}");
        }
        assert_eq!(plain.audiences_of(&["vc@fund.com".into(), "ann@acme.com".into()]), ["Customers", "Investors"]);
    }
}
