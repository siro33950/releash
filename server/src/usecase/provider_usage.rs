use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::provider_usage::{
    CodexUsageObservation, CodexUsageObservationRepository, ProviderUsageClock,
    ProviderUsageRepository, ProviderUsageSource, UsageError,
};
use crate::usecase::state_subscription::{StateChangeSource, StateSubscriptionUsecase};
use std::sync::Arc;
#[derive(Debug, Clone, PartialEq)]
pub struct UsageWindowDto {
    pub used_percent: f64,
    pub window_minutes: u64,
    pub resets_at: u64,
    pub warning: bool,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderUsageDto {
    pub provider: ProviderKind,
    pub windows: Vec<UsageWindowDto>,
    pub plan: Option<String>,
}
pub trait ProviderUsageQueryService: Send + Sync {
    fn snapshot(&self) -> Vec<ProviderUsageDto>;
}
pub struct ProviderUsageUsecase {
    source: Arc<dyn ProviderUsageSource>,
    query: Arc<dyn ProviderUsageQueryService>,
    repository: Arc<dyn ProviderUsageRepository>,
    observations: Arc<dyn CodexUsageObservationRepository>,
    clock: Arc<dyn ProviderUsageClock>,
    subscriptions: StateSubscriptionUsecase,
}
impl ProviderUsageUsecase {
    pub fn new(
        source: Arc<dyn ProviderUsageSource>,
        query: Arc<dyn ProviderUsageQueryService>,
        repository: Arc<dyn ProviderUsageRepository>,
        observations: Arc<dyn CodexUsageObservationRepository>,
        clock: Arc<dyn ProviderUsageClock>,
        subscriptions: StateSubscriptionUsecase,
    ) -> Self {
        Self {
            source,
            query,
            repository,
            observations,
            clock,
            subscriptions,
        }
    }
    pub fn snapshot(&self) -> Vec<ProviderUsageDto> {
        self.query.snapshot()
    }
    pub fn receive_claude(&self, payload: &[u8]) -> Result<(), UsageError> {
        if let Some(usage) = self.source.claude(payload)? {
            self.repository.replace(usage);
            self.subscriptions.notify(StateChangeSource::ProviderUsage);
        }
        Ok(())
    }
    pub fn start_codex_session(&self, session: &str) {
        self.observations
            .start_observation(CodexUsageObservation::new(session, self.clock.now()));
    }
    pub async fn poll_active_codex(
        &self,
        lifecycle: &crate::usecase::provider_lifecycle::ProviderLifecycleUsecase,
    ) -> Result<Vec<UsageError>, crate::usecase::provider_lifecycle::ProviderLifecycleUsecaseError>
    {
        let poll_started_at = self.clock.now();
        let ids = lifecycle.provider_sessions(ProviderKind::Codex).await?;
        Ok(self.poll_codex(&ids, poll_started_at).await)
    }
    pub async fn poll_codex(
        &self,
        ids: &[String],
        poll_started_at: std::time::SystemTime,
    ) -> Vec<UsageError> {
        self.observations.retain_observations(ids, poll_started_at);
        let mut failures = Vec::new();
        for id in ids {
            if let Err(error) = self.observe_codex(id).await {
                failures.push(error);
            }
        }
        failures
    }
    pub async fn observe_codex(&self, session: &str) -> Result<(), UsageError> {
        if !self.observations.has_observation(session) {
            return Ok(());
        }
        if let Some(record) = self.source.codex(session).await? {
            if self.observations.accept_record(session, &record) {
                self.repository.replace(record.usage);
                self.subscriptions.notify(StateChangeSource::ProviderUsage);
            }
        }
        Ok(())
    }
}
#[cfg(test)]
#[path = "provider_usage_test.rs"]
mod provider_usage_tests;
