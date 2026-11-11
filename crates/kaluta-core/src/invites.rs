//! Calendar invitations in mail (spec §14.9, amended 2026-11-04): an
//! email carrying an iCalendar `METHOD:REQUEST` part can be answered —
//! Accept, Maybe, Decline — and the answer goes to the user's Google
//! Calendar, as Gmail's buttons do, so it shows there and the organiser
//! is told. The invitation is read from the conversation's newest
//! calendar part (fetched like any attachment); the event is found on the
//! primary calendar by its `UID`.

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use provider_gcal::Rsvp;

use crate::{Core, CoreError, ErrorKind, runtime};

/// The user's answer to an invitation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum InviteResponse {
    /// Not answered yet.
    NeedsAction,
    Accepted,
    /// Maybe.
    Tentative,
    Declined,
}

impl From<Rsvp> for InviteResponse {
    fn from(r: Rsvp) -> Self {
        match r {
            Rsvp::NeedsAction => Self::NeedsAction,
            Rsvp::Accepted => Self::Accepted,
            Rsvp::Tentative => Self::Tentative,
            Rsvp::Declined => Self::Declined,
        }
    }
}

impl From<InviteResponse> for Rsvp {
    fn from(r: InviteResponse) -> Self {
        match r {
            InviteResponse::NeedsAction => Self::NeedsAction,
            InviteResponse::Accepted => Self::Accepted,
            InviteResponse::Tentative => Self::Tentative,
            InviteResponse::Declined => Self::Declined,
        }
    }
}

/// Whether the invitation can be answered from Kaluta, and if not, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum InviteState {
    /// On the user's calendar: Accept, Maybe and Decline go there.
    Answerable,
    /// The calendar is not connected (Settings › Accounts).
    NotConnected,
    /// Connected for free/busy only: answering needs event access.
    NoEventAccess,
    /// Google has not put the event on the user's calendar.
    NotOnCalendar,
    /// The user organises it, or is not among its guests.
    NotAGuest,
    /// The organiser cancelled it.
    Cancelled,
    /// It is over.
    Past,
}

/// An invitation in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CalendarInvite {
    pub thread_id: String,
    pub title: String,
    /// Milliseconds since the epoch; an all-day event starts at UTC
    /// midnight of its day.
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    pub all_day: bool,
    /// The organiser's name, else their address.
    pub organizer: Option<String>,
    pub location: Option<String>,
    /// The user's answer on their calendar, when it is there.
    pub response: Option<InviteResponse>,
    pub state: InviteState,
}

/// What an iCalendar part says, as far as answering it goes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Ics {
    /// `REQUEST`, `CANCEL`, `REPLY`, `PUBLISH`; empty when absent.
    pub method: String,
    pub uid: String,
    pub summary: String,
    pub start: Option<(DateTime<Utc>, bool)>,
    pub end: Option<DateTime<Utc>>,
    pub organizer: Option<String>,
    pub location: Option<String>,
    pub status: Option<String>,
}

/// iCalendar text unescaped: `\n`, `\,`, `\;`, `\\`.
fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A `DTSTART`/`DTEND` value: UTC (`…Z`), in a `TZID`, a floating local
/// time (taken in the Mac's zone, as the event's own zone is unknown), or
/// a date (all day). The flag says all day.
fn when(params: &[(String, String)], value: &str) -> Option<(DateTime<Utc>, bool)> {
    let value = value.trim();
    let is_date = params.iter().any(|(k, v)| k == "VALUE" && v.eq_ignore_ascii_case("DATE")) || value.len() == 8;
    if is_date {
        let d = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some((d.and_hms_opt(0, 0, 0)?.and_utc(), true));
    }
    if let Some(utc) = value.strip_suffix('Z') {
        let t = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some((t.and_utc(), false));
    }
    let t = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    let zone = params.iter().find(|(k, _)| k == "TZID").map(|(_, v)| v.trim_matches('"'));
    let local = match zone.and_then(|z| z.parse::<Tz>().ok()) {
        Some(tz) => tz.from_local_datetime(&t).earliest()?.with_timezone(&Utc),
        None => chrono::Local.from_local_datetime(&t).earliest()?.with_timezone(&Utc),
    };
    Some((local, false))
}

