//! A local fake of AgentMail's WebSocket (`/v0`, spec §7.9) for tests: it
//! accepts connections on 127.0.0.1, records the key and each
//! subscription, answers `subscribed` (or an error), and sends the events
//! a test pushes. Nothing here talks to AgentMail.

use std::sync::{Arc, Mutex};

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};

#[derive(Default)]
struct State {
    /// Connections accepted (refused handshakes are not counted).
    connections: usize,
    /// Handshakes refused with 401.
    refused: usize,
    /// The `api_key` of each accepted connection, in order.
    keys: Vec<String>,
    /// The `Authorization` header of each accepted connection.
    authorizations: Vec<Option<String>>,
    /// Inbox ids of each subscribe message, in order.
    subscriptions: Vec<Vec<String>>,
    /// Live connections, to push events to or drop.
    live: Vec<mpsc::UnboundedSender<Option<Message>>>,
    /// Refuse the handshake (a bad key).
    refuse: bool,
    /// Answer subscriptions with an error.
    error_on_subscribe: bool,
}

/// A running fake. Dropping it stops accepting connections.
pub struct FakeAgentMailSocket {
    pub addr: std::net::SocketAddr,
    state: Arc<Mutex<State>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FakeAgentMailSocket {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeAgentMailSocket {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind the fake WebSocket");
        let addr = listener.local_addr().expect("its address");
        let state = Arc::new(Mutex::new(State::default()));
        let shared = state.clone();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let state = shared.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, state).await;
                });
            }
        });
        Self { addr, state, task }
    }

    /// The URL to give `AgentMailSocket::new`.
    pub fn url(&self) -> String {
        format!("ws://{}/v0", self.addr)
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn connections(&self) -> usize {
        self.state().connections
    }

    pub fn refused(&self) -> usize {
        self.state().refused
    }

    pub fn keys(&self) -> Vec<String> {
        self.state().keys.clone()
    }

    pub fn authorizations(&self) -> Vec<Option<String>> {
        self.state().authorizations.clone()
    }

    /// Each subscribe message's inbox ids, in order.
    pub fn subscriptions(&self) -> Vec<Vec<String>> {
        self.state().subscriptions.clone()
    }

    /// Every inbox subscribed to, sorted, without repeats.
    pub fn subscribed_inboxes(&self) -> Vec<String> {
        let mut all: Vec<String> = self.state().subscriptions.iter().flatten().cloned().collect();
        all.sort();
        all.dedup();
        all
    }

    pub fn refuse(&self, refuse: bool) {
        self.state().refuse = refuse;
    }

    pub fn error_on_subscribe(&self, error: bool) {
        self.state().error_on_subscribe = error;
    }

    /// Send `message.received` for `inbox` on every live connection.
    pub fn message_received(&self, inbox: &str, message_id: &str) {
        let event = json!({
            "type": "event", "event_type": "message.received", "event_id": format!("ev-{message_id}"),
            "message": {
                "inbox_id": inbox, "thread_id": "t", "message_id": message_id, "labels": ["received", "unread"],
                "timestamp": "2026-10-08T08:00:00Z", "from": "Ada <ada@example.com>", "to": [inbox],
                "size": 90, "created_at": "2026-10-08T08:00:00Z", "updated_at": "2026-10-08T08:00:00Z"
            },
            "thread": { "inbox_id": inbox, "thread_id": "t" }
        });
        self.send(Some(Message::Text(event.to_string().into())));
    }

    /// Close every live connection.
    pub fn drop_connections(&self) {
        self.send(None);
        self.state().live.clear();
    }

    fn send(&self, message: Option<Message>) {
        self.state().live.retain(|tx| tx.send(message.clone()).is_ok());
    }
}

// The handshake callback's type is tungstenite's, large error and all.
#[allow(clippy::result_large_err)]
async fn serve(stream: tokio::net::TcpStream, state: Arc<Mutex<State>>) -> Result<(), ()> {
    let lock = |s: &Arc<Mutex<State>>| s.lock().unwrap_or_else(|e| e.into_inner()).refuse;
    let refuse = lock(&state);
    let seen: Arc<Mutex<(String, Option<String>)>> = Arc::default();
    let record = seen.clone();
    let callback = move |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        if refuse {
            let mut error = ErrorResponse::new(Some("invalid api key".into()));
            *error.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::UNAUTHORIZED;
            return Err(error);
        }
        let key = request
            .uri()
            .query()
            .unwrap_or_default()
            .split('&')
            .find_map(|pair| pair.strip_prefix("api_key="))
            .unwrap_or_default()
            .to_owned();
        let auth = request.headers().get("authorization").and_then(|v| v.to_str().ok()).map(str::to_owned);
        *record.lock().unwrap_or_else(|e| e.into_inner()) = (key, auth);
        Ok(response)
    };
    let ws = match tokio_tungstenite::accept_hdr_async(stream, callback).await {
        Ok(ws) => ws,
        Err(_) => {
            if refuse {
                state.lock().unwrap_or_else(|e| e.into_inner()).refused += 1;
            }
            return Err(());
        }
    };
    let (tx, mut rx) = mpsc::unbounded_channel();
    {
        let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
        let (key, auth) = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
        s.connections += 1;
        s.keys.push(key);
        s.authorizations.push(auth);
        s.live.push(tx);
    }
    let (mut write, mut read) = ws.split();
    loop {
        tokio::select! {
            frame = read.next() => {
                let text = match frame {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return Ok(()),
                    Some(Ok(_)) => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else { continue };
                if value.get("type").and_then(Value::as_str) != Some("subscribe") {
                    continue;
                }
                let inboxes: Vec<String> = value
                    .get("inbox_ids")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default();
                let error = {
                    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
                    s.subscriptions.push(inboxes.clone());
                    s.error_on_subscribe
                };
                let answer = if error {
                    json!({ "type": "error", "name": "ForbiddenError", "message": "not allowed" })
                } else {
                    json!({ "type": "subscribed", "inbox_ids": inboxes, "event_types": value.get("event_types") })
                };
                write.send(Message::Text(answer.to_string().into())).await.map_err(|_| ())?;
            }
            message = rx.recv() => match message {
                Some(Some(message)) => write.send(message).await.map_err(|_| ())?,
                // Dropped by the test: close without a goodbye.
                Some(None) | None => return Ok(()),
            },
        }
    }
}
