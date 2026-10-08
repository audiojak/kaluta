//! Service accounts (spec §7.9, ADR 0015): what an agent-mail service calls
//! an organisation (AgentMail) or an account (Primitive). One holds the API
//! key, the human email, the plan and the user's own domains, shared by its
//! agents, each of which is an account of its own (ADR 0004).
//!
//! The record is `services/<id>/service.json` in the data directory and the
//! key is in the Keychain as `mailbox.api_key.<id>`. A service account
//! takes the id of the agent it was created with.
//!
//! **Migration.** A mailbox made before 2026-10-08 has no `service_account`
//! in its `agent.json` and no `service.json`. It is migrated on read, not
//! rewritten: [`super::read_meta`] gives such an agent its own account id
//! as its service account, and [`read_service`] makes the record up from
//! that agent's `agent.json`. So the Keychain item keeps its name and
//! nothing is re-keyed or signed up again. The record is written the first
//! time something in it changes (a plan read, a verification, a domain, a
//! second agent, the first agent's removal while others remain), and
//! `agent.json` gains the field the next time it is written. Reading again
//! gives the same answer, so the migration is idempotent, and it never
//! deletes anything.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mail_domain::Redacted;
use provider_api::MailboxService;
use serde::{Deserialize, Serialize};

use super::{
    AgentDomain, AgentMailboxPlan, AgentMeta, AgentSendMode, AgentSendRule, AgentService, AgentVerification,
    CODE_SLACK_MS, find_code, limits_text, local_part, read_meta, service_error, write_meta,
};
use crate::registry::{AccountKind, DEMO_ACCOUNT_ID, IndexEntry, accounts_dir, load_index};
use crate::secrets::{self, keys};
use crate::{Core, CoreError, ErrorKind, runtime};

const SERVICE_FILE: &str = "service.json";

pub(crate) fn services_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("services")
}

/// `service.json`: what the agents of a service account share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ServiceMeta {
    pub service: AgentService,
    #[serde(default)]
    pub created_at: i64,
    /// The user's email it was verified with, once known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub human_email: Option<String>,
    #[serde(default)]
    pub verified: bool,
    /// The plan as the service last gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<AgentMailboxPlan>,
    /// Primitive's managed subdomain, where every agent takes a local part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_domain: Option<String>,
    /// The user's own domains on it, as the service last listed them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<KnownDomain>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KnownDomain {
    pub id: String,
    pub domain: String,
    pub verified: bool,
}

pub(crate) fn domain_of(address: &str) -> Option<String> {
    address.rsplit_once('@').map(|(_, d)| d.to_lowercase())
}

impl ServiceMeta {
    /// The service account an agent made before 2026-10-08 stands for.
    pub(crate) fn of_agent(agent: &AgentMeta) -> Self {
        Self {
            service: agent.service,
            created_at: agent.created_at,
            human_email: None,
            verified: false,
            plan: None,
            managed_domain: domain_of(agent.managed_address.as_deref().unwrap_or(&agent.address)),
            domains: vec![],
        }
    }

    fn learn_plan(&mut self, plan: &AgentMailboxPlan) {
        self.verified = plan.verified;
        if plan.email.is_some() {
            self.human_email.clone_from(&plan.email);
        }
        self.plan = Some(plan.clone());
    }

    fn learn_domain(&mut self, d: &AgentDomain) {
        let known = KnownDomain { id: d.id.clone(), domain: d.domain.clone(), verified: d.verified };
        match self.domains.iter_mut().find(|k| k.id == d.id) {
            Some(existing) => *existing = known,
            None => self.domains.push(known),
        }
    }
}

