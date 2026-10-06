//! Agent-mail services (spec §7.9, ADR 0014): creating a mailbox for one
//! of the user's agents and verifying it, beside the [`MailProvider`]
//! (crate::MailProvider) that syncs it.

use async_trait::async_trait;
use mail_domain::Redacted;

use crate::ProviderResult;

/// What a service account may do now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxPlan {
    /// The service's plan name (Primitive: `agent`, `developer`, ...).
    pub name: String,
    /// The account has confirmed an email address.
    pub verified: bool,
    /// It can only reply to addresses that wrote to it first.
    pub reply_only: bool,
    pub send_per_hour: u32,
    pub send_per_day: u32,
    /// The email address the account was verified with, if known.
    pub email: Option<String>,
}

/// A new service account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedUp {
    pub api_key: Redacted<String>,
    /// The mailbox's address.
    pub address: String,
    pub plan: MailboxPlan,
}

/// A verification code is on its way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerificationStarted {
    /// Seconds before *Resend* may be offered.
    pub resend_after_secs: u32,
    /// Seconds the code stays valid.
    pub expires_in_secs: u32,
}

/// One agent-mail service: everything about an account that is not mail.
#[async_trait]
pub trait MailboxService: Send + Sync {
    /// A short name for logs ("primitive").
    fn name(&self) -> &'static str;
    /// The service's terms, shown before creating an account.
    fn terms_url(&self) -> &'static str;
    /// The domain verification codes come from, for filling the code from
    /// the user's own mail.
    fn code_sender_domain(&self) -> &'static str;
    /// Create an account (no authentication). `idempotency_key` makes a
    /// retried call return the same account. Accepts the service's terms:
    /// call only after the user agreed to them.
    async fn sign_up(&self, device_name: &str, idempotency_key: &str) -> ProviderResult<SignedUp>;
    /// The account's plan and limits now.
    async fn plan(&self, api_key: &str) -> ProviderResult<MailboxPlan>;
    /// Email a verification code to `email`.
    async fn start_verification(&self, api_key: &str, email: &str) -> ProviderResult<VerificationStarted>;
    /// Confirm the code; the account's plan afterwards.
    async fn verify(&self, api_key: &str, code: &str) -> ProviderResult<MailboxPlan>;
}
