//! Agent mailboxes (spec §7.9, ADR 0014): an address for one of the
//! user's agents on an agent-mail service, created and verified here with
//! the service's own API, then synced like any account.
//!
//! The account directory holds `agent.json` (service, address, name); the
//! service's API key is in the Keychain as `mailbox.api_key.<id>`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mail_domain::Redacted;
use provider_api::fake::FakeProvider;
use provider_api::token::StaticToken;
use provider_api::{
    BackfillSource, MailProvider, MailboxPlan, MailboxService, ProviderError, ProviderResult, SignedUp,
    VerificationStarted,
};
use serde::{Deserialize, Serialize};

use crate::registry::{AccountKind, IndexEntry, accounts_dir};
use crate::secrets::{self, keys};
use crate::{Core, CoreError, ErrorKind, runtime};

const META_FILE: &str = "agent.json";
/// How far back to look for a verification code before *Send Code*: the
/// message can arrive a little before the clocks agree.
const CODE_SLACK_MS: i64 = 2 * 60 * 1000;

/// The agent-mail services OpenAGC can create mailboxes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum AgentService {
    Primitive,
}

/// Whether agents send from an agent mailbox without asking (spec §7.9).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "snake_case")]
pub enum AgentSendMode {
    /// Send freely; what breaks the writing guide is flagged (the default).
    #[default]
    Freely,
    /// Ask before each send, as on the user's own accounts.
    Ask,
}

/// What an agent mailbox may do now (spec §7.9).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentMailboxPlan {
    /// The service's plan name.
    pub name: String,
    /// The user confirmed an email address.
    pub verified: bool,
    /// It can only reply to addresses that wrote first.
    pub reply_only: bool,
    pub send_per_hour: u32,
    pub send_per_day: u32,
    /// The email it was verified with.
    pub email: Option<String>,
}

