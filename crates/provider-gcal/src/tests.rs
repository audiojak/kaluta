use std::sync::Arc;
use std::time::Duration;

use chrono::{TimeZone, Utc};
use provider_api::token::StaticToken;
use provider_api::{HttpClient, ProviderError, RateLimiter, RetryPolicy};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

async fn calendar(server: &MockServer) -> GoogleCalendar {
    let retry =
        RetryPolicy { max_attempts: 1, base_delay: Duration::from_millis(1), max_delay: Duration::from_millis(5) };
    let http =
        HttpClient::new(Arc::new(StaticToken("tok".into())), Arc::new(RateLimiter::new(1000, 0)), retry).unwrap();
    GoogleCalendar::new(http).with_base(server.uri())
}

fn at(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 2, h, m, 0).unwrap()
}

#[tokio::test]
async fn busy_times_come_back_merged_and_in_order() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/freeBusy"))
        .and(header("authorization", "Bearer tok"))
        .and(body_partial_json(json!({ "items": [{ "id": "primary" }, { "id": "team@example.com" }] })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "kind": "calendar#freeBusy",
            "calendars": {
                "primary": { "busy": [
                    { "start": "2026-11-02T17:00:00Z", "end": "2026-11-02T18:00:00Z" },
                    { "start": "2026-11-02T09:00:00-08:00", "end": "2026-11-02T09:30:00-08:00" }
                ] },
                "team@example.com": { "busy": [
                    { "start": "2026-11-02T17:30:00Z", "end": "2026-11-02T19:00:00Z" }
                ] }
            }
        })))
        .expect(1)
        .mount(&server)
        .await;
    let busy = calendar(&server)
        .await
        .busy(at(0, 0), at(23, 0), &["primary".into(), "team@example.com".into()])
        .await
        .unwrap();
    // 9:00–9:30 Pacific is 17:00–17:30 UTC: all three join.
    assert_eq!(busy, [Busy { start: at(17, 0), end: at(19, 0) }]);
}

#[tokio::test]
async fn a_calendar_it_cannot_read_is_an_error_not_a_free_week() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/freeBusy"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "calendars": { "primary": { "busy": [], "errors": [{ "domain": "global", "reason": "notFound" }] } }
        })))
        .mount(&server)
        .await;
    let err = calendar(&server).await.busy(at(0, 0), at(23, 0), &[]).await.unwrap_err();
    assert!(matches!(&err, ProviderError::Forbidden(m) if m.contains("notFound")), "{err:?}");
}

#[tokio::test]
async fn a_hold_is_tentative_private_and_invites_no_one() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/calendars/primary/events"))
        .and(body_partial_json(json!({
            "summary": "Hold: Launch review",
            "status": "tentative",
            "visibility": "private",
            "start": { "dateTime": "2026-11-02T17:00:00+00:00", "timeZone": "America/Los_Angeles" },
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": "abc123def", "status": "tentative" })))
        .expect(1)
        .mount(&server)
        .await;
    let hold = Hold {
        summary: "Hold: Launch review".into(),
        description: "Offered to Sam in a reply.".into(),
        start: at(17, 0),
        end: at(17, 30),
        time_zone: "America/Los_Angeles".into(),
    };
    let cal = calendar(&server).await;
    assert_eq!(cal.hold(&hold).await.unwrap(), "abc123def");
    let sent: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert!(sent.get("attendees").is_none(), "a hold never invites: {sent}");
    assert_eq!(sent["reminders"]["useDefault"], false);
}

#[tokio::test]
async fn a_hold_is_released_once_and_a_gone_one_is_released_already() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/calendars/primary/events/abc123def"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/calendars/primary/events/gone1"))
        .respond_with(
            ResponseTemplate::new(410)
                .set_body_json(json!({ "error": { "code": 410, "message": "Resource has been deleted" } })),
        )
        .mount(&server)
        .await;
    let cal = calendar(&server).await;
    cal.release("abc123def").await.unwrap();
    cal.release("gone1").await.unwrap();
    // Not an event id: refused before any request.
    assert!(matches!(cal.release("../calendars").await, Err(ProviderError::Invalid(_))));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[test]
fn the_scopes_are_the_least_that_do_the_job() {
    assert!(FREEBUSY_SCOPE.ends_with("/calendar.freebusy"));
    assert!(EVENTS_SCOPE.ends_with("/calendar.events.owned"));
}
