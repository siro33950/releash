use super::fact_log::{FactLogReadBackend, FactReadError};
use super::stored_definition;
use crate::adaptor::gateway::local_event_store::{node_events, reader::storage_unavailable};
use crate::domain::workflow::{IsolatedWorktree, NodeFactMeta, WorktreeInheritance, WorktreeMode};

#[derive(Debug)]
pub(crate) enum WorktreeContextReadError {
    Read(crate::domain::local_event::LocalEventQueryError),
    Corrupt(String),
}

pub(crate) async fn execution_worktree_path(
    backend: &FactLogReadBackend,
    mut node: NodeFactMeta,
    root_meta: NodeFactMeta,
    root: stored_definition::TreeRootContext,
) -> Result<String, WorktreeContextReadError> {
    backend
        .run_indexed(move |connection| {
            Ok((|| {
                let tree_id = &root_meta.tree_id;
                let root_id = &root_meta.node_execution_id;
                let mut visited = std::collections::HashSet::new();
                loop {
                    if !visited.insert(node.node_execution_id.clone()) || &node.tree_id != tree_id {
                        return Err(WorktreeContextReadError::Corrupt(
                            "invalid worktree execution ancestry".into(),
                        ));
                    }
                    let definition = root.definition["nodes"]
                        .get(&node.node_name)
                        .and_then(serde_json::Value::as_object)
                        .ok_or_else(|| {
                            WorktreeContextReadError::Corrupt(
                                "worktree node definition is missing or invalid".into(),
                            )
                        })?;
                    let mode = definition.get("worktree");
                    let mode: Option<WorktreeMode> = mode
                        .cloned()
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(|error| WorktreeContextReadError::Corrupt(error.to_string()))?;
                    let rule = WorktreeInheritance::new(mode);
                    let worktree = rule
                        .for_attempt(
                            root.header.repository_root.as_deref(),
                            &node.node_execution_id,
                            node.attempt,
                        )
                        .map_err(|error| WorktreeContextReadError::Corrupt(error.into()))?;
                    if let Some(path) = rule.isolated_path(worktree.as_ref()) {
                        return Ok(path.to_string());
                    }
                    let Some(parent_id) = &node.parent_id else {
                        return Ok(root.header.worktree_path);
                    };
                    if parent_id == root_id {
                        node = root_meta.clone();
                    } else {
                        let row = node_events::latest_row_for_node(connection, parent_id)
                            .map_err(|error| {
                                WorktreeContextReadError::Read(storage_unavailable(&error))
                            })?
                            .ok_or_else(|| {
                                WorktreeContextReadError::Corrupt(
                                    "parent execution is missing".into(),
                                )
                            })?;
                        node = super::fact_log::node_meta_from_row(&row)
                            .map_err(WorktreeContextReadError::Corrupt)?;
                    }
                }
            })())
        })
        .await
        .map_err(WorktreeContextReadError::Read)?
}

pub(crate) struct StoredWorkspaceWorktreePathQuery {
    data_dir: std::path::PathBuf,
}

impl StoredWorkspaceWorktreePathQuery {
    pub(crate) fn new(data_dir: std::path::PathBuf) -> Self {
        Self { data_dir }
    }
}

#[async_trait::async_trait]
impl crate::usecase::workspace_tree::WorkspaceWorktreePathQuery
    for StoredWorkspaceWorktreePathQuery
{
    async fn workspace_worktree_path(
        &self,
        path: &str,
    ) -> Result<String, crate::domain::workflow::WorkflowError> {
        workspace_worktree_path_with(path, || {
            crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore::open(
                &self.data_dir,
            )
            .map(FactLogReadBackend::ReadOnly)
            .map_err(crate::domain::workflow::WorkflowError::external)
        })
        .await
    }
}

async fn workspace_worktree_path_with(
    path: &str,
    backend: impl FnOnce() -> Result<FactLogReadBackend, crate::domain::workflow::WorkflowError>,
) -> Result<String, crate::domain::workflow::WorkflowError> {
    let Some((node_id, attempt)) = crate::domain::workflow::isolated_worktree_owner(path) else {
        return Ok(path.to_string());
    };
    let requested_path = path.to_string();
    backend()?
        .run_indexed(move |connection| {
            let node = node_events::latest_row_for_node(connection, &node_id)
                .map_err(|error| storage_unavailable(&error))?;
            let Some(node) = node else {
                return Ok(Err("isolated worktree owner is missing".to_string()));
            };
            let root = node_events::first_row_of_tree(connection, &node.tree_id)
                .map_err(|error| storage_unavailable(&error))?;
            Ok((|| -> Result<String, String> {
                if u32::try_from(node.attempt).ok() != Some(attempt) {
                    return Err("isolated worktree attempt does not match".to_string());
                }
                let root = root.ok_or("isolated worktree execution root is missing")?;
                let header = stored_definition::read_tree_header(&root.detail)?
                    .ok_or("execution root metadata is missing")?;
                let repository_root = header
                    .repository_root
                    .as_deref()
                    .ok_or("isolated execution repository root is missing")?;
                if IsolatedWorktree::for_attempt(repository_root, &node_id, attempt).path
                    != requested_path
                {
                    return Err("isolated worktree path does not match its owner".to_string());
                }
                Ok(header.worktree_path)
            })())
        })
        .await
        .map_err(FactReadError::Query)?
        .map_err(FactReadError::Corrupt)
        .map_err(crate::domain::workflow::WorkflowError::from)
}

#[cfg(test)]
#[path = "worktree_context_test.rs"]
mod worktree_context_tests;
