//! Agent mailboxes (spec §7.9, ADR 0014): an address for one of the
//! user's agents on an agent-mail service, created and verified here with
//! the service's own API, then synced like any account.
//!
//! The account directory holds `agent.json` (service, address, name, and
//! its service account); the key, plan, verification and own domains
//! belong to the service account (ADR 0015, [`service_account`]), whose
//! key is in the Keychain as `mailbox.api_key.<service account id>`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use mail_domain::Redacted;
use provider_api::fake::FakeProvider;
use provider_api::token::StaticToken;
use provider_api::{
    BackfillSource, DnsRecord, MailProvider, MailboxDomain, MailboxPlan, MailboxService, ProviderError, ProviderResult,
    SendRule, SignedUp, VerificationStarted,
};
use serde::{Deserialize, Serialize};

use crate::registry::{AccountKind, IndexEntry, accounts_dir};
use crate::secrets::keys;
use crate::{Core, CoreError, ErrorKind, runtime};

mod service_account;

pub use service_account::{AgentAdded, ServiceAccountSummary};

const META_FILE: &str = "agent.json";
/// How far back to look for a verification code before *Send Code*: the
/// message can arrive a little before the clocks agree.
const CODE_SLACK_MS: i64 = 2 * 60 * 1000;

/// The agent-mail services OpenAGC can create mailboxes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum AgentService {
    Primitive,
    /// AgentMail (agentmail.to): an organisation per human email, an inbox
    /// per agent (spec §7.9).
    AgentMail,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
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

/// Where an agent mailbox may send (spec §7.9): `any_recipient`,
/// `managed_zone` (a zone), `your_domain` (a domain) or `address` (one).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentSendRule {
    pub kind: String,
    pub value: Option<String>,
}

