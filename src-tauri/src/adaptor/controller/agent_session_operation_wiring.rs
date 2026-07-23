//! Production adapters from durable operation usecases to the existing
//! agent-session runtime. They only snapshot or execute after acceptance;
//! operation decisions remain in Rust usecases.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::local_event::{
    LocalEventQuery, LocalEventQueryResult, LocalEventTransactionRepository, ObligationRecord,
    SafeOperationFailure, SendObligationKindRecord, SessionLifecycleRecordAction,
    SessionOperationFailureKind,
};
use crate::usecase::agent_session::operation::{
    validate_operation_identity, AcceptedPermissionResponseEffect, AcceptedSendEffect,
    AcceptedStopEffect, AgentSendOperationUsecase, BackendRecoveryReadbackPort,
    BackendRecoveryReadbackRequest, PermissionResponseExecutionStatus, PermissionResponseGate,
    PermissionResponseOperationUsecase, PermissionResponsePlan, RecoveryEffectExecutor,
    RecoveryEffectRequest, RecoveryEffectResult, RecoveryOwnerBatch, SendAdmissionGate, SendPlan,
    SendRecoveryReadbackKind, SendRecoveryReadbackPort, SendRecoveryReadbackRequest,
    SessionCloseRecoveryReadbackPort, SessionCloseRecoveryReadbackRequest, SessionLifecycleAction,
    SessionLifecycleEffect, SessionLifecycleGate, SessionLifecycleOperationUsecase,
    SessionLifecycleSnapshot, SessionLifecycleState, StableRecoveryEffectIdentity,
    StopAdmissionGate, StopCommandOutcome, StopEffectObservation, StopOperationError,
    StopOperationRequest, StopOperationState, StopOperationUsecase, StopRecoveryReadbackPort,
    StopRecoveryReadbackRequest, StopTargetSnapshot,
};
use crate::usecase::agent_session::session::{NextTurnIdError, SessionState, SessionStore};

/// All local transports operate under the same installation authority. The
/// renderer caller journal remains Tauri-owned, but its operation binding must
/// converge with an authenticated loopback WebSocket retry.
pub(crate) const LOCAL_INSTALLATION_OPERATION_PRINCIPAL: &str = "local-app";
const INTERNAL_WORKFLOW_OPERATION_PRINCIPAL: &str = "workflow-runtime";

fn runtime_stop_request_id(session_id: &str, turn_id: u64) -> String {
    let digest =
        Sha256::digest(format!("runtime-stop-request/v1\0{session_id}\0{turn_id}").as_bytes());
    format!("runtime-stop-{}", hex::encode(digest))
}