/// The first event of an iCalendar text. Lines are unfolded first (a
/// line starting with a space or tab continues the one before).
pub(crate) fn parse_ics(text: &str) -> Option<Ics> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        match (line.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => lines.push(line.to_owned()),
        }
    }
    let mut ics = Ics::default();
    let (mut in_event, mut seen_event, mut depth) = (false, false, 0usize);
    for line in &lines {
        let Some((head, value)) = line.split_once(':') else { continue };
        let mut parts = head.split(';');
        let name = parts.next().unwrap_or_default().to_ascii_uppercase();
        let params: Vec<(String, String)> =
            parts.filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_ascii_uppercase(), v.to_owned()))).collect();
        match name.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VEVENT") && !seen_event => in_event = true,
            "BEGIN" if in_event => depth += 1,
            "END" if in_event && depth > 0 => depth -= 1,
            "END" if value.eq_ignore_ascii_case("VEVENT") && in_event => {
                in_event = false;
                seen_event = true;
            }
            "METHOD" if !in_event => ics.method = value.trim().to_ascii_uppercase(),
            _ if !in_event || depth > 0 => {}
            "UID" => ics.uid = value.trim().to_owned(),
            "SUMMARY" => ics.summary = unescape(value).trim().to_owned(),
            "LOCATION" => ics.location = Some(unescape(value).trim().to_owned()).filter(|l| !l.is_empty()),
            "STATUS" => ics.status = Some(value.trim().to_ascii_uppercase()),
            "DTSTART" => ics.start = when(&params, value),
            "DTEND" => ics.end = when(&params, value).map(|(t, _)| t),
            "ORGANIZER" => {
                let name = params.iter().find(|(k, _)| k == "CN").map(|(_, v)| unescape(v.trim_matches('"')));
                let email = value.trim().trim_start_matches("mailto:").trim_start_matches("MAILTO:").to_owned();
                ics.organizer = name.filter(|n| !n.trim().is_empty()).or(Some(email).filter(|e| !e.is_empty()));
            }
            _ => {}
        }
    }
    (seen_event && !ics.uid.is_empty()).then_some(ics)
}

/// The conversation's calendar parts, newest message first, the
/// `text/calendar` alternative before an `.ics` file of the same message.
fn calendar_parts(conn: &mail_store::Connection, thread_id: &str) -> mail_store::StoreResult<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT a.id FROM attachments a JOIN messages m ON m.id = a.message_id JOIN threads t ON t.id = m.thread_id
         WHERE t.gmail_id = ?1
           AND (lower(a.mime_type) IN ('text/calendar', 'application/ics') OR lower(a.filename) LIKE '%.ics')
         ORDER BY m.internal_date DESC, m.id DESC, lower(a.mime_type) = 'text/calendar' DESC, a.id
         LIMIT 4",
    )?;
    let ids = stmt.query_map([thread_id], |r| r.get(0))?.collect::<Result<Vec<i64>, _>>()?;
    Ok(ids)
}

impl Core {
    /// The newest invitation the conversation carries, read from its file.
    async fn read_invite(&self, thread_id: &str) -> Result<Option<Ics>, CoreError> {
        let db = self.db()?;
        let thread = thread_id.to_owned();
        let parts = runtime::run(async move { Ok(db.read(move |c| calendar_parts(c, &thread)).await?) }).await?;
        let cache = self.attachment_cache_dir()?;
        let service = self.sync_service();
        for id in parts {
            let (db, cache, service) = (self.db()?, cache.clone(), service.clone());
            let text = runtime::run(async move {
                let provider = service.as_ref().map(|s| s.engine().provider());
                let file = mail_sync::attachment_file(&db, provider, &cache, id).await?;
                Ok(std::fs::read(&file.path).map(|b| String::from_utf8_lossy(&b).into_owned()).ok())
            })
            .await;
            match text {
                Ok(Some(text)) => {
                    if let Some(ics) = parse_ics(&text) {
                        return Ok(Some(ics));
                    }
                }
                Ok(None) => {}
                Err(e) => tracing::debug!(error = %e, "an invitation's file could not be read"),
            }
        }
        Ok(None)
    }

