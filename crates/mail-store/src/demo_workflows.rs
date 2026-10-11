//! The workflow demo (docs/plans/design-review.md): a small, hand-written
//! mailbox on top of the synthetic one, dated from the moment it is made,
//! so each daily workflow has real work in it: conversations waiting on a
//! reply, sent mail nobody has answered, things to read, Important
//! automated mail for the sorting routine, and mail that arrives while the
//! app is open. All content is invented; every address is under
//! `example.com`/`.org`/`.net`.

use mail_domain::{Body, EmailAddress, Label, LabelColor, LabelId, LabelKind, ListHeaders, MessageId, ThreadId};

use crate::db::Db;
use crate::demo::{DemoStats, ME};
use crate::error::StoreResult;
use crate::write::{IncomingAttachment, IncomingMessage, MailWriter};

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// The sorting routine's labels (spec §11.3), as an earlier run left them.
pub const CADENCE_LABELS: &[(&str, &str, &str)] = &[
    ("Label_MI1", "Marked Important/1-Daily", "#fb4c2f"),
    ("Label_MI2", "Marked Important/2-Weekly-Newsletters", "#4a86e8"),
    ("Label_MI3", "Marked Important/3-Weekly-Events", "#16a766"),
    ("Label_MI4", "Marked Important/4-Weekly-Finance", "#ffad47"),
    ("Label_MI5", "Marked Important/5-Monthly-Pitches", "#999999"),
    ("Label_MI6", "Marked Important/6-Weekly-Hiring", "#a479e2"),
];

#[derive(Clone, Copy)]
struct Who(&'static str, &'static str);

impl Who {
    fn address(self) -> EmailAddress {
        EmailAddress::new(Some(self.0), self.1)
    }
    fn first_name(self) -> &'static str {
        self.0.split(' ').next().unwrap_or(self.0)
    }
}

const MORGAN: Who = Who("Morgan Okafor", "morgan.okafor@example.com");
const SAM: Who = Who("Sam Rivera", "sam.rivera@example.com");
const TAYLOR: Who = Who("Taylor Chen", "taylor.chen@example.com");
const JORDAN: Who = Who("Jordan Park", "jordan.park@example.org");
const CASEY: Who = Who("Casey Novak", "casey.novak@example.net");
const AVERY: Who = Who("Avery Kim", "avery.kim@example.com");
const QUINN: Who = Who("Quinn Silva", "quinn.silva@example.org");
const DREW: Who = Who("Drew Haddad", "drew.haddad@example.net");
const REESE: Who = Who("Reese Tanaka", "reese.tanaka@example.org");
const KENDALL: Who = Who("Kendall Lindqvist", "kendall.lindqvist@example.org");
const HAYDEN: Who = Who("Hayden Mendes", "hayden.mendes@example.net");
const EMERSON: Who = Who("Emerson Ibrahim", "emerson.ibrahim@example.com");
const PEYTON: Who = Who("Peyton Kowalski", "peyton.kowalski@example.com");
const LOGAN: Who = Who("Logan Schmidt", "logan.schmidt@example.com");
const ROWAN: Who = Who("Rowan Nguyen", "rowan.nguyen@example.net");
const RILEY: Who = Who("Riley Moreau", "riley.moreau@example.org");

const ALERTS: Who = Who("Status Alerts", "alerts@example.net");
const BILLING: Who = Who("Billing", "billing@example.net");
const CARDS: Who = Who("Card Services", "cards@example.net");
const DIGEST: Who = Who("The Digest", "digest@example.org");
const UPDATES: Who = Who("Product Updates", "updates@example.com");
const EVENTS: Who = Who("Events", "events@example.org");
const CONFERENCE: Who = Who("Summit Team", "summit@example.org");
const LEADS: Who = Who("Pat at Example Leads", "pat@example.net");
const SPONSORS: Who = Who("Sponsorships", "sponsor@example.org");
const CAREERS: Who = Who("Careers", "careers@example.com");

/// One message: minutes before the thread is made, whether the user wrote
/// it, and its text (greeting and sign-off included).
struct Msg {
    ago: i64,
    from_me: bool,
    text: &'static str,
}

const fn them(ago: i64, text: &'static str) -> Msg {
    Msg { ago, from_me: false, text }
}

