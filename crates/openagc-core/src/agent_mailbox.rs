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
    BackfillSource, DnsRecord, MailProvider, MailboxDomain, MailboxPlan, MailboxService, ProviderError, ProviderResult,
    SignedUp, VerificationStarted,
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

/// A DNS record an own domain needs (spec §7.9).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentDnsRecord {
    /// `MX` or `TXT`.
    pub kind: String,
    pub fqdn: String,
    pub value: String,
    pub priority: Option<u32>,
    /// `inbound_mx`, `ownership_verification`, `spf`, `dkim`, `dmarc`,
    /// `tls_reporting`.
    pub purpose: String,
    pub required: bool,
    /// `pending`, `found`, `missing` or `incorrect`.
    pub status: String,
    pub message: Option<String>,
}

/// One of the user's domains on an agent mailbox's account.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentDomain {
    pub id: String,
    pub domain: String,
    pub verified: bool,
    pub records: Vec<AgentDnsRecord>,
}

impl From<MailboxDomain> for AgentDomain {
    fn from(d: MailboxDomain) -> Self {
        Self {
            id: d.id,
            domain: d.domain,
            verified: d.verified,
            records: d
                .records
                .into_iter()
                .map(|r: DnsRecord| AgentDnsRecord {
                    kind: r.kind,
                    fqdn: r.fqdn,
                    value: r.value,
                    priority: r.priority,
                    purpose: r.purpose,
                    required: r.required,
                    status: r.status,
                    message: r.message,
                })
                .collect(),
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
    /// The address the service gave it, kept when the agent moves to an
    /// own domain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_address: Option<String>,
}

pub(crate) fn read_meta(dir: &Path) -> Option<AgentMeta> {
    let bytes = std::fs::read(dir.join(META_FILE)).ok()?;
    let mut meta = serde_json::from_slice::<AgentMeta>(&bytes).ok().filter(|m| m.kind == "agent")?;
    // Mailboxes created before 2026-10-06 stored Primitive's managed inbox
    // domain as the address; read it as the agent's address there.
    meta.address = mailbox_address(&meta.address, &meta.name);
    meta.managed_address = meta.managed_address.map(|m| mailbox_address(&m, &meta.name));
    Some(meta)
}

/// The agent's name as an address's local part: `Research Scout` →
/// `research-scout`.
pub(crate) fn local_part(name: &str) -> String {
    let dashed: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let local = dashed.split('-').filter(|s| !s.is_empty()).collect::<Vec<_>>().join("-");
    if local.is_empty() { "agent".into() } else { local }
}

/// What a service gives as the mailbox's address, as an address. Primitive
/// answers with its managed inbox's domain (`jade-emu.primitive.email`),
/// which takes mail at any local part: the agent's name is used.
pub(crate) fn mailbox_address(given: &str, name: &str) -> String {
    if given.contains('@') { given.to_owned() } else { format!("{}@{given}", local_part(name)) }
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

    fn agent_client(&self, account_id: &str) -> Result<(Arc<dyn MailboxService>, Redacted<String>), CoreError> {
        let meta = self.agent_meta_or_err(account_id)?;
        Ok((self.mailbox_service(meta.service)?, self.agent_key(account_id)?))
    }

    fn agent_key(&self, account_id: &str) -> Result<Redacted<String>, CoreError> {
        secrets::get_redacted(self.secrets.as_ref(), &keys::mailbox_api_key(account_id))?
            .ok_or_else(|| CoreError::new(ErrorKind::Auth, "this agent mailbox's key is missing from the Keychain"))
    }

    /// Bring the index and the store in line with the mailbox's address
    /// when an older version recorded only its domain (see [`read_meta`]).
    pub(crate) async fn repair_agent_address(&self, account_id: &str, listed: &str) -> Result<(), CoreError> {
        let Some(meta) = self.agent_meta(account_id) else { return Ok(()) };
        if listed == meta.address {
            return Ok(());
        }
        write_meta(&accounts_dir(&self.data_path()).join(account_id), &meta)?;
        let db = self.store_for(account_id).await?;
        let stored = meta.address.clone();
        runtime::run(async move {
            db.write(move |tx| mail_store::read::set_sync_state(tx, "account_email", &stored)).await?;
            Ok(())
        })
        .await?;
        self.register_account(IndexEntry {
            id: account_id.to_owned(),
            kind: AccountKind::Agent,
            email: meta.address.clone(),
            display_name: None,
            avatar_file: None,
            added_at: 0,
            imap: None,
            named_by_user: true,
            service: Some(meta.service),
        })
        .await?;
        tracing::info!(account = %account_id, "agent mailbox's address repaired");
        Ok(())
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
    /// `request_id` is the sheet's own (a UUID): it becomes the account id
    /// and the service's idempotency key, so a retry after a timeout or a
    /// failure part way returns the same service account, never a second.
    pub async fn create_agent_mailbox(
        self: Arc<Self>,
        service: AgentService,
        name: String,
        request_id: String,
    ) -> Result<AgentMailboxCreated, CoreError> {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() || name.chars().count() > 100 {
            return Err(CoreError::new(ErrorKind::InvalidInput, "give the agent a name of up to 100 characters"));
        }
        if !crate::mail::valid_account_id(&request_id) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "request id must be 1-64 of [A-Za-z0-9-]"));
        }
        let core = self.clone();
        runtime::run(async move {
            let client = core.mailbox_service(service)?;
            let account_id = request_id;
            let SignedUp { api_key, address, plan } =
                client.sign_up(&name, &account_id).await.map_err(service_error)?;
            let address = mailbox_address(&address, &name);
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
                    managed_address: Some(address.clone()),
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

    /// The user's own domains on this mailbox's account.
    pub async fn agent_domains(&self, account_id: String) -> Result<Vec<AgentDomain>, CoreError> {
        let (client, key) = self.agent_client(&account_id)?;
        runtime::run(async move {
            Ok(client.domains(key.expose()).await.map_err(service_error)?.into_iter().map(Into::into).collect())
        })
        .await
    }

    /// Add one of the user's domains (a subdomain such as
    /// `agents.example.com` is the usual choice): the DNS records to create.
    pub async fn add_agent_domain(&self, account_id: String, domain: String) -> Result<AgentDomain, CoreError> {
        let domain = domain.trim().trim_end_matches('.').to_lowercase();
        if !domain.contains('.') || domain.contains('@') || domain.contains(char::is_whitespace) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "enter a domain such as agents.example.com"));
        }
        let (client, key) = self.agent_client(&account_id)?;
        runtime::run(
            async move { client.add_domain(key.expose(), &domain).await.map(Into::into).map_err(service_error) },
        )
        .await
    }