impl From<MailboxPlan> for AgentMailboxPlan {
    fn from(p: MailboxPlan) -> Self {
        Self {
            name: p.name,
            verified: p.verified,
            reply_only: p.reply_only,
            send_per_hour: p.send_per_hour,
            send_per_day: p.send_per_day,
            email: p.email,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentMailboxCreated {
    pub account_id: String,
    pub address: String,
    pub plan: AgentMailboxPlan,
}

/// A verification code is on its way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct AgentVerification {
    pub resend_after_secs: u32,
    pub expires_in_secs: u32,
}

/// `agent.json` in the account directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgentMeta {
    pub kind: String,
    pub service: AgentService,
    pub address: String,
    pub name: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub send_mode: AgentSendMode,
}

pub(crate) fn read_meta(dir: &Path) -> Option<AgentMeta> {
    let bytes = std::fs::read(dir.join(META_FILE)).ok()?;
    serde_json::from_slice::<AgentMeta>(&bytes).ok().filter(|m| m.kind == "agent")
}

fn write_meta(dir: &Path, meta: &AgentMeta) -> Result<(), CoreError> {
    std::fs::create_dir_all(dir).map_err(|e| CoreError::new(ErrorKind::Storage, e.to_string()))?;
    let tmp = dir.join(format!("{META_FILE}.tmp"));
    let bytes = serde_json::to_vec_pretty(meta).map_err(|e| CoreError::new(ErrorKind::Internal, e.to_string()))?;
    std::fs::write(&tmp, bytes).map_err(|e| CoreError::new(ErrorKind::Storage, e.to_string()))?;
    std::fs::rename(tmp, dir.join(META_FILE)).map_err(|e| CoreError::new(ErrorKind::Storage, e.to_string()))
}

pub(crate) fn rename_meta(dir: &Path, name: &str) -> Result<(), CoreError> {
    let mut meta =
        read_meta(dir).ok_or_else(|| CoreError::new(ErrorKind::NotFound, "the mailbox's details are missing"))?;
    meta.name = name.to_owned();
    write_meta(dir, &meta)
}

/// What syncs an account: its provider, and a bulk or push source.
pub(crate) type SyncSources = (Arc<dyn MailProvider>, Option<Arc<dyn BackfillSource>>);

/// Appended to the agent's system prompt in an agent mailbox: whose
/// mailbox it is and what its service allows.
pub(crate) fn agent_prompt(meta: &AgentMeta) -> String {
    let limits = match meta.service {
        AgentService::Primitive => {
            "Each message goes to exactly one recipient (no Cc or Bcc): to write to several people, \
             write to each separately. Drafts stay on this Mac until sent."
        }
    };
    format!(
        "\n\n## This is an agent's mailbox\n\nThis mailbox, {address}, belongs to an agent called \
         {name}, not to the user. Mail you send from it goes out as {name} <{address}>. Write as \
         {name}, on the user's behalf. {limits}\n",
        address = meta.address,
        name = meta.name,
    )
}

/// A six-digit code standing on its own in `text`.
pub(crate) fn find_code(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let alone_before = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
            let alone_after = i == bytes.len() || !bytes[i].is_ascii_alphanumeric();
            if i - start == 6 && alone_before && alone_after {
                return Some(text[start..i].to_owned());
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Errors from a service, worded for the sheet that asked.
fn service_error(e: ProviderError) -> CoreError {
    match e {
        ProviderError::Invalid(m) | ProviderError::Forbidden(m) => CoreError::new(ErrorKind::InvalidInput, m),
        ProviderError::Unauthorized => {
            CoreError::new(ErrorKind::Auth, "the service refused this mailbox's key; it may have been revoked")
        }
        other => other.into(),
    }
}

/// Agent-mail state: which services to use, and verifications under way.
#[derive(Default)]
pub(crate) struct AgentMailState {
    /// Development and tests: an in-memory service and mailbox, no network.
    fake: AtomicBool,
    /// Tests: another base URL for the real client (a local mock).
    base: Mutex<Option<String>>,
    fake_service: Mutex<Option<Arc<FakeMailboxService>>>,
    fake_mailboxes: Mutex<HashMap<String, Arc<FakeProvider>>>,
    /// When each account's last code was asked for.
    verifications: Mutex<HashMap<String, i64>>,
}

impl Core {
    fn mailbox_service(&self, service: AgentService) -> Result<Arc<dyn MailboxService>, CoreError> {
        let state = &self.agent_mail;
        if state.fake.load(Ordering::SeqCst) {
            let mut fake = state.fake_service.lock().unwrap_or_else(|e| e.into_inner());
            return Ok(fake.get_or_insert_with(Default::default).clone());
        }
        let base = state.base.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match service {
            AgentService::Primitive => Ok(Arc::new(
                provider_primitive::PrimitiveService::with_base(
                    base.as_deref().unwrap_or(provider_primitive::PRIMITIVE_API),
                )
                .map_err(service_error)?,
            )),
        }
    }

    /// An agent mailbox's details, if `account_id` is one.
    pub(crate) fn agent_meta(&self, account_id: &str) -> Option<AgentMeta> {
        read_meta(&accounts_dir(&self.data_path()).join(account_id))
    }

    pub(crate) fn is_agent(&self, account_id: &str) -> bool {
        self.agent_meta(account_id).is_some()
    }

    fn agent_meta_or_err(&self, account_id: &str) -> Result<AgentMeta, CoreError> {
        self.agent_meta(account_id).ok_or_else(|| CoreError::new(ErrorKind::NotFound, "not an agent mailbox"))
    }

    fn agent_key(&self, account_id: &str) -> Result<Redacted<String>, CoreError> {
        secrets::get_redacted(self.secrets.as_ref(), &keys::mailbox_api_key(account_id))?
            .ok_or_else(|| CoreError::new(ErrorKind::Auth, "this agent mailbox's key is missing from the Keychain"))
    }

    /// The provider and push source that sync an agent mailbox.
    pub(crate) fn agent_provider(&self, account_id: &str) -> Result<SyncSources, CoreError> {
        let meta = self.agent_meta_or_err(account_id)?;
        if self.agent_mail.fake.load(Ordering::SeqCst) {
            let mut mailboxes = self.agent_mail.fake_mailboxes.lock().unwrap_or_else(|e| e.into_inner());
            let fake = mailboxes
                .entry(account_id.to_owned())
                .or_insert_with(|| {
                    let fake = FakeProvider::new(&meta.address, mail_sync::now_millis(), 100);
                    fake.set_labels(provider_primitive::labels());
                    Arc::new(fake)
                })
                .clone();
            return Ok((fake, None));
        }
        let key = self.agent_key(account_id)?;
        let tokens = Arc::new(StaticToken(key.expose().clone()));
        match meta.service {
            AgentService::Primitive => {
                let base = self.agent_mail.base.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let provider = Arc::new(
                    provider_primitive::PrimitiveProvider::with_base(
                        tokens,
                        &meta.address,
                        provider_api::RetryPolicy::default(),
                        base.as_deref().unwrap_or(provider_primitive::PRIMITIVE_API),
                    )
                    .map_err(service_error)?,
                );
                let push: Arc<dyn BackfillSource> = Arc::new(provider_primitive::PrimitivePush(provider.clone()));
                Ok((provider, Some(push)))
            }
        }
    }

    /// The fake mailbox behind an agent account in fake mode (tests,
    /// development), to deliver mail into.
    pub(crate) fn fake_agent_mailbox(&self, account_id: &str) -> Option<Arc<FakeProvider>> {
        self.agent_mail.fake_mailboxes.lock().unwrap_or_else(|e| e.into_inner()).get(account_id).cloned()
    }

    /// Agents on the account this work acts on send without approval: an
    /// agent mailbox set to send freely (spec §7.9).
    pub(crate) fn agent_sends_freely(&self) -> bool {
        self.effective_account_id()
            .and_then(|id| self.agent_meta(&id))
            .is_some_and(|m| m.send_mode == AgentSendMode::Freely)
    }

    /// The From display name for sends: the agent's name.
    pub(crate) fn agent_name(&self, account_id: &str) -> Option<String> {
        self.agent_meta(account_id).map(|m| m.name)
    }

    /// Refuse a send the service cannot make (spec §7.9: Primitive takes
    /// one recipient per message).
    pub(crate) fn check_agent_recipients(&self, recipients: usize) -> Result<(), CoreError> {
        let Some(account) = self.effective_account_id() else { return Ok(()) };
        match self.agent_meta(&account).map(|m| m.service) {
            Some(AgentService::Primitive) if recipients > 1 => {
                Err(CoreError::new(ErrorKind::InvalidInput, provider_primitive::ONE_RECIPIENT_ONLY))
            }
            _ => Ok(()),
        }
    }
}

#[uniffi::export]
impl Core {
    /// Create a mailbox for an agent named `name` on `service` (spec §7.9).
    /// Accepts the service's terms: call only from the user's *Agree and
    /// Create*. The account is registered and its key stored; the app
    /// then opens it and starts sync.
    pub async fn create_agent_mailbox(
        self: Arc<Self>,
        service: AgentService,
        name: String,
    ) -> Result<AgentMailboxCreated, CoreError> {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() || name.chars().count() > 100 {
            return Err(CoreError::new(ErrorKind::InvalidInput, "give the agent a name of up to 100 characters"));
        }
        let core = self.clone();
        runtime::run(async move {
            let client = core.mailbox_service(service)?;
            let account_id = crate::account::new_account_id()?;
            // The account id doubles as the idempotency key: a retried call
            // returns the same service account.
            let SignedUp { api_key, address, plan } =
                client.sign_up(&name, &account_id).await.map_err(service_error)?;
            core.secrets.set(keys::mailbox_api_key(&account_id), api_key.expose().clone())?;
            let dir = accounts_dir(&core.data_path()).join(&account_id);
            write_meta(
                &dir,
                &AgentMeta {
                    kind: "agent".into(),
                    service,
                    address: address.clone(),
                    name: name.clone(),
                    created_at: mail_sync::now_millis(),
                    send_mode: AgentSendMode::default(),
                },
            )?;
            let db = core.store_for(&account_id).await?;
            let stored = address.clone();
            db.write(move |tx| mail_store::read::set_sync_state(tx, "account_email", &stored)).await?;
            core.register_account(IndexEntry {
                id: account_id.clone(),
                kind: AccountKind::Agent,
                email: address.clone(),
                display_name: Some(name),
                avatar_file: None,
                added_at: mail_sync::now_millis(),
                imap: None,
                named_by_user: true,
                service: Some(service),
            })
            .await?;
            tracing::info!(account = %account_id, service = client.name(), "agent mailbox created");
            Ok(AgentMailboxCreated { account_id, address, plan: plan.into() })
        })
        .await
    }

    /// The mailbox's plan and limits now, from the service.
    pub async fn agent_mailbox_plan(&self, account_id: String) -> Result<AgentMailboxPlan, CoreError> {
        let meta = self.agent_meta_or_err(&account_id)?;
        let client = self.mailbox_service(meta.service)?;
        let key = self.agent_key(&account_id)?;
        runtime::run(async move { client.plan(key.expose()).await.map(Into::into).map_err(service_error) }).await
    }

    /// Email a verification code to `email`.
    pub async fn start_agent_mailbox_verification(
        &self,
        account_id: String,
        email: String,
    ) -> Result<AgentVerification, CoreError> {
        let email = email.trim().to_owned();
        if !email.contains('@') {
            return Err(CoreError::new(ErrorKind::InvalidInput, "enter an email address"));
        }
        let meta = self.agent_meta_or_err(&account_id)?;
        let client = self.mailbox_service(meta.service)?;
        let key = self.agent_key(&account_id)?;
        let started: VerificationStarted =
            runtime::run(async move { client.start_verification(key.expose(), &email).await.map_err(service_error) })
                .await?;
        self.agent_mail
            .verifications
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(account_id, mail_sync::now_millis());
        Ok(AgentVerification { resend_after_secs: started.resend_after_secs, expires_in_secs: started.expires_in_secs })
    }

    /// Confirm the code; the plan afterwards.
    pub async fn verify_agent_mailbox(&self, account_id: String, code: String) -> Result<AgentMailboxPlan, CoreError> {
        let code = code.trim().to_owned();
        if code.is_empty() {
            return Err(CoreError::new(ErrorKind::InvalidInput, "enter the code from the email"));
        }
        let meta = self.agent_meta_or_err(&account_id)?;
        let client = self.mailbox_service(meta.service)?;
        let key = self.agent_key(&account_id)?;
        let plan = runtime::run(async move { client.verify(key.expose(), &code).await.map_err(service_error) }).await?;
        self.agent_mail.verifications.lock().unwrap_or_else(|e| e.into_inner()).remove(&account_id);
        Ok(plan.into())
    }

    /// The verification code for `account_id`, if it has arrived in the
    /// user's account `in_account_id` since the code was asked for: only
    /// mail from the service's domain is read (spec §7.9). `None` until
    /// then, or when no code was asked for.
    pub async fn find_agent_mailbox_code(
        &self,
        account_id: String,
        in_account_id: String,
    ) -> Result<Option<String>, CoreError> {
        let meta = self.agent_meta_or_err(&account_id)?;
        let Some(asked) =
            self.agent_mail.verifications.lock().unwrap_or_else(|e| e.into_inner()).get(&account_id).copied()
        else {
            return Ok(None);
        };
        let domain = self.mailbox_service(meta.service)?.code_sender_domain();
        let db = self.store_for(&in_account_id).await?;
        let since = asked - CODE_SLACK_MS;
        runtime::run(async move {
            let texts = db.read(move |c| mail_store::read::recent_text_from_domain(c, domain, since, 5)).await?;
            Ok(texts.iter().find_map(|t| find_code(t)))
        })
        .await
    }

    /// The mailbox's API key, for an agent that calls the service itself
    /// (*Copy API Key*, behind a confirmation in the app).
    pub fn agent_mailbox_api_key(&self, account_id: String) -> Result<String, CoreError> {
        self.agent_meta_or_err(&account_id)?;
        Ok(self.agent_key(&account_id)?.expose().clone())
    }

    /// Whether agents send from this mailbox without asking.
    pub fn agent_send_mode(&self, account_id: String) -> Result<AgentSendMode, CoreError> {
        Ok(self.agent_meta_or_err(&account_id)?.send_mode)
    }

    /// Change it (the mailbox's settings, *When Agents Send*).
    pub fn set_agent_send_mode(&self, account_id: String, mode: AgentSendMode) -> Result<(), CoreError> {
        let dir = accounts_dir(&self.data_path()).join(&account_id);
        let mut meta = self.agent_meta_or_err(&account_id)?;
        meta.send_mode = mode;
        write_meta(&dir, &meta)
    }

    /// The service's terms, shown before creating a mailbox.
    pub fn agent_service_terms_url(&self, service: AgentService) -> String {
        match service {
            AgentService::Primitive => provider_primitive::TERMS_URL.to_owned(),
        }
    }

    /// Whether `account_id` is an agent mailbox.
    pub fn account_is_agent(&self, account_id: String) -> bool {
        self.is_agent(&account_id)
    }

    /// Development and tests: create, verify and sync agent mailboxes
    /// against an in-memory service instead of the real one. The fake
    /// accepts the code `123456`.
    pub fn debug_use_fake_agent_mail(&self, enabled: bool) {
        self.agent_mail.fake.store(enabled, Ordering::SeqCst);
    }

    /// Development and tests: deliver a message into a fake agent mailbox
    /// (as if someone wrote to it), then sync.
    pub fn debug_deliver_to_agent_mailbox(
        &self,
        account_id: String,
        from: String,
        subject: String,
        body: String,
    ) -> Result<(), CoreError> {
        if !self.agent_mail.fake.load(Ordering::SeqCst) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "agent mail is not fake in this run"));
        }
        // The fake mailbox exists from the first provider asked for it,
        // which may be this call when sync has not started yet.
        self.agent_provider(&account_id)?;
        let fake = self
            .fake_agent_mailbox(&account_id)
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "no fake mailbox for that account"))?;
        let now = mail_sync::now_millis();
        let n = fake.message_count() + 1;
        fake.deliver(provider_api::FetchedMessage {
            id: mail_domain::MessageId(format!("in:fake{n}")),
            thread_id: mail_domain::ThreadId(format!("in:fake{n}")),
            label_ids: vec![
                mail_domain::LabelId::new(mail_domain::system_labels::INBOX),
                mail_domain::LabelId::new(mail_domain::system_labels::UNREAD),
            ],
            snippet: body.chars().take(200).collect(),
            internal_date: now,
            message_id_header: Some(format!("fake{n}@agent.test")),
            from: mail_mime::parse_headers([("From", from.as_str())]).from,
            subject,
            date: Some(now),
            body: Some(provider_api::FetchedBody { text: Some(body), html: None, attachments: vec![] }),
            ..Default::default()
        });
        self.sync_now();
        Ok(())
    }
}