const fn me(ago: i64, text: &'static str) -> Msg {
    Msg { ago, from_me: true, text }
}

struct Thread {
    key: &'static str,
    subject: &'static str,
    other: Who,
    inbox: bool,
    /// The last message from the other side is unread.
    unread: bool,
    important: bool,
    category: &'static str,
    labels: &'static [&'static str],
    attachment: Option<&'static str>,
    msgs: &'static [Msg],
}

const PERSONAL: &str = "CATEGORY_PERSONAL";
const UPDATES_TAB: &str = "CATEGORY_UPDATES";
const PROMOTIONS_TAB: &str = "CATEGORY_PROMOTIONS";
const FORUMS_TAB: &str = "CATEGORY_FORUMS";

/// A conversation, inbox defaults: read, not Important.
const fn person(key: &'static str, subject: &'static str, other: Who, msgs: &'static [Msg]) -> Thread {
    Thread {
        key,
        subject,
        other,
        inbox: true,
        unread: false,
        important: false,
        category: PERSONAL,
        labels: &[],
        attachment: None,
        msgs,
    }
}

/// Automated mail, Important and unread in the Inbox: what the sorting
/// routine looks for (`is:important newer_than:1d`).
const fn service(
    key: &'static str,
    subject: &'static str,
    other: Who,
    category: &'static str,
    msgs: &'static [Msg],
) -> Thread {
    Thread {
        key,
        subject,
        other,
        inbox: true,
        unread: true,
        important: true,
        category,
        labels: &[],
        attachment: None,
        msgs,
    }
}

const fn unread(mut t: Thread) -> Thread {
    t.unread = true;
    t
}

const fn important(mut t: Thread) -> Thread {
    t.important = true;
    t
}

const fn archived(mut t: Thread) -> Thread {
    t.inbox = false;
    t
}

const fn labeled(mut t: Thread, labels: &'static [&'static str]) -> Thread {
    t.labels = labels;
    t
}

const fn attached(mut t: Thread, file: &'static str) -> Thread {
    t.attachment = Some(file);
    t
}

/// An earlier run's work: sorted under a cadence label and archived.
const fn sorted(t: Thread, label: &'static [&'static str]) -> Thread {
    archived(labeled(t, label))
}

