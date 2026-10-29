//! Free slots from busy times (spec §14.9): within the user's hours in
//! their time zone, on a quarter-hour grid, a few a day so the offer
//! spans days, never in the past or too soon.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Timelike, Utc, Weekday};
use chrono_tz::Tz;

use crate::Busy;

/// Slots start on this grid, in minutes.
const GRID_MIN: i64 = 15;
/// After an offered slot, the next one that day starts at least this much later.
const GAP_MIN: i64 = 60;

/// What to look for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotRequest {
    /// The first moment a slot may start (now and some notice, already added).
    pub from: DateTime<Utc>,
    /// No slot ends after this.
    pub to: DateTime<Utc>,
    pub minutes: u32,
    pub tz: Tz,
    /// The user's hours, local.
    pub day_start: NaiveTime,
    pub day_end: NaiveTime,
    pub weekdays_only: bool,
    /// Slots at most, and at most a day.
    pub max: usize,
    pub per_day: usize,
}

impl SlotRequest {
    /// Weekdays, 9 to 5 local, eight slots, two a day.
    pub fn new(from: DateTime<Utc>, to: DateTime<Utc>, minutes: u32, tz: Tz) -> Self {
        Self {
            from,
            to,
            minutes,
            tz,
            day_start: NaiveTime::from_hms_opt(9, 0, 0).unwrap_or_default(),
            day_end: NaiveTime::from_hms_opt(17, 0, 0).unwrap_or_default(),
            weekdays_only: true,
            max: 8,
            per_day: 2,
        }
    }
}

/// A time the user is free for the whole meeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl Slot {
    /// "Thu Oct 30, 2–3 pm" in `tz`; "Mon Nov 2, 11:45 am–12:15 pm" across noon.
    pub fn label(&self, tz: Tz) -> String {
        let (s, e) = (self.start.with_timezone(&tz), self.end.with_timezone(&tz));
        let time = |h: u32, m: u32, half: bool| {
            let h12 = if h.is_multiple_of(12) { 12 } else { h % 12 };
            let clock = if m == 0 { h12.to_string() } else { format!("{h12}:{m:02}") };
            if half { format!("{clock} {}", if h < 12 { "am" } else { "pm" }) } else { clock }
        };
        let across = (s.hour() < 12) != (e.hour() < 12);
        format!(
            "{}, {}–{}",
            s.format("%a %b %-d"),
            time(s.hour(), s.minute(), across),
            time(e.hour(), e.minute(), true)
        )
    }
}

/// Free slots of `request.minutes` around `busy` (merged or not), in order.
pub fn free_slots(busy: &[Busy], request: &SlotRequest) -> Vec<Slot> {
    let length = Duration::minutes(i64::from(request.minutes.max(GRID_MIN as u32)));
    let mut out = Vec::new();
    if request.to <= request.from || request.max == 0 {
        return out;
    }
    let mut day = request.from.with_timezone(&request.tz).date_naive();
    let last = request.to.with_timezone(&request.tz).date_naive();
    while day <= last && out.len() < request.max {
        let weekend = matches!(day.weekday(), Weekday::Sat | Weekday::Sun);
        if !(request.weekdays_only && weekend)
            && let (Some(open), Some(close)) =
                (local(request.tz, day, request.day_start), local(request.tz, day, request.day_end))
        {
            let close = close.min(request.to);
            let mut start = round_up(open.max(request.from));
            let mut today = 0;
            while start + length <= close && today < request.per_day && out.len() < request.max {
                let end = start + length;
                match busy.iter().filter(|b| b.start < end && b.end > start).map(|b| b.end).max() {
                    Some(free_at) => start = round_up(free_at),
                    None => {
                        out.push(Slot { start, end });
                        today += 1;
                        start = round_up(end + Duration::minutes(GAP_MIN));
                    }
                }
            }
        }
        let Some(next) = day.succ_opt() else { break };
        day = next;
    }
    out
}

/// A local wall-clock time as UTC; the earlier of two in a repeated hour,
/// none in a skipped one.
fn local(tz: Tz, day: NaiveDate, at: NaiveTime) -> Option<DateTime<Utc>> {
    tz.from_local_datetime(&day.and_time(at)).earliest().map(|d| d.with_timezone(&Utc))
}

