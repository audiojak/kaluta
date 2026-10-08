//! AgentMail's WebSocket push against a local fake (127.0.0.1 only).

use std::sync::Arc;
use std::time::Duration;

use provider_api::token::StaticToken;
use provider_api::{BackfillSource, RetryPolicy};

use super::*;
use crate::ws_fake::FakeAgentMailSocket;

fn quick() -> SocketConfig {
    SocketConfig {
        connect_timeout: Duration::from_secs(2),
        first_backoff: Duration::from_millis(20),
        max_backoff: Duration::from_millis(80),
        failures_before_rest: 3,
        rest: Duration::from_secs(60),
        ping_every: Duration::from_millis(200),
        silence_limit: Duration::from_secs(5),
        stable_after: Duration::ZERO,
    }
}

fn socket(fake: &FakeAgentMailSocket, config: SocketConfig) -> Arc<AgentMailSocket> {
    AgentMailSocket::new(&fake.url(), Arc::new(StaticToken("am_k/1".into())), config).unwrap()
}

fn push(socket: &Arc<AgentMailSocket>, inbox: &str) -> AgentMailPush {
    let tokens = Arc::new(StaticToken("am_k/1".into()));
    // Never called: these tests only wait for events.
    let provider = AgentMailProvider::for_inbox(
        tokens,
        inbox,
        inbox,
        crate::rate_limiter(),
        RetryPolicy::default(),
        "http://127.0.0.1:9",
    )
    .unwrap();
    AgentMailPush::new(Arc::new(provider), socket.clone(), inbox)
}

async fn until(what: &str, ok: impl Fn() -> bool) {
    for _ in 0..300 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

/// The wake-up every agent gets on (re)connecting, taken so a test sees
/// only what follows.
async fn take_catch_up(push: &AgentMailPush) {
    assert_eq!(push.watch(Duration::from_secs(2)).await.unwrap(), Some(true), "woken on subscribing");
}

#[tokio::test]
async fn one_socket_subscribes_every_agents_inbox_and_wakes_only_the_one_an_event_is_for() {
    let fake = FakeAgentMailSocket::start().await;
    let socket = socket(&fake, quick());
    let scout = push(&socket, "scout@agentmail.to");
    let writer = push(&socket, "writer@agentmail.to");
    take_catch_up(&scout).await;
    take_catch_up(&writer).await;
    assert_eq!(fake.connections(), 1, "one connection for the organisation");
    assert_eq!(fake.subscribed_inboxes(), ["scout@agentmail.to", "writer@agentmail.to"]);
    assert_eq!(fake.keys(), ["am_k%2F1"], "the key as the api_key parameter, encoded");
    assert_eq!(fake.authorizations(), [Some("Bearer am_k/1".to_owned())]);
    assert_eq!(socket.state(), SocketState::Connected);

    fake.message_received("writer@agentmail.to", "w1");
    assert_eq!(writer.watch(Duration::from_secs(2)).await.unwrap(), Some(true), "the writer polls");
    assert_eq!(scout.watch(Duration::from_millis(100)).await.unwrap(), Some(false), "the scout does not");
    // An event while no one waits is kept for the next wait.
    fake.message_received("scout@agentmail.to", "s1");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(scout.watch(Duration::from_secs(2)).await.unwrap(), Some(true));
    // Mail for an inbox no agent here has wakes no one.
    fake.message_received("other@agentmail.to", "o1");
    assert_eq!(writer.watch(Duration::from_millis(100)).await.unwrap(), Some(false));
}

#[tokio::test]
async fn an_agent_added_later_is_subscribed_on_the_same_connection() {
    let fake = FakeAgentMailSocket::start().await;
    let socket = socket(&fake, quick());
    let scout = push(&socket, "scout@agentmail.to");
    take_catch_up(&scout).await;
    let clerk = push(&socket, "clerk@agentmail.to");
    until("the clerk's subscription", || fake.subscribed_inboxes().len() == 2).await;
    assert_eq!(fake.connections(), 1);
    fake.message_received("clerk@agentmail.to", "c1");
    // The clerk's catch-up permit may or may not be there; an event is.
    assert_eq!(clerk.watch(Duration::from_secs(2)).await.unwrap(), Some(true));
}

#[tokio::test]
async fn more_than_ten_inboxes_go_out_ten_to_a_subscription() {
    let fake = FakeAgentMailSocket::start().await;
    let socket = socket(&fake, quick());
    let pushes: Vec<AgentMailPush> = (0..12).map(|n| push(&socket, &format!("a{n:02}@agentmail.to"))).collect();
    take_catch_up(&pushes[0]).await;
    until("both subscriptions", || fake.subscriptions().len() == 2).await;
    let sizes: Vec<usize> = fake.subscriptions().iter().map(Vec::len).collect();
    assert_eq!(sizes, [10, 2]);
}

#[tokio::test]
async fn a_dropped_connection_reconnects_subscribes_again_and_wakes_everyone() {
    let fake = FakeAgentMailSocket::start().await;
    let socket = socket(&fake, quick());
    let scout = push(&socket, "scout@agentmail.to");
    take_catch_up(&scout).await;
    fake.drop_connections();
    // Mail that arrived while it was down is found by the poll it wakes.
    assert_eq!(scout.watch(Duration::from_secs(2)).await.unwrap(), Some(true), "woken after reconnecting");
    assert_eq!(fake.connections(), 2);
    assert_eq!(fake.subscriptions().len(), 2, "subscribed again");
    fake.message_received("scout@agentmail.to", "s2");
    assert_eq!(scout.watch(Duration::from_secs(2)).await.unwrap(), Some(true));
}

#[tokio::test]
async fn repeated_failures_rest_the_socket_so_the_agents_poll() {
    let fake = FakeAgentMailSocket::start().await;
    fake.refuse(true);
    let socket = socket(&fake, quick());
    let scout = push(&socket, "scout@agentmail.to");
    let err = scout.watch(Duration::from_secs(5)).await.unwrap_err();
    assert!(err.to_string().contains("polling instead"), "{err}");
    until("the third refusal counted", || fake.refused() == 3).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fake.refused(), 3, "three tries with backoff, then rest");
    assert_eq!(socket.state(), SocketState::Resting);
    assert!(scout.watch(Duration::from_secs(5)).await.is_err(), "still resting: the push loop backs off");
}

