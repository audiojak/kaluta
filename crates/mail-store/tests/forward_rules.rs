//! Forwarding rules in the store (spec §14.15): what the watching reads,
//! and the log that keeps a message from being forwarded twice.

use std::path::PathBuf;

use mail_domain::{EmailAddress, LabelId, MessageId, ThreadId};
use mail_store::forward_rules::{self as store, Evidence, NewProposal, Rule};
use mail_store::{Db, IncomingMessage, MailWriter};

fn open(name: &str) -> Db {
    let dir: PathBuf = std::env::temp_dir().join(format!("kaluta-forward-rules-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Db::open(&dir.join("mail.sqlite")).unwrap()
}

fn mail(id: &str, thread: &str, at: i64, from: &str, to: &[&str], subject: &str, labels: &[&str]) -> IncomingMessage {
    IncomingMessage {
        id: MessageId::new(id),
        thread_id: ThreadId::new(thread),
        from: Some(EmailAddress::new(None, from)),
        to: to.iter().map(|t| EmailAddress::new(None, t)).collect(),
        subject: subject.into(),
        date: at,
        internal_date: at,
        label_ids: labels.iter().map(|l| LabelId::new(*l)).collect(),
        ..Default::default()
    }
}

fn put(db: &Db, messages: Vec<IncomingMessage>) {
    db.write_blocking(move |tx| {
        let mut w = MailWriter::new(tx);
        for m in &messages {
            w.upsert_message(m)?;
        }
        w.finish()
    })
    .unwrap();
}

fn rule(sender: &str, prefix: &str) -> Rule {
    Rule {
        id: 0,
        sender: sender.into(),
        sender_is_domain: false,
        subject_prefix: prefix.into(),
        recipient: "receipts@books.example".into(),
        mode: "auto".into(),
        enabled: true,
        effective_from: 100,
        proposal_id: None,
        created_at: 0,
    }
}

#[test]
fn the_users_forwards_are_found_with_what_they_forwarded() {
    let db = open("forwards");
    put(
        &db,
        vec![
            mail("r1", "t1", 10, "Receipts@Shop.example", &["me@example.com"], "Your receipt #1", &["INBOX"]),
            mail("f1", "t1", 20, "me@example.com", &["Receipts@Books.example"], "Fwd: Your receipt #1", &["SENT"]),
            // To two people: not a pattern to one address.
            mail("r2", "t2", 30, "receipts@shop.example", &["me@example.com"], "Your receipt #2", &["INBOX"]),
            mail("f2", "t2", 40, "me@example.com", &["a@x.example", "b@x.example"], "Fwd: Your receipt #2", &["SENT"]),
            // A reply is not a forward; a forward with nothing received before it has no original.
            mail("r3", "t3", 50, "receipts@shop.example", &["me@example.com"], "Your receipt #3", &["INBOX"]),
            mail("f3", "t3", 60, "me@example.com", &["receipts@shop.example"], "Re: Your receipt #3", &["SENT"]),
            mail("f4", "t4", 70, "me@example.com", &["receipts@books.example"], "FW: something of mine", &["SENT"]),
        ],
    );
    let found = db.read_blocking(|c| store::sent_forwards(c, 0)).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].forward_id, "f1");
    assert_eq!(found[0].recipient, "receipts@books.example", "lower case");
    assert_eq!(
        (found[0].original.message_id.as_str(), found[0].original.from_email.as_str()),
        ("r1", "receipts@shop.example")
    );
    assert_eq!(found[0].original.subject, "Your receipt #1");
    assert!(db.read_blocking(|c| store::sent_forwards(c, 21)).unwrap().is_empty(), "only since then");
}

#[test]
fn received_mail_is_found_by_address_or_domain_and_never_from_spam_trash_or_the_user() {
    let db = open("received");
    put(
        &db,
        vec![
            mail("a", "ta", 10, "receipts@shop.example", &["me@example.com"], "Your receipt #1", &["INBOX"]),
            mail("b", "tb", 20, "Billing@shop.example", &["me@example.com"], "Invoice", &[]),
            mail("c", "tc", 30, "receipts@shop.example", &["me@example.com"], "Your receipt #2", &["SPAM"]),
            mail("d", "td", 40, "receipts@shop.example", &["me@example.com"], "Your receipt #3", &["TRASH"]),
            mail("e", "te", 50, "receipts@shop.example", &["x@example.com"], "Fwd: mine", &["SENT"]),
            mail("f", "tf", 60, "receipts@notshop.example", &["me@example.com"], "Your receipt #4", &["INBOX"]),
            mail("g", "tg", 70, "receipts@evilshop.example", &["me@example.com"], "Your receipt #5", &["INBOX"]),
        ],
    );
    let ids = |sender: &'static str, domain: bool, since: i64| {
        db.read_blocking(move |c| store::received_from(c, sender, domain, since))
            .unwrap()
            .into_iter()
            .map(|m| m.message_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids("receipts@shop.example", false, 0), ["a"]);
    assert_eq!(ids("shop.example", true, 0), ["a", "b"], "the domain exactly: not evilshop.example");
    assert!(ids("receipts@shop.example", false, 11).is_empty());
    assert!(ids("receipts@shop_example", false, 0).is_empty(), "no wildcards");
}