    /// The invitation and where it stands on the user's calendar.
    async fn invite_status(&self, thread_id: &str, ics: Ics) -> Result<(CalendarInvite, Option<String>), CoreError> {
        let Some((start, all_day)) = ics.start else {
            return Err(CoreError::new(ErrorKind::InvalidInput, "the invitation has no start"));
        };
        let end = ics.end.filter(|e| *e > start);
        let mut invite = CalendarInvite {
            thread_id: thread_id.to_owned(),
            title: if ics.summary.is_empty() { "(no title)".into() } else { ics.summary.clone() },
            start_ms: start.timestamp_millis(),
            end_ms: end.map(|e| e.timestamp_millis()),
            all_day,
            organizer: ics.organizer.clone(),
            location: ics.location.clone(),
            response: None,
            state: InviteState::Answerable,
        };
        let over = end.unwrap_or(start + chrono::Duration::days(i64::from(all_day)));
        if ics.method == "CANCEL" || ics.status.as_deref() == Some("CANCELLED") {
            invite.state = InviteState::Cancelled;
            return Ok((invite, None));
        }
        if over < Utc::now() {
            invite.state = InviteState::Past;
        }
        let Some((source, access)) = self.calendar_source().await? else {
            invite.state =
                if invite.state == InviteState::Past { InviteState::Past } else { InviteState::NotConnected };
            return Ok((invite, None));
        };
        if !access.holds {
            if invite.state != InviteState::Past {
                invite.state = InviteState::NoEventAccess;
            }
            return Ok((invite, None));
        }
        let found = source.find_invite(&ics.uid).await?;
        let event_id = match found {
            None => {
                if invite.state != InviteState::Past {
                    invite.state = InviteState::NotOnCalendar;
                }
                None
            }
            Some(e) => {
                invite.response = e.response.map(Into::into);
                if e.response.is_none() && invite.state != InviteState::Past {
                    invite.state = InviteState::NotAGuest;
                }
                Some(e.id)
            }
        };
        Ok((invite, event_id))
    }
}

#[uniffi::export]
impl Core {
    /// The invitation a conversation carries, if any, and the user's answer
    /// on their calendar (spec §14.9). `None` for a conversation without
    /// one, and for replies and announcements (`METHOD:REPLY`, `PUBLISH`),
    /// which have nothing to answer.
    pub async fn calendar_invite(&self, thread_id: String) -> Result<Option<CalendarInvite>, CoreError> {
        let Some(ics) = self.read_invite(&thread_id).await? else { return Ok(None) };
        if !matches!(ics.method.as_str(), "REQUEST" | "CANCEL" | "") {
            return Ok(None);
        }
        match self.invite_status(&thread_id, ics).await {
            Ok((invite, _)) => Ok(Some(invite)),
            Err(e) if e.kind() == ErrorKind::InvalidInput => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Answer the conversation's invitation on the user's Google Calendar;
    /// the organiser is told. The invitation as it now stands.
    pub async fn respond_to_invite(
        &self,
        thread_id: String,
        response: InviteResponse,
    ) -> Result<CalendarInvite, CoreError> {
        if response == InviteResponse::NeedsAction {
            return Err(CoreError::new(ErrorKind::InvalidInput, "answer with Accept, Maybe or Decline"));
        }
        let ics = self
            .read_invite(&thread_id)
            .await?
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "this conversation has no invitation"))?;
        let (mut invite, event_id) = self.invite_status(&thread_id, ics).await?;
        let refuse = |m: &str| Err(CoreError::new(ErrorKind::PermissionDenied, m));
        match invite.state {
            InviteState::Answerable => {}
            InviteState::NotConnected => return refuse("Connect Google Calendar in Settings › Accounts to answer."),
            InviteState::NoEventAccess => {
                return refuse("Kaluta may read this calendar but not change its events; connect it again with holds.");
            }
            InviteState::NotOnCalendar => return refuse("This event is not on your Google Calendar."),
            InviteState::NotAGuest => return refuse("You are not among this event's guests."),
            InviteState::Cancelled => return refuse("The organiser cancelled this event."),
            InviteState::Past => return refuse("This event is over."),
        }
        let (Some(event_id), Some((source, _))) = (event_id, self.calendar_source().await?) else {
            return refuse("This event is not on your Google Calendar.");
        };
        let answered = source.respond(&event_id, response.into()).await?;
        invite.response = Some(answered.into());
        Ok(invite)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CALCOM: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:calcom/ics\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\n\
UID:w6rbu@Cal.com\r\nSUMMARY:Speaker Prep between Alex and\r\n  Sam\r\nDTSTAMP:20261022T195700Z\r\n\
DTSTART:20261026T174500Z\r\nDTEND:20261026T180000Z\r\nLOCATION:Zoom\\, room 2\r\n\
ORGANIZER;CN=\"Alex Rivera\":mailto:alex@example.com\r\nBEGIN:VALARM\r\nSUMMARY:Reminder\r\nEND:VALARM\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn an_invitation_reads_its_event() {
        let ics = parse_ics(CALCOM).unwrap();
        assert_eq!(ics.method, "REQUEST");
        assert_eq!(ics.uid, "w6rbu@Cal.com");
        assert_eq!(ics.summary, "Speaker Prep between Alex and Sam", "folded lines joined, the alarm's left out");
        assert_eq!(ics.start, Some((Utc.with_ymd_and_hms(2026, 10, 26, 17, 45, 0).unwrap(), false)));
        assert_eq!(ics.end, Some(Utc.with_ymd_and_hms(2026, 10, 26, 18, 0, 0).unwrap()));
        assert_eq!(ics.location.as_deref(), Some("Zoom, room 2"));
        assert_eq!(ics.organizer.as_deref(), Some("Alex Rivera"));
    }

