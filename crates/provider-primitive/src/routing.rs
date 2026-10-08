//! Several agents on one Primitive account (spec §7.9, ADR 0015).
//!
//! A Primitive account lists all its mail as one inbox: everything sent to
//! its managed subdomain (any local part) and to its own domains, and
//! everything it sent. Each agent's provider keeps its share:
//!
//! - received mail whose recipient is one of the agent's addresses: the
//!   record's `to_email` (taken as the envelope recipient), else the raw
//!   message's `Delivered-To`, `X-Original-To`, `To` and `Cc`;
//! - sent mail whose `from_header` is one of them;
//! - mail to or from an address no agent of the account has, which goes to
//!   the account's first agent. While the account has several agents it is
//!   labelled [`OTHER_ADDRESSES_LABEL`], so the user can see it came to
//!   another address (the To line says which).
//!
//! Addresses compare without case and without a `+tag` on the local part:
//! `Scout+news@x` is `scout@x`.

use std::sync::Arc;

use mail_domain::{Label, LabelId, LabelKind};

/// The label on mail the first agent keeps for addresses no agent has.
pub const OTHER_ADDRESSES_LABEL: &str = "Local_To_Other_Addresses";

/// Which of a Primitive account's mail is one agent's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Routing {
    /// The agent's addresses: its address, and the managed one when it has
    /// moved to an own domain.
    pub own: Vec<String>,
    /// Every other agent's addresses on the same account.
    pub others: Vec<String>,
    /// This agent is the account's first: it also keeps mail to and from
    /// addresses no agent has.
    pub catch_all: bool,
}

/// Why an agent keeps a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    /// One of its addresses is on it.
    Mine,
    /// No agent's address is on it, and this agent is the first.
    Unclaimed,
}

/// The routing now: read again for each listing page, fetch and label
/// listing, so an agent added or removed is seen without restarting sync.
pub type RoutingSource = Arc<dyn Fn() -> Routing + Send + Sync>;

impl Routing {
    /// The only agent on its account: everything is its own.
    pub fn only(address: &str) -> Self {
        Self { own: vec![address.to_owned()], others: vec![], catch_all: true }
    }

    /// Whether this agent keeps mail carrying `addresses` (the recipients
    /// of received mail, the sender of sent mail), and why; `None` when it
    /// is another agent's.
    pub fn keeps<'a>(&self, addresses: impl IntoIterator<Item = &'a str>) -> Option<Keep> {
        let found: Vec<String> = addresses.into_iter().map(normalize).filter(|a| !a.is_empty()).collect();
        let any_of = |list: &[String]| list.iter().any(|a| found.contains(&normalize(a)));
        if any_of(&self.own) {
            Some(Keep::Mine)
        } else if !self.catch_all || any_of(&self.others) {
            None
        } else {
            Some(Keep::Unclaimed)
        }
    }

    /// Whether unclaimed mail is labelled: only while the account has
    /// other agents (alone, every address is the agent's own).
    pub fn marks(&self) -> bool {
        self.catch_all && !self.others.is_empty()
    }

    /// The marker label, for the label list, when this agent marks mail.
    pub fn label(&self) -> Option<Label> {
        self.marks().then(|| Label {
            id: LabelId::new(OTHER_ADDRESSES_LABEL),
            name: "To Other Addresses".into(),
            kind: LabelKind::User,
            color: None,
            visible: true,
        })
    }
}

/// An address as compared: lowercase, without angle brackets or a `+tag`.
pub fn normalize(address: &str) -> String {
    let a = address.trim().trim_start_matches('<').trim_end_matches('>').trim().to_lowercase();
    match a.rsplit_once('@') {
        Some((local, domain)) => {
            let local = local.split('+').next().unwrap_or(local);
            format!("{local}@{domain}")
        }
        None => a,
    }
}

/// The addresses in a header value (`Ada <ada@x>, bob@y`), or the value
/// itself when it does not parse.
pub(crate) fn addresses_in(value: &str) -> Vec<String> {
    let parsed: Vec<String> =
        mail_mime::parse_headers([("To", value)]).to.into_iter().map(|a| a.email).filter(|e| !e.is_empty()).collect();
    if parsed.is_empty() && value.contains('@') { vec![value.trim().to_owned()] } else { parsed }
}

/// The recipients a raw message names: `Delivered-To` and `X-Original-To`
/// (where the message was delivered), then `To` and `Cc`.
pub(crate) fn raw_recipients(raw: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(raw);
    let head = text.split("\r\n\r\n").next().unwrap_or_default();
    let head = head.split("\n\n").next().unwrap_or_default();
    // Unfold continuation lines, then read the headers named.
    let mut lines: Vec<String> = Vec::new();
    for line in head.lines() {
        match lines.last_mut() {
            Some(last) if line.starts_with([' ', '\t']) => {
                last.push(' ');
                last.push_str(line.trim());
            }
            _ => lines.push(line.to_owned()),
        }
    }
    let mut out = Vec::new();
    for name in ["delivered-to", "x-original-to", "to", "cc"] {
        for line in &lines {
            if let Some((key, value)) = line.split_once(':')
                && key.trim().eq_ignore_ascii_case(name)
            {
                out.extend(addresses_in(value));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> (Routing, Routing) {
        let scout = Routing {
            own: vec!["scout@abc.primitive.email".into()],
            others: vec!["writer@abc.primitive.email".into()],
            catch_all: true,
        };
        let writer = Routing {
            own: vec!["writer@abc.primitive.email".into()],
            others: vec!["scout@abc.primitive.email".into()],
            catch_all: false,
        };
        (scout, writer)
    }

    #[test]
    fn mail_is_its_recipients_and_unclaimed_mail_is_the_first_agents() {
        let (scout, writer) = two();
        assert_eq!(scout.keeps(["Scout@ABC.primitive.email"]), Some(Keep::Mine));
        assert_eq!(writer.keeps(["scout@abc.primitive.email"]), None);
        assert_eq!(writer.keeps(["writer+news@abc.primitive.email"]), Some(Keep::Mine), "a +tag is the agent");
        assert_eq!(scout.keeps(["sales@abc.primitive.email"]), Some(Keep::Unclaimed));
        assert_eq!(writer.keeps(["sales@abc.primitive.email"]), None);
        assert_eq!(scout.keeps([]), Some(Keep::Unclaimed), "nothing to go by: the first agent's");
        // Addressed to both: each keeps it.
        assert_eq!(writer.keeps(["scout@abc.primitive.email", "writer@abc.primitive.email"]), Some(Keep::Mine));
        assert!(scout.marks() && scout.label().is_some());
        assert!(!writer.marks() && writer.label().is_none());
        assert!(!Routing::only("scout@abc.primitive.email").marks(), "alone, nothing is marked");
    }

    #[test]
    fn a_raw_message_names_its_recipients() {
        let raw = b"Delivered-To: writer+x@abc.primitive.email\r\nFrom: a@example.com\r\nTo: Ada <ada@example.com>,\r\n \
                    Bob <bob@example.com>\r\nCc: c@example.com\r\nSubject: To: nobody@x\r\n\r\nTo: body@example.com\r\n";
        assert_eq!(
            raw_recipients(raw),
            vec!["writer+x@abc.primitive.email", "ada@example.com", "bob@example.com", "c@example.com"]
        );
        assert_eq!(normalize("<Writer+X@ABC.primitive.email>"), "writer@abc.primitive.email");
        assert_eq!(addresses_in("\"Scout\" <scout@abc.primitive.email>"), vec!["scout@abc.primitive.email"]);
    }
}