#[test]
fn a_message_is_claimed_once_per_rule_and_an_answer_counts_once() {
    let db = open("log");
    let id = db.write_blocking(|tx| store::save_rule(tx, &rule("receipts@shop.example", "Your receipt"), 1)).unwrap();
    let first = move |tx: &mail_store::Transaction<'_>| {
        store::log_first(tx, id, "m1", "t1", "receipts@books.example", "asked", 5)
    };
    assert!(db.write_blocking(first).unwrap());
    assert!(!db.write_blocking(first).unwrap(), "not twice");
    assert_eq!(db.read_blocking(store::asked_threads).unwrap(), ["t1"]);
    let sent = move |tx: &mail_store::Transaction<'_>| store::log_resolve(tx, id, "m1", "asked", "sent", None, 6);
    assert!(db.write_blocking(sent).unwrap());
    assert!(!db.write_blocking(sent).unwrap(), "an answer counts once");
    assert_eq!(db.read_blocking(move |c| store::rule_sent(c, id)).unwrap(), (1, Some(6)));
    assert_eq!(db.read_blocking(move |c| store::sent_since(c, id, 7)).unwrap(), 0);
    assert_eq!(db.read_blocking(|c| store::log_for_thread(c, "t1")).unwrap()[0].outcome, "sent");
    assert!(db.read_blocking(store::rule_sent_messages).unwrap().contains("m1"));
    assert!(db.read_blocking(move |c| store::logged_messages(c, id)).unwrap().contains("m1"));
    assert!(db.read_blocking(store::asked_threads).unwrap().is_empty());

    // Edited in place; deleted, its log goes with it.
    let mut edited = db.read_blocking(move |c| store::get_rule(c, id)).unwrap().unwrap();
    edited.mode = "ask".into();
    edited.enabled = false;
    let again = edited.clone();
    assert_eq!(db.write_blocking(move |tx| store::save_rule(tx, &again, 9)).unwrap(), id);
    assert_eq!(db.read_blocking(store::rules).unwrap(), vec![edited.clone()]);
    db.write_blocking(move |tx| store::delete_rule(tx, id)).unwrap();
    assert!(db.read_blocking(|c| store::log_for_thread(c, "t1")).unwrap().is_empty());
    db.write_blocking(move |tx| store::restore_rule(tx, &edited, 10)).unwrap();
    assert_eq!(db.read_blocking(store::rules).unwrap()[0].id, id, "back under its id");
}

#[test]
fn a_proposal_keeps_its_evidence_current_and_is_not_raised_again_once_decided() {
    let db = open("proposals");
    let ev = |n: i64| Evidence {
        thread_id: format!("t{n}"),
        message_id: format!("m{n}"),
        subject: "Your receipt".into(),
        at: n,
    };
    let propose = |evidence: Vec<Evidence>| {
        db.write_blocking(move |tx| {
            store::propose(
                tx,
                &NewProposal {
                    statement: "Forward mail from receipts@shop.example",
                    sender: "receipts@shop.example",
                    subject_prefix: "Your receipt",
                    recipient: "receipts@books.example",
                    match_key: "k",
                    evidence,
                },
                1,
            )
        })
        .unwrap()
    };
    let id = propose(vec![ev(1), ev(2), ev(3)]).unwrap();
    assert_eq!(propose(vec![ev(1), ev(2), ev(3), ev(4)]), Some(id));
    assert_eq!(db.read_blocking(store::proposed).unwrap()[0].evidence.len(), 4);
    assert_eq!(db.read_blocking(store::proposed_keys).unwrap(), ["k"]);
    db.write_blocking(move |tx| store::set_proposal_status(tx, id, "rejected", None, 2)).unwrap();
    assert_eq!(propose(vec![ev(5)]), None, "rejected stays rejected");
    db.write_blocking(|tx| store::withdraw_proposed(tx, "k")).unwrap();
    assert!(db.read_blocking(move |c| store::get_proposal(c, id)).unwrap().is_some(), "a decided one is kept");
    db.write_blocking(move |tx| store::set_proposal_status(tx, id, "proposed", None, 3)).unwrap();
    db.write_blocking(|tx| store::withdraw_proposed(tx, "k")).unwrap();
    assert!(db.read_blocking(store::proposed).unwrap().is_empty());
}
