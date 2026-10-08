//! Clean Up (spec §14.12): the account's messages grouped by sender,
//! subject, mailing list, time, category or size, the messages in chosen
//! groups, and the Inbox counts behind the progress card.
//!
//! Everything here counts and lists messages, not threads: acting on a
//! group changes those messages only. A group is named by its key; the set
//! of messages behind a key is resolved when asked, so a group that grew
//! since it was shown is listed (and acted on) as it is now.

use std::collections::BTreeMap;

use mail_domain::{EmailAddress, LabelId, MessageId, Millis, ThreadId, civil_from_days, system_labels};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Transaction, params, params_from_iter};

use crate::error::StoreResult;
use crate::outbox::OutboxOp;
use crate::undo::{self, MessageDiff};
use crate::write::{LOCAL_PREFIX, MailWriter, ThreadChanges};

/// How groups are formed (the window's left column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    /// By sender address; the most used name is the title.
    Sender,
    /// Senders the user has written to (`contacts.sent_count > 0`).
    People,
    /// By identical subject, as stored (`Re:` kept).
    Subject,
    /// By `List-Id`; only mail stored since the list headers were.
    MailingList,
    /// Today, Yesterday, This Week, Last Week, then calendar months.
    Time,
    /// By sender domain, within Gmail's Social category.
    Social,
    /// By sender domain, within Gmail's Promotions category.
    Promotions,
    /// By size: Tiny to Jumbo (see [`SIZE_BUCKETS`]).
    Size,
}

/// Which messages are grouped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Messages carrying `INBOX`.
    Inbox,
    /// Everything but Spam, Trash and drafts.
    AllMail,
}

/// What a query needs besides the store: the view, the scope and, for the
/// Time view, the moment and the user's offset from UTC (taken as
/// parameters so results do not depend on the machine's clock).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Query {
    pub view: View,
    pub scope: Scope,
    pub now: Millis,
    /// Seconds east of UTC. A fixed offset: around a daylight-saving
    /// change, month edges can be an hour off.
    pub utc_offset_secs: i64,
}

/// One row of the groups column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// Opaque to callers; passed back to name the group.
    pub key: String,
    pub title: String,
    /// Other names the sender (or list) used, most used first, at most
    /// [`MAX_AKA`].
    pub aka: Vec<String>,
    /// A secondary line: the address (Sender, People), the list id
    /// (Mailing Lists), the range of sizes (Size).
    pub detail: Option<String>,
    pub count: u64,
}

/// One row of the messages column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupMessage {
    pub id: MessageId,
    pub thread_id: ThreadId,
    pub from: Option<EmailAddress>,
    pub subject: String,
    pub date: Millis,
    pub size: u64,
}

pub const MAX_AKA: usize = 5;

/// Size buckets: key, title, smallest size in bytes (decimal units, as
/// macOS shows sizes), and the range as the group's second line. Each
/// runs up to the next one's start.
pub const SIZE_BUCKETS: &[(&str, &str, u64, &str)] = &[
    ("tiny", "Tiny", 0, "Less than 1 KB"),
    ("small", "Small", 1_000, "1 KB to 10 KB"),
    ("medium", "Medium", 10_000, "10 KB to 100 KB"),
    ("large", "Large", 100_000, "100 KB to 1 MB"),
    ("extra_large", "Extra Large", 1_000_000, "1 MB to 10 MB"),
    ("jumbo", "Jumbo", 10_000_000, "More than 10 MB"),
];

const SOCIAL: &str = "CATEGORY_SOCIAL";
const PROMOTIONS: &str = "CATEGORY_PROMOTIONS";
const DAY_MS: i64 = 86_400_000;
/// The sender's domain, lower-cased.
const DOMAIN: &str = "lower(substr(m.from_email, instr(m.from_email, '@') + 1))";

