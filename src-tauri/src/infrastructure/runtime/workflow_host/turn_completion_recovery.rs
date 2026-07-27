//! Crash-recovery orchestration for durable agent turn-completion handoffs.
//!
//! The implementation is kept in this module so canonical event hydration
//! remains separate from normal provider activation.

use super::resume_projection::ActiveTurnCompletionProjection;
use super::*;
use crate::domain::workflow::entities::workflow_execution::{
    CanonicalNodeFact, TurnCompletionApplication, WorkflowExecution as WorkflowExecutionAggregate,
};
use crate::domain::workflow::{
    ContractValidationResult, NodeExecution as DomainNodeExecution,
    NodeExecutionStatus as DomainNodeExecutionStatus,
};
use crate::infrastructure::runtime::workflow_host::execution_state::{
    FanoutChildRuntime, FanoutRuntimeState,
};
use crate::usecase::agent_session::session::MessagePart;
use crate::usecase::workflow::ports::{
    WorkflowTurnCompleteRecoveryCommand, WorkflowTurnCompleteRecoveryOutcome,
    WorkflowTurnFailureSignal,
};

struct RecoveryTarget {
    target: DomainNodeExecution,
    parent: DomainNodeExecution,
    parent_position: usize,
    current_node_index: usize,
}

fn invalid(message: impl Into<String>) -> WorkflowRuntimeError {
    WorkflowRuntimeError::InvalidState(message.into())
}

fn validate_recovery_target(
    checkpoint: &ActiveTurnCompletionProjection,
    command: &WorkflowTurnCompleteRecoveryCommand,
) -> Result<RecoveryTarget, WorkflowRuntimeError> {
    if checkpoint.execution_id != command.execution_id
        || checkpoint.workflow.name != command.workflow_name
        || checkpoint.projected_execution.workflow_name != command.workflow_name
    {
        return Err(invalid(
            "workflow turn-completion execution identity does not match canonical events",
        ));
    }
    let target = checkpoint
        .projected_execution
        .node_executions
        .iter()
        .find(|node| node.id == command.node_execution_id)
        .cloned()
        .ok_or_else(|| {
            invalid(format!(
                "workflow turn-completion node execution '{}' is absent from canonical events",
                command.node_execution_id
            ))
        })?;
    if target.execution_id != command.execution_id
        || target.node_name != command.node_name
        || target.attempt != command.attempt
        || target.kind != crate::domain::workflow::NodeKindName::Session
        || target.session_id.as_deref() != Some(command.notification.chat_session_id.as_str())
    {
        return Err(invalid(
            "workflow turn-completion node/session coordinates do not match canonical events",
        ));
    }

    let parent = match (
        command.parent_node_name.as_deref(),
        command.parent_attempt,
        target.fanout_parent.as_ref(),
    ) {
        (None, None, None) => target.clone(),
        (Some(parent_name), Some(parent_attempt), Some(parent_ref))
            if parent_ref.parent_node == parent_name
                && parent_ref.parent_attempt == parent_attempt =>
        {
            checkpoint
                .projected_execution
                .node_executions
                .iter()
                .find(|node| {
                    node.fanout_parent.is_none()
                        && node.node_name == parent_name
                        && node.attempt == parent_attempt
                        && node.kind == crate::domain::workflow::NodeKindName::Fanout
                })
                .cloned()
                .ok_or_else(|| {
                    invalid(
                        "workflow turn-completion fanout parent is absent from canonical events",
                    )
                })?
        }
        _ => {
            return Err(invalid(
                "workflow turn-completion fanout coordinates do not match canonical events",
            ));
        }
    };
    let parent_position = checkpoint
        .projected_execution
        .node_executions
        .iter()
        .position(|node| node.id == parent.id)
        .ok_or_else(|| invalid("workflow turn-completion parent position is unavailable"))?;
    let prior_top_level_completions = checkpoint.projected_execution.node_executions
        [..parent_position]
        .iter()
        .filter(|node| {
            node.fanout_parent.is_none() && node.status == DomainNodeExecutionStatus::Succeeded
        })
        .count();
    if u32::try_from(prior_top_level_completions).ok() != Some(command.order) {
        return Err(invalid(
            "workflow turn-completion node order does not match canonical events",
        ));
    }
    let current_node_index = checkpoint
        .workflow
        .nodes
        .iter()
        .position(|node| node.name == parent.node_name)
        .ok_or_else(|| {
            invalid(format!(
                "workflow turn-completion node '{}' is absent from the workflow snapshot",
                parent.node_name
            ))
        })?;
    Ok(RecoveryTarget {
        target,
        parent,
        parent_position,
        current_node_index,
    })
}