    /// Check a domain's records now.
    pub async fn check_agent_domain(&self, account_id: String, domain_id: String) -> Result<AgentDomain, CoreError> {
        let (client, key) = self.agent_client(&account_id)?;
        runtime::run(async move {
            client.verify_domain(key.expose(), &domain_id).await.map(Into::into).map_err(service_error)
        })
        .await
    }

    /// The records as a BIND zone file.
    pub async fn agent_domain_zone_file(&self, account_id: String, domain_id: String) -> Result<String, CoreError> {
        let (client, key) = self.agent_client(&account_id)?;
        runtime::run(async move { client.zone_file(key.expose(), &domain_id).await.map_err(service_error) }).await
    }

    /// Send and receive as `address`: the address the service gave the
    /// mailbox, or one on a verified own domain. Sync restarts with it.
    pub async fn set_agent_address(self: Arc<Self>, account_id: String, address: String) -> Result<(), CoreError> {
        let address = address.trim().to_lowercase();
        let Some((local, domain)) = address.rsplit_once('@') else {
            return Err(CoreError::new(ErrorKind::InvalidInput, "enter an address such as scout@agents.example.com"));
        };
        if local.is_empty() || local.contains(char::is_whitespace) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "enter an address such as scout@agents.example.com"));
        }
        let mut meta = self.agent_meta_or_err(&account_id)?;
        let managed = meta.managed_address.clone().is_some_and(|m| m == address);
        if !managed {
            let domains = self.agent_domains(account_id.clone()).await?;
            if !domains.iter().any(|d| d.verified && d.domain == domain) {
                return Err(CoreError::new(
                    ErrorKind::InvalidInput,
                    format!("{domain} is not one of this mailbox's verified domains"),
                ));
            }
        }
        let dir = accounts_dir(&self.data_path()).join(&account_id);
        let previous = meta.clone();
        meta.address = address.clone();
        write_meta(&dir, &meta)?;
        // The new provider before the old sync stops: if it cannot be made,
        // nothing changes.
        let sources = match self.agent_provider(&account_id) {
            Ok(sources) => sources,
            Err(e) => {
                write_meta(&dir, &previous)?;
                return Err(e);
            }
        };
        let db = self.store_for(&account_id).await?;
        let stored = address.clone();
        runtime::run(async move {
            db.write(move |tx| mail_store::read::set_sync_state(tx, "account_email", &stored)).await?;
            Ok(())
        })
        .await?;
        self.register_account(IndexEntry {
            id: account_id.clone(),
            kind: AccountKind::Agent,
            email: address,
            display_name: None,
            avatar_file: None,
            added_at: 0,
            imap: None,
            named_by_user: true,
            service: Some(meta.service),
        })
        .await?;
        // Sends go from the new address: restart its sync with it.
        if self.is_syncing(&account_id) {
            self.stop_sync_for(&account_id);
            let (provider, push) = sources;
            crate::registry::SCOPED_ACCOUNT.sync_scope(account_id, || self.start_sync_with_backfill(provider, push))?;
        }
        Ok(())
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
    /// Domains by key; each check finds the records, as if DNS had caught up.
    domains: Mutex<HashMap<String, Vec<MailboxDomain>>>,
}

