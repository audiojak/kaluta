//! Offering times (spec §14.9, ADR 0018): free slots from the user's
//! Google Calendar and tentative holds on it, through the mail account's
//! own sign-in once the user connects the calendar. The agent sees slots
//! only, never what the user is doing; a hold invites no one and the user
//! can release it. The user's hours and time zone come from their
//! *Availability* facts, else the Mac's zone and 9 to 5.

use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveTime, Utc};
use chrono_tz::Tz;
use mail_store::calendar as store;
use provider_api::{HttpClient, RateLimiter, RetryPolicy};
use provider_gcal::{Busy, GoogleCalendar, Hold, Slot, SlotRequest};

use crate::{Core, CoreError, ErrorKind, runtime};

/// What the agent is told when the calendar is not connected.
pub(crate) const NOT_CONNECTED: &str = "The user has not connected their calendar to Kaluta.";
/// A hold is at most this long.
pub(crate) const MAX_HOLD_MINUTES: u32 = 240;
/// …and starts within this many days.
pub(crate) const MAX_HOLD_DAYS: i64 = 60;
/// Slots start at least this long from now.
const NOTICE_HOURS: i64 = 2;

/// Where busy times come from and holds go: Google, or a fake.
#[async_trait]
pub(crate) trait CalendarSource: Send + Sync {
    async fn busy(&self, from: DateTime<Utc>, to: DateTime<Utc>, calendars: &[String]) -> Result<Vec<Busy>, CoreError>;
    async fn hold(&self, hold: &Hold) -> Result<String, CoreError>;
    async fn release(&self, event_id: &str) -> Result<(), CoreError>;
}

struct Google(GoogleCalendar);

#[async_trait]
impl CalendarSource for Google {
    async fn busy(&self, from: DateTime<Utc>, to: DateTime<Utc>, calendars: &[String]) -> Result<Vec<Busy>, CoreError> {
        let (cal, calendars) = (self.0.clone(), calendars.to_vec());
        runtime::run(async move { Ok(cal.busy(from, to, &calendars).await?) }).await
    }
    async fn hold(&self, hold: &Hold) -> Result<String, CoreError> {
        let (cal, hold) = (self.0.clone(), hold.clone());
        runtime::run(async move { Ok(cal.hold(&hold).await?) }).await
    }
    async fn release(&self, event_id: &str) -> Result<(), CoreError> {
        let (cal, id) = (self.0.clone(), event_id.to_owned());
        runtime::run(async move { Ok(cal.release(&id).await?) }).await
    }
}

/// The demo's and tests' calendar: busy every day from 19:00 to 20:00 UTC
/// (noon in California), and its own holds.
#[derive(Default)]
pub(crate) struct FakeCalendar {
    holds: Mutex<Vec<(String, Busy)>>,
}

impl FakeCalendar {
    pub(crate) fn holds(&self) -> Vec<(String, Busy)> {
        self.holds.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[async_trait]
impl CalendarSource for FakeCalendar {
    async fn busy(&self, from: DateTime<Utc>, to: DateTime<Utc>, _: &[String]) -> Result<Vec<Busy>, CoreError> {
        let mut busy: Vec<Busy> = self.holds().into_iter().map(|(_, b)| b).collect();
        let mut day = from.date_naive();
        while day <= to.date_naive() {
            let at = |h| day.and_hms_opt(h, 0, 0).map(|t| t.and_utc());
            if let (Some(start), Some(end)) = (at(19), at(20)) {
                busy.push(Busy { start, end });
            }
            let Some(next) = day.succ_opt() else { break };
            day = next;
        }
        Ok(busy)
    }
    async fn hold(&self, hold: &Hold) -> Result<String, CoreError> {
        let mut holds = self.holds.lock().unwrap_or_else(|e| e.into_inner());
        let id = format!("fakehold{}", holds.len() + 1);
        holds.push((id.clone(), Busy { start: hold.start, end: hold.end }));
        Ok(id)
    }
    async fn release(&self, event_id: &str) -> Result<(), CoreError> {
        self.holds.lock().unwrap_or_else(|e| e.into_inner()).retain(|(id, _)| id != event_id);
        Ok(())
    }
}

/// The core's calendar state: a fake, for the demo and tests.
#[derive(Default)]
pub(crate) struct CalendarState {
    fake: OnceLock<Arc<FakeCalendar>>,
}

/// Whether the open account's calendar is connected, and how Kaluta reads it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CalendarStatus {
    /// Free slots can be offered.
    pub connected: bool,
    /// Holds can be made.
    pub can_hold: bool,
    /// IANA name ("America/Los_Angeles").
    pub time_zone: String,
    /// "9 am–5 pm".
    pub hours: String,
}

/// A free time to offer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CalendarSlot {
    pub start_ms: i64,
    pub end_ms: i64,
    /// "Thu Oct 30, 2–3 pm", in the user's zone.
    pub label: String,
}

