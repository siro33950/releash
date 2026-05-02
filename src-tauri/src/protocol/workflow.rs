use serde::{Deserialize, Serialize};

use crate::session::WorkflowState;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStateSync {
    pub worktree_path: String,
    pub workflow_state: WorkflowState,
}
