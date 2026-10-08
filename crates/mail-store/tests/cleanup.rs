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
    assert_eq!(social[0].detail.as_deref(), Some("Friendbook, Friendbook Alerts"), "the senders' names");
    assert!(social[0].aka.is_empty(), "senders of a domain are not aka one another");
    assert_eq!(summary(&groups(&db, View::Promotions, Scope::Inbox)), [("shop.example".into(), 1)]);
    assert_eq!(ids(&db, View::Social, Scope::Inbox, &["friendbook.example", "shop.example"]), ["f1", "f2"]);
    // The filter finds a domain by a sender's name too.
    let q = query(View::Social, Scope::Inbox);
    let found = db.read_blocking(move |c| cleanup::groups(c, &q, "alerts")).unwrap();
    assert_eq!(summary(&found), [("friendbook.example".into(), 2)]);
    // Archived mail is out of the Inbox's groups, in All Mail's.
    store(&db, vec![msg("f3", ("Pics", "pics@photos.example"), "New", NOW, &["CATEGORY_SOCIAL"])]);
    assert_eq!(groups(&db, View::Social, Scope::Inbox).len(), 1);
    assert_eq!(groups(&db, View::Social, Scope::AllMail).len(), 2);
}

#[test]
fn a_domain_with_many_senders_names_three() {
    let db = open("domain-senders");
    let names = ["Ann", "Bo", "Cy", "Di", "Ed"];
    store(
        &db,
        names
            .iter()
            .enumerate()
            .flat_map(|(i, name)| {
                // Ann sends most, then Bo, and so on.
                (0..(names.len() - i)).map(move |j| {
                    msg(&format!("{name}{j}"), (name, "news@shop.example"), "s", NOW, &["INBOX", "CATEGORY_PROMOTIONS"])
                })
            })
            .collect(),
    );
    let promotions = groups(&db, View::Promotions, Scope::Inbox);
    assert_eq!(promotions[0].detail.as_deref(), Some("Ann, Bo, Cy and 2 more"));
}

#[test]
fn no_categories_no_groups() {
    // An IMAP-only or agent mailbox: no category labels at all.
    let db = open("no-categories");
    store(&db, vec![msg("x", ("Sam", "sam@example.com"), "Hi", NOW, &["INBOX"])]);
    assert!(groups(&db, View::Social, Scope::AllMail).is_empty());
    assert!(groups(&db, View::Promotions, Scope::AllMail).is_empty());
    assert!(ids(&db, View::Promotions, Scope::AllMail, &["example.com"]).is_empty());
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
    let ranges: Vec<_> = groups(&db, View::Size, Scope::Inbox).into_iter().filter_map(|g| g.detail).collect();
    assert_eq!(
        ranges,
        ["Less than 1 KB", "1 KB to 10 KB", "10 KB to 100 KB", "100 KB to 1 MB", "1 MB to 10 MB", "More than 10 MB"],
        "each bucket's range is its second line"
    );
}

