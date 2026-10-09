//! AgentMail's string labels and the store's Gmail-shaped ones (spec §7.9).
//!
//! - `unread` is `UNREAD`. Marking read also adds AgentMail's conventional
//!   `read` label, and marking unread removes it.
//! - A sent message (`sent` label) is in `SENT`. A received one is in
//!   `INBOX` unless it carries the app's own [`ARCHIVED`] label: AgentMail
//!   has no inbox or archive of its own, so archiving adds `archived` and
//!   moving back to the Inbox removes it.
//! - `starred` is `STARRED`, `spam` is `SPAM`.
//! - `trash` is shown as `TRASH` if another client put it there, but the
//!   app never sends it: trashing and deleting stay on this Mac (ADR 0014).
//! - Any other label is a user label whose id is its name.

use mail_domain::{Label, LabelId, LabelKind, system_labels};

/// The label the app puts on mail archived in it.
pub const ARCHIVED: &str = "archived";

/// AgentMail's own labels, and the ones this mapping uses: never user
/// labels.
const RESERVED: &[&str] = &[
    "received",
    "sent",
    "read",
    "unread",
    "trash",
    "spam",
    "blocked",
    "unauthenticated",
    "draft",
    "drafts",
    "opened",
    "delivered",
    "bounced",
    "complained",
    "rejected",
    "scheduled",
    "inbox",
    "starred",
    ARCHIVED,
];

pub fn is_reserved(label: &str) -> bool {
    RESERVED.iter().any(|r| r.eq_ignore_ascii_case(label.trim()))
}

/// A store label id that is Gmail's or the store's own (`INBOX`,
/// `CATEGORY_SOCIAL`), not a user label.
fn is_system_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_uppercase() || c == '_')
}

/// The store's labels for a message carrying AgentMail's `labels`.
pub fn to_local(labels: &[String]) -> Vec<LabelId> {
    let has = |l: &str| labels.iter().any(|x| x.trim().eq_ignore_ascii_case(l));
    let mut out = Vec::new();
    if has("unread") {
        out.push(system_labels::UNREAD);
    }
    if has("sent") {
        out.push(system_labels::SENT);
    } else if !has(ARCHIVED) && !has("spam") && !has("trash") {
        out.push(system_labels::INBOX);
    }
    if has("spam") {
        out.push(system_labels::SPAM);
    }
    if has("trash") {
        out.push(system_labels::TRASH);
    }
    if has("starred") {
        out.push(system_labels::STARRED);
    }
    let mut ids: Vec<LabelId> = out.into_iter().map(LabelId::new).collect();
    ids.extend(labels.iter().map(|l| l.trim()).filter(|l| !l.is_empty() && !is_reserved(l)).map(LabelId::new));
    ids.sort();
    ids.dedup();
    ids
}

/// A stored message fetched again from AgentMail (a resync, a refetch):
/// `stored` is what the store has, `fetched` what [`to_local`] made of
/// AgentMail's labels now. What syncs both ways is AgentMail's: read state,
/// the Inbox (archive), Sent, stars and user labels, so changes made
/// elsewhere and missed by the event list are repaired. What is only this
/// Mac's stays:
/// - Trash and Spam (moving mail there never goes to AgentMail; theirs
///   count too), and mail in either is out of the Inbox;
/// - store labels AgentMail cannot carry: Gmail's and the store's own ids
///   (`DRAFT`, `IMPORTANT`, a category, `@…`).
pub fn merge_refetched(stored: &[LabelId], fetched: &[LabelId]) -> Vec<LabelId> {
    const SYNCED: &[&str] = &[
        system_labels::INBOX,
        system_labels::UNREAD,
        system_labels::SENT,
        system_labels::STARRED,
        system_labels::SPAM,
        system_labels::TRASH,
    ];
    let out_of_inbox = |id: &str| id == system_labels::SPAM || id == system_labels::TRASH;
    let synced = |id: &str| SYNCED.contains(&id) || (!is_system_id(id) && !id.starts_with('@') && !is_reserved(id));
    let mut out: Vec<LabelId> = fetched.to_vec();
    out.extend(stored.iter().filter(|l| out_of_inbox(l.as_str()) || !synced(l.as_str())).cloned());
    if out.iter().any(|l| out_of_inbox(l.as_str())) {
        out.retain(|l| l.as_str() != system_labels::INBOX);
    }
    out.sort();
    out.dedup();
    out
}

/// Whether AgentMail's `label` takes mail out of the Inbox ([`to_local`]):
/// `spam` and `trash`.
pub fn leaves_the_inbox(label: &str) -> bool {
    matches!(label.trim().to_ascii_lowercase().as_str(), "spam" | "trash")
}

