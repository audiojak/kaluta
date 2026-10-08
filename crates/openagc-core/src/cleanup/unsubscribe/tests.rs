//! Unsubscribe: header and mailto parsing, and the one-click POST against
//! a local wiremock server only (never a real address).

use std::sync::Arc;

use futures::executor::block_on;
use mail_domain::{EmailAddress, LabelId, ListHeaders, MessageId, ThreadId};
use mail_store::{IncomingMessage, MailWriter};
use wiremock::matchers::{body_string, header, method as http_method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::{CoreConfig, CoreEvent, EventListener};

#[test]
fn a_header_lists_its_uris_in_order() {
    assert_eq!(
        uris("<mailto:leave@list.example?subject=unsubscribe>, <https://list.example/u?id=1>"),
        ["mailto:leave@list.example?subject=unsubscribe", "https://list.example/u?id=1"]
    );
    assert_eq!(uris("  <https://a.example/x\r\n /y>"), ["https://a.example/x/y"], "folded inside the brackets");
    assert_eq!(
        uris("(comment) <https://a.example/>,<mailto:b@a.example>"),
        ["https://a.example/", "mailto:b@a.example"]
    );
    assert!(uris("https://no-brackets.example/").is_empty());
    assert!(uris("<unterminated").is_empty());
}

#[test]
fn one_click_needs_the_post_header_and_https() {
    let both = "<mailto:leave@list.example>, <https://list.example/u/1>";
    let post = Some("List-Unsubscribe=One-Click");
    let site = owners(Some("news.list.example"), None);
    let method = |header: &str, post: Option<&str>| method(header, post, &site);
    assert_eq!(method(both, post), Some(Method::OneClick(Url::parse("https://list.example/u/1").unwrap())));
    assert_eq!(
        method(both, Some("  list-unsubscribe=one-click ")).map(|m| matches!(m, Method::OneClick(_))),
        Some(true)
    );
    // Without the POST header (or with another value), the mailto.
    let mailto = Mailto { to: vec!["leave@list.example".into()], ..Mailto::default() };
    assert_eq!(method(both, None), Some(Method::Mailto(mailto.clone())));
    assert_eq!(method(both, Some("List-Unsubscribe=Yes")), Some(Method::Mailto(mailto)));
    // Plain http, or credentials in the address, are never posted to.
    assert_eq!(method("<http://list.example/u>", post), None);
    assert_eq!(method("<https://user:pw@list.example/u>", post), None);
    assert_eq!(method("<https://list.example/u>", None), None, "a web page alone is not offered");
    assert_eq!(method("", post), None);
}

#[test]
fn one_click_only_to_the_lists_own_site() {
    let post = Some("List-Unsubscribe=One-Click");
    // The List-Id's domain or the sender's, subdomains included.
    let shop = owners(Some("offers.shop.example.com"), Some("news@mail.shop.example.com"));
    assert_eq!(shop, ["example.com", "example.com"]);
    assert!(matches!(method("<https://u.example.com/x>", post, &shop), Some(Method::OneClick(_))));
    let sender_only = owners(None, Some("deals@brand.example.co.uk"));
    assert_eq!(sender_only, ["example.co.uk"]);
    assert!(matches!(method("<https://unsub.example.co.uk/x>", post, &sender_only), Some(Method::OneClick(_))));
    // Another site: not posted to; the mailto if there is one, else nothing.
    assert_eq!(method("<https://tracker.example.net/x>", post, &shop), None);
    assert_eq!(method("<https://other.co.uk/x>", post, &sender_only), None, "co.uk is no one's site");
    assert!(matches!(
        method("<https://tracker.example.net/x>, <mailto:leave@shop.example.com>", post, &shop),
        Some(Method::Mailto(_))
    ));
    assert_eq!(registrable("Weekly-Digest.Example.ORG."), "example.org");
    assert_eq!(registrable("127.0.0.1"), "127.0.0.1");
    assert_eq!(registrable("localhost"), "localhost");
}

#[test]
fn mailto_addresses_subject_and_body() {
    let m =
        parse_mailto("mailto:leave+abc@list.example?subject=Unsubscribe%20me&body=Please%0Aremove+me&x-id=7").unwrap();
    assert_eq!(m.to, ["leave+abc@list.example"], "a plus stays a plus");
    assert_eq!(m.subject, "Unsubscribe me");
    assert_eq!(m.body, "Please\nremove+me");
    let m = parse_mailto("MAILTO:a@list.example,b%40list.example?cc=c@list.example&to=d@list.example").unwrap();
    assert_eq!(m.to, ["a@list.example", "b@list.example", "d@list.example"]);
    assert_eq!(m.cc, ["c@list.example"]);
    let m = parse_mailto("mailto:?to=only@list.example&subject=%E2%9C%93").unwrap();
    assert_eq!((m.to.as_slice(), m.subject.as_str()), (&["only@list.example".to_owned()][..], "✓"));
    assert_eq!(parse_mailto("mailto:?subject=no-recipient"), None);
    assert_eq!(parse_mailto("https://list.example/"), None);
    assert_eq!(parse_mailto("mailto:bad%zzaddress@list.example").unwrap().to, ["bad%zzaddress@list.example"]);
}

#[derive(Default)]
struct Quiet;
impl EventListener for Quiet {
    fn on_event(&self, _account: Option<String>, _event: CoreEvent) {}
}

struct Temp(std::path::PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const NOW: i64 = 1_790_000_000_000;

/// An account "acct" holding list mail whose unsubscribe headers are given.
fn account(name: &str, mail: Vec<IncomingMessage>) -> (Temp, Arc<Core>) {
    let dir = std::env::temp_dir().join(format!("openagc-unsubscribe-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let core = Core::new(
        CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
        Arc::new(crate::secrets::MemorySecrets::default()),
        Arc::new(Quiet),
    )
    .unwrap();
    block_on(core.clone().open_account("acct".into())).unwrap();
    let db = block_on(core.store_for("acct")).unwrap();
    db.write_blocking(move |tx| {
        let mut w = MailWriter::new(tx);
        for m in &mail {
            w.upsert_message(m)?;
        }
        w.finish()?;
        Ok(())
    })
    .unwrap();
    (Temp(dir), core)
}

fn list_mail(id: &str, from: &str, list: Option<&str>, unsubscribe: &str, one_click: bool, at: i64) -> IncomingMessage {
    IncomingMessage {
        id: MessageId::new(id),
        thread_id: ThreadId::new(format!("t-{id}")),
        from: Some(EmailAddress::new(Some("Weekly Digest"), from)),
        to: vec![EmailAddress::new(None, "me@example.com")],
        subject: "Issue".into(),
        date: at,
        internal_date: at,
        label_ids: vec![LabelId::new("INBOX")],
        list: ListHeaders {
            id: list.map(Into::into),
            name: list.map(|_| "Weekly Digest".into()),
            unsubscribe: Some(unsubscribe.into()),
            unsubscribe_post: one_click.then(|| ONE_CLICK_BODY.into()),
        },
        ..Default::default()
    }
}

fn groups(core: &Core, view: CleanupView) -> Vec<crate::cleanup::CleanupGroup> {
    block_on(core.cleanup_groups("acct".into(), view, CleanupScope::Inbox, String::new())).unwrap()
}

#[test]
fn one_click_posts_once_to_the_local_server_and_the_group_says_unsubscribed() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(MockServer::start());
    rt.block_on(
        Mock::given(http_method("POST"))
            .and(path("/u/digest"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string(ONE_CLICK_BODY))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server),
    );
    let url = format!("{}/u/digest", server.uri());
    let header = format!("<mailto:leave@digest.example.org>, <{url}>");
    let (_temp, core) = account(
        "post",
        vec![
            // Two messages of one list: one POST.
            // The local server stands in for the list's own site.
            list_mail("d1", "digest@127.0.0.1", Some("digest.example.org"), &header, true, NOW - 1000),
            list_mail("d2", "digest@127.0.0.1", Some("digest.example.org"), &header, true, NOW),
            // A mailto-only list: the composer's, not posted.
            list_mail(
                "e1",
                "events@example.org",
                Some("events.example.org"),
                "<mailto:leave@events.example.org?subject=stop>",
                false,
                NOW,
            ),
        ],
    );
    let keys = vec!["digest.example.org".to_owned(), "events.example.org".to_owned()];
    let targets = block_on(core.cleanup_unsubscribe_targets(
        "acct".into(),
        CleanupView::MailingList,
        CleanupScope::Inbox,
        keys.clone(),
    ))
    .unwrap();
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].name, "Weekly Digest");
    assert_eq!(targets[0].method, CleanupUnsubscribeMethod::OneClick { host: "127.0.0.1".into() });
    assert_eq!(
        targets[1].method,
        CleanupUnsubscribeMethod::Mailto {
            to: vec!["leave@events.example.org".into()],
            cc: vec![],
            subject: "stop".into(),
            body: String::new()
        }
    );

    assert_eq!(targets[0].message_ids, ["d2"], "read from the newest message");
    let results = rt
        .block_on(core.cleanup_unsubscribe("acct".into(), CleanupView::MailingList, CleanupScope::Inbox, targets))
        .unwrap();
    assert_eq!(
        results,
        [CleanupUnsubscribeResult {
            keys: vec!["digest.example.org".into()],
            name: "Weekly Digest".into(),
            host: "127.0.0.1".into(),
            error: None
        }]
    );
    let received = rt.block_on(server.received_requests()).unwrap();
    assert_eq!(received.len(), 1, "one POST, the mailto left to the composer");
    let request = &received[0];
    for private in ["cookie", "authorization", "proxy-authorization"] {
        assert!(!request.headers.contains_key(private), "no {private} header");
    }

    let lists = groups(&core, CleanupView::MailingList);
    let digest = lists.iter().find(|g| g.key == "digest.example.org").unwrap();
    let events = lists.iter().find(|g| g.key == "events.example.org").unwrap();
    assert!(digest.unsubscribed && !events.unsubscribed, "a mailto's sending is the user's: not recorded");
    rt.block_on(server.verify());
}

