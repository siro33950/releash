//! Abort orchestration.

use super::*;
use crate::domain::workflow::ExecutionTreeArchiveRepository;

impl WorkflowRuntimeHost {
    pub(crate) async fn stop_execution_tree_processes(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.shutdown_active_commands_for_execution(execution_id)
            .await;
        let store = app.store.clone().ok_or_else(|| {
            WorkflowRuntimeError::SessionStore("fact store unavailable".to_string())
        })?;
        let folded = workflow_fact_log::fold_tree_from(
            &workflow_fact_log::FactLogReadBackend::Live(store),
            execution_id,
        )
        .await
        .map_err(|error| {
            let message = error.to_string();
            WorkflowRuntimeError::storage(error, message)
        })?
        .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.to_string()))?;
        for node in &folded.aggregate.node_executions {
            if let Some(session_id) = &node.session_id {
                self.workflow_agent_sessions
                    .stop_agent_session_for_terminal_node_preserving_checkpoint(
                        session_id, &node.id,
                    )
                    .await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn abort_workflow_execution(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<(), WorkflowRuntimeError> {
        let store = app
            .store
            .clone()
            .ok_or_else(|| WorkflowRuntimeError::SessionStore("fact store unavailable".into()))?;
        let location = super::super::ExecutionTreeArchiveFactRepository::from_backend(
            workflow_fact_log::FactLogReadBackend::Live(store),
        )
        .location(execution_id)
        .await
        .map_err(|error| match error {
            crate::domain::workflow::WorkflowError::NotFound(id) => {
                WorkflowRuntimeError::ExecutionNotFound(id)
            }
            error => WorkflowRuntimeError::SessionStore(error.to_string()),
        })?;
        let start_lock = self.workflow_start_lock(&location.worktree_path).await;
        let _start_guard = start_lock.lock().await;
        let gate = self.runtime_activation_gate(execution_id).await;
        gate.request_cancel();
        let mut guard = None;
        tokio::select! {
            biased;
            _ = gate.cancellation_acknowledged() => {}
            acquired = gate.lock.lock() => { guard = Some(acquired); }
        }
        let paused = guard.is_none();
        let result = retry_runtime_conflicts(&self.queue, execution_id, || {
            self.commit_abort_workflow_by_execution_id(app, execution_id, expected_node_name)
        })
        .await;
        match result {
            Ok(snapshot) => {
                if paused {
                    gate.commit_cancel();
                    guard = Some(gate.lock.lock().await);
                }
                let _guard = guard;
                self.cancel_startup_retries(execution_id).await;
                self.shutdown_active_commands_for_execution(execution_id)
                    .await;
                self.finalize_after_commit(app, &snapshot, &snapshot.worktree_path)
                    .await;
                Ok(())
            }
            Err(error) => {
                if paused {
                    gate.rollback_cancel();
                } else {
                    gate.reset_cancel();
                }
                Err(error)
            }
        }
    }

    async fn commit_abort_workflow_by_execution_id(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        crate::common::telemetry::observe_result_async(
            self.commit_abort_workflow_by_execution_id_inner(app, execution_id, expected_node_name),
            |result, _| {
                if result.is_ok() {
                    let classification =
                        FailureClassification::new(NodeExecutionFailureKind::UserAbort);
                    crate::infrastructure::telemetry::metrics::record_workflow_node_failure(
                        classification.kind.as_str(),
                        classification.disposition.as_str(),
                        classification.timeout_kind.map(|kind| kind.as_str()),
                        None,
                    );
                }
            },
        )
        .await
    }

    async fn commit_abort_workflow_by_execution_id_inner(
        &self,
        app: &WorkflowRuntimeDependencies,
        execution_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<RuntimeCommitSnapshot, WorkflowRuntimeError> {
        let before = self
            .load_control_plane_execution(app, execution_id)
            .await?
            .ok_or_else(|| WorkflowRuntimeError::ExecutionNotFound(execution_id.into()))?;
        if !before.is_active() {
            return Err(WorkflowRuntimeError::InvalidState(format!(
                "execution {execution_id} is already terminal"
            )));
        }
        let aborted_node = before.display_current_node();
        if expected_node_name.is_some() && aborted_node.as_deref() != expected_node_name {
            return Err(WorkflowRuntimeError::UnauthorizedApprovalTarget(
                "node does not match".into(),
            ));
        }
        let timestamp = current_timestamp();
        let mut candidate = before.clone();
        candidate.record_aborted_history_for_active_leaves(timestamp);
        candidate.transition_aborted();
        candidate.abort_active_node_executions(timestamp);
        candidate.clear_node_stalls(timestamp);
        let snapshot = self
            .commit_control_plane_candidate(
                app,
                ControlPlaneCommitCandidate {
                    execution_id,
                    snapshot_before: before,
                    candidate,
                    transition_outcome: TransitionOutcome::Applied,
                    events: &[WorkflowEvent::ExecutionAborted {
                        execution_id: execution_id.into(),
                        aborted_node,
                        timestamp,
                    }],
                    provider_events: Vec::new(),
                },
            )
            .await?;
        Ok(snapshot)
    }
}
