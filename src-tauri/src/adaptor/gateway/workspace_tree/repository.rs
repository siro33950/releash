use std::sync::Arc;

use rusqlite::{params, OptionalExtension};

use crate::adaptor::gateway::local_event_store::indexed_projection_codec::{
    decode_workflow_execution_node_detail_v1, decode_workflow_execution_node_tree_v1,
    decode_workflow_execution_record_v1,
};
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::store::LocalEventStore;
use crate::domain::local_event::{
    LocalEventQueryError, SafeOperationFailure, SessionOperationFailureKind,
};
use crate::domain::workspace_tree::{
    WorkspaceIdentity, WorkspaceStructureFact, WorkspaceTree, WorkspaceTreeNode,
    WorkspaceTreeProjector, WorkspaceTreeRepository,
};

pub(crate) const SQL_WORKSPACE_TREE_NODES: &str = "SELECT node.tree_record
     FROM workflow_executions AS execution
     JOIN workflow_execution_nodes AS node
       ON node.execution_id = execution.execution_id
     WHERE execution.workspace_identity = ?1";
pub(crate) const SQL_WORKSPACE_TREE_EXECUTIONS: &str = "SELECT record FROM workflow_executions
     WHERE workspace_identity = ?1";
pub(crate) const SQL_WORKFLOW_NODE_DETAIL: &str = "SELECT node.tree_record, node.detail_record
     FROM workflow_execution_nodes AS node
     JOIN workflow_executions AS execution
       ON execution.execution_id = node.execution_id
     WHERE node.node_id = ?1 AND execution.workspace_identity = ?2";
pub(crate) const SQL_WORKFLOW_NODE_BY_NODE_EXECUTION: &str =
    "SELECT tree_record, detail_record FROM workflow_execution_nodes
     WHERE node_execution_id = ?1";
pub(crate) const SQL_WORKFLOW_NODE_ID_FOR_SESSION: &str = "SELECT node.node_id
     FROM workflow_execution_nodes AS node
     JOIN workflow_executions AS execution
       ON execution.execution_id = node.execution_id
     WHERE node.session_id = ?1 AND execution.workspace_identity = ?2";

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

    pub(super) fn run_indexed<T, F>(&self, run: F) -> Result<T, LocalEventQueryError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection) -> Result<T, LocalEventQueryError> + Send + 'static,
    {
        match &self.backend {
            WorkspaceSqliteBackend::Live(store) => store.submit_indexed_query_blocking(run),
            WorkspaceSqliteBackend::ReadOnly(store) => store.submit_indexed_query_blocking(run),
        }
    }
}

