//! AgentMail's WebSocket as the push source of an organisation's agents
//! (spec §7.9; docs.agentmail.to/websockets, read 2026-10-08).
//!
//! `wss://ws.agentmail.to/v0?api_key=…`; the client sends
//! `{"type":"subscribe","inbox_ids":[…],"event_types":[…]}` and the server
//! answers `{"type":"subscribed",…}`, then sends
//! `{"type":"event","event_type":"message.received","event_id":…,"message":{"inbox_id":…}}`
//! (or `send`, `delivery`, … for the other message events), or
//! `{"type":"error","name":…,"message":…}`.
//!
//! - **One socket per service account.** Its agents share the key, and a
//!   subscription names several inboxes (at most ten per message, so more
//!   are sent as several), so [`AgentMailSocket`] is shared by every agent
//!   of the organisation; each agent's [`AgentMailPush`] waits for its own
//!   inbox.
//! - **A wake-up, not the source of truth.** An event only makes the
//!   agent's sync poll now ([`BackfillSource::watch`] answers `true`); the
//!   payload is not stored. Label changes still come from polling the
//!   inbox's events, which the socket does not carry.
//! - **Reconnects with backoff**, subscribing again and waking every agent
//!   (mail may have arrived meanwhile). After repeated failures it rests
//!   for a while and [`AgentMailPush::watch`] fails, so the agents' push
//!   loop backs off and the 30 s / 5 min poll carries on alone.
//! - TLS through `tokio-rustls` with the webpki roots, as Gmail's IMAP;
//!   plain `ws://` only to the loopback address (tests).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use mail_domain::MessageId;
use provider_api::{
    BackfillSource, FetchedMessage, MailProvider, Priority, ProviderError, ProviderResult, TokenSource,
};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::AgentMailProvider;

/// AgentMail's WebSocket endpoint.
pub const AGENTMAIL_WS: &str = "wss://ws.agentmail.to/v0";
/// Inboxes per subscribe message (the API's limit for a subscription).
const INBOXES_PER_SUBSCRIBE: usize = 10;
/// The events that wake an agent: mail in, and mail out from elsewhere
/// (a send with the same key). Spam, blocked and unauthenticated mail
/// need permissions a key may lack, which would fail the subscription.
const EVENT_TYPES: [&str; 2] = ["message.received", "message.sent"];

/// Timings, shortened in tests.
#[derive(Debug, Clone)]
pub struct SocketConfig {
    pub connect_timeout: Duration,
    pub first_backoff: Duration,
    pub max_backoff: Duration,
    /// Failures in a row before resting.
    pub failures_before_rest: u32,
    /// How long it rests (the agents poll) before trying again.
    pub rest: Duration,
    /// A ping this often, so a dead connection is noticed.
    pub ping_every: Duration,
    /// No frame at all (not even a pong) for this long: reconnect.
    pub silence_limit: Duration,
    /// A connection dropped sooner than this after subscribing counts as a
    /// failure (toward resting), so a server that closes at once is not
    /// reconnected to in a loop.
    pub stable_after: Duration,
}

impl Default for SocketConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(15),
            first_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            failures_before_rest: 5,
            rest: Duration::from_secs(15 * 60),
            ping_every: Duration::from_secs(60),
            silence_limit: Duration::from_secs(150),
            stable_after: Duration::from_secs(30),
        }
    }
}

/// Where the socket stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketState {
    /// Not started, connecting, or between attempts.
    Connecting,
    /// Subscribed: events arrive.
    Connected,
    /// Failed repeatedly; the agents poll until it tries again.
    Resting,
}

/// One WebSocket for a service account's inboxes, started on the first
/// wait and stopped when the last agent's [`AgentMailPush`] goes.
pub struct AgentMailSocket {
    url: String,
    tokens: Arc<dyn TokenSource>,
    config: SocketConfig,
    /// Each inbox's waiters (how many pushes share it) and its wake-up.
    inboxes: Mutex<HashMap<String, (usize, Arc<Notify>)>>,
    state: watch::Sender<SocketState>,
    /// Inboxes added while connected, to subscribe to at once.
    commands: mpsc::UnboundedSender<String>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<String>>>,
    started: AtomicBool,
}

