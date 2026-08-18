use std::sync::Arc;

use crate::domain::local_event::CommitOperationKind;
use crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent;
use crate::domain::workflow::entities::workflow_execution::{
    NodeRestartMode, ProviderStopRejection, TransitionOutcome,
    WorkflowExecution as DomainWorkflowExecution,
};
use crate::domain::workflow::services::secret_masker as workflow_secret_masker;
use crate::domain::workflow::{NodeCompletionSignal, WorkflowError, WorkflowEvent};

use super::command::{ApprovalCommand, RetryNodeCommand, SubmitOutputCommand};
use super::output_submission as submission;
use super::runtime_driver::{self, NodeOutcome};
use super::runtime_error::WorkflowRuntimeError;
use super::runtime_snapshot::RuntimeCommitSnapshot;

pub(crate) struct WorkflowControlPlaneCommit {
    pub(crate) operation_kind: CommitOperationKind,
    pub(crate) execution_id: String,
    pub(crate) before: DomainWorkflowExecution,
    pub(crate) after: DomainWorkflowExecution,
    pub(crate) workflow_events: Vec<WorkflowEvent>,
    pub(crate) provider_events: Vec<ScopedProviderLifecycleEvent>,
}

#[async_trait::async_trait]
pub(crate) trait WorkflowControlPlaneGateway: Send + Sync {
    fn current_timestamp(&self) -> f64;

    fn new_node_execution_id(&self) -> String;

    async fn resolve_workflow_execution_id(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<String>, WorkflowError>;

    async fn load_active_execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<DomainWorkflowExecution>, WorkflowError>;

    async fn recover_active_executions(&self) -> Result<(), WorkflowError>;

    async fn load_persisted_events(
        &self,
        execution_id: &str,
    ) -> Result<Vec<WorkflowEvent>, WorkflowError>;

    fn configured_secret_values(&self) -> Vec<String>;

    fn approval_auto_approve_enabled(&self) -> bool {
        false
    }

    async fn commit_control_plane(
        &self,
        commit: WorkflowControlPlaneCommit,
    ) -> Result<RuntimeCommitSnapshot, WorkflowError>;

    async fn finish_control_plane_commit(
        &self,
        worktree_path: &str,
        snapshot: &RuntimeCommitSnapshot,
        outcome: Option<NodeOutcome>,
    ) -> Result<(), WorkflowError>;
}

#[derive(Clone)]
pub(crate) struct WorkflowControlPlaneUsecase {
    runtime: Arc<dyn WorkflowControlPlaneGateway>,
}

impl WorkflowControlPlaneUsecase {
    fn node_execution_id_source(&self) -> impl FnMut() -> String {
        let runtime = Arc::clone(&self.runtime);
        move || runtime.new_node_execution_id()
    }

    pub(crate) fn new(runtime: Arc<dyn WorkflowControlPlaneGateway>) -> Self {
        Self { runtime }
    }

    pub(crate) async fn resolve_approval(
        &self,
        command: ApprovalCommand,
    ) -> Result<(), WorkflowError> {
        super::command::WorkflowRuntimeCommandPreflight.validate_approval(&command)?;
        super::command::retry_control_plane_conflicts(|| {
            self.resolve_approval_once(command.clone())
        })
        .await
    }

