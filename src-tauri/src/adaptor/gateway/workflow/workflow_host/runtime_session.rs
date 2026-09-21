//! Agent-session activation procedure for workflow nodes.

use super::WorkflowRuntimeDependencies;

use crate::usecase::workflow::runtime_snapshot::RuntimeCommitSnapshot;

/// ワークフロー状態をブロードキャストする。
/// スナップショットは呼び出し元がロック内で確定したものを受け取る。
pub(crate) async fn broadcast_state(
    app: &WorkflowRuntimeDependencies,
    worktree_path: &str,
    commit_snapshot: RuntimeCommitSnapshot,
) {
    let mut state =
        crate::usecase::workflow::runtime_snapshot::runtime_commit_snapshot_to_domain_snapshot(
            commit_snapshot,
        );
    for node in &mut state.node_executions {
        match app.processes.presence(
            &state.worktree_path,
            &node.id,
            node.kind,
            node.session_id.as_deref(),
        ) {
            Ok(presence) => node.process_presence = presence,
            Err(error) => {
                log::warn!("workflow process presence unavailable: {error}");
                node.process_presence = crate::domain::workflow::NodeProcessPresence::Unknown;
            }
        }
    }
    crate::adaptor::gateway::workflow::emit_workflow_execution_from_snapshot(
        &app.push,
        worktree_path,
        state,
    )
    .await;
}