impl AgentMailSocket {
    /// A socket for the organisation whose key `tokens` gives. `url` is
    /// [`AGENTMAIL_WS`] but in tests; `ws://` is refused unless loopback.
    pub fn new(url: &str, tokens: Arc<dyn TokenSource>, config: SocketConfig) -> ProviderResult<Arc<Self>> {
        let parsed = Endpoint::parse(url)?;
        if !parsed.tls && !parsed.loopback() {
            return Err(ProviderError::Invalid("AgentMail's WebSocket must use wss://".into()));
        }
        let (commands, receiver) = mpsc::unbounded_channel();
        Ok(Arc::new(Self {
            url: url.trim_end_matches('/').to_owned(),
            tokens,
            config,
            inboxes: Mutex::new(HashMap::new()),
            state: watch::Sender::new(SocketState::Connecting),
            commands,
            receiver: Mutex::new(Some(receiver)),
            started: AtomicBool::new(false),
        }))
    }

    pub fn state(&self) -> SocketState {
        *self.state.borrow()
    }

    fn inboxes(&self) -> std::sync::MutexGuard<'_, HashMap<String, (usize, Arc<Notify>)>> {
        self.inboxes.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn add_inbox(&self, inbox: &str) -> Arc<Notify> {
        let mut inboxes = self.inboxes();
        let entry = inboxes.entry(inbox.to_owned()).or_insert_with(|| (0, Arc::new(Notify::new())));
        entry.0 += 1;
        let notify = entry.1.clone();
        if entry.0 == 1 {
            // Subscribed with the rest when it next connects; at once if
            // it is connected now.
            let _ = self.commands.send(inbox.to_owned());
        }
        notify
    }

    fn remove_inbox(&self, inbox: &str) {
        let mut inboxes = self.inboxes();
        if let Some(entry) = inboxes.get_mut(inbox) {
            entry.0 -= 1;
            if entry.0 == 0 {
                // The protocol has no unsubscribe: its events are ignored.
                inboxes.remove(inbox);
            }
        }
    }

    fn wake(&self, inbox: Option<&str>) {
        let inboxes = self.inboxes();
        match inbox.and_then(|i| inboxes.get(i)) {
            Some((_, notify)) => notify.notify_one(),
            // An event naming no inbox we know: everyone polls.
            None if inbox.is_none() => inboxes.values().for_each(|(_, n)| n.notify_one()),
            None => {}
        }
    }

    /// Start the connection task (once), in the current Tokio runtime.
    fn ensure_started(self: &Arc<Self>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let Some(receiver) = self.receiver.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
        let weak = Arc::downgrade(self);
        tokio::spawn(run(weak, receiver));
    }
}

/// How a connection ended.
enum Ended {
    /// The socket's owners are gone: stop.
    Closed,
    /// It was subscribed for a while ([`SocketConfig::stable_after`]),
    /// then dropped: reconnect after the first backoff.
    Dropped,
    /// It never got going, or the server said no.
    Failed(String),
}

