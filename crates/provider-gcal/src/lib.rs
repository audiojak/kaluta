//! Google Calendar for offering times (spec §14.9, ADR 0018): free/busy
//! on the user's calendars and tentative holds on their primary one, with
//! the mail account's own token once calendar access is granted. Slots
//! are found here from busy blocks; the agent sees slots only, never what
//! the user is doing. Holds have no attendees: inviting is sending, and
//! stays out (spec §14.13).

mod slots;

use chrono::{DateTime, Utc};
use provider_api::http::ErrorHook;
use provider_api::{HttpClient, Priority, ProviderError, ProviderResult};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub use slots::{Slot, SlotRequest, free_slots};

/// The API's root.
pub const BASE: &str = "https://www.googleapis.com/calendar/v3";
/// The least scope for `freeBusy.query`: availability only, no events.
pub const FREEBUSY_SCOPE: &str = "https://www.googleapis.com/auth/calendar.freebusy";
/// The least scope for holds: events on calendars the user owns.
pub const EVENTS_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events.owned";
/// Marks Kaluta's holds among the user's events.
const HOLD_MARK: &str = "kaluta-hold";

/// A time the user is busy, start inclusive, end exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Busy {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// A tentative hold: a private event with no attendees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    pub summary: String,
    pub description: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// IANA name ("Europe/Zurich"), so the event shows in the user's zone.
    pub time_zone: String,
}

/// The user's Google Calendar, through the mail account's token.
#[derive(Clone)]
pub struct GoogleCalendar {
    http: HttpClient,
    base: String,
}

impl GoogleCalendar {
    /// `http` carries the account's token; a deleted event (410) reads as
    /// not found.
    pub fn new(http: HttpClient) -> Self {
        let gone: ErrorHook =
            Arc::new(|status, _body| (status == 410).then(|| ProviderError::NotFound("the event is gone".into())));
        Self { http: http.with_error_hook(gone), base: BASE.into() }
    }

    /// Another root (a test's mock server).
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    /// The busy times on `calendars` (ids, `primary` for the user's own)
    /// between `from` and `to`, merged and in order. A calendar the API
    /// cannot read is an error, not a free week.
    pub async fn busy(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        calendars: &[String],
    ) -> ProviderResult<Vec<Busy>> {
        let ids: Vec<String> = if calendars.is_empty() { vec!["primary".into()] } else { calendars.to_vec() };
        let body = json!({
            "timeMin": from.to_rfc3339(),
            "timeMax": to.to_rfc3339(),
            "items": ids.iter().map(|id| json!({ "id": id })).collect::<Vec<_>>(),
        });
        let url = format!("{}/freeBusy", self.base);
        let answer: FreeBusy = self.http.json(1, Priority::Interactive, |c| c.post(&url).json(&body)).await?;
        let mut busy = Vec::new();
        for id in &ids {
            let Some(cal) = answer.calendars.get(id) else {
                return Err(ProviderError::Invalid(format!("no free/busy for calendar {id}")));
            };
            if let Some(e) = cal.errors.first() {
                let reason = e.get("reason").and_then(Value::as_str).unwrap_or("unknown");
                return Err(ProviderError::Forbidden(format!("calendar {id}: {reason}")));
            }
            for b in &cal.busy {
                let (Ok(start), Ok(end)) =
                    (DateTime::parse_from_rfc3339(&b.start), DateTime::parse_from_rfc3339(&b.end))
                else {
                    return Err(ProviderError::Decode(format!("busy time {} – {}", b.start, b.end)));
                };
                busy.push(Busy { start: start.with_timezone(&Utc), end: end.with_timezone(&Utc) });
            }
        }
        Ok(merge(busy))
    }

    /// Make a tentative, private hold on the primary calendar, with no
    /// attendees and no reminders. Its event id.
    pub async fn hold(&self, hold: &Hold) -> ProviderResult<String> {
        let at = |t: DateTime<Utc>| json!({ "dateTime": t.to_rfc3339(), "timeZone": hold.time_zone });
        let body = json!({
            "summary": hold.summary,
            "description": hold.description,
            "start": at(hold.start),
            "end": at(hold.end),
            "status": "tentative",
            "transparency": "opaque",
            "visibility": "private",
            "reminders": { "useDefault": false },
            "extendedProperties": { "private": { HOLD_MARK: "1" } },
        });
        let url = format!("{}/calendars/primary/events", self.base);
        let made: Created = self.http.json(1, Priority::Interactive, |c| c.post(&url).json(&body)).await?;
        if made.id.is_empty() {
            return Err(ProviderError::Invalid("the hold came back without an id".into()));
        }
        Ok(made.id)
    }

    /// Delete a hold. One already gone is released.
    pub async fn release(&self, event_id: &str) -> ProviderResult<()> {
        // Event ids are base32hex: anything else is not one of ours.
        if event_id.is_empty() || !event_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(ProviderError::Invalid(format!("not an event id: {event_id}")));
        }
        let url = format!("{}/calendars/primary/events/{event_id}", self.base);
        match self.http.empty(1, Priority::Interactive, |c| c.delete(&url)).await {
            Err(ProviderError::NotFound(_)) => Ok(()),
            other => other,
        }
    }
}

/// Busy times in order, overlapping and touching ones joined.
fn merge(mut busy: Vec<Busy>) -> Vec<Busy> {
    busy.retain(|b| b.end > b.start);
    busy.sort_by_key(|b| b.start);
    let mut out: Vec<Busy> = Vec::with_capacity(busy.len());
    for b in busy {
        match out.last_mut() {
            Some(last) if b.start <= last.end => last.end = last.end.max(b.end),
            _ => out.push(b),
        }
    }
    out
}

#[derive(Deserialize)]
struct FreeBusy {
    #[serde(default)]
    calendars: std::collections::HashMap<String, CalendarBusy>,
}

#[derive(Deserialize)]
struct CalendarBusy {
    #[serde(default)]
    busy: Vec<Period>,
    #[serde(default)]
    errors: Vec<Value>,
}

#[derive(Deserialize)]
struct Period {
    start: String,
    end: String,
}

#[derive(Deserialize)]
struct Created {
    #[serde(default)]
    id: String,
}

#[cfg(test)]
mod tests;