fn has_canonical_completion_fact(
    events: &[WorkflowEvent],
    command: &WorkflowTurnCompleteRecoveryCommand,
) -> bool {
    let target_started_position = events.iter().position(|event| {
        matches!(
            event,
            WorkflowEvent::NodeStarted {
                node_execution_id,
                node_name,
                attempt,
                ..
            } if node_execution_id == &command.node_execution_id
                && node_name == &command.node_name
                && *attempt == command.attempt
        )
    });
    let Some(target_started_position) = target_started_position else {
        return false;
    };
    events
        .iter()
        .skip(target_started_position.saturating_add(1))
        .any(|event| match event {
            WorkflowEvent::NodeCompleted {
                node_execution_id,
                node_name,
                attempt,
                ..
            }
            | WorkflowEvent::NodeFailed {
                node_execution_id,
                node_name,
                attempt,
                ..
            } => {
                node_execution_id == &command.node_execution_id
                    && node_name == &command.node_name
                    && *attempt == command.attempt
            }
            WorkflowEvent::ApprovalRequested {
                node_execution_id,
                node_name,
                ..
            } => node_execution_id == &command.node_execution_id && node_name == &command.node_name,
            // A retry transition currently records the new attempt's NodeStarted
            // as its durable fact. Bind it to the same top-level node and a
            // strictly newer attempt; fanout children do not use this path.
            WorkflowEvent::NodeStarted {
                node_name,
                attempt,
                fanout_parent: None,
                ..
            } if command.parent_node_name.is_none() => {
                node_name == &command.node_name && *attempt > command.attempt
            }
            _ => false,
        })
}

fn runtime_result_from_artifact(
    checkpoint: &ActiveTurnCompletionProjection,
    contract: Option<&str>,
    value: &serde_json::Value,
) -> Result<Option<String>, WorkflowRuntimeError> {
    let Some(contract) = contract else {
        return Ok(None);
    };
    match workflow_contract::validate_artifact_value(
        &workflow_schemas_to_domain(&checkpoint.workflow.schemas),
        contract,
        value.clone(),
    ) {
        ContractValidationResult::Valid { result, .. } => Ok(result),
        ContractValidationResult::Invalid(violation) => Err(invalid(format!(
            "canonical workflow artifact for contract '{contract}' is invalid: {}",
            violation.reason
        ))),
    }
}

fn hydrate_runtime_artifacts(
    checkpoint: &ActiveTurnCompletionProjection,
) -> Result<HashMap<String, RuntimeArtifact>, WorkflowRuntimeError> {
    let mut artifacts = HashMap::new();
    artifacts.insert(
        crate::domain::workflow::services::reference::REQUEST_ARTIFACT.to_string(),
        workflow_prompt::request_node_artifact(&checkpoint.request, checkpoint.started_at),
    );
    for artifact in checkpoint
        .projected_execution
        .artifacts
        .iter()
        .filter(|artifact| {
            artifact.node_name != crate::domain::workflow::services::reference::REQUEST_ARTIFACT
        })
    {
        let execution = checkpoint
            .projected_execution
            .node_executions
            .iter()
            .rev()
            .find(|node| {
                node.node_name == artifact.node_name
                    && node
                        .artifact
                        .as_ref()
                        .is_some_and(|candidate| candidate.value == artifact.value)
            })
            .ok_or_else(|| {
                invalid(format!(
                    "canonical artifact for node '{}' has no producing node execution",
                    artifact.node_name
                ))
            })?;
        let contract = artifact.contract.clone().or_else(|| {
            checkpoint
                .workflow
                .nodes
                .iter()
                .find(|node| node.name == artifact.node_name)
                .and_then(|node| node.artifact.clone())
        });
        let result =
            runtime_result_from_artifact(checkpoint, contract.as_deref(), &artifact.value)?
                .or_else(|| execution.result_summary.clone());
        artifacts.insert(
            artifact.node_name.clone(),
            RuntimeArtifact {
                node_name: artifact.node_name.clone(),
                attempt: execution.attempt,
                session_id: execution.session_id.clone(),
                result,
                artifact: Some(artifact.value.clone()),
                contract,
                token_usage: execution
                    .token_usage
                    .as_ref()
                    .map(resume_orchestration::runtime_token_usage),
                completed_at: artifact.produced_at,
            },
        );
    }
    Ok(artifacts)
}