const THREADS: &[Thread] = &[
    // Waiting on the user (task 2).
    unread(important(labeled(
        person(
            "renewal",
            "Renewal terms for next year",
            MORGAN,
            &[them(
                2 * HOUR,
                "Hi Jo,\n\nWe're starting our budget for next year. If we commit to two years instead of one, can you keep this year's per-seat price? Our finance team needs an answer by Friday to include it.\n\nThanks,\nMorgan",
            )],
        ),
        &["Label_6"],
    ))),
    unread(important(person(
        "board-prep",
        "Board prep — Thursday at 2?",
        SAM,
        &[them(
            40 * MINUTE,
            "Hi Jo,\n\nCan you do Thursday at 2pm for the board prep with Avery? If not, Friday at 10 is open too. I'll send the invite once you pick.\n\nSam",
        )],
    ))),
    labeled(
        person(
            "designer",
            "Interview feedback for the designer role",
            TAYLOR,
            &[
                me(26 * HOUR, "Hi Taylor,\n\nHow did the final round with the designer candidate go?\n\nJo"),
                them(
                    5 * HOUR,
                    "Hi Jo,\n\nStrong portfolio and great with the team; a little light on systems work. I'd lean towards an offer. Do you want to meet them once more, or should I start on the offer?\n\nTaylor",
                ),
            ],
        ),
        &["Label_3"],
    ),
    unread(person(
        "intro",
        "Intro: Riley Moreau <> Jo",
        JORDAN,
        &[them(
            22 * HOUR,
            "Hi Jo,\n\nRiley runs partnerships at a company that sells to the same teams you do, and has been asking how you handle onboarding. I think you'd both get a lot from a call. Riley, meet Jo; I'll let you two take it from here.\n\nBest,\nJordan",
        )],
    )),
    important(attached(
        person(
            "redlines",
            "Contract redlines — two open questions",
            CASEY,
            &[them(
                20 * HOUR,
                "Hi Jo,\n\nOur counsel's redlines are attached. Two open questions: can the liability cap be twelve months of fees instead of six, and can we have 60 days' notice instead of 30 for price changes? Everything else looks fine to us.\n\nRegards,\nCasey",
            )],
        ),
        "Contract-redlines.pdf",
    )),
    unread(important(labeled(
        person(
            "launch",
            "Launch date — go or no-go?",
            AVERY,
            &[them(
                3 * HOUR,
                "Hi Jo,\n\nQA signed off this morning. The only open item is the pricing page copy. Are we go for Tuesday, or do you want to push to Thursday?\n\nAvery",
            )],
        ),
        &["Label_8"],
    ))),
    person(
        "dinner",
        "Dinner next week?",
        QUINN,
        &[them(
            2 * DAY,
            "Hey Jo,\n\nIt's been ages. Are you free for dinner next Wednesday or Thursday? I'm happy to come to you.\n\nQuinn",
        )],
    ),
    unread(labeled(
        person(
            "press",
            "Interview request about the launch",
            DREW,
            &[them(
                6 * HOUR,
                "Hello Jo,\n\nI'm writing a piece on small teams shipping AI features and would love twenty minutes with you this week or next. Would any afternoon work?\n\nBest regards,\nDrew Haddad",
            )],
        ),
        &["Label_9"],
    )),
    // The user waiting on others (task 4): their message is the last one.
    archived(labeled(
        person(
            "pilot",
            "Proposal for the Q4 pilot",
            REESE,
            &[
                them(
                    8 * DAY,
                    "Hi Jo,\n\nCould you send a proposal for a small pilot in Q4? Ten seats, one team, about eight weeks.\n\nThanks,\nReese",
                ),
                me(
                    6 * DAY,
                    "Hi Reese,\n\nHere's the proposal: ten seats for eight weeks, onboarding included, and a review call at week six. Happy to walk your team through it.\n\nJo",
                ),
            ],
        ),
        &["Label_7"],
    )),
    archived(person(
        "ip",
        "Question on the IP assignment",
        KENDALL,
        &[me(
            4 * DAY,
            "Hi Kendall,\n\nDoes the IP assignment in our contractor agreement cover work done before the contract was signed? A contractor asked, and I'd like to answer correctly.\n\nThanks,\nJo",
        )],
    )),
    archived(person(
        "quote",
        "Updated quote for the office move",
        HAYDEN,
        &[me(
            9 * DAY,
            "Hi Hayden,\n\nCould you send an updated quote for the move? We'd now like the 14th rather than the 7th.\n\nThanks,\nJo",
        )],
    )),
    archived(labeled(
        person(
            "references",
            "References for the designer candidate",
            EMERSON,
            &[me(
                3 * DAY,
                "Hi Emerson,\n\nThe designer candidate listed you as a reference. Would you have fifteen minutes this week to tell me about working with them?\n\nThanks,\nJo",
            )],
        ),
        &["Label_3"],
    )),
    // To read and file.
    person(
        "standup",
        "Notes from today's standup",
        PEYTON,
        &[them(
            4 * HOUR,
            "Hi all,\n\nNotes: pricing page copy due tomorrow; onboarding emails in review; the bug in exports is fixed and shipping tonight. Nothing needs you today.\n\nPeyton",
        )],
    ),
    person(
        "office",
        "Office closed Monday",
        LOGAN,
        &[them(
            DAY,
            "Hi everyone,\n\nThe office is closed on Monday for building maintenance. Work from home; nothing else changes.\n\nLogan",
        )],
    ),
    person(
        "thanks",
        "Re: Slides for the workshop",
        ROWAN,
        &[
            me(30 * HOUR, "Hi Rowan,\n\nHere are the slides from yesterday's workshop.\n\nJo"),
            them(7 * HOUR, "Thanks Jo, these are perfect. I'll share them with the team.\n\nRowan"),
        ],
    ),
    // Important automated mail from the last day (task 5): one or two
    // for each of the routine's labels.
    service(
        "outage",
        "Degraded performance in us-east",
        ALERTS,
        UPDATES_TAB,
        &[them(
            3 * HOUR,
            "We are investigating degraded performance for services in us-east. Some requests may be slower than usual. Next update in 60 minutes.",
        )],
    ),
    service(
        "declined",
        "Your payment was declined",
        BILLING,
        UPDATES_TAB,
        &[them(
            9 * HOUR,
            "We couldn't charge the card ending 4242 for your Example Cloud subscription. Update your payment method within 7 days to avoid interruption.",
        )],
    ),
    service(
        "receipt",
        "Your receipt from Example Cloud",
        BILLING,
        UPDATES_TAB,
        &[them(
            5 * HOUR,
            "Thanks for your payment of $240.00 for Example Cloud, Team plan, October. This receipt is for your records.",
        )],
    ),
    service(
        "statement",
        "Your monthly statement is ready",
        CARDS,
        UPDATES_TAB,
        &[them(
            12 * HOUR,
            "Your statement for the card ending 1881 is ready. Balance: $3,412.18. Payment due on the 28th.",
        )],
    ),
    service(
        "digest",
        "This week in design systems",
        DIGEST,
        FORUMS_TAB,
        &[them(
            6 * HOUR,
            "Five links worth your time: tokens at scale, accessible colour, motion that helps, the case for fewer components, and a teardown of a mail app.",
        )],
    ),
    service(
        "product",
        "New in Example Analytics: dashboards",
        UPDATES,
        PROMOTIONS_TAB,
        &[them(
            10 * HOUR,
            "Dashboards are here. Pin any chart, share a live link with your team, and schedule a weekly email of the numbers that matter.",
        )],
    ),
    service(
        "meetup",
        "You're invited: Founders Meetup on the 23rd",
        EVENTS,
        UPDATES_TAB,
        &[them(
            8 * HOUR,
            "Join 80 founders for talks and drinks on the 23rd at 6pm. RSVP to save your spot; space is limited.",
        )],
    ),
    service(
        "earlybird",
        "Early-bird pricing ends Friday",
        CONFERENCE,
        PROMOTIONS_TAB,
        &[them(
            15 * HOUR,
            "Summit 2027 early-bird tickets are 40% off until Friday at midnight. Group discounts for teams of five or more.",
        )],
    ),
    service(
        "followup",
        "Just following up on my last note",
        LEADS,
        PROMOTIONS_TAB,
        &[them(
            11 * HOUR,
            "Hi Jo, just following up on my last note. Could I get 15 minutes on your calendar to show how we fill your pipeline with qualified leads?",
        )],
    ),
    service(
        "sponsor",
        "Sponsorship packages for Summit 2027",
        SPONSORS,
        PROMOTIONS_TAB,
        &[them(
            18 * HOUR,
            "Put your brand in front of 2,000 builders. Our sponsorship packages start at $5,000; the prospectus is one click away.",
        )],
    ),
    service(
        "application",
        "New application for Product Designer",
        CAREERS,
        UPDATES_TAB,
        &[them(
            2 * HOUR,
            "A new candidate applied for Product Designer. Review their application and portfolio in your hiring dashboard.",
        )],
    ),
    service(
        "candidates",
        "3 new candidates applied this week",
        CAREERS,
        UPDATES_TAB,
        &[them(
            20 * HOUR,
            "Three new candidates applied to your open roles this week: two for Product Designer and one for Support Lead.",
        )],
    ),
    // An earlier run's sorting (task 5's stream), from the last few days.
    sorted(
        service(
            "outage-old",
            "Resolved: elevated error rates",
            ALERTS,
            UPDATES_TAB,
            &[them(
                2 * DAY,
                "The issue causing elevated error rates has been resolved. All services are operating normally.",
            )],
        ),
        &["Label_MI1"],
    ),
    sorted(
        service(
            "digest-old",
            "The week's best reads on product",
            DIGEST,
            FORUMS_TAB,
            &[them(
                3 * DAY,
                "Pricing pages that convert, writing release notes people read, and why your onboarding email is too long.",
            )],
        ),
        &["Label_MI2"],
    ),
    sorted(
        service(
            "newsletter-old",
            "Field notes: issue 42",
            UPDATES,
            PROMOTIONS_TAB,
            &[them(
                4 * DAY,
                "This issue: interviews with three support leads on what they automate, and what they never will.",
            )],
        ),
        &["Label_MI2"],
    ),
    sorted(
        service(
            "webinar-old",
            "Webinar: shipping weekly without breaking things",
            EVENTS,
            UPDATES_TAB,
            &[them(
                3 * DAY,
                "Join us Thursday at noon for a live session on release trains, feature flags and rollbacks.",
            )],
        ),
        &["Label_MI3"],
    ),
    sorted(
        service(
            "receipt-old",
            "Your receipt from Example Maps",
            BILLING,
            UPDATES_TAB,
            &[them(2 * DAY, "Thanks for your payment of $18.00 for Example Maps, Pro, October.")],
        ),
        &["Label_MI4"],
    ),
    sorted(
        service(
            "renewal-old",
            "Your subscription renews next week",
            BILLING,
            UPDATES_TAB,
            &[them(
                5 * DAY,
                "Your Example Docs subscription renews on the 18th for $96.00 a year. No action is needed.",
            )],
        ),
        &["Label_MI4"],
    ),
    sorted(
        service(
            "pitch-old",
            "Quick question about your hiring",
            LEADS,
            PROMOTIONS_TAB,
            &[them(
                4 * DAY,
                "Hi Jo, we've placed 40 designers at startups like yours this year. Open to a quick chat about your hiring plans?",
            )],
        ),
        &["Label_MI5"],
    ),
    sorted(
        service(
            "candidate-old",
            "New application for Support Lead",
            CAREERS,
            UPDATES_TAB,
            &[them(
                3 * DAY,
                "A new candidate applied for Support Lead. Review their application in your hiring dashboard.",
            )],
        ),
        &["Label_MI6"],
    ),
];