    /// The demo's meetup carries an invitation: unanswerable until the
    /// calendar is connected, then answered on it.
    #[test]
    fn the_demo_invitation_is_answered_on_the_calendar() {
        use crate::analysis_run::tests::rt;
        let s = crate::guide::tests::demo("invites-core");
        let core = &s.1;
        rt(core.debug_seed_workflow_demo(0)).unwrap();
        assert_eq!(rt(core.calendar_invite("demowf-renewal".into())).unwrap(), None, "no invitation in it");
        let invite = rt(core.calendar_invite("demowf-meetup".into())).unwrap().expect("the meetup invites");
        assert_eq!(invite.title, "Founders Meetup on the 23rd");
        assert_eq!(invite.state, InviteState::NotConnected);
        assert_eq!(invite.location.as_deref(), Some("Example Hall"));
        assert_eq!(invite.end_ms.map(|e| e - invite.start_ms), Some(2 * 3_600_000));
        let refused = rt(core.respond_to_invite("demowf-meetup".into(), InviteResponse::Accepted)).unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::PermissionDenied);

        core.debug_use_fake_calendar();
        let invite = rt(core.calendar_invite("demowf-meetup".into())).unwrap().unwrap();
        assert_eq!((invite.state, invite.response), (InviteState::Answerable, Some(InviteResponse::NeedsAction)));
        let answered = rt(core.respond_to_invite("demowf-meetup".into(), InviteResponse::Tentative)).unwrap();
        assert_eq!(answered.response, Some(InviteResponse::Tentative));
        let again = rt(core.calendar_invite("demowf-meetup".into())).unwrap().unwrap();
        assert_eq!(again.response, Some(InviteResponse::Tentative), "as the calendar now has it");
        assert!(rt(core.respond_to_invite("demowf-meetup".into(), InviteResponse::NeedsAction)).is_err());
    }

    /// A synced invitation's file is not stored with the message: it is
    /// downloaded from the server the first time the bar is shown, as
    /// Google Calendar sends it (an inline `text/calendar` part and an
    /// `invite.ics` file of the same).
    #[test]
    fn a_synced_invitation_is_downloaded_and_read() {
        use std::sync::Arc;

        use futures::executor::block_on;
        use mail_domain::{EmailAddress, LabelId, MessageId, ThreadId};
        use provider_api::fake::FakeProvider;
        use provider_api::{FetchedAttachment, FetchedBody, FetchedMessage};

        struct Quiet;
        impl crate::EventListener for Quiet {
            fn on_event(&self, _: Option<String>, _: crate::CoreEvent) {}
        }
        let dir = std::env::temp_dir().join(format!("kaluta-core-invite-fetch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let core = crate::Core::new(
            crate::CoreConfig { data_dir: dir.to_string_lossy().into_owned(), log_dir: None },
            Arc::new(crate::secrets::MemorySecrets::default()),
            Arc::new(Quiet),
        )
        .unwrap();
        let fake = Arc::new(FakeProvider::new("me@example.com", 1_790_000_000_000, 50));
        let part = |part: &str, id: &str, mime: &str| FetchedAttachment {
            part_id: Some(part.into()),
            attachment_id: Some(id.into()),
            filename: "invite.ics".into(),
            mime_type: mime.into(),
            size: CALCOM.len() as u64,
            content_id: None,
            is_inline: false,
            data: None,
        };
        fake.seed(FetchedMessage {
            id: MessageId::new("inv1"),
            thread_id: ThreadId::new("t-inv"),
            label_ids: vec![LabelId::new("INBOX")],
            internal_date: 1_790_000_000_000,
            from: Some(EmailAddress::new(None, "alex@example.com")),
            subject: "Updated invitation: Speaker Prep".into(),
            body: Some(FetchedBody {
                text: Some("Invitation".into()),
                html: None,
                attachments: vec![part("0.2", "cal-inline", "text/calendar"), part("1", "cal-file", "application/ics")],
            }),
            ..Default::default()
        });
        let future =
            CALCOM.replace("20261026T174500Z", "20991026T174500Z").replace("20261026T180000Z", "20991026T180000Z");
        fake.set_attachment_bytes("cal-inline", future.clone().into_bytes());
        fake.set_attachment_bytes("cal-file", future.into_bytes());
        block_on(core.store_for("acct")).unwrap();
        crate::registry::SCOPED_ACCOUNT.sync_scope("acct".to_owned(), || core.start_sync_with(fake.clone())).unwrap();
        block_on(core.clone().set_current_account("acct".into())).unwrap();
        for _ in 0..200 {
            if block_on(core.list_threads("INBOX".into(), None, 10)).map(|p| p.rows.len()).unwrap_or(0) == 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        let invite = block_on(core.calendar_invite("t-inv".into())).unwrap().expect("the invitation is read");
        assert_eq!(invite.title, "Speaker Prep between Alex and Sam");
        assert_eq!(invite.state, InviteState::NotConnected);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zones_dates_and_cancellations() {
        let google = "BEGIN:VCALENDAR\nMETHOD:CANCEL\nBEGIN:VEVENT\nDTSTART;TZID=America/Los_Angeles:20261104T100000\n\
DTEND;TZID=America/Los_Angeles:20261104T103000\nUID:g1@google.com\nORGANIZER:mailto:sam@example.com\n\
STATUS:CANCELLED\nEND:VEVENT\nEND:VCALENDAR\n";
        let ics = parse_ics(google).unwrap();
        assert_eq!(ics.method, "CANCEL");
        assert_eq!(ics.start, Some((Utc.with_ymd_and_hms(2026, 11, 4, 18, 0, 0).unwrap(), false)), "PST is UTC−8");
        assert_eq!(ics.organizer.as_deref(), Some("sam@example.com"));
        assert_eq!(ics.status.as_deref(), Some("CANCELLED"));
        let day = "BEGIN:VEVENT\nUID:d1\nDTSTART;VALUE=DATE:20261225\nEND:VEVENT\n";
        assert_eq!(parse_ics(day).unwrap().start, Some((Utc.with_ymd_and_hms(2026, 12, 25, 0, 0, 0).unwrap(), true)));
        assert!(parse_ics("BEGIN:VCALENDAR\nEND:VCALENDAR\n").is_none(), "no event");
        assert!(parse_ics("BEGIN:VEVENT\nSUMMARY:x\nEND:VEVENT\n").is_none(), "no UID");
    }
}
