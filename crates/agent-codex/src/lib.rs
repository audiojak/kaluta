//! Codex adapter (spec §9.2, §9.4): detection, and a long-lived
//! `codex app-server` speaking JSON-RPC over stdio.

mod detect;
mod session;

use std::sync::Arc;

use agent_api::process::Locator;
use agent_api::{
    AgentModel, AgentProvider, AgentResult, AgentSession, AgentStatus, EventSink, ModelFallback, ProviderId,
    SessionConfig,
};
use async_trait::async_trait;

pub use detect::MINIMUM_VERSION;
pub use session::{DISABLED_FEATURES, readable_error, refused_model, server_args};

pub struct CodexProvider {
    locator: Locator,
    /// A model Codex refused this launch, and the one used instead.
    fallback: session::Fallback,
}

impl CodexProvider {
    pub fn new(locator: Locator) -> Self {
        Self { locator, fallback: Default::default() }
    }

    pub fn standard() -> Arc<Self> {
        Arc::new(Self::new(Locator::standard()))
    }
}

#[async_trait]
impl AgentProvider for CodexProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    async fn detect(&self) -> AgentStatus {
        detect::detect(&self.locator).await
    }

    async fn start_session(&self, cfg: SessionConfig, sink: EventSink) -> AgentResult<Box<dyn AgentSession>> {
        session::start(&self.locator, cfg, sink, self.fallback.clone()).await
    }

    async fn models(&self) -> AgentResult<Vec<AgentModel>> {
        session::list_models(&self.locator).await
    }

    fn model_fallback(&self) -> Option<ModelFallback> {
        self.fallback.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}