fn hydrate_node_history(
    checkpoint: &ActiveTurnCompletionProjection,
    target: &RecoveryTarget,
) -> Vec<crate::domain::workflow::NodeHistoryEntry> {
    checkpoint.projected_execution.node_executions[..target.parent_position]
        .iter()
        .filter(|node| {
            node.fanout_parent.is_none() && node.status == DomainNodeExecutionStatus::Succeeded
        })
        .map(|node| {
            let fanout_children = (node.kind == crate::domain::workflow::NodeKindName::Fanout)
                .then(|| {
                    checkpoint
                        .projected_execution
                        .node_executions
                        .iter()
                        .filter_map(|child| {
                            let parent = child.fanout_parent.as_ref()?;
                            if parent.parent_node != node.node_name
                                || parent.parent_attempt != node.attempt
                                || child.status != DomainNodeExecutionStatus::Succeeded
                            {
                                return None;
                            }
                            Some(crate::domain::workflow::FanoutChildSnapshot {
                                node_name: child.node_name.clone(),
                                session_id: child.session_id.clone(),
                                result: child.result_summary.clone(),
                                attempt: child.attempt,
                                completed_at: child.completed_at.unwrap_or(child.started_at),
                                artifact: child
                                    .artifact
                                    .as_ref()
                                    .map(|artifact| artifact.value.clone()),
                                contract: child
                                    .artifact
                                    .as_ref()
                                    .and_then(|artifact| artifact.contract.clone()),
                                state: crate::domain::workflow::NODE_STATUS_COMPLETED.to_string(),
                                failure_kind: None,
                                failure_disposition: None,
                            })
                        })
                        .collect()
                });
            crate::domain::workflow::NodeHistoryEntry {
                node_name: node.node_name.clone(),
                completed_at: node.completed_at.unwrap_or(node.started_at),
                result: node.result_summary.clone(),
                session_id: node.session_id.clone(),
                token_usage: node
                    .token_usage
                    .as_ref()
                    .map(resume_orchestration::runtime_token_usage),
                artifact: node
                    .artifact
                    .as_ref()
                    .map(|artifact| artifact.value.clone()),
                attempt: node.attempt,
                fanout_children,
                state: crate::domain::workflow::NODE_STATUS_COMPLETED.to_string(),
            }
        })
        .collect()
}