/// An in-memory agent-mail service: sign-up always works, the code is
/// `123456`. Development and tests only; never a real account.
#[derive(Default)]
pub(crate) struct FakeMailboxService {
    accounts: Mutex<HashMap<String, MailboxPlan>>,
}

fn fake_plan(verified: bool, email: Option<String>) -> MailboxPlan {
    MailboxPlan {
        name: if verified { "developer" } else { "agent" }.into(),
        verified,
        reply_only: !verified,
        send_per_hour: if verified { 1000 } else { 10 },
        send_per_day: if verified { 10_000 } else { 50 },
        email,
    }
}

#[async_trait::async_trait]
impl MailboxService for FakeMailboxService {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn terms_url(&self) -> &'static str {
        provider_primitive::TERMS_URL
    }

    fn code_sender_domain(&self) -> &'static str {
        provider_primitive::CODE_SENDER_DOMAIN
    }

    async fn sign_up(&self, device_name: &str, idempotency_key: &str) -> ProviderResult<SignedUp> {
        let slug: String = device_name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_owned();
        let key = format!("fake_{idempotency_key}");
        let plan = fake_plan(false, None);
        self.accounts.lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone(), plan.clone());
        let local = if slug.is_empty() { "agent".to_owned() } else { slug };
        Ok(SignedUp { api_key: Redacted::new(key), address: format!("{local}@demo.primitive.email"), plan })
    }

    async fn plan(&self, api_key: &str) -> ProviderResult<MailboxPlan> {
        self.accounts.lock().unwrap_or_else(|e| e.into_inner()).get(api_key).cloned().ok_or(ProviderError::Unauthorized)
    }

    async fn start_verification(&self, api_key: &str, email: &str) -> ProviderResult<VerificationStarted> {
        let mut accounts = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        let plan = accounts.get_mut(api_key).ok_or(ProviderError::Unauthorized)?;
        plan.email = Some(email.to_owned());
        Ok(VerificationStarted { resend_after_secs: 30, expires_in_secs: 600 })
    }

    async fn verify(&self, api_key: &str, code: &str) -> ProviderResult<MailboxPlan> {
        let mut accounts = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        let plan = accounts.get_mut(api_key).ok_or(ProviderError::Unauthorized)?;
        if code != "123456" {
            return Err(ProviderError::Invalid("That code is not right".into()));
        }
        *plan = fake_plan(true, plan.email.clone());
        Ok(plan.clone())
    }
}

#[cfg(test)]
mod tests;
