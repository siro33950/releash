//! Agent-session activation procedure for workflow nodes.

use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

/// ワークフロー状態をブロードキャストする。
/// スナップショットは呼び出し元がロック内で確定したものを受け取る。
pub(crate) async fn broadcast_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    worktree_path: &str,
    commit_snapshot: RuntimeCommitSnapshot,
) {
    crate::adaptor::gateway::workflow::emit_workflow_execution_from_snapshot(
        app,
        worktree_path,
        crate::usecase::workflow::runtime_snapshot::runtime_commit_snapshot_to_domain_snapshot(
            commit_snapshot,
        ),
    )
    .await;
}