/// Routes workflow/internal runtime interrupts through the same durable Stop
/// boundary as Tauri and loopback WebSocket callers. `StopOperationUsecase`
/// continues to derive the backend operation identity and owner binding.
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
impl crate::usecase::agent_session::runtime::DurableStopDriver for DurableStopOperationDriver {
    async fn stop(
        &self,
        session_id: &str,
        turn_id: u64,
        expected_session_revision: u64,
    ) -> Result<(), String> {
        let request_id = runtime_stop_request_id(session_id, turn_id);
        // A repeated internal call may observe the post-acceptance projection
        // revision. Querying by the deterministic caller identity first keeps
        // it a pure replay instead of rebinding that identity to a new CAS.
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
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::runtime::DurableWorkflowSendDriver
    for DurableWorkflowSendOperationDriver
{
    async fn send(
        &self,
        request: crate::usecase::agent_session::runtime::DurableWorkflowTurnRequest,
    ) -> Result<(), String> {
        let meta = self
            .session_store
            .get_session_meta(&self.data_dir, &request.session_id)?
            .ok_or_else(|| {
                format!(
                    "The workflow turn session does not exist: {}",
                    request.session_id
                )
            })?;
        if !meta.workflow_node_session {
            return Err("The durable workflow Send target is not a workflow session.".to_string());
        }
        if meta.permission_mode != request.permission_mode.as_str() {
            return Err(
                "The durable workflow Send permission differs from the session authority."
                    .to_string(),
            );
        }
        let canonical_payload = serde_json::to_string(&CanonicalSendCommandV1 {
            target: CanonicalSendTargetV1::WorkflowTurn {
                chat_session_id: request.session_id,
                base_system_prompt: request.base_system_prompt,
                workflow_instructions: request.workflow_instructions,
            },
            content: request.content,
            permission_mode: request.permission_mode.as_str().to_string(),
            plan_mode: meta.plan_mode,
            backend_id: None,
            model_id: None,
            images: Vec::new(),
            mentions: Vec::new(),
            editor_context: None,
        })
        .map_err(|_| "The durable workflow Send payload could not be encoded.".to_string())?;
        match self
            .operation
            .send(crate::usecase::agent_session::operation::SendOperationRequest {
                principal: INTERNAL_WORKFLOW_OPERATION_PRINCIPAL.to_string(),
                operation_id: request.operation_id,
                canonical_payload,
            })
            .await
            .map_err(|error| format!("The durable workflow Send failed: {error:?}"))?
        {
            crate::usecase::agent_session::operation::SendCommandOutcome::Accepted(accepted) => {
                if accepted.receipt.session_id != meta.id
                    || !matches!(
                        accepted.receipt.disposition,
                        crate::domain::agent_session::events::SendDisposition::StartedTurn { .. }
                    )
                {
                    return Err(
                        "The durable workflow Send converged on an incompatible receipt."
                            .to_string(),
                    );
                }
                Ok(())
            }
            crate::usecase::agent_session::operation::SendCommandOutcome::RejectedBeforeCommit {
                failure,
            } => Err(failure.to_string()),
            crate::usecase::agent_session::operation::SendCommandOutcome::OutcomeUnknown {
                operation_id,
            } => Err(format!(
                "The durable workflow Send acceptance is unknown ({operation_id})."
            )),
        }
    }
}

pub(crate) fn bind_runtime_durable_workflow_send_driver(
    runtime: &Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    operation: Arc<AgentSendOperationUsecase>,
    session_store: Arc<SessionStore>,
    data_dir: PathBuf,
) {
    runtime.set_durable_workflow_send_driver(Arc::new(DurableWorkflowSendOperationDriver {
        operation,
        session_store,
        data_dir,
    }));
}

struct RuntimeTerminalOperationParticipantProvider {
    stop_operation: Arc<StopOperationUsecase>,
    send_operation: Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::session::RuntimeTerminalParticipantProvider
    for RuntimeTerminalOperationParticipantProvider
{
    async fn prepare(
        &self,
        terminal: &crate::domain::local_event::TerminalRecordMutation,
    ) -> Result<crate::usecase::agent_session::session::RuntimeTerminalParticipants, String> {
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
        Ok(
            crate::usecase::agent_session::session::RuntimeTerminalParticipants {
                events: participants.events,
                mutations: participants.mutations,
            },
        )
    }
}

pub(crate) fn bind_runtime_terminal_operation_participant_provider(
    session_store: &Arc<SessionStore>,
    stop_operation: Arc<StopOperationUsecase>,
    send_operation: Arc<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
) {
    session_store.set_runtime_terminal_participant_provider(Arc::new(
        RuntimeTerminalOperationParticipantProvider {
            stop_operation,
            send_operation,
        },
    ));
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CanonicalSendTargetV1 {
    Direct {
        chat_session_id: Option<String>,
        worktree_path: String,
    },
    WorkflowApproval {
        execution_id: String,
    },
    WorkflowTurn {
        chat_session_id: String,
        base_system_prompt: Option<String>,
        workflow_instructions: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CanonicalSendCommandV1 {
    pub target: CanonicalSendTargetV1,
    pub content: String,
    pub permission_mode: String,
    pub plan_mode: bool,
    pub backend_id: Option<String>,
    pub model_id: Option<String>,
    pub images: Vec<crate::usecase::agent_session::session::ImageAttachment>,
    pub mentions: Vec<crate::adaptor::protocol::mention::MentionReferenceInput>,
    pub editor_context: Option<crate::usecase::agent_session::runtime::usecase::AgentEditorContext>,
}

impl CanonicalSendCommandV1 {
    pub(crate) fn into_runtime_request(
        self,
        accepted_session_id: &str,
        accepted_worktree_path: &str,
    ) -> Result<
        crate::usecase::agent_session::runtime::usecase::AcceptedRuntimeSendInput,
        SafeOperationFailure,
    > {
        let permission_mode = crate::domain::agent_session::PermissionMode::parse(
            &self.permission_mode,
        )
        .map_err(|_| RuntimeSendOperationGate::failure("The permission mode is invalid."))?;
        let (base_system_prompt, workflow_instructions) = match &self.target {
            CanonicalSendTargetV1::WorkflowTurn {
                base_system_prompt,
                workflow_instructions,
                ..
            } => (base_system_prompt.clone(), workflow_instructions.clone()),
            CanonicalSendTargetV1::Direct { .. }
            | CanonicalSendTargetV1::WorkflowApproval { .. } => (None, Vec::new()),
        };
        // Acceptance already fixed and materialized the target session and
        // effective provider configuration. Validate that the immutable
        // command still names that target, but never hand mutable target or
        // configuration fields to the effect-only runtime seam.
        match &self.target {
            CanonicalSendTargetV1::Direct {
                chat_session_id: Some(session_id),
                worktree_path,
            } if session_id != accepted_session_id || worktree_path != accepted_worktree_path => {
                return Err(RuntimeSendOperationGate::failure(
                    "The accepted send target no longer matches its receipt.",
                ));
            }
            CanonicalSendTargetV1::Direct {
                chat_session_id: None,
                worktree_path,
            } if worktree_path != accepted_worktree_path => {
                return Err(RuntimeSendOperationGate::failure(
                    "The accepted send worktree no longer matches its receipt.",
                ));
            }
            CanonicalSendTargetV1::WorkflowTurn {
                chat_session_id, ..
            } if chat_session_id != accepted_session_id => {
                return Err(RuntimeSendOperationGate::failure(
                    "The accepted workflow send target no longer matches its receipt.",
                ));
            }
            CanonicalSendTargetV1::Direct { .. }
            | CanonicalSendTargetV1::WorkflowApproval { .. }
            | CanonicalSendTargetV1::WorkflowTurn { .. } => {}
        }
        Ok(
            crate::usecase::agent_session::runtime::usecase::AcceptedRuntimeSendInput {
                content: self.content,
                permission_mode,
                plan_mode: self.plan_mode,
                images: self.images,
                mentions: crate::adaptor::protocol::mention::into_domain_vec(self.mentions),
                editor_context: self.editor_context,
                base_system_prompt,
                workflow_instructions,
            },
        )
    }
}

pub(crate) struct RuntimeSendOperationGate {
    runtime: Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    session_store: Arc<SessionStore>,
    data_dir: PathBuf,
    status_sink:
        OnceLock<Weak<crate::usecase::agent_session::operation::AgentSendOperationUsecase>>,
    workflow_runtime: OnceLock<Weak<crate::usecase::workflow::WorkflowRuntimeUsecase>>,
}

impl RuntimeSendOperationGate {
    pub(crate) fn new(
        runtime: Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
        session_store: Arc<SessionStore>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            runtime,
            session_store,
            data_dir,
            status_sink: OnceLock::new(),
            workflow_runtime: OnceLock::new(),
        }
    }

    pub(crate) fn bind_workflow_runtime(
        &self,
        runtime: Weak<crate::usecase::workflow::WorkflowRuntimeUsecase>,
    ) {
        let _ = self.workflow_runtime.set(runtime);
    }

    pub(crate) fn bind_status_sink(
        &self,
        sink: Weak<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
    ) {
        let _ = self.status_sink.set(sink.clone());
        self.runtime.set_accepted_send_obligation_driver(Arc::new(
            RuntimeAcceptedSendObligationDriver { sink },
        ));
    }

    fn failure(label: &str) -> SafeOperationFailure {
        SafeOperationFailure::new(
            SessionOperationFailureKind::PersistFailure,
            true,
            label,
            uuid::Uuid::new_v4().to_string(),
        )
    }

    fn turn_identity_failure(error: NextTurnIdError) -> SafeOperationFailure {
        match error {
            NextTurnIdError::CapacityExceeded => SafeOperationFailure::new(
                SessionOperationFailureKind::CapacityExceeded,
                false,
                "The session has exhausted its turn identity capacity.",
                uuid::Uuid::new_v4().to_string(),
            ),
            NextTurnIdError::Unavailable(_) => {
                Self::failure("The next turn identity is unavailable.")
            }
        }
    }
}

struct RuntimeAcceptedSendObligationDriver {
    sink: Weak<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
}

async fn persist_accepted_send_status(
    sink: &crate::usecase::agent_session::operation::AgentSendOperationUsecase,
    operation_id: &str,
    status: crate::usecase::agent_session::operation::SendExecutionStatus,
) {
    loop {
        if sink
            .record_execution_status(operation_id, status.clone())
            .await
            .is_ok()
        {
            return;
        }
        // The accepted operation and pending obligation remain the durable
        // retry identity. Do not let an external-effect task finish while its
        // canonical outcome is still absent.
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::runtime::ports::AcceptedSendObligationDriver
    for RuntimeAcceptedSendObligationDriver
{
    async fn reserve_turn_execution(
        &self,
        operation_id: &str,
        obligation_id: &str,
    ) -> Result<(), ()> {
        let sink = self.sink.upgrade().ok_or(())?;
        sink.transition_obligation(
            crate::usecase::agent_session::operation::ObligationTransition {
                operation_id,
                obligation_id,
                expected_kind: "turn_execution",
                expected_state: "pending",
                next_state: "effect_reserved",
                keep_pending: true,
                status: Some(
                crate::usecase::agent_session::operation::SendExecutionStatus::ProviderStartReserved {
                    obligation_id: obligation_id.to_string(),
                },
                ),
            },
        )
        .await
        .map_err(|_| ())
    }

    async fn mark_turn_running(
        &self,
        operation_id: &str,
        obligation_id: &str,
        turn_id: u64,
    ) -> Result<(), ()> {
        let sink = self.sink.upgrade().ok_or(())?;
        sink.mark_turn_running(operation_id, obligation_id, turn_id)
            .await
            .map_err(|_| ())
    }

    async fn reconcile_turn_execution(&self, operation_id: &str, _obligation_id: &str) {
        let Some(sink) = self.sink.upgrade() else {
            return;
        };
        persist_accepted_send_status(
            &sink,
            operation_id,
            crate::usecase::agent_session::operation::SendExecutionStatus::ReconciliationRequired {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::OutcomeUnknown,
                    true,
                    "Queued turn execution requires same-effect readback.",
                    uuid::Uuid::new_v4().to_string(),
                ),
            },
        )
        .await;
    }
}

#[async_trait::async_trait]
impl SendAdmissionGate for RuntimeSendOperationGate {
    async fn plan_send(
        &self,
        principal: &str,
        canonical_payload: &str,
    ) -> Result<SendPlan, SafeOperationFailure> {
        let command: CanonicalSendCommandV1 = serde_json::from_str(canonical_payload)
            .map_err(|_| Self::failure("The exact send payload is incompatible."))?;
        let digest = Sha256::digest(canonical_payload.as_bytes());
        let input_ref = format!("send-input-v1:{}", hex::encode(digest));
        // Workflow targets are deliberately resolved only from this gate. The
        // send usecase performs its existing-operation lookup before invoking
        // the gate, so response-loss/restart replay never depends on the
        // workflow still being at its original approval checkpoint.
        let workflow_turn = matches!(&command.target, CanonicalSendTargetV1::WorkflowTurn { .. });
        let (chat_session_id, worktree_path, workflow_projection_busy) = match &command.target {
            CanonicalSendTargetV1::Direct {
                chat_session_id,
                worktree_path,
            } => (chat_session_id.clone(), worktree_path.clone(), false),
            CanonicalSendTargetV1::WorkflowApproval { execution_id } => {
                let runtime = self
                    .workflow_runtime
                    .get()
                    .and_then(Weak::upgrade)
                    .ok_or_else(|| {
                        Self::failure("The workflow approval resolver is unavailable.")
                    })?;
                let target = runtime
                    .prepare_approval_chat(execution_id, &command.content)
                    .await
                    .map_err(|_| Self::failure("The workflow approval target is unavailable."))?;
                (Some(target.chat_session_id), target.worktree_path, false)
            }
            CanonicalSendTargetV1::WorkflowTurn {
                chat_session_id, ..
            } => {
                if principal != INTERNAL_WORKFLOW_OPERATION_PRINCIPAL {
                    return Err(Self::failure(
                        "The workflow turn target is reserved for the workflow runtime.",
                    ));
                }
                let meta = self
                    .session_store
                    .get_session_meta(&self.data_dir, chat_session_id)
                    .map_err(|_| Self::failure("The workflow turn target is unavailable."))?
                    .ok_or_else(|| Self::failure("The workflow turn target does not exist."))?;
                if !matches!(meta.state, SessionState::Active | SessionState::Idle) {
                    return Err(Self::failure("The workflow turn target is not open."));
                }
                let projection_busy = meta.state != SessionState::Idle
                    || self
                        .session_store
                        .load_queue_paused_at(&self.data_dir, chat_session_id)
                        .map_err(|_| {
                            Self::failure("The workflow turn queue state is unavailable.")
                        })?
                        .is_some();
                (
                    Some(chat_session_id.clone()),
                    meta.worktree_path,
                    projection_busy,
                )
            }
        };
        let session_id = chat_session_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.session_store
            .ensure_no_unresolved_recovery(&session_id)
            .await?;
        let (busy, provider_established, next_turn_id) = if workflow_turn {
            (
                workflow_projection_busy
                    || self
                        .runtime
                        .workflow_send_runtime_is_busy(&session_id)
                        .await,
                self.runtime
                    .provider_session_is_confirmed(&session_id)
                    .await,
                self.session_store
                    .next_turn_id(&self.data_dir, &session_id)
                    .map_err(Self::turn_identity_failure)?,
            )
        } else if chat_session_id.is_some() {
            let response = self
                .runtime
                .get_session(&session_id)
                .await
                .map_err(|_| Self::failure("The send target is unavailable."))?
                .ok_or_else(|| Self::failure("The send target does not exist."))?;
            let provider_established = self
                .runtime
                .provider_session_is_confirmed(&session_id)
                .await;
            (
                response.turn_phase != crate::usecase::agent_session::status::TurnPhase::Idle
                    || response.queue_paused
                    || response.pending_queue_count > 0,
                provider_established,
                self.session_store
                    .next_turn_id(&self.data_dir, &session_id)
                    .map_err(Self::turn_identity_failure)?,
            )
        } else {
            (false, false, 1)
        };
        if workflow_turn && busy {
            return Err(Self::failure(
                "The workflow turn target already has pending work.",
            ));
        }
        let queue_item_id = format!("queue-{}", &hex::encode(digest)[..32]);
        let prompt = crate::domain::agent_session::events::PromptInput {
            content: command.content.clone(),
            mentions: crate::adaptor::protocol::mention::into_domain_vec(command.mentions.clone()),
            attachment_refs: Vec::new(),
            parts: command
                .images
                .iter()
                .map(
                    |image| crate::domain::agent_session::entities::MessagePart::Image {
                        data: image.data.clone(),
                        media_type: image.media_type.clone(),
                    },
                )
                .collect(),
        };
        let initial_session = if chat_session_id.is_none() {
            let permission_mode =
                crate::domain::agent_session::PermissionMode::parse(&command.permission_mode)
                    .map_err(|_| Self::failure("The permission mode is invalid."))?;
            Some(
                crate::usecase::agent_session::session::build_new_session_with_id(
                    session_id.clone(),
                    &worktree_path,
                    command.backend_id.clone(),
                    permission_mode,
                    command.model_id.clone(),
                    command.plan_mode,
                    false,
                    None,
                ),
            )
        } else {
            None
        };
        Ok(SendPlan {
            session_id,
            initial_session,
            disposition: if busy {
                crate::domain::agent_session::events::SendDisposition::Queued {
                    queue_item_id: queue_item_id.clone(),
                }
            } else {
                crate::domain::agent_session::events::SendDisposition::StartedTurn {
                    turn_id: next_turn_id.to_string(),
                }
            },
            input_ref,
            human_message_id: format!("human-{}", &hex::encode(digest)[..32]),
            prompt,
            reserved_turn_id: busy.then(|| next_turn_id.to_string()),
            provider_established,
        })
    }

    async fn acceptance_state_mutations(
        &self,
        plan: &SendPlan,
        events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, SafeOperationFailure> {
        self.session_store
            .prepare_send_acceptance_mutations(
                crate::usecase::agent_session::session::SendAcceptanceProjectionInput {
                    session_id: &plan.session_id,
                    initial_session: plan.initial_session.as_ref(),
                    human_message_id: &plan.human_message_id,
                    prompt: &plan.prompt,
                    disposition: &plan.disposition,
                    reserved_turn_id: plan.reserved_turn_id.as_deref(),
                    input_ref: &plan.input_ref,
                    events,
                },
            )
            .map_err(|_| Self::failure("The send projection could not be prepared."))
    }

    async fn start_provider_effect(&self, effect: &AcceptedSendEffect) {
        let Ok(command) = serde_json::from_str::<CanonicalSendCommandV1>(&effect.canonical_payload)
        else {
            log::error!(
                "accepted send payload is incompatible [{}]",
                effect.operation_id
            );
            return;
        };
        // The accepted obligation owns the resolved session identity. For a
        // workflow send, recover its worktree from the durable session shell
        // rather than resolving mutable workflow state for a second time.
        let accepted_worktree_path = match self
            .session_store
            .get_session_meta(&self.data_dir, &effect.session_id)
        {
            Ok(Some(meta)) => meta.worktree_path,
            _ => {
                log::error!(
                    "accepted send target is unavailable [{}]",
                    effect.operation_id
                );
                return;
            }
        };
        let Ok(request) = command
            .clone()
            .into_runtime_request(&effect.session_id, &accepted_worktree_path)
        else {
            return;
        };
        let runtime = self.runtime.clone();
        let status_sink = self.status_sink.get().cloned();
        let operation_id = effect.operation_id.clone();
        let obligation_id = effect.execution_obligation_id.clone();
        let establish_obligation_id = effect.establish_obligation_id.clone();
        let disposition = effect.disposition.clone();
        let reserved_turn_id = effect.reserved_turn_id.clone();
        let session_id = effect.session_id.clone();
        let human_message_id = effect.human_message_id.clone();
        let assistant_message_id = effect.assistant_message_id.clone();
        tokio::spawn(async move {
            let Some(sink) = status_sink.as_ref().and_then(Weak::upgrade) else {
                log::error!("accepted send status authority is unavailable [{operation_id}]");
                return;
            };
            if let Some(establish_obligation_id) = establish_obligation_id.as_deref() {
                if sink
                    .transition_obligation(
                        crate::usecase::agent_session::operation::ObligationTransition {
                            operation_id: &operation_id,
                            obligation_id: establish_obligation_id,
                            expected_kind: "provider_establish",
                            expected_state: "pending",
                            next_state: "effect_reserved",
                            keep_pending: true,
                            status: None,
                        },
                    )
                    .await
                    .is_err()
                {
                    return;
                }
                if runtime
                    .establish_accepted_provider(&session_id)
                    .await
                    .is_err()
                    || sink
                        .transition_obligation(
                            crate::usecase::agent_session::operation::ObligationTransition {
                                operation_id: &operation_id,
                                obligation_id: establish_obligation_id,
                                expected_kind: "provider_establish",
                                expected_state: "effect_reserved",
                                next_state: "completed",
                                keep_pending: false,
                                status: None,
                            },
                        )
                        .await
                        .is_err()
                {
                    persist_accepted_send_status(
                        &sink,
                        &operation_id,
                        crate::usecase::agent_session::operation::SendExecutionStatus::ReconciliationRequired {
                            failure: SafeOperationFailure::new(
                                SessionOperationFailureKind::OutcomeUnknown,
                                true,
                                "Provider establishment requires same-effect readback.",
                                uuid::Uuid::new_v4().to_string(),
                            ),
                        },
                    )
                    .await;
                    return;
                }
            }
            let immediate_turn = matches!(
                &disposition,
                crate::domain::agent_session::events::SendDisposition::StartedTurn { .. }
            );
            if immediate_turn
                && sink
                    .transition_obligation(
                        crate::usecase::agent_session::operation::ObligationTransition {
                            operation_id: &operation_id,
                            obligation_id: &obligation_id,
                            expected_kind: "turn_execution",
                            expected_state: "pending",
                            next_state: "effect_reserved",
                            keep_pending: true,
                            status: Some(
                            crate::usecase::agent_session::operation::SendExecutionStatus::ProviderStartReserved {
                                obligation_id: obligation_id.clone(),
                            },
                            ),
                        },
                    )
                    .await
                    .is_err()
            {
                return;
            }
            if runtime
                .execute_accepted_send(
                    crate::usecase::agent_session::runtime::AcceptedSendExecution {
                        request,
                        operation_id: &operation_id,
                        execution_obligation_id: &obligation_id,
                        session_id: &session_id,
                        human_message_id: &human_message_id,
                        assistant_message_id: assistant_message_id.as_deref(),
                        disposition: disposition.clone(),
                        reserved_turn_id: reserved_turn_id.as_deref(),
                    },
                )
                .await
                .is_err()
            {
                persist_accepted_send_status(
                    &sink,
                    &operation_id,
                    crate::usecase::agent_session::operation::SendExecutionStatus::ReconciliationRequired {
                        failure: SafeOperationFailure::new(
                            SessionOperationFailureKind::ExternalEffectFailed,
                            true,
                            "The accepted send requires reconciliation.",
                            uuid::Uuid::new_v4().to_string(),
                        ),
                    },
                )
                .await;
                log::warn!(
                    "accepted send effect requires reconciliation [{}]",
                    operation_id
                );
            } else {
                let status = match disposition {
                    crate::domain::agent_session::events::SendDisposition::StartedTurn {
                        turn_id,
                    } => crate::usecase::agent_session::operation::SendExecutionStatus::Running {
                        turn_id,
                    },
                    crate::domain::agent_session::events::SendDisposition::Queued {
                        queue_item_id,
                    } => crate::usecase::agent_session::operation::SendExecutionStatus::Queued {
                        queue_item_id,
                        reserved_turn_id: reserved_turn_id.unwrap_or_default(),
                    },
                };
                match status {
                    crate::usecase::agent_session::operation::SendExecutionStatus::Running {
                        turn_id,
                    } => match turn_id.parse::<u64>() {
                        Ok(turn_id) => {
                            if sink
                                .mark_turn_running(&operation_id, &obligation_id, turn_id)
                                .await
                                .is_err()
                            {
                                persist_accepted_send_status(
                                    &sink,
                                    &operation_id,
                                    crate::usecase::agent_session::operation::SendExecutionStatus::ReconciliationRequired {
                                        failure: SafeOperationFailure::new(
                                            SessionOperationFailureKind::OutcomeUnknown,
                                            true,
                                            "The accepted turn start requires same-effect readback.",
                                            uuid::Uuid::new_v4().to_string(),
                                        ),
                                    },
                                )
                                .await;
                            }
                        }
                        Err(_) => {
                            persist_accepted_send_status(
                                &sink,
                                &operation_id,
                                crate::usecase::agent_session::operation::SendExecutionStatus::ReconciliationRequired {
                                    failure: SafeOperationFailure::new(
                                        SessionOperationFailureKind::InvalidEffectIntent,
                                        false,
                                        "The accepted turn identity is incompatible.",
                                        uuid::Uuid::new_v4().to_string(),
                                    ),
                                },
                            )
                            .await;
                        }
                    },
                    crate::usecase::agent_session::operation::SendExecutionStatus::Queued {
                        queue_item_id,
                        reserved_turn_id,
                    } => {
                        persist_accepted_send_status(
                            &sink,
                            &operation_id,
                            crate::usecase::agent_session::operation::SendExecutionStatus::Queued {
                                queue_item_id,
                                reserved_turn_id,
                            },
                        )
                        .await;
                        runtime.drain_accepted_queue_if_idle(&session_id).await;
                    }
                    status => {
                        persist_accepted_send_status(&sink, &operation_id, status).await;
                    }
                }
            }
        });
    }
}

pub(crate) struct RuntimeAgentSessionOperationGate {
    runtime: Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    session_store: Arc<SessionStore>,
    data_dir: PathBuf,
    stop_operation: OnceLock<Weak<StopOperationUsecase>>,
    send_operation:
        OnceLock<Weak<crate::usecase::agent_session::operation::AgentSendOperationUsecase>>,
}

impl RuntimeAgentSessionOperationGate {
    pub(crate) fn new(
        runtime: Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
        session_store: Arc<SessionStore>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            runtime,
            session_store,
            data_dir,
            stop_operation: OnceLock::new(),
            send_operation: OnceLock::new(),
        }
    }

    pub(crate) fn bind_stop_operation(&self, operation: Weak<StopOperationUsecase>) {
        let _ = self.stop_operation.set(operation);
    }

    pub(crate) fn bind_send_operation(
        &self,
        operation: Weak<crate::usecase::agent_session::operation::AgentSendOperationUsecase>,
    ) {
        let _ = self.send_operation.set(operation);
    }

    fn failure(label: &str) -> SafeOperationFailure {
        SafeOperationFailure::new(
            SessionOperationFailureKind::ExternalEffectFailed,
            true,
            label,
            uuid::Uuid::new_v4().to_string(),
        )
    }
}

#[async_trait::async_trait]
impl SessionLifecycleGate for RuntimeAgentSessionOperationGate {
    async fn session_snapshot(
        &self,
        session_id: &str,
    ) -> Result<SessionLifecycleSnapshot, SafeOperationFailure> {
        let meta = self
            .session_store
            .get_session_meta(&self.data_dir, session_id)
            .map_err(|_| Self::failure("The session snapshot is unavailable."))?
            .ok_or_else(|| Self::failure("The session does not exist."))?;
        let runtime = self
            .runtime
            .get_session(session_id)
            .await
            .map_err(|_| Self::failure("The session runtime snapshot is unavailable."))?;
        let queue_paused = runtime.as_ref().is_some_and(|value| value.queue_paused);
        let has_pending_permission = runtime
            .as_ref()
            .is_some_and(|value| value.pending_permission_request.is_some());
        let lifecycle = match meta.state {
            SessionState::Closed => SessionLifecycleState::Closed,
            SessionState::Archived => SessionLifecycleState::Archived,
            _ => SessionLifecycleState::Open {
                idle: runtime.as_ref().is_none_or(|value| {
                    value.turn_phase == crate::usecase::agent_session::status::TurnPhase::Idle
                }),
                active_turn_id: runtime
                    .as_ref()
                    .filter(|value| {
                        value.turn_phase != crate::usecase::agent_session::status::TurnPhase::Idle
                    })
                    .and_then(|value| value.active_turn_id),
            },
        };
        Ok(SessionLifecycleSnapshot {
            session_revision: i64::try_from(meta.state_revision)
                .map_err(|_| Self::failure("The session revision is out of range."))?,
            lifecycle,
            queue_paused,
            has_runtime: self.runtime.has_live_runtime(session_id).await,
            has_pending_permission,
            has_pending_recovery: meta.pending_recovery_message.is_some(),
            has_pending_provider_operation: runtime.as_ref().is_some_and(|value| {
                value.turn_phase != crate::usecase::agent_session::status::TurnPhase::Idle
            }),
        })
    }

    async fn acceptance_state_mutations(
        &self,
        session_id: &str,
        action: &SessionLifecycleAction,
        events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, SafeOperationFailure> {
        let (state, backend_selection) = match action {
            SessionLifecycleAction::Close => (SessionState::Closed, None),
            SessionLifecycleAction::ArchiveOpen | SessionLifecycleAction::ArchiveClosed => {
                (SessionState::Archived, None)
            }
            SessionLifecycleAction::SwitchBackend { backend_id } => {
                let selected_model = self
                    .runtime
                    .default_model_for_backend(backend_id)
                    .map_err(|_| Self::failure("The selected backend is unavailable."))?;
                (
                    SessionState::Idle,
                    Some((backend_id.as_str(), selected_model)),
                )
            }
        };
        self.session_store
            .prepare_lifecycle_acceptance_mutations(
                session_id,
                events,
                state,
                backend_selection
                    .as_ref()
                    .map(|(backend_id, model)| (*backend_id, model.as_str())),
            )
            .map_err(|_| Self::failure("The session lifecycle projection could not be prepared."))
    }

    async fn terminal_participants(
        &self,
        terminal: &crate::domain::local_event::TerminalRecordMutation,
    ) -> Result<crate::usecase::agent_session::operation::TerminalParticipants, SafeOperationFailure>
    {
        let stop_operation = self
            .stop_operation
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| Self::failure("The Stop terminal participant is unavailable."))?;
        let mut participants = stop_operation
            .prepare_runtime_terminal_participants(terminal)
            .await
            .map_err(|_| Self::failure("The Stop terminal participant could not be prepared."))?;
        let send_operation = self
            .send_operation
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| Self::failure("The send terminal participant is unavailable."))?;
        let send = send_operation
            .prepare_runtime_terminal_participants(terminal)
            .await
            .map_err(|_| Self::failure("The send terminal participant could not be prepared."))?;
        participants.events.extend(send.events);
        participants.mutations.extend(send.mutations);
        Ok(participants)
    }

    async fn execute(&self, effect: &SessionLifecycleEffect) -> Result<(), SafeOperationFailure> {
        match &effect.action {
            SessionLifecycleAction::Close
            | SessionLifecycleAction::ArchiveOpen
            | SessionLifecycleAction::SwitchBackend { .. } => self
                .runtime
                .force_close_session(&effect.session_id)
                .await
                .map_err(|_| Self::failure("The session runtime could not be closed.")),
            SessionLifecycleAction::ArchiveClosed => Ok(()),
        }
    }
}

#[async_trait::async_trait]
impl BackendRecoveryReadbackPort for RuntimeAgentSessionOperationGate {
    async fn read_backend_recovery(
        &self,
        request: &BackendRecoveryReadbackRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
        use crate::domain::agent_session::events::{
            AgentSessionDomainEvent, RecoveryResultClassification,
        };

        let expected_identity = format!(
            "backend-recovery:{}:{}",
            request.session_id, request.recovery_id
        );
        if request.effect_identity.as_str() != expected_identity {
            return Err(Self::failure(
                "The backend-recovery readback identity does not match its durable effect.",
            ));
        }
        let events = self
            .session_store
            .load_session_events(&self.data_dir, &request.session_id)
            .map_err(|_| Self::failure("The backend-recovery result could not be read."))?;
        let mut started = false;
        let mut result = None;
        for event in events {
            match event {
                AgentSessionDomainEvent::BackendSessionRecoveryStarted { recovery_id, .. }
                    if recovery_id == request.recovery_id =>
                {
                    started = true;
                    result = Some((
                        RecoveryResultClassification::Pending,
                        "The backend session recovery remains in progress.".to_string(),
                    ));
                }
                AgentSessionDomainEvent::BackendSessionRecoveryCompleted {
                    recovery_id, ..
                } if recovery_id == request.recovery_id && started => {
                    result = Some((
                        RecoveryResultClassification::Succeeded,
                        "The backend session recovery completed.".to_string(),
                    ));
                }
                AgentSessionDomainEvent::BackendSessionRecoveryFailed { recovery_id, .. }
                    if recovery_id == request.recovery_id && started =>
                {
                    result = Some((
                        RecoveryResultClassification::Ambiguous,
                        "The backend session recovery requires reconciliation.".to_string(),
                    ));
                }
                _ => {}
            }
        }
        let (classification, safe_result) = result.ok_or_else(|| {
            Self::failure("The backend-recovery effect has no matching durable result stream.")
        })?;
        if classification == RecoveryResultClassification::Pending {
            let participants = self
                .session_store
                .prepare_backend_recovery_readback_completion(
                    &request.session_id,
                    &request.recovery_id,
                )
                .map_err(|_| {
                    Self::failure("The backend-recovery owner completion could not be prepared.")
                })?;
            if let Some(participants) = participants {
                return Ok(RecoveryEffectResult {
                    classification: RecoveryResultClassification::Succeeded,
                    safe_result:
                        "The durable provider identity completed the backend session recovery."
                            .to_string(),
                    owner_mutations: participants.mutations,
                    owner_batch: Some(RecoveryOwnerBatch {
                        expected_heads: participants.expected_heads,
                        events: participants.events,
                        canonical_events: participants.canonical_events,
                        participant_digest: participants.participant_digest,
                    }),
                });
            }
        }
        Ok(RecoveryEffectResult {
            classification,
            safe_result,
            owner_mutations: Vec::new(),
            owner_batch: None,
        })
    }
}

#[cfg(test)]
mod lifecycle_gate_tests {
    use std::sync::Arc;

    use super::{RuntimeAgentSessionOperationGate, RuntimeSendOperationGate};
    use crate::domain::local_event::{
        LocalEventQuery, LocalEventQueryResult, LocalEventTransactionRepository as _,
    };
    use crate::usecase::agent_session::operation::{
        AgentSendOperationUsecase, OperationBindingAuthority, SessionLifecycleAction,
        SessionLifecycleCommandResult, SessionLifecycleGate as _, SessionLifecycleOperationState,
        SessionLifecycleOperationUsecase, SessionLifecycleRequest, SessionLifecycleState,
        StopOperationUsecase,
    };
    use crate::usecase::agent_session::session::SessionState;
    use crate::usecase::agent_session::status::TurnPhase;

    async fn assert_completed_history_idle_action(
        label: &str,
        action: SessionLifecycleAction,
        expected_state: SessionState,
        expected_backend: &str,
    ) {
        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            store.clone();
        session_store.set_local_event_repository_with_projection_codec(
            repository.clone(),
            store.generation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );

        let session_id = format!("f02-completed-history-{label}");
        let mut session = crate::usecase::agent_session::session::build_new_session_with_id(
            session_id.clone(),
            data.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            crate::domain::agent_session::PermissionMode::Ask,
            Some("claude-4-sonnet".to_string()),
            false,
            false,
            None,
        );
        session.state = SessionState::Idle;
        session_store
            .save_full_session_for_migration_or_restore(data.path(), &session)
            .unwrap();
        session_store
            .append_session_event_and_project_state(
                data.path(),
                &session_id,
                crate::domain::agent_session::events::AgentSessionDomainEvent::TurnStarted {
                    turn_id: 1,
                    message_id: format!("{label}-human"),
                    assistant_message_id: Some(format!("{label}-assistant")),
                    prompt: crate::domain::agent_session::events::PromptInput::default(),
                    at: 1.0,
                },
            )
            .unwrap();
        session_store
            .append_session_event_and_project_state(
                data.path(),
                &session_id,
                crate::domain::agent_session::events::AgentSessionDomainEvent::TurnCompleted {
                    turn_id: 1,
                    exit_code: 0,
                    stop_reason: None,
                    token_usage: None,
                },
            )
            .unwrap();
        let before = session_store
            .get_session_meta(data.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(before.state, SessionState::Done);
        assert_eq!(before.last_turn_id, Some(1));
        assert!(session_store
            .load_session_events(data.path(), &session_id)
            .unwrap()
            .iter()
            .any(|event| matches!(
                event,
                crate::domain::agent_session::events::AgentSessionDomainEvent::TurnCompleted {
                    turn_id: 1,
                    ..
                }
            )));

        let (runtime, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                data.path(),
            );
        runtime
            .start_session(
                &session_id,
                crate::usecase::agent_session::runtime::usecase::StartSessionOptions {
                    permission_mode: crate::domain::agent_session::PermissionMode::Ask,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(runtime.turn_phase(&session_id).await, Some(TurnPhase::Idle));
        assert!(runtime.has_live_runtime(&session_id).await);
        assert_eq!(
            runtime
                .get_session(&session_id)
                .await
                .unwrap()
                .expect("completed-history runtime snapshot")
                .active_turn_id,
            None,
            "the public runtime snapshot must not expose historical last_turn_id as active"
        );

        let operation_gate = Arc::new(RuntimeAgentSessionOperationGate::new(
            runtime.clone(),
            session_store.clone(),
            data.path().to_path_buf(),
        ));
        let lifecycle_snapshot = operation_gate.session_snapshot(&session_id).await.unwrap();
        let authority: Arc<dyn OperationBindingAuthority> = store.clone();
        let lifecycle_gate: Arc<
            dyn crate::usecase::agent_session::operation::SessionLifecycleGate,
        > = operation_gate.clone();
        let lifecycle = SessionLifecycleOperationUsecase::new(
            repository.clone(),
            authority.clone(),
            lifecycle_gate,
            store.generation_id().to_string(),
        );
        let stop_gate: Arc<dyn crate::usecase::agent_session::operation::StopAdmissionGate> =
            operation_gate.clone();
        let stop = Arc::new(StopOperationUsecase::new(
            repository,
            authority,
            stop_gate,
            store.generation_id().to_string(),
        ));
        operation_gate.bind_stop_operation(Arc::downgrade(&stop));
        let send_gate = Arc::new(RuntimeSendOperationGate::new(
            runtime,
            session_store.clone(),
            data.path().to_path_buf(),
        ));
        let send = Arc::new(AgentSendOperationUsecase::new(
            store.clone(),
            store.clone(),
            send_gate.clone(),
            store.generation_id().to_string(),
        ));
        operation_gate.bind_send_operation(Arc::downgrade(&send));
        send_gate.bind_status_sink(Arc::downgrade(&send));

        let result = lifecycle
            .request(SessionLifecycleRequest {
                principal: "local-app".to_string(),
                request_id: format!("f02-{label}"),
                session_id: session_id.clone(),
                expected_session_revision: i64::try_from(before.state_revision).unwrap(),
                action,
            })
            .await
            .unwrap();
        assert!(
            matches!(
                result,
                SessionLifecycleCommandResult::Accepted {
                    state: SessionLifecycleOperationState::Completed,
                    ..
                }
            ),
            "historically completed idle {label} action was not accepted: {result:?}"
        );
        assert_eq!(
            lifecycle_snapshot.lifecycle,
            SessionLifecycleState::Open {
                idle: true,
                active_turn_id: None,
            },
            "historical last_turn_id must not become the active runtime turn"
        );

        let after = session_store
            .get_session_meta(data.path(), &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(after.state, expected_state);
        assert_eq!(after.backend_id, expected_backend);
        let events = session_store
            .load_session_events(data.path(), &session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    crate::domain::agent_session::events::AgentSessionDomainEvent::TurnInterrupted {
                        ..
                    }
                ))
                .count(),
            0,
            "idle lifecycle action must not synthesize a terminal event"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    crate::domain::agent_session::events::AgentSessionDomainEvent::QueuePaused {
                        ..
                    }
                ))
                .count(),
            1
        );
        assert!(matches!(
            store
                .query(LocalEventQuery::TerminalByTurn {
                    session_id: session_id.clone(),
                    turn_id: "1".to_string(),
                })
                .await
                .unwrap(),
            LocalEventQueryResult::TerminalByTurn(None)
        ));
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .into_iter()
                .filter(|kind| *kind == crate::test_support::TestRuntimeCallKind::Close)
                .count(),
            1,
            "the live idle runtime must be closed exactly once"
        );
    }

    #[tokio::test]
    async fn f02_runtime_idle_with_completed_history_close_has_no_synthetic_terminal() {
        assert_completed_history_idle_action(
            "close",
            SessionLifecycleAction::Close,
            SessionState::Closed,
            "claude",
        )
        .await;
    }

    #[tokio::test]
    async fn f02_runtime_idle_with_completed_history_archive_open_has_no_synthetic_terminal() {
        assert_completed_history_idle_action(
            "archive-open",
            SessionLifecycleAction::ArchiveOpen,
            SessionState::Archived,
            "claude",
        )
        .await;
    }

    #[tokio::test]
    async fn f02_runtime_idle_with_completed_history_switch_backend_is_accepted() {
        assert_completed_history_idle_action(
            "switch-backend",
            SessionLifecycleAction::SwitchBackend {
                backend_id: "codex".to_string(),
            },
            SessionState::Idle,
            "codex",
        )
        .await;
    }
}

#[async_trait::async_trait]
impl StopAdmissionGate for RuntimeAgentSessionOperationGate {
    async fn target_snapshot(
        &self,
        session_id: &str,
    ) -> Result<StopTargetSnapshot, SafeOperationFailure> {
        let meta = self
            .session_store
            .get_session_meta(&self.data_dir, session_id)
            .map_err(|_| Self::failure("The Stop target snapshot is unavailable."))?
            .ok_or_else(|| Self::failure("The Stop target does not exist."))?;
        let runtime = self
            .runtime
            .get_session(session_id)
            .await
            .map_err(|_| Self::failure("The Stop runtime snapshot is unavailable."))?
            .ok_or_else(|| Self::failure("The Stop runtime does not exist."))?;
        if runtime.turn_phase == crate::usecase::agent_session::status::TurnPhase::Idle {
            return Err(Self::failure("The Stop target has no active turn."));
        }
        let active_turn_id = runtime
            .active_turn_id
            .ok_or_else(|| Self::failure("The Stop target has no active turn."))?;
        Ok(StopTargetSnapshot {
            session_revision: meta.state_revision,
            active_turn_id: active_turn_id.to_string(),
            queue_paused: runtime.queue_paused,
        })
    }

    async fn acceptance_state_mutations(
        &self,
        session_id: &str,
        events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, SafeOperationFailure> {
        self.session_store
            .prepare_event_projection_mutations(session_id, events)
            .map_err(|_| Self::failure("The Stop acceptance projection could not be prepared."))
    }

    async fn terminal_state_mutations(
        &self,
        session_id: &str,
        events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, SafeOperationFailure> {
        if self
            .session_store
            .get_session_meta(&self.data_dir, session_id)
            .map_err(|_| Self::failure("The Stop terminal projection could not be read."))?
            .is_none()
        {
            // Corrupt/legacy test fixtures may retain the durable Stop owner
            // rows without a session projection. The event remains part of
            // the atomic closure; production sessions always have this row.
            return Ok(Vec::new());
        }
        self.session_store
            .prepare_event_projection_mutations(session_id, events)
            .map_err(|_| Self::failure("The Stop terminal projection could not be prepared."))
    }

    async fn terminal_participants(
        &self,
        terminal: &crate::domain::local_event::TerminalRecordMutation,
    ) -> Result<crate::usecase::agent_session::operation::TerminalParticipants, SafeOperationFailure>
    {
        let send_operation = self
            .send_operation
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| Self::failure("The send terminal participant is unavailable."))?;
        send_operation
            .prepare_runtime_terminal_participants(terminal)
            .await
            .map_err(|_| Self::failure("The send terminal participant could not be prepared."))
    }

    async fn interrupt(
        &self,
        effect: &AcceptedStopEffect,
    ) -> Result<StopEffectObservation, SafeOperationFailure> {
        let before = self
            .runtime
            .get_session(&effect.session_id)
            .await
            .map_err(|_| Self::failure("The Stop target could not be revalidated."))?;
        if before.as_ref().is_none_or(|session| {
            session.turn_phase == crate::usecase::agent_session::status::TurnPhase::Idle
                || session
                    .active_turn_id
                    .is_none_or(|turn_id| turn_id.to_string() != effect.turn_id)
        }) {
            return Ok(StopEffectObservation {
                terminal_reason: None,
            });
        }
        self.runtime
            .interrupt_provider_effect(&effect.session_id)
            .await
            .map_err(|_| Self::failure("The provider interrupt could not be confirmed."))?;
        Ok(StopEffectObservation {
            terminal_reason: None,
        })
    }

    async fn timeout_terminal_committed(&self, effect: &AcceptedStopEffect) {
        let Ok(turn_id) = effect.turn_id.parse::<u64>() else {
            log::warn!(
                "Stop Timeout committed with an invalid runtime turn identity: {}",
                effect.turn_id
            );
            return;
        };
        self.runtime
            .seal_stop_timeout_terminal(&effect.session_id, turn_id)
            .await;
    }
}

pub(crate) struct RuntimePermissionResponseOperationGate {
    runtime: Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
    session_store: Arc<SessionStore>,
}

impl RuntimePermissionResponseOperationGate {
    pub(crate) fn new(
        runtime: Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
        session_store: Arc<SessionStore>,
    ) -> Self {
        Self {
            runtime,
            session_store,
        }
    }

    fn admission_failure() -> SafeOperationFailure {
        SafeOperationFailure::new(
            SessionOperationFailureKind::InvalidEffectIntent,
            true,
            "The permission request is no longer available for this exact response.",
            uuid::Uuid::new_v4().to_string(),
        )
    }

    fn provider_failure() -> SafeOperationFailure {
        SafeOperationFailure::new(
            SessionOperationFailureKind::OutcomeUnknown,
            false,
            "The provider permission response could not be confirmed.",
            uuid::Uuid::new_v4().to_string(),
        )
        .with_detail("Use the saved operation identity to reconcile this response.")
    }
}

#[async_trait::async_trait]
impl PermissionResponseGate for RuntimePermissionResponseOperationGate {
    async fn plan_response(
        &self,
        session_id: &str,
        response: &crate::domain::agent_session::entities::PermissionResponse,
    ) -> Result<PermissionResponsePlan, SafeOperationFailure> {
        let (turn_id, from_runtime_state) = self
            .runtime
            .prepare_permission_response_operation(session_id, response)
            .await
            .map_err(|_| Self::admission_failure())?;
        Ok(PermissionResponsePlan {
            session_id: session_id.to_string(),
            request_id: response.request_id.clone(),
            turn_id,
            response: response.clone(),
            from_runtime_state,
        })
    }

    async fn completion_state_mutations(
        &self,
        effect: &AcceptedPermissionResponseEffect,
        events: &[crate::domain::agent_session::events::AgentSessionDomainEvent],
    ) -> Result<Vec<crate::domain::local_event::LocalStateMutation>, SafeOperationFailure> {
        self.session_store
            .prepare_event_projection_mutations(&effect.plan.session_id, events)
            .map_err(|_| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::PersistFailure,
                    true,
                    "The permission response projection could not be prepared.",
                    uuid::Uuid::new_v4().to_string(),
                )
            })
    }

    async fn execute(
        &self,
        effect: &AcceptedPermissionResponseEffect,
    ) -> Result<(), SafeOperationFailure> {
        self.runtime
            .execute_accepted_permission_response_effect(
                &effect.plan.session_id,
                effect.plan.turn_id,
                effect.plan.response.clone(),
            )
            .await
            .map_err(|_| Self::provider_failure())
    }

    async fn after_completion(&self, effect: &AcceptedPermissionResponseEffect) {
        self.runtime
            .apply_permission_response_completion(
                &effect.plan.session_id,
                &effect.plan.response,
                effect.plan.from_runtime_state,
            )
            .await;
    }
}

/// Production recovery stays conservative unless an effect-specific adapter
/// can prove readback or safe cancellation. It never turns an opaque pending
/// record into a blind provider retry.
pub(crate) struct ConservativeRecoveryExecutor {
    stop_readback: Arc<dyn StopRecoveryReadbackPort>,
    session_close_readback: Arc<dyn SessionCloseRecoveryReadbackPort>,
    backend_recovery_readback: Arc<dyn BackendRecoveryReadbackPort>,
    send_readback: Arc<dyn SendRecoveryReadbackPort>,
    permission_response: Arc<PermissionResponseOperationUsecase>,
    repository: Arc<dyn LocalEventTransactionRepository>,
}

impl ConservativeRecoveryExecutor {
    pub(crate) fn new(
        stop_operation: Arc<StopOperationUsecase>,
        session_lifecycle_operation: Arc<SessionLifecycleOperationUsecase>,
        backend_recovery_readback: Arc<dyn BackendRecoveryReadbackPort>,
        send_operation: Arc<AgentSendOperationUsecase>,
        permission_response: Arc<PermissionResponseOperationUsecase>,
        repository: Arc<dyn LocalEventTransactionRepository>,
    ) -> Self {
        Self {
            stop_readback: stop_operation,
            session_close_readback: session_lifecycle_operation,
            backend_recovery_readback,
            send_readback: send_operation,
            permission_response,
            repository,
        }
    }

    #[cfg(test)]
    fn from_readback_ports(
        stop_readback: Arc<dyn StopRecoveryReadbackPort>,
        session_close_readback: Arc<dyn SessionCloseRecoveryReadbackPort>,
        backend_recovery_readback: Arc<dyn BackendRecoveryReadbackPort>,
        send_readback: Arc<dyn SendRecoveryReadbackPort>,
        permission_response: Arc<PermissionResponseOperationUsecase>,
        repository: Arc<dyn LocalEventTransactionRepository>,
    ) -> Self {
        Self {
            stop_readback,
            session_close_readback,
            backend_recovery_readback,
            send_readback,
            permission_response,
            repository,
        }
    }
}

enum ProductionRecoveryReadbackRequest {
    Stop(StopRecoveryReadbackRequest),
    SessionClose(SessionCloseRecoveryReadbackRequest),
    BackendRecovery(BackendRecoveryReadbackRequest),
    Send(SendRecoveryReadbackRequest),
}

fn stop_readback_obligation_id(session_id: &str, turn_id: &str) -> String {
    let digest =
        Sha256::digest(format!("stop-target-obligation/v1\0{session_id}\0{turn_id}").as_bytes());
    format!("stop-target-{}", hex::encode(digest))
}

fn session_close_readback_obligation_id(session_id: &str) -> String {
    let digest = Sha256::digest(format!("session-lifecycle-target/v1\0{session_id}").as_bytes());
    format!("session-lifecycle-target-{}", hex::encode(digest))
}

fn production_readback_request(
    obligation_id: &str,
    immutable_obligation: &ObligationRecord,
) -> Result<ProductionRecoveryReadbackRequest, ()> {
    let immutable_obligation = match immutable_obligation {
        ObligationRecord::RecoveryTransition { original, .. }
        | ObligationRecord::Observed { original, .. } => {
            return production_readback_request(obligation_id, original)
        }
        record => record,
    };
    match immutable_obligation {
        ObligationRecord::StopInterrupt {
            operation_id,
            session_id,
            turn_id,
            ..
        } => {
            if validate_operation_identity(operation_id).is_err()
                || obligation_id != stop_readback_obligation_id(session_id, turn_id)
            {
                return Err(());
            }
            Ok(ProductionRecoveryReadbackRequest::Stop(
                StopRecoveryReadbackRequest {
                    effect_identity: StableRecoveryEffectIdentity::parse(
                        obligation_id.to_string(),
                    )?,
                    operation_id: operation_id.clone(),
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                },
            ))
        }
        ObligationRecord::SessionClose {
            obligation_id: stored_obligation_id,
            operation_id,
            session_id,
            action: SessionLifecycleRecordAction::Close,
            ..
        } => {
            if validate_operation_identity(operation_id).is_err()
                || stored_obligation_id != obligation_id
                || obligation_id != session_close_readback_obligation_id(session_id)
            {
                return Err(());
            }
            Ok(ProductionRecoveryReadbackRequest::SessionClose(
                SessionCloseRecoveryReadbackRequest {
                    effect_identity: StableRecoveryEffectIdentity::parse(
                        obligation_id.to_string(),
                    )?,
                    operation_id: operation_id.clone(),
                    session_id: session_id.clone(),
                },
            ))
        }
        ObligationRecord::BackendSessionRecovery {
            session_id,
            recovery_id,
            ..
        } => {
            let expected_effect_identity = format!("backend-recovery:{session_id}:{recovery_id}");
            if obligation_id != expected_effect_identity {
                return Err(());
            }
            Ok(ProductionRecoveryReadbackRequest::BackendRecovery(
                BackendRecoveryReadbackRequest {
                    effect_identity: StableRecoveryEffectIdentity::parse(expected_effect_identity)?,
                    session_id: session_id.clone(),
                    recovery_id: recovery_id.clone(),
                },
            ))
        }
        ObligationRecord::Send {
            obligation_id: stored_obligation_id,
            operation_id,
            session_id,
            kind,
            ..
        } => {
            if validate_operation_identity(operation_id).is_err()
                || stored_obligation_id != obligation_id
            {
                return Err(());
            }
            let SendObligationKindRecord::TurnExecution = kind else {
                // Provider establishment has no operation-bound terminal
                // winner. A session-level provider identity could belong to a
                // later establishment, so advertising ReadAgain here would
                // turn mutable owner state into proof for the wrong effect.
                return Err(());
            };
            let kind = SendRecoveryReadbackKind::TurnExecution;
            let expected_effect_identity = format!("{operation_id}.exec");
            if obligation_id != expected_effect_identity {
                return Err(());
            }
            Ok(ProductionRecoveryReadbackRequest::Send(
                SendRecoveryReadbackRequest {
                    effect_identity: StableRecoveryEffectIdentity::parse(expected_effect_identity)?,
                    operation_id: operation_id.clone(),
                    session_id: session_id.clone(),
                    kind,
                },
            ))
        }
        ObligationRecord::TerminalCommit { .. }
        | ObligationRecord::PermissionResponse { .. }
        | ObligationRecord::SessionClose { .. }
        | ObligationRecord::WorkflowShutdown { .. }
        | ObligationRecord::WorkflowTurnCompletion { .. }
        | ObligationRecord::RecoveryPublication { .. }
        | ObligationRecord::LegacyReconciliation { .. }
        | ObligationRecord::ProviderEstablish { .. }
        | ObligationRecord::TurnExecution { .. }
        | ObligationRecord::RecoveryReserved { .. }
        | ObligationRecord::RecoveryCompleted { .. }
        | ObligationRecord::FeedbackReservation { .. }
        | ObligationRecord::Feedback { .. }
        | ObligationRecord::WorkflowExecution { .. }
        | ObligationRecord::RecoveryTransition { .. }
        | ObligationRecord::Observed { .. } => Err(()),
    }
}

fn original_recovery_obligation(record: &ObligationRecord) -> &ObligationRecord {
    match record {
        ObligationRecord::RecoveryTransition { original, .. }
        | ObligationRecord::Observed { original, .. } => original_recovery_obligation(original),
        record => record,
    }
}

fn recovery_handoff_target_matches(
    request: &RecoveryEffectRequest,
    current: &crate::domain::local_event::ObligationView,
) -> bool {
    current.revision.value() as u64 == request.origin_revision
        && current.record == request.immutable_obligation
        && current
            .pending
            .as_ref()
            .map(|pending| pending.owner.as_str())
            == request.expected_owner.as_deref()
}

#[async_trait::async_trait]
impl RecoveryEffectExecutor for ConservativeRecoveryExecutor {
    fn supports_read_again(
        &self,
        obligation_id: &str,
        immutable_obligation: &ObligationRecord,
    ) -> bool {
        production_readback_request(obligation_id, immutable_obligation).is_ok()
    }

    async fn validate_handoff(
        &self,
        request: &RecoveryEffectRequest,
    ) -> Result<crate::usecase::agent_session::operation::RecoveryEffectHandoff, SafeOperationFailure>
    {
        use crate::usecase::agent_session::operation::RecoveryEffectHandoff;

        let result = self
            .repository
            .query(LocalEventQuery::ObligationByIdentity {
                obligation_id: request.obligation_id.clone(),
            })
            .await
            .map_err(|_| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "The recovery target could not be revalidated.",
                    uuid::Uuid::new_v4().to_string(),
                )
            })?;
        let LocalEventQueryResult::ObligationByIdentity(current) = result else {
            return Err(SafeOperationFailure::new(
                SessionOperationFailureKind::Internal,
                false,
                "The recovery target returned an incompatible readback.",
                uuid::Uuid::new_v4().to_string(),
            ));
        };
        let Some(current) = current else {
            return Ok(RecoveryEffectHandoff::TargetRevisionChanged);
        };
        if !recovery_handoff_target_matches(request, &current) {
            return Ok(RecoveryEffectHandoff::TargetRevisionChanged);
        }
        Ok(RecoveryEffectHandoff::Ready)
    }

    async fn execute(
        &self,
        request: &RecoveryEffectRequest,
    ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
        use crate::domain::agent_session::events::{
            RecoveryActionKind as A, RecoveryResultClassification as C,
        };
        match request.action {
            A::ReadAgain => match production_readback_request(
                &request.obligation_id,
                &request.immutable_obligation,
            )
            .map_err(|_| {
                SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "This obligation does not have a supported typed readback.",
                    uuid::Uuid::new_v4().to_string(),
                )
            })? {
                ProductionRecoveryReadbackRequest::Stop(target) => {
                    self.stop_readback.read_stop(&target).await
                }
                ProductionRecoveryReadbackRequest::SessionClose(target) => {
                    self.session_close_readback
                        .read_session_close(&target)
                        .await
                }
                ProductionRecoveryReadbackRequest::BackendRecovery(target) => {
                    self.backend_recovery_readback
                        .read_backend_recovery(&target)
                        .await
                }
                ProductionRecoveryReadbackRequest::Send(target) => {
                    self.send_readback.read_send(&target).await
                }
            },
            A::UseObservedResult => match request.authoritative_observation.as_ref() {
                Some(observation) => Ok(RecoveryEffectResult {
                    classification: observation.classification,
                    safe_result: observation.safe_view.clone(),
                    owner_mutations: Vec::new(),
                    owner_batch: None,
                }),
                _ => Err(SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "A verified backend observation is required for this recovery action.",
                    uuid::Uuid::new_v4().to_string(),
                )),
            },
            A::KeepForManualResolution => Ok(RecoveryEffectResult {
                classification: C::Unchanged,
                safe_result: "The obligation was retained for manual resolution.".to_string(),
                owner_mutations: Vec::new(),
                owner_batch: None,
            }),
            A::RetrySameEffect => {
                let operation_id =
                    match original_recovery_obligation(&request.immutable_obligation) {
                        ObligationRecord::PermissionResponse { operation_id, .. } => {
                            Some(operation_id.clone())
                        }
                        _ => None,
                    }
                    .ok_or_else(|| {
                        SafeOperationFailure::new(
                            SessionOperationFailureKind::InvalidEffectIntent,
                            false,
                            "The saved permission operation identity is unavailable.",
                            uuid::Uuid::new_v4().to_string(),
                        )
                    })?;
                let operation = self
                    .permission_response
                    .resume_operation(&operation_id)
                    .await
                    .map_err(|_| {
                        SafeOperationFailure::new(
                            SessionOperationFailureKind::OutcomeUnknown,
                            false,
                            "The exact permission response could not be confirmed.",
                            uuid::Uuid::new_v4().to_string(),
                        )
                    })?;
                match operation.latest_status {
                    PermissionResponseExecutionStatus::Completed { .. } => {
                        Ok(RecoveryEffectResult {
                            classification: C::Succeeded,
                            safe_result: "The exact permission response was delivered.".to_string(),
                            owner_mutations: Vec::new(),
                            owner_batch: None,
                        })
                    }
                    PermissionResponseExecutionStatus::AwaitingProviderResponse { .. } => {
                        Ok(RecoveryEffectResult {
                            classification: C::Pending,
                            safe_result: "The saved permission response remains pending."
                                .to_string(),
                            owner_mutations: Vec::new(),
                            owner_batch: None,
                        })
                    }
                    PermissionResponseExecutionStatus::ReconciliationRequired { .. }
                    | PermissionResponseExecutionStatus::Failed { .. } => {
                        Ok(RecoveryEffectResult {
                            classification: C::Ambiguous,
                            safe_result:
                                "The provider result requires permission-response reconciliation."
                                    .to_string(),
                            owner_mutations: Vec::new(),
                            owner_batch: None,
                        })
                    }
                }
            }
            A::CancelIfSafe => match request.authoritative_observation.as_ref() {
                Some(observation)
                    if observation.classification == C::ConfirmedNoEffect
                        && observation.cancellable =>
                {
                    Ok(RecoveryEffectResult {
                        classification: C::CancelledBeforeEffect,
                        safe_result: "The effect was confirmed absent and safely cancelled."
                            .to_string(),
                        owner_mutations: Vec::new(),
                        owner_batch: None,
                    })
                }
                _ => Err(SafeOperationFailure::new(
                    SessionOperationFailureKind::InvalidEffectIntent,
                    false,
                    "A cancellable confirmed-no-effect proof is required.",
                    uuid::Uuid::new_v4().to_string(),
                )),
            },
        }
    }
}

