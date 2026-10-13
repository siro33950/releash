use crate::domain::provider_lifecycle::ProviderKind;
use crate::usecase::state_subscription::{StateChangeSource, StateSubscriptionUsecase};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct UsageWindow {
    pub used_percent: f64,
    pub window_minutes: u64,
    pub resets_at: u64,
    pub warning: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderUsage {
    pub provider: ProviderKind,
    pub windows: Vec<UsageWindow>,
    pub plan: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum UsageError {
    #[error("Invalid provider usage")]
    Invalid,
    #[error("Provider usage source failed: {0}")]
    Technical(crate::domain::failure::TechnicalFailure),
}

#[async_trait::async_trait]
pub trait ProviderUsageSource: Send + Sync {
    fn claude(&self, payload: &[u8]) -> Result<Option<ProviderUsage>, UsageError>;
    async fn codex(&self, session: &str) -> Result<Option<(String, ProviderUsage)>, UsageError>;
}

pub trait ProviderUsageQueryService: Send + Sync {
    fn snapshot(&self) -> Vec<ProviderUsage>;
}

pub trait ProviderUsageOutput: Send + Sync {
    fn replace(&self, usage: ProviderUsage);
}

pub struct ProviderUsageUsecase {
    source: Arc<dyn ProviderUsageSource>,
    query: Arc<dyn ProviderUsageQueryService>,
    output: Arc<dyn ProviderUsageOutput>,
    subscriptions: StateSubscriptionUsecase,
    codex_observation_lock: Arc<tokio::sync::Mutex<()>>,
    codex_observations: tokio::sync::Mutex<std::collections::HashMap<String, String>>,
}

impl ProviderUsageUsecase {
    pub fn new(
        source: Arc<dyn ProviderUsageSource>,
        query: Arc<dyn ProviderUsageQueryService>,
        output: Arc<dyn ProviderUsageOutput>,
        subscriptions: StateSubscriptionUsecase,
    ) -> Self {
        Self {
            source,
            query,
            output,
            subscriptions,
            codex_observation_lock: Default::default(),
            codex_observations: Default::default(),
        }
    }

    pub fn snapshot(&self) -> Vec<ProviderUsage> {
        self.query.snapshot()
    }

    pub fn receive_claude(&self, payload: &[u8]) -> Result<(), UsageError> {
        if let Some(usage) = self.source.claude(payload)? {
            self.output.replace(usage);
            self.subscriptions.notify(StateChangeSource::ProviderUsage);
        }
        Ok(())
    }

    pub(crate) async fn lock_codex_observation(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.codex_observation_lock.clone().lock_owned().await
    }

    pub async fn seed_codex_session(&self, session: &str) -> Result<(), UsageError> {
        let mut observations = self.codex_observations.lock().await;
        if let Some((record, _)) = self.source.codex(session).await? {
            observations.insert(session.to_string(), record);
        }
        Ok(())
    }

    pub async fn poll_active_codex(
        &self,
        lifecycle: &crate::usecase::provider_lifecycle::ProviderLifecycleUsecase,
    ) -> Result<Vec<UsageError>, UsageError> {
        let _observation = self.lock_codex_observation().await;
        let ids = lifecycle
            .provider_sessions(ProviderKind::Codex)
            .await
            .map_err(|error| {
                UsageError::Technical(crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Other,
                    message: error.to_string(),
                })
            })?;
        Ok(self.poll_codex(&ids).await)
    }

    pub async fn poll_codex(&self, ids: &[String]) -> Vec<UsageError> {
        self.codex_observations
            .lock()
            .await
            .retain(|id, _| ids.contains(id));
        let mut failures = Vec::new();
        for id in ids {
            if let Err(error) = self.observe_codex(id).await {
                failures.push(error);
            }
        }
        failures
    }

    pub async fn observe_codex(&self, session: &str) -> Result<(), UsageError> {
        let mut observations = self.codex_observations.lock().await;
        if let Some((record, usage)) = self.source.codex(session).await? {
            if observations.get(session) != Some(&record) {
                self.output.replace(usage);
                observations.insert(session.to_string(), record);
                self.subscriptions.notify(StateChangeSource::ProviderUsage);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "provider_usage_test.rs"]
mod provider_usage_tests;