/// Mail that arrives while the app is open, in order: mostly new
/// conversations, and one answer to mail the user is waiting on.
const ARRIVALS: &[Thread] = &[
    unread(important(person(
        "arrive-budget",
        "Budget sign-off needed today",
        AVERY,
        &[them(
            0,
            "Hi Jo,\n\nFinance needs your sign-off on the launch budget today. It's the same as the draft you saw, plus $2,000 for the press kit. OK to approve?\n\nAvery",
        )],
    ))),
    service(
        "arrive-alert",
        "Certificate expires in 7 days",
        ALERTS,
        UPDATES_TAB,
        &[them(0, "The TLS certificate for app.example.com expires in 7 days. Renew it to avoid interruption.")],
    ),
    unread(important(person(
        "pilot",
        "Proposal for the Q4 pilot",
        REESE,
        &[them(
            0,
            "Hi Jo,\n\nSorry for the slow reply; the team liked the proposal. Can we start on the 1st, and could the review call be at week four instead of six?\n\nThanks,\nReese",
        )],
    ))),
    service(
        "arrive-newsletter",
        "Five things we learned shipping AI features",
        DIGEST,
        FORUMS_TAB,
        &[them(
            0,
            "Start with the boring cases, measure the edits people make, never send without a human, and two more.",
        )],
    ),
    unread(person(
        "arrive-coffee",
        "Coffee when you're back in town?",
        RILEY,
        &[them(
            0,
            "Hi Jo,\n\nThanks for taking the call last week. I'm in town on the 21st and 22nd if you'd like to continue over coffee.\n\nRiley",
        )],
    )),
    service(
        "arrive-receipt",
        "Your receipt from Example Calendar",
        BILLING,
        UPDATES_TAB,
        &[them(0, "Thanks for your payment of $8.00 for Example Calendar, Personal, October.")],
    ),
    unread(important(person(
        "arrive-candidate",
        "Offer letter draft for review",
        TAYLOR,
        &[them(
            0,
            "Hi Jo,\n\nI drafted the offer for the designer. Can you check the start date and the equity line before I send it?\n\nTaylor",
        )],
    ))),
    service(
        "arrive-pitch",
        "One last message",
        LEADS,
        PROMOTIONS_TAB,
        &[them(
            0,
            "Hi Jo, I don't want to clutter your inbox, so this is my last note. If pipeline is ever a priority, I'm one reply away.",
        )],
    ),
];

