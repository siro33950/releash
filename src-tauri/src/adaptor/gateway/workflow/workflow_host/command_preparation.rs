//! Command input preparation for the workflow driver.

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
    pub(super) session_id: Option<String>,
}

pub(super) fn command_execution_input_is_current(
    execution: &DomainWorkflowExecution,
    input: &CommandExecutionInput,
) -> bool {
    // Running のみ受理する。stop は対象 node を Paused にするため、is_active
    //（Paused 含む）で判定すると停止後の stale command を準備・起動してしまう。
    // resume は新しい attempt（Running で開始）を作るので Paused を通す必要はない。
    execution.is_active()
        && execution.node_executions.iter().any(|node_execution| {
            node_execution.id == input.node_execution_id
                && node_execution.status
                    == crate::domain::workflow::entities::workflow_execution::RuntimeNodeExecutionStatus::Running
        })
}

impl WorkflowRuntimeHost {
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
                .iter()
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
            let _ = execution.record_node_display_command(
                &input.node_execution_id,
                display_command.clone(),
                timestamp,
            );
            let snapshot = RuntimeCommitSnapshot::from_execution(execution)?;
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
        self.finalize_after_commit(app, &snapshot, &worktree_path)
            .await;
        Ok(true)
    }
}

#[cfg(test)]
mod command_preparation_tests {
    use super::*;
    use crate::domain::workflow::entities::workflow_execution::{
        WorkflowExecution, WorkflowExecutionRestore,
    };
    use crate::domain::workflow::{NodeDefinition, NodeKindName, WorkflowDefinition};

    fn input_for(node_execution_id: &str) -> CommandExecutionInput {
        CommandExecutionInput {
            execution_id: "execution-1".to_string(),
            node_execution_id: node_execution_id.to_string(),
            node_name: "check".to_string(),
            attempt: 1,
            worktree_path: "/repo".to_string(),
            raw_command: Some("true".to_string()),
            contract: None,
            schemas: Default::default(),
            session_id: None,
        }
    }

    fn execution_with_running_command() -> (WorkflowExecution, String) {
        let mut execution = WorkflowExecution::restore_runtime(WorkflowExecutionRestore {
            id: "execution-1".to_string(),
            workflow: WorkflowDefinition {
                name: "wf".to_string(),
                entry: "check".to_string(),
                nodes: vec![NodeDefinition {
                    name: "check".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..WorkflowExecutionRestore::default()
        });
        let node_execution_id = execution
            .begin_node_attempt(
                "check".to_string(),
                NodeKindName::Command,
                1,
                None,
                "command-1".to_string(),
                1.0,
            )
            .unwrap();
        (execution, node_execution_id)
    }

    #[test]
    fn command_input_is_current_only_while_the_node_execution_is_running() {
        let (mut execution, node_execution_id) = execution_with_running_command();
        let input = input_for(&node_execution_id);
        assert!(command_execution_input_is_current(&execution, &input));

        // stop で Paused になった stale command は受理しない。
        assert_eq!(
            execution.pause_node_execution(&node_execution_id, 2.0),
            crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
        );
        assert!(!command_execution_input_is_current(&execution, &input));
    }
}