/// Free times, in the user's zone.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CalendarSlots {
    pub time_zone: String,
    pub slots: Vec<CalendarSlot>,
}

/// A hold Kaluta made.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CalendarHoldInfo {
    pub id: i64,
    pub title: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub label: String,
    pub thread_id: Option<String>,
    pub released: bool,
}

/// The user's hours from a *Usual hours* fact ("9–5", "9am-6pm",
/// "09:00–17:30", "10-6pm"); without am or pm an end before the start is
/// in the afternoon.
pub(crate) fn parse_hours(text: &str) -> Option<(NaiveTime, NaiveTime)> {
    let t = text.to_lowercase().replace(['–', '—'], "-").replace(" to ", "-");
    let (a, b) = t.split_once('-')?;
    let part = |p: &str| -> Option<(u32, u32, Option<bool>)> {
        let p = p.trim().replace('.', ":");
        let (p, pm) = if let Some(x) = p.strip_suffix("pm") {
            (x.trim().to_owned(), Some(true))
        } else if let Some(x) = p.strip_suffix("am") {
            (x.trim().to_owned(), Some(false))
        } else {
            (p, None)
        };
        let (h, m) = match p.split_once(':') {
            Some((h, m)) => (h.trim().parse().ok()?, m.trim().parse().ok()?),
            None => (p.trim().parse().ok()?, 0),
        };
        (h <= 24 && m < 60).then_some((h, m, pm))
    };
    let (mut h1, m1, pm1) = part(a)?;
    let (mut h2, m2, pm2) = part(b)?;
    let twelve = |h: u32, pm: Option<bool>| match pm {
        Some(true) if h < 12 => h + 12,
        Some(false) if h == 12 => 0,
        _ => h,
    };
    h1 = twelve(h1, pm1);
    h2 = twelve(h2, pm2);
    if pm1.is_none() && pm2 == Some(true) && h1 < 12 && h1 + 12 <= h2 {
        h1 += 12;
    }
    if pm2.is_none() && h2 * 60 + m2 <= h1 * 60 + m1 && h2 < 12 {
        h2 += 12;
    }
    let (start, end) = (NaiveTime::from_hms_opt(h1, m1, 0)?, NaiveTime::from_hms_opt(h2.min(23), m2, 0)?);
    (start < end).then_some((start, end))
}

/// A time zone from a *Time zone* fact: an IANA name, alone or within it.
pub(crate) fn parse_zone(text: &str) -> Option<Tz> {
    Tz::from_str(text.trim()).ok().or_else(|| {
        text.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ','))
            .find(|w| w.contains('/'))
            .and_then(|w| Tz::from_str(w).ok())
    })
}

fn hours_label(start: NaiveTime, end: NaiveTime) -> String {
    let at = |t: NaiveTime| {
        let (pm, h) = (t.format("%P").to_string(), t.format("%-I").to_string());
        let m = t.format("%M").to_string();
        if m == "00" { format!("{h} {pm}") } else { format!("{h}:{m} {pm}") }
    };
    format!("{}–{}", at(start), at(end))
}

impl Core {
    /// The calendar for the open account, what it grants, and the calendars
    /// that count; `None` when not connected.
    async fn calendar_source(&self) -> Result<Option<(Arc<dyn CalendarSource>, store::CalendarAccess)>, CoreError> {
        if let Some(fake) = self.calendar.fake.get() {
            let access = store::CalendarAccess {
                free_busy: true,
                holds: true,
                calendars_json: r#"["primary"]"#.into(),
                connected_at: 0,
            };
            return Ok(Some((fake.clone() as Arc<dyn CalendarSource>, access)));
        }
        let Some(account) = self.effective_account_id() else { return Ok(None) };
        if self.is_agent(&account) || self.is_archive(&account) {
            return Ok(None);
        }
        let db = self.db()?;
        let access = runtime::run(async move { Ok(db.read(store::access).await?) }).await?;
        let Some(access) = access.filter(|a| a.free_busy) else { return Ok(None) };
        let tokens = self.token_source(&account)?;
        let http = HttpClient::new(tokens, Arc::new(RateLimiter::new(60, 10)), RetryPolicy::default())?;
        Ok(Some((Arc::new(Google(GoogleCalendar::new(http))), access)))
    }

