//! Creating and verifying AgentMail organisations, adding inboxes and
//! making inbox keys (spec §7.9, ADR 0015).

use std::time::Duration;

use async_trait::async_trait;
use mail_domain::Redacted;
use provider_api::{
    AddedMailbox, MailboxPlan, MailboxService, ProviderError, ProviderResult, SignedUp, VerificationStarted,
};
use serde_json::json;

use crate::{AGENTMAIL_API, CODE_SENDER_DOMAIN, INBOX_LIMIT, MANAGED_DOMAIN, TERMS_URL, agentmail_error, wire};

/// Codes last 24 hours.
const CODE_LIFETIME_SECS: u32 = 24 * 60 * 60;
/// AgentMail states no wait before asking again; a minute is polite.
const RESEND_AFTER_SECS: u32 = 60;

/// An agent's name as an AgentMail username: `Research Scout` →
/// `research-scout`.
pub fn username_of(name: &str) -> String {
    let dashed: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let local = dashed.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    if local.is_empty() { "agent".into() } else { local }
}

/// The plan's name from its inbox limit (AgentMail's organisation gives
/// limits, not a plan name): Free 3, Developer 10, Startup 150.
pub fn plan_name(inbox_limit: Option<u32>) -> String {
    match inbox_limit {
        Some(3) => "free".into(),
        Some(10) => "developer".into(),
        Some(150) => "startup".into(),
        Some(n) => format!("{n} inboxes"),
        None => "agentmail".into(),
    }
}

/// AgentMail's limits are per month and per new recipient, not per hour or
/// day: those fields are 0 ("none stated") and the app words the limits
/// (`agent_service_limits` in the core).
fn plan(inbox_limit: Option<u32>, verified: bool) -> MailboxPlan {
    MailboxPlan {
        name: plan_name(inbox_limit),
        verified,
        reply_only: false,
        send_per_hour: 0,
        send_per_day: 0,
        email: None,
    }
}

/// Creating and verifying AgentMail organisations.
pub struct AgentMailService {
    client: reqwest::Client,
    base: String,
}

impl AgentMailService {
    pub fn new() -> ProviderResult<Self> {
        Self::with_base(AGENTMAIL_API)
    }

    pub fn with_base(base: &str) -> ProviderResult<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("OpenAGC/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .https_only(false) // tests talk to a local mock
            .build()
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        Ok(Self { client, base: base.trim_end_matches('/').to_owned() })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/v0/{path}", self.base)
    }

    async fn call<T: serde::de::DeserializeOwned>(&self, request: reqwest::RequestBuilder) -> ProviderResult<T> {
        let (status, body) = self.call_raw(request).await?;
        if (200..300).contains(&status) {
            return serde_json::from_str(&body).map_err(|e| ProviderError::Decode(e.to_string()));
        }
        Err(error_of(status, &body))
    }

    async fn call_raw(&self, request: reqwest::RequestBuilder) -> ProviderResult<(u16, String)> {
        let response = request.send().await.map_err(|e| ProviderError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(|e| ProviderError::Network(e.to_string()))?;
        Ok((status, body))
    }
}

/// An error answer, classified and worded.
fn error_of(status: u16, body: &str) -> ProviderError {
    match status {
        401 => ProviderError::Unauthorized,
        429 => ProviderError::RateLimited { retry_after: None },
        _ => agentmail_error(status, body).unwrap_or_else(|| match status {
            s if s >= 500 => ProviderError::Server { status: s, message: format!("AgentMail answered {s}") },
            s => ProviderError::Invalid(format!("AgentMail answered {s}")),
        }),
    }
}

#[async_trait]
impl MailboxService for AgentMailService {
    fn name(&self) -> &'static str {
        "agentmail"
    }

