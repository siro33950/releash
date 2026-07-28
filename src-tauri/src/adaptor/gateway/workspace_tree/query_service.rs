use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rusqlite::{params, OptionalExtension as _};

use super::repository::{codec_query_error, sql_query_error};
use super::SqliteWorkspaceTreeRepository;
use crate::adaptor::gateway::local_event_store::indexed_projection_codec::{
    decode_session_public_summary_v1, decode_workflow_execution_record_v1,
};
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::store::LocalEventStore;
use crate::domain::local_event::AgentSessionSummaryRecord;
use crate::domain::workflow::{
    ExecutionStatusFilter, WorkflowError, WorkflowExecutionArchiveRepository,
    WorkflowExecutionSummary, WorkflowPageRequest, WORKFLOW_ARCHIVE_REASON_MANUAL,
};
use crate::domain::workspace_tree::{
    WorkspaceIdentity, WorkspaceNodeKind, WorkspaceSessionListKind, WorkspaceTree,
    WorkspaceTreeNode, WorkspaceTreeRepository, WorkspaceTreeVisibilityPolicy,
};
use crate::usecase::agent_session::session::{session_summary_from_record, SessionSummary};
use crate::usecase::workflow::{
    WorkspaceCommandNodeContentDto, WorkspaceCommandResultDto, WorkspaceFanoutDto,
    WorkspaceNodeCapabilitiesDto, WorkspaceNodeContentDto, WorkspaceNodeDetailDto,
    WorkspaceNodeDto, WorkspaceSessionNodeContentDto, WorkspaceTreeItemDto,
    WorkspaceTreeSnapshotDto, WorkspaceWorkflowCapabilitiesDto, WorkspaceWorkflowDto,
    WorkspaceWorkflowHistoryItemDto,
};
use crate::usecase::workspace_tree::{
    WorkspaceNodeApprovalRoute, WorkspaceNodeCloseRoute, WorkspaceQueryService,
};

pub(crate) const SQL_SESSION_RECORDS: &str = "SELECT public_summary FROM session_projection
     WHERE workspace_identity = ?1 AND public_list_kind = ?2
     ORDER BY public_sort_key_bits DESC, session_id";
pub(crate) const SQL_EXECUTIONS_BY_WORKSPACE_AND_KIND: &str =
    "SELECT record FROM workflow_executions
     WHERE workspace_identity = ?1 AND list_kind = ?2
     ORDER BY sort_at_bits DESC, execution_id
     LIMIT ?3 OFFSET ?4";
pub(crate) const SQL_EXECUTIONS_BY_WORKSPACE: &str = "SELECT record FROM workflow_executions
     WHERE workspace_identity = ?1
     ORDER BY list_kind, sort_at_bits DESC, execution_id
     LIMIT ?3 OFFSET ?4";
pub(crate) const SQL_EXECUTIONS_BY_KIND: &str = "SELECT record FROM workflow_executions
     WHERE list_kind = ?2
     ORDER BY sort_at_bits DESC, execution_id
     LIMIT ?3 OFFSET ?4";
pub(crate) const SQL_EXECUTIONS_ALL: &str = "SELECT record FROM workflow_executions
     ORDER BY list_kind, sort_at_bits DESC, execution_id
     LIMIT ?3 OFFSET ?4";

pub(crate) struct SqliteWorkspaceQueryService {
    repository: Arc<SqliteWorkspaceTreeRepository>,
    archives: Arc<dyn WorkflowExecutionArchiveRepository>,
}

impl SqliteWorkspaceQueryService {
    pub(crate) fn new(
        store: Arc<LocalEventStore>,
        archives: Arc<dyn WorkflowExecutionArchiveRepository>,
    ) -> Arc<Self> {
        Arc::new(Self {
            repository: SqliteWorkspaceTreeRepository::new(store),
            archives,
        })
    }

    pub(crate) fn new_read_only(
        store: Arc<LocalEventReadStore>,
        archives: Arc<dyn WorkflowExecutionArchiveRepository>,
    ) -> Arc<Self> {
        Arc::new(Self {
            repository: SqliteWorkspaceTreeRepository::new_read_only(store),
            archives,
        })
    }