/// A service account's record: `service.json`, or for a mailbox made
/// before service accounts, the one its agent stands for.
pub(crate) fn read_service(data_dir: &Path, id: &str) -> Option<ServiceMeta> {
    if !crate::mail::valid_account_id(id) {
        return None;
    }
    let stored = std::fs::read(services_dir(data_dir).join(id).join(SERVICE_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ServiceMeta>(&bytes).ok());
    stored.or_else(|| {
        let agent = read_meta(&accounts_dir(data_dir).join(id))?;
        (agent.service_account == id).then(|| ServiceMeta::of_agent(&agent))
    })
}

fn service_written(data_dir: &Path, id: &str) -> bool {
    services_dir(data_dir).join(id).join(SERVICE_FILE).is_file()
}

pub(crate) fn write_service(data_dir: &Path, id: &str, meta: &ServiceMeta) -> Result<(), CoreError> {
    let storage = |e: std::io::Error| CoreError::new(ErrorKind::Storage, e.to_string());
    let dir = services_dir(data_dir).join(id);
    std::fs::create_dir_all(&dir).map_err(storage)?;
    let tmp = dir.join(format!("{SERVICE_FILE}.tmp"));
    let bytes = serde_json::to_vec_pretty(meta).map_err(|e| CoreError::new(ErrorKind::Internal, e.to_string()))?;
    std::fs::write(&tmp, bytes).map_err(storage)?;
    std::fs::rename(tmp, dir.join(SERVICE_FILE)).map_err(storage)
}

/// Every agent mailbox on disk, by account id.
pub(crate) fn agents_on_disk(data_dir: &Path) -> Vec<(String, AgentMeta)> {
    let Ok(entries) = std::fs::read_dir(accounts_dir(data_dir)) else { return Vec::new() };
    let mut found: Vec<(String, AgentMeta)> = entries
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            if id == DEMO_ACCOUNT_ID || !crate::mail::valid_account_id(&id) {
                return None;
            }
            read_meta(&e.path()).map(|m| (id, m))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
}

/// A service account as the app lists it (spec §7.9).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ServiceAccountSummary {
    pub id: String,
    pub service: AgentService,
    /// The user's email it was verified with, once known.
    pub human_email: Option<String>,
    pub verified: bool,
    /// The plan as the service last gave it.
    pub plan: Option<AgentMailboxPlan>,
    /// Where its agents take addresses (Primitive's managed subdomain).
    pub managed_domain: Option<String>,
    /// Its agents' account ids, in the accounts' order.
    pub agent_account_ids: Vec<String>,
}

/// An agent added to a service account.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentAdded {
    pub account_id: String,
    pub address: String,
    pub service_account_id: String,
}

impl Core {
    /// The service account an agent mailbox belongs to.
    pub(crate) fn service_of(&self, account_id: &str) -> Result<String, CoreError> {
        Ok(self.agent_meta_or_err(account_id)?.service_account)
    }

    pub(crate) fn service_meta(&self, id: &str) -> Result<ServiceMeta, CoreError> {
        read_service(&self.data_path(), id)
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "no such service account"))
    }

    /// Change a service account's record, writing it (the first write of a
    /// migrated one).
    pub(crate) fn update_service(&self, id: &str, change: impl FnOnce(&mut ServiceMeta)) -> Result<(), CoreError> {
        let _guard = self.agent_mail.records.lock().unwrap_or_else(|e| e.into_inner());
        let mut meta = self.service_meta(id)?;
        change(&mut meta);
        write_service(&self.data_path(), id, &meta)
    }

    /// A plan from the service, with what the record knows and the service
    /// does not say: AgentMail's organisation tells neither whether it is
    /// verified nor the human email, and an organisation once verified stays
    /// so.
    fn with_record(&self, id: &str, mut plan: AgentMailboxPlan) -> Result<AgentMailboxPlan, CoreError> {
        let meta = self.service_meta(id)?;
        if meta.service == AgentService::AgentMail {
            plan.verified |= meta.verified;
            plan.email = plan.email.or(meta.human_email);
        }
        Ok(plan)
    }

    pub(crate) fn service_key(&self, id: &str) -> Result<Redacted<String>, CoreError> {
        secrets::get_redacted(self.secrets.as_ref(), &keys::mailbox_api_key(id))?
            .ok_or_else(|| CoreError::new(ErrorKind::Auth, "this agent mailbox's key is missing from the Keychain"))
    }

    pub(crate) fn service_client(&self, id: &str) -> Result<(Arc<dyn MailboxService>, Redacted<String>), CoreError> {
        let meta = self.service_meta(id)?;
        Ok((self.mailbox_service(meta.service)?, self.service_key(id)?))
    }

    /// `leaving` is removed from this Mac: with no other agent left on its
    /// service account `id`, forget the account's record and key; else keep
    /// both, writing the record if it was only read from `leaving`.
    pub(crate) fn release_service_account(&self, id: &str, leaving: &str) -> Result<(), CoreError> {
        let data_dir = self.data_path();
        let removed = self.open_accounts.read().unwrap_or_else(|e| e.into_inner()).removed.clone();
        let others = agents_on_disk(&data_dir)
            .into_iter()
            .any(|(account, meta)| account != leaving && !removed.contains(&account) && meta.service_account == id);
        let _guard = self.agent_mail.records.lock().unwrap_or_else(|e| e.into_inner());
        if others {
            if !service_written(&data_dir, id)
                && let Some(meta) = read_service(&data_dir, id)
            {
                write_service(&data_dir, id, &meta)?;
            }
            return Ok(());
        }
        self.secrets.delete(keys::mailbox_api_key(id))?;
        if crate::mail::valid_account_id(id) {
            let _ = std::fs::remove_dir_all(services_dir(&data_dir).join(id));
        }
        tracing::info!(service_account = %id, "service account's last agent removed; its key is forgotten");
        Ok(())
    }
}

