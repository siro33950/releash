use std::sync::Arc;

use crate::domain::provider_lifecycle::{
    ProviderHookHealthOutcome, ProviderHookHealthRepository, ProviderHookHealthRepositoryError,
    ProviderKind, ProviderLifecycleUnavailableReason,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderHookHealthWarning {
    pub(crate) provider: ProviderKind,
    pub(crate) launch_id: String,
    pub(crate) reason: ProviderLifecycleUnavailableReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderHookHealthFailureObservation {
    pub(crate) provider: ProviderKind,
    pub(crate) launch_id: String,
    pub(crate) reason: ProviderLifecycleUnavailableReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderHookHealthFailureQueryError {
    Unavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub(crate) trait ProviderHookHealthFailureQuery: Send + Sync {
    async fn list(
        &self,
        limit: usize,
    ) -> Result<Vec<ProviderHookHealthFailureObservation>, ProviderHookHealthFailureQueryError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderHookHealthUsecaseError {
    InvalidInput,
    StorageUnavailable,
    Corrupt,
}

pub(crate) struct ProviderHookHealthUsecase {
    repository: Arc<dyn ProviderHookHealthRepository>,
}

pub(crate) struct ProviderHookHealthReadUsecase {
    health: Arc<ProviderHookHealthUsecase>,
    failures: Arc<dyn ProviderHookHealthFailureQuery>,
}

impl ProviderHookHealthReadUsecase {
    pub(crate) fn new(
        health: Arc<ProviderHookHealthUsecase>,
        failures: Arc<dyn ProviderHookHealthFailureQuery>,
    ) -> Self {
        Self { health, failures }
    }

    pub(crate) async fn warnings(
        &self,
    ) -> Result<Vec<ProviderHookHealthWarning>, ProviderHookHealthUsecaseError> {
        let observations = self.failures.list(256).await.map_err(|error| match error {
            ProviderHookHealthFailureQueryError::Unavailable => {
                ProviderHookHealthUsecaseError::StorageUnavailable
            }
            ProviderHookHealthFailureQueryError::Corrupt => ProviderHookHealthUsecaseError::Corrupt,
        })?;
        for observation in observations {
            self.health
                .record_unavailable(
                    observation.provider,
                    &observation.launch_id,
                    observation.reason,
                    &format!(
                        "provider-hook-delivery-failure.{}.{}",
                        provider_label(observation.provider),
                        observation.launch_id
                    ),
                )
                .await?;
        }
        self.health.warnings().await
    }
}

impl ProviderHookHealthUsecase {
    pub(crate) fn new(repository: Arc<dyn ProviderHookHealthRepository>) -> Self {
        Self { repository }
    }

    #[cfg(test)]
    pub(crate) async fn record_launch(
        &self,
        provider: ProviderKind,
        launch_id: &str,
        caller_request_id: &str,
    ) -> Result<(), ProviderHookHealthUsecaseError> {
        self.record_launch_with_warning(provider, launch_id, None, caller_request_id)
            .await
    }

    pub(crate) async fn record_launch_with_warning(
        &self,
        provider: ProviderKind,
        launch_id: &str,
        warning: Option<ProviderLifecycleUnavailableReason>,
        caller_request_id: &str,
    ) -> Result<(), ProviderHookHealthUsecaseError> {
        if launch_id.trim().is_empty() || caller_request_id.trim().is_empty() {
            return Err(ProviderHookHealthUsecaseError::InvalidInput);
        }
        for _ in 0..4 {
            let mut versioned = self.repository.load(provider).await.map_err(map_error)?;
            let launch_outcome = versioned.health_mut().observe_launch(launch_id);
            let warning_outcome = warning.map(|reason| {
                versioned
                    .health_mut()
                    .observe_unavailable(launch_id, reason)
            });
            if launch_outcome == ProviderHookHealthOutcome::Duplicate
                && warning_outcome
                    .is_none_or(|outcome| outcome == ProviderHookHealthOutcome::Duplicate)
            {
                return Ok(());
            }
            match self.repository.save(versioned, caller_request_id).await {
                Ok(_) => return Ok(()),
                Err(ProviderHookHealthRepositoryError::Conflict) => continue,
                Err(error) => return Err(map_error(error)),
            }
        }
        Err(ProviderHookHealthUsecaseError::StorageUnavailable)
    }

    pub(crate) async fn record_unavailable(
        &self,
        provider: ProviderKind,
        launch_id: &str,
        reason: ProviderLifecycleUnavailableReason,
        caller_request_id: &str,
    ) -> Result<(), ProviderHookHealthUsecaseError> {
        if launch_id.trim().is_empty() || caller_request_id.trim().is_empty() {
            return Err(ProviderHookHealthUsecaseError::InvalidInput);
        }
        for _ in 0..4 {
            let mut versioned = self.repository.load(provider).await.map_err(map_error)?;
            if versioned
                .health_mut()
                .observe_unavailable(launch_id, reason)
                == ProviderHookHealthOutcome::Duplicate
            {
                return Ok(());
            }
            match self.repository.save(versioned, caller_request_id).await {
                Ok(_) => return Ok(()),
                Err(ProviderHookHealthRepositoryError::Conflict) => continue,
                Err(error) => return Err(map_error(error)),
            }
        }
        Err(ProviderHookHealthUsecaseError::StorageUnavailable)
    }

    pub(crate) async fn record_session_started(
        &self,
        provider: ProviderKind,
        launch_id: &str,
        caller_request_id: &str,
    ) -> Result<(), ProviderHookHealthUsecaseError> {
        if launch_id.trim().is_empty() || caller_request_id.trim().is_empty() {
            return Err(ProviderHookHealthUsecaseError::InvalidInput);
        }
        for _ in 0..4 {
            let mut versioned = self.repository.load(provider).await.map_err(map_error)?;
            if versioned
                .health_mut()
                .observe_active_session_started(launch_id)
                == ProviderHookHealthOutcome::Duplicate
            {
                return Ok(());
            }
            match self.repository.save(versioned, caller_request_id).await {
                Ok(_) => return Ok(()),
                Err(ProviderHookHealthRepositoryError::Conflict) => continue,
                Err(error) => return Err(map_error(error)),
            }
        }
        Err(ProviderHookHealthUsecaseError::StorageUnavailable)
    }

    pub(crate) async fn warnings(
        &self,
    ) -> Result<Vec<ProviderHookHealthWarning>, ProviderHookHealthUsecaseError> {
        let mut warnings = Vec::new();
        for provider in ProviderKind::supported() {
            let health = self.repository.load(*provider).await.map_err(map_error)?;
            if let Some((launch_id, reason)) = health.health().warning() {
                warnings.push(ProviderHookHealthWarning {
                    provider: *provider,
                    launch_id: launch_id.to_string(),
                    reason,
                });
            }
        }
        Ok(warnings)
    }
}

fn map_error(error: ProviderHookHealthRepositoryError) -> ProviderHookHealthUsecaseError {
    match error {
        ProviderHookHealthRepositoryError::InvalidInput => {
            ProviderHookHealthUsecaseError::InvalidInput
        }
        ProviderHookHealthRepositoryError::Conflict
        | ProviderHookHealthRepositoryError::StorageUnavailable => {
            ProviderHookHealthUsecaseError::StorageUnavailable
        }
        ProviderHookHealthRepositoryError::Corrupt => ProviderHookHealthUsecaseError::Corrupt,
    }
}

fn provider_label(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Claude => "claude",
        ProviderKind::Codex => "codex",
    }
}
