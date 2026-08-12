use std::sync::Arc;

use crate::domain::local_event::CommitOperationKind;
use crate::domain::provider_lifecycle::ScopedProviderLifecycleEvent;
use crate::domain::workflow::entities::workflow_execution::{
    FanoutChildRuntimeState, NodeRestartMode, ProviderStopRejection, TransitionOutcome,
    WorkflowExecution as DomainWorkflowExecution,
};
use crate::domain::workflow::services::{
    fanout as workflow_fanout, secret_masker as workflow_secret_masker,
    transition as workflow_transition,
};
use crate::domain::workflow::{
    NodeCompletionSignal, WorkflowError, WorkflowEvent, NODE_STATUS_COMPLETED,
};

use super::command::{ApprovalCommand, RetryNodeCommand, SubmitOutputCommand};
use super::output_submission as submission;
use super::runtime_driver::{self, NodeOutcome};
use super::runtime_error::WorkflowRuntimeError;
use super::runtime_events;
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

    async fn finish_retried_fanout_commit(
        &self,
        worktree_path: &str,
        snapshot: &RuntimeCommitSnapshot,
        node_execution_id: &str,
    ) -> Result<(), WorkflowError>;
}

#[derive(Clone)]
pub(crate) struct WorkflowControlPlaneUsecase {
    runtime: Arc<dyn WorkflowControlPlaneGateway>,
}

impl WorkflowControlPlaneUsecase {
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
        let (outcome, workflow_events) = if target.fanout_parent.is_some() {
            apply_fanout_approval(
                &mut candidate,
                &target,
                event_comment,
                self.runtime.new_node_execution_id(),
                timestamp,
            )?
        } else {
            apply_linear_approval(
                &mut candidate,
                &target,
                event_comment,
                self.runtime.new_node_execution_id(),
                timestamp,
            )?
        };
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

        let active = self.runtime.load_active_execution(&execution_id).await?;
        let persisted_events = if active.is_none() {
            Some(self.runtime.load_persisted_events(&execution_id).await?)
        } else {
            None
        };

        let Some(current) = active else {
            let events = persisted_events.unwrap_or_default();
            if submit_was_persisted(&events, &command.node_execution_id) {
                return Ok(());
            }
            return Err(WorkflowError::NotFound(format!(
                "Active node execution not found: {}",
                command.node_execution_id
            )));
        };