    fn session_records(
        &self,
        workspace_identity: &WorkspaceIdentity,
        list: WorkspaceSessionListKind,
    ) -> Result<Vec<AgentSessionSummaryRecord>, WorkflowError> {
        let workspace = workspace_identity.as_str().to_string();
        let list = list.label().to_string();
        self.repository
            .run_indexed(move |connection| {
                let mut statement = connection
                    .prepare(SQL_SESSION_RECORDS)
                    .map_err(sql_query_error)?;
                let records = statement
                    .query_map(params![workspace, list], |row| row.get::<_, String>(0))
                    .map_err(sql_query_error)?
                    .map(|row| {
                        row.map_err(sql_query_error).and_then(|raw| {
                            decode_session_public_summary_v1(&raw).map_err(codec_query_error)
                        })
                    })
                    .collect();
                records
            })
            .map_err(query_error)
    }

    fn execution_records(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
        page: Option<WorkflowPageRequest>,
    ) -> Result<Vec<crate::domain::local_event::WorkflowExecutionMetadataRecord>, WorkflowError>
    {
        let workspace = workspace_identity.map(|identity| identity.as_str().to_string());
        let list_kind = status.map(|filter| match filter {
            ExecutionStatusFilter::Active => "active".to_string(),
            ExecutionStatusFilter::Terminal => "terminal".to_string(),
        });
        let (limit, offset) = page
            .map(|page| {
                (
                    i64::try_from(page.limit).unwrap_or(i64::MAX),
                    i64::try_from(page.offset).unwrap_or(i64::MAX),
                )
            })
            .unwrap_or((i64::MAX, 0));
        self.repository
            .run_indexed(move |connection| {
                let (sql, first, second) = match (&workspace, &list_kind) {
                    (Some(_), Some(_)) => (
                        SQL_EXECUTIONS_BY_WORKSPACE_AND_KIND,
                        workspace.clone(),
                        list_kind.clone(),
                    ),
                    (Some(_), None) => (SQL_EXECUTIONS_BY_WORKSPACE, workspace.clone(), None),
                    (None, Some(_)) => (SQL_EXECUTIONS_BY_KIND, None, list_kind.clone()),
                    (None, None) => (SQL_EXECUTIONS_ALL, None, None),
                };
                let mut statement = connection.prepare(sql).map_err(sql_query_error)?;
                let records = statement
                    .query_map(params![first, second, limit, offset], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(sql_query_error)?
                    .map(|row| {
                        row.map_err(sql_query_error).and_then(|raw| {
                            decode_workflow_execution_record_v1(&raw).map_err(codec_query_error)
                        })
                    })
                    .collect();
                records
            })
            .map_err(query_error)
    }
}

impl WorkspaceQueryService for SqliteWorkspaceQueryService {
    fn workspace_tree(
        &self,
        workspace_identity: &WorkspaceIdentity,
    ) -> Result<WorkspaceTreeSnapshotDto, WorkflowError> {
        let tree = self
            .repository
            .load(workspace_identity)
            .map_err(query_error)?
            .unwrap_or_else(|| WorkspaceTree::empty(workspace_identity.as_str()));
        let execution_ids = tree
            .nodes()
            .iter()
            .filter_map(|node| node.execution_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let archive = self.archives.manual_archive_snapshot_for(&execution_ids)?;
        let hidden = WorkspaceTreeVisibilityPolicy::hidden_branch_ids(
            &tree,
            archive
                .records
                .iter()
                .map(|record| record.execution_id.as_str()),
        );
        let preferred_node_id = tree.preferred_node_id(&hidden);
        let nodes = project_tree(&tree, &hidden);
        Ok(WorkspaceTreeSnapshotDto {
            nodes,
            preferred_node_id,
        })
    }

    fn node_detail(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError> {
        self.repository
            .load_node(workspace_identity, node_id)
            .map(|node| node.map(node_detail))
            .map_err(query_error)
    }

    fn session_node_id(
        &self,
        workspace_identity: &WorkspaceIdentity,
        session_id: &str,
    ) -> Result<Option<String>, WorkflowError> {
        self.repository
            .node_id_for_session(workspace_identity, session_id)
            .map_err(query_error)
    }

    fn node_approval_command(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<WorkspaceNodeApprovalRoute, WorkflowError> {
        let node = self
            .repository
            .load_node(workspace_identity, node_id)
            .map_err(query_error)?;
        node_approval_route(node)
    }

    fn node_close_session_id(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<WorkspaceNodeCloseRoute, WorkflowError> {
        let node = self
            .repository
            .load_node(workspace_identity, node_id)
            .map_err(query_error)?;
        Ok(node_close_route(node))
    }

    fn session_summaries(
        &self,
        workspace_identity: &WorkspaceIdentity,
        list: WorkspaceSessionListKind,
    ) -> Result<Vec<SessionSummary>, WorkflowError> {
        self.session_records(workspace_identity, list)?
            .into_iter()
            .map(session_summary)
            .collect()
    }

    fn execution_summaries(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
        page: Option<WorkflowPageRequest>,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError> {
        self.execution_records(workspace_identity, status, page)?
            .into_iter()
            .map(execution_summary)
            .collect()
    }

    fn execution_summary(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        let execution_id = execution_id.to_string();
        self.repository
            .run_indexed(move |connection| {
                connection
                    .query_row(
                        "SELECT record FROM workflow_executions WHERE execution_id = ?1",
                        params![execution_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(sql_query_error)?
                    .map(|raw| decode_workflow_execution_record_v1(&raw).map_err(codec_query_error))
                    .transpose()
            })
            .map_err(query_error)?
            .map(execution_summary)
            .transpose()
    }

    fn workflow_history(
        &self,
        workspace_identity: &WorkspaceIdentity,
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkflowError> {
        let summaries = self
            .execution_records(Some(workspace_identity), None, None)?
            .into_iter()
            .map(|summary| (summary.execution_id.clone(), summary))
            .collect::<HashMap<_, _>>();
        let archive = self
            .archives
            .manual_archive_snapshot_for(&summaries.keys().cloned().collect::<Vec<_>>())?;
        let mut history = archive
            .records
            .into_iter()
            .filter_map(|record| {
                summaries
                    .get(&record.execution_id)
                    .map(|summary| WorkspaceWorkflowHistoryItemDto {
                        execution_id: record.execution_id,
                        worktree_path: summary.worktree_path.clone(),
                        title: summary.workflow_name.clone(),
                        status: summary.status.as_str().to_string(),
                        updated_at: f64::from_bits(summary.updated_at_bits),
                        archived_at: record.archived_at,
                        archive_reason: WORKFLOW_ARCHIVE_REASON_MANUAL.to_string(),
                    })
            })
            .collect::<Vec<_>>();
        history.sort_by(|left, right| {
            right
                .archived_at
                .total_cmp(&left.archived_at)
                .then_with(|| left.execution_id.cmp(&right.execution_id))
        });
        Ok(history)
    }
}

fn node_approval_route(
    node: Option<WorkspaceTreeNode>,
) -> Result<WorkspaceNodeApprovalRoute, WorkflowError> {
    let Some(node) = node else {
        return Ok(WorkspaceNodeApprovalRoute::Missing);
    };
    if !node.can_approve {
        return Ok(WorkspaceNodeApprovalRoute::NotWaiting);
    }
    let (Some(execution_id), Some(node_execution_id), Some(node_name)) =
        (node.execution_id, node.node_execution_id, node.node_name)
    else {
        return Err(record_projection_error(
            "Workspace approval routing record is corrupt",
        ));
    };
    Ok(WorkspaceNodeApprovalRoute::Command(
        crate::usecase::workflow::command::ApprovalCommand {
            execution_id,
            node_name,
            node_execution_id: Some(node_execution_id),
            comment: None,
        },
    ))
}

fn node_close_route(node: Option<WorkspaceTreeNode>) -> WorkspaceNodeCloseRoute {
    let Some(node) = node else {
        return WorkspaceNodeCloseRoute::Missing;
    };
    match (node.can_close, node.session_id) {
        (true, Some(session_id)) => WorkspaceNodeCloseRoute::Session(session_id),
        _ => WorkspaceNodeCloseRoute::NotSupported,
    }
}

fn project_tree(tree: &WorkspaceTree, hidden: &HashSet<String>) -> Vec<WorkspaceTreeItemDto> {
    let mut children: HashMap<Option<&str>, Vec<&WorkspaceTreeNode>> = HashMap::new();
    for node in tree.nodes() {
        if !node.is_internal_rule_record() {
            children
                .entry(node.parent_id.as_deref())
                .or_default()
                .push(node);
        }
    }
    for siblings in children.values_mut() {
        siblings.sort_by_key(|node| (node.sibling_order, node.id.as_str()));
    }
    fn branch(
        parent: Option<&str>,
        children: &HashMap<Option<&str>, Vec<&WorkspaceTreeNode>>,
        hidden: &HashSet<String>,
    ) -> Vec<WorkspaceTreeItemDto> {
        children
            .get(&parent)
            .into_iter()
            .flatten()
            .filter(|node| !hidden.contains(&node.id))
            .map(|node| match node.kind {
                WorkspaceNodeKind::Workflow => {
                    WorkspaceTreeItemDto::Workflow(WorkspaceWorkflowDto {
                        id: node.id.clone(),
                        title: node.title.clone(),
                        status: node.status.as_public_str().to_string(),
                        capabilities: WorkspaceWorkflowCapabilitiesDto {
                            can_stop: node.can_stop,
                            can_resume: node.can_resume,
                            resume_unavailable_reason: node.resume_unavailable_reason.clone(),
                            can_abort: node.can_abort,
                            can_archive: node.can_archive,
                        },
                        children: branch(Some(&node.id), children, hidden),
                        updated_at: node.updated_at(),
                    })
                }
                WorkspaceNodeKind::Fanout => WorkspaceTreeItemDto::Fanout(WorkspaceFanoutDto {
                    id: node.id.clone(),
                    title: node.title.clone(),
                    status: node.status.as_public_str().to_string(),
                    children: branch(Some(&node.id), children, hidden),
                    updated_at: node.updated_at(),
                }),
                _ => WorkspaceTreeItemDto::Node(WorkspaceNodeDto {
                    id: node.id.clone(),
                    title: node.title.clone(),
                    status: node.status.as_public_str().to_string(),
                    error_reason: node.error_reason.clone(),
                    content_kind: if node.kind == WorkspaceNodeKind::WorkflowCommand {
                        "command"
                    } else {
                        "session"
                    },
                    capabilities: WorkspaceNodeCapabilitiesDto {
                        can_approve: node.can_approve,
                        can_close: node.can_close,
                    },
                    updated_at: node.updated_at(),
                }),
            })
            .collect()
    }
    branch(None, &children, hidden)
}

fn node_detail(node: WorkspaceTreeNode) -> WorkspaceNodeDetailDto {
    let updated_at = node.updated_at();
    let content = if node.kind == WorkspaceNodeKind::WorkflowCommand {
        WorkspaceNodeContentDto::Command(WorkspaceCommandNodeContentDto {
            display_command: node.display_command,
            result: node.command_result.map(|result| WorkspaceCommandResultDto {
                exit_code: result.exit_code,
                duration: result.duration,
                stdout: result.stdout,
                stderr: result.stderr,
            }),
        })
    } else {
        WorkspaceNodeContentDto::Session(WorkspaceSessionNodeContentDto {
            session_id: node.session_id,
        })
    };
    WorkspaceNodeDetailDto {
        id: node.id,
        title: node.title,
        status: node.status.as_public_str().to_string(),
        error_reason: node.error_reason,
        capabilities: WorkspaceNodeCapabilitiesDto {
            can_approve: node.can_approve,
            can_close: node.can_close,
        },
        updated_at,
        content,
    }
}

fn session_summary(record: AgentSessionSummaryRecord) -> Result<SessionSummary, WorkflowError> {
    session_summary_from_record(&record).map_err(|error| record_projection_error(&error))
}

fn execution_summary(
    record: crate::domain::local_event::WorkflowExecutionMetadataRecord,
) -> Result<WorkflowExecutionSummary, WorkflowError> {
    let started_at = f64::from_bits(record.started_at_bits);
    let updated_at = f64::from_bits(record.updated_at_bits);
    let completed_at = record.completed_at_bits.map(f64::from_bits);
    if !started_at.is_finite()
        || !updated_at.is_finite()
        || completed_at.is_some_and(|value| !value.is_finite())
    {
        return Err(record_projection_error(
            "Workspace execution summary contains an invalid timestamp",
        ));
    }
    Ok(WorkflowExecutionSummary {
        execution_id: record.execution_id,
        workflow_name: record.workflow_name,
        status: record.status,
        worktree_path: record.worktree_path,
        current_node: record.current_node,
        created_from: record.created_from,
        started_at,
        updated_at,
        completed_at,
        error_reason: record.error_reason,
        interruption_reason: record.interruption_reason,
        resume_from_node: record.resume_from_node,
        total_token_usage: record.total_token_usage,
    })
}

fn query_error(error: crate::domain::local_event::LocalEventQueryError) -> WorkflowError {
    use crate::domain::local_event::LocalEventQueryError;

    match error {
        LocalEventQueryError::StorageUnavailable { failure } => WorkflowError::StorageUnavailable {
            message: failure.to_string(),
            retryable: failure.retryable,
        },
        error @ (LocalEventQueryError::QueryBusy | LocalEventQueryError::DeadlineExceeded) => {
            WorkflowError::StorageUnavailable {
                message: error.to_string(),
                retryable: true,
            }
        }
        error @ LocalEventQueryError::Corrupt { .. } => {
            WorkflowError::CorruptStoredState(error.to_string())
        }
        error @ LocalEventQueryError::IncompatibleStoredEvent { .. } => {
            WorkflowError::IncompatibleStoredEvent(error.to_string())
        }
        error => WorkflowError::external(error.to_string()),
    }
}

fn record_projection_error(context: &str) -> WorkflowError {
    let correlation_id = uuid::Uuid::new_v4();
    log::error!("Workspace query record invariant failure [{correlation_id}]: {context}");
    WorkflowError::CorruptStoredState(format!("{context} (correlation_id={correlation_id})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::local_event::WorkflowExecutionMetadataRecord;
    use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus, TokenUsage};
    use crate::domain::workspace_tree::{WorkspaceNodeStatus, WorkspaceTreeNode};

    fn node() -> WorkspaceTreeNode {
        WorkspaceTreeNode {
            id: "node".to_string(),
            parent_id: None,
            sibling_order: 0,
            kind: WorkspaceNodeKind::WorkflowSession,
            title: "Review".to_string(),
            status: WorkspaceNodeStatus::Waiting,
            error_reason: None,
            updated_at_bits: 1.0f64.to_bits(),
            execution_id: None,
            node_execution_id: None,
            node_name: None,
            attempt: Some(1),
            session_id: None,
            can_approve: true,
            can_close: false,
            can_stop: false,
            can_resume: false,
            recovery_owner_reason: None,
            resume_unavailable_reason: None,
            can_abort: false,
            can_archive: false,
            display_command: None,
            command_result: None,
            dynamic_fanout: false,
        }
    }

    #[test]
    fn approval_route_rejects_corrupt_waiting_record() {
        assert!(matches!(
            node_approval_route(Some(node())),
            Err(WorkflowError::CorruptStoredState(_))
        ));
    }

    #[test]
    fn close_route_reports_not_supported_without_closeable_session() {
        assert!(matches!(
            node_close_route(Some(node())),
            WorkspaceNodeCloseRoute::NotSupported
        ));
    }

    #[test]
    fn direct_session_close_target_is_materialized_only_for_its_opaque_node() {
        let mut direct = node();
        direct.kind = WorkspaceNodeKind::Session;
        direct.attempt = None;
        direct.session_id = Some("direct-session".to_string());
        direct.can_approve = false;
        direct.can_close = true;

        assert!(matches!(
            node_close_route(Some(direct)),
            WorkspaceNodeCloseRoute::Session(session_id) if session_id == "direct-session"
        ));
        assert!(matches!(
            node_close_route(None),
            WorkspaceNodeCloseRoute::Missing
        ));
    }

    #[test]
    fn error_reason_is_exposed_on_direct_session_badges_and_detail() {
        let mut direct = node();
        direct.kind = WorkspaceNodeKind::Session;
        direct.status = WorkspaceNodeStatus::Error;
        direct.error_reason = Some("provider failed".to_string());
        direct.attempt = None;
        direct.session_id = Some("direct-session".to_string());
        direct.can_approve = false;
        direct.can_close = true;
        let tree = WorkspaceTree::restore("/repo", vec![direct.clone()]).unwrap();

        let badges = project_tree(&tree, &HashSet::new());
        let [WorkspaceTreeItemDto::Node(badge)] = badges.as_slice() else {
            panic!("direct Session must project to one node badge");
        };
        assert_eq!(badge.status, "error");
        assert_eq!(badge.error_reason.as_deref(), Some("provider failed"));

        let detail = node_detail(direct);
        assert_eq!(detail.status, "error");
        assert_eq!(detail.error_reason.as_deref(), Some("provider failed"));
    }

    #[test]
    fn execution_summary_rejects_non_finite_timestamp() {
        let record = WorkflowExecutionMetadataRecord {
            execution_id: "execution".to_string(),
            workflow_name: "workflow".to_string(),
            status: ExecutionStatus::Running,
            worktree_path: "/repo".to_string(),
            current_node: None,
            created_from: ExecutionOrigin::DesktopUi,
            started_at_bits: f64::NAN.to_bits(),
            updated_at_bits: 1.0f64.to_bits(),
            completed_at_bits: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: TokenUsage::default(),
        };
        assert!(matches!(
            execution_summary(record),
            Err(WorkflowError::CorruptStoredState(_))
        ));
    }
}