#[tokio::test]
async fn a_server_that_drops_right_after_subscribing_is_not_hammered() {
    let fake = FakeAgentMailSocket::start().await;
    fake.close_after_subscribe(true);
    let socket = socket(&fake, SocketConfig { stable_after: Duration::from_secs(30), ..quick() });
    let scout = push(&socket, "scout@agentmail.to");
    // Each connection subscribes and is dropped at once: it backs off and,
    // after three such drops, rests while the agents poll.
    let mut wakes = 0;
    loop {
        match scout.watch(Duration::from_secs(5)).await {
            Ok(Some(true)) => wakes += 1,
            Ok(other) => panic!("neither woken nor resting: {other:?}"),
            Err(_) => break,
        }
    }
    assert_eq!(socket.state(), SocketState::Resting);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(fake.connections(), 3, "three tries with backoff, not a tight loop");
    assert!(wakes <= 3, "woken once per connection at most: {wakes}");
    assert!(scout.watch(Duration::from_millis(50)).await.is_err(), "resting: the push loop backs off");
}

#[tokio::test]
async fn a_drop_after_a_while_reconnects_after_a_pause() {
    let fake = FakeAgentMailSocket::start().await;
    let socket = socket(&fake, SocketConfig { first_backoff: Duration::from_millis(300), ..quick() });
    let scout = push(&socket, "scout@agentmail.to");
    take_catch_up(&scout).await;
    fake.drop_connections();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(fake.connections(), 1, "not back at once");
    assert_eq!(scout.watch(Duration::from_secs(2)).await.unwrap(), Some(true), "woken after reconnecting");
    assert_eq!(fake.connections(), 2);
}

#[tokio::test]
async fn an_error_answer_to_the_subscription_counts_as_a_failure() {
    let fake = FakeAgentMailSocket::start().await;
    fake.error_on_subscribe(true);
    let socket = socket(&fake, quick());
    let scout = push(&socket, "scout@agentmail.to");
    assert!(scout.watch(Duration::from_secs(5)).await.is_err());
    assert_eq!(fake.connections(), 3);
}

#[tokio::test]
async fn the_socket_closes_when_its_last_agent_goes() {
    let fake = FakeAgentMailSocket::start().await;
    let socket = socket(&fake, quick());
    let scout = push(&socket, "scout@agentmail.to");
    take_catch_up(&scout).await;
    drop(scout);
    drop(socket);
    // The fake sees the close; nothing reconnects.
    tokio::time::sleep(Duration::from_millis(300)).await;
    fake.message_received("scout@agentmail.to", "s3");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(fake.connections(), 1);
}

#[test]
fn plain_ws_is_for_the_loopback_address_only() {
    let tokens = || Arc::new(StaticToken("k".into())) as Arc<dyn TokenSource>;
    assert!(AgentMailSocket::new("ws://ws.agentmail.to/v0", tokens(), SocketConfig::default()).is_err());
    assert!(AgentMailSocket::new("ws://127.0.0.1:9/v0", tokens(), SocketConfig::default()).is_ok());
    assert!(AgentMailSocket::new(AGENTMAIL_WS, tokens(), SocketConfig::default()).is_ok());
    assert!(AgentMailSocket::new("https://ws.agentmail.to/v0", tokens(), SocketConfig::default()).is_err());
    assert_eq!(percent_encode("am_k/1+="), "am_k%2F1%2B%3D");
    assert_eq!(redact("GET /v0?api_key=am_k%2F1 failed", "am_k/1"), "GET /v0?api_key=… failed");
}