fn hydrate_fanout_runtime(
    checkpoint: &ActiveTurnCompletionProjection,
    target: &RecoveryTarget,
) -> Result<Option<FanoutRuntimeState>, WorkflowRuntimeError> {
    let Some(target_parent) = target.target.fanout_parent.as_ref() else {
        return Ok(None);
    };
    let mut children = Vec::new();
    for child in checkpoint
        .projected_execution
        .node_executions
        .iter()
        .filter(|node| {
            node.fanout_parent.as_ref().is_some_and(|parent| {
                parent.parent_node == target_parent.parent_node
                    && parent.parent_attempt == target_parent.parent_attempt
            })
        })
    {
        let state = match child.status {
            DomainNodeExecutionStatus::Running | DomainNodeExecutionStatus::WaitingApproval => {
                FanoutChildRuntimeState::Running
            }
            DomainNodeExecutionStatus::Succeeded => FanoutChildRuntimeState::Completed,
            DomainNodeExecutionStatus::Failed | DomainNodeExecutionStatus::Aborted => {
                return Err(invalid(
                    "workflow turn-completion recovery does not support a fanout with a previously failed child",
                ));
            }
        };
        let contract = checkpoint
            .workflow
            .nodes
            .iter()
            .find(|node| node.name == child.node_name)
            .and_then(|node| node.artifact.clone());
        let artifact = child
            .artifact
            .as_ref()
            .map(|artifact| artifact.value.clone());
        let result = match artifact.as_ref() {
            Some(value) => runtime_result_from_artifact(checkpoint, contract.as_deref(), value)?
                .or_else(|| child.result_summary.clone()),
            None => child.result_summary.clone(),
        };
        children.push(FanoutChildRuntime {
            node_execution_id: child.id.clone(),
            node_name: child.node_name.clone(),
            session_id: child.session_id.clone().unwrap_or_default(),
            state,
            result,
            artifact,
            contract,
            failure_kind: None,
            failure_disposition: None,
            token_usage: child
                .token_usage
                .as_ref()
                .map(resume_orchestration::runtime_token_usage)
                .unwrap_or_default(),
            attempt: child.attempt,
            completed_at: child.completed_at,
        });
    }
    if !children
        .iter()
        .any(|child| child.node_execution_id == target.target.id)
    {
        return Err(invalid(
            "workflow turn-completion fanout target is absent from the reconstructed runtime",
        ));
    }
    Ok(Some(FanoutRuntimeState {
        parent_node_name: target_parent.parent_node.clone(),
        parent_node_execution_id: target.parent.id.clone(),
        children,
    }))
}

fn hydrate_active_execution(
    checkpoint: &ActiveTurnCompletionProjection,
    target: &RecoveryTarget,
) -> Result<DomainWorkflowExecution, WorkflowRuntimeError> {
    if target.target.status != DomainNodeExecutionStatus::Running
        || target.parent.status != DomainNodeExecutionStatus::Running
    {
        return Err(invalid(
            "workflow turn-completion target is not active in canonical events",
        ));
    }
    let state = match checkpoint.projected_execution.status {
        ExecutionStatus::Running => RuntimeExecutionState::Running,
        ExecutionStatus::WaitingApproval => RuntimeExecutionState::WaitingApproval,
        other => {
            return Err(invalid(format!(
                "workflow turn-completion target cannot be hydrated from status {}",
                other.as_str()
            )));
        }
    };
    let fanout_runtime = hydrate_fanout_runtime(checkpoint, target)?;
    Ok(
        crate::infrastructure::runtime::workflow_host::execution_state::domain_workflow_execution! {
            id: checkpoint.execution_id.clone(),
            workflow: checkpoint.workflow.clone(),
            lifecycle: DomainWorkflowExecution::lifecycle_from_state(state),
            current_node_index: target.current_node_index,
            node_execution_counts: checkpoint.node_execution_counts.clone(),
            loop_guard_reset_baselines: checkpoint.loop_guard_reset_baselines.clone(),
            node_history: hydrate_node_history(checkpoint, target),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: checkpoint.permission_mode.clone(),
            },
            worktree_path: checkpoint.worktree_path.clone(),
            created_from: checkpoint.created_from,
            error_reason: None,
            started_at: checkpoint.started_at,
            updated_at: checkpoint.projected_execution.updated_at,
            current_session_id: target
                .target
                .fanout_parent
                .is_none()
                .then(|| target.target.session_id.clone())
                .flatten(),
            current_node_token_usage: TokenUsage::default(),
            artifacts: hydrate_runtime_artifacts(checkpoint)?,
            node_executions: checkpoint
                .projected_execution
                .node_executions
                .iter()
                .map(resume_orchestration::runtime_node_execution)
                .collect(),
            request: Some(checkpoint.request.clone()),
            fanout_runtime,
            current_stall_observations: Vec::new(),
        },
    )
}