/// Domains as given to the app, learned into the record on the way.
fn learn_domains(core: &Core, id: &str, domains: &[AgentDomain]) -> Result<(), CoreError> {
    core.update_service(id, |meta| {
        meta.domains.retain(|k| domains.iter().any(|d| d.id == k.id));
        for d in domains {
            meta.learn_domain(d);
        }
    })
}

#[uniffi::export]
impl Core {
    /// The service accounts with their agents, in the accounts' order
    /// (spec §7.9, ADR 0015).
    pub async fn list_service_accounts(&self) -> Result<Vec<ServiceAccountSummary>, CoreError> {
        let data_dir = self.data_path();
        let _guard = self.index_lock.lock().await;
        runtime::run(async move {
            tokio::task::spawn_blocking(move || {
                let mut out: Vec<ServiceAccountSummary> = Vec::new();
                for entry in load_index(&data_dir).into_iter().filter(|e| e.kind == AccountKind::Agent) {
                    let Some(agent) = read_meta(&accounts_dir(&data_dir).join(&entry.id)) else { continue };
                    if let Some(listed) = out.iter_mut().find(|s| s.id == agent.service_account) {
                        listed.agent_account_ids.push(entry.id);
                        continue;
                    }
                    let Some(meta) = read_service(&data_dir, &agent.service_account) else { continue };
                    out.push(ServiceAccountSummary {
                        id: agent.service_account,
                        service: meta.service,
                        human_email: meta.human_email,
                        verified: meta.verified,
                        plan: meta.plan,
                        managed_domain: meta.managed_domain,
                        agent_account_ids: vec![entry.id],
                    });
                }
                out
            })
            .await
            .map_err(|e| CoreError::new(ErrorKind::Internal, e.to_string()))
        })
        .await
    }

    /// The service account an agent mailbox belongs to.
    pub fn agent_service_account(&self, account_id: String) -> Result<String, CoreError> {
        self.service_of(&account_id)
    }