/// The groups of a view, largest first (Time and Size in their own
/// order). `filter` keeps groups whose title, aka, detail or key contains
/// it, ignoring case.
pub fn groups(conn: &Connection, q: &Query, filter: &str) -> StoreResult<Vec<Group>> {
    let mut params = Vec::new();
    let scope = scope_sql(conn, q.scope, &mut params)?;
    let mut groups = match q.view {
        View::Sender | View::People => {
            let people = if q.view == View::People {
                " AND EXISTS (SELECT 1 FROM contacts c WHERE c.email = m.from_email AND c.sent_count > 0)"
            } else {
                ""
            };
            let sql = format!(
                "SELECT lower(m.from_email), m.from_name, COUNT(*) FROM messages m
                 WHERE {scope} AND m.from_email IS NOT NULL AND m.from_email != ''{people}
                 GROUP BY m.from_email COLLATE NOCASE, m.from_name"
            );
            named(conn, &sql, &params, Titled::ByName)?
        }
        View::MailingList => {
            let sql = format!(
                "SELECT m.list_id, m.list_name, COUNT(*) FROM messages m
                 WHERE {scope} AND m.list_id IS NOT NULL GROUP BY m.list_id, m.list_name"
            );
            named(conn, &sql, &params, Titled::ByName)?
        }
        View::Social | View::Promotions => {
            let category = label_rowid(conn, if q.view == View::Social { SOCIAL } else { PROMOTIONS })?;
            params.push(Value::Integer(category));
            let n = params.len();
            let sql = format!(
                "SELECT {DOMAIN}, m.from_name, COUNT(*) FROM messages m
                 WHERE {scope} AND m.id IN (SELECT message_id FROM message_labels WHERE label_id = ?{n})
                   AND instr(m.from_email, '@') > 0
                 GROUP BY 1, m.from_name"
            );
            named(conn, &sql, &params, Titled::ByKey)?
        }
        View::Subject => {
            let sql = format!("SELECT m.subject, COUNT(*) FROM messages m WHERE {scope} GROUP BY m.subject");
            let mut stmt = conn.prepare_cached(&sql)?;
            let rows = stmt.query_map(params_from_iter(&params), |r| {
                let subject: String = r.get(0)?;
                let count: i64 = r.get(1)?;
                Ok(Group {
                    title: if subject.trim().is_empty() { "(no subject)".to_owned() } else { subject.clone() },
                    key: subject,
                    aka: vec![],
                    detail: None,
                    count: count as u64,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        }
        View::Time => time_groups(conn, q, &scope, params)?,
        View::Size => size_groups(conn, &scope, &params)?,
    };
    if matches!(
        q.view,
        View::Sender | View::People | View::Subject | View::MailingList | View::Social | View::Promotions
    ) {
        groups.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                .then_with(|| a.key.cmp(&b.key))
        });
    }
    let filter = filter.trim().to_lowercase();
    if !filter.is_empty() {
        groups.retain(|g| {
            let hit = |s: &str| s.to_lowercase().contains(&filter);
            hit(&g.title) || hit(&g.key) || g.detail.as_deref().is_some_and(hit) || g.aka.iter().any(|a| hit(a))
        });
    }
    Ok(groups)
}

/// The messages in the groups named by `keys`, newest first, paged.
pub fn messages(
    conn: &Connection,
    q: &Query,
    keys: &[String],
    offset: u32,
    limit: u32,
) -> StoreResult<Vec<CleanupMessage>> {
    let mut params = Vec::new();
    let filter = selection_sql(conn, q, keys, &mut params)?;
    params.push(Value::Integer(i64::from(limit)));
    params.push(Value::Integer(i64::from(offset)));
    let n = params.len();
    // The page's ids first, from the views' covering indexes (which end
    // in `date`); only those rows are then read.
    let sql = format!(
        "SELECT m.gmail_id, t.gmail_id, m.from_name, m.from_email, m.subject, m.date, m.size_estimate
         FROM (SELECT m.id FROM messages m WHERE {filter} ORDER BY m.date DESC, m.id DESC LIMIT ?{} OFFSET ?{n}) p
         JOIN messages m ON m.id = p.id JOIN threads t ON t.id = m.thread_id
         ORDER BY m.date DESC, m.id DESC",
        n - 1
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params_from_iter(&params), |r| {
        let name: Option<String> = r.get(2)?;
        let email: Option<String> = r.get(3)?;
        Ok(CleanupMessage {
            id: MessageId(r.get(0)?),
            thread_id: ThreadId(r.get(1)?),
            from: email.map(|e| EmailAddress::new(name.as_deref(), &e)),
            subject: r.get(4)?,
            date: r.get(5)?,
            size: r.get::<_, i64>(6)?.max(0) as u64,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// How many messages the groups named by `keys` hold.
pub fn count(conn: &Connection, q: &Query, keys: &[String]) -> StoreResult<u64> {
    let mut params = Vec::new();
    let filter = selection_sql(conn, q, keys, &mut params)?;
    let sql = format!("SELECT COUNT(*) FROM messages m WHERE {filter}");
    let n: i64 = conn.prepare_cached(&sql)?.query_row(params_from_iter(&params), |r| r.get(0))?;
    Ok(n as u64)
}

/// Every message in the groups named by `keys`, as they are now: what an
/// action applies to, the same messages [`count`] counts.
pub fn message_ids(conn: &Connection, q: &Query, keys: &[String]) -> StoreResult<Vec<MessageId>> {
    let mut params = Vec::new();
    let filter = selection_sql(conn, q, keys, &mut params)?;
    let sql = format!("SELECT m.gmail_id FROM messages m WHERE {filter} ORDER BY m.id");
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params_from_iter(&params), |r| Ok(MessageId(r.get(0)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// What [`apply`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Per message, the labels it really gained and lost: the undo record.
    /// Messages the change left as they were are not in it.
    pub diffs: Vec<MessageDiff>,
    /// The same changes for the provider, in batches ([`undo::batched_ops`]).
    pub ops: Vec<OutboxOp>,
    /// The threads that changed, for the UI.
    pub changes: ThreadChanges,
}

/// Add `add` and remove `remove` on every message in the groups named by
/// `keys`, resolved now, in the caller's transaction: a group that grew
/// since it was shown is acted on as it is now. Message-level: the other
/// messages of a mixed thread stay where they are, while the thread's
/// labels, counts and mailboxes follow through [`MailWriter`]. Queueing
/// `ops` and recording `diffs` for undo are the caller's.
pub fn apply(
    tx: &Transaction<'_>,
    q: &Query,
    keys: &[String],
    add: &[LabelId],
    remove: &[LabelId],
) -> StoreResult<Applied> {
    let ids = message_ids(tx, q, keys)?;
    let mut diffs = Vec::new();
    let mut w = MailWriter::new(tx);
    for message in ids {
        let before = crate::outbox::current_labels(tx, &message)?;
        let added: Vec<LabelId> = add.iter().filter(|l| !before.contains(l)).cloned().collect();
        let removed: Vec<LabelId> = remove.iter().filter(|l| before.contains(l) && !add.contains(l)).cloned().collect();
        if added.is_empty() && removed.is_empty() {
            continue;
        }
        w.modify_message_labels(&message, &added, &removed)?;
        diffs.push(MessageDiff { message, added, removed });
    }
    let changes = w.finish()?;
    let ops = undo::batched_ops(&diffs);
    Ok(Applied { diffs, ops, changes })
}

/// Messages in the Inbox now.
pub fn inbox_count(conn: &Connection) -> StoreResult<u64> {
    let n: i64 = conn
        .prepare_cached(
            "SELECT COUNT(*) FROM message_labels WHERE label_id = (SELECT id FROM labels WHERE gmail_id = ?1)",
        )?
        .query_row([system_labels::INBOX], |r| r.get(0))?;
    Ok(n as u64)
}

/// Record the Inbox count at the start of `day` (`YYYY-MM-DD`). The first
/// record of a day stands; returns whether this one was it.
pub fn record_inbox_day(conn: &Connection, day: &str, count: u64) -> StoreResult<bool> {
    let n = conn
        .prepare_cached("INSERT OR IGNORE INTO inbox_history (day, count) VALUES (?1, ?2)")?
        .execute(params![day, count as i64])?;
    Ok(n > 0)
}

/// Daily Inbox counts from `since` (`YYYY-MM-DD`) on, oldest first.
pub fn inbox_days(conn: &Connection, since: &str) -> StoreResult<Vec<(String, u64)>> {
    let mut stmt = conn.prepare_cached("SELECT day, count FROM inbox_history WHERE day >= ?1 ORDER BY day")?;
    let rows = stmt.query_map([since], |r| Ok((r.get(0)?, r.get::<_, i64>(1)?.max(0) as u64)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

const BASELINE_KEY: &str = "baseline_count";
const BASELINE_AT_KEY: &str = "baseline_at";

/// The Inbox count when Clean Up was first opened, and when that was.
pub fn baseline(conn: &Connection) -> StoreResult<Option<(u64, Millis)>> {
    let get = |key: &str| -> StoreResult<Option<i64>> {
        let v: Option<String> = conn
            .prepare_cached("SELECT value FROM cleanup_meta WHERE key = ?1")?
            .query_row([key], |r| r.get(0))
            .optional()?;
        Ok(v.and_then(|v| v.parse().ok()))
    };
    Ok(get(BASELINE_KEY)?.zip(get(BASELINE_AT_KEY)?).map(|(n, at)| (n.max(0) as u64, at)))
}

/// Set the baseline unless it is set; returns whether it was set now.
pub fn set_baseline_once(conn: &Connection, count: u64, at: Millis) -> StoreResult<bool> {
    if baseline(conn)?.is_some() {
        return Ok(false);
    }
    let mut stmt = conn.prepare_cached("INSERT OR REPLACE INTO cleanup_meta (key, value) VALUES (?1, ?2)")?;
    stmt.execute(params![BASELINE_KEY, count.to_string()])?;
    stmt.execute(params![BASELINE_AT_KEY, at.to_string()])?;
    Ok(true)
}

/// How [`named`] titles a group.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Titled {
    /// By the most used name (else the key), the key as detail: senders
    /// and lists.
    ByName,
    /// By the key (a domain), every name as aka.
    ByKey,
}

/// Groups from rows of (key, name, count): one group per key, the names
/// most used first.
fn named(conn: &Connection, sql: &str, params: &[Value], titled: Titled) -> StoreResult<Vec<Group>> {
    let mut by_key: BTreeMap<String, (u64, BTreeMap<String, u64>)> = BTreeMap::new();
    let mut stmt = conn.prepare_cached(sql)?;
    let mut rows = stmt.query(params_from_iter(params))?;
    while let Some(r) = rows.next()? {
        let key: String = r.get(0)?;
        let name: Option<String> = r.get(1)?;
        let n = r.get::<_, i64>(2)?.max(0) as u64;
        let entry = by_key.entry(key).or_default();
        entry.0 += n;
        if let Some(name) = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()) {
            *entry.1.entry(name).or_default() += n;
        }
    }
    Ok(by_key
        .into_iter()
        .map(|(key, (count, names))| {
            let mut names: Vec<(String, u64)> = names.into_iter().collect();
            names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let mut names = names.into_iter().map(|(n, _)| n);
            let (title, detail) = match titled {
                Titled::ByName => (names.next().unwrap_or_else(|| key.clone()), Some(key.clone())),
                Titled::ByKey => (key.clone(), None),
            };
            Group { title, aka: names.take(MAX_AKA).collect(), detail, count, key }
        })
        .collect())
}

/// The day boundaries of the Time view, in UTC milliseconds.
struct Days {
    today: i64,
    yesterday: i64,
    this_week: i64,
    last_week: i64,
}

impl Days {
    fn of(q: &Query) -> Self {
        let offset = q.utc_offset_secs * 1000;
        let day = (q.now + offset).div_euclid(DAY_MS);
        let today = day * DAY_MS - offset;
        // 1970-01-01 was a Thursday; weeks start on Monday.
        let weekday = (day + 3).rem_euclid(7);
        let this_week = today - weekday * DAY_MS;
        Days { today, yesterday: today - DAY_MS, this_week, last_week: this_week - 7 * DAY_MS }
    }

    /// `[start, end)` of a Time key; `None` for an unknown key.
    fn range(&self, q: &Query, key: &str) -> Option<(i64, i64)> {
        Some(match key {
            "today" => (self.today, i64::MAX),
            "yesterday" => (self.yesterday, self.today),
            "this_week" => (self.this_week, self.yesterday),
            "last_week" => (self.last_week, self.this_week.min(self.yesterday)),
            month => {
                let (y, m) = month.split_once('-')?;
                let (y, m): (i64, i64) = (y.parse().ok()?, m.parse().ok()?);
                if !(1..=12).contains(&m) {
                    return None;
                }
                let next = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
                let offset = q.utc_offset_secs * 1000;
                let start = days_from_civil(y, m, 1) * DAY_MS - offset;
                let end = days_from_civil(next.0, next.1, 1) * DAY_MS - offset;
                // Months hold what is older than Last Week.
                (start, end.min(self.last_week))
            }
        })
    }
}

fn time_groups(conn: &Connection, q: &Query, scope: &str, mut params: Vec<Value>) -> StoreResult<Vec<Group>> {
    let days = Days::of(q);
    let n = params.len();
    params.extend([days.today, days.yesterday, days.this_week, days.last_week, q.utc_offset_secs].map(Value::Integer));
    let sql = format!(
        "SELECT CASE WHEN m.date >= ?{a} THEN 'today' WHEN m.date >= ?{b} THEN 'yesterday'
                     WHEN m.date >= ?{c} THEN 'this_week' WHEN m.date >= ?{d} THEN 'last_week'
                     ELSE strftime('%Y-%m', m.date / 1000 + ?{e}, 'unixepoch') END AS k, COUNT(*)
         FROM messages m WHERE {scope} GROUP BY k",
        a = n + 1,
        b = n + 2,
        c = n + 3,
        d = n + 4,
        e = n + 5
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let counts: BTreeMap<String, i64> =
        stmt.query_map(params_from_iter(&params), |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
    let fixed =
        [("today", "Today"), ("yesterday", "Yesterday"), ("this_week", "This Week"), ("last_week", "Last Week")];
    let mut out: Vec<Group> =
        fixed.iter().filter_map(|(key, title)| counts.get(*key).map(|n| group(key, title, *n))).collect();
    let mut months: Vec<(&String, &i64)> = counts.iter().filter(|(k, _)| !fixed.iter().any(|(f, _)| f == k)).collect();
    months.sort_by(|a, b| b.0.cmp(a.0));
    out.extend(months.into_iter().map(|(key, n)| group(key, &month_title(key), *n)));
    Ok(out)
}

fn size_groups(conn: &Connection, scope: &str, params: &[Value]) -> StoreResult<Vec<Group>> {
    let cases: String = SIZE_BUCKETS
        .iter()
        .enumerate()
        .rev()
        .map(|(i, (_, _, min, _))| format!("WHEN m.size_estimate >= {min} THEN {i} "))
        .collect();
    let sql =
        format!("SELECT CASE {cases}ELSE 0 END AS b, COUNT(*) FROM messages m WHERE {scope} GROUP BY b ORDER BY b");
    let mut stmt = conn.prepare_cached(&sql)?;
    let rows = stmt.query_map(params_from_iter(params), |r| {
        let (key, title, _, range) = SIZE_BUCKETS[r.get::<_, i64>(0)?.clamp(0, SIZE_BUCKETS.len() as i64 - 1) as usize];
        Ok(Group { detail: Some(range.to_owned()), ..group(key, title, r.get(1)?) })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn group(key: &str, title: &str, count: i64) -> Group {
    Group { key: key.to_owned(), title: title.to_owned(), aka: vec![], detail: None, count: count.max(0) as u64 }
}

fn month_title(key: &str) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    match key.split_once('-').and_then(|(y, m)| Some((y, m.parse::<usize>().ok()?))) {
        Some((y, m)) if (1..=12).contains(&m) => format!("{} {y}", MONTHS[m - 1]),
        _ => key.to_owned(),
    }
}

/// Days since 1970-01-01 for a civil date (inverse of [`civil_from_days`]).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    debug_assert_eq!(civil_from_days(days), (if m <= 2 { y + 1 } else { y }, m, d));
    days
}

fn label_rowid(conn: &Connection, gmail_id: &str) -> StoreResult<i64> {
    // A label the store has never seen matches nothing.
    Ok(conn
        .prepare_cached("SELECT id FROM labels WHERE gmail_id = ?1")?
        .query_row([gmail_id], |r| r.get(0))
        .optional()?
        .unwrap_or(-1))
}

/// The scope as a condition on `m`, its values appended to `params`.
/// Optimistic local copies of sent mail (ids starting `local-`, replaced
/// when Gmail's copy syncs back) are never in scope: they are not the
/// provider's to change yet, so groups, counts and actions all leave them
/// out and the numbers shown are the numbers changed.
fn scope_sql(conn: &Connection, scope: Scope, params: &mut Vec<Value>) -> StoreResult<String> {
    // A range on the unique `gmail_id` index: the few local copies, once.
    params.push(Value::Text(LOCAL_PREFIX.to_owned()));
    params.push(Value::Text(LOCAL_PREFIX.trim_end_matches('-').to_owned() + "."));
    let local = format!(
        "m.id NOT IN (SELECT id FROM messages WHERE gmail_id >= ?{} AND gmail_id < ?{})",
        params.len() - 1,
        params.len()
    );
    let scope = match scope {
        Scope::Inbox => {
            params.push(Value::Integer(label_rowid(conn, system_labels::INBOX)?));
            format!("m.id IN (SELECT message_id FROM message_labels WHERE label_id = ?{})", params.len())
        }
        Scope::AllMail => {
            // Spam, Trash and drafts are few (Gmail empties the first two
            // after 30 days): one set to probe, and the views' indexes cover
            // the rest of the query.
            for label in [system_labels::SPAM, system_labels::TRASH, system_labels::DRAFT] {
                params.push(Value::Integer(label_rowid(conn, label)?));
            }
            let n = params.len();
            format!(
                "m.id NOT IN (SELECT message_id FROM message_labels WHERE label_id IN (?{}, ?{}, ?{n}))",
                n - 2,
                n - 1
            )
        }
    };
    Ok(format!("{scope} AND {local}"))
}

/// Scope and membership in the groups named by `keys`, as a condition on `m`.
fn selection_sql(conn: &Connection, q: &Query, keys: &[String], params: &mut Vec<Value>) -> StoreResult<String> {
    let scope = scope_sql(conn, q.scope, params)?;
    let keyed = |params: &mut Vec<Value>, column: &str| {
        params.push(Value::Text(serde_json::to_string(keys).unwrap_or_else(|_| "[]".into())));
        format!("{column} IN (SELECT value FROM json_each(?{}))", params.len())
    };
    let member = match q.view {
        View::Sender => keyed(params, "m.from_email COLLATE NOCASE"),
        View::People => format!(
            "{} AND EXISTS (SELECT 1 FROM contacts c WHERE c.email = m.from_email AND c.sent_count > 0)",
            keyed(params, "m.from_email COLLATE NOCASE")
        ),
        View::Subject => keyed(params, "m.subject"),
        View::MailingList => keyed(params, "m.list_id"),
        View::Social | View::Promotions => {
            let category = label_rowid(conn, if q.view == View::Social { SOCIAL } else { PROMOTIONS })?;
            params.push(Value::Integer(category));
            let n = params.len();
            format!(
                "m.id IN (SELECT message_id FROM message_labels WHERE label_id = ?{n}) AND {}",
                keyed(params, DOMAIN)
            )
        }
        View::Time => {
            let days = Days::of(q);
            ranges(params, "m.date", keys.iter().filter_map(|k| days.range(q, k)))
        }
        View::Size => {
            let buckets = keys.iter().filter_map(|k| {
                let i = SIZE_BUCKETS.iter().position(|(key, _, _, _)| key == k)?;
                let end = SIZE_BUCKETS.get(i + 1).map_or(i64::MAX, |b| b.2 as i64);
                Some((SIZE_BUCKETS[i].2 as i64, end))
            });
            ranges(params, "m.size_estimate", buckets)
        }
    };
    Ok(format!("{scope} AND ({member})"))
}

/// `[start, end)` ranges of `column`, OR-ed; nothing matches none.
fn ranges(params: &mut Vec<Value>, column: &str, ranges: impl Iterator<Item = (i64, i64)>) -> String {
    let parts: Vec<String> = ranges
        .filter(|(start, end)| start < end)
        .map(|(start, end)| {
            params.push(Value::Integer(start));
            params.push(Value::Integer(end));
            format!("({column} >= ?{} AND {column} < ?{})", params.len() - 1, params.len())
        })
        .collect();
    if parts.is_empty() { "0".to_owned() } else { parts.join(" OR ") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_days_round_trip() {
        for days in [-1000, 0, 59, 365, 20_000, 20_729] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
    }

    #[test]
    fn time_boundaries_follow_the_local_calendar() {
        // Thursday 2026-10-08 10:00 at UTC-7 (17:00 UTC).
        let now = (days_from_civil(2026, 10, 8) * DAY_MS) + 17 * 3_600_000;
        let q = Query { view: View::Time, scope: Scope::Inbox, now, utc_offset_secs: -7 * 3600 };
        let days = Days::of(&q);
        let local_midnight = days_from_civil(2026, 10, 8) * DAY_MS + 7 * 3_600_000;
        assert_eq!(days.today, local_midnight);
        assert_eq!(days.this_week, local_midnight - 3 * DAY_MS, "Monday the 5th");
        assert_eq!(days.last_week, local_midnight - 10 * DAY_MS);
        assert_eq!(
            days.range(&q, "2026-09"),
            Some((days_from_civil(2026, 9, 1) * DAY_MS + 7 * 3_600_000, days.last_week))
        );
        assert_eq!(days.range(&q, "2026-13"), None);
        assert_eq!(month_title("2026-09"), "September 2026");
    }
}