/// An optimistic copy of mail just sent (`local-…`) is not Gmail's yet:
/// an action would leave it alone, so the groups and the count leave it
/// out too, and the numbers shown are the numbers changed.
#[test]
fn local_copies_of_sent_mail_are_neither_counted_nor_acted_on() {
    let db = open("local-copies");
    store(
        &db,
        vec![
            msg("sent1", ("Me", "me@example.com"), "s", NOW - DAY, &["SENT", "INBOX"]),
            msg("local-abc", ("Me", "me@example.com"), "s", NOW, &["SENT", "INBOX"]),
            msg("other", ("A", "a@example.com"), "s", NOW, &["INBOX"]),
        ],
    );
    for scope in [Scope::Inbox, Scope::AllMail] {
        let sender = groups(&db, View::Sender, scope);
        assert_eq!(summary(&sender), [("a@example.com".into(), 1), ("me@example.com".into(), 1)], "{scope:?}");
        let q = query(View::Sender, scope);
        let k = keys(&["me@example.com"]);
        let counted = db.read_blocking(move |c| cleanup::count(c, &q, &k)).unwrap();
        let acted = ids(&db, View::Sender, scope, &["me@example.com"]);
        assert_eq!((counted, acted.as_slice()), (1, ["sent1".to_owned()].as_slice()), "{scope:?}");
        let q = query(View::Sender, scope);
        let k = keys(&["me@example.com"]);
        let shown = db.read_blocking(move |c| cleanup::messages(c, &q, &k, 0, 10)).unwrap();
        assert_eq!(shown.len(), 1, "{scope:?}");
        let sizes: u64 = groups(&db, View::Size, scope).iter().map(|g| g.count).sum();
        assert_eq!(sizes, 2, "{scope:?}");
    }
    // Applying to the group changes exactly what was counted.
    let q = query(View::Sender, Scope::Inbox);
    let applied = db
        .write_blocking(move |tx| cleanup::apply(tx, &q, &keys(&["me@example.com"]), &[], &[LabelId::new("INBOX")]))
        .unwrap();
    assert_eq!(applied.diffs.len(), 1);
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

#[test]
fn progress_counts_today_in_the_users_calendar() {
    let db = open("progress-today");
    // 12:00 UTC is 05:00 at UTC-7: local midnight was 07:00 UTC.
    let offset = -7 * 3600;
    let midnight = NOW - 5 * 3_600_000;
    assert_eq!(cleanup::local_midnight(NOW, offset), midnight);
    assert_eq!(cleanup::day_key(NOW, offset), "2026-10-08");
    assert_eq!(cleanup::day_key(midnight - 1, offset), "2026-10-07");
    let sent = msg("sent", ("Me", "me@example.com"), "s", NOW - 1000, &["SENT"]);
    store(
        &db,
        vec![
            // In the Inbox since before midnight: four, one archived today below.
            msg("old1", ("A", "a@example.com"), "s", midnight - 3 * DAY, &["INBOX"]),
            msg("old2", ("A", "a@example.com"), "s", midnight - 2 * DAY, &["INBOX"]),
            msg("old3", ("B", "b@example.com"), "s", midnight - DAY, &["INBOX"]),
            msg("old4", ("B", "b@example.com"), "s", midnight - 1, &["INBOX"]),
            // Arrived today: two still in the Inbox, one archived, one spam.
            msg("new1", ("C", "c@example.com"), "s", midnight + 1, &["INBOX"]),
            msg("new2", ("C", "c@example.com"), "s", NOW - 60_000, &["INBOX", "UNREAD"]),
            msg("new3", ("C", "c@example.com"), "s", NOW - 50_000, &[]),
            msg("spam", ("D", "d@example.com"), "s", NOW - 40_000, &["SPAM"]),
            sent,
        ],
    );
    let (recorded, again, p) = db
        .write_blocking(move |tx| {
            let recorded = cleanup::record_today(tx, NOW, offset)?;
            let again = cleanup::record_today(tx, NOW + 1000, offset)?;
            Ok((recorded, again, cleanup::progress(tx, NOW, offset)?))
        })
        .unwrap();
    assert!(recorded && !again, "one record a day");
    // Six in the Inbox now, two of them from today: four at midnight.
    assert_eq!((p.at_midnight, p.received_today, p.now), (4, 3, 6));
    assert_eq!(p.removed_today, 1, "the one that arrived today and was archived");
    assert_eq!(p.days, [("2026-10-08".to_owned(), 4)]);
    assert_eq!(p.baseline, None);

    // Archiving two older ones: removed today follows; midnight stands.
    store(
        &db,
        vec![
            msg("old1", ("A", "a@example.com"), "s", midnight - 3 * DAY, &[]),
            msg("old2", ("A", "a@example.com"), "s", midnight - 2 * DAY, &[]),
        ],
    );
    let p = db.read_blocking(move |c| cleanup::progress(c, NOW, offset)).unwrap();
    assert_eq!((p.at_midnight, p.received_today, p.removed_today, p.now), (4, 3, 3, 4));
}

#[test]
fn progress_history_baseline_and_percent() {
    let db = open("progress-history");
    store(
        &db,
        (0..10).map(|i| msg(&format!("m{i}"), ("A", "a@example.com"), "s", NOW - 40 * DAY, &["INBOX"])).collect(),
    );
    let p = db
        .write_blocking(|tx| {
            // 40 days of records: the card shows the last 30, today's included.
            for d in 1..=40 {
                let day = cleanup::day_key(NOW - d * DAY, 0);
                cleanup::record_inbox_day(tx, &day, 100 - d as u64)?;
            }
            cleanup::set_baseline_once(tx, 40, NOW - DAY)?;
            cleanup::progress(tx, NOW, 0)
        })
        .unwrap();
    assert_eq!(p.days.len(), 30);
    assert_eq!(p.days.first().unwrap(), &("2026-09-09".to_owned(), 71));
    assert_eq!(p.days[28], ("2026-10-07".to_owned(), 99));
    assert_eq!(p.days.last().unwrap(), &("2026-10-08".to_owned(), 10), "today, worked out before its record");
    assert_eq!((p.baseline, p.now), (Some(40), 10));
    assert_eq!(p.percent(), 75);

    // The Inbox outgrowing the baseline raises it; it never falls.
    let (rose, fell, base) = db
        .write_blocking(|tx| {
            Ok((cleanup::raise_baseline(tx, 60)?, cleanup::raise_baseline(tx, 5)?, cleanup::baseline(tx)?))
        })
        .unwrap();
    assert!(rose && !fell);
    assert_eq!(base.map(|b| b.0), Some(60));

    let pct = |baseline, now| {
        cleanup::Progress { baseline, at_midnight: 0, received_today: 0, removed_today: 0, now, days: vec![] }.percent()
    };
    assert_eq!(pct(None, 5), 0, "no baseline yet");
    assert_eq!(pct(Some(5), 9), 0, "grown past it: clamped");
    assert_eq!(pct(Some(0), 0), 100);
    assert_eq!(pct(None, 0), 100, "an empty Inbox is Inbox Zero");
    assert_eq!(pct(Some(3), 1), 66);
}

#[test]
fn a_groups_newest_list_headers_and_its_unsubscribe() {
    let db = open("unsubscribe");
    let listed = |id: &str, at: i64, unsubscribe: Option<&str>| {
        let mut m = msg(id, ("Digest", "digest@example.org"), "Issue", at, &["INBOX"]);
        m.list = ListHeaders {
            id: Some("digest.example.org".into()),
            name: Some("Weekly Digest".into()),
            unsubscribe: unsubscribe.map(Into::into),
            unsubscribe_post: unsubscribe.map(|_| "List-Unsubscribe=One-Click".into()),
        };
        m
    };
    store(
        &db,
        vec![
            listed("old", NOW - DAY, Some("<https://old.example.org/u>")),
            listed("new", NOW, Some("<https://digest.example.org/u/1>, <mailto:leave@digest.example.org>")),
            msg("x", ("Sam", "sam@example.com"), "Hi", NOW, &["INBOX"]),
        ],
    );
    let q = query(View::MailingList, Scope::Inbox);
    let k = keys(&["digest.example.org", "nothing.example.org"]);
    let found = db.read_blocking(move |c| cleanup::newest_list_headers(c, &q, &k)).unwrap();
    assert_eq!(found.len(), 1, "a group with no messages is left out");
    let newest = &found[0];
    assert_eq!(newest.key, "digest.example.org");
    assert_eq!(
        newest.unsubscribe.as_deref(),
        Some("<https://digest.example.org/u/1>, <mailto:leave@digest.example.org>"),
        "the newest message's, not an older one's"
    );
    assert_eq!(newest.unsubscribe_post.as_deref(), Some("List-Unsubscribe=One-Click"));
    assert_eq!(newest.list_name.as_deref(), Some("Weekly Digest"));
    assert_eq!(newest.from.as_ref().map(|f| f.email.as_str()), Some("digest@example.org"));

    // Sender view: the same message, keyed by address.
    let q = query(View::Sender, Scope::Inbox);
    let found = db.read_blocking(move |c| cleanup::newest_list_headers(c, &q, &keys(&["sam@example.com"]))).unwrap();
    assert_eq!(found[0].unsubscribe, None, "no list headers");

    // Recorded unsubscribes mark the list's group and the sender's.
    assert!(!groups(&db, View::MailingList, Scope::Inbox)[0].unsubscribed);
    db.write_blocking(|tx| {
        cleanup::record_unsubscribed(
            tx,
            &[
                cleanup::unsubscribe_identity(cleanup::UnsubscribeKind::List, "digest.example.org"),
                cleanup::unsubscribe_identity(cleanup::UnsubscribeKind::Sender, "Digest@Example.org"),
            ],
            NOW,
        )
    })
    .unwrap();
    assert!(groups(&db, View::MailingList, Scope::Inbox)[0].unsubscribed);
    let senders = groups(&db, View::Sender, Scope::Inbox);
    assert!(senders.iter().find(|g| g.key == "digest@example.org").unwrap().unsubscribed);
    assert!(!senders.iter().find(|g| g.key == "sam@example.com").unwrap().unsubscribed);
    assert!(groups(&db, View::Subject, Scope::Inbox).iter().all(|g| !g.unsubscribed), "a subject is no list");
    assert_eq!(db.read_blocking(cleanup::unsubscribed).unwrap().len(), 2);
}
