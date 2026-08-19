use std::collections::HashSet;

use super::entities::WorkspaceTree;
use super::value_objects::WorkspaceNodeKind;

pub struct WorkspaceTreeVisibilityPolicy;

impl WorkspaceTreeVisibilityPolicy {
    pub fn hidden_branch_ids<'a>(
        tree: &'a WorkspaceTree,
        active_archive_execution_ids: impl IntoIterator<Item = &'a str>,
    ) -> HashSet<String> {
        let archived = active_archive_execution_ids
            .into_iter()
            .collect::<HashSet<_>>();
        tree.nodes()
            .iter()
            .filter(|node| {
                node.kind == WorkspaceNodeKind::Workflow
                    && node
                        .execution_id
                        .as_deref()
                        .is_some_and(|execution_id| archived.contains(execution_id))
            })
            .map(|node| node.id.clone())
            .collect()
    }
}