#[test]
fn a_failure_or_a_redirect_is_reported_in_words_and_not_recorded() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(MockServer::start());
    rt.block_on(
        Mock::given(http_method("POST")).and(path("/broken")).respond_with(ResponseTemplate::new(500)).mount(&server),
    );
    rt.block_on(
        Mock::given(http_method("POST"))
            .and(path("/moved"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "https://landing.example.org/finish"))
            .mount(&server),
    );
    let broken = format!("<{}/broken>", server.uri());
    let moved = format!("<{}/moved>", server.uri());
    let (_temp, core) = account(
        "fail",
        vec![
            list_mail("b1", "broken@127.0.0.1", None, &broken, true, NOW),
            list_mail("m1", "moved@127.0.0.1", None, &moved, true, NOW),
        ],
    );
    // From the Sender view: groups keyed by address, no list id.
    let keys = vec!["broken@127.0.0.1".to_owned(), "moved@127.0.0.1".to_owned()];
    let targets =
        block_on(core.cleanup_unsubscribe_targets("acct".into(), CleanupView::Sender, CleanupScope::Inbox, keys))
            .unwrap();
    let results = rt
        .block_on(core.cleanup_unsubscribe("acct".into(), CleanupView::Sender, CleanupScope::Inbox, targets))
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].error.as_deref(), Some("127.0.0.1 answered 500 Internal Server Error"));
    assert_eq!(
        results[1].error.as_deref(),
        Some("the list wants you to open a page at landing.example.org to finish"),
        "a redirect is not done: it wants a page opened"
    );
    let paths: Vec<String> =
        rt.block_on(server.received_requests()).unwrap().iter().map(|r| r.url.path().to_owned()).collect();
    assert_eq!(paths, ["/broken", "/moved"], "the redirect is not followed");
    let senders = groups(&core, CleanupView::Sender);
    assert!(!senders.iter().any(|g| g.unsubscribed), "neither is recorded");
}