fn validate_recovery_can_avoid_provider_effect(
    checkpoint: &ActiveTurnCompletionProjection,
    target: &RecoveryTarget,
    command: &WorkflowTurnCompleteRecoveryCommand,
) -> Result<(), WorkflowRuntimeError> {
    if command.notification.interrupted
        && command.notification.exit_code == 0
        && command.notification.failure_signal.is_none()
    {
        return Err(invalid(
            "workflow turn-completion recovery cannot apply a no-op interruption",
        ));
    }
    let successful_turn =
        command.notification.exit_code == 0 && command.notification.failure_signal.is_none();
    let node = checkpoint
        .workflow
        .nodes
        .iter()
        .find(|node| node.name == target.target.node_name)
        .ok_or_else(|| invalid("workflow turn-completion target definition is missing"))?;
    if successful_turn
        && !node.is_approval_session()
        && node.artifact.is_some()
        && target.target.artifact.is_none()
    {
        return Err(invalid(
            "workflow turn-completion recovery retained a missing structured-output repair for manual reconciliation",
        ));
    }
    Ok(())
}

fn record_only_completion_event(command: &WorkflowTurnCompleteRecoveryCommand) -> WorkflowEvent {
    let timestamp = current_timestamp();
    let failure_signal = command
        .notification
        .failure_signal
        .map(|signal| match signal {
            WorkflowTurnFailureSignal::ModelRefusal => {
                crate::domain::workflow::services::transition::SessionFailureSignal::ModelRefusal
            }
        });
    if command.notification.exit_code != 0 || failure_signal.is_some() {
        let kind = crate::domain::workflow::services::transition::classify_session_error(
            command.notification.exit_code,
            failure_signal,
        );
        return WorkflowEvent::NodeFailed {
            execution_id: command.execution_id.clone(),
            node_execution_id: command.node_execution_id.clone(),
            node_name: command.node_name.clone(),
            attempt: command.attempt,
            reason: format!(
                "workflow-owned session failed (exit_code: {})",
                command.notification.exit_code
            ),
            failure_kind: kind,
            retry_count: None,
            timestamp,
        };
    }

    let token_usage = command.notification.token_usage.as_ref().map(|usage| {
        crate::domain::workflow::TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
        }
    });
    let result_summary = (!command.notification.final_text_parts.is_empty())
        .then(|| command.notification.final_text_parts.join("\n"));
    WorkflowEvent::NodeCompleted {
        execution_id: command.execution_id.clone(),
        node_execution_id: command.node_execution_id.clone(),
        node_name: command.node_name.clone(),
        attempt: command.attempt,
        result_summary,
        token_usage,
        timestamp,
    }
}

