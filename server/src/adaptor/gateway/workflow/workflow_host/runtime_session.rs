//! Agent-session activation procedure for workflow nodes.

use super::WorkflowRuntimeDependencies;

/// ワークフロー状態をブロードキャストする。
pub async fn broadcast_state(app: &WorkflowRuntimeDependencies, worktree_path: &str) {
    app.state_changes.notify(
        crate::usecase::state_subscription::StateChangeSource::Worktree(worktree_path.into()),
    );
}
