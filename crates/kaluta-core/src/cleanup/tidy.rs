//! Tidy the Inbox: Clean Up's light version on the Dashboard (spec §14.12,
//! *On the Dashboard*). A few large groups of Inbox mail the user is
//! unlikely to answer, each archived with one click and undone with ⌘Z:
//! Promotions, the mail of the busiest senders the user has never written
//! to, and everything older than three months.

use mail_domain::Millis;
use mail_store::cleanup::{self, Query, Scope, View};
use mail_store::{Connection, StoreResult};

use super::CleanupView;
use crate::{Core, CoreError, runtime};

/// A group must hold at least this many messages to be offered.
pub(crate) const MIN_SUGGESTION: u64 = 20;
/// How many of the busiest senders are offered.
const SENDERS: usize = 3;
/// Mail older than this many calendar months back is offered as a whole.
const OLD_MONTHS: i64 = 3;

/// One suggestion on the card: archive these Clean Up groups.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TidySuggestion {
    /// "Promotions", "Zoom", "Older than three months".
    pub title: String,
    /// "From 41 senders", "no-reply@zoom.us", "Before August 2026".
    pub detail: String,
    /// The Clean Up view and groups to archive (`cleanup_apply`, Inbox).
    pub view: CleanupView,
    pub keys: Vec<String>,
    /// Inbox messages in them now.
    pub count: u64,
}

/// The Dashboard's Inbox card.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InboxTidy {
    /// Messages in the Inbox now.
    pub inbox: u64,
    /// Largest first.
    pub suggestions: Vec<TidySuggestion>,
}

fn query(view: View, now: Millis, utc_offset_secs: i64) -> Query {
    Query { view, scope: Scope::Inbox, now, utc_offset_secs }
}

/// `YYYY-MM` of the month `back` months before `day` (`YYYY-MM-DD`).
fn month_back(day: &str, back: i64) -> Option<String> {
    let y: i64 = day.get(0..4)?.parse().ok()?;
    let m: i64 = day.get(5..7)?.parse().ok()?;
    let months = y * 12 + (m - 1) - back;
    Some(format!("{:04}-{:02}", months.div_euclid(12), months.rem_euclid(12) + 1))
}

/// "August 2026" for `2026-08`.
fn month_name(key: &str) -> String {
    const NAMES: [&str; 12] = [
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
    let month = key.get(5..7).and_then(|m| m.parse::<usize>().ok()).filter(|m| (1..=12).contains(m));
    match (key.get(0..4), month) {
        (Some(y), Some(m)) => format!("{} {y}", NAMES[m - 1]),
        _ => key.to_owned(),
    }
}

fn plural(n: usize, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// The card at `now`, for a user whose own address is `me` (left out of
/// the senders: mail to oneself is a note, not noise).
pub(crate) fn suggest(
    conn: &Connection,
    now: Millis,
    utc_offset_secs: i64,
    me: Option<&str>,
) -> StoreResult<InboxTidy> {
    let inbox = cleanup::groups(conn, &query(View::Time, now, utc_offset_secs), "")?.iter().map(|g| g.count).sum();
    let mut out = Vec::new();

    let promotions = cleanup::groups(conn, &query(View::Promotions, now, utc_offset_secs), "")?;
    let count: u64 = promotions.iter().map(|g| g.count).sum();
    // Promotions in the Inbox by domain: a sender all of whose mail is
    // there is not offered again on its own.
    let promoted: std::collections::HashMap<String, u64> = if count >= MIN_SUGGESTION {
        promotions.iter().map(|g| (g.key.clone(), g.count)).collect()
    } else {
        Default::default()
    };
    if count >= MIN_SUGGESTION {
        out.push(TidySuggestion {
            title: "Promotions".into(),
            detail: format!("From {}", plural(promotions.len(), "sender")),
            view: CleanupView::Promotions,
            keys: promotions.into_iter().map(|g| g.key).collect(),
            count,
        });
    }

    // The busiest senders the user has never written to.
    let people: std::collections::HashSet<String> =
        cleanup::groups(conn, &query(View::People, now, utc_offset_secs), "")?.into_iter().map(|g| g.key).collect();
    let me = me.map(str::to_lowercase);
    let senders = cleanup::groups(conn, &query(View::Sender, now, utc_offset_secs), "")?
        .into_iter()
        .filter(|g| g.count >= MIN_SUGGESTION && !people.contains(&g.key) && Some(&g.key) != me.as_ref())
        .filter(|g| {
            let domain = g.key.rsplit_once('@').map_or("", |(_, d)| d);
            promoted.get(domain).is_none_or(|&n| n < g.count)
        })
        .take(SENDERS);
    for g in senders {
        out.push(TidySuggestion {
            detail: g.detail.clone().unwrap_or_else(|| g.key.clone()),
            title: g.title,
            view: CleanupView::Sender,
            keys: vec![g.key],
            count: g.count,
        });
    }

    // Whole months before the one three back: what is that old is done
    // with, or too late to answer.
    let today = cleanup::day_key(now, utc_offset_secs);
    if let Some(cutoff) = month_back(&today, OLD_MONTHS) {
        let old: Vec<cleanup::Group> = cleanup::groups(conn, &query(View::Time, now, utc_offset_secs), "")?
            .into_iter()
            .filter(|g| g.key.len() == 7 && g.key.as_bytes()[4] == b'-' && g.key < cutoff)
            .collect();
        let count: u64 = old.iter().map(|g| g.count).sum();
        if count >= MIN_SUGGESTION {
            out.push(TidySuggestion {
                title: "Older than three months".into(),
                detail: format!("Before {}", month_name(&cutoff)),
                view: CleanupView::Time,
                keys: old.into_iter().map(|g| g.key).collect(),
                count,
            });
        }
    }

    out.sort_by_key(|s| std::cmp::Reverse(s.count));
    Ok(InboxTidy { inbox, suggestions: out })
}

#[uniffi::export]
impl Core {
    /// The Dashboard's Inbox card for `account_id`: the Inbox's size and a
    /// few groups worth archiving whole. Archive one with `cleanup_apply`
    /// (its view and keys, scope Inbox).
    pub async fn inbox_tidy(&self, account_id: String) -> Result<InboxTidy, CoreError> {
        let db = self.store_for(&account_id).await?;
        let (now, offset) = (mail_sync::now_millis(), super::utc_offset_now());
        runtime::run(async move {
            Ok(db
                .read(move |c| {
                    let me = mail_store::read::sync_state(c, "account_email")?;
                    suggest(c, now, offset, me.as_deref())
                })
                .await?)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_back_cross_years() {
        assert_eq!(month_back("2026-11-04", 3).as_deref(), Some("2026-08"));
        assert_eq!(month_back("2026-02-28", 3).as_deref(), Some("2025-11"));
        assert_eq!(month_back("2026-01-01", 12).as_deref(), Some("2025-01"));
        assert_eq!(month_name("2026-08"), "August 2026");
    }
}