fn fake_records(domain: &str, status: &str) -> Vec<DnsRecord> {
    let record = |kind: &str, fqdn: String, value: &str, purpose: &str| DnsRecord {
        kind: kind.into(),
        fqdn,
        value: value.into(),
        priority: (kind == "MX").then_some(10),
        purpose: purpose.into(),
        required: true,
        status: status.into(),
        message: None,
    };
    vec![
        record("MX", domain.to_owned(), "in.demo.primitive.email", "inbound_mx"),
        record("TXT", domain.to_owned(), "v=spf1 include:demo.primitive.email ~all", "spf"),
        record("TXT", format!("prim._domainkey.{domain}"), "v=DKIM1; k=rsa; p=MIGf…", "dkim"),
        record("TXT", format!("_dmarc.{domain}"), "v=DMARC1; p=none", "dmarc"),
    ]
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
        let plan = self
            .accounts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key.clone())
            .or_insert_with(|| fake_plan(false, None))
            .clone();
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

    async fn domains(&self, api_key: &str) -> ProviderResult<Vec<MailboxDomain>> {
        Ok(self.domains.lock().unwrap_or_else(|e| e.into_inner()).get(api_key).cloned().unwrap_or_default())
    }

    async fn add_domain(&self, api_key: &str, domain: &str) -> ProviderResult<MailboxDomain> {
        // A bare domain stands for one whose mail goes elsewhere.
        if domain.matches('.').count() < 2 {
            return Err(ProviderError::Invalid(provider_primitive::DOMAIN_RECEIVES_ELSEWHERE.into()));
        }
        let mut all = self.domains.lock().unwrap_or_else(|e| e.into_inner());
        let list = all.entry(api_key.to_owned()).or_default();
        if let Some(existing) = list.iter().find(|d| d.domain == domain) {
            return Ok(existing.clone());
        }
        let added = MailboxDomain {
            id: format!("fake-domain-{}", list.len() + 1),
            domain: domain.to_owned(),
            verified: false,
            records: fake_records(domain, "pending"),
        };
        list.push(added.clone());
        Ok(added)
    }

    async fn verify_domain(&self, api_key: &str, domain_id: &str) -> ProviderResult<MailboxDomain> {
        let mut all = self.domains.lock().unwrap_or_else(|e| e.into_inner());
        let domain = all
            .get_mut(api_key)
            .and_then(|l| l.iter_mut().find(|d| d.id == domain_id))
            .ok_or_else(|| ProviderError::NotFound(domain_id.to_owned()))?;
        domain.verified = true;
        domain.records = fake_records(&domain.domain, "found");
        Ok(domain.clone())
    }

    async fn zone_file(&self, api_key: &str, domain_id: &str) -> ProviderResult<String> {
        let all = self.domains.lock().unwrap_or_else(|e| e.into_inner());
        let domain = all
            .get(api_key)
            .and_then(|l| l.iter().find(|d| d.id == domain_id))
            .ok_or_else(|| ProviderError::NotFound(domain_id.to_owned()))?;
        Ok(fake_records(&domain.domain, "pending")
            .iter()
            .map(|r| format!("{}. 300 IN {} {}\n", r.fqdn, r.kind, r.value))
            .collect())
    }
}

#[cfg(test)]
mod tests;