        let target = match submission::validate_submit_target_context(
            &current,
            &execution_id,
            &command.node_execution_id,
        ) {
            Ok(target) => target,
            Err(error) => {
                let events = self.runtime.load_persisted_events(&execution_id).await?;
                if submit_was_persisted(&events, &command.node_execution_id) {
                    return Ok(());
                }
                return Err(runtime_error_to_workflow_error(error));
            }
        };
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
        match candidate.record_node_completion_signal(
            &command.node_execution_id,
            NodeCompletionSignal::Submit,
            timestamp,
        ) {
            crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied => {}
            crate::domain::workflow::entities::workflow_execution::TransitionOutcome::AlreadyApplied => {
                return Ok(())
            }
            _ => {
                return Err(WorkflowError::invalid_state(format!(
                    "node execution '{}' cannot accept Submit",
                    command.node_execution_id
                )))
            }
        }
        let mut events = vec![WorkflowEvent::NodeSubmitReceived {
            execution_id: execution_id.clone(),
            node_execution_id: command.node_execution_id.clone(),
            timestamp,
        }];
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
        let (outcome, handshake_events) = apply_completion_handshake(
            &mut candidate,
            &command.node_execution_id,
            self.runtime.new_node_execution_id(),
            timestamp,
        )?;
        events.extend(handshake_events);
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
                fanout_parent: restarted.attempt.fanout_parent.clone(),
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
        if restarted.fanout_child {
            self.runtime
                .finish_retried_fanout_commit(&worktree_path, &snapshot, &restarted.attempt.id)
                .await?;
        } else {
            self.runtime
                .finish_control_plane_commit(
                    &worktree_path,
                    &snapshot,
                    Some(NodeOutcome::RetryCurrentNode(snapshot.clone())),
                )
                .await?;
        }
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
            let (outcome, events) = apply_completion_handshake(
                &mut candidate,
                &command.node_execution_id,
                self.runtime.new_node_execution_id(),
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

fn apply_linear_approval(
    execution: &mut DomainWorkflowExecution,
    target: &crate::domain::workflow::entities::workflow_execution::ApprovalAttemptTarget,
    comment: Option<String>,
    next_node_execution_id: String,
    timestamp: f64,
) -> Result<(NodeOutcome, Vec<WorkflowEvent>), WorkflowError> {
    let node = execution
        .workflow
        .nodes
        .get(execution.current_node_index)
        .ok_or_else(|| WorkflowError::invalid_state("current node index is out of range"))?;
    let contract = node.artifact.clone();
    let submitted = contract.as_deref().and_then(|contract| {
        submission::submitted_node_artifact_for(
            &execution.artifacts,
            &target.node_name,
            target.attempt,
            contract,
        )
    });
    let artifact = submitted
        .as_ref()
        .and_then(|output| output.artifact.clone());
    let application = workflow_transition::ApprovalApplication {
        effective_result: submitted
            .as_ref()
            .and_then(|output| output.result.clone())
            .unwrap_or_else(|| "approve".to_string()),
        artifact: artifact.clone(),
        contract: submitted.as_ref().and(contract),
    };
    let plan = execution.plan_approval_application(application)?;
    let entry = runtime_driver::make_node_history_entry(
        execution,
        Some(plan.completion.result),
        plan.completion.artifact,
        plan.completion.contract,
        timestamp,
    );
    execution.record_history_entry(entry, timestamp);
    let mut outcome = runtime_driver::apply_advance(execution, next_node_execution_id, timestamp)
        .map_err(runtime_error_to_workflow_error)?;
    if execution.complete_node_execution(&target.node_execution_id, artifact, None, timestamp)
        != TransitionOutcome::Applied
    {
        return Err(WorkflowError::invalid_state(format!(
            "NodeExecution '{}' could not be approved",
            target.node_execution_id
        )));
    }
    *outcome.snapshot_mut() = RuntimeCommitSnapshot::from_execution(execution)
        .map_err(runtime_error_to_workflow_error)?;
    let approval_event = WorkflowEvent::ApprovalResolved {
        execution_id: execution.id.clone(),
        node_execution_id: target.node_execution_id.clone(),
        node_name: target.node_name.clone(),
        comment,
        timestamp,
    };
    let events = runtime_events::required_events_for_approval_commit(approval_event, &mut outcome)
        .map_err(runtime_error_to_workflow_error)?;
    Ok((outcome, events))
}

fn apply_fanout_approval(
    execution: &mut DomainWorkflowExecution,
    target: &crate::domain::workflow::entities::workflow_execution::ApprovalAttemptTarget,
    comment: Option<String>,
    next_node_execution_id: String,
    timestamp: f64,
) -> Result<(NodeOutcome, Vec<WorkflowEvent>), WorkflowError> {
    if execution.admit_fanout_approval() != TransitionOutcome::Applied {
        return Err(WorkflowError::invalid_state(
            "workflow does not accept fanout approval",
        ));
    }
    let node_contract = execution
        .workflow
        .nodes
        .iter()
        .find(|node| node.name == target.node_name)
        .and_then(|node| node.artifact.clone());
    let child = execution
        .fanout_runtime
        .as_ref()
        .and_then(|fanout| {
            fanout
                .children
                .iter()
                .find(|child| child.node_execution_id == target.node_execution_id)
        })
        .cloned()
        .ok_or_else(|| {
            WorkflowError::invalid_state(format!(
                "fanout child '{}' disappeared",
                target.node_execution_id
            ))
        })?;
    let result = child.result.clone().or_else(|| Some("approve".to_string()));
    if execution.complete_fanout_child_execution(
        &target.node_execution_id,
        result.clone(),
        target.artifact.clone(),
        node_contract.clone(),
        child.token_usage.clone(),
        timestamp,
    ) != TransitionOutcome::Applied
    {
        return Err(WorkflowError::invalid_state(format!(
            "fanout child '{}' could not be approved",
            target.node_execution_id
        )));
    }
    execution.record_successful_node_completion(&target.node_name, timestamp);
    let mut events = vec![WorkflowEvent::ApprovalResolved {
        execution_id: execution.id.clone(),
        node_execution_id: target.node_execution_id.clone(),
        node_name: target.node_name.clone(),
        comment,
        timestamp,
    }];
    events.push(WorkflowEvent::NodeCompleted {
        execution_id: execution.id.clone(),
        node_execution_id: target.node_execution_id.clone(),
        node_name: target.node_name.clone(),
        attempt: target.attempt,
        result_summary: result,
        token_usage: Some(child.token_usage),
        timestamp,
    });

    let all_children_succeeded = execution.fanout_runtime.as_ref().is_some_and(|fanout| {
        fanout
            .children
            .iter()
            .all(|child| child.state == FanoutChildRuntimeState::Completed)
    });
    let mut outcome = if all_children_succeeded {
        let fanout = execution.fanout_runtime.as_ref().ok_or_else(|| {
            WorkflowError::invalid_state("fanout runtime disappeared before completion")
        })?;
        let parent_node_name = fanout.parent_node_name.clone();
        let parent_node_execution_id = fanout.parent_node_execution_id.clone();
        let parent_attempt = execution
            .node_execution_counts
            .get(&parent_node_name)
            .copied()
            .unwrap_or(1);
        let child_inputs = fanout
            .children
            .iter()
            .map(|child| workflow_fanout::FanoutChildCompletionInput {
                node_name: child.node_name.clone(),
                session_id: (!child.session_id.is_empty()).then(|| child.session_id.clone()),
                result: child.result.clone(),
                artifact: child.artifact.clone().unwrap_or(serde_json::Value::Null),
                contract: child.contract.clone(),
                token_usage: child.token_usage.clone(),
                attempt: child.attempt,
                completed_at: child.completed_at.unwrap_or(timestamp),
                state: NODE_STATUS_COMPLETED.to_string(),
                failure_kind: child.failure_kind,
                failure_disposition: child.failure_disposition,
            })
            .collect::<Vec<_>>();
        let plan = workflow_fanout::plan_fanout_parent_completion(
            &parent_node_name,
            parent_attempt,
            &child_inputs,
            timestamp,
        );
        events.push(WorkflowEvent::ArtifactProduced {
            execution_id: execution.id.clone(),
            node_execution_id: parent_node_execution_id.clone(),
            node_name: parent_node_name,
            contract: None,
            value: plan
                .parent_artifact
                .artifact
                .clone()
                .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
            request_id: None,
            submitted_at: None,
            timestamp,
        });
        if execution.finalize_fanout_parent(
            &parent_node_execution_id,
            plan.parent_artifact,
            plan.history_entry,
            timestamp,
        ) != TransitionOutcome::Applied
        {
            return Err(WorkflowError::invalid_state(
                "fanout parent could not be completed",
            ));
        }
        runtime_driver::apply_advance(execution, next_node_execution_id, timestamp)
            .map_err(runtime_error_to_workflow_error)?
    } else {
        NodeOutcome::Persist(
            RuntimeCommitSnapshot::from_execution(execution)
                .map_err(runtime_error_to_workflow_error)?,
        )
    };
    events.extend(
        runtime_events::pre_commit_required_events_for_outcome(&outcome)
            .map_err(runtime_error_to_workflow_error)?,
    );
    *outcome.snapshot_mut() = RuntimeCommitSnapshot::from_execution(execution)
        .map_err(runtime_error_to_workflow_error)?;
    Ok((outcome, events))
}

fn submit_was_persisted(events: &[WorkflowEvent], expected_node_execution_id: &str) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            WorkflowEvent::NodeSubmitReceived {
                node_execution_id,
                ..
            } if node_execution_id == expected_node_execution_id
        )
    })
}

fn apply_completion_handshake(
    execution: &mut DomainWorkflowExecution,
    node_execution_id: &str,
    next_node_execution_id: String,
    timestamp: f64,
) -> Result<(Option<NodeOutcome>, Vec<WorkflowEvent>), WorkflowError> {
    let applied = execution.apply_node_completion_handshake(
        node_execution_id,
        next_node_execution_id,
        timestamp,
    )?;
    let outcome = applied
        .advance
        .map(|advance| runtime_driver::node_outcome_from_advance(execution, advance))
        .transpose()
        .map_err(runtime_error_to_workflow_error)?;
    let mut events = applied.events;
    if let Some(outcome) = outcome.as_ref() {
        events.extend(
            runtime_events::pre_commit_required_events_for_outcome(outcome)
                .map_err(runtime_error_to_workflow_error)?,
        );
    }
    Ok((outcome, events))
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
