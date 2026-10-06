//! Command input preparation for the workflow driver.

use super::*;

#[derive(Clone)]
pub struct CommandExecutionInput {
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

    pub async fn commit_command_spawned(
        &self,
        app: &WorkflowRuntimeDependencies,
        input: &CommandExecutionInput,
        display_command: String,
    ) -> Result<bool, WorkflowRuntimeError> {
        let timestamp = current_timestamp();

        let result = retry_runtime_conflicts(&self.queue, &input.node_execution_id, || async {
            let Some(before) = self.load_current_command(app, input).await? else {
                return Ok(None);
            };
            let mut candidate = before.clone();
            candidate.record_node_display_command(
                &input.node_execution_id,
                display_command.clone(),
                timestamp,
            );
            self.commit_control_plane_candidate(
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
            .map(Some)
        })
        .await;
        let snapshot = match result {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => return Ok(false),
            Err(error @ WorkflowRuntimeError::Conflict(_)) => {
                log::warn!(
                    "workflow {}: command {} start was not applied: {error}",
                    input.execution_id,
                    input.node_execution_id
                );
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let worktree_path = snapshot.worktree_path.clone();
        self.finalize_after_commit(app, &snapshot, &worktree_path)
            .await;
        Ok(true)
    }
}

#[cfg(test)]
#[path = "command_preparation_test.rs"]
mod command_preparation_tests;

#[cfg(feature = "test-support")]
impl CommandExecutionInput {
    pub fn test_attempt(&self) -> u32 {
        self.attempt
    }
    pub fn test_attempt_mut(&mut self) -> &mut u32 {
        &mut self.attempt
    }
    pub fn test_execution_id(&self) -> String {
        self.execution_id.clone()
    }
    pub fn test_execution_id_mut(&mut self) -> &mut String {
        &mut self.execution_id
    }
    pub fn test_node_execution_id(&self) -> String {
        self.node_execution_id.clone()
    }
    pub fn test_node_name(&self) -> String {
        self.node_name.clone()
    }
    pub fn test_raw_command(&self) -> Option<String> {
        self.raw_command.clone()
    }
    pub fn test_raw_command_mut(&mut self) -> &mut Option<String> {
        &mut self.raw_command
    }
}

#[cfg(feature = "test-support")]
impl CommandExecutionInput {
    pub fn test_new(
        identity: (String, String, String, u32),
        worktree_path: String,
        raw_command: Option<String>,
        definition_env: Vec<(String, String)>,
        contract: Option<String>,
        schemas: BTreeMap<String, DomainSchemaDef>,
        session_id: Option<String>,
    ) -> Self {
        let (execution_id, node_execution_id, node_name, attempt) = identity;
        Self {
            execution_id,
            node_execution_id,
            node_name,
            attempt,
            worktree_path,
            raw_command,
            definition_env,
            contract,
            schemas,
            session_id,
        }
    }
}