    fn terms_url(&self) -> &'static str {
        TERMS_URL
    }

    fn code_sender_domain(&self) -> &'static str {
        CODE_SENDER_DOMAIN
    }

    /// `POST /v0/agent/sign-up {username, human_email}`: the organisation,
    /// its key and its first inbox; the code goes to `human_email` at once.
    /// Without `human_email` the inbox can only receive. Never call it for
    /// an organisation the user already has: the same email returns it
    /// with a rotated key (the core refuses first).
    async fn sign_up(
        &self,
        device_name: &str,
        _idempotency_key: &str,
        human_email: Option<&str>,
    ) -> ProviderResult<SignedUp> {
        let username = username_of(device_name);
        let mut body = json!({ "username": username, "source": "openagc" });
        if let Some(email) = human_email.map(str::trim).filter(|e| !e.is_empty()) {
            body["human_email"] = json!(email);
        }
        let signed: wire::SignUp = self.call(self.client.post(self.url("agent/sign-up")).json(&body)).await?;
        // The inbox id is its address; should it not be, the address is the
        // username on AgentMail's domain.
        let address = if signed.inbox_id.contains('@') {
            signed.inbox_id.to_lowercase()
        } else {
            format!("{username}@{MANAGED_DOMAIN}")
        };
        Ok(SignedUp {
            api_key: Redacted::new(signed.api_key),
            address,
            plan: plan(None, false),
            inbox_id: Some(signed.inbox_id),
        })
    }

    /// The organisation's limits. AgentMail does not say there whether it
    /// is verified: `verified` is false here and the core keeps what the
    /// verification told it.
    async fn plan(&self, api_key: &str) -> ProviderResult<MailboxPlan> {
        let org: wire::Organization =
            self.call(self.client.get(self.url("organizations")).bearer_auth(api_key)).await?;
        Ok(plan(org.inbox_limit, false))
    }

    /// The code was sent at sign-up; this asks for it again
    /// (`POST /v0/agent/human`). With the same email it keeps the key and
    /// resends or renews the code; another email would replace the human,
    /// which AgentMail allows twice per organisation, so the core only
    /// passes the email the service account was created with.
    async fn start_verification(&self, api_key: &str, email: &str) -> ProviderResult<VerificationStarted> {
        let _: wire::HumanAttached = self
            .call(
                self.client
                    .post(self.url("agent/human"))
                    .bearer_auth(api_key)
                    .json(&json!({ "human_email": email.trim() })),
            )
            .await?;
        Ok(VerificationStarted { resend_after_secs: RESEND_AFTER_SECS, expires_in_secs: CODE_LIFETIME_SECS })
    }

    async fn verify(&self, api_key: &str, code: &str) -> ProviderResult<MailboxPlan> {
        let verified: wire::Verified = self
            .call(
                self.client
                    .post(self.url("agent/verify"))
                    .bearer_auth(api_key)
                    .json(&json!({ "otp_code": code.trim() })),
            )
            .await?;
        if !verified.verified {
            return Err(ProviderError::Invalid("That code is not right".into()));
        }
        // The limits after verifying; verified whatever they say.
        let limit = match self.plan(api_key).await {
            Ok(p) => p.name,
            Err(e) => {
                tracing::warn!(error = %e, "verified, but the plan could not be read");
                plan_name(Some(3))
            }
        };
        Ok(MailboxPlan { name: limit, ..plan(None, true) })
    }

    /// `POST /v0/inboxes`: another inbox in the organisation, for another
    /// agent. `idempotency_key` is AgentMail's `client_id`, so a retry
    /// returns the same inbox.
    async fn add_mailbox(
        &self,
        api_key: &str,
        username: &str,
        domain: Option<&str>,
        display_name: &str,
        idempotency_key: &str,
    ) -> ProviderResult<AddedMailbox> {
        let mut body = json!({
            "username": username,
            "display_name": display_name,
            "client_id": idempotency_key.replace('@', "_"),
        });
        if let Some(domain) = domain.filter(|d| !d.eq_ignore_ascii_case(MANAGED_DOMAIN)) {
            body["domain"] = json!(domain);
        }
        let (status, text) =
            self.call_raw(self.client.post(self.url("inboxes")).bearer_auth(api_key).json(&body)).await?;
        if !(200..300).contains(&status) {
            let parsed: wire::ErrorBody = serde_json::from_str(&text).unwrap_or_default();
            let said = format!("{} {}", parsed.message.as_deref().unwrap_or(""), parsed.fix.as_deref().unwrap_or(""))
                .to_ascii_lowercase();
            // The plan's inboxes are all used (the code is a guess until
            // seen: hand-check list).
            if parsed.code.as_deref() == Some("limit_exceeded") || (status == 403 && said.contains("limit")) {
                return Err(ProviderError::Invalid(INBOX_LIMIT.into()));
            }
            return Err(error_of(status, &text));
        }
        let inbox: wire::Inbox = serde_json::from_str(&text).map_err(|e| ProviderError::Decode(e.to_string()))?;
        let address = inbox.email.clone().unwrap_or_else(|| inbox.inbox_id.clone()).to_lowercase();
        Ok(AddedMailbox { address, inbox_id: inbox.inbox_id })
    }

    /// `POST /v0/inboxes/{id}/api-keys`: a key that reaches that inbox
    /// only. AgentMail makes them once the organisation is verified.
    async fn mailbox_api_key(&self, api_key: &str, inbox_id: &str, name: &str) -> ProviderResult<Redacted<String>> {
        let url = format!("{}/v0/inboxes/{}/api-keys", self.base, crate::escape(inbox_id));
        let key: wire::ApiKey =
            self.call(self.client.post(url).bearer_auth(api_key).json(&json!({ "name": name }))).await?;
        Ok(Redacted::new(key.api_key))
    }
}
