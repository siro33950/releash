use std::sync::Arc;

use crate::domain::agent_session::repository::{
    ProviderAgentSessionRepositoryError, VersionedProviderAgentSession,
};
use crate::domain::provider_lifecycle::{
    ProviderLifecycleIngressResult, ProviderLifecycleSignal, ProviderLifecycleSignalKind,
    ProviderLifecycleSlotId, ProviderLifecycleUnavailableObservation, ScopedProviderLifecycleEvent,
};
use crate::usecase::agent_session::{
    ProviderAgentSessionUsecase, ProviderAgentSessionUsecaseError,
};

use super::{
    ProviderHookHealthUsecase, ProviderHookHealthUsecaseError, ProviderLifecycleUsecase,
    ProviderLifecycleUsecaseError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderLifecycleIngressUsecaseError {
    InvalidInput,
    Conflict,
    StorageUnavailable,
    Corrupt,
}

#[async_trait::async_trait]
pub(crate) trait ProviderSessionStartTransaction: Send + Sync {
    async fn commit_session_started(
        &self,
        session: VersionedProviderAgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError>;
}

impl From<ProviderLifecycleUsecaseError> for ProviderLifecycleIngressUsecaseError {
    fn from(error: ProviderLifecycleUsecaseError) -> Self {
        map_lifecycle_error(error)
    }
}

pub(crate) struct ProviderLifecycleIngressUsecase {
    lifecycle: Arc<ProviderLifecycleUsecase>,
    sessions: Arc<ProviderAgentSessionUsecase>,
    hook_health: Arc<ProviderHookHealthUsecase>,
    session_start_transaction: Arc<dyn ProviderSessionStartTransaction>,
}

#[async_trait::async_trait]
pub(crate) trait ProviderLifecycleIngressPort: Send + Sync {
    async fn receive(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        capability: &str,
        signal: ProviderLifecycleSignal,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleIngressUsecaseError>;

    async fn report_unavailable(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        capability: &str,
        observation: ProviderLifecycleUnavailableObservation,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleIngressUsecaseError>;
}

impl ProviderLifecycleIngressUsecase {
    pub(crate) fn new(
        lifecycle: Arc<ProviderLifecycleUsecase>,
        sessions: Arc<ProviderAgentSessionUsecase>,
        hook_health: Arc<ProviderHookHealthUsecase>,
        session_start_transaction: Arc<dyn ProviderSessionStartTransaction>,
    ) -> Self {
        Self {
            lifecycle,
            sessions,
            hook_health,
            session_start_transaction,
        }
    }

    pub(crate) async fn receive(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        capability: &str,
        signal: ProviderLifecycleSignal,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleIngressUsecaseError> {
        let provider = signal.provider();
        let agent_session_id = signal.scope().agent_session_id().to_string();
        let binding_id = signal.binding_id().to_string();
        let kind = signal.clone().into_kind();
        if let ProviderLifecycleSignalKind::SessionStarted {
            provider_session_id,
            transcript_ref,
        } = kind
        {
            let _operation = self
                .sessions
                .lock_operation(&agent_session_id)
                .await
                .map_err(map_session_error)?;
            let prepared = self
                .sessions
                .prepare_provider_session_association(
                    &agent_session_id,
                    &provider_session_id,
                    transcript_ref.as_deref(),
                )
                .await
                .map_err(map_session_error)?;
            let has_session_changes = !prepared.session().uncommitted_events().is_empty();
            let transaction = self.session_start_transaction.clone();
            let caller_request_id = format!("provider-session-associated.{binding_id}");
            let result = self
                .lifecycle
                .receive_session_started_with_commit(
                    slot_id,
                    capability,
                    signal,
                    move |lifecycle_events| async move {
                        if lifecycle_events.is_empty() && !has_session_changes {
                            return Ok(());
                        }
                        transaction
                            .commit_session_started(prepared, lifecycle_events, &caller_request_id)
                            .await
                            .map(|_| ())
                            .map_err(map_session_repository_error)
                    },
                )
                .await?;
            if matches!(
                result,
                ProviderLifecycleIngressResult::Applied | ProviderLifecycleIngressResult::Duplicate
            ) {
                let caller_request_id = format!(
                    "provider-session-started.{binding_id}.{}",
                    crate::other::id::unique_simple_id()
                );
                self.hook_health
                    .record_session_started(provider, slot_id.as_str(), &caller_request_id)
                    .await
                    .map_err(map_hook_health_error)?;
            }
            return Ok(result);
        }
        let result = self
            .lifecycle
            .receive(slot_id, capability, signal)
            .await
            .map_err(map_lifecycle_error)?;
        if !matches!(
            result,
            ProviderLifecycleIngressResult::Applied | ProviderLifecycleIngressResult::Duplicate
        ) {
            return Ok(result);
        }
        Ok(result)
    }

    pub(crate) async fn report_unavailable(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        capability: &str,
        observation: ProviderLifecycleUnavailableObservation,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleIngressUsecaseError> {
        let provider = observation.provider();
        let reason = observation.reason();
        let binding_id = observation.binding_id().to_string();
        let result = self
            .lifecycle
            .report_unavailable(slot_id, capability, observation)
            .await
            .map_err(map_lifecycle_error)?;
        if matches!(
            result,
            ProviderLifecycleIngressResult::Applied | ProviderLifecycleIngressResult::Duplicate
        ) {
            let caller_request_id = format!(
                "provider-hook-unavailable.{binding_id}.{}",
                crate::other::id::unique_simple_id()
            );
            self.hook_health
                .record_unavailable(provider, slot_id.as_str(), reason, &caller_request_id)
                .await
                .map_err(map_hook_health_error)?;
        }
        Ok(result)
    }
}

#[async_trait::async_trait]
impl ProviderLifecycleIngressPort for ProviderLifecycleIngressUsecase {
    async fn receive(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        capability: &str,
        signal: ProviderLifecycleSignal,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleIngressUsecaseError> {
        ProviderLifecycleIngressUsecase::receive(self, slot_id, capability, signal).await
    }

    async fn report_unavailable(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        capability: &str,
        observation: ProviderLifecycleUnavailableObservation,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleIngressUsecaseError> {
        ProviderLifecycleIngressUsecase::report_unavailable(self, slot_id, capability, observation)
            .await
    }
}

#[async_trait::async_trait]
impl ProviderLifecycleIngressPort for ProviderLifecycleUsecase {
    async fn receive(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        capability: &str,
        signal: ProviderLifecycleSignal,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleIngressUsecaseError> {
        ProviderLifecycleUsecase::receive(self, slot_id, capability, signal)
            .await
            .map_err(map_lifecycle_error)
    }

    async fn report_unavailable(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        capability: &str,
        observation: ProviderLifecycleUnavailableObservation,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleIngressUsecaseError> {
        ProviderLifecycleUsecase::report_unavailable(self, slot_id, capability, observation)
            .await
            .map_err(map_lifecycle_error)
    }
}

fn map_lifecycle_error(
    error: ProviderLifecycleUsecaseError,
) -> ProviderLifecycleIngressUsecaseError {
    match error {
        ProviderLifecycleUsecaseError::InvalidInput => {
            ProviderLifecycleIngressUsecaseError::InvalidInput
        }
        ProviderLifecycleUsecaseError::StorageUnavailable => {
            ProviderLifecycleIngressUsecaseError::StorageUnavailable
        }
        ProviderLifecycleUsecaseError::Corrupt => ProviderLifecycleIngressUsecaseError::Corrupt,
    }
}

fn map_hook_health_error(
    error: ProviderHookHealthUsecaseError,
) -> ProviderLifecycleIngressUsecaseError {
    match error {
        ProviderHookHealthUsecaseError::InvalidInput => {
            ProviderLifecycleIngressUsecaseError::InvalidInput
        }
        ProviderHookHealthUsecaseError::StorageUnavailable => {
            ProviderLifecycleIngressUsecaseError::StorageUnavailable
        }
        ProviderHookHealthUsecaseError::Corrupt => ProviderLifecycleIngressUsecaseError::Corrupt,
    }
}

fn map_session_error(
    error: ProviderAgentSessionUsecaseError,
) -> ProviderLifecycleIngressUsecaseError {
    match error {
        ProviderAgentSessionUsecaseError::NotFound
        | ProviderAgentSessionUsecaseError::InvalidOperation => {
            ProviderLifecycleIngressUsecaseError::InvalidInput
        }
        ProviderAgentSessionUsecaseError::Conflict
        | ProviderAgentSessionUsecaseError::ProviderSessionAlreadyOwned { .. } => {
            ProviderLifecycleIngressUsecaseError::Conflict
        }
        ProviderAgentSessionUsecaseError::Unavailable => {
            ProviderLifecycleIngressUsecaseError::StorageUnavailable
        }
        ProviderAgentSessionUsecaseError::Corrupt => ProviderLifecycleIngressUsecaseError::Corrupt,
    }
}

fn map_session_repository_error(
    error: ProviderAgentSessionRepositoryError,
) -> ProviderLifecycleIngressUsecaseError {
    match error {
        ProviderAgentSessionRepositoryError::AlreadyExists
        | ProviderAgentSessionRepositoryError::Conflict => {
            ProviderLifecycleIngressUsecaseError::Conflict
        }
        ProviderAgentSessionRepositoryError::ProviderSessionAlreadyOwned { .. } => {
            ProviderLifecycleIngressUsecaseError::Conflict
        }
        ProviderAgentSessionRepositoryError::InvalidRequest => {
            ProviderLifecycleIngressUsecaseError::InvalidInput
        }
        ProviderAgentSessionRepositoryError::Unavailable => {
            ProviderLifecycleIngressUsecaseError::StorageUnavailable
        }
        ProviderAgentSessionRepositoryError::Corrupt => {
            ProviderLifecycleIngressUsecaseError::Corrupt
        }
    }
}
