//! The Granola integration (spec §14.14): optional, off until the user
//! connects it in Settings › Integrations. Signing in goes through the
//! browser (Granola's MCP server, OAuth with dynamic registration); the
//! tokens live in the Keychain, never in Swift. Kaluta only reads:
//! recent meetings, their notes and transcripts, for the follow-ups job.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use provider_granola::{GranolaMcp, MeetingPage, Pending, Tokens};
use serde::{Deserialize, Serialize};

use crate::account::SignInStart;
use crate::{Core, CoreError, ErrorKind, runtime};

/// The Keychain item holding the connection.
const SECRET_KEY: &str = "granola.connection";
/// How long to wait for the user in the browser.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(300);

/// What the Keychain holds: the tokens and the Granola account's address.
#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    tokens: Tokens,
    #[serde(default)]
    email: Option<String>,
}

/// Whether Granola is connected, and as whom.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GranolaStatus {
    pub connected: bool,
    /// The Granola account's address, once Kaluta has read it.
    pub email: Option<String>,
}

/// Where meetings come from: Granola, or a fake.
#[async_trait]
pub(crate) trait MeetingSource: Send + Sync {
    /// Every meeting between two local days (`YYYY-MM-DD`), all pages.
    async fn meetings(&self, from_day: &str, to_day: &str) -> Result<MeetingPage, CoreError>;
    async fn notes(&self, meeting_id: &str) -> Result<String, CoreError>;
    async fn transcript(&self, meeting_id: &str, max_chars: u32) -> Result<String, CoreError>;
}

/// Pages read at most per listing.
const MAX_PAGES: usize = 4;

#[async_trait]
impl MeetingSource for GranolaMcp {
    async fn meetings(&self, from_day: &str, to_day: &str) -> Result<MeetingPage, CoreError> {
        let mut all = MeetingPage::default();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let page = self.list_meetings(from_day, to_day, cursor.as_deref()).await?;
            all.connected_email = all.connected_email.or(page.connected_email);
            all.meetings.extend(page.meetings);
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        Ok(all)
    }
    async fn notes(&self, meeting_id: &str) -> Result<String, CoreError> {
        Ok(self.meeting_notes(meeting_id).await?)
    }
    async fn transcript(&self, meeting_id: &str, max_chars: u32) -> Result<String, CoreError> {
        Ok(GranolaMcp::transcript(self, meeting_id, max_chars).await?)
    }
}

/// On the core's runtime: the MCP client uses tokio's timers and sockets.
struct OnRuntime(Arc<GranolaMcp>);

#[async_trait]
impl MeetingSource for OnRuntime {
    async fn meetings(&self, from_day: &str, to_day: &str) -> Result<MeetingPage, CoreError> {
        let (mcp, from, to) = (self.0.clone(), from_day.to_owned(), to_day.to_owned());
        runtime::run(async move { mcp.meetings(&from, &to).await }).await
    }
    async fn notes(&self, meeting_id: &str) -> Result<String, CoreError> {
        let (mcp, id) = (self.0.clone(), meeting_id.to_owned());
        runtime::run(async move { MeetingSource::notes(&*mcp, &id).await }).await
    }
    async fn transcript(&self, meeting_id: &str, max_chars: u32) -> Result<String, CoreError> {
        let (mcp, id) = (self.0.clone(), meeting_id.to_owned());
        runtime::run(async move { MeetingSource::transcript(&*mcp, &id, max_chars).await }).await
    }
}

/// The demo's and tests' Granola: two meetings yesterday, one with
/// someone else and one alone, for `email`.
pub(crate) struct FakeGranola {
    pub email: String,
    pub now: i64,
}

impl FakeGranola {
    fn at(&self, hours_ago: i64) -> String {
        chrono::DateTime::from_timestamp_millis(self.now - hours_ago * 3_600_000)
            .map(|t| t.to_rfc3339())
            .unwrap_or_default()
    }
}

