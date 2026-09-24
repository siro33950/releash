//! Agent-session activation procedure for workflow nodes.

use super::WorkflowRuntimeDependencies;

/// ワークフロー状態をブロードキャストする。
pub(crate) async fn broadcast_state(app: &WorkflowRuntimeDependencies, worktree_path: &str) {
    app.state_changes.invalidate(
        crate::domain::state_subscription::StateChangeSource::Worktree(worktree_path.into()),
    );
}