async fn run(weak: Weak<AgentMailSocket>, mut commands: mpsc::UnboundedReceiver<String>) {
    let config = match weak.upgrade() {
        Some(socket) => socket.config.clone(),
        None => return,
    };
    let mut failures = 0u32;
    let mut backoff = config.first_backoff;
    loop {
        let ended = connection(&weak, &mut commands, &config).await;
        let wait = match ended {
            Ended::Closed => return,
            Ended::Dropped => {
                failures = 0;
                backoff = config.first_backoff;
                tracing::info!("AgentMail's WebSocket closed; reconnecting");
                // Not at once: every reconnection wakes every agent.
                config.first_backoff
            }
            Ended::Failed(why) => {
                failures += 1;
                tracing::info!(failures, why = %why, "AgentMail's WebSocket failed");
                if failures >= config.failures_before_rest {
                    tracing::warn!("AgentMail's WebSocket keeps failing; polling only for a while");
                    failures = 0;
                    backoff = config.first_backoff;
                    match weak.upgrade() {
                        Some(socket) => socket.state.send_replace(SocketState::Resting),
                        None => return,
                    };
                    config.rest
                } else {
                    let wait = backoff;
                    backoff = (backoff * 2).min(config.max_backoff);
                    wait
                }
            }
        };
        // Wait, but stop as soon as the owners go (the channel closes
        // with the socket). Inboxes added meanwhile are subscribed with
        // the rest on connecting.
        let sleep = tokio::time::sleep(wait);
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                () = &mut sleep => break,
                command = commands.recv() => if command.is_none() { return },
            }
        }
        match weak.upgrade() {
            Some(socket) => socket.state.send_replace(SocketState::Connecting),
            None => return,
        };
    }
}

/// One connection: connect, subscribe every inbox, then wake inboxes as
/// events arrive until it drops.
async fn connection(
    weak: &Weak<AgentMailSocket>,
    commands: &mut mpsc::UnboundedReceiver<String>,
    config: &SocketConfig,
) -> Ended {
    let (url, tokens) = match weak.upgrade() {
        Some(socket) => (socket.url.clone(), socket.tokens.clone()),
        None => return Ended::Closed,
    };
    let key = match tokens.access_token().await {
        Ok(key) => key,
        Err(e) => return Ended::Failed(e.to_string()),
    };
    let endpoint = match Endpoint::parse(&url) {
        Ok(endpoint) => endpoint,
        Err(e) => return Ended::Failed(e.to_string()),
    };
    // The key is in the query, as the API documents, and in the
    // Authorization header as for its REST calls. Never logged.
    let with_key = format!("{url}?api_key={}", percent_encode(key.expose()));
    let mut request = match with_key.into_client_request() {
        Ok(request) => request,
        Err(e) => return Ended::Failed(format!("bad WebSocket address: {}", redact(&e.to_string(), key.expose()))),
    };
    if let Ok(value) = format!("Bearer {}", key.expose()).parse() {
        request.headers_mut().insert("authorization", value);
    }
    if let Ok(value) = concat!("Kaluta/", env!("CARGO_PKG_VERSION")).parse() {
        request.headers_mut().insert("user-agent", value);
    }
    let tcp =
        match tokio::time::timeout(config.connect_timeout, TcpStream::connect((endpoint.host.as_str(), endpoint.port)))
            .await
        {
            Ok(Ok(tcp)) => tcp,
            Ok(Err(e)) => return Ended::Failed(e.to_string()),
            Err(_) => return Ended::Failed("connecting timed out".into()),
        };
    if endpoint.tls {
        let roots = tokio_rustls::rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        let tls = tokio_rustls::rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        let name = match tokio_rustls::rustls::pki_types::ServerName::try_from(endpoint.host.clone()) {
            Ok(name) => name,
            Err(e) => return Ended::Failed(e.to_string()),
        };
        let connector = tokio_rustls::TlsConnector::from(Arc::new(tls));
        match tokio::time::timeout(config.connect_timeout, connector.connect(name, tcp)).await {
            Ok(Ok(stream)) => session(weak, commands, config, request, stream, key.expose()).await,
            Ok(Err(e)) => Ended::Failed(e.to_string()),
            Err(_) => Ended::Failed("TLS timed out".into()),
        }
    } else {
        session(weak, commands, config, request, tcp, key.expose()).await
    }
}

