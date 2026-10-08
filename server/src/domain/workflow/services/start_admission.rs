use crate::domain::workflow::WorkflowExecutionSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeActiveExecution {
    pub worktree_path: String,
    pub execution_id: String,
    pub workflow_name: String,
}

pub fn validate_worktree_start(
    worktree_path: &str,
    candidates: &[WorkflowExecutionSummary],
) -> Result<(), WorktreeActiveExecution> {
    match candidates
        .iter()
        .find(|execution| execution.worktree_path == worktree_path && execution.status.is_active())
    {
        Some(execution) => Err(WorktreeActiveExecution {
            worktree_path: worktree_path.into(),
            execution_id: execution.execution_id.clone(),
            workflow_name: execution.workflow_name.clone(),
        }),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "start_admission_test.rs"]
mod start_admission_tests;