/// Up to the next quarter hour (every zone's offset is whole quarters).
fn round_up(t: DateTime<Utc>) -> DateTime<Utc> {
    let grid = GRID_MIN * 60;
    let secs = t.timestamp() + i64::from(t.timestamp_subsec_nanos() > 0);
    DateTime::from_timestamp((secs + grid - 1).div_euclid(grid) * grid, 0).unwrap_or(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::America::Los_Angeles;

    fn la(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Los_Angeles.with_ymd_and_hms(y, mo, d, h, mi, 0).earliest().unwrap().with_timezone(&Utc)
    }

    fn labels(slots: &[Slot]) -> Vec<String> {
        slots.iter().map(|s| s.label(Los_Angeles)).collect()
    }

    #[test]
    fn busy_times_leave_the_gaps_two_a_day_within_the_hours() {
        // Mon Nov 2 to the end of Wed Nov 4, 2026.
        let request = SlotRequest::new(la(2026, 11, 2, 0, 0), la(2026, 11, 5, 0, 0), 30, Los_Angeles);
        let busy = [
            Busy { start: la(2026, 11, 2, 8, 0), end: la(2026, 11, 2, 10, 10) },
            Busy { start: la(2026, 11, 3, 0, 0), end: la(2026, 11, 4, 0, 0) }, // all day Tuesday
            Busy { start: la(2026, 11, 4, 9, 0), end: la(2026, 11, 4, 16, 45) },
        ];
        let slots = free_slots(&busy, &request);
        assert_eq!(
            labels(&slots),
            ["Mon Nov 2, 10:15–10:45 am", "Mon Nov 2, 11:45 am–12:15 pm"],
            "Tuesday is taken; Wednesday has no half hour left"
        );
    }

    #[test]
    fn weekends_the_past_and_the_notice_are_left_out() {
        // Saturday at 1:07 pm: nothing before Monday 9.
        let request = SlotRequest::new(la(2026, 11, 7, 13, 7), la(2026, 11, 10, 0, 0), 60, Los_Angeles);
        let slots = free_slots(&[], &request);
        assert_eq!(labels(&slots)[0], "Mon Nov 9, 9–10 am");
        assert_eq!(slots.len(), 2, "two a day: {:?}", labels(&slots));
        assert_eq!(labels(&slots)[1], "Mon Nov 9, 11 am–12 pm", "an hour apart");
        // Mid-afternoon: the next quarter hour.
        let request = SlotRequest::new(la(2026, 11, 9, 13, 7), la(2026, 11, 9, 23, 0), 60, Los_Angeles);
        assert_eq!(labels(&free_slots(&[], &request)), ["Mon Nov 9, 1:15–2:15 pm", "Mon Nov 9, 3:15–4:15 pm"]);
        // Nothing asked for, or a range that ended: nothing.
        assert!(free_slots(&[], &SlotRequest { max: 0, ..request.clone() }).is_empty());
        assert!(free_slots(&[], &SlotRequest { to: request.from, ..request }).is_empty());
    }

    #[test]
    fn nine_oclock_stays_nine_across_the_clock_change() {
        // Daylight saving ends in Los Angeles on Sunday, Nov 1, 2026.
        let request = SlotRequest {
            per_day: 1,
            ..SlotRequest::new(la(2026, 10, 30, 0, 0), la(2026, 11, 3, 0, 0), 30, Los_Angeles)
        };
        let slots = free_slots(&[], &request);
        assert_eq!(labels(&slots), ["Fri Oct 30, 9–9:30 am", "Mon Nov 2, 9–9:30 am"]);
        assert_eq!(slots[0].start.hour(), 16, "PDT is UTC-7");
        assert_eq!(slots[1].start.hour(), 17, "PST is UTC-8");
    }

    #[test]
    fn the_cap_holds_across_days() {
        let request = SlotRequest::new(la(2026, 11, 2, 0, 0), la(2026, 11, 30, 0, 0), 30, Los_Angeles);
        let slots = free_slots(&[], &request);
        assert_eq!(slots.len(), 8);
        assert_eq!(labels(&slots)[7], "Thu Nov 5, 10:30–11 am");
    }
}
