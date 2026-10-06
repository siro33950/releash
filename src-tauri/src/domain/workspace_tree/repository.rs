use super::{WorkspaceIdentity, WorkspaceTree, WorkspaceTreeNode};

/// Read-only port for restoring a Workspace aggregate from canonical indexed
/// records. There is intentionally no save/CAS operation.
#[async_trait::async_trait]
pub trait WorkspaceTreeRepository: Send + Sync {
    /// 複数の Workspace の実行木をまとめて読む。結果は指定した順に並ぶ。
    /// 実行木を持たない Workspace は空の木になる。
    async fn load_trees(
        &self,
        workspace_identities: &[WorkspaceIdentity],
    ) -> Vec<Result<WorkspaceTree, crate::domain::workflow::WorkflowError>>;

    async fn load_node_by_session_id(
        &self,
        workspace_identity: &WorkspaceIdentity,
        session_id: &str,
    ) -> Result<Option<WorkspaceTreeNode>, crate::domain::local_event::LocalEventQueryError>;

    async fn load_node(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceTreeNode>, crate::domain::local_event::LocalEventQueryError>;

    #[cfg(any(test, feature = "test-support"))]
    async fn load_node_by_node_execution_id(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<WorkspaceTreeNode>, crate::domain::local_event::LocalEventQueryError>;
}