    /// The user's time zone and hours: their facts, else the Mac's zone and 9 to 5.
    async fn calendar_rules(&self) -> (Tz, NaiveTime, NaiveTime) {
        let facts = self.list_facts(vec![crate::facts::FactStatus::Accepted]).await.unwrap_or_default();
        let fact = |label: &str| {
            facts
                .iter()
                .find(|f| f.category == "availability" && f.label.eq_ignore_ascii_case(label) && !f.overridden)
                .map(|f| f.value.clone())
        };
        let zone = fact("Time zone")
            .as_deref()
            .and_then(parse_zone)
            .or_else(|| iana_time_zone::get_timezone().ok().and_then(|z| Tz::from_str(&z).ok()))
            .unwrap_or(Tz::UTC);
        let nine_to_five = (
            NaiveTime::from_hms_opt(9, 0, 0).unwrap_or_default(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap_or_default(),
        );
        let (start, end) = fact("Usual hours").as_deref().and_then(parse_hours).unwrap_or(nine_to_five);
        (zone, start, end)
    }

    /// Free slots of `minutes` over the next `days` (clamped to 15–240
    /// minutes and 1–21 days); `None` when the calendar is not connected.
    pub(crate) async fn free_slots(&self, minutes: u32, days: u32) -> Result<Option<CalendarSlots>, CoreError> {
        let Some((source, access)) = self.calendar_source().await? else { return Ok(None) };
        let (zone, day_start, day_end) = self.calendar_rules().await;
        let now = Utc::now();
        let from = now + Duration::hours(NOTICE_HOURS);
        let to = now + Duration::days(i64::from(days.clamp(1, 21)));
        let calendars: Vec<String> = serde_json::from_str(&access.calendars_json).unwrap_or_default();
        let busy = source.busy(from, to, &calendars).await?;
        let request = SlotRequest { day_start, day_end, ..SlotRequest::new(from, to, minutes.clamp(15, 240), zone) };
        let slots = provider_gcal::free_slots(&busy, &request)
            .into_iter()
            .map(|s| CalendarSlot {
                start_ms: s.start.timestamp_millis(),
                end_ms: s.end.timestamp_millis(),
                label: s.label(zone),
            })
            .collect();
        Ok(Some(CalendarSlots { time_zone: zone.name().to_owned(), slots }))
    }

    /// Hold `minutes` from `start` for `title`: checked, made, recorded.
    pub(crate) async fn hold_time(
        &self,
        start: DateTime<Utc>,
        minutes: u32,
        title: &str,
        thread_id: Option<String>,
    ) -> Result<CalendarHoldInfo, CoreError> {
        let invalid = |m: &str| CoreError::new(ErrorKind::InvalidInput, m);
        if !(15..=MAX_HOLD_MINUTES).contains(&minutes) {
            return Err(invalid("a hold is between 15 minutes and 4 hours"));
        }
        let now = Utc::now();
        if start < now || start > now + Duration::days(MAX_HOLD_DAYS) {
            return Err(invalid("a hold starts in the next 60 days"));
        }
        let title: String = crate::guide::clean_text(title).chars().take(100).collect();
        if title.is_empty() {
            return Err(invalid("a hold needs a title"));
        }
        let Some((source, access)) = self.calendar_source().await? else {
            return Err(CoreError::new(ErrorKind::PermissionDenied, NOT_CONNECTED));
        };
        if !access.holds {
            return Err(CoreError::new(
                ErrorKind::PermissionDenied,
                "Kaluta may read this calendar but not hold times on it.",
            ));
        }
        let (zone, ..) = self.calendar_rules().await;
        let end = start + Duration::minutes(i64::from(minutes));
        let hold = Hold {
            summary: format!("Hold: {title}"),
            description: "Held by Kaluta while waiting for an answer. Delete it to release the time.".into(),
            start,
            end,
            time_zone: zone.name().to_owned(),
        };
        let event_id = source.hold(&hold).await?;
        let db = self.db()?;
        let (summary, thread, at) = (hold.summary.clone(), thread_id.clone(), now.timestamp_millis());
        let (s, e) = (start.timestamp_millis(), end.timestamp_millis());
        let id = runtime::run(async move {
            Ok(db.write(move |tx| store::add_hold(tx, &event_id, &summary, s, e, thread.as_deref(), at)).await?)
        })
        .await?;
        Ok(CalendarHoldInfo {
            id,
            title: hold.summary,
            start_ms: s,
            end_ms: e,
            label: Slot { start, end }.label(zone),
            thread_id,
            released: false,
        })
    }
}

#[uniffi::export]
impl Core {
    /// Whether the open account's calendar is connected (spec §14.9).
    pub async fn calendar_status(&self) -> Result<CalendarStatus, CoreError> {
        let connected = self.calendar_source().await?;
        let (zone, start, end) = self.calendar_rules().await;
        Ok(CalendarStatus {
            connected: connected.is_some(),
            can_hold: connected.is_some_and(|(_, a)| a.holds),
            time_zone: zone.name().to_owned(),
            hours: hours_label(start, end),
        })
    }

