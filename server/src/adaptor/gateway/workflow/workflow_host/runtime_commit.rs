//! Workflow runtime transaction preparation and rollback.

use crate::adaptor::gateway::workflow::workflow_host::execution_state::DomainExecutionTree;
use crate::domain::workflow::WorkflowEvent;

pub(crate) struct RequiredEventCommit<'a> {
    pub(crate) execution_id: &'a str,
    pub(crate) snapshot_before: DomainExecutionTree,
    pub(crate) candidate: DomainExecutionTree,
    pub(crate) required_events: Vec<WorkflowEvent>,
    pub(crate) append_error_context: &'a str,
}
