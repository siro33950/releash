use std::path::PathBuf;
use std::sync::Arc;

use crate::domain::agent_session::services::{
    admit_workflow_send_target, runtime_stop_request_id, workflow_send_receipt_matches,
    workflow_send_should_retry, WorkflowSendTargetRejection, INTERNAL_WORKFLOW_OPERATION_PRINCIPAL,
    WORKFLOW_SEND_RETRY_ATTEMPTS,
};
use crate::usecase::agent_session::runtime::{
    DurableStopDriver, DurableWorkflowSendDriver, DurableWorkflowSendError,
    DurableWorkflowSendPayloadEncoder, DurableWorkflowTurnRequest,
};
use crate::usecase::agent_session::session::{
    RuntimeTerminalParticipantProvider, RuntimeTerminalParticipants, SessionStore,
};

use super::{
    AgentSendOperationUsecase, SendCommandOutcome, SendOperationRequest, StopCommandOutcome,
    StopOperationError, StopOperationRequest, StopOperationState, StopOperationUsecase,
    LOCAL_INSTALLATION_OPERATION_PRINCIPAL,
};

struct DurableStopOperationDriver {
    operation: Arc<StopOperationUsecase>,
}

impl DurableStopOperationDriver {
    fn new(operation: Arc<StopOperationUsecase>) -> Self {
        Self { operation }
    }

    fn state_result(state: StopOperationState) -> Result<(), String> {
        match state {
            StopOperationState::Accepted | StopOperationState::Completed { .. } => Ok(()),
            StopOperationState::ReconciliationRequired { failure } => Err(failure.to_string()),
        }
    }

    fn operation_error(error: StopOperationError) -> String {
        match error {
            StopOperationError::InvalidRequest => "The durable Stop request is invalid.".into(),
            StopOperationError::PayloadConflict => {
                "The durable Stop identity is bound to another payload.".into()
            }
            StopOperationError::ShutdownInProgress => "Application shutdown is in progress.".into(),
            StopOperationError::NotFound => "The durable Stop operation was not found.".into(),
            StopOperationError::CapacityExceeded => {
                "The durable Stop capacity is exhausted.".into()
            }
            StopOperationError::StaleTarget => "The durable Stop target changed.".into(),
            StopOperationError::QueryBusy => "The durable Stop query is busy.".into(),
            StopOperationError::DeadlineExceeded => {
                "The durable Stop query deadline was exceeded.".into()
            }
            StopOperationError::StorageUnavailable { failure } => failure.to_string(),
            StopOperationError::Internal { correlation_id } => {
                format!("The durable Stop operation failed ({correlation_id}).")
            }
        }
    }
}

#[async_trait::async_trait]
impl DurableStopDriver for DurableStopOperationDriver {
    async fn stop(
        &self,
        session_id: &str,
        turn_id: u64,
        expected_session_revision: u64,
    ) -> Result<(), String> {
        let request_id = runtime_stop_request_id(session_id, turn_id);
        match self
            .operation
            .get_operation(LOCAL_INSTALLATION_OPERATION_PRINCIPAL, &request_id)
            .await
        {
            Ok((_, state)) => return Self::state_result(state),
            Err(StopOperationError::NotFound) => {}
            Err(error) => return Err(Self::operation_error(error)),
        }
        match self
            .operation
            .request(StopOperationRequest {
                principal: LOCAL_INSTALLATION_OPERATION_PRINCIPAL.to_string(),
                request_id,
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
                expected_session_revision,
            })
            .await
            .map_err(Self::operation_error)?
        {
            StopCommandOutcome::Accepted { state, .. } => Self::state_result(state),
            StopCommandOutcome::RejectedBeforeCommit { failure } => Err(failure.to_string()),
            StopCommandOutcome::OutcomeUnknown { request_id } => Err(format!(
                "The durable Stop acceptance is unknown ({request_id})."
            )),
        }
    }
}

pub(crate) fn bind_runtime_durable_stop_driver(
    runtime: &Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    operation: Arc<StopOperationUsecase>,
) {
    runtime.set_durable_stop_driver(Arc::new(DurableStopOperationDriver::new(operation)));
}

struct DurableWorkflowSendOperationDriver {
    operation: Arc<AgentSendOperationUsecase>,
    session_store: Arc<SessionStore>,
    data_dir: PathBuf,
    payload_encoder: Arc<dyn DurableWorkflowSendPayloadEncoder>,
}