    /// Free times to offer for a meeting of `minutes` over the next `days`.
    pub async fn calendar_free_slots(&self, minutes: u32, days: u32) -> Result<CalendarSlots, CoreError> {
        self.free_slots(minutes, days).await?.ok_or_else(|| CoreError::new(ErrorKind::PermissionDenied, NOT_CONNECTED))
    }

    /// Hold a time offered in a reply (Hold these, ⇧⌘H).
    pub async fn calendar_hold(
        &self,
        start_ms: i64,
        minutes: u32,
        title: String,
        thread_id: Option<String>,
    ) -> Result<CalendarHoldInfo, CoreError> {
        let start = DateTime::from_timestamp_millis(start_ms)
            .ok_or_else(|| CoreError::new(ErrorKind::InvalidInput, "not a time"))?;
        self.hold_time(start, minutes, &title, thread_id).await
    }

    /// Release a hold: the event is deleted from the calendar.
    pub async fn calendar_release(&self, hold_id: i64) -> Result<(), CoreError> {
        let db = self.db()?;
        let row = runtime::run(async move { Ok(db.read(move |c| store::hold(c, hold_id)).await?) })
            .await?
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "no such hold"))?;
        if row.released_at.is_some() {
            return Ok(());
        }
        let Some((source, _)) = self.calendar_source().await? else {
            return Err(CoreError::new(ErrorKind::PermissionDenied, NOT_CONNECTED));
        };
        source.release(&row.event_id).await?;
        let db = self.db()?;
        let now = Utc::now().timestamp_millis();
        runtime::run(async move { Ok(db.write(move |tx| store::release_hold(tx, hold_id, now)).await?) }).await?;
        Ok(())
    }

    /// The holds not released that have not ended, soonest first.
    pub async fn calendar_holds(&self) -> Result<Vec<CalendarHoldInfo>, CoreError> {
        let (zone, ..) = self.calendar_rules().await;
        let db = self.db()?;
        let now = Utc::now().timestamp_millis();
        let rows = runtime::run(async move { Ok(db.read(move |c| store::active_holds(c, now)).await?) }).await?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let (start, end) =
                    (DateTime::from_timestamp_millis(r.start_at)?, DateTime::from_timestamp_millis(r.end_at)?);
                Some(CalendarHoldInfo {
                    id: r.id,
                    title: r.title,
                    start_ms: r.start_at,
                    end_ms: r.end_at,
                    label: Slot { start, end }.label(zone),
                    thread_id: r.thread_id,
                    released: r.released_at.is_some(),
                })
            })
            .collect())
    }

    /// Stop using the open account's calendar. The sign-in keeps its
    /// grant; removing Kaluta's access in the Google account revokes it.
    pub async fn disconnect_calendar(&self) -> Result<(), CoreError> {
        let db = self.db()?;
        runtime::run(async move { Ok(db.write(store::clear_access).await?) }).await
    }

    /// Development and UI tests: a fake calendar (busy 19:00–20:00 UTC each
    /// day), connected for every account.
    pub fn debug_use_fake_calendar(&self) {
        let _ = self.calendar.fake.set(Arc::new(FakeCalendar::default()));
    }
}

