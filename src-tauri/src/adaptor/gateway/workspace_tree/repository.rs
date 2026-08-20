use std::sync::Arc;

use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::store::LocalEventStore;
use crate::adaptor::gateway::workflow::fact_log::{self, FactLogReadBackend};
use crate::domain::local_event::{LocalEventQueryError, WorkflowExecutionMetadataRecord};
#[cfg(test)]
use crate::domain::local_event::{SafeOperationFailure, SessionOperationFailureKind};
use crate::domain::workflow::services::fact_replay::{self, FoldedTree};
use crate::domain::workflow::TreeRootFact;
use crate::domain::workspace_tree::{
    RuntimeSnapshotNodeProjection, WorkspaceIdentity, WorkspaceStructureFact, WorkspaceTree,
    WorkspaceTreeNode, WorkspaceTreeProjector, WorkspaceTreeRepository,
};

#[derive(Clone)]
enum WorkspaceSqliteBackend {
    Live(Arc<LocalEventStore>),
    ReadOnly(Arc<LocalEventReadStore>),
}

/// The only concrete `WorkspaceTreeRepository` implementation.
pub(crate) struct SqliteWorkspaceTreeRepository {
    backend: WorkspaceSqliteBackend,
}

impl SqliteWorkspaceTreeRepository {
    pub(crate) fn new(store: Arc<LocalEventStore>) -> Arc<Self> {
        Arc::new(Self {
            backend: WorkspaceSqliteBackend::Live(store),
        })
    }

    pub(crate) fn new_read_only(store: Arc<LocalEventReadStore>) -> Arc<Self> {
        Arc::new(Self {
            backend: WorkspaceSqliteBackend::ReadOnly(store),
        })
    }

    pub(super) fn fact_backend(&self) -> FactLogReadBackend {
        match &self.backend {
            WorkspaceSqliteBackend::Live(store) => FactLogReadBackend::Live(Arc::clone(store)),
            WorkspaceSqliteBackend::ReadOnly(store) => {
                FactLogReadBackend::ReadOnly(Arc::clone(store))
            }
        }
    }

    /// workspace（= worktree）に root を植えた全実行木の fold と metadata。
    pub(super) fn folded_workspace_trees(
        &self,
        workspace: &str,
    ) -> Result<Vec<(FoldedTree, WorkflowExecutionMetadataRecord)>, LocalEventQueryError> {
        let backend = self.fact_backend();
        let tree_ids =
            fact_log::list_tree_ids(&backend, Some(workspace)).map_err(fold_query_error)?;
        let mut trees = Vec::new();
        for tree_id in tree_ids {
            let Some(folded) =
                fact_log::fold_tree_from(&backend, &tree_id).map_err(fold_query_error)?
            else {
                continue;
            };
            let model = fact_replay::derive_read_model(&folded);
            let record = fact_log::metadata_record_from_read_model(&model);
            trees.push((folded, record));
        }
        Ok(trees)
    }

    /// 1 tree の fold と metadata。
    pub(super) fn folded_tree(
        &self,
        tree_id: &str,
    ) -> Result<Option<(FoldedTree, WorkflowExecutionMetadataRecord)>, LocalEventQueryError> {
        let backend = self.fact_backend();
        let Some(folded) = fact_log::fold_tree_from(&backend, tree_id).map_err(fold_query_error)?
        else {
            return Ok(None);
        };
        let model = fact_replay::derive_read_model(&folded);
        let record = fact_log::metadata_record_from_read_model(&model);
        Ok(Some((folded, record)))
    }

    fn tree_nodes(
        workspace: &str,
        folded: &FoldedTree,
        record: &WorkflowExecutionMetadataRecord,
    ) -> Result<Vec<WorkspaceTreeNode>, LocalEventQueryError> {
        let node_recovery_reasons = folded
            .isolated_worktrees
            .entries()
            .filter_map(|entry| {
                entry
                    .recovery_cause()
                    .map(|cause| (entry.owner.node_execution_id.clone(), cause.to_string()))
            })
            .collect::<Vec<_>>();
        crate::domain::workspace_tree::runtime_snapshot_nodes(RuntimeSnapshotNodeProjection {
            execution_id: &folded.aggregate.id,
            workflow_name: &folded.aggregate.workflow.name,
            workspace_identity: workspace,
            workflow_definition: &folded.aggregate.workflow,
            node_executions: &folded.aggregate.node_executions,
            started_at: folded.aggregate.started_at,
            updated_at: folded.aggregate.updated_at,
            execution: record,
            recovery_owner_reason: None,
            node_recovery_reasons: &node_recovery_reasons,
        })
        .map_err(invariant_query_error)
    }

