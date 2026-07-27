//! Command input preparation for the runtime adapter.

use super::*;

#[derive(Clone)]
pub(super) struct CommandExecutionInput {
    pub(super) execution_id: String,
    pub(super) node_execution_id: String,
    pub(super) node_name: String,
    pub(super) attempt: u32,
    pub(super) worktree_path: String,
    pub(super) raw_command: Option<String>,
    pub(super) contract: Option<String>,
    pub(super) schemas: BTreeMap<String, DomainSchemaDef>,
    pub(super) fanout_parent: Option<String>,
    pub(super) session_id: Option<String>,
}

pub(super) fn command_execution_input_is_current(
    execution: &WorkflowRuntimeRecord,
    input: &CommandExecutionInput,
) -> bool {
    let node_execution_is_active = execution.node_executions.iter().any(|node_execution| {
        node_execution.id == input.node_execution_id && node_execution.status.is_active()
    });
    if input.fanout_parent.is_some() {
        node_execution_is_active
            && execution.fanout_runtime.as_ref().is_some_and(|fanout| {
                fanout.children.iter().any(|child| {
                    child.node_execution_id == input.node_execution_id
                        && child.state == FanoutChildRuntimeState::Running
                })
            })
    } else {
        node_execution_is_active
            && is_still_current_execution(execution, &input.node_name, input.attempt)
    }
}

impl WorkflowRuntimeExecutor {
    pub(super) async fn commit_command_prepared<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        input: &CommandExecutionInput,
    ) -> Result<bool, WorkflowRuntimeError> {
        let raw_command = input.raw_command.as_deref().ok_or_else(|| {
            WorkflowRuntimeError::InvalidState(format!(
                "raw command for node execution '{}' is unavailable",
                input.node_execution_id
            ))
        })?;
        let secrets = secret_source::collect_configured_secret_values(app);
        let display_command = workflow_secret_masker::mask_sensitive_text(raw_command, &secrets);
        let timestamp = current_timestamp();

        // Keep validation, runtime projection mutation, and the required append under the same
        // execution lock. A concurrent stop can therefore either win before preparation (no
        // event and no spawn) or after the durable CommandPrepared fact.
        let (snapshot, worktree_path) = {
            let mut executions = self.executions.lock().await;
            let Some(execution) = executions.get_mut(&input.execution_id) else {
                return Ok(false);
            };
            if !command_execution_input_is_current(execution, input) {
                return Ok(false);
            }

            let snapshot_before = execution.clone();
            let node_execution = execution
                .node_executions
                .iter_mut()
                .find(|node_execution| node_execution.id == input.node_execution_id)
                .ok_or_else(|| {
                    WorkflowRuntimeError::InvalidState(format!(
                        "active command node execution '{}' disappeared before preparation",
                        input.node_execution_id
                    ))
                })?;
            if node_execution.kind != NodeKindName::Command {
                return Err(WorkflowRuntimeError::InvalidState(format!(
                    "node execution '{}' is not a command",
                    input.node_execution_id
                )));
            }
            node_execution.display_command = Some(display_command.clone());
            execution.updated_at = timestamp;
            let snapshot = execution.to_commit_snapshot();
            let event = WorkflowEvent::CommandPrepared {
                execution_id: input.execution_id.clone(),
                node_execution_id: input.node_execution_id.clone(),
                display_command,
                timestamp,
            };
            if let Err(error) = self.write_log_required_batch(app, &[event]) {
                *execution = snapshot_before;
                return Err(WorkflowRuntimeError::SessionStore(format!(
                    "command prepared event append failed: {error}"
                )));
            }
            (snapshot, execution.worktree_path.clone())
        };

        self.sync_state_after_required_event_commit(&snapshot)
            .await?;
        record_failed_snapshot_telemetry(&snapshot);
        self.finalize_after_commit(app, &snapshot, &worktree_path)
            .await;
        Ok(true)
    }
}
