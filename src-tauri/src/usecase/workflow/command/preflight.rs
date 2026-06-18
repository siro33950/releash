use crate::domain::workflow::services::approval_rules;
use crate::domain::workflow::{
    ContractType, NodeName, RunId, WorkflowError, WorkflowName, WorktreePath,
};

use super::{AbortRunCommand, ApprovalCommand, StartRunCommand, SubmitOutputCommand};
use crate::usecase::workflow::ports::{
    PendingRuntimeCommand, PendingRuntimeCommandPayload, WorkflowTurnCompleteCommand,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkflowRuntimeCommandPreflight;

impl WorkflowRuntimeCommandPreflight {
    pub(crate) fn validate_start_run(
        &self,
        command: &StartRunCommand,
    ) -> Result<(), WorkflowError> {
        WorkflowName::new(command.workflow_file_stem.clone())?;
        WorktreePath::new(command.worktree_path.clone())?;
        if command.permission_mode.trim().is_empty() {
            return Err(WorkflowError::validation(
                "permission_mode must not be empty",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_abort_run(
        &self,
        command: &AbortRunCommand,
    ) -> Result<(), WorkflowError> {
        RunId::new(command.run_id.clone())?;
        validate_optional_node_name(command.expected_node_name.as_deref())?;
        Ok(())
    }

    pub(crate) fn validate_approval(&self, command: &ApprovalCommand) -> Result<(), WorkflowError> {
        RunId::new(command.run_id.clone())?;
        validate_optional_node_name(command.node_name.as_deref())?;
        approval_rules::validate_approval_decision(&command.decision)
            .map_err(|err| WorkflowError::validation(err.to_string()))
    }

    pub(crate) fn validate_submit_output(
        &self,
        command: &SubmitOutputCommand,
    ) -> Result<(), WorkflowError> {
        RunId::new(command.run_id.clone())?;
        NodeName::new(command.step_name.clone())?;
        ContractType::new(command.contract.clone())?;
        Ok(())
    }

    pub(crate) fn validate_pending_runtime_command(
        &self,
        command: &PendingRuntimeCommand,
    ) -> Result<(), WorkflowError> {
        RunId::new(command.run_id.clone())?;
        RunId::new(command.request_id.clone())?;
        match &command.payload {
            PendingRuntimeCommandPayload::Approve { node_name, comment } => {
                validate_optional_node_name(node_name.as_deref())?;
                approval_rules::validate_approval_decision(
                    &crate::domain::workflow::ApprovalDecision::Approve {
                        comment: comment.clone(),
                    },
                )
                .map_err(|err| WorkflowError::validation(err.to_string()))
            }
            PendingRuntimeCommandPayload::Reject { node_name, reason } => {
                validate_optional_node_name(node_name.as_deref())?;
                approval_rules::validate_approval_decision(
                    &crate::domain::workflow::ApprovalDecision::Reject {
                        reason: reason.clone(),
                    },
                )
                .map_err(|err| WorkflowError::validation(err.to_string()))
            }
            PendingRuntimeCommandPayload::Abort { node_name } => {
                validate_optional_node_name(node_name.as_deref())
            }
            PendingRuntimeCommandPayload::SubmitOutput {
                step_name,
                contract,
                structured_output: _,
            } => {
                NodeName::new(step_name.clone())?;
                ContractType::new(contract.clone())?;
                Ok(())
            }
        }
    }

    pub(crate) fn validate_turn_complete(
        &self,
        command: &WorkflowTurnCompleteCommand,
    ) -> Result<(), WorkflowError> {
        if command.chat_session_id.trim().is_empty() {
            return Err(WorkflowError::validation(
                "chat_session_id must not be empty",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_run_lookup(&self, run_id: &str) -> Result<(), WorkflowError> {
        RunId::new(run_id.to_string()).map(|_| ())
    }

    pub(crate) fn validate_worktree_lookup(
        &self,
        worktree_path: &str,
    ) -> Result<(), WorkflowError> {
        WorktreePath::new(worktree_path.to_string()).map(|_| ())
    }

    pub(crate) fn validate_approval_chat(
        &self,
        run_id: &str,
        content: &str,
    ) -> Result<(), WorkflowError> {
        RunId::new(run_id.to_string())?;
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