/// How many arrivals there are before they repeat.
pub fn arrival_count() -> usize {
    ARRIVALS.len()
}

/// The labels the workflow demo uses beyond the synthetic mailbox's.
fn cadence_labels() -> Vec<Label> {
    CADENCE_LABELS
        .iter()
        .map(|(id, name, color)| Label {
            id: LabelId::new(*id),
            name: (*name).to_owned(),
            kind: LabelKind::User,
            color: Some(LabelColor { background: (*color).to_owned(), text: "#ffffff".to_owned() }),
            visible: true,
        })
        .collect()
}

/// Add the workflow demo's conversations to `db`, dated back from `now`.
/// Blocking, like [`crate::demo::generate`], which should run first (the
/// nested labels some threads carry are its own).
pub fn generate(db: &Db, now: i64) -> StoreResult<DemoStats> {
    let labels = cadence_labels();
    let mut messages = Vec::new();
    for t in THREADS {
        messages.extend(thread_messages(t, &format!("demowf-{}", t.key), now));
    }
    let stats = DemoStats { threads: THREADS.len() as u32, messages: messages.len() as u32 };
    db.write_blocking(move |tx| {
        let mut w = MailWriter::new(tx);
        w.upsert_labels(&labels)?;
        for m in &messages {
            w.upsert_message(m)?;
        }
        w.finish()?;
        Ok(())
    })?;
    Ok(stats)
}