impl WorkflowRuntimeHost {
    pub(crate) async fn recover_turn_complete<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
        command: WorkflowTurnCompleteRecoveryCommand,
    ) -> Result<WorkflowTurnCompleteRecoveryOutcome, WorkflowRuntimeError> {
        let data_dir = match self.execution_store.configured_data_dir().await {
            Some(data_dir) => data_dir,
            None => crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
                .map_err(WorkflowRuntimeError::SessionStore)?,
        };
        let events = self
            .durable_workflow_event_log(&data_dir)
            .await?
            .read_log_durable(&command.execution_id)
            .await
            .map_err(WorkflowRuntimeError::SessionStore)?;
        let checkpoint = workflow_resume_projection::project_turn_completion_checkpoint(
            &command.execution_id,
            &events,
        )
        .map_err(invalid)?;
        let target = validate_recovery_target(&checkpoint, &command)?;
        if has_canonical_completion_fact(&events, &command) {
            return Ok(WorkflowTurnCompleteRecoveryOutcome::AlreadyApplied);
        }
        if checkpoint.projected_execution.status.is_resumable() {
            validate_recovery_can_avoid_provider_effect(&checkpoint, &target, &command)?;
            let event = record_only_completion_event(&command);
            let fact = match &event {
                WorkflowEvent::NodeCompleted { .. } => CanonicalNodeFact::Completed,
                WorkflowEvent::NodeFailed {
                    reason,
                    failure_kind,
                    ..
                } => CanonicalNodeFact::Failed {
                    reason: reason.clone(),
                    kind: *failure_kind,
                },
                _ => unreachable!("record-only completion must produce a canonical node fact"),
            };
            let lifecycle = WorkflowExecutionAggregate::restore(
                RuntimeExecutionState::Interrupted,
                checkpoint.projected_execution.interruption_reason,
            );
            let decision = lifecycle.apply_turn_completion(fact);
            if decision.application != TurnCompletionApplication::RecordOnly {
                return Err(invalid(
                    "interrupted workflow completion did not select record-only application",
                ));
            }
            self.write_log_required(app, event)
                .map_err(WorkflowRuntimeError::SessionStore)?;
            return Ok(WorkflowTurnCompleteRecoveryOutcome::Applied);
        }
        if checkpoint.projected_execution.status.is_finished() {
            return Ok(WorkflowTurnCompleteRecoveryOutcome::Retired(
                crate::domain::local_event::WorkflowObligationRetirementReason::Superseded,
            ));
        }
        validate_recovery_can_avoid_provider_effect(&checkpoint, &target, &command)?;
        let execution = hydrate_active_execution(&checkpoint, &target)?;

        {
            let mut executions = self.executions.lock().await;
            if executions.contains_key(&command.execution_id) {
                return Err(invalid(format!(
                    "workflow turn-completion execution '{}' is already live but has no matching canonical completion fact",
                    command.execution_id
                )));
            }
            executions.insert(command.execution_id.clone(), execution);
        }
        {
            let mut refs = self.session_workflow_refs.lock().await;
            if let Some(existing) = refs.get(&command.notification.chat_session_id) {
                if existing.execution_id != command.execution_id {
                    self.executions.lock().await.remove(&command.execution_id);
                    return Err(invalid(
                        "workflow turn-completion session is owned by a different live execution",
                    ));
                }
            }
            refs.insert(
                command.notification.chat_session_id.clone(),
                SessionWorkflowRef {
                    execution_id: command.execution_id.clone(),
                },
            );
        }
        self.recovery_effect_suppression
            .lock()
            .await
            .insert(command.execution_id.clone());

        let final_parts = command
            .notification
            .final_text_parts
            .iter()
            .cloned()
            .map(|content| MessagePart::Text {
                content,
                parent_tool_use_id: None,
            })
            .collect::<Vec<_>>();
        let token_usage = command
            .notification
            .token_usage
            .as_ref()
            .map(|usage| (usage.input_tokens, usage.output_tokens));
        let failure_signal = command
            .notification
            .failure_signal
            .map(|signal| match signal {
                WorkflowTurnFailureSignal::ModelRefusal => {
                    crate::domain::workflow::services::transition::SessionFailureSignal::ModelRefusal
                }
            });
        let apply_result = self
            .on_turn_complete(
                app,
                session_store,
                agent_runtime,
                &command.notification.chat_session_id,
                command.notification.exit_code,
                failure_signal,
                &final_parts,
                token_usage,
            )
            .await;

        self.recovery_effect_suppression
            .lock()
            .await
            .remove(&command.execution_id);
        self.cleanup_session_workflow_refs_by_execution_id(&command.execution_id)
            .await;
        self.executions.lock().await.remove(&command.execution_id);
        self.release_execution_facet_contents(&command.execution_id)
            .await;
        self.release_fanout_resume_checkpoint(&command.execution_id)
            .await;
        apply_result?;

        let committed_events = self
            .durable_workflow_event_log(&data_dir)
            .await?
            .read_log_durable(&command.execution_id)
            .await
            .map_err(WorkflowRuntimeError::SessionStore)?;
        let committed_checkpoint = workflow_resume_projection::project_turn_completion_checkpoint(
            &command.execution_id,
            &committed_events,
        )
        .map_err(invalid)?;
        validate_recovery_target(&committed_checkpoint, &command)?;
        if !has_canonical_completion_fact(&committed_events, &command) {
            return Err(invalid(
                "workflow turn-completion recovery returned without a matching canonical node fact",
            ));
        }
        Ok(WorkflowTurnCompleteRecoveryOutcome::Applied)
    }
}
