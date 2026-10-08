//! Clean Up's groups and the messages behind them (spec §14.12), one test
//! per view, plus the list headers on the write path.

use std::path::PathBuf;

use mail_domain::{EmailAddress, LabelId, ListHeaders, MessageId, ThreadId};
use mail_store::cleanup::{self, Group, Query, Scope, View};
use mail_store::{Db, IncomingMessage, MailWriter, consistency};

const DAY: i64 = 86_400_000;
/// Thursday 2026-10-08 12:00 UTC.
const NOW: i64 = 1_791_460_800_000;

fn open(name: &str) -> Db {
    let dir: PathBuf = std::env::temp_dir().join(format!("openagc-cleanup-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Db::open(&dir.join("mail.sqlite")).unwrap()
}

fn msg(id: &str, from: (&str, &str), subject: &str, at: i64, labels: &[&str]) -> IncomingMessage {
    IncomingMessage {
        id: MessageId::new(id),
        thread_id: ThreadId::new(format!("t-{id}")),
        from: Some(EmailAddress::new(Some(from.0).filter(|n| !n.is_empty()), from.1)),
        to: vec![EmailAddress::new(None, "me@example.com")],
        subject: subject.into(),
        date: at,
        internal_date: at,
        label_ids: labels.iter().map(|l| LabelId::new(*l)).collect(),
        ..Default::default()
    }
}

fn store(db: &Db, messages: Vec<IncomingMessage>) {
    db.write_blocking(move |tx| {
        let mut w = MailWriter::new(tx);
        for m in &messages {
            w.upsert_message(m)?;
        }
        w.finish()?;
        Ok(())
    })
    .unwrap();
    let problems = db.read_blocking(consistency::check).unwrap();
    assert!(problems.is_empty(), "{problems:?}");
}

fn query(view: View, scope: Scope) -> Query {
    Query { view, scope, now: NOW, utc_offset_secs: 0 }
}

fn groups(db: &Db, view: View, scope: Scope) -> Vec<Group> {
    db.read_blocking(|c| cleanup::groups(c, &query(view, scope), "")).unwrap()
}

fn summary(groups: &[Group]) -> Vec<(String, u64)> {
    groups.iter().map(|g| (g.key.clone(), g.count)).collect()
}

fn keys(k: &[&str]) -> Vec<String> {
    k.iter().map(|s| (*s).to_owned()).collect()
}

fn ids(db: &Db, view: View, scope: Scope, k: &[&str]) -> Vec<String> {
    let k = keys(k);
    let mut ids: Vec<String> = db
        .read_blocking(|c| cleanup::message_ids(c, &query(view, scope), &k))
        .unwrap()
        .into_iter()
        .map(|m| m.0)
        .collect();
    ids.sort();
    ids
}

#[test]
fn list_headers_are_stored_and_survive_a_refresh_without_them() {
    let db = open("list-headers");
    let mut m = msg("m1", ("News", "news@example.com"), "Issue 1", NOW, &["INBOX"]);
    m.list = ListHeaders {
        id: Some("news.example.com".into()),
        name: Some("Example News".into()),
        unsubscribe: Some("<https://example.com/u>".into()),
        unsubscribe_post: Some("List-Unsubscribe=One-Click".into()),
    };
    store(&db, vec![m.clone()]);
    // A header-only refresh from a source without them keeps what is stored.
    store(&db, vec![IncomingMessage { list: ListHeaders::default(), ..m }]);
    let row: (Option<String>, Option<String>, Option<String>, Option<String>) = db
        .read_blocking(|c| {
            Ok(c.query_row(
                "SELECT list_id, list_name, list_unsubscribe, list_unsubscribe_post FROM messages WHERE gmail_id = 'm1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?)
        })
        .unwrap();
    assert_eq!(row.0.as_deref(), Some("news.example.com"));
    assert_eq!(row.1.as_deref(), Some("Example News"));
    assert_eq!(row.2.as_deref(), Some("<https://example.com/u>"));
    assert_eq!(row.3.as_deref(), Some("List-Unsubscribe=One-Click"));
}

#[test]
fn sender_groups_by_address_titled_by_the_most_used_name() {
    let db = open("sender");
    store(
        &db,
        vec![
            msg("a1", ("Amazon", "orders@amazon.example"), "Order 1", NOW, &["INBOX"]),
            msg("a2", ("Amazon.com", "Orders@Amazon.example"), "Order 2", NOW, &["INBOX"]),
            msg("a3", ("Amazon", "orders@amazon.example"), "Order 3", NOW, &["INBOX"]),
            msg("b1", ("Sam", "sam@example.com"), "Hi", NOW, &["INBOX"]),
            msg("b2", ("Sam", "sam@example.com"), "Old", NOW, &[]),
            msg("s1", ("Spam", "spam@example.net"), "Win", NOW, &["SPAM"]),
            msg("d1", ("Me", "me@example.com"), "Draft", NOW, &["DRAFT"]),
        ],
    );
    let inbox = groups(&db, View::Sender, Scope::Inbox);
    assert_eq!(summary(&inbox), [("orders@amazon.example".into(), 3), ("sam@example.com".into(), 1)]);
    assert_eq!(inbox[0].title, "Amazon");
    assert_eq!(inbox[0].aka, ["Amazon.com"]);
    assert_eq!(inbox[0].detail.as_deref(), Some("orders@amazon.example"));

    let all = groups(&db, View::Sender, Scope::AllMail);
    assert_eq!(
        summary(&all),
        [("orders@amazon.example".into(), 3), ("sam@example.com".into(), 2)],
        "no spam, no drafts"
    );

    let filtered = db.read_blocking(|c| cleanup::groups(c, &query(View::Sender, Scope::Inbox), "AMAZON.COM")).unwrap();
    assert_eq!(summary(&filtered), [("orders@amazon.example".into(), 3)], "the filter matches an aka");

    assert_eq!(ids(&db, View::Sender, Scope::Inbox, &["orders@amazon.example"]), ["a1", "a2", "a3"]);
    assert_eq!(ids(&db, View::Sender, Scope::AllMail, &["sam@example.com"]), ["b1", "b2"]);
}

#[test]
fn people_are_senders_the_user_has_written_to() {
    let db = open("people");
    let mut sent = msg("out", ("Me", "me@example.com"), "Re: Hi", NOW, &["SENT"]);
    sent.to = vec![EmailAddress::new(None, "Sam@example.com")];
    store(
        &db,
        vec![
            sent,
            msg("p1", ("Sam", "sam@example.com"), "Hi", NOW, &["INBOX"]),
            msg("n1", ("Shop", "deals@shop.example"), "Sale", NOW, &["INBOX"]),
        ],
    );
    assert_eq!(summary(&groups(&db, View::People, Scope::Inbox)), [("sam@example.com".into(), 1)]);
    assert_eq!(ids(&db, View::People, Scope::Inbox, &["sam@example.com", "deals@shop.example"]), ["p1"]);
}

#[test]
fn subjects_group_exactly_as_stored() {
    let db = open("subject");
    store(
        &db,
        vec![
            msg("a", ("A", "a@example.com"), "Weekly report", NOW, &["INBOX"]),
            msg("b", ("B", "b@example.com"), "Weekly report", NOW, &["INBOX"]),
            msg("c", ("C", "c@example.com"), "Re: Weekly report", NOW, &["INBOX"]),
            msg("d", ("D", "d@example.com"), "", NOW, &["INBOX"]),
        ],
    );
    let g = groups(&db, View::Subject, Scope::Inbox);
    assert_eq!(summary(&g)[0], ("Weekly report".into(), 2));
    assert_eq!(g.len(), 3);
    assert!(g.iter().any(|g| g.key.is_empty() && g.title == "(no subject)"));
    assert_eq!(ids(&db, View::Subject, Scope::Inbox, &["Re: Weekly report"]), ["c"]);
}

#[test]
fn mailing_lists_group_by_list_id() {
    let db = open("lists");
    let listed = |id: &str, list: &str, name: &str| {
        let mut m = msg(id, ("Digest", "digest@example.org"), "Issue", NOW, &["INBOX"]);
        m.list.id = Some(list.into());
        m.list.name = Some(name.into());
        m
    };
    store(
        &db,
        vec![
            listed("l1", "digest.example.org", "Weekly Digest"),
            listed("l2", "digest.example.org", "Weekly Digest"),
            listed("l3", "digest.example.org", "The Digest"),
            listed("l4", "events.example.org", "Events"),
            msg("x", ("Sam", "sam@example.com"), "Hi", NOW, &["INBOX"]),
        ],
    );
    let g = groups(&db, View::MailingList, Scope::Inbox);
    assert_eq!(summary(&g), [("digest.example.org".into(), 3), ("events.example.org".into(), 1)]);
    assert_eq!((g[0].title.as_str(), g[0].aka.as_slice()), ("Weekly Digest", &["The Digest".to_owned()][..]));
    assert_eq!(g[0].detail.as_deref(), Some("digest.example.org"));
    assert_eq!(ids(&db, View::MailingList, Scope::Inbox, &["events.example.org"]), ["l4"]);
}

#[test]
fn time_buckets_follow_the_calendar_then_months() {
    let db = open("time");
    let today = NOW - 12 * 3_600_000; // 2026-10-08 00:00 UTC, a Thursday
    store(
        &db,
        vec![
            msg("now", ("A", "a@example.com"), "s", NOW, &["INBOX"]),
            msg("future", ("A", "a@example.com"), "s", NOW + DAY, &["INBOX"]),
            msg("yday", ("A", "a@example.com"), "s", today - 1, &["INBOX"]),
            msg("mon", ("A", "a@example.com"), "s", today - 3 * DAY, &["INBOX"]),
            msg("lastwk", ("A", "a@example.com"), "s", today - 5 * DAY, &["INBOX"]),
            msg("oct1", ("A", "a@example.com"), "s", today - 7 * DAY + 3_600_000, &["INBOX"]),
            msg("sep", ("A", "a@example.com"), "s", today - 20 * DAY, &["INBOX"]),
            msg("aug", ("A", "a@example.com"), "s", today - 60 * DAY, &["INBOX"]),
        ],
    );
    let g = groups(&db, View::Time, Scope::Inbox);
    assert_eq!(
        summary(&g),
        [
            ("today".into(), 2),
            ("yesterday".into(), 1),
            ("this_week".into(), 1),
            ("last_week".into(), 2),
            ("2026-09".into(), 1),
            ("2026-08".into(), 1),
        ]
    );
    assert_eq!(g[4].title, "September 2026");
    assert_eq!(ids(&db, View::Time, Scope::Inbox, &["last_week", "2026-08"]), ["aug", "lastwk", "oct1"]);

    // Thirteen hours west of UTC it is still Wednesday the 7th, 23:00:
    // yesterday's message (UTC) is today's, and Monday's is last week's.
    let west = Query { view: View::Time, scope: Scope::Inbox, now: NOW, utc_offset_secs: -13 * 3600 };
    let g = db.read_blocking(|c| cleanup::groups(c, &west, "")).unwrap();
    assert_eq!(summary(&g)[..2], [("today".into(), 3), ("last_week".into(), 3)]);
}

#[test]
fn social_and_promotions_group_by_sender_domain() {
    let db = open("categories");
    store(
        &db,
        vec![
            msg("f1", ("Friendbook", "notify@friendbook.example"), "Poke", NOW, &["INBOX", "CATEGORY_SOCIAL"]),
            msg("f2", ("Friendbook Alerts", "alerts@Friendbook.example"), "Tag", NOW, &["INBOX", "CATEGORY_SOCIAL"]),
            msg("p1", ("Shop", "deals@shop.example"), "Sale", NOW, &["INBOX", "CATEGORY_PROMOTIONS"]),
            msg("x", ("Sam", "sam@example.com"), "Hi", NOW, &["INBOX"]),
        ],
    );
    let social = groups(&db, View::Social, Scope::Inbox);
    assert_eq!(summary(&social), [("friendbook.example".into(), 2)]);
    assert_eq!(social[0].title, "friendbook.example");
    assert_eq!(social[0].aka, ["Friendbook", "Friendbook Alerts"]);
    assert_eq!(summary(&groups(&db, View::Promotions, Scope::Inbox)), [("shop.example".into(), 1)]);
    assert_eq!(ids(&db, View::Social, Scope::Inbox, &["friendbook.example", "shop.example"]), ["f1", "f2"]);
}

#[test]
fn size_buckets_run_from_tiny_to_jumbo() {
    let db = open("size");
    let sized = |id: &str, size: u64| IncomingMessage {
        size_estimate: size,
        ..msg(id, ("A", "a@example.com"), "s", NOW, &["INBOX"])
    };
    store(
        &db,
        vec![
            sized("t", 999),
            sized("s", 1_000),
            sized("m", 50_000),
            sized("l", 999_999),
            sized("xl", 1_000_000),
            sized("j1", 10_000_000),
            sized("j2", 30_000_000),
        ],
    );
    assert_eq!(
        summary(&groups(&db, View::Size, Scope::Inbox)),
        [
            ("tiny".into(), 1),
            ("small".into(), 1),
            ("medium".into(), 1),
            ("large".into(), 1),
            ("extra_large".into(), 1),
            ("jumbo".into(), 2)
        ]
    );
    assert_eq!(ids(&db, View::Size, Scope::Inbox, &["jumbo", "tiny", "nonsense"]), ["j1", "j2", "t"]);
}

#[test]
fn messages_page_newest_first_and_resolve_as_they_are_now() {
    let db = open("messages");
    store(&db, (0..5).map(|i| msg(&format!("m{i}"), ("A", "a@example.com"), "s", NOW - i * DAY, &["INBOX"])).collect());
    let q = query(View::Sender, Scope::Inbox);
    let k = keys(&["a@example.com"]);
    let page = |offset, limit| {
        let k = k.clone();
        db.read_blocking(move |c| cleanup::messages(c, &q, &k, offset, limit)).unwrap()
    };
    let first: Vec<_> = page(0, 2).into_iter().map(|m| m.id.0).collect();
    let second: Vec<_> = page(2, 2).into_iter().map(|m| m.id.0).collect();
    assert_eq!((first, second), (vec!["m0".to_owned(), "m1".into()], vec!["m2".to_owned(), "m3".into()]));
    let row = &page(0, 1)[0];
    assert_eq!(row.thread_id.0, "t-m0");
    assert_eq!(row.from.as_ref().map(|f| f.email.as_str()), Some("a@example.com"));

    // The group grows after it was shown: count and ids see it as it is now.
    store(&db, vec![msg("new", ("A", "a@example.com"), "s", NOW, &["INBOX"])]);
    let n = db.read_blocking(|c| cleanup::count(c, &q, &keys(&["a@example.com"]))).unwrap();
    assert_eq!(n, 6);
    assert_eq!(ids(&db, View::Sender, Scope::Inbox, &["a@example.com"]).len(), 6);
    assert_eq!(db.read_blocking(|c| cleanup::count(c, &q, &[])).unwrap(), 0, "no keys, no messages");
}

#[test]
fn inbox_history_and_baseline() {
    let db = open("progress");
    store(&db, vec![msg("a", ("A", "a@example.com"), "s", NOW, &["INBOX"])]);
    let (first, again, days, set, kept, base) = db
        .write_blocking(|tx| {
            let n = cleanup::inbox_count(tx)?;
            let first = cleanup::record_inbox_day(tx, "2026-10-08", n)?;
            let again = cleanup::record_inbox_day(tx, "2026-10-08", 99)?;
            let days = cleanup::inbox_days(tx, "2026-10-01")?;
            let set = cleanup::set_baseline_once(tx, 1200, NOW)?;
            let kept = !cleanup::set_baseline_once(tx, 5, NOW + 1)?;
            Ok((first, again, days, set, kept, cleanup::baseline(tx)?))
        })
        .unwrap();
    assert!(first && !again, "the first count of a day stands");
    assert_eq!(days, [("2026-10-08".to_owned(), 1)]);
    assert!(set && kept);
    assert_eq!(base, Some((1200, NOW)));
}
