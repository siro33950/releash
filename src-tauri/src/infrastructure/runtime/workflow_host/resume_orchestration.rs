//! Resume orchestration for durable runtime state.

use super::*;

fn runtime_node_kind_name(kind: crate::domain::workflow::NodeKindName) -> NodeKindName {
    match kind {
        crate::domain::workflow::NodeKindName::Session => NodeKindName::Session,
        crate::domain::workflow::NodeKindName::Fanout => NodeKindName::Fanout,
        crate::domain::workflow::NodeKindName::Command => NodeKindName::Command,
    }
}

pub(super) fn runtime_token_usage(usage: &crate::domain::workflow::TokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    }
}

pub(super) fn runtime_node_execution(
    node: &crate::domain::workflow::NodeExecution,
) -> NodeExecution {
    NodeExecution {
        id: node.id.clone(),
        execution_id: node.execution_id.clone(),
        node_name: node.node_name.clone(),
        kind: runtime_node_kind_name(node.kind),
        attempt: node.attempt,
        status: match node.status {
            crate::domain::workflow::NodeExecutionStatus::Running => NodeExecutionStatus::Running,
            crate::domain::workflow::NodeExecutionStatus::WaitingApproval => {
                NodeExecutionStatus::WaitingApproval
            }
            crate::domain::workflow::NodeExecutionStatus::Succeeded => {
                NodeExecutionStatus::Succeeded
            }
            crate::domain::workflow::NodeExecutionStatus::Failed => NodeExecutionStatus::Failed,
            crate::domain::workflow::NodeExecutionStatus::Aborted => NodeExecutionStatus::Aborted,
        },
        session_id: node.session_id.clone(),
        display_command: node.display_command.clone(),
        artifact: node
            .artifact
            .as_ref()
            .map(|artifact| artifact.value.clone()),
        token_usage: node.token_usage.as_ref().map(runtime_token_usage),
        failure: node.failure.as_ref().map(|failure| {
            crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionFailure {
                reason: failure.reason.clone(),
                kind: failure.kind,
            }
        }),
        fanout_parent: node.fanout_parent.as_ref().map(|parent| {
            crate::domain::workflow::FanoutParentRef {
                parent_node: parent.parent_node.clone(),
                parent_attempt: parent.parent_attempt,
                item_index: parent.item_index,
                child_index: parent.child_index,
            }
        }),
        started_at: node.started_at,
        completed_at: node.completed_at,
    }
}