#[async_trait::async_trait]
impl DurableWorkflowSendDriver for DurableWorkflowSendOperationDriver {
    async fn send(
        &self,
        request: DurableWorkflowTurnRequest,
    ) -> Result<(), DurableWorkflowSendError> {
        let meta = self
            .session_store
            .get_session_meta(&self.data_dir, &request.session_id)
            .map_err(DurableWorkflowSendError::SessionStore)?
            .ok_or_else(|| DurableWorkflowSendError::SessionNotFound(request.session_id.clone()))?;
        admit_workflow_send_target(
            meta.workflow_node_session,
            &meta.permission_mode,
            request.permission_mode.as_str(),
        )
        .map_err(|rejection| match rejection {
            WorkflowSendTargetRejection::NotWorkflowSession => {
                DurableWorkflowSendError::InvalidWorkflowTarget
            }
            WorkflowSendTargetRejection::AuthorityMismatch => {
                DurableWorkflowSendError::AuthorityMismatch
            }
        })?;
        let canonical_payload = self.payload_encoder.encode(&request, meta.plan_mode)?;
        let operation_request = SendOperationRequest {
            principal: INTERNAL_WORKFLOW_OPERATION_PRINCIPAL.to_string(),
            operation_id: request.operation_id,
            canonical_payload,
        };
        for attempt in 0..WORKFLOW_SEND_RETRY_ATTEMPTS {
            match self
                .operation
                .send(operation_request.clone())
                .await
                .map_err(DurableWorkflowSendError::Operation)?
            {
                SendCommandOutcome::Accepted(accepted) => {
                    if !workflow_send_receipt_matches(
                        &meta.id,
                        &accepted.receipt.session_id,
                        &accepted.receipt.disposition,
                    ) {
                        return Err(DurableWorkflowSendError::IncompatibleReceipt);
                    }
                    return Ok(());
                }
                SendCommandOutcome::RejectedBeforeCommit { failure }
                    if workflow_send_should_retry(failure.retryable, attempt) =>
                {
                    tokio::task::yield_now().await;
                }
                SendCommandOutcome::RejectedBeforeCommit { failure } => {
                    return Err(DurableWorkflowSendError::Admission(failure));
                }
                SendCommandOutcome::OutcomeUnknown { operation_id } => {
                    return Err(DurableWorkflowSendError::OutcomeUnknown(operation_id));
                }
            }
        }
        unreachable!("the bounded workflow Send retry loop always returns")
    }
}

pub(crate) fn bind_runtime_durable_workflow_send_driver(
    runtime: &Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    operation: Arc<AgentSendOperationUsecase>,
    session_store: Arc<SessionStore>,
    data_dir: PathBuf,
    payload_encoder: Arc<dyn DurableWorkflowSendPayloadEncoder>,
) {
    runtime.set_durable_workflow_send_driver(Arc::new(DurableWorkflowSendOperationDriver {
        operation,
        session_store,
        data_dir,
        payload_encoder,
    }));
}

struct RuntimeTerminalOperationParticipantProvider {
    stop_operation: Arc<StopOperationUsecase>,
    send_operation: Arc<AgentSendOperationUsecase>,
}

#[async_trait::async_trait]
impl RuntimeTerminalParticipantProvider for RuntimeTerminalOperationParticipantProvider {
    async fn prepare(
        &self,
        terminal: &crate::domain::local_event::TerminalRecordMutation,
    ) -> Result<RuntimeTerminalParticipants, String> {
        let mut participants = self
            .stop_operation
            .prepare_runtime_terminal_participants(terminal)
            .await?;
        let send = self
            .send_operation
            .prepare_runtime_terminal_participants(terminal)
            .await?;
        participants.events.extend(send.events);
        participants.mutations.extend(send.mutations);
        Ok(RuntimeTerminalParticipants {
            events: participants.events,
            mutations: participants.mutations,
        })
    }
}

pub(crate) fn bind_runtime_terminal_operation_participant_provider(
    session_store: &Arc<SessionStore>,
    stop_operation: Arc<StopOperationUsecase>,
    send_operation: Arc<AgentSendOperationUsecase>,
) {
    session_store.set_runtime_terminal_participant_provider(Arc::new(
        RuntimeTerminalOperationParticipantProvider {
            stop_operation,
            send_operation,
        },
    ));
}