/// What one AgentMail label event means for the store: labels added and
/// labels removed, as [`to_local`] reads the same labels. Nothing for
/// AgentMail's own bookkeeping labels.
///
/// `spam` or `trash` added also takes the message out of the Inbox. Taken
/// off, the Inbox is not put back here: whether the message is in it
/// depends on its other labels (archived or sent mail stays out), which the
/// event does not carry, so the provider reads the message
/// ([`leaves_the_inbox`]).
pub fn event_change(label: &str, added: bool) -> Option<(Vec<LabelId>, Vec<LabelId>)> {
    let label = label.trim();
    let one = |id: &str| vec![LabelId::new(id)];
    let (local, inverted) = match label.to_ascii_lowercase().as_str() {
        "unread" => (one(system_labels::UNREAD), false),
        // Archived is the Inbox's absence.
        ARCHIVED => (one(system_labels::INBOX), true),
        "starred" => (one(system_labels::STARRED), false),
        "spam" | "trash" => {
            let id = if label.eq_ignore_ascii_case("spam") { system_labels::SPAM } else { system_labels::TRASH };
            return Some(if added { (one(id), one(system_labels::INBOX)) } else { (vec![], one(id)) });
        }
        _ if label.is_empty() || is_reserved(label) => return None,
        _ => (one(label), false),
    };
    Some(if added != inverted { (local, vec![]) } else { (vec![], local) })
}

/// The AgentMail labels to add and to remove for a change made in the
/// app; empty when nothing goes to AgentMail (trash, spam and the store's
/// own labels stay on this Mac). A change that moves mail to Trash or Spam
/// (Clean Up's Trash and Spam, Mark as Junk) leaves the Inbox on this Mac
/// only: archiving it at AgentMail would act there for an action that
/// stays here, as the trash endpoints do (ADR 0014).
pub fn to_server(add: &[LabelId], remove: &[LabelId]) -> (Vec<String>, Vec<String>) {
    let (mut plus, mut minus) = (Vec::<String>::new(), Vec::<String>::new());
    let local_only = add.iter().any(|l| l.as_str() == system_labels::TRASH || l.as_str() == system_labels::SPAM);
    for (ids, adding) in [(add, true), (remove, false)] {
        for id in ids {
            let (on, off) = if adding { (&mut plus, &mut minus) } else { (&mut minus, &mut plus) };
            match id.as_str() {
                system_labels::INBOX if local_only => {}
                system_labels::INBOX => off.push(ARCHIVED.into()),
                system_labels::UNREAD => {
                    on.push("unread".into());
                    off.push("read".into());
                }
                system_labels::STARRED => on.push("starred".into()),
                other if is_system_id(other) || is_reserved(other) => {}
                user => on.push(user.to_owned()),
            }
        }
    }
    // A label both added and removed in one change: leave it alone.
    let both: Vec<String> = plus.iter().filter(|l| minus.contains(l)).cloned().collect();
    plus.retain(|l| !both.contains(l));
    minus.retain(|l| !both.contains(l));
    plus.sort();
    plus.dedup();
    minus.sort();
    minus.dedup();
    (plus, minus)
}

fn system_label(id: &str, visible: bool) -> Label {
    Label { id: LabelId::new(id), name: id.into(), kind: LabelKind::System, color: None, visible }
}