async fn session<S: AsyncRead + AsyncWrite + Unpin>(
    weak: &Weak<AgentMailSocket>,
    commands: &mut mpsc::UnboundedReceiver<String>,
    config: &SocketConfig,
    request: tokio_tungstenite::tungstenite::handshake::client::Request,
    stream: S,
    key: &str,
) -> Ended {
    let handshake = tokio_tungstenite::client_async(request, stream);
    let mut ws = match tokio::time::timeout(config.connect_timeout, handshake).await {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(e)) => return Ended::Failed(redact(&e.to_string(), key)),
        Err(_) => return Ended::Failed("the WebSocket handshake timed out".into()),
    };
    // Every inbox, ten to a message. The queue of inboxes added meanwhile
    // is emptied first and the list read after: an inbox added in between
    // is then in the list, or its command still queued (subscribed twice at
    // worst), never neither.
    while commands.try_recv().is_ok() {}
    let inboxes: Vec<String> = match weak.upgrade() {
        Some(socket) => socket.inboxes().keys().cloned().collect(),
        None => return Ended::Closed,
    };
    for chunk in inboxes.chunks(INBOXES_PER_SUBSCRIBE) {
        if let Err(e) = ws.send(subscribe(chunk)).await {
            return Ended::Failed(e.to_string());
        }
    }
    let mut subscribed = inboxes.is_empty();
    let mut subscribed_at = tokio::time::Instant::now();
    if subscribed {
        mark_connected(weak);
    }
    // Dropped once subscribed: a failure all the same if it came soon after
    // (a server closing at once must not be reconnected to in a loop).
    let dropped = |subscribed: bool, at: tokio::time::Instant, why: String| {
        if !subscribed {
            Ended::Failed(why)
        } else if at.elapsed() < config.stable_after {
            Ended::Failed(format!("dropped soon after subscribing ({why})"))
        } else {
            Ended::Dropped
        }
    };
    let mut last_frame = tokio::time::Instant::now();
    let mut ping = tokio::time::interval(config.ping_every);
    ping.reset();
    let deadline = tokio::time::Instant::now() + config.connect_timeout;
    loop {
        tokio::select! {
            frame = ws.next() => {
                last_frame = tokio::time::Instant::now();
                let text = match frame {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None => return dropped(subscribed, subscribed_at, "closed".into()),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return dropped(subscribed, subscribed_at, redact(&e.to_string(), key)),
                };
                let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                match value.get("type").and_then(Value::as_str) {
                    Some("subscribed") if !subscribed => {
                        subscribed = true;
                        subscribed_at = tokio::time::Instant::now();
                        mark_connected(weak);
                    }
                    Some("event") => match weak.upgrade() {
                        Some(socket) => socket.wake(event_inbox(&value).as_deref()),
                        None => return Ended::Closed,
                    },
                    Some("error") => {
                        let name = value.get("name").and_then(Value::as_str).unwrap_or("error");
                        let message = value.get("message").and_then(Value::as_str).unwrap_or("");
                        let _ = ws.close(None).await;
                        return Ended::Failed(format!("{name}: {}", redact(message, key)));
                    }
                    _ => {}
                }
            }
            command = commands.recv() => match command {
                None => {
                    let _ = ws.close(None).await;
                    return Ended::Closed;
                }
                Some(inbox) => {
                    if let Err(e) = ws.send(subscribe(std::slice::from_ref(&inbox))).await {
                        return dropped(subscribed, subscribed_at, e.to_string());
                    }
                }
            },
            _ = ping.tick() => {
                if weak.strong_count() == 0 {
                    let _ = ws.close(None).await;
                    return Ended::Closed;
                }
                if last_frame.elapsed() > config.silence_limit {
                    return dropped(subscribed, subscribed_at, "silent".into());
                }
                if ws.send(Message::Ping(Vec::new().into())).await.is_err() {
                    return dropped(subscribed, subscribed_at, "ping failed".into());
                }
            }
            () = tokio::time::sleep_until(deadline), if !subscribed => {
                return Ended::Failed("no answer to the subscription".into());
            }
        }
    }
}

/// Subscribed: every agent polls once (mail may have arrived while it was
/// not), then waits for events.
fn mark_connected(weak: &Weak<AgentMailSocket>) {
    if let Some(socket) = weak.upgrade() {
        socket.state.send_replace(SocketState::Connected);
        socket.wake(None);
    }
}