    async fn resolve_approval_once(&self, command: ApprovalCommand) -> Result<(), WorkflowError> {
        let current = self
            .runtime
            .load_active_execution(&command.execution_id)
            .await?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "Workflow execution not found: {}",
                    command.execution_id
                ))
            })?;
        let target = match current.resolve_approval_attempt_target(
            &command.node_name,
            command.node_execution_id.as_deref(),
        ) {
            Ok(target) => target,
            Err(error) => {
                let events = self
                    .runtime
                    .load_persisted_events(&command.execution_id)
                    .await?;
                if approval_was_persisted(
                    &events,
                    &command.node_name,
                    command.node_execution_id.as_deref(),
                ) {
                    return Ok(());
                }
                return Err(error);
            }
        };
        let timestamp = self.runtime.current_timestamp();
        let event_comment = command.comment.map(|comment| {
            workflow_secret_masker::mask_sensitive_text(
                &comment,
                &self.runtime.configured_secret_values(),
            )
        });
        let mut candidate = current.clone();
        let mut new_id = self.node_execution_id_source();
        let applied =
            candidate.apply_approval(&target.node_execution_id, &mut new_id, timestamp)?;
        let mut workflow_events = vec![WorkflowEvent::ApprovalResolved {
            execution_id: candidate.id.clone(),
            node_execution_id: target.node_execution_id.clone(),
            node_name: target.node_name.clone(),
            comment: event_comment,
            timestamp,
        }];
        workflow_events.extend(applied.events);
        let outcome = runtime_driver::node_outcome_from_advance(&candidate, applied.decision)
            .map_err(runtime_error_to_workflow_error)?;
        let worktree_path = current.worktree_path.clone();
        let snapshot = self
            .runtime
            .commit_control_plane(WorkflowControlPlaneCommit {
                operation_kind: CommitOperationKind::UserMutation,
                execution_id: command.execution_id,
                before: current,
                after: candidate,
                workflow_events,
                provider_events: Vec::new(),
            })
            .await?;
        self.runtime
            .finish_control_plane_commit(&worktree_path, &snapshot, Some(outcome))
            .await?;
        self.auto_approve_if_needed(&snapshot).await
    }

    async fn auto_approve_if_needed(
        &self,
        snapshot: &RuntimeCommitSnapshot,
    ) -> Result<(), WorkflowError> {
        if !self.runtime.approval_auto_approve_enabled() {
            return Ok(());
        }
        let Some(target) = snapshot
			.node_executions
			.iter()
			.find(|attempt| {
				attempt.status
					== crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::WaitingApproval
			})
		else {
			return Ok(());
		};
        Box::pin(self.resolve_approval(ApprovalCommand {
            execution_id: snapshot.execution_id.clone(),
            node_name: target.node_name.clone(),
            node_execution_id: Some(target.id.clone()),
            comment: None,
        }))
        .await
    }

    pub(crate) async fn submit_output(
        &self,
        command: SubmitOutputCommand,
    ) -> Result<(), WorkflowError> {
        super::command::retry_control_plane_conflicts(|| self.submit_output_once(command.clone()))
            .await
    }

    async fn submit_output_once(&self, command: SubmitOutputCommand) -> Result<(), WorkflowError> {
        submission::validate_submit_output_request(&command.node_execution_id)
            .map_err(runtime_error_to_workflow_error)?;

        let execution_id = self
            .runtime
            .resolve_workflow_execution_id(&command.node_execution_id)
            .await?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "Node execution not found: {}",
                    command.node_execution_id
                ))
            })?;

        let current = self
            .runtime
            .load_active_execution(&execution_id)
            .await?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "Active node execution not found: {}",
                    command.node_execution_id
                ))
            })?;

        let target = submission::validate_submit_target_context(
            &current,
            &execution_id,
            &command.node_execution_id,
        )
        .map_err(runtime_error_to_workflow_error)?;
        let validated_artifact = if let Some(artifact) = command.artifact {
            submission::validate_artifact_contract_for_workflow(
                &current.workflow,
                &target.node_name,
                &artifact.contract,
            )
            .map_err(runtime_error_to_workflow_error)?;
            let validated = submission::validate_submission_output_with_secrets(
                &current.workflow,
                &artifact.contract,
                artifact.value,
                &self.runtime.configured_secret_values(),
            )
            .map_err(runtime_error_to_workflow_error)?;
            Some((artifact.contract, validated))
        } else {
            None
        };
        let timestamp = self.runtime.current_timestamp();
        let mut candidate = current.clone();
        let submit_signal_applied = match candidate.record_node_completion_signal(
            &command.node_execution_id,
            NodeCompletionSignal::Submit,
            timestamp,
        ) {
            TransitionOutcome::Applied => true,
            TransitionOutcome::AlreadyApplied => false,
            _ => {
                return Err(WorkflowError::invalid_state(format!(
                    "node execution '{}' cannot accept Submit",
                    command.node_execution_id
                )))
            }
        };
        if !submit_signal_applied && validated_artifact.is_none() {
            return Ok(());
        }
        let mut events = Vec::new();
        if submit_signal_applied {
            events.push(WorkflowEvent::NodeSubmitReceived {
                execution_id: execution_id.clone(),
                node_execution_id: command.node_execution_id.clone(),
                timestamp,
            });
        }
        if let Some((contract, validated)) = validated_artifact {
            if candidate.apply_submitted_output(
                target.node_name.clone(),
                &command.node_execution_id,
                target.attempt,
                target.session_id,
                contract.clone(),
                validated.artifact.clone(),
                validated.result,
                timestamp,
            )
                != crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
            {
                return Err(WorkflowError::invalid_state(format!(
                    "node execution '{}' disappeared during Submit",
                    command.node_execution_id
                )));
            }
            events.push(submission::artifact_produced_event(
                &execution_id,
                &command.node_execution_id,
                &target.node_name,
                contract,
                validated.artifact,
                None,
                None,
                timestamp,
            ));
        }
        let outcome = if submit_signal_applied {
            let mut new_id = self.node_execution_id_source();
            let (outcome, handshake_events) = apply_completion_handshake(
                &mut candidate,
                &command.node_execution_id,
                &mut new_id,
                timestamp,
            )?;
            events.extend(handshake_events);
            outcome
        } else {
            None
        };
        let worktree_path = current.worktree_path.clone();
        let snapshot = self
            .runtime
            .commit_control_plane(WorkflowControlPlaneCommit {
                operation_kind: CommitOperationKind::UserMutation,
                execution_id,
                before: current,
                after: candidate,
                workflow_events: events,
                provider_events: Vec::new(),
            })
            .await?;
        self.runtime
            .finish_control_plane_commit(&worktree_path, &snapshot, outcome)
            .await?;
        self.auto_approve_if_needed(&snapshot).await
    }

    pub(crate) async fn retry_node(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
        super::command::retry_control_plane_conflicts(|| self.retry_node_once(command.clone()))
            .await
    }

    async fn retry_node_once(&self, command: RetryNodeCommand) -> Result<(), WorkflowError> {
        let current = self
            .runtime
            .load_active_execution(&command.execution_id)
            .await?
            .ok_or_else(|| {
                WorkflowError::NotFound(format!(
                    "Workflow execution not found: {}",
                    command.execution_id
                ))
            })?;
        let timestamp = self.runtime.current_timestamp();
        let mut candidate = current.clone();
        let restarted = candidate
            .restart_node_attempt_at(
                &command.node_execution_id,
                self.runtime.new_node_execution_id(),
                timestamp,
                NodeRestartMode::ExplicitRetry,
            )
            .ok_or_else(|| {
                WorkflowError::invalid_state(format!(
                    "node execution '{}' is not retryable",
                    command.node_execution_id
                ))
            })?;
        let events = vec![
            WorkflowEvent::NodeRetryRequested {
                execution_id: command.execution_id.clone(),
                node_execution_id: command.node_execution_id,
                timestamp,
            },
            WorkflowEvent::NodeStarted {
                execution_id: command.execution_id.clone(),
                node_execution_id: restarted.attempt.id.clone(),
                node_name: restarted.attempt.node_name.clone(),
                kind: restarted.attempt.kind,
                attempt: restarted.attempt.attempt,
                parent: restarted.attempt.parent.clone(),
                timestamp,
            },
        ];
        let worktree_path = current.worktree_path.clone();
        let snapshot = self
            .runtime
            .commit_control_plane(WorkflowControlPlaneCommit {
                operation_kind: CommitOperationKind::UserMutation,
                execution_id: command.execution_id,
                before: current,
                after: candidate,
                workflow_events: events,
                provider_events: Vec::new(),
            })
            .await?;
        self.runtime
            .finish_control_plane_commit(
                &worktree_path,
                &snapshot,
                Some(NodeOutcome::StartLeaves(
                    snapshot.clone(),
                    vec![restarted.leaf],
                )),
            )
            .await?;
        Ok(())
    }

    pub(crate) async fn record_provider_stop(
        &self,
        command: crate::usecase::provider_lifecycle::ProviderWorkflowStopCommand,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), WorkflowError> {
        super::command::retry_control_plane_conflicts(|| {
            self.record_provider_stop_once(command.clone(), lifecycle_events.clone())
        })
        .await
    }

    async fn record_provider_stop_once(
        &self,
        command: crate::usecase::provider_lifecycle::ProviderWorkflowStopCommand,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), WorkflowError> {
        let mut active = self
            .runtime
            .load_active_execution(&command.workflow_execution_id)
            .await?;
        if active.is_none() {
            self.runtime.recover_active_executions().await?;
            active = self
                .runtime
                .load_active_execution(&command.workflow_execution_id)
                .await?;
        }
        let Some(current) = active else {
            if lifecycle_events.is_empty() {
                return Ok(());
            }
            return Err(WorkflowError::NotFound(format!(
                "Workflow execution not found: {}",
                command.workflow_execution_id
            )));
        };
        let timestamp = self.runtime.current_timestamp();
        let mut candidate = current.clone();
        let mut workflow_events = match candidate.record_provider_stop(
            &command.node_execution_id,
            &command.agent_session_id,
            timestamp,
        ) {
            Ok(crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied) => {
                vec![WorkflowEvent::NodeStopReceived {
                    execution_id: command.workflow_execution_id.clone(),
                    node_execution_id: command.node_execution_id.clone(),
                    timestamp,
                }]
            }
            Ok(crate::domain::workflow::entities::workflow_execution::TransitionOutcome::AlreadyApplied)
            | Ok(crate::domain::workflow::entities::workflow_execution::TransitionOutcome::NotApplicable) => {
                Vec::new()
            }
            Err(ProviderStopRejection::NodeExecutionNotFound) => {
                return Err(WorkflowError::invalid_state(format!(
                    "node execution '{}' is not part of workflow '{}'",
                    command.node_execution_id, command.workflow_execution_id
                )))
            }
            Err(ProviderStopRejection::SessionDoesNotOwnAttempt) => {
                return Err(WorkflowError::invalid_state(format!(
                    "AgentSession '{}' does not own node execution '{}'",
                    command.agent_session_id, command.node_execution_id
                )))
            }
            _ => {
                return Err(WorkflowError::invalid_state(format!(
                    "node execution '{}' cannot accept Provider Stop",
                    command.node_execution_id
                )))
            }
        };
        if workflow_events.is_empty() && lifecycle_events.is_empty() {
            return Ok(());
        }
        let outcome = if workflow_events.is_empty() {
            None
        } else {
            let mut new_id = self.node_execution_id_source();
            let (outcome, events) = apply_completion_handshake(
                &mut candidate,
                &command.node_execution_id,
                &mut new_id,
                timestamp,
            )?;
            workflow_events.extend(events);
            outcome
        };
        let worktree_path = current.worktree_path.clone();
        let snapshot = self
            .runtime
            .commit_control_plane(WorkflowControlPlaneCommit {
                operation_kind: CommitOperationKind::Workflow,
                execution_id: command.workflow_execution_id,
                before: current,
                after: candidate,
                workflow_events,
                provider_events: lifecycle_events,
            })
            .await?;
        self.runtime
            .finish_control_plane_commit(&worktree_path, &snapshot, outcome)
            .await?;
        self.auto_approve_if_needed(&snapshot).await
    }
}

