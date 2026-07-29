use std::sync::Arc;

use rusqlite::{params, OptionalExtension};

use crate::adaptor::gateway::local_event_store::indexed_projection_codec::{
    decode_session_public_summary_v1, decode_workflow_execution_node_detail_v1,
    decode_workflow_execution_node_tree_v1, decode_workflow_execution_record_v1,
};
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::store::LocalEventStore;
use crate::domain::local_event::{
    LocalEventQueryError, SafeOperationFailure, SessionOperationFailureKind,
};
use crate::domain::workspace_tree::{
    WorkspaceIdentity, WorkspaceSessionPublicationPolicy, WorkspaceStructureFact, WorkspaceTree,
    WorkspaceTreeNode, WorkspaceTreeProjector, WorkspaceTreeRepository,
};

pub(crate) const SQL_WORKSPACE_TREE_NODES: &str = "SELECT node.tree_record
     FROM workflow_executions AS execution
     JOIN workflow_execution_nodes AS node
       ON node.execution_id = execution.execution_id
     WHERE execution.workspace_identity = ?1";
pub(crate) const SQL_WORKSPACE_TREE_EXECUTIONS: &str = "SELECT record FROM workflow_executions
     WHERE workspace_identity = ?1";
pub(crate) const SQL_WORKSPACE_TREE_ACTIVE_SESSIONS: &str =
    "SELECT public_summary FROM session_projection
     WHERE workspace_identity = ?1
       AND public_list_kind = 'active'
       AND public_summary IS NOT NULL";
pub(crate) const SQL_WORKFLOW_NODE_DETAIL: &str = "SELECT node.tree_record, node.detail_record
     FROM workflow_execution_nodes AS node
     JOIN workflow_executions AS execution
       ON execution.execution_id = node.execution_id
     WHERE node.node_id = ?1 AND execution.workspace_identity = ?2";
pub(crate) const SQL_SESSION_NODE_DETAIL_FALLBACK: &str =
    "SELECT public_summary FROM session_projection
     WHERE workspace_identity = ?1
       AND public_summary IS NOT NULL
       AND json_extract(public_summary, '$.node_id') = ?2";
pub(crate) const SQL_WORKFLOW_NODE_ID_FOR_SESSION: &str = "SELECT node.node_id
     FROM workflow_execution_nodes AS node
     JOIN workflow_executions AS execution
       ON execution.execution_id = node.execution_id
     WHERE node.session_id = ?1 AND execution.workspace_identity = ?2";
pub(crate) const SQL_DIRECT_NODE_ID_FOR_SESSION: &str =
    "SELECT json_extract(public_summary, '$.node_id')
     FROM session_projection
     WHERE session_id = ?1
       AND workspace_identity = ?2
       AND public_summary IS NOT NULL";

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
            let mut facts = execution_statement
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

            let mut session_statement = connection
                .prepare(SQL_WORKSPACE_TREE_ACTIVE_SESSIONS)
                .map_err(sql_query_error)?;
            facts.extend(
                session_statement
                    .query_map(params![workspace], |row| row.get::<_, String>(0))
                    .map_err(sql_query_error)?
                    .map(|row| {
                        row.map_err(sql_query_error)
                            .and_then(|raw| {
                                decode_session_public_summary_v1(&raw).map_err(codec_query_error)
                            })
                            .map(|summary| {
                                WorkspaceSessionPublicationPolicy::structure_fact(&summary, None)
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );

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

        self.run_indexed(move |connection| {
            let raw = connection
                .query_row(
                    SQL_SESSION_NODE_DETAIL_FALLBACK,
                    params![workspace, requested],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(sql_query_error)?;
            direct_session_tree(raw, workspace)
                .map(|tree| tree.and_then(|tree| tree.nodes().first().cloned()))
        })
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
            if workflow_node.is_some() {
                return Ok(workflow_node);
            }
            connection
                .query_row(
                    SQL_DIRECT_NODE_ID_FOR_SESSION,
                    params![session, workspace],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(sql_query_error)
        })
    }
}

fn direct_session_tree(
    raw: Option<String>,
    workspace: String,
) -> Result<Option<WorkspaceTree>, LocalEventQueryError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let summary = decode_session_public_summary_v1(&raw).map_err(codec_query_error)?;
    let mut tree = WorkspaceTree::empty(workspace);
    WorkspaceTreeProjector::project(
        &mut tree,
        [WorkspaceSessionPublicationPolicy::structure_fact(
            &summary, None,
        )],
    )
    .map_err(invariant_query_error)?;
    Ok(Some(tree))
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
    use crate::adaptor::gateway::local_event_store::indexed_projection_codec::encode_session_public_summary_v1;
    use crate::domain::local_event::{AgentSessionStateRecord, AgentSessionSummaryRecord};

    fn sqlite_failure(code: i32) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None)
    }

    fn summary(updated_at: f64) -> AgentSessionSummaryRecord {
        AgentSessionSummaryRecord {
            id: "direct-session".to_string(),
            worktree_path: "/repo".to_string(),
            state: AgentSessionStateRecord::Idle,
            error_reason: None,
            created_at_bits: 1.0f64.to_bits(),
            updated_at_bits: updated_at.to_bits(),
            first_message: "Review this change".to_string(),
            message_count: 1,
            agent_session_id: Some("backend-session".to_string()),
            context_carry: None,
            permission_mode: "default".to_string(),
            plan_mode: false,
            permission_profile_id: None,
            backend_id: Some("codex".to_string()),
            workflow_node_session: false,
            workflow_node_context: None,
        }
    }

    #[test]
    fn direct_session_tree_distinguishes_empty_and_single_node_records() {
        assert!(direct_session_tree(None, "/repo".to_string())
            .unwrap()
            .is_none());

        let raw = encode_session_public_summary_v1(&summary(2.0)).unwrap();
        let tree = direct_session_tree(Some(raw), "/repo".to_string())
            .unwrap()
            .expect("one public Session record must materialize a tree");
        assert_eq!(tree.nodes().len(), 1);
        let node = tree.session_node("direct-session").unwrap();
        assert_eq!(node.title, "Review this change");
        assert_eq!(node.session_id.as_deref(), Some("direct-session"));
    }

    #[test]
    fn corrupt_record_is_incompatible_while_non_finite_fact_is_corrupt() {
        assert!(matches!(
            direct_session_tree(Some("{not-json".to_string()), "/repo".to_string()),
            Err(LocalEventQueryError::IncompatibleStoredEvent { .. })
        ));

        let raw = encode_session_public_summary_v1(&summary(f64::NAN)).unwrap();
        assert!(matches!(
            direct_session_tree(Some(raw), "/repo".to_string()),
            Err(LocalEventQueryError::Corrupt { .. })
        ));
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