/// The `n`th arrival (0-based), dated `now`: new conversations get ids of
/// their own each time round, so they keep coming; an answer to an
/// existing conversation comes only the first time round.
pub fn arrival(n: usize, now: i64) -> Option<IncomingMessage> {
    let t = &ARRIVALS[n % ARRIVALS.len()];
    let round = n / ARRIVALS.len();
    let answers_existing = THREADS.iter().any(|e| e.key == t.key);
    if answers_existing && round > 0 {
        return None;
    }
    let thread_key = if answers_existing { format!("demowf-{}", t.key) } else { format!("demowf-{}-{round}", t.key) };
    let mut message = thread_messages(t, &thread_key, now).pop()?;
    if answers_existing {
        // After the messages already there.
        message.id = MessageId::new(format!("{thread_key}-reply{round}"));
        message.rfc822_message_id = Some(format!("<{thread_key}.reply{round}@example.com>"));
        message.subject = format!("Re: {}", t.subject);
    }
    Some(message)
}

fn thread_messages(t: &Thread, thread_key: &str, now: i64) -> Vec<IncomingMessage> {
    let thread_id = ThreadId::new(thread_key);
    let me_address = EmailAddress::new(Some("Me"), ME);
    let other = t.other.address();
    let last_theirs = t.msgs.iter().rposition(|m| !m.from_me);
    let mut previous: Option<(String, String, String)> = None;
    let mut out = Vec::with_capacity(t.msgs.len());
    for (i, m) in t.msgs.iter().enumerate() {
        let at = now - m.ago;
        let mut labels: Vec<LabelId> = Vec::new();
        if m.from_me {
            labels.push(LabelId::new("SENT"));
        } else {
            labels.push(LabelId::new(t.category));
            if t.inbox {
                labels.push(LabelId::new("INBOX"));
            }
            if t.important {
                labels.push(LabelId::new("IMPORTANT"));
            }
            if t.unread && Some(i) == last_theirs {
                labels.push(LabelId::new("UNREAD"));
            }
        }
        labels.extend(t.labels.iter().map(|l| LabelId::new(*l)));
        let author = if m.from_me { "Jo".to_owned() } else { t.other.first_name().to_owned() };
        let mut text = m.text.to_owned();
        let mut html = paragraphs(m.text);
        if let Some((who, before_text, before_html)) = previous.take() {
            let quoted: String = before_text.lines().map(|l| format!("> {l}\n")).collect();
            text.push_str(&format!("\n\nOn an earlier day, {who} wrote:\n{quoted}"));
            html.push_str(&format!(
                "<div><div dir=\"ltr\">On an earlier day, {who} wrote:<br></div><blockquote style=\"margin:0px 0px 0px 0.8ex;padding-left:1ex\">{before_html}</blockquote></div>"
            ));
        }
        previous = Some((author, m.text.to_owned(), paragraphs(m.text)));
        let attachments = match t.attachment {
            Some(file) if i == 0 && !m.from_me => {
                let pdf = crate::demo::one_page_pdf(t.subject);
                vec![IncomingAttachment {
                    part_id: Some("2".into()),
                    provider_attachment_id: None,
                    filename: file.to_owned(),
                    mime_type: "application/pdf".into(),
                    size: pdf.len() as u64,
                    content_id: None,
                    is_inline: false,
                    data: Some(pdf),
                }]
            }
            _ => vec![],
        };
        let subject =
            if i == 0 || t.subject.starts_with("Re: ") { t.subject.to_owned() } else { format!("Re: {}", t.subject) };
        let size = (text.len() + attachments.iter().map(|a| a.size as usize).sum::<usize>()) as u64;
        out.push(IncomingMessage {
            id: MessageId::new(format!("{thread_key}-m{i}")),
            thread_id: thread_id.clone(),
            rfc822_message_id: Some(format!("<{thread_key}.{i}@example.com>")),
            from: Some(if m.from_me { me_address.clone() } else { other.clone() }),
            to: vec![if m.from_me { other.clone() } else { me_address.clone() }],
            subject,
            date: at,
            internal_date: at,
            snippet: snippet(m.text),
            label_ids: labels,
            size_estimate: size,
            body: Some(Body { text_plain: Some(text), html_sanitized: Some(html), has_remote_images: false }),
            attachments,
            list: ListHeaders::default(),
            ..Default::default()
        });
    }
    out
}