/// The system labels an AgentMail inbox has. User labels have no listing
/// at AgentMail: they appear as messages carry them, or are made on the
/// Mac, and are kept by the store (labels are the store's, see
/// [`crate::AgentMailProvider`]).
pub fn labels() -> Vec<Label> {
    vec![
        system_label(system_labels::INBOX, true),
        system_label(system_labels::SENT, true),
        system_label(system_labels::DRAFT, true),
        system_label(system_labels::SPAM, true),
        system_label(system_labels::TRASH, true),
        system_label(system_labels::STARRED, true),
        system_label(system_labels::UNREAD, false),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<LabelId> {
        v.iter().map(|s| LabelId::new(*s)).collect()
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn received_mail_is_in_the_inbox_until_archived_and_sent_mail_is_sent() {
        assert_eq!(to_local(&strings(&["received", "unread"])), ids(&["INBOX", "UNREAD"]));
        assert_eq!(to_local(&strings(&["received", "archived", "billing"])), ids(&["billing"]));
        assert_eq!(to_local(&strings(&["sent"])), ids(&["SENT"]));
        // No `received` label at all: still received mail.
        assert_eq!(to_local(&strings(&["unread", "starred"])), ids(&["INBOX", "STARRED", "UNREAD"]));
        assert_eq!(to_local(&strings(&["received", "spam"])), ids(&["SPAM"]));
    }

    #[test]
    fn changes_in_the_app_become_labels_at_agentmail_and_trash_stays_local() {
        // Archive.
        assert_eq!(to_server(&[], &ids(&["INBOX"])), (strings(&["archived"]), vec![]));
        // Back to the Inbox.
        assert_eq!(to_server(&ids(&["INBOX"]), &[]), (vec![], strings(&["archived"])));
        // Read, unread.
        assert_eq!(to_server(&[], &ids(&["UNREAD"])), (strings(&["read"]), strings(&["unread"])));
        assert_eq!(to_server(&ids(&["UNREAD"]), &[]), (strings(&["unread"]), strings(&["read"])));
        // Labels and stars by name; trash, spam and categories stay here.
        assert_eq!(
            to_server(&ids(&["billing", "STARRED", "TRASH", "SPAM", "CATEGORY_SOCIAL"]), &ids(&["receipts"])),
            (strings(&["billing", "starred"]), strings(&["receipts"]))
        );
        // Trash and spam leave the Inbox on this Mac only (Clean Up's
        // Trash and Spam, Mark as Junk): nothing is archived at AgentMail,
        // as the trash endpoints send nothing (ADR 0014).
        assert_eq!(to_server(&ids(&["TRASH"]), &ids(&["INBOX", "SENT"])), (vec![], vec![]));
        assert_eq!(to_server(&ids(&["SPAM"]), &ids(&["INBOX"])), (vec![], vec![]));
        assert_eq!(
            to_server(&ids(&["SPAM", "billing"]), &ids(&["INBOX", "UNREAD"])),
            (strings(&["billing", "read"]), strings(&["unread"]))
        );
        // Undoing them may take `archived` off: harmless, and it repairs
        // mail archived there by an older version.
        assert_eq!(to_server(&ids(&["INBOX"]), &ids(&["TRASH"])), (vec![], strings(&["archived"])));
    }

    #[test]
    fn a_refetch_takes_agentmails_labels_and_keeps_what_is_only_this_macs() {
        // Read and archived elsewhere, starred there; a user label taken off
        // there and another put on: AgentMail's.
        assert_eq!(
            merge_refetched(&ids(&["INBOX", "UNREAD", "billing"]), &ids(&["STARRED", "receipts"])),
            ids(&["STARRED", "receipts"])
        );
        // Trashed or marked spam on this Mac: it stays there, out of the
        // Inbox, whatever AgentMail says of the Inbox.
        assert_eq!(merge_refetched(&ids(&["TRASH", "UNREAD"]), &ids(&["INBOX"])), ids(&["TRASH"]));
        assert_eq!(merge_refetched(&ids(&["SPAM"]), &ids(&["INBOX", "UNREAD"])), ids(&["SPAM", "UNREAD"]));
        // Spam at AgentMail counts too.
        assert_eq!(merge_refetched(&ids(&["INBOX"]), &ids(&["SPAM"])), ids(&["SPAM"]));
        // Labels AgentMail cannot carry stay.
        assert_eq!(
            merge_refetched(&ids(&["INBOX", "IMPORTANT", "CATEGORY_SOCIAL", "@archive"]), &ids(&["INBOX"])),
            ids(&["@archive", "CATEGORY_SOCIAL", "IMPORTANT", "INBOX"])
        );
    }

    #[test]
    fn label_events_mean_the_same_as_labels() {
        assert_eq!(event_change("unread", false), Some((vec![], ids(&["UNREAD"]))));
        assert_eq!(event_change("archived", true), Some((vec![], ids(&["INBOX"]))));
        assert_eq!(event_change("archived", false), Some((ids(&["INBOX"]), vec![])));
        assert_eq!(event_change("billing", true), Some((ids(&["billing"]), vec![])));
        assert_eq!(event_change("read", true), None);
        // Spam and Trash leave the Inbox, as `to_local` has it; out of them,
        // the Inbox is the message's own business (archived or sent mail
        // stays out), so the provider asks AgentMail.
        assert_eq!(event_change("spam", true), Some((ids(&["SPAM"]), ids(&["INBOX"]))));
        assert_eq!(event_change("trash", true), Some((ids(&["TRASH"]), ids(&["INBOX"]))));
        assert_eq!(event_change("Spam", false), Some((vec![], ids(&["SPAM"]))));
        assert_eq!(event_change("trash", false), Some((vec![], ids(&["TRASH"]))));
        assert!(leaves_the_inbox("spam") && leaves_the_inbox(" TRASH") && !leaves_the_inbox("starred"));
    }
}