fn subscribe(inboxes: &[String]) -> Message {
    Message::Text(json!({ "type": "subscribe", "inbox_ids": inboxes, "event_types": EVENT_TYPES }).to_string().into())
}

/// The inbox an event is about: its payload object's `inbox_id`
/// (`message`, `send`, `delivery`, `bounce`, `complaint`, `reject`,
/// `open`, or top level for calendar events).
fn event_inbox(event: &Value) -> Option<String> {
    if let Some(inbox) = event.get("inbox_id").and_then(Value::as_str) {
        return Some(inbox.to_owned());
    }
    ["message", "send", "delivery", "bounce", "complaint", "reject", "open"]
        .iter()
        .find_map(|key| event.get(*key)?.get("inbox_id")?.as_str().map(str::to_owned))
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Errors may echo the URL: never let the key through.
fn redact(text: &str, key: &str) -> String {
    if key.is_empty() { text.to_owned() } else { text.replace(key, "…").replace(&percent_encode(key), "…") }
}

struct Endpoint {
    tls: bool,
    host: String,
    port: u16,
}

impl Endpoint {
    fn parse(url: &str) -> ProviderResult<Self> {
        let invalid = || ProviderError::Invalid(format!("not a WebSocket address: {url}"));
        let (tls, rest) = if let Some(rest) = url.strip_prefix("wss://") {
            (true, rest)
        } else if let Some(rest) = url.strip_prefix("ws://") {
            (false, rest)
        } else {
            return Err(invalid());
        };
        let authority = rest.split(['/', '?']).next().unwrap_or_default();
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host, port.parse().map_err(|_| invalid())?),
            _ => (authority, if tls { 443 } else { 80 }),
        };
        if host.is_empty() || host.contains(':') {
            return Err(invalid());
        }
        Ok(Self { tls, host: host.to_owned(), port })
    }

    fn loopback(&self) -> bool {
        self.host == "localhost" || self.host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
    }
}

/// An agent's push source: its provider for backfill, and its inbox's
/// wake-ups from the organisation's socket.
pub struct AgentMailPush {
    provider: Arc<AgentMailProvider>,
    socket: Arc<AgentMailSocket>,
    inbox: String,
    notify: Arc<Notify>,
}

impl AgentMailPush {
    pub fn new(provider: Arc<AgentMailProvider>, socket: Arc<AgentMailSocket>, inbox: &str) -> Self {
        let notify = socket.add_inbox(inbox);
        Self { provider, socket, inbox: inbox.to_owned(), notify }
    }
}

impl Drop for AgentMailPush {
    fn drop(&mut self) {
        self.socket.remove_inbox(&self.inbox);
    }
}

#[async_trait]
impl BackfillSource for AgentMailPush {
    async fn fetch(&self, ids: &[MessageId]) -> ProviderResult<Vec<FetchedMessage>> {
        self.provider.fetch_messages(ids, Priority::Background).await
    }

    /// Up to `max` for an event about this agent's inbox: `true` then
    /// (the sync polls), `false` when the time runs out. Fails while the
    /// socket rests after repeated failures, so the push loop backs off
    /// and polling carries on.
    async fn watch(&self, max: Duration) -> ProviderResult<Option<bool>> {
        self.socket.ensure_started();
        let mut state = self.socket.state.subscribe();
        let resting = || ProviderError::Unavailable("AgentMail's WebSocket keeps failing; polling instead".into());
        if *state.borrow_and_update() == SocketState::Resting {
            return Err(resting());
        }
        let rests = async {
            loop {
                if state.changed().await.is_err() || *state.borrow_and_update() == SocketState::Resting {
                    return;
                }
            }
        };
        tokio::select! {
            () = self.notify.notified() => Ok(Some(true)),
            () = tokio::time::sleep(max) => Ok(Some(false)),
            () = rests => Err(resting()),
        }
    }

    fn name(&self) -> &'static str {
        "agentmail"
    }
}

#[cfg(test)]
mod tests;
