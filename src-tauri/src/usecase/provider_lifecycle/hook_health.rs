use std::sync::Arc;

use crate::domain::provider_lifecycle::{
    ProviderHookHealthOutcome, ProviderHookHealthRepository, ProviderHookHealthRepositoryError,
    ProviderKind, ProviderLifecycleUnavailableReason,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHookHealthWarning {
    pub(crate) provider: ProviderKind,
    pub(crate) launch_id: String,
    pub reason: ProviderLifecycleUnavailableReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHookHealthFailureObservation {
    pub provider: ProviderKind,
    pub launch_id: String,
    pub reason: ProviderLifecycleUnavailableReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderHookHealthFailureQueryError {
    Technical(crate::domain::failure::TechnicalFailure),
    Corrupt,
}

#[async_trait::async_trait]
pub trait ProviderHookHealthFailureQuery: Send + Sync {
    async fn list(
        &self,
        limit: usize,
    ) -> Result<
        Vec<Result<ProviderHookHealthFailureObservation, ProviderHookHealthFailureQueryError>>,
        ProviderHookHealthFailureQueryError,
    >;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderHookHealthUsecaseError {
    Technical(crate::domain::failure::TechnicalFailure),
    Conflict,
    Store(crate::domain::failure::StorageFailure),
    InvalidInput,
    StorageUnavailable,
    Corrupt,
}

pub struct ProviderHookHealthUsecase {
    repository: Arc<dyn ProviderHookHealthRepository>,
    state_publisher: Option<crate::usecase::state_subscription::StateSubscriptionUsecase>,
}

pub struct ProviderHookHealthReadUsecase {
    health: Arc<ProviderHookHealthUsecase>,
    failures: Arc<dyn ProviderHookHealthFailureQuery>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderHookHealthReadResult {
    pub warnings: Vec<ProviderHookHealthWarning>,
    pub failures: Vec<ProviderHookHealthFailureQueryError>,
}
impl ProviderHookHealthReadUsecase {
    pub fn new(
        health: Arc<ProviderHookHealthUsecase>,
        failures: Arc<dyn ProviderHookHealthFailureQuery>,
    ) -> Self {
        Self { health, failures }
    }

    pub(crate) async fn warnings(
        &self,
    ) -> Result<ProviderHookHealthReadResult, ProviderHookHealthUsecaseError> {
        let observations = self.failures.list(256).await.map_err(|error| match error {
            ProviderHookHealthFailureQueryError::Technical(failure) => {
                ProviderHookHealthUsecaseError::Technical(failure)
            }
            ProviderHookHealthFailureQueryError::Corrupt => ProviderHookHealthUsecaseError::Corrupt,
        })?;
        let mut failures = Vec::new();
        for observation in observations {
            let observation = match observation {
                Ok(observation) => observation,
                Err(error) => {
                    failures.push(error);
                    continue;
                }
            };
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
        Ok(ProviderHookHealthReadResult {
            warnings: self.health.warnings().await?,
            failures,
        })
    }
}

impl ProviderHookHealthUsecase {
    pub fn new(repository: Arc<dyn ProviderHookHealthRepository>) -> Self {
        Self {
            repository,
            state_publisher: None,
        }
    }

    pub fn with_state_publisher(
        mut self,
        publisher: crate::usecase::state_subscription::StateSubscriptionUsecase,
    ) -> Self {
        self.state_publisher = Some(publisher);
        self
    }

    fn health_changed(&self) {
        if let Some(publisher) = &self.state_publisher {
            publisher
                .notify(crate::usecase::state_subscription::StateChangeSource::ProviderHookHealth);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn record_launch(
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
                Ok(_) => {
                    self.health_changed();
                    return Ok(());
                }
                Err(error)
                    if crate::usecase::failure::Failure::from(&error)
                        == crate::usecase::failure::Failure::Technical(
                            crate::domain::failure::TechnicalFailureNature::Transient,
                        ) =>
                {
                    continue
                }
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
                Ok(_) => {
                    self.health_changed();
                    return Ok(());
                }
                Err(error)
                    if crate::usecase::failure::Failure::from(&error)
                        == crate::usecase::failure::Failure::Technical(
                            crate::domain::failure::TechnicalFailureNature::Transient,
                        ) =>
                {
                    continue
                }
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
                Ok(_) => {
                    self.health_changed();
                    return Ok(());
                }
                Err(error)
                    if crate::usecase::failure::Failure::from(&error)
                        == crate::usecase::failure::Failure::Technical(
                            crate::domain::failure::TechnicalFailureNature::Transient,
                        ) =>
                {
                    continue
                }
                Err(error) => return Err(map_error(error)),
            }
        }
        Err(ProviderHookHealthUsecaseError::StorageUnavailable)
    }

    pub async fn warnings(
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
        ProviderHookHealthRepositoryError::Conflict => ProviderHookHealthUsecaseError::Conflict,
        ProviderHookHealthRepositoryError::StorageUnavailable => {
            ProviderHookHealthUsecaseError::StorageUnavailable
        }
        ProviderHookHealthRepositoryError::Store(kind) => {
            ProviderHookHealthUsecaseError::Store(kind)
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

#[cfg(test)]
#[path = "hook_health_error_test.rs"]
pub(crate) mod error_tests;