impl From<SendRule> for AgentSendRule {
    fn from(r: SendRule) -> Self {
        match r {
            SendRule::AnyRecipient => Self { kind: "any_recipient".into(), value: None },
            SendRule::ManagedZone(z) => Self { kind: "managed_zone".into(), value: Some(z) },
            SendRule::YourDomain(d) => Self { kind: "your_domain".into(), value: Some(d) },
            SendRule::Address(a) => Self { kind: "address".into(), value: Some(a) },
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
    /// The service account it belongs to (ADR 0015). Mailboxes created
    /// before 2026-10-08 have none: see [`read_meta`].
    #[serde(default)]
    pub service_account: String,
    /// AgentMail: the service's id for its inbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox_id: Option<String>,
}

pub(crate) fn read_meta(dir: &Path) -> Option<AgentMeta> {
    let bytes = std::fs::read(dir.join(META_FILE)).ok()?;
    let mut meta = serde_json::from_slice::<AgentMeta>(&bytes).ok().filter(|m| m.kind == "agent")?;
    // Mailboxes created before 2026-10-06 stored Primitive's managed inbox
    // domain as the address; read it as the agent's address there.
    meta.address = mailbox_address(&meta.address, &meta.name);
    meta.managed_address = meta.managed_address.map(|m| mailbox_address(&m, &meta.name));
    // Mailboxes created before 2026-10-08 are a service account of their
    // own, under their account id: the Keychain item keeps its name
    // (`service_account` module docs).
    if meta.service_account.is_empty() {
        meta.service_account = dir.file_name()?.to_string_lossy().into_owned();
    }
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

/// What a service lets an agent mailbox do, in words for the agent's
/// prompt and the app (composer, settings): `verified` and `human_email`
/// are its service account's.
pub(crate) fn limits_text(service: AgentService, verified: bool, human_email: Option<&str>) -> String {
    match service {
        AgentService::Primitive => "Each message goes to exactly one recipient (no Cc or Bcc): to write to \
             several people, write to each separately. Drafts stay on this Mac until sent."
            .into(),
        AgentService::AgentMail => {
            let before = if verified {
                String::new()
            } else {
                let human = human_email.map_or_else(|| "the email it was created with".to_owned(), str::to_owned);
                format!(
                    "Until its service account is verified, it can write only to {human}; AgentMail refuses \
                     anyone else. "
                )
            };
            format!(
                "{before}A message may go to several people (To, Cc and Bcc). AgentMail's free plan sends 3,000 \
                 messages a month across the service account, and a new inbox may write to at most 3 different \
                 people in its first hour, 5 in its first day and 10 in its first week. A message with its \
                 attachments may be up to 6 MB. Drafts stay on this Mac until sent."
            )
        }
    }
}

/// Appended to the agent's system prompt in an agent mailbox: whose
/// mailbox it is and what its service allows (`limits`, from
/// [`limits_text`]). `others` names the other agents of its service
/// account, whose sends count against the same limits.
pub(crate) fn agent_prompt(meta: &AgentMeta, others: &[String], limits: &str) -> String {
    let shared = match others {
        [] => String::new(),
        [one] => format!(
            " This mailbox shares its service account's sending limits with the mailbox of another agent, \
             {one}: its sends count against them too."
        ),
        many => format!(
            " This mailbox shares its service account's sending limits with the mailboxes of {} other \
             agents ({}): their sends count against them too.",
            many.len(),
            many.join(", ")
        ),
    };
    format!(
        "\n\n## This is an agent's mailbox\n\nThis mailbox, {address}, belongs to an agent called \
         {name}, not to the user. Mail you send from it goes out as {name} <{address}>. Write as \
         {name}, on the user's behalf. {limits}{shared}\n",
        address = meta.address,
        name = meta.name,
    )
}

/// The agent's addresses: its address, and the service's own one when it
/// has moved to an own domain.
fn addresses_of(meta: &AgentMeta) -> Vec<String> {
    let mut out = vec![meta.address.clone()];
    out.extend(meta.managed_address.clone().filter(|m| *m != meta.address));
    out
}

/// Which of its Primitive account's mail is `account_id`'s, from the
/// agents on disk now (spec §7.9, ADR 0015): its own addresses, the other
/// agents', and whether it is the service account's first agent (the
/// oldest), which keeps mail to addresses no agent has. `fallback` is the
/// agent as its sync started, should its directory be gone (being removed).
pub(crate) fn primitive_routing(
    data_dir: &Path,
    account_id: &str,
    fallback: &AgentMeta,
) -> provider_primitive::Routing {
    let agents: Vec<(String, AgentMeta)> = service_account::agents_on_disk(data_dir)
        .into_iter()
        .filter(|(_, m)| m.service_account == fallback.service_account)
        .collect();
    let first = agents.iter().min_by(|a, b| (a.1.created_at, &a.0).cmp(&(b.1.created_at, &b.0))).map(|(id, _)| id);
    let own =
        agents.iter().find(|(id, _)| id == account_id).map_or_else(|| addresses_of(fallback), |(_, m)| addresses_of(m));
    provider_primitive::Routing {
        own,
        others: agents.iter().filter(|(id, _)| id != account_id).flat_map(|(_, m)| addresses_of(m)).collect(),
        catch_all: first.is_none_or(|f| f == account_id),
    }
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
    fake_services: Mutex<HashMap<AgentService, Arc<FakeMailboxService>>>,
    fake_mailboxes: Mutex<HashMap<String, Arc<FakeProvider>>>,
    /// When each service account's last code was asked for.
    verifications: Mutex<HashMap<String, i64>>,
    /// Held while a service account's record is read and rewritten.
    records: Mutex<()>,
    /// Held while an agent is added, so two cannot take one address.
    adding: tokio::sync::Mutex<()>,
    /// One rate limiter per service account: its agents share the key's
    /// request limit.
    limiters: Mutex<HashMap<String, Arc<provider_api::RateLimiter>>>,
}

impl Core {
    pub(crate) fn mailbox_service(&self, service: AgentService) -> Result<Arc<dyn MailboxService>, CoreError> {
        let state = &self.agent_mail;
        if state.fake.load(Ordering::SeqCst) {
            let mut fakes = state.fake_services.lock().unwrap_or_else(|e| e.into_inner());
            return Ok(fakes.entry(service).or_insert_with(|| Arc::new(FakeMailboxService::new(service))).clone());
        }
        let base = state.base.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match service {
            AgentService::Primitive => Ok(Arc::new(
                provider_primitive::PrimitiveService::with_base(
                    base.as_deref().unwrap_or(provider_primitive::PRIMITIVE_API),
                )
                .map_err(service_error)?,
            )),
            AgentService::AgentMail => Ok(Arc::new(
                provider_agentmail::AgentMailService::with_base(
                    base.as_deref().unwrap_or(provider_agentmail::AGENTMAIL_API),
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

    pub(crate) fn agent_meta_or_err(&self, account_id: &str) -> Result<AgentMeta, CoreError> {
        self.agent_meta(account_id).ok_or_else(|| CoreError::new(ErrorKind::NotFound, "not an agent mailbox"))
    }

    /// The agent's key: its service account's.
    fn agent_key(&self, account_id: &str) -> Result<Redacted<String>, CoreError> {
        self.service_key(&self.service_of(account_id)?)
    }

    /// The Keychain item holding an agent's key: its service account's.
    pub(crate) fn agent_key_name(&self, account_id: &str) -> Option<String> {
        self.agent_meta(account_id).map(|m| keys::mailbox_api_key(&m.service_account))
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
                    fake.set_labels(match meta.service {
                        AgentService::Primitive => provider_primitive::labels(),
                        AgentService::AgentMail => provider_agentmail::labels(),
                    });
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
                let limiter = self
                    .agent_mail
                    .limiters
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(meta.service_account.clone())
                    .or_insert_with(provider_primitive::rate_limiter)
                    .clone();
                // Which mail is this agent's, read from disk as it syncs, so
                // agents added or removed later are seen (spec §7.9).
                let (data_dir, id, agent) = (self.data_path(), account_id.to_owned(), meta.clone());
                let routing: provider_primitive::RoutingSource =
                    Arc::new(move || primitive_routing(&data_dir, &id, &agent));
                let provider = Arc::new(
                    provider_primitive::PrimitiveProvider::for_agent(
                        tokens,
                        &meta.address,
                        routing,
                        limiter,
                        provider_api::RetryPolicy::default(),
                        base.as_deref().unwrap_or(provider_primitive::PRIMITIVE_API),
                    )
                    .map_err(service_error)?,
                );
                let push: Arc<dyn BackfillSource> = Arc::new(provider_primitive::PrimitivePush(provider.clone()));
                Ok((provider, Some(push)))
            }
            // One inbox of the organisation; polled (no push source yet).
            AgentService::AgentMail => {
                let base = self.agent_mail.base.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let limiter = self
                    .agent_mail
                    .limiters
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(meta.service_account.clone())
                    .or_insert_with(provider_agentmail::rate_limiter)
                    .clone();
                let inbox = meta.inbox_id.clone().unwrap_or_else(|| meta.address.clone());
                let provider = provider_agentmail::AgentMailProvider::for_inbox(
                    tokens,
                    &meta.address,
                    &inbox,
                    limiter,
                    provider_api::RetryPolicy::default(),
                    base.as_deref().unwrap_or(provider_agentmail::AGENTMAIL_API),
                )
                .map_err(service_error)?;
                Ok((Arc::new(provider), None))
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

    /// The names of the other agents on `meta`'s service account.
    pub(crate) fn fellow_agents(&self, account_id: &str, meta: &AgentMeta) -> Vec<String> {
        service_account::agents_on_disk(&self.data_path())
            .into_iter()
            .filter(|(id, m)| id != account_id && m.service_account == meta.service_account)
            .map(|(_, m)| m.name)
            .collect()
    }

    /// The From display name for sends: the agent's name.
    pub(crate) fn agent_name(&self, account_id: &str) -> Option<String> {
        self.agent_meta(account_id).map(|m| m.name)
    }

    /// Refuse a send the service cannot make (spec §7.9): Primitive takes
    /// one recipient per message; AgentMail, until its service account is
    /// verified, writes only to the human email.
    pub(crate) fn check_agent_recipients(&self, recipients: &[String]) -> Result<(), CoreError> {
        let Some(account) = self.effective_account_id() else { return Ok(()) };
        let Some(meta) = self.agent_meta(&account) else { return Ok(()) };
        match meta.service {
            AgentService::Primitive if recipients.len() > 1 => {
                Err(CoreError::new(ErrorKind::InvalidInput, provider_primitive::ONE_RECIPIENT_ONLY))
            }
            AgentService::Primitive => Ok(()),
            AgentService::AgentMail => {
                let Ok(service) = self.service_meta(&meta.service_account) else { return Ok(()) };
                if service.verified {
                    return Ok(());
                }
                let human = service.human_email.unwrap_or_default();
                let others: Vec<&String> =
                    recipients.iter().filter(|r| !r.trim().eq_ignore_ascii_case(human.trim())).collect();
                if others.is_empty() {
                    return Ok(());
                }
                Err(CoreError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "Until this AgentMail service account is verified, it can write only to {}. Verify it \
                         in Settings › Accounts, or write to {} alone.",
                        if human.is_empty() { "the email it was created with" } else { human.as_str() },
                        if human.is_empty() { "that email" } else { human.as_str() },
                    ),
                ))
            }
        }
    }

    /// AgentMail keeps one organisation per human email, and signing up
    /// again with it returns that organisation with a new key, breaking
    /// every agent holding the old one (ADR 0015). Refuse before asking:
    /// an email with an AgentMail service account here, or a request that
    /// already got a key.
    fn refuse_second_sign_up(&self, email: &str, request_id: &str) -> Result<(), CoreError> {
        let data_dir = self.data_path();
        let mut ids: Vec<String> =
            service_account::agents_on_disk(&data_dir).into_iter().map(|(_, m)| m.service_account).collect();
        ids.sort();
        ids.dedup();
        let existing = ids.iter().find(|id| {
            service_account::read_service(&data_dir, id).is_some_and(|s| {
                s.service == AgentService::AgentMail
                    && s.human_email.as_deref().is_some_and(|h| h.trim().eq_ignore_ascii_case(email.trim()))
            })
        });
        if existing.is_some() {
            return Err(CoreError::new(
                ErrorKind::InvalidInput,
                format!(
                    "{email} already has an AgentMail service account here. Add the agent to it instead: signing \
                     up again would replace the key its agents use."
                ),
            ));
        }
        if crate::secrets::get_redacted(self.secrets.as_ref(), &keys::mailbox_api_key(request_id))?.is_some() {
            return Err(CoreError::new(
                ErrorKind::InvalidInput,
                "this AgentMail sign-up already went through once; signing up again would replace its key",
            ));
        }
        Ok(())
    }

    /// The limits text for an agent mailbox (see [`limits_text`]).
    pub(crate) fn agent_limits_text(&self, meta: &AgentMeta) -> String {
        let service = self.service_meta(&meta.service_account).ok();
        limits_text(
            meta.service,
            service.as_ref().is_some_and(|s| s.verified),
            service.as_ref().and_then(|s| s.human_email.as_deref()),
        )
    }
}

#[uniffi::export]
impl Core {
    /// Create a mailbox for an agent named `name` on `service` (spec §7.9),
    /// in a new service account. Accepts the service's terms: call only
    /// from the user's *Agree and Create*. The account is registered and
    /// its key stored; the app then opens it and starts sync.
    /// `request_id` is the sheet's own (a UUID): it becomes the account id,
    /// the service account's id and the service's idempotency key, so a
    /// retry after a timeout or a failure part way returns the same
    /// service account, never a second.
    ///
    /// `human_email` is the user's email: AgentMail needs it (it sends the
    /// verification code there at once, and without it the mailbox only
    /// receives); Primitive ignores it. AgentMail keeps one organisation per
    /// human email and signing up again rotates its key, so this refuses an
    /// email that already has an AgentMail service account here: add the
    /// agent to it with `add_agent` instead.
    pub async fn create_agent_mailbox(
        self: Arc<Self>,
        service: AgentService,
        name: String,
        human_email: Option<String>,
        request_id: String,
    ) -> Result<AgentMailboxCreated, CoreError> {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() || name.chars().count() > 100 {
            return Err(CoreError::new(ErrorKind::InvalidInput, "give the agent a name of up to 100 characters"));
        }
        if !crate::mail::valid_account_id(&request_id) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "request id must be 1-64 of [A-Za-z0-9-]"));
        }
        let human_email = human_email.map(|e| e.trim().to_owned()).filter(|e| !e.is_empty());
        if service == AgentService::AgentMail {
            let Some(email) = human_email.as_deref() else {
                return Err(CoreError::new(
                    ErrorKind::InvalidInput,
                    "AgentMail needs your email: it sends the code there, and without it the mailbox cannot send",
                ));
            };
            if !email.contains('@') {
                return Err(CoreError::new(ErrorKind::InvalidInput, "enter an email address"));
            }
            // A retry of a creation that finished on this Mac.
            if let Some(existing) = read_meta(&accounts_dir(&self.data_path()).join(&request_id)) {
                return Ok(AgentMailboxCreated {
                    account_id: request_id.clone(),
                    address: existing.address,
                    plan: self
                        .service_meta(&existing.service_account)
                        .ok()
                        .and_then(|s| s.plan)
                        .unwrap_or_else(|| provider_agentmail_plan(false)),
                });
            }
            self.refuse_second_sign_up(email, &request_id)?;
        }
        let core = self.clone();
        // AgentMail emails the code at sign-up: mail from then on may carry
        // it (*Fill Code from <address>*), as after *Send Code*.
        let asked = mail_sync::now_millis();
        runtime::run(async move {
            let client = core.mailbox_service(service)?;
            let account_id = request_id;
            let SignedUp { api_key, address, plan, inbox_id } =
                client.sign_up(&name, &account_id, human_email.as_deref()).await.map_err(service_error)?;
            if service == AgentService::AgentMail && !plan.verified {
                core.agent_mail
                    .verifications
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(account_id.clone(), asked);
            }
            let address = mailbox_address(&address, &name);
            let mut plan: AgentMailboxPlan = plan.into();
            if service == AgentService::AgentMail {
                plan.email = plan.email.or_else(|| human_email.clone());
            }
            // The service account takes the id of its first agent.
            core.secrets.set(keys::mailbox_api_key(&account_id), api_key.expose().clone())?;
            {
                let _guard = core.agent_mail.records.lock().unwrap_or_else(|e| e.into_inner());
                let mut record = service_account::read_service(&core.data_path(), &account_id).unwrap_or(
                    service_account::ServiceMeta {
                        service,
                        created_at: mail_sync::now_millis(),
                        human_email: None,
                        verified: false,
                        plan: None,
                        managed_domain: service_account::domain_of(&address),
                        domains: vec![],
                    },
                );
                record.verified = plan.verified;
                record.human_email = plan.email.clone().or(record.human_email).or_else(|| human_email.clone());
                record.plan = Some(plan.clone());
                service_account::write_service(&core.data_path(), &account_id, &record)?;
            }
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
                    service_account: account_id.clone(),
                    inbox_id,
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
            Ok(AgentMailboxCreated { account_id, address, plan })
        })
        .await
    }

    // The calls below take an agent's account id and act on its service
    // account (ADR 0015); the app moves to the `service_account_*` calls.

    /// The plan and limits now, from the service: the agent's service
    /// account's.
    pub async fn agent_mailbox_plan(&self, account_id: String) -> Result<AgentMailboxPlan, CoreError> {
        self.service_account_plan(self.service_of(&account_id)?).await
    }

    /// Email a verification code to `email`, for the agent's service account.
    pub async fn start_agent_mailbox_verification(
        &self,
        account_id: String,
        email: String,
    ) -> Result<AgentVerification, CoreError> {
        self.start_service_account_verification(self.service_of(&account_id)?, email).await
    }

    /// Confirm the code; the plan afterwards.
    pub async fn verify_agent_mailbox(&self, account_id: String, code: String) -> Result<AgentMailboxPlan, CoreError> {
        self.verify_service_account(self.service_of(&account_id)?, code).await
    }

    /// The verification code for the agent's service account, if it has
    /// arrived in the user's account `in_account_id` (see
    /// `find_service_account_code`).
    pub async fn find_agent_mailbox_code(
        &self,
        account_id: String,
        in_account_id: String,
    ) -> Result<Option<String>, CoreError> {
        self.find_service_account_code(self.service_of(&account_id)?, in_account_id).await
    }

    /// The agent's service account's API key (*Copy API Key*).
    pub fn agent_mailbox_api_key(&self, account_id: String) -> Result<String, CoreError> {
        self.service_account_api_key(self.service_of(&account_id)?)
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

    /// Where this mailbox may send now, from the service, broadest first.
    pub async fn agent_send_rules(&self, account_id: String) -> Result<Vec<AgentSendRule>, CoreError> {
        self.service_account_send_rules(self.service_of(&account_id)?).await
    }

    /// The user's own domains on the agent's service account.
    pub async fn agent_domains(&self, account_id: String) -> Result<Vec<AgentDomain>, CoreError> {
        self.service_account_domains(self.service_of(&account_id)?).await
    }

    /// Add one of the user's domains to the agent's service account.
    pub async fn add_agent_domain(&self, account_id: String, domain: String) -> Result<AgentDomain, CoreError> {
        self.add_service_account_domain(self.service_of(&account_id)?, domain).await
    }

    /// Check a domain's records now.
    pub async fn check_agent_domain(&self, account_id: String, domain_id: String) -> Result<AgentDomain, CoreError> {
        self.check_service_account_domain(self.service_of(&account_id)?, domain_id).await
    }

    /// The records as a BIND zone file.
    pub async fn agent_domain_zone_file(&self, account_id: String, domain_id: String) -> Result<String, CoreError> {
        self.service_account_domain_zone_file(self.service_of(&account_id)?, domain_id).await
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
        let taken = service_account::agents_on_disk(&self.data_path()).into_iter().any(|(other, a)| {
            other != account_id
                && a.service_account == meta.service_account
                && (a.address == address || a.managed_address.as_deref() == Some(address.as_str()))
        });
        if taken {
            return Err(CoreError::new(
                ErrorKind::InvalidInput,
                format!("another agent of this service account is {address}"),
            ));
        }
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
            AgentService::AgentMail => provider_agentmail::TERMS_URL.to_owned(),
        }
    }

    /// What an agent mailbox on `service` may do, in words for the
    /// composer and settings (spec §7.9): `verified` and `human_email` are
    /// its service account's. The agent's prompt says the same.
    pub fn agent_service_limits(&self, service: AgentService, verified: bool, human_email: Option<String>) -> String {
        limits_text(service, verified, human_email.as_deref())
    }

    /// Where the service's dashboard signs in; the user signs in with the
    /// email the mailbox was verified with.
    pub fn agent_service_dashboard_url(&self, service: AgentService) -> String {
        match service {
            AgentService::Primitive => provider_primitive::DASHBOARD_URL.to_owned(),
            AgentService::AgentMail => provider_agentmail::DASHBOARD_URL.to_owned(),
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
/// `123456`. Development and tests only; never a real account. As
/// Primitive, one service account takes mail at any local part of its
/// domain, so several agents can share it (`Core::add_agent`); as
/// AgentMail, an organisation per human email holds up to three inboxes,
/// and signing up again with the same email rotates its key (counted, so
/// tests can show the core never does it). Each agent still has a fake
/// mailbox of its own.
pub(crate) struct FakeMailboxService {
    service: AgentService,
    accounts: Mutex<HashMap<String, MailboxPlan>>,
    rotations: std::sync::atomic::AtomicU32,
    /// Domains by key; each check finds the records, as if DNS had caught up.
    domains: Mutex<HashMap<String, Vec<MailboxDomain>>>,
    /// AgentMail: the organisation's key by human email.
    organisations: Mutex<HashMap<String, String>>,
    /// AgentMail: sign-ups that repeated an organisation's email.
    pub(crate) repeated_sign_ups: std::sync::atomic::AtomicU32,
    /// AgentMail: inbox addresses by key.
    inboxes: Mutex<HashMap<String, Vec<String>>>,
}

impl FakeMailboxService {
    pub(crate) fn new(service: AgentService) -> Self {
        Self {
            service,
            accounts: Mutex::default(),
            rotations: Default::default(),
            domains: Mutex::default(),
            organisations: Mutex::default(),
            repeated_sign_ups: Default::default(),
            inboxes: Mutex::default(),
        }
    }

    fn plan_for(&self, verified: bool, email: Option<String>) -> MailboxPlan {
        match self.service {
            AgentService::Primitive => fake_plan(verified, email),
            AgentService::AgentMail => MailboxPlan {
                name: "free".into(),
                verified,
                reply_only: false,
                send_per_hour: 0,
                send_per_day: 0,
                email,
            },
        }
    }
}

/// AgentMail's plan as signed up, before the service is asked.
pub(crate) fn provider_agentmail_plan(verified: bool) -> AgentMailboxPlan {
    AgentMailboxPlan {
        name: provider_agentmail::plan_name(Some(3)),
        verified,
        reply_only: false,
        send_per_hour: 0,
        send_per_day: 0,
        email: None,
    }
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
        match self.service {
            AgentService::Primitive => provider_primitive::TERMS_URL,
            AgentService::AgentMail => provider_agentmail::TERMS_URL,
        }
    }

    fn code_sender_domain(&self) -> &'static str {
        match self.service {
            AgentService::Primitive => provider_primitive::CODE_SENDER_DOMAIN,
            AgentService::AgentMail => provider_agentmail::CODE_SENDER_DOMAIN,
        }
    }

    async fn sign_up(
        &self,
        device_name: &str,
        idempotency_key: &str,
        human_email: Option<&str>,
    ) -> ProviderResult<SignedUp> {
        let local = local_part(device_name);
        let key = format!("fake_{idempotency_key}");
        if self.service == AgentService::AgentMail {
            let address = format!("{local}@{}", provider_agentmail::MANAGED_DOMAIN);
            let email = human_email.map(str::to_lowercase);
            let known = email
                .as_ref()
                .and_then(|e| self.organisations.lock().unwrap_or_else(|p| p.into_inner()).get(e).cloned());
            if let Some(old) = known {
                // AgentMail: the same organisation with a new key; the old
                // one stops working.
                self.repeated_sign_ups.fetch_add(1, Ordering::SeqCst);
                let fresh = self.rotate_key(&old).await?;
                self.organisations
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(email.unwrap_or_default(), fresh.expose().clone());
                let plan = self.plan(fresh.expose()).await?;
                return Ok(SignedUp { api_key: fresh, address: address.clone(), plan, inbox_id: Some(address) });
            }
            let taken =
                self.inboxes.lock().unwrap_or_else(|e| e.into_inner()).values().flatten().any(|a| *a == address);
            if taken {
                return Err(ProviderError::Invalid(provider_agentmail::USERNAME_TAKEN.into()));
            }
            if let Some(email) = email.clone() {
                self.organisations.lock().unwrap_or_else(|p| p.into_inner()).insert(email, key.clone());
            }
            self.inboxes.lock().unwrap_or_else(|e| e.into_inner()).insert(key.clone(), vec![address.clone()]);
            let plan = self
                .accounts
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(key.clone())
                .or_insert_with(|| self.plan_for(false, email))
                .clone();
            return Ok(SignedUp {
                api_key: Redacted::new(key),
                address: address.clone(),
                plan,
                inbox_id: Some(address),
            });
        }
        let plan = self
            .accounts
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(key.clone())
            .or_insert_with(|| fake_plan(false, None))
            .clone();
        Ok(SignedUp {
            api_key: Redacted::new(key),
            address: format!("{local}@demo.primitive.email"),
            plan,
            inbox_id: None,
        })
    }

    async fn add_mailbox(
        &self,
        api_key: &str,
        username: &str,
        domain: Option<&str>,
        _display_name: &str,
        _idempotency_key: &str,
    ) -> ProviderResult<provider_api::AddedMailbox> {
        if self.service != AgentService::AgentMail {
            return Err(ProviderError::Unavailable("this service adds no mailboxes to an account".into()));
        }
        self.plan(api_key).await?;
        let address = format!("{username}@{}", domain.unwrap_or(provider_agentmail::MANAGED_DOMAIN));
        let mut inboxes = self.inboxes.lock().unwrap_or_else(|e| e.into_inner());
        if inboxes.values().flatten().any(|a| *a == address) {
            return Err(ProviderError::Invalid(provider_agentmail::USERNAME_TAKEN.into()));
        }
        let mine = inboxes.entry(api_key.to_owned()).or_default();
        if mine.len() >= 3 {
            return Err(ProviderError::Invalid(provider_agentmail::INBOX_LIMIT.into()));
        }
        mine.push(address.clone());
        Ok(provider_api::AddedMailbox { address: address.clone(), inbox_id: address })
    }

    async fn mailbox_api_key(&self, api_key: &str, inbox_id: &str, _name: &str) -> ProviderResult<Redacted<String>> {
        if self.service != AgentService::AgentMail {
            return Err(ProviderError::Unavailable("this service has no keys for one mailbox".into()));
        }
        if !self.plan(api_key).await?.verified {
            return Err(ProviderError::Forbidden(provider_agentmail::NOT_VERIFIED_YET.into()));
        }
        Ok(Redacted::new(format!("fake_inbox_{inbox_id}")))
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
        *plan = self.plan_for(true, plan.email.clone());
        Ok(plan.clone())
    }

    async fn rotate_key(&self, api_key: &str) -> ProviderResult<Redacted<String>> {
        let mut accounts = self.accounts.lock().unwrap_or_else(|e| e.into_inner());
        let plan = accounts.remove(api_key).ok_or(ProviderError::Unauthorized)?;
        let fresh = format!("{api_key}-r{}", self.rotations.fetch_add(1, Ordering::SeqCst) + 1);
        accounts.insert(fresh.clone(), plan);
        let mut domains = self.domains.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(list) = domains.remove(api_key) {
            domains.insert(fresh.clone(), list);
        }
        let mut inboxes = self.inboxes.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(list) = inboxes.remove(api_key) {
            inboxes.insert(fresh.clone(), list);
        }
        Ok(Redacted::new(fresh))
    }

    async fn send_rules(&self, api_key: &str) -> ProviderResult<Vec<SendRule>> {
        let plan = self.plan(api_key).await?;
        if self.service == AgentService::AgentMail {
            return Ok(vec![]);
        }
        let mut rules = vec![SendRule::ManagedZone("primitive.email".into())];
        rules.extend(
            self.domains(api_key).await?.into_iter().filter(|d| d.verified).map(|d| SendRule::YourDomain(d.domain)),
        );
        rules.extend(plan.email.filter(|_| plan.verified).map(SendRule::Address));
        Ok(rules)
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