impl WorkspaceTreeRepository for SqliteWorkspaceTreeRepository {
    fn load(
        &self,
        workspace_identity: &WorkspaceIdentity,
    ) -> Result<Option<WorkspaceTree>, LocalEventQueryError> {
        let workspace = workspace_identity.as_str().to_string();
        self.run_indexed(move |connection| {
            let mut node_statement = connection
                .prepare(SQL_WORKSPACE_TREE_NODES)
                .map_err(sql_query_error)?;
            let nodes = node_statement
                .query_map(params![workspace], |row| row.get::<_, String>(0))
                .map_err(sql_query_error)?
                .map(|row| {
                    row.map_err(sql_query_error).and_then(|raw| {
                        decode_workflow_execution_node_tree_v1(&raw).map_err(codec_query_error)
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

            let mut execution_statement = connection
                .prepare(SQL_WORKSPACE_TREE_EXECUTIONS)
                .map_err(sql_query_error)?;
            let facts = execution_statement
                .query_map(params![workspace], |row| row.get::<_, String>(0))
                .map_err(sql_query_error)?
                .map(|row| {
                    row.map_err(sql_query_error)
                        .and_then(|raw| {
                            decode_workflow_execution_record_v1(&raw).map_err(codec_query_error)
                        })
                        .map(execution_summary_fact)
                })
                .collect::<Result<Vec<_>, _>>()?;

            if nodes.is_empty() && facts.is_empty() {
                return Ok(None);
            }
            let mut tree =
                WorkspaceTree::restore(workspace, nodes).map_err(invariant_query_error)?;
            WorkspaceTreeProjector::project(&mut tree, facts).map_err(invariant_query_error)?;
            Ok(Some(tree))
        })
    }

    fn load_node(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceTreeNode>, LocalEventQueryError> {
        let workspace = workspace_identity.as_str().to_string();
        let requested = node_id.to_string();
        let lookup_workspace = workspace.clone();
        let lookup_requested = requested.clone();
        let workflow_node = self.run_indexed(move |connection| {
            connection
                .query_row(
                    SQL_WORKFLOW_NODE_DETAIL,
                    params![lookup_requested, lookup_workspace],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(sql_query_error)
        })?;
        if let Some((tree_record, detail_record)) = workflow_node {
            return decode_workflow_execution_node_detail_v1(&tree_record, &detail_record)
                .map(Some)
                .map_err(codec_query_error);
        }

        Ok(None)
    }

    fn load_node_by_node_execution_id(
        &self,
        node_execution_id: &str,
    ) -> Result<Option<WorkspaceTreeNode>, LocalEventQueryError> {
        let requested = node_execution_id.to_string();
        let workflow_node = self.run_indexed(move |connection| {
            connection
                .query_row(
                    SQL_WORKFLOW_NODE_BY_NODE_EXECUTION,
                    params![requested],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()
                .map_err(sql_query_error)
        })?;
        workflow_node
            .map(|(tree_record, detail_record)| {
                decode_workflow_execution_node_detail_v1(&tree_record, &detail_record)
                    .map_err(codec_query_error)
            })
            .transpose()
    }

    fn node_id_for_session(
        &self,
        workspace_identity: &WorkspaceIdentity,
        session_id: &str,
    ) -> Result<Option<String>, LocalEventQueryError> {
        let workspace = workspace_identity.as_str().to_string();
        let session = session_id.to_string();
        self.run_indexed(move |connection| {
            let workflow_node = connection
                .query_row(
                    SQL_WORKFLOW_NODE_ID_FOR_SESSION,
                    params![session, workspace],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(sql_query_error)?;
            Ok(workflow_node)
        })
    }
}

fn execution_summary_fact(
    execution: crate::domain::local_event::WorkflowExecutionMetadataRecord,
) -> WorkspaceStructureFact {
    WorkspaceStructureFact::WorkflowSummaryProjected {
        execution_id: execution.execution_id,
        workflow_name: execution.workflow_name,
        status: execution.status,
        updated_at: f64::from_bits(execution.updated_at_bits),
    }
}

pub(super) fn sql_query_error(error: rusqlite::Error) -> LocalEventQueryError {
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
mod legacy_projection_tests {
    use super::*;
    use crate::adaptor::gateway::local_event_store::layout::StoreLayout;
    use crate::adaptor::gateway::local_event_store::store::LocalEventStoreConfig;
    use crate::domain::local_event::{
        LocalEventQuery, LocalEventQueryResult, LocalEventTransactionRepository,
    };

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

        let result = store
            .query(LocalEventQuery::AgentSessionProjectionPage {
                workspace_identity: "/repo".to_string(),
                lifecycle: None,
                origin: None,
                limit: 100,
                after_agent_session_id: None,
            })
            .await
            .unwrap();
        let LocalEventQueryResult::AgentSessionProjectionPage(page) = result else {
            panic!("canonical AgentSession page expected");
        };
        assert!(page.sessions.is_empty());

        let repository = SqliteWorkspaceTreeRepository::new(store);
        assert!(repository
            .load(&WorkspaceIdentity::new("/repo"))
            .unwrap()
            .is_none());
    }
}

fn store_corruption_query_error(error: impl std::fmt::Display) -> LocalEventQueryError {
    let correlation_id = uuid::Uuid::new_v4().to_string();
    log::error!("Workspace indexed store corruption [{correlation_id}]: {error}");
    LocalEventQueryError::Corrupt { correlation_id }
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

    #[test]
    fn snapshot_mode_never_materializes_command_payloads() {
        assert!(!SQL_WORKSPACE_TREE_NODES.contains("detail_record"));
        assert!(!SQL_WORKSPACE_TREE_NODES.contains("display_command"));
        assert!(!SQL_WORKSPACE_TREE_NODES.contains("command_result"));
        assert!(SQL_WORKFLOW_NODE_DETAIL.contains("detail_record"));
    }
}