fn hydrate_resumed_execution(
    checkpoint: &workflow_resume_projection::ResumeProjection,
    now: f64,
) -> Result<(DomainWorkflowExecution, Option<FanoutResumeCheckpoint>), WorkflowRuntimeError> {
    let current_node_index = checkpoint
        .workflow
        .nodes
        .iter()
        .position(|node| node.name == checkpoint.resume_from_node)
        .ok_or_else(|| {
            WorkflowRuntimeError::InvalidWorkflow(format!(
                "resume node '{}' is absent from workflow '{}'",
                checkpoint.resume_from_node, checkpoint.workflow.name
            ))
        })?;

    let mut node_history = Vec::new();
    let mut artifacts = HashMap::new();
    artifacts.insert(
        crate::domain::workflow::services::reference::REQUEST_ARTIFACT.to_string(),
        workflow_prompt::request_node_artifact(&checkpoint.request, checkpoint.started_at),
    );
    for node in &checkpoint.confirmed_top_level_nodes {
        let completed_at = node.completed_at.unwrap_or(node.started_at);
        let fanout_children =
            (node.kind == crate::domain::workflow::NodeKindName::Fanout).then(|| {
                checkpoint
                    .projected_node_executions
                    .iter()
                    .filter_map(|child| {
                        let parent = child.fanout_parent.as_ref()?;
                        if parent.parent_node != node.node_name
                            || parent.parent_attempt != node.attempt
                            || child.status
                                != crate::domain::workflow::NodeExecutionStatus::Succeeded
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
                    .collect::<Vec<_>>()
            });
        node_history.push(crate::domain::workflow::NodeHistoryEntry {
            node_name: node.node_name.clone(),
            completed_at,
            result: node.result_summary.clone(),
            session_id: node.session_id.clone(),
            token_usage: node.token_usage.as_ref().map(runtime_token_usage),
            artifact: node
                .artifact
                .as_ref()
                .map(|artifact| artifact.value.clone()),
            attempt: node.attempt,
            fanout_children,
            state: crate::domain::workflow::NODE_STATUS_COMPLETED.to_string(),
        });
        if let Some(artifact) = node.artifact.as_ref() {
            let contract = artifact.contract.clone().or_else(|| {
                checkpoint
                    .workflow
                    .nodes
                    .iter()
                    .find(|definition| definition.name == node.node_name)
                    .and_then(|definition| definition.artifact.clone())
            });
            artifacts.insert(
                node.node_name.clone(),
                RuntimeArtifact {
                    node_name: node.node_name.clone(),
                    attempt: node.attempt,
                    session_id: node.session_id.clone(),
                    result: node.result_summary.clone(),
                    artifact: Some(artifact.value.clone()),
                    contract,
                    token_usage: node.token_usage.as_ref().map(runtime_token_usage),
                    completed_at,
                },
            );
        }
    }

    let mut node_execution_counts = checkpoint.node_execution_counts.clone();
    let resumed_attempt = node_execution_counts
        .entry(checkpoint.resume_from_node.clone())
        .and_modify(|attempt| *attempt = attempt.saturating_add(1))
        .or_insert(1)
        .to_owned();
    let mut node_executions = checkpoint
        .projected_node_executions
        .iter()
        .map(runtime_node_execution)
        .collect::<Vec<_>>();
    node_executions.push(NodeExecution {
        id: uuid::Uuid::new_v4().to_string(),
        execution_id: checkpoint.execution_id.clone(),
        node_name: checkpoint.resume_from_node.clone(),
        kind: checkpoint.workflow.nodes[current_node_index].kind_name(),
        attempt: resumed_attempt,
        status: NodeExecutionStatus::Running,
        session_id: None,
        display_command: None,
        artifact: None,
        token_usage: None,
        failure: None,
        fanout_parent: None,
        started_at: now,
        completed_at: None,
    });

    let fanout_checkpoint =
        (!checkpoint.confirmed_fanout_children.is_empty()).then(|| FanoutResumeCheckpoint {
            parent_node_name: checkpoint.resume_from_node.clone(),
            children: checkpoint
                .confirmed_fanout_children
                .iter()
                .map(|child| FanoutResumeChild {
                    node_name: child.node_name.clone(),
                    item_index: child.item_index,
                    child_index: child.child_index,
                    reusable: workflow_fanout_runtime::ReusableFanoutChild {
                        result: child.result_summary.clone(),
                        display_command: child.display_command.clone(),
                        artifact: child.artifact.clone(),
                        contract: child.contract.clone(),
                        token_usage: child.token_usage.as_ref().map(runtime_token_usage),
                        completed_at: child.completed_at,
                    },
                })
                .collect(),
        });

    Ok((
        crate::infrastructure::runtime::workflow_host::execution_state::domain_workflow_execution! {
            id: checkpoint.execution_id.clone(),
            workflow: checkpoint.workflow.clone(),
            lifecycle: DomainWorkflowExecution::lifecycle_from_state(RuntimeExecutionState::Running),
            current_node_index,
            node_execution_counts,
            loop_guard_reset_baselines: checkpoint.loop_guard_reset_baselines.clone(),
            node_history,
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: checkpoint.permission_mode.clone(),
            },
            worktree_path: checkpoint.worktree_path.clone(),
            created_from: checkpoint.created_from,
            error_reason: None,
            started_at: checkpoint.started_at,
            updated_at: now,
            current_session_id: None,
            current_node_token_usage: TokenUsage::default(),
            artifacts,
            node_executions,
            request: Some(checkpoint.request.clone()),
            fanout_runtime: None,
            current_stall_observations: Vec::new(),
        },
        fanout_checkpoint,
    ))
}

pub(super) async fn resume_workflow_execution<R: tauri::Runtime + 'static>(
    driver: &WorkflowRuntimeHost,
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    agent_runtime: &Arc<AgentSessionRuntimeUsecase>,
    execution_id: &str,
) -> Result<(), WorkflowRuntimeError> {
    let metadata = driver
        .validate_execution_command_target(execution_id)
        .await?;
    if metadata.status != ExecutionStatus::Interrupted {
        return Err(WorkflowRuntimeError::InvalidState(format!(
            "execution {execution_id} cannot be resumed from status {}",
            metadata.status.as_str()
        )));
    }

    let data_dir = match driver.execution_store.configured_data_dir().await {
        Some(data_dir) => data_dir,
        None => crate::infrastructure::platform::app_data_dir::resolve_data_dir(app).map_err(
            |error| WorkflowRuntimeError::SessionStore(format!("resolve_data_dir: {error}")),
        )?,
    };
    let events = driver
        .durable_workflow_event_log(&data_dir)
        .await?
        .read_log_durable(execution_id)
        .await
        .map_err(WorkflowRuntimeError::SessionStore)?;
    let checkpoint = workflow_resume_projection::project_resume_checkpoint(execution_id, &events)
        .map_err(WorkflowRuntimeError::InvalidState)?;
    if checkpoint.worktree_path != metadata.worktree_path
        || checkpoint.resume_from_node != metadata.resume_from_node.clone().unwrap_or_default()
    {
        return Err(WorkflowRuntimeError::UnauthorizedWorktree(format!(
            "execution {execution_id} metadata does not match its event-log checkpoint"
        )));
    }
    session_store
        .ensure_no_unresolved_recovery(execution_id)
        .await
        .map_err(|failure| {
            WorkflowRuntimeError::InvalidState(format!(
                "execution {execution_id} cannot resume while recovery {} is unresolved: {failure}",
                failure.correlation_id
            ))
        })?;
    let mut owned_session_ids = checkpoint
        .projected_node_executions
        .iter()
        .filter_map(|node| node.session_id.clone())
        .collect::<Vec<_>>();
    owned_session_ids.sort();
    owned_session_ids.dedup();
    for session_id in owned_session_ids {
        session_store
            .ensure_no_unresolved_recovery(&session_id)
            .await
            .map_err(|failure| {
                WorkflowRuntimeError::InvalidState(format!(
                    "execution {execution_id} cannot resume while owned session {session_id} has unresolved recovery {}: {failure}",
                    failure.correlation_id
                ))
            })?;
        agent_runtime
            .ensure_recovery_operation_allowed(&session_id)
            .map_err(|error| {
                WorkflowRuntimeError::InvalidState(format!(
                    "execution {execution_id} cannot resume while owned session {session_id} has unresolved recovery: {error}"
                ))
            })?;
    }
    workflow_runtime_start_guard::validate_workflow_shape(&checkpoint.workflow)?;
    let registry = agent_runtime.backend_registry();
    let definition =
        crate::infrastructure::runtime::workflow_host::runtime_mapping::workflow_definition_to_domain(
            &checkpoint.workflow,
        );
    crate::domain::workflow::validation::validate_models(&definition, |model| {
        registry
            .resolve_model_entry(model)
            .map(|entry| Some(entry.backend))
    })
    .map_err(|error| WorkflowRuntimeError::InvalidWorkflow(error.to_string()))?;
    let facet_contents =
        WorkflowRuntimeHost::resolve_facet_contents_for_workflow(&checkpoint.workflow)?;

    let now = current_timestamp();
    let (execution, fanout_checkpoint) = hydrate_resumed_execution(&checkpoint, now)?;
    let snapshot = execution.to_commit_snapshot();
    let node_started = workflow_runtime_events::node_started_event_for_snapshot(&snapshot)?;
    let reservation = driver
        .execution_store
        .reserve_interrupted_for_resume(execution_id, now)
        .await
        .map_err(|error| match error {
            ExecutionStoreError::WorktreeAlreadyActive { .. } => {
                WorkflowRuntimeError::AlreadyActive(checkpoint.workflow.name.clone())
            }
            ExecutionStoreError::ExecutionNotFound { .. } => {
                WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string())
            }
            ExecutionStoreError::InvalidStatusTransition { .. } => {
                WorkflowRuntimeError::InvalidState(error.to_string())
            }
            other => WorkflowRuntimeError::SessionStore(format!(
                "ExecutionStore resume reservation failed: {other}"
            )),
        })?;

    {
        let mut executions = driver.executions.lock().await;
        if executions.contains_key(execution_id)
            || find_any_by_worktree(&executions, &checkpoint.worktree_path)
                .is_some_and(DomainWorkflowExecution::is_active)
        {
            drop(executions);
            let _ = driver
                .execution_store
                .rollback_resume_reservation(reservation)
                .await;
            return Err(WorkflowRuntimeError::AlreadyActive(
                checkpoint.workflow.name.clone(),
            ));
        }
        executions.insert(execution_id.to_string(), execution);
    }
    driver
        .execution_facet_contents
        .lock()
        .await
        .insert(execution_id.to_string(), facet_contents);
    if let Some(fanout_checkpoint) = fanout_checkpoint {
        driver
            .fanout_resume_checkpoints
            .lock()
            .await
            .insert(execution_id.to_string(), fanout_checkpoint);
    }

    let resumed_events = [
        WorkflowEvent::ExecutionResumed {
            execution_id: execution_id.to_string(),
            resume_from_node: checkpoint.resume_from_node.clone(),
            timestamp: now,
        },
        node_started,
    ];
    #[cfg(test)]
    let injected_failure = driver
        .fail_next_required_event_append
        .swap(false, Ordering::AcqRel);
    #[cfg(not(test))]
    let injected_failure = false;
    let resume_projection_mutations = driver
        .execution_store
        .prepare_atomic_existing_snapshot_mutations(&snapshot)
        .await
        .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string()))?;
    let append_result = if injected_failure {
        Err("injected required event append failure".to_string())
    } else {
        driver.write_log_required_batch_with_mutations_as(
            app,
            CommitOperationKind::UserMutation,
            &resumed_events,
            resume_projection_mutations,
        )
    };
    if let Err(error) = append_result {
        driver.executions.lock().await.remove(execution_id);
        driver
            .fanout_resume_checkpoints
            .lock()
            .await
            .remove(execution_id);
        driver.release_execution_facet_contents(execution_id).await;
        driver
            .execution_store
            .rollback_resume_reservation(reservation)
            .await
            .map_err(|rollback_error| {
                WorkflowRuntimeError::SessionStore(format!(
                    "ExecutionResumed log failed: {error}; reservation rollback failed: {rollback_error}"
                ))
            })?;
        return Err(WorkflowRuntimeError::SessionStore(format!(
            "ExecutionResumed log failed: {error}"
        )));
    }

    if let Err(error) = driver
        .execution_store
        .commit_resume_reservation(&reservation)
        .await
    {
        #[cfg(test)]
        if driver
            .execution_store
            .local_event_authority()
            .await
            .is_none()
        {
            let crash_timestamp = current_timestamp();
            let checkpoint_result = WorkflowEventLog::new(&data_dir).append_batch(&[
                WorkflowEvent::ExecutionInterrupted {
                    execution_id: execution_id.to_string(),
                    reason: ExecutionInterruptionReason::Crash,
                    timestamp: crash_timestamp,
                },
            ]);
            driver.executions.lock().await.remove(execution_id);
            driver
                .fanout_resume_checkpoints
                .lock()
                .await
                .remove(execution_id);
            driver.release_execution_facet_contents(execution_id).await;
            if let Err(checkpoint_error) = checkpoint_result {
                let rollback_result = driver
                    .execution_store
                    .rollback_resume_reservation(reservation)
                    .await;
                return Err(WorkflowRuntimeError::SessionStore(format!(
                    "ExecutionResumed metadata commit failed: {error}; crash checkpoint failed: {checkpoint_error}; reservation rollback: {}",
                    rollback_result
                        .as_ref()
                        .map(|_| "ok".to_string())
                        .unwrap_or_else(|rollback_error| rollback_error.to_string())
                )));
            }
            let projection_result = driver
                .execution_store
                .checkpoint_failed_resume(
                    reservation,
                    ExecutionInterruptionReason::Crash,
                    crash_timestamp,
                )
                .await;
            log::warn!(
                "workflow {execution_id}: accepted Resume metadata commit failed after durable event; crash checkpoint recorded: {error}; crash metadata projection: {}",
                projection_result
                    .as_ref()
                    .map(|_| "ok".to_string())
                    .unwrap_or_else(|projection_error| projection_error.to_string())
            );
            return Ok(());
        }
        log::warn!(
            "workflow {execution_id}: derived ExecutionStore projection refresh failed after the atomic SQLite resume commit: {error}"
        );
    }

    workflow_runtime_session::broadcast_state(app, &checkpoint.worktree_path, snapshot.clone())
        .await;
    if let Err(error) = driver
        .start_current_node_runtime(app, session_store, agent_runtime, &checkpoint.worktree_path)
        .await
    {
        if let Err(interrupt_error) = driver
            .interrupt_active_execution(
                app,
                agent_runtime,
                execution_id,
                ExecutionInterruptionReason::Crash,
            )
            .await
        {
            return Err(WorkflowRuntimeError::SessionStore(format!(
                "resumed runtime start failed: {error}; crash checkpoint failed: {interrupt_error}"
            )));
        }
        log::warn!(
            "workflow {execution_id}: accepted Resume runtime start failed; crash checkpoint recorded: {error}"
        );
    }
    Ok(())
}
