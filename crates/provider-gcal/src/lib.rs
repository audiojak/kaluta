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

/// The user's answer to an invitation, as Google Calendar names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rsvp {
    NeedsAction,
    Accepted,
    Tentative,
    Declined,
}

impl Rsvp {
    pub fn as_api(self) -> &'static str {
        match self {
            Self::NeedsAction => "needsAction",
            Self::Accepted => "accepted",
            Self::Tentative => "tentative",
            Self::Declined => "declined",
        }
    }

    pub fn from_api(s: &str) -> Option<Self> {
        Some(match s {
            "needsAction" => Self::NeedsAction,
            "accepted" => Self::Accepted,
            "tentative" => Self::Tentative,
            "declined" => Self::Declined,
            _ => return None,
        })
    }
}

/// An invitation as it sits on the user's primary calendar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvitedEvent {
    pub id: String,
    /// The user's answer; `None` when they are not among its guests (they
    /// organise it, or it reached them through a group).
    pub response: Option<Rsvp>,
}

impl GoogleCalendar {
    /// The event an invitation's `UID` names on the primary calendar, if
    /// Google has put it there (it does for invitations to the account's
    /// address, Google's own and most others). Cancelled copies are not it.
    pub async fn find_invite(&self, ical_uid: &str) -> ProviderResult<Option<InvitedEvent>> {
        let url = format!("{}/calendars/primary/events", self.base);
        let query = [("iCalUID", ical_uid), ("showDeleted", "false"), ("maxResults", "5")];
        let list: EventList = self.http.json(1, Priority::Interactive, |c| c.get(&url).query(&query)).await?;
        Ok(list
            .items
            .into_iter()
            .find(|e| e.status.as_deref() != Some("cancelled"))
            .map(|e| InvitedEvent { response: own_response(&e.attendees), id: e.id }))
    }

    /// Answer the invitation `event_id` on the primary calendar, as
    /// Gmail's buttons do: the user's own guest entry changes and the
    /// organiser is told (`sendUpdates=all`). The other guests are sent
    /// back as they were, since a patch replaces the whole list.
    pub async fn respond(&self, event_id: &str, rsvp: Rsvp) -> ProviderResult<Rsvp> {
        if event_id.is_empty() || !event_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(ProviderError::Invalid(format!("not an event id: {event_id}")));
        }
        let url = format!("{}/calendars/primary/events/{event_id}", self.base);
        let event: Event = self.http.json(1, Priority::Interactive, |c| c.get(&url)).await?;
        let mut attendees = event.attendees;
        let mine = attendees
            .iter_mut()
            .find(|a| a.get("self").and_then(Value::as_bool) == Some(true))
            .ok_or_else(|| ProviderError::Invalid("you are not on this event's guest list".into()))?;
        mine["responseStatus"] = json!(rsvp.as_api());
        let body = json!({ "attendees": attendees });
        let query = [("sendUpdates", "all")];
        let updated: Event =
            self.http.json(1, Priority::Interactive, |c| c.patch(&url).query(&query).json(&body)).await?;
        Ok(own_response(&updated.attendees).unwrap_or(rsvp))
    }
}

fn own_response(attendees: &[Value]) -> Option<Rsvp> {
    attendees
        .iter()
        .find(|a| a.get("self").and_then(Value::as_bool) == Some(true))
        .and_then(|a| a.get("responseStatus").and_then(Value::as_str))
        .and_then(Rsvp::from_api)
}

#[derive(Deserialize)]
struct EventList {
    #[serde(default)]
    items: Vec<Event>,
}

#[derive(Deserialize)]
struct Event {
    #[serde(default)]
    id: String,
    #[serde(default)]
    status: Option<String>,
    /// Kept whole: a patch sends the list back.
    #[serde(default)]
    attendees: Vec<Value>,
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