#[cfg(test)]
fn permission_response_retry_material(
    record: &ObligationRecord,
) -> Result<
    (
        String,
        crate::domain::agent_session::entities::PermissionResponse,
    ),
    SafeOperationFailure,
> {
    let invalid = || {
        SafeOperationFailure::new(
            SessionOperationFailureKind::InvalidEffectIntent,
            false,
            "The stored exact permission response is unavailable.",
            uuid::Uuid::new_v4().to_string(),
        )
    };
    let record = original_recovery_obligation(record);
    let ObligationRecord::PermissionResponse {
        operation_id,
        effect_identity,
        session_id,
        response,
        owner_access: true,
        state: crate::domain::local_event::ObligationStateRecord::Pending,
        ..
    } = record
    else {
        return Err(invalid());
    };
    if session_id.is_empty()
        || response.request_id.is_empty()
        || effect_identity != &format!("permission-response:{operation_id}")
    {
        return Err(invalid());
    }
    Ok((session_id.clone(), response.clone()))
}

#[cfg(test)]
mod send_execution_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::{
        CanonicalSendCommandV1, CanonicalSendTargetV1, RuntimeSendOperationGate,
        LOCAL_INSTALLATION_OPERATION_PRINCIPAL,
    };
    use crate::adaptor::protocol::agent_session_v1::{
        SendCommandOutcomeDtoV1, SendExecutionStatusDtoV1,
    };
    use crate::domain::local_event::LocalEventTransactionRepository as _;
    use crate::test_support::TestRuntimeCallKind;
    use crate::usecase::agent_session::session::AgentSessionProjectionCodec as _;

    fn commit_b006_first_turn_terminal(
        session_store: &crate::usecase::agent_session::session::SessionStore,
        data_path: &std::path::Path,
        session_id: &str,
        active_turn_id: u64,
        active_assistant_message_id: &str,
    ) {
        let terminal_result = crate::domain::agent_session::entities::TurnResult::Completed {
            stop_reason: None,
            token_usage: None,
        };
        session_store
            .append_terminal_events_and_materialize(
                data_path,
                session_id,
                &[
                    crate::domain::agent_session::events::AgentSessionDomainEvent::FinalPartsRecorded {
                        turn_id: active_turn_id,
                        message_id: active_assistant_message_id.to_string(),
                        parts: Vec::new(),
                    },
                    crate::domain::agent_session::events::AgentSessionDomainEvent::TurnCompleted {
                        turn_id: active_turn_id,
                        exit_code: 0,
                        stop_reason: None,
                        token_usage: None,
                    },
                ],
                active_assistant_message_id,
                0,
                crate::usecase::agent_session::session::now_timestamp(),
                &terminal_result,
            )
            .expect("turn one terminal must commit before queue drain");
    }

    #[tokio::test]
    async fn b006_production_queued_send_replays_after_response_loss_and_restart_before_one_start()
    {
        run_b006_production_queued_restart(false).await;
    }

    #[tokio::test]
    async fn b006_recovery_into_already_idle_session_auto_drains_exact_queue_item_once() {
        run_b006_production_queued_restart(true).await;
    }

    async fn run_b006_production_queued_restart(terminal_before_recovery: bool) {
        let data = tempfile::tempdir().unwrap();
        let data_path = data.path().to_path_buf();
        let session_id = "b006-queued-restart-session".to_string();
        let operation_id = "b006-queued-restart-operation".to_string();

        // Establish a real active first turn in the original process. The
        // queued acceptance below must reserve the next identity without
        // handing that queue item to the provider.
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data_path.clone(),
            ),
        )
        .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            store.clone();
        session_store.set_local_event_repository_with_projection_codec(
            repository.clone(),
            store.generation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let (runtime, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                &data_path,
            );
        let mut session = crate::usecase::agent_session::session::build_new_session_with_id(
            session_id.clone(),
            data_path.to_string_lossy().as_ref(),
            Some("claude".to_string()),
            crate::domain::agent_session::PermissionMode::Ask,
            Some("claude-4-sonnet".to_string()),
            false,
            false,
            None,
        );
        session.state = crate::usecase::agent_session::session::SessionState::Idle;
        session_store
            .save_full_session_for_migration_or_restore(&data_path, &session)
            .unwrap();
        runtime
            .start_session(
                &session_id,
                crate::usecase::agent_session::runtime::usecase::StartSessionOptions {
                    permission_mode: crate::domain::agent_session::PermissionMode::Ask,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        controller
            .emit(
                &session_id,
                crate::domain::agent_session::gateway::AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "b006-provider-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !runtime.provider_session_is_confirmed(&session_id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the first provider session must become durably confirmed");
        let active = runtime
            .send_message(
                crate::usecase::agent_session::runtime::usecase::SendAgentMessageRequest {
                    chat_session_id: Some(session_id.clone()),
                    worktree_path: data_path.to_string_lossy().to_string(),
                    content: "turn one remains active".to_string(),
                    permission_mode: crate::domain::agent_session::PermissionMode::Ask,
                    plan_mode: false,
                    backend_id: Some("claude".to_string()),
                    model_id: Some("claude-4-sonnet".to_string()),
                    images: None,
                    mentions: None,
                    editor_context: None,
                },
            )
            .await
            .unwrap();
        let active_assistant_message_id = active
            .agent_message
            .expect("the active turn must have a durable assistant projection")
            .id;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let starts = controller
                    .call_kinds_for(&session_id)
                    .into_iter()
                    .filter(|kind| *kind == TestRuntimeCallKind::StartTurn)
                    .count();
                if starts == 1
                    && runtime.turn_phase(&session_id).await
                        == Some(crate::usecase::agent_session::status::TurnPhase::Streaming)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("turn one must remain active before accepting the queued send");
        let active_turn_id = session_store
            .get_session_meta(&data_path, &session_id)
            .unwrap()
            .and_then(|meta| meta.last_turn_id)
            .expect("turn one must have a durable identity");

        let gate = Arc::new(RuntimeSendOperationGate::new(
            runtime.clone(),
            session_store.clone(),
            data_path.clone(),
        ));
        let send = Arc::new(
            crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
                repository.clone(),
                store.clone(),
                gate.clone(),
                store.generation_id().to_string(),
            ),
        );
        gate.bind_status_sink(Arc::downgrade(&send));
        let command = CanonicalSendCommandV1 {
            target: CanonicalSendTargetV1::Direct {
                chat_session_id: Some(session_id.clone()),
                worktree_path: data_path.to_string_lossy().to_string(),
            },
            content: "queued exactly once".to_string(),
            permission_mode: "ask".to_string(),
            plan_mode: false,
            backend_id: Some("claude".to_string()),
            model_id: Some("claude-4-sonnet".to_string()),
            images: Vec::new(),
            mentions: Vec::new(),
            editor_context: None,
        };
        let request = crate::usecase::agent_session::operation::SendOperationRequest {
            principal: LOCAL_INSTALLATION_OPERATION_PRINCIPAL.to_string(),
            operation_id: operation_id.clone(),
            canonical_payload: serde_json::to_string(&command).unwrap(),
        };

        store.fault_injector().arm_drop_reply();
        assert_eq!(
            send.send(request.clone()).await.unwrap(),
            crate::usecase::agent_session::operation::SendCommandOutcome::OutcomeUnknown {
                operation_id: operation_id.clone(),
            },
            "the caller must lose only the reply after the queued acceptance commit"
        );
        let accepted_before_restart = send
            .get_operation(LOCAL_INSTALLATION_OPERATION_PRINCIPAL, &operation_id)
            .await
            .expect("the dropped reply must leave one durable acceptance");
        let crate::domain::agent_session::events::SendDisposition::Queued { queue_item_id } =
            &accepted_before_restart.receipt.disposition
        else {
            panic!("an active first turn must fix the disposition as Queued")
        };
        let expected_queue_item_id = queue_item_id.clone();
        assert!(matches!(
            &accepted_before_restart.latest_status,
            crate::usecase::agent_session::operation::SendExecutionStatus::AwaitingProviderStart {
                dependency_obligation_ids
            } if dependency_obligation_ids.is_empty()
        ));
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .into_iter()
                .filter(|kind| *kind == TestRuntimeCallKind::StartTurn)
                .count(),
            1,
            "reply loss must not start the queued provider turn"
        );

        let before_restart_session = session_store
            .load_full_session_for_restore(&data_path, &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            before_restart_session
                .messages
                .iter()
                .filter(|message| {
                    message.role == crate::usecase::agent_session::session::MessageRole::Human
                        && message.content == "queued exactly once"
                })
                .count(),
            1,
            "queued acceptance must materialize one human projection"
        );
        let before_restart_projection = match store
            .query(
                crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                    session_id: session_id.clone(),
                },
            )
            .await
            .unwrap()
        {
            crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(
                Some(projection),
            ) => projection,
            other => panic!("queued session projection missing: {other:?}"),
        };
        let codec =
            crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1;
        let before_restart_projection =
            codec.decode(&before_restart_projection.projection).unwrap();
        assert_eq!(before_restart_projection.pending_send_queue.len(), 1);
        assert_eq!(
            before_restart_projection.pending_send_queue[0].queue_item_id,
            expected_queue_item_id
        );
        let expected_human_message_id = before_restart_projection.pending_send_queue[0]
            .human_message_id
            .clone();
        let expected_reserved_turn_id = before_restart_projection.pending_send_queue[0]
            .reserved_turn_id
            .parse::<u64>()
            .expect("the queued acceptance must reserve a numeric turn identity");
        let expected_assistant_message_id = format!("{expected_human_message_id}:agent");
        assert_eq!(
            before_restart_projection.meta.state,
            crate::usecase::agent_session::session::SessionState::Active,
            "turn one must still be durably active at the response-loss boundary"
        );

        // End the first process completely, including its provider event
        // stream, before reopening the SQLite authority and the full runtime
        // composition.
        controller.close_event_streams_for_test(&session_id);
        drop(send);
        drop(gate);
        drop(runtime);
        drop(controller);
        drop(session_store);
        drop(repository);
        drop(store);
        tokio::task::yield_now().await;

        let reopened = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data_path.clone(),
            ),
        )
        .unwrap();
        let restarted_session_store = Arc::new(crate::test_support::build_session_store());
        let restarted_repository: Arc<
            dyn crate::domain::local_event::LocalEventTransactionRepository,
        > = reopened.clone();
        restarted_session_store.set_local_event_repository_with_projection_codec(
            restarted_repository.clone(),
            reopened.generation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        if terminal_before_recovery {
            commit_b006_first_turn_terminal(
                restarted_session_store.as_ref(),
                &data_path,
                &session_id,
                active_turn_id,
                &active_assistant_message_id,
            );
            assert_eq!(
                restarted_session_store
                    .get_session_meta(&data_path, &session_id)
                    .unwrap()
                    .unwrap()
                    .state,
                crate::usecase::agent_session::session::SessionState::Done,
                "the predecessor must be durably terminal before queue rehydration"
            );
        }
        let (restarted_runtime, restarted_controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                restarted_session_store.clone(),
                &data_path,
            );
        let restarted_gate = Arc::new(RuntimeSendOperationGate::new(
            restarted_runtime.clone(),
            restarted_session_store.clone(),
            data_path.clone(),
        ));
        let restarted_send = Arc::new(
            crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
                restarted_repository,
                reopened.clone(),
                restarted_gate.clone(),
                reopened.generation_id().to_string(),
            ),
        );
        restarted_gate.bind_status_sink(Arc::downgrade(&restarted_send));

        let replay = restarted_send
            .send(request.clone())
            .await
            .expect("the exact operation and payload must replay after restart");
        let crate::usecase::agent_session::operation::SendCommandOutcome::Accepted(replay) = replay
        else {
            panic!("restart replay must resolve the dropped acceptance")
        };
        assert_eq!(replay.receipt, accepted_before_restart.receipt);
        assert_eq!(replay.latest_status, accepted_before_restart.latest_status);
        assert_eq!(
            restarted_controller
                .call_kinds_for(&session_id)
                .into_iter()
                .filter(|kind| *kind == TestRuntimeCallKind::StartTurn)
                .count(),
            0,
            "receipt replay must not start the queued turn"
        );

        restarted_runtime
            .start_session(
                &session_id,
                crate::usecase::agent_session::runtime::usecase::StartSessionOptions {
                    permission_mode: crate::domain::agent_session::PermissionMode::Ask,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        restarted_controller
            .emit(
                &session_id,
                crate::domain::agent_session::gateway::AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "b006-provider-session".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::Resumed,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !restarted_runtime
                .provider_session_is_confirmed(&session_id)
                .await
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the restarted provider session must become confirmed");
        if terminal_before_recovery {
            assert_eq!(
                restarted_runtime.turn_phase(&session_id).await,
                Some(crate::usecase::agent_session::status::TurnPhase::Idle),
                "the recovered queue must arrive after the predecessor is already idle"
            );
        }

        restarted_send
            .recover_pending_provider_effects()
            .await
            .expect("startup recovery must restore the accepted queue item");
        if !terminal_before_recovery {
            let queued_after_restart = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let operation = restarted_send
                        .get_operation(LOCAL_INSTALLATION_OPERATION_PRINCIPAL, &operation_id)
                        .await
                        .unwrap();
                    let session = restarted_runtime
                        .get_session(&session_id)
                        .await
                        .unwrap()
                        .unwrap();
                    if matches!(
                        &operation.latest_status,
                        crate::usecase::agent_session::operation::SendExecutionStatus::Queued {
                            queue_item_id,
                            ..
                        } if queue_item_id == &expected_queue_item_id
                    ) && session.pending_queue_count == 1
                    {
                        break operation;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("startup recovery must converge on one in-memory queue item");
            assert_eq!(
                queued_after_restart.receipt,
                accepted_before_restart.receipt
            );
            assert_eq!(
                restarted_controller
                    .call_kinds_for(&session_id)
                    .into_iter()
                    .filter(|kind| *kind == TestRuntimeCallKind::StartTurn)
                    .count(),
                0,
                "startup recovery may restore the queue but cannot start it before turn one is terminal"
            );

            let after_restart_session = restarted_session_store
                .load_full_session_for_restore(&data_path, &session_id)
                .unwrap()
                .unwrap();
            assert_eq!(
                after_restart_session
                    .messages
                    .iter()
                    .filter(|message| {
                        message.role == crate::usecase::agent_session::session::MessageRole::Human
                            && message.content == "queued exactly once"
                    })
                    .count(),
                1,
                "restart recovery and replay must not duplicate the human projection"
            );
            let after_restart_projection = match reopened
                .query(
                    crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                        session_id: session_id.clone(),
                    },
                )
                .await
                .unwrap()
            {
                crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(
                    Some(projection),
                ) => projection,
                other => panic!("restarted queued session projection missing: {other:?}"),
            };
            let after_restart_projection =
                codec.decode(&after_restart_projection.projection).unwrap();
            assert_eq!(after_restart_projection.pending_send_queue.len(), 1);
            assert_eq!(
                after_restart_projection.pending_send_queue[0].queue_item_id,
                expected_queue_item_id
            );

            // Commit turn one's real terminal winner, then let the runtime drain
            // the already-restored queue. Exactly this boundary makes the second
            // provider turn executable.
            commit_b006_first_turn_terminal(
                restarted_session_store.as_ref(),
                &data_path,
                &session_id,
                active_turn_id,
                &active_assistant_message_id,
            );
            assert_eq!(
                restarted_session_store
                    .get_session_meta(&data_path, &session_id)
                    .unwrap()
                    .unwrap()
                    .state,
                crate::usecase::agent_session::session::SessionState::Done
            );
            restarted_runtime
                .drain_next_queued_turn_for_test(&session_id)
                .await;
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let starts = restarted_controller
                    .call_kinds_for(&session_id)
                    .into_iter()
                    .filter(|kind| *kind == TestRuntimeCallKind::StartTurn)
                    .count();
                let operation = restarted_send
                    .get_operation(LOCAL_INSTALLATION_OPERATION_PRINCIPAL, &operation_id)
                    .await
                    .unwrap();
                if starts == 1
                    && matches!(
                        operation.latest_status,
                        crate::usecase::agent_session::operation::SendExecutionStatus::Running { .. }
                    )
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the executable queue item must start exactly once");

        let drained_projection = match reopened
            .query(
                crate::domain::local_event::LocalEventQuery::SessionProjectionByIdentity {
                    session_id: session_id.clone(),
                },
            )
            .await
            .unwrap()
        {
            crate::domain::local_event::LocalEventQueryResult::SessionProjectionByIdentity(
                Some(projection),
            ) => codec.decode(&projection.projection).unwrap(),
            other => panic!("drained queued session projection missing: {other:?}"),
        };
        assert!(
            drained_projection.pending_send_queue.is_empty(),
            "TurnStarted must remove the exact canonical queue item in its own commit"
        );
        assert_eq!(
            drained_projection.meta.last_turn_id,
            Some(expected_reserved_turn_id),
            "queue drain must consume the turn identity reserved at acceptance"
        );
        let drained_session = restarted_session_store
            .load_full_session_for_restore(&data_path, &session_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            drained_session
                .messages
                .iter()
                .filter(|message| message.id == expected_human_message_id)
                .count(),
            1,
            "queue drain must reuse the accepted human projection"
        );
        assert_eq!(
            drained_session
                .messages
                .iter()
                .filter(|message| message.id == expected_assistant_message_id)
                .count(),
            1,
            "TurnStarted must atomically materialize one reserved assistant projection"
        );

        let replay_after_start = restarted_send
            .send(request)
            .await
            .expect("same-operation replay must remain available after queue start");
        let crate::usecase::agent_session::operation::SendCommandOutcome::Accepted(
            replay_after_start,
        ) = replay_after_start
        else {
            panic!("the started queued send must remain top-level Accepted")
        };
        assert_eq!(replay_after_start.receipt, accepted_before_restart.receipt);
        restarted_send
            .recover_pending_provider_effects()
            .await
            .expect("a running queued send remains a no-op for startup recovery");
        assert_eq!(
            restarted_controller
                .call_kinds_for(&session_id)
                .into_iter()
                .filter(|kind| *kind == TestRuntimeCallKind::StartTurn)
                .count(),
            1,
            "post-start replay and recovery must not repeat the provider effect"
        );
        assert_eq!(
            restarted_session_store
                .load_full_session_for_restore(&data_path, &session_id)
                .unwrap()
                .unwrap()
                .messages
                .iter()
                .filter(|message| {
                    message.role == crate::usecase::agent_session::session::MessageRole::Human
                        && message.content == "queued exactly once"
                })
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn b008_provider_start_failure_keeps_public_acceptance_and_immutable_receipt() {
        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            store.clone();
        session_store.set_local_event_repository_with_projection_codec(
            repository,
            store.generation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let (runtime, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                data.path(),
            );
        let session_id = "b008-provider-session".to_string();
        let mut session = crate::usecase::agent_session::session::build_new_session_with_id(
            session_id.clone(),
            data.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            crate::domain::agent_session::PermissionMode::Ask,
            Some("claude-4-sonnet".to_string()),
            false,
            false,
            None,
        );
        session.state = crate::usecase::agent_session::session::SessionState::Idle;
        session_store
            .save_full_session_for_migration_or_restore(data.path(), &session)
            .unwrap();
        runtime
            .start_session(
                &session_id,
                crate::usecase::agent_session::runtime::usecase::StartSessionOptions {
                    permission_mode: crate::domain::agent_session::PermissionMode::Ask,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        controller
            .emit(
                &session_id,
                crate::domain::agent_session::gateway::AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "b008-confirmed-provider".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !runtime.provider_session_is_confirmed(&session_id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("test provider session must become durably confirmed");
        let gate = Arc::new(RuntimeSendOperationGate::new(
            runtime,
            session_store,
            data.path().to_path_buf(),
        ));
        let send = Arc::new(
            crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
                store.clone(),
                store.clone(),
                gate.clone(),
                store.generation_id().to_string(),
            ),
        );
        gate.bind_status_sink(Arc::downgrade(&send));
        let journal = crate::usecase::agent_session::operation::CallerAttemptJournal::new(
            store.clone(),
            store.clone(),
            store.generation_id().to_string(),
        );
        let command = CanonicalSendCommandV1 {
            target: CanonicalSendTargetV1::Direct {
                chat_session_id: Some(session_id.clone()),
                worktree_path: data.path().to_string_lossy().to_string(),
            },
            content: "provider failure after acceptance".to_string(),
            permission_mode: "ask".to_string(),
            plan_mode: false,
            backend_id: Some("claude".to_string()),
            model_id: Some("claude-4-sonnet".to_string()),
            images: Vec::new(),
            mentions: Vec::new(),
            editor_context: None,
        };
        controller.fail_next_start_turn();

        let first =
            crate::adaptor::controller::command::agent_session::session::dispatch_durable_send(
                store.as_ref(),
                send.as_ref(),
                &journal,
                "b008-provider-failure".to_string(),
                command.clone(),
            )
            .await
            .expect("public send must return its committed acceptance");
        let SendCommandOutcomeDtoV1::Accepted { operation: first } = first else {
            panic!("provider execution must not rewrite top-level acceptance")
        };
        let original_receipt = serde_json::to_value(&first.receipt).unwrap();
        assert_eq!(first.receipt.session_id, session_id);

        let reconciled = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let operation = crate::adaptor::controller::command::agent_session::session::get_agent_send_operation_for_principal(
                    send.as_ref(),
                    LOCAL_INSTALLATION_OPERATION_PRINCIPAL,
                    "b008-provider-failure".to_string(),
                )
                .await
                .expect("accepted operation must remain publicly queryable");
                if matches!(
                    &operation.latest_status,
                    SendExecutionStatusDtoV1::ReconciliationRequired { failure }
                        if failure.kind == "external_effect_failed"
                ) {
                    break operation;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("provider failure must converge to reconciliation-required");
        assert_eq!(
            serde_json::to_value(&reconciled.receipt).unwrap(),
            original_receipt
        );
        let SendExecutionStatusDtoV1::ReconciliationRequired { failure } =
            &reconciled.latest_status
        else {
            panic!("provider failure changed to an incompatible public status")
        };
        assert_eq!(failure.kind, "external_effect_failed");

        let replay =
            crate::adaptor::controller::command::agent_session::session::dispatch_durable_send(
                store.as_ref(),
                send.as_ref(),
                &journal,
                "b008-provider-failure".to_string(),
                command,
            )
            .await
            .expect("same public identity must replay after provider failure");
        let SendCommandOutcomeDtoV1::Accepted { operation: replay } = replay else {
            panic!("provider failure must preserve top-level Accepted on replay")
        };
        assert_eq!(
            serde_json::to_value(&replay.receipt).unwrap(),
            original_receipt
        );
        assert!(matches!(
            replay.latest_status,
            SendExecutionStatusDtoV1::ReconciliationRequired { ref failure }
                if failure.kind == "external_effect_failed"
        ));
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .into_iter()
                .filter(|kind| *kind == TestRuntimeCallKind::StartTurn)
                .count(),
            1,
            "public replay must not execute the accepted provider effect twice"
        );
    }
}

#[cfg(test)]
mod workflow_send_execution_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::{
        bind_runtime_durable_workflow_send_driver, RuntimeSendOperationGate,
        INTERNAL_WORKFLOW_OPERATION_PRINCIPAL,
    };
    use crate::domain::local_event::{
        LocalEventQuery, LocalEventQueryResult, LocalEventTransactionRepository as _,
        ObligationRecord,
    };
    use crate::test_support::TestRuntimeCallKind;
    use crate::usecase::agent_session::runtime::{
        durable_workflow_turn_operation_id, DurableWorkflowTurnRequest,
    };

    #[tokio::test]
    async fn workflow_turn_commits_one_durable_send_before_provider_io_and_replay_is_effect_free() {
        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            store.clone();
        session_store.set_local_event_repository_with_projection_codec(
            repository,
            store.generation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let (runtime, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                Arc::clone(&session_store),
                data.path(),
            );
        let session_id = "workflow-durable-send-session".to_string();
        let mut session = crate::usecase::agent_session::session::build_new_session_with_id(
            session_id.clone(),
            data.path().to_string_lossy().as_ref(),
            Some("claude".to_string()),
            crate::domain::agent_session::PermissionMode::Ask,
            Some("claude-4-sonnet".to_string()),
            false,
            true,
            None,
        );
        session.state = crate::usecase::agent_session::session::SessionState::Idle;
        session_store
            .save_full_session_for_migration_or_restore(data.path(), &session)
            .unwrap();
        runtime
            .start_session(
                &session_id,
                crate::usecase::agent_session::runtime::usecase::StartSessionOptions {
                    permission_mode: crate::domain::agent_session::PermissionMode::Ask,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        controller
            .emit(
                &session_id,
                crate::domain::agent_session::gateway::AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: "workflow-confirmed-provider".to_string(),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !runtime.provider_session_is_confirmed(&session_id).await {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("workflow provider session must become confirmed");

        let gate = Arc::new(RuntimeSendOperationGate::new(
            Arc::clone(&runtime),
            Arc::clone(&session_store),
            data.path().to_path_buf(),
        ));
        let send = Arc::new(
            crate::usecase::agent_session::operation::AgentSendOperationUsecase::new(
                store.clone(),
                store.clone(),
                gate.clone(),
                store.generation_id().to_string(),
            ),
        );
        gate.bind_status_sink(Arc::downgrade(&send));
        bind_runtime_durable_workflow_send_driver(
            &runtime,
            Arc::clone(&send),
            Arc::clone(&session_store),
            data.path().to_path_buf(),
        );

        let operation_id = durable_workflow_turn_operation_id("workflow-node-execution", "initial");
        let request = DurableWorkflowTurnRequest {
            operation_id: operation_id.clone(),
            session_id: session_id.clone(),
            content: "run durable workflow turn".to_string(),
            permission_mode: crate::domain::agent_session::PermissionMode::Ask,
            base_system_prompt: Some("workflow base prompt".to_string()),
            workflow_instructions: vec!["workflow instruction".to_string()],
        };
        let session_guard = runtime
            .acquire_session_control_after_recovery(&session_id)
            .await;
        tokio::time::timeout(
            Duration::from_secs(2),
            runtime.start_workflow_turn_locked(request.clone()),
        )
        .await
        .expect("durable Send acceptance must not reacquire the workflow-owned session lock")
        .expect("workflow Send must commit");

        let accepted = send
            .get_operation(INTERNAL_WORKFLOW_OPERATION_PRINCIPAL, &operation_id)
            .await
            .expect("workflow Send operation must be durable before provider I/O");
        assert!(matches!(
            accepted.receipt.disposition,
            crate::domain::agent_session::events::SendDisposition::StartedTurn { .. }
        ));
        assert!(matches!(
            store
                .query(LocalEventQuery::ObligationByIdentity {
                    obligation_id: format!("{operation_id}.exec"),
                })
                .await
                .unwrap(),
            LocalEventQueryResult::ObligationByIdentity(Some(ref obligation))
                if matches!(
                    &obligation.record,
                    ObligationRecord::Send {
                        operation_id: stored_operation_id,
                        ..
                    } if stored_operation_id == &operation_id
                ) && obligation.pending.is_some()
        ));
        let (_, page, _) = session_store
            .get_session_with_latest_page(data.path(), &session_id, 16)
            .unwrap()
            .expect("accepted workflow session projection");
        assert_eq!(
            page.messages
                .iter()
                .filter(|message| {
                    message.role == crate::usecase::agent_session::session::MessageRole::Human
                        && message.content == "run durable workflow turn"
                })
                .count(),
            1
        );
        assert!(controller
            .call_kinds_for(&session_id)
            .iter()
            .all(|call| !matches!(call, TestRuntimeCallKind::StartTurn)));

        drop(session_guard);
        tokio::time::timeout(Duration::from_secs(2), async {
            while controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurn))
                .count()
                != 1
            {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the committed workflow Send must start exactly one provider turn");

        runtime
            .start_workflow_turn_locked(request)
            .await
            .expect("same workflow operation must replay its immutable acceptance");
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert_eq!(
            controller
                .call_kinds_for(&session_id)
                .iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurn))
                .count(),
            1,
            "same-operation replay must not start another provider turn"
        );
        assert!(controller
            .call_kinds_for(&session_id)
            .iter()
            .any(|call| matches!(
                call,
                TestRuntimeCallKind::StartTurnSystemPrompt {
                    system_prompt: Some(system_prompt)
                } if system_prompt.contains("workflow base prompt")
                    && system_prompt.contains("workflow instruction")
            )));
    }
}

#[cfg(test)]
mod stop_execution_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::{
        bind_runtime_terminal_operation_participant_provider, RuntimeAgentSessionOperationGate,
        RuntimeSendOperationGate, LOCAL_INSTALLATION_OPERATION_PRINCIPAL,
    };
    use crate::domain::agent_session::entities::{
        InterruptReason as DomainInterruptReason, MessagePart, TurnResult,
    };
    use crate::domain::agent_session::events::{AgentSessionDomainEvent, StopResolution};
    use crate::domain::agent_session::gateway::AgentRuntimeEvent;
    use crate::domain::local_event::{
        AgentTerminalKind, AgentTurnTerminalResultRecord, LocalEventQuery, LocalEventQueryResult,
        LocalEventTransactionRepository as _, StopResolutionKind, TerminalInterruptReasonRecord,
        TerminalResultRecord,
    };
    use crate::test_support::{TestAgentRuntimeController, TestRuntimeCallKind};
    use crate::usecase::agent_session::operation::{
        AgentSendOperationUsecase, StopCommandOutcome, StopOperationRequest, StopOperationState,
        StopOperationUsecase,
    };
    use crate::usecase::agent_session::runtime::usecase::{
        SendAgentMessageRequest, StartSessionOptions,
    };
    use crate::usecase::agent_session::session::{SessionState, SessionStore};
    use crate::usecase::agent_session::status::TurnPhase;

    struct StopFixture {
        data: tempfile::TempDir,
        store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        session_store: Arc<SessionStore>,
        runtime: Arc<crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase>,
        controller: TestAgentRuntimeController,
        stop: Arc<StopOperationUsecase>,
        session_id: String,
        turn_id: u64,
        session_revision: u64,
    }

    async fn wait_for_phase(
        runtime: &crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase,
        session_id: &str,
        phase: TurnPhase,
    ) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if runtime
                    .get_session(session_id)
                    .await
                    .unwrap()
                    .is_some_and(|session| session.turn_phase == phase)
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("runtime phase must converge");
    }

    async fn stop_fixture(backend_id: &str, label: &str) -> StopFixture {
        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository> =
            store.clone();
        session_store.set_local_event_repository_with_projection_codec(
			repository.clone(),
			store.generation_id().to_string(),
			Arc::new(
				crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
			),
		);
        let (runtime, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                data.path(),
            );
        let session_id = format!("f01-{backend_id}-{label}");
        let model = match backend_id {
            "codex" => "gpt-5.6-sol",
            "claude" => "claude-4-sonnet",
            other => panic!("unexpected backend {other}"),
        };
        let mut session = crate::usecase::agent_session::session::build_new_session_with_id(
            session_id.clone(),
            data.path().to_string_lossy().as_ref(),
            Some(backend_id.to_string()),
            crate::domain::agent_session::PermissionMode::Ask,
            Some(model.to_string()),
            false,
            false,
            None,
        );
        session.state = SessionState::Idle;
        session_store
            .save_full_session_for_migration_or_restore(data.path(), &session)
            .unwrap();
        runtime
            .start_session(
                &session_id,
                StartSessionOptions {
                    permission_mode: crate::domain::agent_session::PermissionMode::Ask,
                    plan_mode: false,
                },
            )
            .await
            .unwrap();
        controller
            .emit(
                &session_id,
                AgentRuntimeEvent::SessionEstablished {
                    backend_session_id: format!("{label}-provider-session"),
                    resume: crate::domain::agent_session::gateway::ResumeOutcome::NotRequested,
                },
            )
            .unwrap();
        runtime
            .send_message(SendAgentMessageRequest {
                chat_session_id: Some(session_id.clone()),
                worktree_path: data.path().to_string_lossy().to_string(),
                content: "finish only after the provider terminal".to_string(),
                permission_mode: crate::domain::agent_session::PermissionMode::Ask,
                plan_mode: false,
                backend_id: Some(backend_id.to_string()),
                model_id: Some(model.to_string()),
                images: None,
                mentions: None,
                editor_context: None,
            })
            .await
            .unwrap();
        wait_for_phase(runtime.as_ref(), &session_id, TurnPhase::Streaming).await;

        let operation_gate = Arc::new(RuntimeAgentSessionOperationGate::new(
            runtime.clone(),
            session_store.clone(),
            data.path().to_path_buf(),
        ));
        let send_gate = Arc::new(RuntimeSendOperationGate::new(
            runtime.clone(),
            session_store.clone(),
            data.path().to_path_buf(),
        ));
        let send = Arc::new(AgentSendOperationUsecase::new(
            repository.clone(),
            store.clone(),
            send_gate,
            store.generation_id().to_string(),
        ));
        operation_gate.bind_send_operation(Arc::downgrade(&send));
        let stop = Arc::new(StopOperationUsecase::new(
            repository,
            store.clone(),
            operation_gate.clone(),
            store.generation_id().to_string(),
        ));
        operation_gate.bind_stop_operation(Arc::downgrade(&stop));
        bind_runtime_terminal_operation_participant_provider(&session_store, stop.clone(), send);
        let meta = session_store
            .get_session_meta(data.path(), &session_id)
            .unwrap()
            .unwrap();
        StopFixture {
            data,
            store,
            session_store,
            runtime,
            controller,
            stop,
            session_id,
            turn_id: meta.last_turn_id.unwrap(),
            session_revision: meta.state_revision,
        }
    }

    async fn wait_for_interrupt_handoff(fixture: &StopFixture) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if fixture
                    .controller
                    .call_kinds_for(&fixture.session_id)
                    .into_iter()
                    .filter(|kind| *kind == TestRuntimeCallKind::Interrupt)
                    .count()
                    == 1
                {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("interrupt transport write must happen once");
    }

    fn assert_final_part(
        session_store: &SessionStore,
        data_dir: &std::path::Path,
        session_id: &str,
        expected: &str,
    ) {
        let session = session_store
            .load_full_session_for_restore(data_dir, session_id)
            .unwrap()
            .unwrap();
        let matching = session
            .messages
            .iter()
            .flat_map(|message| message.parts.iter().flatten())
            .filter(|part| matches!(part, MessagePart::Text { content, .. } if content == expected))
            .count();
        assert_eq!(matching, 1, "final parts must be materialized exactly once");
    }

    #[derive(Clone, Copy)]
    enum ExpectedRuntimeTerminal {
        Completed,
        Interrupted(DomainInterruptReason),
    }

    async fn assert_runtime_winner(
        fixture: &StopFixture,
        outcome: StopCommandOutcome,
        expected: ExpectedRuntimeTerminal,
        final_part: &str,
    ) {
        let StopCommandOutcome::Accepted { receipt, state } = outcome else {
            panic!("Stop must remain accepted")
        };
        assert_eq!(
            state,
            StopOperationState::Completed {
                resolution: StopResolution::Superseded,
            }
        );
        let terminal = match fixture
            .store
            .query(LocalEventQuery::TerminalByTurn {
                session_id: fixture.session_id.clone(),
                turn_id: fixture.turn_id.to_string(),
            })
            .await
            .unwrap()
        {
            LocalEventQueryResult::TerminalByTurn(Some(terminal)) => terminal,
            other => panic!("runtime terminal missing: {other:?}"),
        };
        let TerminalResultRecord::AgentTurn { kind, result, .. } = &terminal.result else {
            panic!("runtime terminal has the wrong closed record kind: {terminal:?}")
        };
        let expected_kind = match expected {
            ExpectedRuntimeTerminal::Completed => AgentTerminalKind::Completed,
            ExpectedRuntimeTerminal::Interrupted(DomainInterruptReason::Abort) => {
                AgentTerminalKind::Abort
            }
            ExpectedRuntimeTerminal::Interrupted(DomainInterruptReason::Timeout) => {
                AgentTerminalKind::Timeout
            }
            ExpectedRuntimeTerminal::Interrupted(DomainInterruptReason::Crash) => {
                AgentTerminalKind::Crash
            }
            ExpectedRuntimeTerminal::Interrupted(DomainInterruptReason::SessionClosed) => {
                AgentTerminalKind::SessionClosed
            }
        };
        assert_eq!(*kind, expected_kind);
        match (expected, result) {
            (
                ExpectedRuntimeTerminal::Completed,
                AgentTurnTerminalResultRecord::Current(TurnResult::Completed { .. }),
            ) => {}
            (
                ExpectedRuntimeTerminal::Interrupted(expected_reason),
                AgentTurnTerminalResultRecord::Current(TurnResult::Interrupted { reason, .. }),
            ) => assert_eq!(*reason, expected_reason),
            (_, actual) => panic!("unexpected runtime terminal result: {actual:?}"),
        }
        assert!(matches!(
            fixture
                .store
                .query(LocalEventQuery::StopResolutionByOperation {
                    stop_operation_id: receipt.operation_id.clone(),
                })
                .await
                .unwrap(),
            LocalEventQueryResult::StopResolutionByOperation(Some(resolution))
                if resolution.resolution == StopResolutionKind::Superseded
        ));
        assert!(matches!(
            fixture
                .store
                .query(LocalEventQuery::PendingRecoveryPage {
                    limit: 16,
                    partition: None,
                    owner: Some(fixture.session_id.clone()),
                    ordered_key_prefix: None,
                    shutdown_plan: None,
                    cursor: None,
                })
                .await
                .unwrap(),
            LocalEventQueryResult::PendingRecoveryPage(page) if page.entries.is_empty()
        ));
        let events = fixture
            .session_store
            .load_session_events(fixture.data.path(), &fixture.session_id)
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    AgentSessionDomainEvent::TurnCompleted { .. }
                        | AgentSessionDomainEvent::TurnInterrupted { .. }
                ))
                .count(),
            1,
            "exactly one terminal event must win"
        );
        assert_final_part(
            fixture.session_store.as_ref(),
            fixture.data.path(),
            &fixture.session_id,
            final_part,
        );

        // Re-open the canonical SQLite authority through the real read-only
        // adapter while the writer remains live; projections must agree.
        let reader =
            crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore::open(
                fixture.data.path(),
            )
            .unwrap();
        let reloaded_terminal = reader
            .query(LocalEventQuery::TerminalByTurn {
                session_id: fixture.session_id.clone(),
                turn_id: fixture.turn_id.to_string(),
            })
            .await
            .unwrap();
        assert_eq!(
            reloaded_terminal,
            LocalEventQueryResult::TerminalByTurn(Some(terminal))
        );
        let reloaded_session_store = Arc::new(crate::test_support::build_session_store());
        let reader_repository: Arc<
            dyn crate::domain::local_event::LocalEventTransactionRepository,
        > = reader.clone();
        reloaded_session_store.set_local_event_repository_with_projection_codec(
			reader_repository,
			reader.generation_id().to_string(),
			Arc::new(
				crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
			),
		);
        assert_final_part(
            reloaded_session_store.as_ref(),
            fixture.data.path(),
            &fixture.session_id,
            final_part,
        );
    }

    #[tokio::test]
    async fn f01_transport_handoff_does_not_complete_stop_before_runtime_terminal() {
        for backend_id in ["codex", "claude"] {
            for (label, terminal_event, expected) in [
                (
                    "normal",
                    AgentRuntimeEvent::TurnCompleted(TurnResult::Completed {
                        stop_reason: None,
                        token_usage: None,
                    }),
                    ExpectedRuntimeTerminal::Completed,
                ),
                (
                    "fatal",
                    AgentRuntimeEvent::Fatal {
                        message: format!("{backend_id} provider fatal"),
                    },
                    ExpectedRuntimeTerminal::Interrupted(DomainInterruptReason::Crash),
                ),
                (
                    "abort",
                    AgentRuntimeEvent::TurnCompleted(TurnResult::Interrupted {
                        reason: DomainInterruptReason::Abort,
                        error: Some(format!("{backend_id} provider abort")),
                    }),
                    ExpectedRuntimeTerminal::Interrupted(DomainInterruptReason::Abort),
                ),
            ] {
                let fixture = stop_fixture(backend_id, label).await;
                let request = StopOperationRequest {
                    principal: LOCAL_INSTALLATION_OPERATION_PRINCIPAL.to_string(),
                    request_id: format!("f01-{backend_id}-{label}-stop"),
                    session_id: fixture.session_id.clone(),
                    turn_id: fixture.turn_id.to_string(),
                    expected_session_revision: fixture.session_revision,
                };
                let stop = fixture.stop.clone();
                let pending = tokio::spawn(async move { stop.request(request).await });
                wait_for_interrupt_handoff(&fixture).await;
                tokio::time::sleep(Duration::from_millis(25)).await;
                assert!(
                    !pending.is_finished(),
                    "{backend_id} transport acknowledgement is only a handoff"
                );

                let final_part = format!("{backend_id} {label} final part");
                fixture
                    .controller
                    .emit(
                        &fixture.session_id,
                        AgentRuntimeEvent::PartsMerged(vec![MessagePart::Text {
                            content: final_part.clone(),
                            parent_tool_use_id: None,
                        }]),
                    )
                    .unwrap();
                fixture
                    .controller
                    .emit(&fixture.session_id, terminal_event)
                    .unwrap();
                let outcome = tokio::time::timeout(Duration::from_secs(2), pending)
                    .await
                    .expect("runtime terminal must resolve Stop")
                    .unwrap()
                    .unwrap();
                assert_runtime_winner(&fixture, outcome, expected, &final_part).await;
            }
        }
    }

    #[tokio::test]
    async fn f01_no_provider_terminal_cas_timeout_and_fences_late_provider_events() {
        for backend_id in ["codex", "claude"] {
            let fixture = stop_fixture(backend_id, "timeout").await;
            tokio::time::pause();
            let request = StopOperationRequest {
                principal: LOCAL_INSTALLATION_OPERATION_PRINCIPAL.to_string(),
                request_id: format!("f01-{backend_id}-timeout-stop"),
                session_id: fixture.session_id.clone(),
                turn_id: fixture.turn_id.to_string(),
                expected_session_revision: fixture.session_revision,
            };
            let stop = fixture.stop.clone();
            let pending = tokio::spawn(async move { stop.request(request).await });
            wait_for_interrupt_handoff(&fixture).await;
            tokio::time::advance(Duration::from_millis(9_999)).await;
            tokio::task::yield_now().await;
            assert!(
                !pending.is_finished(),
                "Stop must keep accepting the provider terminal through T0+10"
            );
            tokio::time::advance(Duration::from_millis(1)).await;
            tokio::time::resume();
            let outcome = tokio::time::timeout(Duration::from_secs(2), pending)
                .await
                .expect("Timeout must converge at T0+10")
                .unwrap()
                .unwrap();
            let StopCommandOutcome::Accepted { receipt, state } = outcome else {
                panic!("Timeout Stop must remain accepted")
            };
            assert_eq!(
                state,
                StopOperationState::Completed {
                    resolution: StopResolution::Succeeded,
                }
            );
            let terminal = match fixture
                .store
                .query(LocalEventQuery::TerminalByTurn {
                    session_id: fixture.session_id.clone(),
                    turn_id: fixture.turn_id.to_string(),
                })
                .await
                .unwrap()
            {
                LocalEventQueryResult::TerminalByTurn(Some(terminal)) => terminal,
                other => panic!("Timeout terminal missing: {other:?}"),
            };
            assert_eq!(terminal.terminal_identity, receipt.operation_id.clone());
            assert!(matches!(
                &terminal.result,
                TerminalResultRecord::Stop {
                    operation_id,
                    reason: Some(TerminalInterruptReasonRecord::Timeout),
                    result:
                        TurnResult::Interrupted {
                            reason: DomainInterruptReason::Timeout,
                            ..
                        },
                    ..
                } if operation_id == &receipt.operation_id
            ));
            assert!(matches!(
                fixture
                    .store
                    .query(LocalEventQuery::StopResolutionByOperation {
                        stop_operation_id: receipt.operation_id.clone(),
                    })
                    .await
                    .unwrap(),
                LocalEventQueryResult::StopResolutionByOperation(Some(resolution))
                    if resolution.resolution == StopResolutionKind::Succeeded
            ));
            let calls = fixture.controller.call_kinds_for(&fixture.session_id);
            assert_eq!(
                calls
                    .iter()
                    .filter(|kind| **kind == TestRuntimeCallKind::Interrupt)
                    .count(),
                1
            );
            assert_eq!(
                calls
                    .iter()
                    .filter(|kind| **kind == TestRuntimeCallKind::Close)
                    .count(),
                1,
                "the Timeout winner must close the old provider exactly once"
            );
            assert!(!fixture.runtime.has_live_runtime(&fixture.session_id).await);

            let late_part = format!("{backend_id} late provider mutation");
            fixture
                .controller
                .emit(
                    &fixture.session_id,
                    AgentRuntimeEvent::PartsMerged(vec![MessagePart::Text {
                        content: late_part.clone(),
                        parent_tool_use_id: None,
                    }]),
                )
                .unwrap();
            fixture
                .controller
                .emit(
                    &fixture.session_id,
                    AgentRuntimeEvent::TurnCompleted(TurnResult::Failed {
                        error: "late provider failure".to_string(),
                        token_usage: None,
                    }),
                )
                .unwrap();
            tokio::time::sleep(Duration::from_millis(25)).await;
            assert_eq!(
                fixture
                    .store
                    .query(LocalEventQuery::TerminalByTurn {
                        session_id: fixture.session_id.clone(),
                        turn_id: fixture.turn_id.to_string(),
                    })
                    .await
                    .unwrap(),
                LocalEventQueryResult::TerminalByTurn(Some(terminal.clone()))
            );
            let session = fixture
                .session_store
                .load_full_session_for_restore(fixture.data.path(), &fixture.session_id)
                .unwrap()
                .unwrap();
            assert!(!session.messages.iter().any(|message| {
                message.parts.iter().flatten().any(
					|part| matches!(part, MessagePart::Text { content, .. } if content == &late_part),
				)
            }));
            let events = fixture
                .session_store
                .load_session_events(fixture.data.path(), &fixture.session_id)
                .unwrap();
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        AgentSessionDomainEvent::TurnCompleted { .. }
                            | AgentSessionDomainEvent::TurnInterrupted { .. }
                    ))
                    .count(),
                1
            );
            assert!(matches!(
                fixture
                    .store
                    .query(LocalEventQuery::PendingRecoveryPage {
                        limit: 16,
                        partition: None,
                        owner: Some(fixture.session_id.clone()),
                        ordered_key_prefix: None,
                        shutdown_plan: None,
                        cursor: None,
                    })
                    .await
                    .unwrap(),
                LocalEventQueryResult::PendingRecoveryPage(page) if page.entries.is_empty()
            ));

            let reader =
                crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore::open(
                    fixture.data.path(),
                )
                .unwrap();
            assert_eq!(
                reader
                    .query(LocalEventQuery::TerminalByTurn {
                        session_id: fixture.session_id.clone(),
                        turn_id: fixture.turn_id.to_string(),
                    })
                    .await
                    .unwrap(),
                LocalEventQueryResult::TerminalByTurn(Some(terminal))
            );
        }
    }
}

#[cfg(test)]
mod recovery_executor_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::{
        bind_runtime_terminal_operation_participant_provider, permission_response_retry_material,
        recovery_handoff_target_matches, session_close_readback_obligation_id,
        stop_readback_obligation_id, ConservativeRecoveryExecutor,
        RuntimeAgentSessionOperationGate, RuntimePermissionResponseOperationGate,
        RuntimeSendOperationGate,
    };
    use crate::domain::agent_session::entities::PermissionResponseDecision;
    use crate::domain::agent_session::entities::{PermissionResponse, TurnResult};
    use crate::domain::agent_session::events::{
        BackendSessionRecoveryReason, RecoveryActionKind, RecoveryResultClassification,
        SendDisposition, StopResolution,
    };
    use crate::domain::local_event::{
        AgentTerminalKind, AgentTurnTerminalResultRecord, CommitBatchResult, CommitIdentity,
        CommitOperationKind, IdempotencyBinding, LocalAtomicBatch, LocalEventQuery,
        LocalEventQueryResult, LocalEventTransactionRepository, LocalStateMutation,
        ObligationMutation, ObligationRecord, ObligationStateRecord, ObligationView, OperationKind,
        OperationReceiptRecord, OperationRecordMutation, OperationStatusRecord,
        OperationStatusValue, PendingIndexEntry, PendingIndexEntryView, PendingPartition,
        RecordAuthentication, RecoveryPublicationMessageKindRecord,
        RecoveryPublicationMessageRecord, RecoveryPublicationObligationRecord, Revision,
        RevisionGuard, SafeOperationFailure, SendObligationDispositionRecord,
        SendObligationKindRecord, SessionLifecycleRecordAction, SessionOperationFailureKind,
        SessionProjectionMutation, StopResolutionKind, TerminalRecordMutation,
        TerminalResultRecord,
    };
    use crate::usecase::agent_session::operation::{
        AcceptedPermissionResponseEffect, AcceptedStopEffect, AgentSendOperationUsecase,
        BackendRecoveryReadbackPort, BackendRecoveryReadbackRequest, PendingRecoveryCategory,
        PendingRecoveryQuery, PermissionResponseGate, PermissionResponseOperationUsecase,
        PermissionResponsePlan, RecoveryActionError, RecoveryActionOutcome,
        RecoveryActionRejection, RecoveryActionRequest, RecoveryActionResultOutcome,
        RecoveryActionStatus, RecoveryActionUsecase, RecoveryEffectExecutor, RecoveryEffectRequest,
        RecoveryEffectResult, SendRecoveryReadbackPort, SendRecoveryReadbackRequest,
        SessionCloseRecoveryReadbackPort, SessionCloseRecoveryReadbackRequest,
        SessionLifecycleOperationUsecase, StopAdmissionGate, StopCommandOutcome,
        StopEffectObservation, StopOperationRequest, StopOperationState, StopOperationUsecase,
        StopRecoveryReadbackPort, StopRecoveryReadbackRequest, StopTargetSnapshot,
    };

    #[derive(Default)]
    struct RecordingReadbackPorts {
        calls: Mutex<Vec<String>>,
    }

    impl RecordingReadbackPorts {
        fn result(
            &self,
            label: &str,
            effect_identity: &str,
            classification: RecoveryResultClassification,
        ) -> RecoveryEffectResult {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{label}:{effect_identity}"));
            RecoveryEffectResult {
                classification,
                safe_result: format!("{label} typed readback"),
                owner_mutations: Vec::new(),
                owner_batch: None,
            }
        }
    }

    #[async_trait::async_trait]
    impl StopRecoveryReadbackPort for RecordingReadbackPorts {
        async fn read_stop(
            &self,
            request: &StopRecoveryReadbackRequest,
        ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
            Ok(self.result(
                "stop",
                request.effect_identity.as_str(),
                RecoveryResultClassification::Succeeded,
            ))
        }
    }

    #[async_trait::async_trait]
    impl SessionCloseRecoveryReadbackPort for RecordingReadbackPorts {
        async fn read_session_close(
            &self,
            request: &SessionCloseRecoveryReadbackRequest,
        ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
            Ok(self.result(
                "session-close",
                request.effect_identity.as_str(),
                RecoveryResultClassification::ConfirmedNoEffect,
            ))
        }
    }

    #[async_trait::async_trait]
    impl BackendRecoveryReadbackPort for RecordingReadbackPorts {
        async fn read_backend_recovery(
            &self,
            request: &BackendRecoveryReadbackRequest,
        ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
            Ok(self.result(
                "backend-recovery",
                request.effect_identity.as_str(),
                RecoveryResultClassification::Ambiguous,
            ))
        }
    }

    #[async_trait::async_trait]
    impl SendRecoveryReadbackPort for RecordingReadbackPorts {
        async fn read_send(
            &self,
            request: &SendRecoveryReadbackRequest,
        ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
            Ok(self.result(
                "send",
                request.effect_identity.as_str(),
                RecoveryResultClassification::Pending,
            ))
        }
    }

    struct UnusedPermissionGate;

    fn unused_failure() -> SafeOperationFailure {
        SafeOperationFailure::new(
            SessionOperationFailureKind::InvalidEffectIntent,
            false,
            "unused permission gate",
            "f05-unused-permission-gate".to_string(),
        )
    }

    #[async_trait::async_trait]
    impl PermissionResponseGate for UnusedPermissionGate {
        async fn plan_response(
            &self,
            _session_id: &str,
            _response: &PermissionResponse,
        ) -> Result<PermissionResponsePlan, SafeOperationFailure> {
            Err(unused_failure())
        }

        async fn execute(
            &self,
            _effect: &AcceptedPermissionResponseEffect,
        ) -> Result<(), SafeOperationFailure> {
            Err(unused_failure())
        }
    }

    fn production_executor_fixture() -> (
        ConservativeRecoveryExecutor,
        Arc<RecordingReadbackPorts>,
        tempfile::TempDir,
    ) {
        let data = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                data.path().to_path_buf(),
            ),
        )
        .unwrap();
        let permission = Arc::new(PermissionResponseOperationUsecase::new(
            store.clone(),
            store.clone(),
            Arc::new(UnusedPermissionGate),
            store.generation_id().to_string(),
        ));
        let ports = Arc::new(RecordingReadbackPorts::default());
        let executor = ConservativeRecoveryExecutor::from_readback_ports(
            ports.clone(),
            ports.clone(),
            ports.clone(),
            ports.clone(),
            permission,
            store,
        );
        (executor, ports, data)
    }

    fn handoff_request() -> RecoveryEffectRequest {
        RecoveryEffectRequest {
            action_id: "action-1".to_string(),
            obligation_id: "obligation-1".to_string(),
            origin_revision: 7,
            expected_owner: Some("session-1".to_string()),
            action: RecoveryActionKind::ReadAgain,
            immutable_obligation: ObligationRecord::StopInterrupt {
                operation_id: "stop-operation-1".to_string(),
                session_id: "session-1".to_string(),
                turn_id: "1".to_string(),
                expected_revision: 7,
                deadline_ms: 0,
                state: ObligationStateRecord::ReconciliationRequired,
            },
            authoritative_observation: None,
        }
    }

    fn handoff_target(revision: i64, owner: Option<&str>) -> ObligationView {
        let record = handoff_request().immutable_obligation;
        ObligationView {
            obligation_id: "obligation-1".to_string(),
            record,
            record_sha256: [0; 32],
            pending: owner.map(|owner| PendingIndexEntryView {
                ordered_key: "pending-1".to_string(),
                owner: owner.to_string(),
                partition: PendingPartition::Owner,
                shutdown_plan: None,
            }),
            revision: Revision::new(revision).unwrap(),
        }
    }

    #[tokio::test]
    async fn f05_production_executor_dispatches_four_typed_readbacks_by_stable_effect_identity() {
        let (executor, ports, _data) = production_executor_fixture();
        let stop_obligation_id = stop_readback_obligation_id("session-stop", "7");
        let session_close_obligation_id = session_close_readback_obligation_id("session-close");
        let fixtures = vec![
            (
                stop_obligation_id.clone(),
                ObligationRecord::StopInterrupt {
                    operation_id: "stop-operation".to_string(),
                    session_id: "session-stop".to_string(),
                    turn_id: "7".to_string(),
                    expected_revision: 0,
                    deadline_ms: 0,
                    state: ObligationStateRecord::ReconciliationRequired,
                },
                RecoveryResultClassification::Succeeded,
            ),
            (
                session_close_obligation_id.clone(),
                ObligationRecord::SessionClose {
                    obligation_id: session_close_obligation_id.clone(),
                    operation_id: "close-operation".to_string(),
                    session_id: "session-close".to_string(),
                    action: SessionLifecycleRecordAction::Close,
                    state: ObligationStateRecord::ReconciliationRequired,
                },
                RecoveryResultClassification::ConfirmedNoEffect,
            ),
            (
                "backend-recovery:session-backend:recovery-1".to_string(),
                ObligationRecord::BackendSessionRecovery {
                    session_id: "session-backend".to_string(),
                    recovery_id: "recovery-1".to_string(),
                    detail: None,
                    state: ObligationStateRecord::EffectReserved,
                },
                RecoveryResultClassification::Ambiguous,
            ),
            (
                "send-operation.exec".to_string(),
                ObligationRecord::Send {
                    obligation_id: "send-operation.exec".to_string(),
                    operation_id: "send-operation".to_string(),
                    session_id: "session-send".to_string(),
                    kind: SendObligationKindRecord::TurnExecution,
                    disposition: SendObligationDispositionRecord::StartedTurn,
                    human_message_id: Some("human-1".to_string()),
                    assistant_message_id: None,
                    reserved_turn_id: Some("7".to_string()),
                    turn_id: Some("7".to_string()),
                    dependency_obligation_ids: Vec::new(),
                    canonical_payload: "payload".to_string(),
                    state: ObligationStateRecord::ReconciliationRequired,
                },
                RecoveryResultClassification::Pending,
            ),
        ];

        for (index, (obligation_id, record, expected)) in fixtures.into_iter().enumerate() {
            let request = RecoveryEffectRequest {
                action_id: format!("action-{index}"),
                obligation_id,
                origin_revision: 4,
                expected_owner: None,
                action: RecoveryActionKind::ReadAgain,
                immutable_obligation: record,
                authoritative_observation: None,
            };
            assert!(
                executor.supports_read_again(&request.obligation_id, &request.immutable_obligation)
            );
            assert_eq!(
                executor.execute(&request).await.unwrap().classification,
                expected
            );
        }
        assert_eq!(
            *ports.calls.lock().unwrap(),
            vec![
                format!("stop:{stop_obligation_id}"),
                format!("session-close:{session_close_obligation_id}"),
                "backend-recovery:backend-recovery:session-backend:recovery-1".to_string(),
                "send:send-operation.exec".to_string(),
            ]
        );

        for unsupported in [
            ObligationRecord::Send {
                obligation_id: "send-provider-operation.establish".to_string(),
                operation_id: "send-provider-operation".to_string(),
                session_id: "session-send-provider".to_string(),
                kind: SendObligationKindRecord::ProviderEstablish,
                disposition: SendObligationDispositionRecord::StartedTurn,
                human_message_id: Some("human-provider-1".to_string()),
                assistant_message_id: None,
                reserved_turn_id: Some("8".to_string()),
                turn_id: Some("8".to_string()),
                dependency_obligation_ids: Vec::new(),
                canonical_payload: "provider-payload".to_string(),
                state: ObligationStateRecord::ReconciliationRequired,
            },
            ObligationRecord::WorkflowShutdown {
                operation_id: "quit-1".to_string(),
                effect_identity: "workflow-1".to_string(),
                owner_revision: 0,
                execution_id: "execution-1".to_string(),
                state: ObligationStateRecord::EffectReserved,
            },
            ObligationRecord::RecoveryPublication {
                session_id: "session-1".to_string(),
                recovery_id: "recovery-1".to_string(),
                message_id: "message-1".to_string(),
                source_obligation_id: "source-1".to_string(),
                detail: RecoveryPublicationObligationRecord::Pending {
                    pending_message: RecoveryPublicationMessageRecord {
                        kind: RecoveryPublicationMessageKindRecord::Notice,
                        recovery_id: "recovery-1".to_string(),
                        message_id: "message-1".to_string(),
                        error: None,
                    },
                },
                state: ObligationStateRecord::Pending,
            },
        ] {
            assert!(!executor.supports_read_again("unsupported", &unsupported));
        }
    }

    #[test]
    fn f05_production_executor_rejects_unexecutable_read_again_capabilities() {
        let (executor, ports, _data) = production_executor_fixture();
        let stop_obligation_id = stop_readback_obligation_id("session-stop", "7");
        let close_obligation_id = session_close_readback_obligation_id("session-close");
        let fixtures = vec![
            (
                "terminal commit is not a stop readback",
                stop_obligation_id.clone(),
                ObligationRecord::TerminalCommit {
                    operation_id: "stop-operation".to_string(),
                    session_id: "session-stop".to_string(),
                    turn_id: "7".to_string(),
                    terminal_identity: "terminal-1".to_string(),
                    state: ObligationStateRecord::ReconciliationRequired,
                },
            ),
            (
                "typed records still require an executable operation identity",
                stop_obligation_id,
                ObligationRecord::StopInterrupt {
                    operation_id: "invalid operation".to_string(),
                    session_id: "session-stop".to_string(),
                    turn_id: "7".to_string(),
                    expected_revision: 0,
                    deadline_ms: 0,
                    state: ObligationStateRecord::ReconciliationRequired,
                },
            ),
            (
                "legacy provider establish is not unified Send",
                "send-operation.establish".to_string(),
                ObligationRecord::ProviderEstablish {
                    operation_id: "send-operation".to_string(),
                    effect_identity: "send-operation.establish".to_string(),
                    session_id: "session-send".to_string(),
                    state: ObligationStateRecord::ReconciliationRequired,
                },
            ),
            (
                "legacy turn execution is not unified Send",
                "send-operation.exec".to_string(),
                ObligationRecord::TurnExecution {
                    operation_id: "send-operation".to_string(),
                    session_id: "session-send".to_string(),
                    turn_id: "7".to_string(),
                    state: ObligationStateRecord::ReconciliationRequired,
                },
            ),
            (
                "non-Close lifecycle action is not executable",
                close_obligation_id.clone(),
                ObligationRecord::SessionClose {
                    obligation_id: close_obligation_id.clone(),
                    operation_id: "close-operation".to_string(),
                    session_id: "session-close".to_string(),
                    action: SessionLifecycleRecordAction::ArchiveOpen,
                    state: ObligationStateRecord::ReconciliationRequired,
                },
            ),
            (
                "StopInterrupt requires its deterministic target identity",
                "wrong-stop-effect".to_string(),
                ObligationRecord::StopInterrupt {
                    operation_id: "stop-operation".to_string(),
                    session_id: "session-stop".to_string(),
                    turn_id: "7".to_string(),
                    expected_revision: 0,
                    deadline_ms: 0,
                    state: ObligationStateRecord::ReconciliationRequired,
                },
            ),
            (
                "SessionClose requires matching stored and deterministic identities",
                "wrong-close-effect".to_string(),
                ObligationRecord::SessionClose {
                    obligation_id: close_obligation_id,
                    operation_id: "close-operation".to_string(),
                    session_id: "session-close".to_string(),
                    action: SessionLifecycleRecordAction::Close,
                    state: ObligationStateRecord::ReconciliationRequired,
                },
            ),
            (
                "backend recovery requires its exact effect identity",
                "wrong-backend-effect".to_string(),
                ObligationRecord::BackendSessionRecovery {
                    session_id: "session-backend".to_string(),
                    recovery_id: "recovery-1".to_string(),
                    detail: None,
                    state: ObligationStateRecord::EffectReserved,
                },
            ),
            (
                "unified Send requires the effect identity for its kind",
                "send-operation.wrong".to_string(),
                ObligationRecord::Send {
                    obligation_id: "send-operation.wrong".to_string(),
                    operation_id: "send-operation".to_string(),
                    session_id: "session-send".to_string(),
                    kind: SendObligationKindRecord::TurnExecution,
                    disposition: SendObligationDispositionRecord::StartedTurn,
                    human_message_id: Some("human-1".to_string()),
                    assistant_message_id: None,
                    reserved_turn_id: Some("7".to_string()),
                    turn_id: Some("7".to_string()),
                    dependency_obligation_ids: Vec::new(),
                    canonical_payload: "payload".to_string(),
                    state: ObligationStateRecord::EffectReserved,
                },
            ),
        ];

        for (label, obligation_id, record) in fixtures {
            assert!(
                !executor.supports_read_again(&obligation_id, &record),
                "{label}"
            );
        }
        assert!(ports.calls.lock().unwrap().is_empty());
    }

    #[derive(Default)]
    struct F05ResultLostStopGate {
        interrupts: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl StopAdmissionGate for F05ResultLostStopGate {
        async fn target_snapshot(
            &self,
            _session_id: &str,
        ) -> Result<StopTargetSnapshot, SafeOperationFailure> {
            Ok(StopTargetSnapshot {
                session_revision: 0,
                active_turn_id: "1".to_string(),
                queue_paused: false,
            })
        }

        async fn interrupt(
            &self,
            _effect: &AcceptedStopEffect,
        ) -> Result<StopEffectObservation, SafeOperationFailure> {
            self.interrupts.fetch_add(1, Ordering::SeqCst);
            Err(SafeOperationFailure::new(
                SessionOperationFailureKind::OutcomeUnknown,
                true,
                "The provider effect result was lost before it could be persisted.",
                "f05-stop-result-lost".to_string(),
            ))
        }
    }

    fn f05_production_recovery_graph(
        store: &Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        data_dir: &std::path::Path,
    ) -> (
        RecoveryActionUsecase,
        Arc<StopOperationUsecase>,
        crate::test_support::TestAgentRuntimeController,
    ) {
        let session_store = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
        let authority: Arc<
            dyn crate::usecase::agent_session::operation::OperationBindingAuthority,
        > = store.clone();
        session_store.set_local_event_repository_with_projection_codec(
            repository.clone(),
            store.generation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let (runtime, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                data_dir,
            );
        let operation_gate = Arc::new(RuntimeAgentSessionOperationGate::new(
            runtime.clone(),
            session_store.clone(),
            data_dir.to_path_buf(),
        ));
        let lifecycle = Arc::new(SessionLifecycleOperationUsecase::new(
            repository.clone(),
            authority.clone(),
            operation_gate.clone(),
            store.generation_id().to_string(),
        ));
        let stop = Arc::new(StopOperationUsecase::new(
            repository.clone(),
            authority.clone(),
            operation_gate.clone(),
            store.generation_id().to_string(),
        ));
        operation_gate.bind_stop_operation(Arc::downgrade(&stop));
        let send_gate = Arc::new(RuntimeSendOperationGate::new(
            runtime.clone(),
            session_store.clone(),
            data_dir.to_path_buf(),
        ));
        let send = Arc::new(AgentSendOperationUsecase::new(
            repository.clone(),
            authority.clone(),
            send_gate.clone(),
            store.generation_id().to_string(),
        ));
        operation_gate.bind_send_operation(Arc::downgrade(&send));
        send_gate.bind_status_sink(Arc::downgrade(&send));
        bind_runtime_terminal_operation_participant_provider(
            &session_store,
            stop.clone(),
            send.clone(),
        );
        let permission = Arc::new(PermissionResponseOperationUsecase::new(
            repository,
            authority,
            Arc::new(RuntimePermissionResponseOperationGate::new(
                runtime,
                session_store,
            )),
            store.generation_id().to_string(),
        ));
        let executor = Arc::new(ConservativeRecoveryExecutor::new(
            stop.clone(),
            lifecycle,
            operation_gate,
            send,
            permission,
            store.clone(),
        ));
        (
            RecoveryActionUsecase::new(
                store.clone(),
                store.clone(),
                executor,
                store.generation_id().to_string(),
            ),
            stop,
            controller,
        )
    }

    #[tokio::test]
    async fn f05_production_stop_read_again_uses_durable_terminal_winner_after_restart() {
        let directory = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ),
        )
        .unwrap();
        let gate = Arc::new(F05ResultLostStopGate::default());
        let seed_stop = StopOperationUsecase::new(
            store.clone(),
            store.clone(),
            gate.clone(),
            store.generation_id().to_string(),
        );
        let session_id = "f05-production-stop-session";
        let turn_id = "1";
        let caller_request_id = "f05-production-stop-request";
        let accepted = seed_stop
            .request(StopOperationRequest {
                principal: "local-app".to_string(),
                request_id: caller_request_id.to_string(),
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
                expected_session_revision: 0,
            })
            .await
            .unwrap();
        let StopCommandOutcome::Accepted { receipt, state } = accepted else {
            panic!("Stop was not durably accepted: {accepted:?}");
        };
        assert!(matches!(
            state,
            StopOperationState::ReconciliationRequired { .. }
        ));
        assert_eq!(gate.interrupts.load(Ordering::SeqCst), 1);

        let obligation_id = format!(
            "stop-target-{}",
            hex::encode(
                crate::usecase::agent_session::operation::OperationBindingAuthority::digest(
                    store.as_ref(),
                    format!("stop-target-obligation/v1\0{session_id}\0{turn_id}").as_bytes(),
                ),
            ),
        );
        let terminal_identity = "f05-production-provider-terminal";
        f05_commit_mutations(
            &store,
            "f05-production-provider-terminal-evidence",
            vec![LocalStateMutation::TerminalRecord(TerminalRecordMutation {
                session_id: session_id.to_string(),
                turn_id: turn_id.to_string(),
                terminal_identity: terminal_identity.to_string(),
                result: TerminalResultRecord::AgentTurn {
                    kind: AgentTerminalKind::Completed,
                    session_id: session_id.to_string(),
                    turn_id: turn_id.to_string(),
                    message_id: "f05-production-assistant".to_string(),
                    streaming_final_sequence: 0,
                    completed_at_bits: 1.0_f64.to_bits(),
                    result: AgentTurnTerminalResultRecord::Current(TurnResult::Completed {
                        stop_reason: None,
                        token_usage: None,
                    }),
                },
                participant_digest: [5; 32],
            })],
        )
        .await;
        assert!(matches!(
            store
                .query(LocalEventQuery::StopResolutionByOperation {
                    stop_operation_id: receipt.operation_id.clone(),
                })
                .await
                .unwrap(),
            LocalEventQueryResult::StopResolutionByOperation(None)
        ));

        drop(seed_stop);
        drop(store);
        let reopened = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ),
        )
        .unwrap();
        let (recovery, stop, controller) =
            f05_production_recovery_graph(&reopened, directory.path());
        let request = f05_read_again_request(&recovery, session_id, &obligation_id).await;

        let first = recovery.request(request.clone()).await.unwrap();
        let RecoveryActionOutcome::Completed { result, .. } = &first else {
            panic!("ReadAgain did not complete: {first:?}");
        };
        assert_eq!(
            result.outcome,
            RecoveryActionResultOutcome::Terminal,
            "a durable terminal winner is authoritative evidence that the provider effect completed"
        );
        assert_eq!(
            result.classification,
            RecoveryResultClassification::Succeeded
        );
        let (saved_receipt, saved_state) = stop
            .get_operation("local-app", &receipt.operation_id)
            .await
            .unwrap();
        assert_eq!(saved_receipt, receipt);
        assert_eq!(
            saved_state,
            StopOperationState::Completed {
                resolution: StopResolution::Superseded,
            }
        );
        assert!(matches!(
            reopened
                .query(LocalEventQuery::StopResolutionByOperation {
                    stop_operation_id: saved_receipt.operation_id.clone(),
                })
                .await
                .unwrap(),
            LocalEventQueryResult::StopResolutionByOperation(Some(ref resolution))
                if resolution.resolution == StopResolutionKind::Superseded
        ));
        assert!(matches!(
            reopened
                .query(LocalEventQuery::ObligationByIdentity {
                    obligation_id: obligation_id.clone(),
                })
                .await
                .unwrap(),
            LocalEventQueryResult::ObligationByIdentity(Some(ref obligation))
                if obligation.pending.is_none()
        ));
        assert!(matches!(
            reopened
                .query(LocalEventQuery::TerminalByTurn {
                    session_id: session_id.to_string(),
                    turn_id: turn_id.to_string(),
                })
                .await
                .unwrap(),
            LocalEventQueryResult::TerminalByTurn(Some(ref terminal))
                if terminal.terminal_identity == terminal_identity
        ));
        assert_eq!(recovery.request(request).await.unwrap(), first);
        assert_eq!(gate.interrupts.load(Ordering::SeqCst), 1);
        assert!(!controller
            .call_kinds_for(session_id)
            .contains(&crate::test_support::TestRuntimeCallKind::Interrupt));
    }

    #[tokio::test]
    async fn f05_production_backend_recovery_read_again_finishes_from_durable_owner_evidence_after_restart(
    ) {
        // Given a real backend-recovery reservation whose external effect has
        // durably installed a new provider identity, but whose completion
        // event/result/publication batch was lost at the crash boundary.
        let directory = tempfile::tempdir().unwrap();
        let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ),
        )
        .unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
        session_store.set_local_event_repository_with_projection_codec(
            repository,
            store.generation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let session_id = "f05-production-backend-recovery-session";
        let recovery_id = "f05-production-backend-recovery";
        let provider_session_id = "f05-provider-session-after-effect";
        let session = crate::usecase::agent_session::session::build_new_session_with_id(
            session_id.to_string(),
            "/tmp/f05-backend-recovery",
            Some("codex".to_string()),
            crate::domain::agent_session::PermissionMode::Ask,
            None,
            false,
            false,
            None,
        );
        session_store
            .save_full_session_for_migration_or_restore(directory.path(), &session)
            .unwrap();
        session_store
            .begin_backend_session_recovery(
                directory.path(),
                session_id,
                recovery_id,
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();

        let projection = store
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: session_id.to_string(),
            })
            .await
            .unwrap();
        let LocalEventQueryResult::SessionProjectionByIdentity(Some(projection)) = projection
        else {
            panic!("backend-recovery owner projection missing after reservation");
        };
        let codec =
            crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1;
        let mut durable_owner =
            crate::usecase::agent_session::session::AgentSessionProjectionCodec::decode(
                &codec,
                &projection.projection,
            )
            .unwrap();
        assert_eq!(durable_owner.meta.provider_session_generation, 0);
        assert_eq!(durable_owner.meta.agent_session_id, None);
        durable_owner.meta.agent_session_id = Some(provider_session_id.to_string());
        durable_owner.meta.provider_session_generation = 1;
        durable_owner.meta.context_reinjection_generation = Some(1);
        let durable_owner =
            crate::usecase::agent_session::session::AgentSessionProjectionCodec::encode(
                &codec,
                &durable_owner,
            )
            .unwrap();
        f05_commit_mutations(
            &store,
            "f05-production-backend-recovery-effect-owner-evidence",
            vec![LocalStateMutation::SessionProjection(
                SessionProjectionMutation {
                    session_id: session_id.to_string(),
                    projection: durable_owner,
                    expected: RevisionGuard::Expected(projection.revision),
                    revision: projection.revision.next().unwrap(),
                },
            )],
        )
        .await;

        let events = session_store
            .load_session_events(directory.path(), session_id)
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            crate::domain::agent_session::events::AgentSessionDomainEvent::BackendSessionRecoveryStarted {
                recovery_id: stored_recovery_id,
                ..
            } if stored_recovery_id == recovery_id
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            crate::domain::agent_session::events::AgentSessionDomainEvent::BackendSessionRecoveryCompleted {
                recovery_id: stored_recovery_id,
                ..
            } if stored_recovery_id == recovery_id
        )));
        let obligation_id = format!("backend-recovery:{session_id}:{recovery_id}");
        assert!(matches!(
            store
                .query(LocalEventQuery::ObligationByIdentity {
                    obligation_id: obligation_id.clone(),
                })
                .await
                .unwrap(),
            LocalEventQueryResult::ObligationByIdentity(Some(ref obligation))
                if obligation.pending.is_some()
                    && matches!(
                        obligation.record,
                        ObligationRecord::BackendSessionRecovery {
                            state: ObligationStateRecord::EffectReserved,
                            ..
                        }
                    )
        ));

        // When the process restarts and backend-issued ReadAgain runs through
        // the concrete production RuntimeAgentSessionOperationGate.
        drop(session_store);
        drop(store);
        let reopened = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                directory.path().to_path_buf(),
            ),
        )
        .unwrap();
        let (recovery, _stop, controller) =
            f05_production_recovery_graph(&reopened, directory.path());
        let request = f05_read_again_request(&recovery, session_id, &obligation_id).await;
        let first = recovery.request(request.clone()).await.unwrap();

        // Then the durable provider identity is authoritative success evidence:
        // the source is terminal and the owner completion/publication
        // participants converge without invoking the provider effect again.
        let RecoveryActionOutcome::Completed { result, .. } = &first else {
            panic!("backend-recovery ReadAgain did not complete: {first:?}");
        };
        assert_eq!(
            result.outcome,
            RecoveryActionResultOutcome::Terminal,
            "a newer durable provider identity proves that the backend-recovery effect completed"
        );
        assert_eq!(
            result.classification,
            RecoveryResultClassification::Succeeded
        );
        assert!(matches!(
            reopened
                .query(LocalEventQuery::ObligationByIdentity {
                    obligation_id: obligation_id.clone(),
                })
                .await
                .unwrap(),
            LocalEventQueryResult::ObligationByIdentity(Some(ref obligation))
                if obligation.pending.is_none()
                    && matches!(
                        obligation.record,
                        ObligationRecord::RecoveryTransition {
                            recovery_action:
                                crate::domain::local_event::ObligationRecoveryActionRecord {
                                    state: ObligationStateRecord::Completed,
                                    classification: Some(RecoveryResultClassification::Succeeded),
                                    ..
                                },
                            ..
                        }
                    )
        ));

        let projection = reopened
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: session_id.to_string(),
            })
            .await
            .unwrap();
        let LocalEventQueryResult::SessionProjectionByIdentity(Some(projection)) = projection
        else {
            panic!("backend-recovery owner projection missing after ReadAgain");
        };
        let completed_owner =
            crate::usecase::agent_session::session::AgentSessionProjectionCodec::decode(
                &codec,
                &projection.projection,
            )
            .unwrap();
        assert_eq!(
            completed_owner.meta.agent_session_id.as_deref(),
            Some(provider_session_id)
        );
        assert_eq!(completed_owner.meta.provider_session_generation, 1);
        assert_eq!(completed_owner.meta.context_reinjection_generation, Some(1));
        let pending_publication = completed_owner
            .meta
            .pending_recovery_message
            .as_ref()
            .expect("successful readback must reserve the recovery publication");
        let publication_message_id = match pending_publication {
            crate::usecase::agent_session::session::PendingRecoveryMessage::Notice {
                recovery_id: stored_recovery_id,
                message_id,
            } if stored_recovery_id == recovery_id => message_id,
            other => panic!("unexpected backend-recovery publication: {other:?}"),
        };
        assert!(completed_owner.meta.recovery_publication_snapshot.is_none());
        assert!(completed_owner.reducer_events.iter().any(|event| matches!(
            event,
            crate::domain::agent_session::events::AgentSessionDomainEvent::BackendSessionRecoveryCompleted {
                recovery_id: stored_recovery_id,
                provider_session_generation: 1,
                ..
            } if stored_recovery_id == recovery_id
        )));
        let pending = recovery
            .pending(PendingRecoveryQuery {
                limit: 16,
                partition: None,
                owner: Some(session_id.to_string()),
                shutdown_plan: None,
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(pending.entries.len(), 1);
        assert_eq!(
            pending.entries[0].category,
            PendingRecoveryCategory::RecoveryPublication
        );
        assert_eq!(
            pending.entries[0].original_identity,
            publication_message_id.as_str()
        );
        assert_eq!(recovery.request(request).await.unwrap(), first);
        assert!(controller.call_kinds_for(session_id).is_empty());
    }

    #[derive(Clone, Copy)]
    enum F05FinishFault {
        FailBeforeCommit,
        DropReply,
        None,
    }

    fn f05_session_projection(
        owner: &str,
        state: crate::usecase::agent_session::session::SessionState,
    ) -> crate::domain::local_event::SessionProjectionRecord {
        let mut session = crate::usecase::agent_session::session::build_new_session_with_id(
            owner.to_string(),
            "/tmp/f05-readback",
            Some("codex".to_string()),
            crate::domain::agent_session::PermissionMode::Ask,
            None,
            false,
            false,
            None,
        );
        session.state = state;
        crate::usecase::agent_session::session::AgentSessionProjectionCodec::encode(
            &crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            &crate::usecase::agent_session::session::CanonicalAgentSessionProjection {
                meta: crate::usecase::agent_session::session::SessionMeta::from_session(&session),
                title: None,
                messages: Vec::new(),
                reducer_events: Vec::new(),
                queue_paused_at: None,
                latest_token_usage: None,
                pending_send_queue: Vec::new(),
            },
        )
        .unwrap()
    }

    struct AtomicProductionReadbackPorts {
        store: Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        calls: Arc<AtomicUsize>,
        expected_label: &'static str,
        expected_effect_identity: String,
        fault: F05FinishFault,
        stop: Arc<StopOperationUsecase>,
        lifecycle: Arc<SessionLifecycleOperationUsecase>,
        backend: Arc<RuntimeAgentSessionOperationGate>,
        send: Arc<AgentSendOperationUsecase>,
    }

    impl AtomicProductionReadbackPorts {
        fn observe_result(&self, label: &'static str, effect_identity: &str) {
            assert_eq!(label, self.expected_label);
            assert_eq!(effect_identity, self.expected_effect_identity);
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.fault {
                F05FinishFault::FailBeforeCommit => {
                    self.store.fault_injector().arm_fail_before_commit();
                }
                F05FinishFault::DropReply => self.store.fault_injector().arm_drop_reply(),
                F05FinishFault::None => {}
            }
        }
    }

    #[async_trait::async_trait]
    impl StopRecoveryReadbackPort for AtomicProductionReadbackPorts {
        async fn read_stop(
            &self,
            request: &StopRecoveryReadbackRequest,
        ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
            let result = self.stop.read_stop(request).await?;
            self.observe_result("stop", request.effect_identity.as_str());
            Ok(result)
        }
    }

    #[async_trait::async_trait]
    impl SessionCloseRecoveryReadbackPort for AtomicProductionReadbackPorts {
        async fn read_session_close(
            &self,
            request: &SessionCloseRecoveryReadbackRequest,
        ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
            let result = self.lifecycle.read_session_close(request).await?;
            self.observe_result("session-close", request.effect_identity.as_str());
            Ok(result)
        }
    }

    #[async_trait::async_trait]
    impl BackendRecoveryReadbackPort for AtomicProductionReadbackPorts {
        async fn read_backend_recovery(
            &self,
            request: &BackendRecoveryReadbackRequest,
        ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
            let result = self.backend.read_backend_recovery(request).await?;
            self.observe_result("backend-recovery", request.effect_identity.as_str());
            Ok(result)
        }
    }

    #[async_trait::async_trait]
    impl SendRecoveryReadbackPort for AtomicProductionReadbackPorts {
        async fn read_send(
            &self,
            request: &SendRecoveryReadbackRequest,
        ) -> Result<RecoveryEffectResult, SafeOperationFailure> {
            let result = self.send.read_send(request).await?;
            self.observe_result("send", request.effect_identity.as_str());
            Ok(result)
        }
    }

    fn f05_session_store(
        store: &Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    ) -> Arc<crate::usecase::agent_session::session::SessionStore> {
        let session_store = Arc::new(crate::test_support::build_session_store());
        let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
        session_store.set_local_event_repository_with_projection_codec(
            repository,
            store.generation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        session_store
    }

    fn f05_production_recovery_usecase(
        store: &Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        session_store: Arc<crate::usecase::agent_session::session::SessionStore>,
        data_dir: &std::path::Path,
        calls: Arc<AtomicUsize>,
        expected_label: &'static str,
        expected_effect_identity: String,
        fault: F05FinishFault,
    ) -> RecoveryActionUsecase {
        let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
        let authority: Arc<
            dyn crate::usecase::agent_session::operation::OperationBindingAuthority,
        > = store.clone();
        let (runtime, _controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                session_store.clone(),
                data_dir,
            );
        let operation_gate = Arc::new(RuntimeAgentSessionOperationGate::new(
            runtime.clone(),
            session_store.clone(),
            data_dir.to_path_buf(),
        ));
        let lifecycle = Arc::new(SessionLifecycleOperationUsecase::new(
            repository.clone(),
            authority.clone(),
            operation_gate.clone(),
            store.generation_id().to_string(),
        ));
        let stop = Arc::new(StopOperationUsecase::new(
            repository.clone(),
            authority.clone(),
            operation_gate.clone(),
            store.generation_id().to_string(),
        ));
        operation_gate.bind_stop_operation(Arc::downgrade(&stop));
        let send_gate = Arc::new(RuntimeSendOperationGate::new(
            runtime,
            session_store,
            data_dir.to_path_buf(),
        ));
        let send = Arc::new(AgentSendOperationUsecase::new(
            repository.clone(),
            authority.clone(),
            send_gate.clone(),
            store.generation_id().to_string(),
        ));
        operation_gate.bind_send_operation(Arc::downgrade(&send));
        send_gate.bind_status_sink(Arc::downgrade(&send));
        let permission = Arc::new(PermissionResponseOperationUsecase::new(
            repository,
            authority,
            Arc::new(UnusedPermissionGate),
            store.generation_id().to_string(),
        ));
        let ports = Arc::new(AtomicProductionReadbackPorts {
            store: store.clone(),
            calls,
            expected_label,
            expected_effect_identity,
            fault,
            stop,
            lifecycle,
            backend: operation_gate,
            send,
        });
        let executor = Arc::new(ConservativeRecoveryExecutor::from_readback_ports(
            ports.clone(),
            ports.clone(),
            ports.clone(),
            ports,
            permission,
            store.clone(),
        ));
        RecoveryActionUsecase::new(
            store.clone(),
            store.clone(),
            executor,
            store.generation_id().to_string(),
        )
    }

    async fn f05_commit_mutations(
        store: &Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        identity: &str,
        mutations: Vec<LocalStateMutation>,
    ) {
        let result = store
            .commit_batch(LocalAtomicBatch {
                commit_id: CommitIdentity::parse(identity).unwrap(),
                idempotency: IdempotencyBinding {
                    generation_id: store.generation_id().to_string(),
                    operation_kind: CommitOperationKind::Recovery,
                    idempotency_key: identity.to_string(),
                    payload_hash:
                        crate::usecase::agent_session::operation::OperationBindingAuthority::digest(
                            store.as_ref(),
                            identity.as_bytes(),
                        ),
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: mutations,
            })
            .await
            .unwrap();
        assert!(matches!(
            result,
            CommitBatchResult::Committed(_) | CommitBatchResult::Replayed(_)
        ));
    }

    fn f05_pending_obligation(
        obligation_id: &str,
        owner: &str,
        record: &ObligationRecord,
        expected: RevisionGuard,
        revision: Revision,
    ) -> LocalStateMutation {
        LocalStateMutation::Obligation(ObligationMutation {
            obligation_id: obligation_id.to_string(),
            record: record.clone(),
            pending: Some(PendingIndexEntry {
                ordered_key: format!("f05:{obligation_id}"),
                owner: owner.to_string(),
                partition: PendingPartition::Owner,
                shutdown_plan: None,
            }),
            expected,
            revision,
        })
    }

    fn f05_stale_readback_fixture(record: &ObligationRecord) -> (String, ObligationRecord) {
        let mut record = record.clone();
        let obligation_id = match &mut record {
            ObligationRecord::StopInterrupt {
                session_id,
                turn_id,
                ..
            } => {
                session_id.push_str("-stale");
                stop_readback_obligation_id(session_id, turn_id)
            }
            ObligationRecord::SessionClose {
                obligation_id: stored,
                session_id,
                ..
            } => {
                session_id.push_str("-stale");
                let obligation_id = session_close_readback_obligation_id(session_id);
                *stored = obligation_id.clone();
                obligation_id
            }
            ObligationRecord::BackendSessionRecovery {
                session_id,
                recovery_id,
                ..
            } => {
                session_id.push_str("-stale");
                format!("backend-recovery:{session_id}:{recovery_id}")
            }
            ObligationRecord::Send {
                obligation_id: stored,
                operation_id,
                kind,
                ..
            } => {
                operation_id.push_str("-stale");
                let obligation_id = match kind {
                    SendObligationKindRecord::ProviderEstablish => {
                        format!("{operation_id}.establish")
                    }
                    SendObligationKindRecord::TurnExecution => format!("{operation_id}.exec"),
                };
                *stored = obligation_id.clone();
                obligation_id
            }
            _ => panic!("F05 stale fixture must remain an executable typed readback"),
        };
        (obligation_id, record)
    }

    async fn f05_read_again_request(
        usecase: &RecoveryActionUsecase,
        owner: &str,
        obligation_id: &str,
    ) -> RecoveryActionRequest {
        let page = usecase
            .pending(PendingRecoveryQuery {
                limit: 16,
                partition: None,
                owner: Some(owner.to_string()),
                shutdown_plan: None,
                cursor: None,
            })
            .await
            .unwrap();
        let entry = page
            .entries
            .iter()
            .find(|entry| entry.obligation_id == obligation_id)
            .expect("seeded F05 obligation");
        let identity = entry
            .action_identities
            .iter()
            .find(|identity| identity.action == RecoveryActionKind::ReadAgain)
            .expect("production executor advertises typed ReadAgain");
        RecoveryActionRequest {
            action_id: identity.action_id.clone(),
            obligation_id: obligation_id.to_string(),
            origin_revision: identity.origin_revision,
            action: RecoveryActionKind::ReadAgain,
        }
    }

    fn f05_operation_seed(record: &ObligationRecord) -> Option<LocalStateMutation> {
        let authentication = RecordAuthentication {
            principal_mac: [7; 32],
            binding_hmac: [8; 32],
        };
        let (kind, operation_id, receipt, value) = match record {
            ObligationRecord::StopInterrupt {
                operation_id,
                session_id,
                turn_id,
                expected_revision,
                ..
            } => (
                OperationKind::Stop,
                operation_id,
                OperationReceiptRecord::Stop {
                    operation_id: operation_id.clone(),
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    accepted_revision: *expected_revision,
                    authentication,
                },
                OperationStatusValue::ReconciliationRequired {
                    failure: unused_failure(),
                },
            ),
            ObligationRecord::SessionClose {
                operation_id,
                session_id,
                action,
                ..
            } => (
                OperationKind::SessionLifecycle,
                operation_id,
                OperationReceiptRecord::SessionLifecycle {
                    operation_id: operation_id.clone(),
                    session_id: session_id.clone(),
                    action: action.clone(),
                    first_accepted_revision: 0,
                    commit_operation_kind: CommitOperationKind::SessionLifecycle,
                    authentication,
                },
                OperationStatusValue::Accepted,
            ),
            ObligationRecord::Send {
                operation_id,
                session_id,
                turn_id,
                reserved_turn_id,
                ..
            } => (
                OperationKind::Send,
                operation_id,
                OperationReceiptRecord::Send {
                    operation_id: operation_id.clone(),
                    session_id: session_id.clone(),
                    input_ref: format!("{operation_id}.input"),
                    disposition: SendDisposition::StartedTurn {
                        turn_id: turn_id
                            .as_ref()
                            .or(reserved_turn_id.as_ref())
                            .expect("F05 send fixture has a durable turn")
                            .clone(),
                    },
                    authentication,
                },
                OperationStatusValue::ReconciliationRequired {
                    failure: unused_failure(),
                },
            ),
            ObligationRecord::BackendSessionRecovery { .. } => return None,
            _ => panic!("F05 fixture must remain a production typed readback"),
        };
        Some(LocalStateMutation::OperationRecord(
            OperationRecordMutation {
                kind,
                operation_id: operation_id.clone(),
                receipt,
                latest_status: OperationStatusRecord {
                    kind,
                    migration_quit: false,
                    value,
                },
                expected: RevisionGuard::Absent,
                revision: Revision::new(0).unwrap(),
            },
        ))
    }

    fn f05_terminal_evidence(record: &ObligationRecord) -> Option<LocalStateMutation> {
        let (session_id, turn_id) = match record {
            ObligationRecord::StopInterrupt {
                session_id,
                turn_id,
                ..
            }
            | ObligationRecord::Send {
                session_id,
                turn_id: Some(turn_id),
                ..
            } => (session_id, turn_id),
            _ => return None,
        };
        Some(LocalStateMutation::TerminalRecord(TerminalRecordMutation {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            terminal_identity: format!("f05-{session_id}-terminal-winner"),
            result: TerminalResultRecord::AgentTurn {
                kind: AgentTerminalKind::Completed,
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                message_id: format!("f05-{session_id}-assistant"),
                streaming_final_sequence: 0,
                completed_at_bits: 1.0_f64.to_bits(),
                result: AgentTurnTerminalResultRecord::Current(TurnResult::Completed {
                    stop_reason: None,
                    token_usage: None,
                }),
            },
            participant_digest: [9; 32],
        }))
    }

    async fn f05_seed_backend_readback(
        store: &Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        session_store: &Arc<crate::usecase::agent_session::session::SessionStore>,
        data_dir: &std::path::Path,
        session_id: &str,
        recovery_id: &str,
    ) -> (ObligationRecord, Revision) {
        let session = crate::usecase::agent_session::session::build_new_session_with_id(
            session_id.to_string(),
            "/tmp/f05-backend-readback",
            Some("codex".to_string()),
            crate::domain::agent_session::PermissionMode::Ask,
            None,
            false,
            false,
            None,
        );
        session_store
            .save_full_session_for_migration_or_restore(data_dir, &session)
            .unwrap();
        session_store
            .begin_backend_session_recovery(
                data_dir,
                session_id,
                recovery_id,
                BackendSessionRecoveryReason::BackendSessionLost,
            )
            .unwrap();

        let current = store
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: session_id.to_string(),
            })
            .await
            .unwrap();
        let LocalEventQueryResult::SessionProjectionByIdentity(Some(current)) = current else {
            panic!("F05 backend owner projection missing after reservation");
        };
        let codec =
            crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1;
        let mut projection =
            crate::usecase::agent_session::session::AgentSessionProjectionCodec::decode(
                &codec,
                &current.projection,
            )
            .unwrap();
        projection.meta.agent_session_id = Some("f05-backend-provider-after-effect".to_string());
        projection.meta.provider_session_generation = 1;
        projection.meta.context_reinjection_generation = Some(1);
        let projection =
            crate::usecase::agent_session::session::AgentSessionProjectionCodec::encode(
                &codec,
                &projection,
            )
            .unwrap();
        let evidence_revision = current.revision.next().unwrap();
        f05_commit_mutations(
            store,
            "f05-backend-durable-owner-evidence",
            vec![LocalStateMutation::SessionProjection(
                SessionProjectionMutation {
                    session_id: session_id.to_string(),
                    projection,
                    expected: RevisionGuard::Expected(current.revision),
                    revision: evidence_revision,
                },
            )],
        )
        .await;

        let obligation_id = format!("backend-recovery:{session_id}:{recovery_id}");
        let source = store
            .query(LocalEventQuery::ObligationByIdentity { obligation_id })
            .await
            .unwrap();
        let LocalEventQueryResult::ObligationByIdentity(Some(source)) = source else {
            panic!("F05 backend source obligation missing after reservation");
        };
        (source.record, evidence_revision)
    }

    async fn f05_assert_owner_witness(
        store: &Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
        label: &str,
        record: &ObligationRecord,
        backend_evidence_revision: Option<Revision>,
        completed: bool,
    ) {
        let (kind, operation_id) = match record {
            ObligationRecord::StopInterrupt { operation_id, .. } => {
                (Some(OperationKind::Stop), Some(operation_id))
            }
            ObligationRecord::SessionClose { operation_id, .. } => {
                (Some(OperationKind::SessionLifecycle), Some(operation_id))
            }
            ObligationRecord::Send { operation_id, .. } => {
                (Some(OperationKind::Send), Some(operation_id))
            }
            ObligationRecord::BackendSessionRecovery { .. } => (None, None),
            _ => panic!("F05 witness must remain a production typed readback"),
        };
        if let (Some(kind), Some(operation_id)) = (kind, operation_id) {
            let operation = store
                .query(LocalEventQuery::OperationByIdentity {
                    kind,
                    operation_id: operation_id.clone(),
                })
                .await
                .unwrap();
            let LocalEventQueryResult::OperationByIdentity(Some(operation)) = operation else {
                panic!("F05 {label} owner operation is missing");
            };
            assert_eq!(operation.revision.value(), i64::from(completed));
            match (label, completed, &operation.latest_status.value) {
                ("stop", false, OperationStatusValue::ReconciliationRequired { .. })
                | ("session-close", false, OperationStatusValue::Accepted)
                | ("send", false, OperationStatusValue::ReconciliationRequired { .. })
                | (
                    "stop",
                    true,
                    OperationStatusValue::StopCompleted {
                        resolution: StopResolution::Superseded,
                    },
                )
                | ("session-close", true, OperationStatusValue::Completed)
                | ("send", true, OperationStatusValue::Terminal { .. }) => {}
                other => panic!("unexpected F05 {label} owner operation witness: {other:?}"),
            }
            if label == "stop" {
                let resolution = store
                    .query(LocalEventQuery::StopResolutionByOperation {
                        stop_operation_id: operation_id.clone(),
                    })
                    .await
                    .unwrap();
                assert!(if completed {
                    matches!(
                        resolution,
                        LocalEventQueryResult::StopResolutionByOperation(Some(ref value))
                            if value.resolution == StopResolutionKind::Superseded
                    )
                } else {
                    matches!(
                        resolution,
                        LocalEventQueryResult::StopResolutionByOperation(None)
                    )
                });
            }
            return;
        }

        let ObligationRecord::BackendSessionRecovery { session_id, .. } = record else {
            unreachable!();
        };
        let projection = store
            .query(LocalEventQuery::SessionProjectionByIdentity {
                session_id: session_id.clone(),
            })
            .await
            .unwrap();
        let LocalEventQueryResult::SessionProjectionByIdentity(Some(projection)) = projection
        else {
            panic!("F05 backend owner projection is missing");
        };
        let evidence_revision =
            backend_evidence_revision.expect("F05 backend evidence revision is bound");
        assert_eq!(
            projection.revision,
            if completed {
                evidence_revision.next().unwrap()
            } else {
                evidence_revision
            }
        );
        let crate::domain::local_event::SessionProjectionRecord::AgentSession(projection) =
            projection.projection
        else {
            panic!("F05 backend owner projection has the wrong semantic family");
        };
        assert_eq!(projection.meta.provider_session_generation, 1);
        assert_eq!(projection.meta.context_reinjection_generation, Some(1));
        assert_eq!(
            projection.meta.pending_recovery_message.is_some(),
            completed
        );
        assert_eq!(
            projection.meta.recovery_publication_snapshot.is_none(),
            completed
        );
    }

    #[tokio::test]
    async fn f05_production_readbacks_are_atomic_across_loss_restart_replay_and_stale_revision() {
        let stop_obligation_id = stop_readback_obligation_id("f05-stop-session", "7");
        let session_close_obligation_id = session_close_readback_obligation_id("f05-close-session");
        let fixtures = [
            (
                "stop",
                stop_obligation_id,
                ObligationRecord::StopInterrupt {
                    operation_id: "f05-stop-operation".to_string(),
                    session_id: "f05-stop-session".to_string(),
                    turn_id: "7".to_string(),
                    expected_revision: 0,
                    deadline_ms: 0,
                    state: ObligationStateRecord::ReconciliationRequired,
                },
            ),
            (
                "session-close",
                session_close_obligation_id.clone(),
                ObligationRecord::SessionClose {
                    obligation_id: session_close_obligation_id,
                    operation_id: "f05-close-operation".to_string(),
                    session_id: "f05-close-session".to_string(),
                    action: SessionLifecycleRecordAction::Close,
                    state: ObligationStateRecord::ReconciliationRequired,
                },
            ),
            (
                "backend-recovery",
                "backend-recovery:f05-backend-session:f05-recovery".to_string(),
                ObligationRecord::BackendSessionRecovery {
                    session_id: "f05-backend-session".to_string(),
                    recovery_id: "f05-recovery".to_string(),
                    detail: None,
                    state: ObligationStateRecord::EffectReserved,
                },
            ),
            (
                "send",
                "f05-send-operation.exec".to_string(),
                ObligationRecord::Send {
                    obligation_id: "f05-send-operation.exec".to_string(),
                    operation_id: "f05-send-operation".to_string(),
                    session_id: "f05-send-session".to_string(),
                    kind: SendObligationKindRecord::TurnExecution,
                    disposition: SendObligationDispositionRecord::StartedTurn,
                    human_message_id: Some("human-1".to_string()),
                    assistant_message_id: None,
                    reserved_turn_id: Some("7".to_string()),
                    turn_id: Some("7".to_string()),
                    dependency_obligation_ids: Vec::new(),
                    canonical_payload: "payload".to_string(),
                    state: ObligationStateRecord::EffectReserved,
                },
            ),
        ];

        for (label, obligation_id, seeded_record) in fixtures {
            let directory = tempfile::tempdir().unwrap();
            let calls = Arc::new(AtomicUsize::new(0));
            let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                    directory.path().to_path_buf(),
                ),
            )
            .unwrap();
            let session_store = f05_session_store(&store);
            let mut record = seeded_record;
            let backend_evidence_revision = if let ObligationRecord::BackendSessionRecovery {
                session_id,
                recovery_id,
                ..
            } = &record
            {
                let (durable_record, evidence_revision) = f05_seed_backend_readback(
                    &store,
                    &session_store,
                    directory.path(),
                    session_id,
                    recovery_id,
                )
                .await;
                record = durable_record;
                Some(evidence_revision)
            } else {
                let owner = match &record {
                    ObligationRecord::StopInterrupt { session_id, .. }
                    | ObligationRecord::SessionClose { session_id, .. }
                    | ObligationRecord::Send { session_id, .. } => session_id,
                    _ => unreachable!(),
                };
                let mut mutations = Vec::new();
                if let ObligationRecord::SessionClose { .. } = &record {
                    mutations.push(LocalStateMutation::SessionProjection(
                        SessionProjectionMutation {
                            session_id: owner.clone(),
                            projection: f05_session_projection(
                                owner,
                                crate::usecase::agent_session::session::SessionState::Closed,
                            ),
                            expected: RevisionGuard::Absent,
                            revision: Revision::new(0).unwrap(),
                        },
                    ));
                }
                mutations.push(f05_operation_seed(&record).unwrap());
                if let Some(terminal) = f05_terminal_evidence(&record) {
                    mutations.push(terminal);
                }
                mutations.push(f05_pending_obligation(
                    &obligation_id,
                    owner,
                    &record,
                    RevisionGuard::Absent,
                    Revision::new(0).unwrap(),
                ));
                f05_commit_mutations(&store, &format!("f05-{label}-seed"), mutations).await;
                None
            };
            let owner = match &record {
                ObligationRecord::StopInterrupt { session_id, .. }
                | ObligationRecord::SessionClose { session_id, .. }
                | ObligationRecord::BackendSessionRecovery { session_id, .. }
                | ObligationRecord::Send { session_id, .. } => session_id.clone(),
                _ => unreachable!(),
            };

            let first = f05_production_recovery_usecase(
                &store,
                session_store,
                directory.path(),
                calls.clone(),
                label,
                obligation_id.clone(),
                F05FinishFault::FailBeforeCommit,
            );
            let request = f05_read_again_request(&first, &owner, &obligation_id).await;
            let first_result = first.request(request.clone()).await;
            assert!(
                matches!(
                    first_result,
                    Err(RecoveryActionError::StorageUnavailable { .. })
                ),
                "{label} fail-before-commit result: {first_result:?}"
            );
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(matches!(
                first.get_action_status(&request.action_id).await.unwrap(),
                RecoveryActionStatus::InProgress { .. }
            ));
            f05_assert_owner_witness(&store, label, &record, backend_evidence_revision, false)
                .await;
            let before = store
                .query(LocalEventQuery::ObligationByIdentity {
                    obligation_id: obligation_id.to_string(),
                })
                .await
                .unwrap();
            assert!(matches!(
                before,
                LocalEventQueryResult::ObligationByIdentity(Some(ref obligation))
                    if obligation.revision.value() == 1 && obligation.pending.is_some()
            ));
            drop(first);
            drop(store);

            let reopened = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                    directory.path().to_path_buf(),
                ),
            )
            .unwrap();
            let after_restart = f05_production_recovery_usecase(
                &reopened,
                f05_session_store(&reopened),
                directory.path(),
                calls.clone(),
                label,
                obligation_id.clone(),
                F05FinishFault::DropReply,
            );
            assert_eq!(
                after_restart.request(request.clone()).await.unwrap(),
                RecoveryActionOutcome::ActionOutcomeUnknown {
                    action_id: request.action_id.clone(),
                }
            );
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            assert!(matches!(
                after_restart
                    .get_action_status(&request.action_id)
                    .await
                    .unwrap(),
                RecoveryActionStatus::Completed { .. }
            ));
            let action = reopened
                .query(LocalEventQuery::RecoveryActionByIdentity {
                    action_id: request.action_id.clone(),
                })
                .await
                .unwrap();
            assert!(matches!(
                action,
                LocalEventQueryResult::RecoveryActionByIdentity(Some(ref action))
                    if action.completed.is_some() && action.revision.value() == 1
            ));
            let obligation = reopened
                .query(LocalEventQuery::ObligationByIdentity {
                    obligation_id: obligation_id.to_string(),
                })
                .await
                .unwrap();
            let LocalEventQueryResult::ObligationByIdentity(Some(obligation)) = obligation else {
                panic!("F05 obligation missing after finish");
            };
            assert_eq!(obligation.revision.value(), 2);
            assert!(
                obligation.pending.is_none(),
                "{label} source obligation must settle in the atomic finish batch"
            );
            assert!(matches!(
                obligation.record,
                ObligationRecord::RecoveryTransition {
                    recovery_action: crate::domain::local_event::ObligationRecoveryActionRecord {
                        state: ObligationStateRecord::Completed,
                        classification: Some(RecoveryResultClassification::Succeeded),
                        ..
                    },
                    ..
                }
            ));
            f05_assert_owner_witness(&reopened, label, &record, backend_evidence_revision, true)
                .await;
            drop(after_restart);
            drop(reopened);

            let replay_store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
                crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                    directory.path().to_path_buf(),
                ),
            )
            .unwrap();
            let replay = f05_production_recovery_usecase(
                &replay_store,
                f05_session_store(&replay_store),
                directory.path(),
                calls.clone(),
                label,
                obligation_id.clone(),
                F05FinishFault::None,
            );
            let completed = replay.request(request.clone()).await.unwrap();
            assert!(matches!(completed, RecoveryActionOutcome::Completed { .. }));
            assert_eq!(replay.request(request).await.unwrap(), completed);
            assert_eq!(calls.load(Ordering::SeqCst), 2);

            let (stale_obligation_id, stale_record) = f05_stale_readback_fixture(&record);
            let stale_owner = format!("{owner}-stale");
            f05_commit_mutations(
                &replay_store,
                &format!("f05-{label}-stale-seed"),
                vec![f05_pending_obligation(
                    &stale_obligation_id,
                    &stale_owner,
                    &stale_record,
                    RevisionGuard::Absent,
                    Revision::new(0).unwrap(),
                )],
            )
            .await;
            let stale = f05_read_again_request(&replay, &stale_owner, &stale_obligation_id).await;
            f05_commit_mutations(
                &replay_store,
                &format!("f05-{label}-stale-advance"),
                vec![f05_pending_obligation(
                    &stale_obligation_id,
                    &stale_owner,
                    &stale_record,
                    RevisionGuard::Expected(Revision::new(0).unwrap()),
                    Revision::new(1).unwrap(),
                )],
            )
            .await;
            assert_eq!(
                replay.request(stale.clone()).await.unwrap(),
                RecoveryActionOutcome::Rejected {
                    action_id: stale.action_id,
                    rejection: RecoveryActionRejection::RevisionConflict {
                        current_revision: 1,
                    },
                }
            );
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        }
    }

    #[test]
    fn production_handoff_fence_rejects_revision_and_owner_drift() {
        let request = handoff_request();
        assert!(recovery_handoff_target_matches(
            &request,
            &handoff_target(7, Some("session-1")),
        ));
        assert!(!recovery_handoff_target_matches(
            &request,
            &handoff_target(8, Some("session-1")),
        ));
        assert!(!recovery_handoff_target_matches(
            &request,
            &handoff_target(7, Some("session-2")),
        ));
        assert!(!recovery_handoff_target_matches(
            &request,
            &handoff_target(7, None),
        ));
    }

    #[test]
    fn permission_retry_material_preserves_exact_allow_payload() {
        let record = ObligationRecord::PermissionResponse {
            operation_id: "permission-operation-1".to_string(),
            effect_identity: "permission-response:permission-operation-1".to_string(),
            session_id: "session-1".to_string(),
            turn_id: "7".to_string(),
            response: PermissionResponse {
                request_id: "permission-1".to_string(),
                decision: PermissionResponseDecision::Allow {
                    updated_input: Some(
                        crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                            r#"{"command":"echo exact","flag":true}"#.to_string(),
                        ),
                    ),
                    answers: Some(
                        crate::domain::agent_session::value_objects::JsonPayload::new_unchecked(
                            r#"{"choice":["a","b"]}"#.to_string(),
                        ),
                    ),
                },
            },
            owner_access: true,
            from_runtime_state: true,
            state: ObligationStateRecord::Pending,
        };
        let (session_id, response) = permission_response_retry_material(&record).unwrap();
        assert_eq!(session_id, "session-1");
        assert_eq!(response.request_id, "permission-1");
        let PermissionResponseDecision::Allow {
            updated_input,
            answers,
        } = response.decision
        else {
            panic!("exact allow decision changed kind");
        };
        assert_eq!(
            updated_input.unwrap().as_str(),
            r#"{"command":"echo exact","flag":true}"#
        );
        assert_eq!(answers.unwrap().as_str(), r#"{"choice":["a","b"]}"#);
    }

    #[test]
    fn effect_reserved_or_missing_permission_payload_is_not_retry_material() {
        for record in [
            ObligationRecord::PermissionResponse {
                operation_id: "permission-operation-1".to_string(),
                effect_identity: "permission-response:permission-operation-1".to_string(),
                session_id: "session-1".to_string(),
                turn_id: "7".to_string(),
                response: PermissionResponse {
                    request_id: "permission-1".to_string(),
                    decision: PermissionResponseDecision::Deny {
                        message: Some("no".to_string()),
                    },
                },
                owner_access: true,
                from_runtime_state: true,
                state: ObligationStateRecord::EffectReserved,
            },
            ObligationRecord::PermissionResponse {
                operation_id: "permission-operation-1".to_string(),
                effect_identity: "permission-response:permission-operation-1".to_string(),
                session_id: "session-1".to_string(),
                turn_id: "7".to_string(),
                response: PermissionResponse {
                    request_id: "permission-1".to_string(),
                    decision: PermissionResponseDecision::Deny { message: None },
                },
                owner_access: false,
                from_runtime_state: true,
                state: ObligationStateRecord::Pending,
            },
        ] {
            assert!(permission_response_retry_material(&record).is_err());
        }
    }
}
