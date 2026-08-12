use super::{WorkspaceIdentity, WorkspaceTree, WorkspaceTreeNode};

/// Read-only port for restoring a Workspace aggregate from canonical indexed
/// records. There is intentionally no save/CAS operation.
pub trait WorkspaceTreeRepository: Send + Sync {
    fn load(
        &self,
        workspace_identity: &WorkspaceIdentity,
    ) -> Result<Option<WorkspaceTree>, crate::domain::local_event::LocalEventQueryError>;

    fn load_node(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceTreeNode>, crate::domain::local_event::LocalEventQueryError>;

    fn load_node_by_node_execution_id(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<WorkspaceTreeNode>, crate::domain::local_event::LocalEventQueryError>;

    fn node_id_for_session(
        &self,
        workspace_identity: &WorkspaceIdentity,
        session_id: &str,
    ) -> Result<Option<String>, crate::domain::local_event::LocalEventQueryError>;
}
