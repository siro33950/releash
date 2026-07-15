use crate::domain::workflow::services::approval_rules;
use crate::domain::workflow::{
    ContractType, NodeName, WorkflowError, WorkflowExecutionId, WorkflowName, WorktreePath,
};

use super::{AbortExecutionCommand, ApprovalCommand, StartExecutionCommand, SubmitOutputCommand};
use crate::usecase::workflow::ports::{
    WorkflowStallClearedNotification, WorkflowStallObservedNotification,
    WorkflowTurnCompleteNotification,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkflowRuntimeCommandPreflight;

impl WorkflowRuntimeCommandPreflight {
    pub(crate) fn validate_start_execution(
        &self,
        command: &StartExecutionCommand,
    ) -> Result<(), WorkflowError> {
        WorkflowName::new(command.workflow_name.clone())?;
        WorktreePath::new(command.worktree_path.clone())?;
        if command.permission_mode.trim().is_empty() {
            return Err(WorkflowError::validation(
                "permission_mode must not be empty",
            ));
        }
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

    pub(crate) fn validate_approval(&self, command: &ApprovalCommand) -> Result<(), WorkflowError> {
        WorkflowExecutionId::new(command.execution_id.clone())?;
        NodeName::new(command.node_name.clone())?;
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
        NodeName::new(command.node_name.clone())?;
        ContractType::new(command.contract.clone())?;
        Ok(())
    }

    pub(crate) fn validate_turn_complete(
        &self,
        command: &WorkflowTurnCompleteNotification,
    ) -> Result<(), WorkflowError> {
        if command.chat_session_id.trim().is_empty() {
            return Err(WorkflowError::validation(
                "chat_session_id must not be empty",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_stall_observed(
        &self,
        command: &WorkflowStallObservedNotification,
    ) -> Result<(), WorkflowError> {
        if command.chat_session_id.trim().is_empty() {
            return Err(WorkflowError::validation(
                "chat_session_id must not be empty",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_stall_cleared(
        &self,
        command: &WorkflowStallClearedNotification,
    ) -> Result<(), WorkflowError> {
        if command.chat_session_id.trim().is_empty() {
            return Err(WorkflowError::validation(
                "chat_session_id must not be empty",
            ));
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

    pub(crate) fn validate_worktree_lookup(
        &self,
        worktree_path: &str,
    ) -> Result<(), WorkflowError> {
        WorktreePath::new(worktree_path.to_string()).map(|_| ())
    }

    pub(crate) fn validate_approval_chat(
        &self,
        execution_id: &str,
        content: &str,
    ) -> Result<(), WorkflowError> {
        WorkflowExecutionId::new(execution_id.to_string())?;
        if content.trim().is_empty() {
            return Err(WorkflowError::validation(
                "approval chat content must not be empty",
            ));
        }
        Ok(())
    }
}

fn validate_optional_node_name(value: Option<&str>) -> Result<(), WorkflowError> {
    if let Some(value) = value {
        NodeName::new(value.to_string())?;
    }
    Ok(())
}
