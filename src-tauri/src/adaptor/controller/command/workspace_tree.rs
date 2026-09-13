pub(crate) const COMMAND_NAMES: &[&str] = &[
    "list_workspace_worktree_nodes",
    "get_workspace_tree_selection_reconciliation",
    "list_workspace_workflow_history",
    "get_workspace_node_detail",
    "get_workspace_session_node_id",
    "approve_workspace_node",
    "retry_workspace_node",
    "rename_workspace_session_node",
    "archive_workspace_workflow_execution",
    "restore_workspace_workflow_execution",
];

pub(crate) fn register(router: &mut super::CommandRouter) {
    router.register_domain(
        COMMAND_NAMES,
        Box::new(super::client::handle_registered_invoke),
    );
}
