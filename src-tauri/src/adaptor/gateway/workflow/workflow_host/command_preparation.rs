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
    pub(super) definition_env: Vec<(String, String)>,
    pub(super) contract: Option<String>,
    pub(super) schemas: BTreeMap<String, DomainSchemaDef>,
    pub(super) session_id: Option<String>,
}

pub(super) fn command_execution_input_is_current(
    execution: &DomainExecutionTree,
    input: &CommandExecutionInput,
) -> bool {
    match execution.validate_command_attempt(
        &input.node_execution_id,
        &input.node_name,
        input.attempt,
    ) {
        Ok(()) => true,
        Err(reason) => {
            log::warn!(
                "workflow {}: command {} was not applied: {reason}",
                input.execution_id,
                input.node_execution_id
            );
            false
        }
    }
}

impl WorkflowRuntimeHost {
    pub(super) async fn load_current_command(
        &self,
        app: &WorkflowRuntimeDependencies,
        input: &CommandExecutionInput,
    ) -> Result<Option<DomainExecutionTree>, WorkflowRuntimeError> {
        let execution = self
            .load_control_plane_execution(app, &input.execution_id)
            .await
            .inspect_err(|error| {
                log::warn!(
                    "workflow {}: command {} was not applied: {error}",
                    input.execution_id,
                    input.node_execution_id
                )
            })?;
        match execution {
            Some(execution) if command_execution_input_is_current(&execution, input) => {
                Ok(Some(execution))
            }
            Some(_) => Ok(None),
            None => {
                log::warn!(
                    "workflow {}: command {} was not applied: execution tree was not found",
                    input.execution_id,
                    input.node_execution_id
                );
                Ok(None)
            }
        }
    }

    pub(super) async fn commit_command_spawned(
        &self,
        app: &WorkflowRuntimeDependencies,
        input: &CommandExecutionInput,
        display_command: String,
    ) -> Result<bool, WorkflowRuntimeError> {
        let timestamp = current_timestamp();

        let mut attempts = 0;
        let snapshot = loop {
            attempts += 1;
            let Some(before) = self.load_current_command(app, input).await? else {
                return Ok(false);
            };
            let mut candidate = before.clone();
            candidate.record_node_display_command(
                &input.node_execution_id,
                display_command.clone(),
                timestamp,
            );
            match self
                .commit_control_plane_candidate(
                    app,
                    ControlPlaneCommitCandidate {
                        execution_id: &input.execution_id,
                        snapshot_before: before,
                        candidate,
                        transition_outcome: TransitionOutcome::Applied,
                        events: &[WorkflowEvent::CommandSpawned {
                            execution_id: input.execution_id.clone(),
                            node_execution_id: input.node_execution_id.clone(),
                            display_command: display_command.clone(),
                            timestamp,
                        }],
                        provider_events: Vec::new(),
                    },
                )
                .await
            {
                Err(WorkflowRuntimeError::Conflict(_))
                    if attempts < crate::usecase::workflow::command::CONTROL_PLANE_MAX_ATTEMPTS =>
                {
                    continue
                }
                Err(error @ WorkflowRuntimeError::Conflict(_)) => {
                    log::warn!(
                        "workflow {}: command {} start was not applied: {error}",
                        input.execution_id,
                        input.node_execution_id
                    );
                    return Ok(false);
                }
                result => break result?,
            }
        };
        let worktree_path = snapshot.worktree_path.clone();
        self.finalize_after_commit(app, &snapshot, &worktree_path)
            .await;
        Ok(true)
    }
}

#[cfg(test)]
mod command_preparation_tests {
    use super::*;
    use crate::domain::workflow::entities::workflow_execution::{
        ExecutionTree, ExecutionTreeRestore,
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
            definition_env: Vec::new(),
            contract: None,
            schemas: Default::default(),
            session_id: None,
        }
    }

    fn execution_with_running_command() -> (ExecutionTree, String) {
        let mut execution = ExecutionTree::restore_runtime(ExecutionTreeRestore {
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
            ..ExecutionTreeRestore::default()
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

        // Abort 後の command は起動しない。
        assert_eq!(
            execution.abort_node_execution(&node_execution_id, 2.0),
            crate::domain::workflow::entities::workflow_execution::TransitionOutcome::Applied
        );
        assert!(!command_execution_input_is_current(&execution, &input));
    }
}