    pub(super) fn workspace_tree_from_folded(
        &self,
        workspace: &str,
        trees: &[(FoldedTree, WorkflowExecutionMetadataRecord)],
    ) -> Result<Option<WorkspaceTree>, LocalEventQueryError> {
        if trees.is_empty() {
            return Ok(None);
        }
        let mut nodes = Vec::new();
        let mut facts = Vec::new();
        for (folded, record) in trees {
            if !matches!(folded.root, TreeRootFact::Workflow(_)) {
                continue;
            }
            nodes.extend(Self::tree_nodes(workspace, folded, record)?);
            facts.push(execution_summary_fact(record));
        }
        let mut tree =
            WorkspaceTree::restore(workspace.to_string(), nodes).map_err(invariant_query_error)?;
        WorkspaceTreeProjector::project(&mut tree, facts).map_err(invariant_query_error)?;
        Ok(Some(tree))
    }
}

impl WorkspaceTreeRepository for SqliteWorkspaceTreeRepository {
    fn load_node(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceTreeNode>, LocalEventQueryError> {
        let workspace = workspace_identity.as_str().to_string();
        let trees = self.folded_workspace_trees(&workspace)?;
        for (folded, record) in &trees {
            if let Some(node) = Self::tree_nodes(&workspace, folded, record)?
                .into_iter()
                .find(|node| node.id == node_id)
            {
                return Ok(Some(node));
            }
        }
        Ok(None)
    }

    fn load_node_by_node_execution_id(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<WorkspaceTreeNode>, LocalEventQueryError> {
        let backend = self.fact_backend();
        let Some(tree_id) = backend
            .tree_id_for_node(node_execution_id)
            .map_err(fold_query_error)?
        else {
            return Ok(None);
        };
        let Some((folded, record)) = self.folded_tree(&tree_id)? else {
            return Ok(None);
        };
        let workspace = record.worktree_path.clone();
        Ok(Self::tree_nodes(&workspace, &folded, &record)?
            .into_iter()
            .find(|node| node.node_execution_id.as_deref() == Some(node_execution_id)))
    }

    fn node_id_for_session(
        &self,
        workspace_identity: &WorkspaceIdentity,
        session_id: &str,
    ) -> Result<Option<String>, LocalEventQueryError> {
        let workspace = workspace_identity.as_str().to_string();
        let backend = self.fact_backend();
        let Some((tree_id, node_execution_id)) =
            fact_log::find_session_attachment(&backend, session_id).map_err(fold_query_error)?
        else {
            return Ok(None);
        };
        let Some((folded, record)) = self.folded_tree(&tree_id)? else {
            return Ok(None);
        };
        if record.worktree_path != workspace {
            return Ok(None);
        }
        Ok(Self::tree_nodes(&workspace, &folded, &record)?
            .into_iter()
            .find(|node| node.node_execution_id.as_deref() == Some(node_execution_id.as_str()))
            .map(|node| node.id))
    }
}

fn execution_summary_fact(execution: &WorkflowExecutionMetadataRecord) -> WorkspaceStructureFact {
    WorkspaceStructureFact::WorkflowSummaryProjected {
        execution_id: execution.execution_id.clone(),
        workflow_name: execution.workflow_name.clone(),
        status: execution.status,
        updated_at: f64::from_bits(execution.updated_at_bits),
    }
}

#[cfg(test)]
fn sql_query_error(error: rusqlite::Error) -> LocalEventQueryError {
    if let rusqlite::Error::SqliteFailure(inner, _) = &error {
        if matches!(
            inner.code,
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
        ) {
            return LocalEventQueryError::QueryBusy;
        }
        if matches!(
            inner.code,
            rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
        ) {
            return store_corruption_query_error(error);
        }
    }
    match error {
        rusqlite::Error::FromSqlConversionFailure(..)
        | rusqlite::Error::IntegralValueOutOfRange(..)
        | rusqlite::Error::InvalidColumnIndex(..)
        | rusqlite::Error::InvalidColumnName(..)
        | rusqlite::Error::InvalidColumnType(..) => codec_query_error(error.to_string()),
        other => {
            let correlation_id = uuid::Uuid::new_v4().to_string();
            log::warn!("Workspace indexed query failure [{correlation_id}]: {other}");
            LocalEventQueryError::StorageUnavailable {
                failure: SafeOperationFailure::new(
                    SessionOperationFailureKind::StorageUnavailable,
                    true,
                    "Workspace indexed query failed",
                    correlation_id,
                ),
            }
        }
    }
}

#[cfg(test)]
fn store_corruption_query_error(error: impl std::fmt::Display) -> LocalEventQueryError {
    let correlation_id = uuid::Uuid::new_v4().to_string();
    log::error!("Workspace indexed store corruption [{correlation_id}]: {error}");
    LocalEventQueryError::Corrupt { correlation_id }
}

pub(super) fn fold_query_error(reason: String) -> LocalEventQueryError {
    codec_query_error(reason)
}

#[cfg(test)]
mod legacy_projection_tests {
    use super::*;
    use crate::adaptor::gateway::local_event_store::layout::StoreLayout;
    use crate::adaptor::gateway::local_event_store::store::LocalEventStoreConfig;

    #[tokio::test]
    async fn legacy_agent_projection_row_is_ignored_by_canonical_session_and_workspace_queries() {
        let root = tempfile::TempDir::new().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(root.path().to_path_buf()))
                .unwrap();

        let connection =
            rusqlite::Connection::open(StoreLayout::new(root.path()).database_path()).unwrap();
        connection
            .execute(
                "INSERT INTO logical_commits (
                    commit_id, installation_id, operation_kind, idempotency_key, payload_hash,
                    state, first_global_sequence, last_global_sequence, event_count,
                    mutation_count, stream_heads_json, result_hash, committed_at_ms
                 ) VALUES (?1, 'legacy-install', 'projection', 'legacy-key', ?2,
                    'sealed', NULL, NULL, 0, 1, '[]', ?2, 0)",
                rusqlite::params!["legacy-commit", [0_u8; 32].as_slice()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO session_projection (
                    session_id, projection, revision, commit_id, workspace_identity,
                    public_list_kind, public_sort_key_bits, public_summary
                 ) VALUES (?1, ?2, 0, ?3, ?4, 'active', 0, ?5)",
                rusqlite::params![
                    "legacy-session-1",
                    r#"{"schema":"legacy_agent_session_projection_v0"}"#,
                    "legacy-commit",
                    "/repo",
                    r#"{"schema":"legacy_session_public_summary_v0"}"#,
                ],
            )
            .unwrap();
        drop(connection);

        let repository = SqliteWorkspaceTreeRepository::new(store);
        let trees = repository.folded_workspace_trees("/repo").unwrap();
        assert!(repository
            .workspace_tree_from_folded("/repo", &trees)
            .unwrap()
            .is_none());
    }
}

pub(super) fn codec_query_error(error: String) -> LocalEventQueryError {
    let correlation_id = uuid::Uuid::new_v4().to_string();
    log::error!("Workspace indexed record codec failure [{correlation_id}]: {error}");
    LocalEventQueryError::IncompatibleStoredEvent { correlation_id }
}

fn invariant_query_error(error: impl std::fmt::Display) -> LocalEventQueryError {
    let correlation_id = uuid::Uuid::new_v4().to_string();
    log::error!("Workspace indexed record invariant failure [{correlation_id}]: {error}");
    LocalEventQueryError::Corrupt { correlation_id }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlite_failure(code: i32) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None)
    }

    #[test]
    fn sqlite_query_errors_preserve_store_and_record_failure_classification() {
        for code in [rusqlite::ffi::SQLITE_CORRUPT, rusqlite::ffi::SQLITE_NOTADB] {
            assert!(matches!(
                sql_query_error(sqlite_failure(code)),
                LocalEventQueryError::Corrupt { .. }
            ));
        }
        for code in [rusqlite::ffi::SQLITE_BUSY, rusqlite::ffi::SQLITE_LOCKED] {
            assert!(matches!(
                sql_query_error(sqlite_failure(code)),
                LocalEventQueryError::QueryBusy
            ));
        }
        assert!(matches!(
            sql_query_error(rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid value",
                )),
            )),
            LocalEventQueryError::IncompatibleStoredEvent { .. }
        ));
        assert!(matches!(
            sql_query_error(sqlite_failure(rusqlite::ffi::SQLITE_IOERR)),
            LocalEventQueryError::StorageUnavailable { .. }
        ));
    }
}