#[async_trait]
impl MeetingSource for FakeGranola {
    async fn meetings(&self, _from: &str, _to: &str) -> Result<MeetingPage, CoreError> {
        let me = provider_granola::Participant { name: "Jo".into(), email: Some(self.email.clone()) };
        Ok(MeetingPage {
            connected_email: Some(self.email.clone()),
            meetings: vec![
                provider_granola::Meeting {
                    id: "fake-pilot".into(),
                    title: "Q4 pilot kickoff with Reese".into(),
                    meeting_at: Some(self.at(20)),
                    participants: vec![
                        me.clone(),
                        provider_granola::Participant {
                            name: "Reese Park".into(),
                            email: Some("reese@example.org".into()),
                        },
                    ],
                    url: Some("https://notes.granola.ai/d/fake-pilot".into()),
                },
                provider_granola::Meeting {
                    id: "fake-focus".into(),
                    title: "Focus time".into(),
                    meeting_at: Some(self.at(26)),
                    participants: vec![me],
                    url: None,
                },
            ],
            next_cursor: None,
        })
    }
    async fn notes(&self, meeting_id: &str) -> Result<String, CoreError> {
        Ok(match meeting_id {
            "fake-pilot" => "Summary: Reese wants the pilot proposal in writing by Friday, with pricing for 50 seats. \
                Jo agreed to send the proposal and the security questionnaire."
                .into(),
            _ => "Summary: Jo blocked time to write.".into(),
        })
    }
    async fn transcript(&self, meeting_id: &str, _max: u32) -> Result<String, CoreError> {
        Ok(match meeting_id {
            "fake-pilot" => "Reese: Can you send the proposal by Friday? Jo: Yes, with the questionnaire.".into(),
            _ => String::new(),
        })
    }
}

/// The core's Granola state: sign-ins under way, the client, a fake.
#[derive(Default)]
pub(crate) struct GranolaState {
    pending: Mutex<HashMap<String, Pending>>,
    client: Mutex<Option<Arc<GranolaMcp>>>,
    fake: OnceLock<Arc<FakeGranola>>,
}

fn http() -> Result<reqwest::Client, CoreError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| CoreError::new(ErrorKind::Network, e.to_string()))
}

impl Core {
    fn granola_stored(&self) -> Result<Option<Stored>, CoreError> {
        Ok(self.secrets.get(SECRET_KEY.into())?.and_then(|json| serde_json::from_str(&json).ok()))
    }

    fn granola_store(&self, stored: &Stored) -> Result<(), CoreError> {
        let json = serde_json::to_string(stored).map_err(|e| CoreError::new(ErrorKind::Internal, e.to_string()))?;
        self.secrets.set(SECRET_KEY.into(), json)
    }

    /// Where meetings come from now; `None` when Granola is not connected.
    pub(crate) fn meeting_source(self: &Arc<Self>) -> Result<Option<Arc<dyn MeetingSource>>, CoreError> {
        if let Some(fake) = self.granola.fake.get() {
            return Ok(Some(fake.clone()));
        }
        if let Some(client) = self.granola.client.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Ok(Some(Arc::new(OnRuntime(client))));
        }
        let Some(stored) = self.granola_stored()? else { return Ok(None) };
        let core = Arc::downgrade(self);
        let keep = Arc::new(move |tokens: &Tokens| {
            if let Some(core) = core.upgrade() {
                let email = core.granola_stored().ok().flatten().and_then(|s| s.email);
                if let Err(e) = core.granola_store(&Stored { tokens: tokens.clone(), email }) {
                    tracing::warn!(error = %e, "Granola's new tokens could not be kept");
                }
            }
        });
        let client = Arc::new(GranolaMcp::new(http()?, provider_granola::MCP_URL, stored.tokens, Some(keep)));
        *self.granola.client.lock().unwrap_or_else(|e| e.into_inner()) = Some(client.clone());
        Ok(Some(Arc::new(OnRuntime(client))))
    }

    /// The Granola account's address, once known.
    pub(crate) fn granola_email(&self) -> Option<String> {
        if let Some(fake) = self.granola.fake.get() {
            return Some(fake.email.clone());
        }
        self.granola_stored().ok().flatten().and_then(|s| s.email)
    }

    /// Remember the Granola account's address (read from a listing).
    pub(crate) fn note_granola_email(&self, email: &str) {
        if let Ok(Some(mut stored)) = self.granola_stored()
            && stored.email.as_deref() != Some(email)
        {
            stored.email = Some(email.to_owned());
            let _ = self.granola_store(&stored);
        }
    }

    /// Granola refused the connection for good: forget it, so the user is
    /// asked to connect again rather than failing every hour.
    pub(crate) fn granola_signed_out(&self) {
        *self.granola.client.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let _ = self.secrets.delete(SECRET_KEY.into());
        self.account_events().emit(crate::CoreEvent::WorkflowsChanged);
    }
}

