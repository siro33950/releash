use std::sync::Arc;

use crate::domain::agent_session::aggregates::AgentSessionMutationOutcome;
use crate::domain::agent_session::repository::{
    AgentSessionRepositoryError, VersionedAgentSession,
};
use crate::domain::provider_lifecycle::{
    ProviderLifecycleIngressResult, ProviderLifecycleSignal, ProviderLifecycleSignalKind,
    ProviderLifecycleSlotId, ProviderLifecycleUnavailableObservation, ScopedProviderLifecycleEvent,
};
use crate::domain::workflow::AgentSessionActivity;
use crate::usecase::agent_session::{
    AgentSessionChangeNotifier, AgentSessionUsecase, AgentSessionUsecaseError,
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
        session: VersionedAgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderExecutionTreeStopCommand {
    pub(crate) agent_session_id: String,
    pub(crate) tree_id: String,
    pub(crate) node_execution_id: String,
    pub(crate) binding_id: String,
}

#[async_trait::async_trait]
pub(crate) trait ProviderExecutionTreeStopTransaction: Send + Sync {
    async fn commit_provider_stop(
        &self,
        command: ProviderExecutionTreeStopCommand,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleIngressUsecaseError>;
}

impl From<ProviderLifecycleUsecaseError> for ProviderLifecycleIngressUsecaseError {
    fn from(error: ProviderLifecycleUsecaseError) -> Self {
        map_lifecycle_error(error)
    }
}

pub(crate) struct ProviderLifecycleIngressUsecase {
    lifecycle: Arc<ProviderLifecycleUsecase>,
    sessions: Arc<AgentSessionUsecase>,
    hook_health: Arc<ProviderHookHealthUsecase>,
    session_start_transaction: Arc<dyn ProviderSessionStartTransaction>,
    execution_tree_stop_transaction: Arc<dyn ProviderExecutionTreeStopTransaction>,
    change_notifier: Arc<dyn AgentSessionChangeNotifier>,
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
        sessions: Arc<AgentSessionUsecase>,
        hook_health: Arc<ProviderHookHealthUsecase>,
        session_start_transaction: Arc<dyn ProviderSessionStartTransaction>,
        execution_tree_stop_transaction: Arc<dyn ProviderExecutionTreeStopTransaction>,
        change_notifier: Arc<dyn AgentSessionChangeNotifier>,
    ) -> Self {
        Self {
            lifecycle,
            sessions,
            hook_health,
            session_start_transaction,
            execution_tree_stop_transaction,
            change_notifier,
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
        match kind {
            ProviderLifecycleSignalKind::SessionStarted {
                provider_session_id,
                transcript_ref,
            } => {
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
                    .receive_with_commit(
                        slot_id,
                        capability,
                        signal,
                        move |lifecycle_events| async move {
                            if lifecycle_events.is_empty() && !has_session_changes {
                                return Ok(());
                            }
                            transaction
                                .commit_session_started(
                                    prepared,
                                    lifecycle_events,
                                    &caller_request_id,
                                )
                                .await
                                .map(|_| ())
                                .map_err(map_session_repository_error)
                        },
                    )
                    .await?;
                if matches!(
                    result,
                    ProviderLifecycleIngressResult::Applied
                        | ProviderLifecycleIngressResult::Duplicate
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
                Ok(result)
            }
            ProviderLifecycleSignalKind::StopObserved { .. } => {
                let _operation = self
                    .sessions
                    .lock_operation(&agent_session_id)
                    .await
                    .map_err(map_session_error)?;
                let session = self
                    .sessions
                    .find(&agent_session_id)
                    .await
                    .map_err(map_session_error)?
                    .ok_or(ProviderLifecycleIngressUsecaseError::InvalidInput)?;
                let tree_location = session.session().tree_location().clone();
                let transaction = self.execution_tree_stop_transaction.clone();
                let command = ProviderExecutionTreeStopCommand {
                    agent_session_id,
                    tree_id: tree_location.tree_id().to_string(),
                    node_execution_id: tree_location.node_execution_id().to_string(),
                    binding_id,
                };
                self.lifecycle
                    .receive_with_commit(
                        slot_id,
                        capability,
                        signal,
                        move |lifecycle_events| async move {
                            transaction
                                .commit_provider_stop(command, lifecycle_events)
                                .await
                        },
                    )
                    .await
            }
            ProviderLifecycleSignalKind::StopFailed { .. } => {
                self.receive_activity_observation(
                    slot_id,
                    capability,
                    signal,
                    AgentSessionActivity::AwaitingInstruction,
                    "provider-stop-failed-activity",
                )
                .await
            }
            ProviderLifecycleSignalKind::ActivityObserved { activity, .. } => {
                self.receive_activity_observation(
                    slot_id,
                    capability,
                    signal,
                    activity,
                    "provider-activity",
                )
                .await
            }
        }
    }

    async fn receive_activity_observation(
        &self,
        slot_id: &ProviderLifecycleSlotId,
        capability: &str,
        signal: ProviderLifecycleSignal,
        activity: AgentSessionActivity,
        caller_request_id_prefix: &str,
    ) -> Result<ProviderLifecycleIngressResult, ProviderLifecycleIngressUsecaseError> {
        let agent_session_id = signal.scope().agent_session_id().to_string();
        let binding_id = signal.binding_id().to_string();
        let _operation = self
            .sessions
            .lock_operation(&agent_session_id)
            .await
            .map_err(map_session_error)?;
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
        let observation = self
            .sessions
            .observe_activity(
                &agent_session_id,
                activity,
                &format!(
                    "{caller_request_id_prefix}.{binding_id}.{}",
                    crate::other::id::unique_simple_id()
                ),
            )
            .await
            .map_err(map_session_error)?;
        if observation.outcome == AgentSessionMutationOutcome::Applied {
            self.change_notifier
                .agent_session_changed(&observation.worktree_path);
        }
        Ok(merge_activity_outcome(result, observation.outcome))
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

fn merge_activity_outcome(
    lifecycle_result: ProviderLifecycleIngressResult,
    activity_outcome: AgentSessionMutationOutcome,
) -> ProviderLifecycleIngressResult {
    if activity_outcome == AgentSessionMutationOutcome::Applied {
        ProviderLifecycleIngressResult::Applied
    } else {
        lifecycle_result
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

fn map_session_error(error: AgentSessionUsecaseError) -> ProviderLifecycleIngressUsecaseError {
    match error {
        AgentSessionUsecaseError::NotFound | AgentSessionUsecaseError::InvalidOperation => {
            ProviderLifecycleIngressUsecaseError::InvalidInput
        }
        AgentSessionUsecaseError::Conflict
        | AgentSessionUsecaseError::ProviderSessionAlreadyOwned { .. } => {
            ProviderLifecycleIngressUsecaseError::Conflict
        }
        AgentSessionUsecaseError::Unavailable => {
            ProviderLifecycleIngressUsecaseError::StorageUnavailable
        }
        AgentSessionUsecaseError::Corrupt => ProviderLifecycleIngressUsecaseError::Corrupt,
    }
}

fn map_session_repository_error(
    error: AgentSessionRepositoryError,
) -> ProviderLifecycleIngressUsecaseError {
    match error {
        AgentSessionRepositoryError::Conflict => ProviderLifecycleIngressUsecaseError::Conflict,
        AgentSessionRepositoryError::ProviderSessionAlreadyOwned { .. } => {
            ProviderLifecycleIngressUsecaseError::Conflict
        }
        AgentSessionRepositoryError::InvalidRequest => {
            ProviderLifecycleIngressUsecaseError::InvalidInput
        }
        AgentSessionRepositoryError::Unavailable => {
            ProviderLifecycleIngressUsecaseError::StorageUnavailable
        }
        AgentSessionRepositoryError::Corrupt => ProviderLifecycleIngressUsecaseError::Corrupt,
    }
}