#[test]
fn new_mail_since_the_confirmation_is_never_posted_to() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(MockServer::start());
    rt.block_on(Mock::given(http_method("POST")).respond_with(ResponseTemplate::new(200)).mount(&server));
    let shown = format!("<{}/u/shown>", server.uri());
    let (_temp, core) =
        account("newer", vec![list_mail("d1", "digest@127.0.0.1", Some("digest.example.org"), &shown, true, NOW)]);
    let keys = vec!["digest.example.org".to_owned()];
    let targets =
        block_on(core.cleanup_unsubscribe_targets("acct".into(), CleanupView::MailingList, CleanupScope::Inbox, keys))
            .unwrap();
    assert_eq!(targets[0].method, CleanupUnsubscribeMethod::OneClick { host: "127.0.0.1".into() });

    // While the sheet is open, newer mail of the list arrives with another address.
    let db = block_on(core.store_for("acct")).unwrap();
    let newer = list_mail(
        "d2",
        "digest@127.0.0.1",
        Some("digest.example.org"),
        &format!("<{}/u/unseen>", server.uri()),
        true,
        NOW + 1000,
    );
    db.write_blocking(move |tx| {
        let mut w = MailWriter::new(tx);
        w.upsert_message(&newer)?;
        w.finish()?;
        Ok(())
    })
    .unwrap();
    let results = rt
        .block_on(core.cleanup_unsubscribe("acct".into(), CleanupView::MailingList, CleanupScope::Inbox, targets))
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].error.as_deref(), Some(REVIEW_AGAIN));
    assert!(rt.block_on(server.received_requests()).unwrap().is_empty(), "nothing posted");
    assert!(!groups(&core, CleanupView::MailingList)[0].unsubscribed);

    // A target the caller made up (a host never shown) is refused too.
    let forged = vec![CleanupUnsubscribeTarget {
        keys: vec!["digest.example.org".into()],
        message_ids: vec!["d2".into()],
        name: "Weekly Digest".into(),
        method: CleanupUnsubscribeMethod::OneClick { host: "elsewhere.example".into() },
    }];
    let results = rt
        .block_on(core.cleanup_unsubscribe("acct".into(), CleanupView::MailingList, CleanupScope::Inbox, forged))
        .unwrap();
    assert_eq!(results[0].error.as_deref(), Some(REVIEW_AGAIN));
    assert!(rt.block_on(server.received_requests()).unwrap().is_empty(), "still nothing posted");
}