#[cfg(test)]
impl Core {
    pub(crate) fn fake_calendar(&self) -> Option<Arc<FakeCalendar>> {
        self.calendar.fake.get().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn usual_hours_read_as_people_write_them() {
        assert_eq!(parse_hours("9–5"), Some((t(9, 0), t(17, 0))));
        assert_eq!(parse_hours("9am - 6pm"), Some((t(9, 0), t(18, 0))));
        assert_eq!(parse_hours("09:00–17:30"), Some((t(9, 0), t(17, 30))));
        assert_eq!(parse_hours("10-6pm"), Some((t(10, 0), t(18, 0))));
        assert_eq!(parse_hours("1-5pm"), Some((t(13, 0), t(17, 0))), "an afternoon");
        assert_eq!(parse_hours("8.30am to 4.30pm"), Some((t(8, 30), t(16, 30))));
        assert_eq!(parse_hours("8:30am to 4:30pm"), Some((t(8, 30), t(16, 30))));
        assert_eq!(parse_hours("whenever"), None);
        assert_eq!(hours_label(t(9, 0), t(17, 30)), "9 am–5:30 pm");
    }

    #[test]
    fn slots_are_offered_once_connected_and_a_hold_is_busy_until_released() {
        use crate::analysis_run::tests::rt;
        let s = crate::guide::tests::demo("calendar-core");
        let core = &s.1;
        let status = rt(core.calendar_status()).unwrap();
        assert!(!status.connected && !status.can_hold, "not until the user connects it");
        let err = rt(core.calendar_free_slots(30, 10)).unwrap_err();
        assert_eq!((err.kind(), err.to_string()), (ErrorKind::PermissionDenied, NOT_CONNECTED.to_owned()));

        core.debug_use_fake_calendar();
        assert!(rt(core.calendar_status()).unwrap().can_hold);
        let slots = rt(core.calendar_free_slots(30, 10)).unwrap();
        assert!(!slots.slots.is_empty() && slots.slots.len() <= 8, "{slots:?}");
        let soon = Utc::now() + Duration::hours(NOTICE_HOURS);
        for slot in &slots.slots {
            let (start, end) = (
                DateTime::from_timestamp_millis(slot.start_ms).unwrap(),
                DateTime::from_timestamp_millis(slot.end_ms).unwrap(),
            );
            let busy = start.date_naive().and_hms_opt(19, 0, 0).unwrap().and_utc();
            assert!(!(start < busy + Duration::hours(1) && end > busy), "around the fake's busy hour: {slot:?}");
            assert!(start >= soon - Duration::minutes(1) && end - start == Duration::minutes(30));
        }

        // Held: busy now, recorded, and released once.
        let first = slots.slots[0].clone();
        let held = rt(core.calendar_hold(first.start_ms, 30, "Launch review".into(), Some("t1".into()))).unwrap();
        assert_eq!((held.title.as_str(), held.label.as_str()), ("Hold: Launch review", first.label.as_str()));
        assert_eq!(core.fake_calendar().unwrap().holds().len(), 1);
        let again = rt(core.calendar_free_slots(30, 10)).unwrap();
        assert!(!again.slots.iter().any(|s| s.start_ms == first.start_ms), "the held time is not offered again");
        assert_eq!(rt(core.calendar_holds()).unwrap().iter().map(|h| h.id).collect::<Vec<_>>(), [held.id]);
        rt(core.calendar_release(held.id)).unwrap();
        rt(core.calendar_release(held.id)).unwrap();
        assert!(core.fake_calendar().unwrap().holds().is_empty());
        assert!(rt(core.calendar_holds()).unwrap().is_empty());

        // Never a week, the past, months ahead, or nothing.
        let now = Utc::now().timestamp_millis();
        let day = 24 * 60 * 60 * 1000;
        for (start, minutes, title) in [
            (now + day, 7 * 24 * 60, "Block"),
            (now - day, 30, "Past"),
            (now + 90 * day, 30, "Far"),
            (now + day, 30, "  "),
        ] {
            let err = rt(core.calendar_hold(start, minutes, title.into(), None)).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "{title}");
        }
        assert!(core.fake_calendar().unwrap().holds().is_empty());
    }

    #[test]
    fn a_zone_is_found_in_what_was_written() {
        assert_eq!(parse_zone("America/Los_Angeles"), Some(chrono_tz::America::Los_Angeles));
        assert_eq!(parse_zone("Pacific (America/Los_Angeles)"), Some(chrono_tz::America::Los_Angeles));
        assert_eq!(parse_zone("Pacific time"), None);
    }
}