fn approval_was_persisted(
    events: &[WorkflowEvent],
    node_name: &str,
    node_execution_id: Option<&str>,
) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            WorkflowEvent::ApprovalResolved {
                node_execution_id: persisted_id,
                node_name: persisted_name,
                ..
            } if persisted_name == node_name
                && node_execution_id.is_none_or(|expected| expected == persisted_id)
        )
    })
}

fn apply_completion_handshake(
    execution: &mut DomainWorkflowExecution,
    node_execution_id: &str,
    new_id: &mut dyn FnMut() -> String,
    timestamp: f64,
) -> Result<(Option<NodeOutcome>, Vec<WorkflowEvent>), WorkflowError> {
    let applied =
        execution.apply_node_completion_handshake(node_execution_id, new_id, timestamp)?;
    let outcome = applied
        .advance
        .map(|advance| runtime_driver::node_outcome_from_advance(execution, advance))
        .transpose()
        .map_err(runtime_error_to_workflow_error)?;
    Ok((outcome, applied.events))
}

fn runtime_error_to_workflow_error(error: WorkflowRuntimeError) -> WorkflowError {
    match error {
        WorkflowRuntimeError::InvalidWorkflow(message)
        | WorkflowRuntimeError::ValidationError(message) => WorkflowError::validation(message),
        WorkflowRuntimeError::ExecutionNotFound(message)
        | WorkflowRuntimeError::SessionNotFound(message) => WorkflowError::NotFound(message),
        WorkflowRuntimeError::AlreadyActive(message)
        | WorkflowRuntimeError::InvalidState(message) => WorkflowError::InvalidState(message),
        WorkflowRuntimeError::Conflict(message) => WorkflowError::Conflict(message),
        WorkflowRuntimeError::UnauthorizedWorktree(message) => WorkflowError::validation(message),
        WorkflowRuntimeError::UnauthorizedApprovalTarget(message) => {
            WorkflowError::UnauthorizedApprovalTarget(message)
        }
        WorkflowRuntimeError::SessionStore(message)
        | WorkflowRuntimeError::AgentSession(message) => WorkflowError::external(message),
    }
}