#[test]
fn views_without_a_list_or_sender_offer_nothing() {
    let (_temp, core) = account(
        "views",
        vec![list_mail(
            "d1",
            "digest@example.org",
            Some("digest.example.org"),
            "<https://list.example.org/u>",
            true,
            NOW,
        )],
    );
    let targets = block_on(core.cleanup_unsubscribe_targets(
        "acct".into(),
        CleanupView::Subject,
        CleanupScope::Inbox,
        vec!["Issue".into()],
    ))
    .unwrap();
    assert!(targets.is_empty(), "a subject names no list");
    let targets = block_on(core.cleanup_unsubscribe_targets(
        "acct".into(),
        CleanupView::Sender,
        CleanupScope::Inbox,
        vec!["digest@example.org".into()],
    ))
    .unwrap();
    assert_eq!(targets[0].method, CleanupUnsubscribeMethod::OneClick { host: "list.example.org".into() });
    assert!(
        block_on(core.cleanup_unsubscribe("acct".into(), CleanupView::Sender, CleanupScope::Inbox, vec![])).is_err()
    );
}

#[test]
fn only_secure_addresses_are_posted_to() {
    let post = |u: &str| block_on(post_one_click(&Url::parse(u).unwrap()));
    assert_eq!(post("http://list.example/u"), Err("list.example is not a secure address".into()));
    assert_eq!(post("https://me:secret@list.example/u"), Err("list.example is not a secure address".into()));
}