#[uniffi::export]
impl Core {
    /// Whether Granola is connected (spec §14.14).
    pub fn granola_status(&self) -> Result<GranolaStatus, CoreError> {
        if let Some(fake) = self.granola.fake.get() {
            return Ok(GranolaStatus { connected: true, email: Some(fake.email.clone()) });
        }
        let stored = self.granola_stored()?;
        Ok(GranolaStatus { connected: stored.is_some(), email: stored.and_then(|s| s.email) })
    }

    /// Start connecting Granola: Kaluta registers with Granola's sign-in
    /// and the app opens the URL in the browser.
    pub async fn begin_granola_connect(&self) -> Result<SignInStart, CoreError> {
        let pending =
            runtime::run(async move { Ok(provider_granola::oauth::begin(&http()?, provider_granola::MCP_URL).await?) })
                .await?;
        let url = pending.url.clone();
        let session_id = format!("granola-{}", mail_sync::now_millis());
        self.granola.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(session_id.clone(), pending);
        Ok(SignInStart { session_id, authorization_url: url })
    }

    /// Wait for the browser, keep the tokens, and read who it is.
    pub async fn complete_granola_connect(self: Arc<Self>, session_id: String) -> Result<GranolaStatus, CoreError> {
        let pending = self
            .granola
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&session_id)
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that Granola sign-in is no longer open"))?;
        let tokens = runtime::run(async move { Ok(pending.finish(&http()?, CONNECT_TIMEOUT).await?) }).await?;
        *self.granola.client.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.granola_store(&Stored { tokens, email: None })?;
        // Who it is: the listing names the account.
        if let Some(source) = self.meeting_source()? {
            let today = crate::analysis_run::day_of(mail_sync::now_millis());
            match source.meetings(&today, &today).await {
                Ok(page) => {
                    if let Some(email) = page.connected_email {
                        self.note_granola_email(&email);
                    }
                }
                Err(e) => tracing::warn!(error = %e, "Granola connected but its meetings could not be read yet"),
            }
        }
        self.account_events().emit(crate::CoreEvent::WorkflowsChanged);
        self.granola_status()
    }

    /// Give up a sign-in the user closed.
    pub fn cancel_granola_connect(&self, session_id: String) {
        self.granola.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&session_id);
    }

    /// Stop using Granola: the Keychain item goes, and the meetings read
    /// from it with their suggestions (the open account's).
    pub async fn disconnect_granola(&self) -> Result<(), CoreError> {
        *self.granola.client.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.secrets.delete(SECRET_KEY.into())?;
        if let Ok(db) = self.db() {
            runtime::run(async move { Ok(db.write(mail_store::followups::clear).await?) }).await?;
        }
        self.account_events().emit(crate::CoreEvent::WorkflowsChanged);
        Ok(())
    }

    /// Development and UI tests: a fake Granola with two meetings, as the
    /// open account's address.
    pub fn debug_use_fake_granola(&self, email: String) {
        let _ = self.granola.fake.set(Arc::new(FakeGranola { email, now: mail_sync::now_millis() }));
    }
}
