use crate::domain::workflow::services::approval_rules;
use crate::domain::workflow::{
    ContractType, NodeDefinitionName, WorkflowDefinitionName, WorkflowError, WorkflowExecutionId,
    WorkspaceWorktreePath,
};

use super::{
    AbortExecutionCommand, ApprovalCommand, ResumeExecutionCommand, StartExecutionCommand,
    StopExecutionCommand, SubmitOutputCommand,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkflowRuntimeCommandPreflight;

impl WorkflowRuntimeCommandPreflight {
    pub(crate) fn validate_start_execution(
        &self,
        command: &StartExecutionCommand,
    ) -> Result<(), WorkflowError> {
        WorkflowDefinitionName::new(command.workflow_name.clone())?;
        WorkspaceWorktreePath::new(command.worktree_path.clone())?;
        Ok(())
    }

    pub(crate) fn validate_abort_execution(
        &self,
        command: &AbortExecutionCommand,
    ) -> Result<(), WorkflowError> {
        WorkflowExecutionId::new(command.execution_id.clone())?;
        validate_optional_node_name(command.expected_node_name.as_deref())?;
        Ok(())
    }

    pub(crate) fn validate_stop_execution(
        &self,
        command: &StopExecutionCommand,
    ) -> Result<(), WorkflowError> {
        WorkflowExecutionId::new(command.execution_id.clone()).map(|_| ())
    }

    pub(crate) fn validate_resume_execution(
        &self,
        command: &ResumeExecutionCommand,
    ) -> Result<(), WorkflowError> {
        WorkflowExecutionId::new(command.execution_id.clone()).map(|_| ())
    }

    pub(crate) fn validate_approval(&self, command: &ApprovalCommand) -> Result<(), WorkflowError> {
        WorkflowExecutionId::new(command.execution_id.clone())?;
        NodeDefinitionName::new(command.node_name.clone())?;
        approval_rules::validate_optional_comment_text(
            command.comment.as_deref(),
            "Approve comment",
        )
        .map_err(|err| WorkflowError::validation(err.to_string()))
    }

    pub(crate) fn validate_submit_output(
        &self,
        command: &SubmitOutputCommand,
    ) -> Result<(), WorkflowError> {
        WorkflowExecutionId::new(command.execution_id.clone())?;
        NodeDefinitionName::new(command.node_name.clone())?;
        if command.node_execution_id.trim().is_empty() {
            return Err(WorkflowError::validation(
                "node_execution_id must not be empty",
            ));
        }
        if let Some(artifact) = &command.artifact {
            ContractType::new(artifact.contract.clone())?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn validate_execution_lookup(
        &self,
        execution_id: &str,
    ) -> Result<(), WorkflowError> {
        WorkflowExecutionId::new(execution_id.to_string()).map(|_| ())
    }
}

fn validate_optional_node_name(value: Option<&str>) -> Result<(), WorkflowError> {
    if let Some(value) = value {
        NodeDefinitionName::new(value.to_string())?;
    }
    Ok(())
}
