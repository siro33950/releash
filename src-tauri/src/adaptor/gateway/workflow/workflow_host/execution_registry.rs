//! Gateway-owned lookup over retained workflow execution aggregates.

use std::collections::HashMap;

use crate::adaptor::gateway::workflow::workflow_host::execution_state::DomainWorkflowExecution;
use crate::domain::workflow::ExecutionTreeLaunch;

/// validate_start 用に「workflow として起こされ、worktree_path が一致する exec」を引く。
/// Session として起こされた木は workflow の単一起動制約を所有しない。
pub(crate) fn find_any_by_worktree<'a>(
    execs: &'a HashMap<String, DomainWorkflowExecution>,
    worktree_path: &str,
) -> Option<&'a DomainWorkflowExecution> {
    execs.values().find(|execution| {
        execution.launched_as == ExecutionTreeLaunch::Workflow
            && execution.worktree_path == worktree_path
    })
}