    /// Add an agent named `name` to service account `service_account_id`:
    /// no sign-up, terms or code (spec §7.9). On Primitive no API call is
    /// made: the address is the name as a local part on `domain` (one of
    /// the service account's verified own domains), else on its managed
    /// subdomain, and must not be another of its agents'. `request_id` is
    /// the sheet's own (a UUID): it becomes the account id, so a retry
    /// returns the same agent. The app then opens it and starts sync.
    pub async fn add_agent(
        self: Arc<Self>,
        service_account_id: String,
        name: String,
        domain: Option<String>,
        request_id: String,
    ) -> Result<AgentAdded, CoreError> {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() || name.chars().count() > 100 {
            return Err(CoreError::new(ErrorKind::InvalidInput, "give the agent a name of up to 100 characters"));
        }
        if !crate::mail::valid_account_id(&request_id) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "request id must be 1-64 of [A-Za-z0-9-]"));
        }
        let data_dir = self.data_path();
        let dir = accounts_dir(&data_dir).join(&request_id);
        // A retry: the agent is already there.
        if let Some(existing) = read_meta(&dir) {
            if existing.service_account != service_account_id {
                return Err(CoreError::new(ErrorKind::InvalidInput, "that request made another agent"));
            }
            return Ok(AgentAdded { account_id: request_id, address: existing.address, service_account_id });
        }
        let service = self.service_meta(&service_account_id)?;
        let managed = service.managed_domain.clone();
        let domain = match domain.map(|d| d.trim().trim_end_matches('.').to_lowercase()) {
            Some(d) if Some(&d) != managed.as_ref() => {
                let known = |s: &ServiceMeta| s.domains.iter().any(|k| k.verified && k.domain == d);
                if !known(&service) {
                    // Not known verified yet: ask the service once.
                    self.service_account_domains(service_account_id.clone()).await?;
                    if !known(&self.service_meta(&service_account_id)?) {
                        return Err(CoreError::new(
                            ErrorKind::InvalidInput,
                            format!("{d} is not one of this service account's verified domains"),
                        ));
                    }
                }
                d
            }
            _ => managed.clone().ok_or_else(|| {
                CoreError::new(ErrorKind::InvalidInput, "this service account has no domain to add an agent on")
            })?,
        };
        let local = local_part(&name);
        let address = format!("{local}@{domain}");
        let managed_address = managed.as_ref().map(|m| format!("{local}@{m}"));

        // One addition at a time per Mac, so two agents cannot take one address.
        let _adding = self.agent_mail.adding.lock().await;
        let taken = agents_on_disk(&data_dir).into_iter().find(|(_, a)| {
            a.service_account == service_account_id
                && (a.address == address
                    || a.managed_address.as_ref().is_some_and(|m| Some(m) == managed_address.as_ref()))
        });
        if let Some((_, other)) = taken {
            return Err(CoreError::new(
                ErrorKind::InvalidInput,
                format!("{} already has {address}; give this agent another name", other.name),
            ));
        }
        // Primitive: a local part, no call. AgentMail: an inbox in the
        // organisation, whose id `agent.json` keeps; the request id makes a
        // retry return the same inbox. Never a sign-up (ADR 0015).
        let (address, inbox_id) = match service.service {
            AgentService::Primitive => (address, None),
            AgentService::AgentMail => {
                let (client, key) = self.service_client(&service_account_id)?;
                let custom = (Some(&domain) != managed.as_ref()).then(|| domain.clone());
                let (username, display, request) = (local.clone(), name.clone(), request_id.clone());
                let added = runtime::run(async move {
                    client
                        .add_mailbox(key.expose(), &username, custom.as_deref(), &display, &request)
                        .await
                        .map_err(service_error)
                })
                .await?;
                (added.address, Some(added.inbox_id))
            }
        };
        // The record outlives the agent it may have been read from.
        if !service_written(&data_dir, &service_account_id) {
            self.update_service(&service_account_id, |_| {})?;
        }
        write_meta(
            &dir,
            &AgentMeta {
                kind: "agent".into(),
                service: service.service,
                address: address.clone(),
                name: name.clone(),
                created_at: mail_sync::now_millis(),
                send_mode: AgentSendMode::default(),
                managed_address,
                service_account: service_account_id.clone(),
                inbox_id,
            },
        )?;
        drop(_adding);
        let core = self.clone();
        let account_id = request_id;
        runtime::run(async move {
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
                service: Some(service.service),
            })
            .await?;
            tracing::info!(account = %account_id, service_account = %service_account_id, "agent added");
            Ok(AgentAdded { account_id, address, service_account_id })
        })
        .await
    }

    /// The service account's plan and limits now, from the service.
    pub async fn service_account_plan(&self, service_account_id: String) -> Result<AgentMailboxPlan, CoreError> {
        let (client, key) = self.service_client(&service_account_id)?;
        let plan: AgentMailboxPlan =
            runtime::run(async move { client.plan(key.expose()).await.map(Into::into).map_err(service_error) }).await?;
        let plan = self.with_record(&service_account_id, plan)?;
        self.update_service(&service_account_id, |m| m.learn_plan(&plan))?;
        Ok(plan)
    }

    /// Email a verification code to `email`.
    pub async fn start_service_account_verification(
        &self,
        service_account_id: String,
        email: String,
    ) -> Result<AgentVerification, CoreError> {
        let email = email.trim().to_owned();
        if !email.contains('@') {
            return Err(CoreError::new(ErrorKind::InvalidInput, "enter an email address"));
        }
        let meta = self.service_meta(&service_account_id)?;
        // AgentMail: the code goes to the human the organisation was made
        // with; asking with another email would replace that human, which
        // AgentMail allows only twice.
        if meta.service == AgentService::AgentMail
            && let Some(human) = meta.human_email.as_deref().filter(|h| !h.trim().eq_ignore_ascii_case(&email))
        {
            return Err(CoreError::new(
                ErrorKind::InvalidInput,
                format!(
                    "AgentMail sends the code to {human}, the email this service account was created with. To use \
                     another email, change it in AgentMail's console."
                ),
            ));
        }
        let (client, key) = self.service_client(&service_account_id)?;
        let asked = email.clone();
        let started =
            runtime::run(async move { client.start_verification(key.expose(), &email).await.map_err(service_error) })
                .await?;
        if meta.service == AgentService::AgentMail && meta.human_email.is_none() {
            self.update_service(&service_account_id, |m| m.human_email = Some(asked))?;
        }
        self.agent_mail
            .verifications
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(service_account_id, mail_sync::now_millis());
        Ok(AgentVerification { resend_after_secs: started.resend_after_secs, expires_in_secs: started.expires_in_secs })
    }

    /// Confirm the code; the plan afterwards. Every agent of the service
    /// account is verified with it.
    pub async fn verify_service_account(
        &self,
        service_account_id: String,
        code: String,
    ) -> Result<AgentMailboxPlan, CoreError> {
        let code = code.trim().to_owned();
        if code.is_empty() {
            return Err(CoreError::new(ErrorKind::InvalidInput, "enter the code from the email"));
        }
        let (client, key) = self.service_client(&service_account_id)?;
        let plan: AgentMailboxPlan =
            runtime::run(
                async move { client.verify(key.expose(), &code).await.map(Into::into).map_err(service_error) },
            )
            .await?;
        let plan = self.with_record(&service_account_id, plan)?;
        self.agent_mail.verifications.lock().unwrap_or_else(|e| e.into_inner()).remove(&service_account_id);
        self.update_service(&service_account_id, |m| m.learn_plan(&plan))?;
        Ok(plan)
    }

    /// The verification code for the service account, if it has arrived in
    /// the user's account `in_account_id` since the code was asked for:
    /// only mail from the service's domain is read (spec §7.9). `None`
    /// until then, or when no code was asked for.
    pub async fn find_service_account_code(
        &self,
        service_account_id: String,
        in_account_id: String,
    ) -> Result<Option<String>, CoreError> {
        let meta = self.service_meta(&service_account_id)?;
        let Some(asked) =
            self.agent_mail.verifications.lock().unwrap_or_else(|e| e.into_inner()).get(&service_account_id).copied()
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

    /// The service account's API key, for an agent that calls the service
    /// itself (*Copy API Key*, behind a confirmation in the app). It reaches
    /// every agent of the service account.
    pub fn service_account_api_key(&self, service_account_id: String) -> Result<String, CoreError> {
        self.service_meta(&service_account_id)?;
        Ok(self.service_key(&service_account_id)?.expose().clone())
    }

    /// AgentMail, once verified: a new key that reaches only this agent's
    /// inbox, for an agent outside the app that should not reach the other
    /// agents of the organisation (*Copy API Key*, spec §7.9). Each call
    /// makes another key at AgentMail; it is not stored here.
    pub async fn agent_inbox_api_key(&self, account_id: String) -> Result<String, CoreError> {
        let agent = self.agent_meta_or_err(&account_id)?;
        if agent.service != AgentService::AgentMail {
            return Err(CoreError::new(ErrorKind::InvalidInput, "only AgentMail makes keys for one inbox"));
        }
        if !self.service_meta(&agent.service_account)?.verified {
            return Err(CoreError::new(ErrorKind::InvalidInput, provider_agentmail::NOT_VERIFIED_YET));
        }
        let inbox = agent.inbox_id.clone().unwrap_or_else(|| agent.address.clone());
        let (client, key) = self.service_client(&agent.service_account)?;
        let label = format!("OpenAGC {}", agent.name);
        let made =
            runtime::run(
                async move { client.mailbox_api_key(key.expose(), &inbox, &label).await.map_err(service_error) },
            )
            .await?;
        Ok(made.expose().clone())
    }

    /// What the service account's agents may do, in words (spec §7.9): the
    /// same text the agents' prompts carry.
    pub fn service_account_limits(&self, service_account_id: String) -> Result<String, CoreError> {
        let meta = self.service_meta(&service_account_id)?;
        Ok(limits_text(meta.service, meta.verified, meta.human_email.as_deref()))
    }

    /// Replace the service account's key at the service and in the
    /// Keychain (*Rotate Key*); all its agents use the new one. Sync picks
    /// it up when it next starts.
    pub async fn rotate_service_account_key(&self, service_account_id: String) -> Result<(), CoreError> {
        let (client, key) = self.service_client(&service_account_id)?;
        let fresh = runtime::run(async move { client.rotate_key(key.expose()).await.map_err(service_error) }).await?;
        self.secrets.set(keys::mailbox_api_key(&service_account_id), fresh.expose().clone())?;
        tracing::info!(service_account = %service_account_id, "service account's key replaced");
        Ok(())
    }

    /// Where the service account's agents may send now, broadest first.
    pub async fn service_account_send_rules(
        &self,
        service_account_id: String,
    ) -> Result<Vec<AgentSendRule>, CoreError> {
        let (client, key) = self.service_client(&service_account_id)?;
        runtime::run(async move {
            Ok(client.send_rules(key.expose()).await.map_err(service_error)?.into_iter().map(Into::into).collect())
        })
        .await
    }

    /// The user's own domains on the service account.
    pub async fn service_account_domains(&self, service_account_id: String) -> Result<Vec<AgentDomain>, CoreError> {
        let (client, key) = self.service_client(&service_account_id)?;
        let domains: Vec<AgentDomain> = runtime::run(async move {
            Ok(client.domains(key.expose()).await.map_err(service_error)?.into_iter().map(Into::into).collect())
        })
        .await?;
        learn_domains(self, &service_account_id, &domains)?;
        Ok(domains)
    }

    /// Add one of the user's domains (a subdomain such as
    /// `agents.example.com` is the usual choice): the DNS records to create.
    pub async fn add_service_account_domain(
        &self,
        service_account_id: String,
        domain: String,
    ) -> Result<AgentDomain, CoreError> {
        let domain = domain.trim().trim_end_matches('.').to_lowercase();
        if !domain.contains('.') || domain.contains('@') || domain.contains(char::is_whitespace) {
            return Err(CoreError::new(ErrorKind::InvalidInput, "enter a domain such as agents.example.com"));
        }
        let (client, key) = self.service_client(&service_account_id)?;
        let added: AgentDomain = runtime::run(async move {
            client.add_domain(key.expose(), &domain).await.map(Into::into).map_err(service_error)
        })
        .await?;
        self.update_service(&service_account_id, |m| m.learn_domain(&added))?;
        Ok(added)
    }

    /// Check a domain's records now.
    pub async fn check_service_account_domain(
        &self,
        service_account_id: String,
        domain_id: String,
    ) -> Result<AgentDomain, CoreError> {
        let (client, key) = self.service_client(&service_account_id)?;
        let checked: AgentDomain = runtime::run(async move {
            client.verify_domain(key.expose(), &domain_id).await.map(Into::into).map_err(service_error)
        })
        .await?;
        self.update_service(&service_account_id, |m| m.learn_domain(&checked))?;
        Ok(checked)
    }

    /// The records as a BIND zone file.
    pub async fn service_account_domain_zone_file(
        &self,
        service_account_id: String,
        domain_id: String,
    ) -> Result<String, CoreError> {
        let (client, key) = self.service_client(&service_account_id)?;
        runtime::run(async move { client.zone_file(key.expose(), &domain_id).await.map_err(service_error) }).await
    }
}