/// The text without its greeting, on one line, as a list shows it.
fn snippet(text: &str) -> String {
    let mut lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if lines.len() > 1 && lines[0].ends_with(',') {
        lines.remove(0);
    }
    lines.join(" ").chars().take(120).collect()
}

fn paragraphs(text: &str) -> String {
    text.split("\n\n").map(|p| format!("<p>{}</p>", p.replace('\n', "<br>"))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read;
    use mail_domain::MailboxKind;

    const NOW: i64 = 1_790_000_000_000;

    /// A fresh store in a directory of its own, removed when dropped.
    struct Store(std::path::PathBuf, Db);

    impl Drop for Store {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("kaluta-demo-workflows-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::open(&dir.join("mail.sqlite")).unwrap();
        Store(dir, db)
    }

    #[test]
    fn every_address_is_invented() {
        for t in THREADS.iter().chain(ARRIVALS) {
            let domain = t.other.1.split('@').nth(1).unwrap();
            assert!(["example.com", "example.org", "example.net"].contains(&domain), "{}", t.other.1);
        }
    }

    #[test]
    fn the_newest_mail_is_from_the_last_hour_and_the_routine_has_work() {
        let Store(_, db) = &store("newest");
        let stats = generate(db, NOW).unwrap();
        assert_eq!(stats.threads, THREADS.len() as u32);
        let newest = THREADS.iter().flat_map(|t| t.msgs).map(|m| m.ago).min().unwrap();
        assert!(newest <= HOUR);
        let important_today = THREADS
            .iter()
            .filter(|t| t.inbox && t.important && t.category != PERSONAL && t.msgs.iter().all(|m| m.ago < DAY))
            .count();
        assert!(important_today >= 6, "{important_today}");
        let mailboxes = db.read_blocking(read::list_mailboxes).unwrap();
        let important = mailboxes.iter().find(|m| m.kind == MailboxKind::Important).unwrap();
        assert!(important.total_count > 0);
        assert!(db.read_blocking(crate::consistency::check).unwrap().is_empty());
    }

    #[test]
    fn the_user_is_waiting_on_some_sent_mail() {
        let waiting: Vec<_> =
            THREADS.iter().filter(|t| t.msgs.last().is_some_and(|m| m.from_me && m.ago >= 3 * DAY)).collect();
        assert!(waiting.len() >= 3);
    }

    #[test]
    fn arrivals_keep_coming_but_answer_a_thread_once() {
        let Store(_, db) = &store("arrivals");
        generate(db, NOW).unwrap();
        let n = arrival_count();
        let first: Vec<_> = (0..n).filter_map(|i| arrival(i, NOW)).collect();
        assert_eq!(first.len(), n);
        let reply = first.iter().find(|m| m.thread_id.as_str() == "demowf-pilot").unwrap();
        assert!(reply.subject.starts_with("Re: "));
        assert_ne!(reply.id.as_str(), "demowf-pilot-m0");
        let second: Vec<_> = (n..2 * n).filter_map(|i| arrival(i, NOW)).collect();
        assert_eq!(second.len(), n - 1, "the answer comes once");
        assert!(second.iter().all(|m| m.thread_id.as_str().ends_with("-1")));
    }
}
