use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::SqliteWorkspaceTreeRepository;
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::domain::workflow::{
    ExecutionStatusFilter, TreeRootFact, WorkflowError, WorkflowExecutionArchiveRepository,
    WorkflowExecutionSummary, WorkflowPageRequest, WORKFLOW_ARCHIVE_REASON_MANUAL,
};
use crate::domain::workspace_tree::{
    WorkspaceIdentity, WorkspaceNodeKind, WorkspaceTree, WorkspaceTreeNode,
    WorkspaceTreeRepository, WorkspaceTreeVisibilityPolicy,
};
use crate::usecase::workflow::{
    WorkspaceCommandNodeContentDto, WorkspaceCommandResultDto, WorkspaceFanoutDto,
    WorkspaceNodeCapabilitiesDto, WorkspaceNodeContentDto, WorkspaceNodeDetailDto,
    WorkspaceNodeDto, WorkspaceSessionNodeContentDto, WorkspaceTreeItemDto,
    WorkspaceTreeSnapshotDto, WorkspaceWorkflowCapabilitiesDto, WorkspaceWorkflowDto,
    WorkspaceWorkflowHistoryItemDto,
};
use crate::usecase::workspace_tree::WorkspaceQueryService;

pub(crate) struct SqliteWorkspaceQueryService {
    repository: Arc<SqliteWorkspaceTreeRepository>,
    archives: Arc<dyn WorkflowExecutionArchiveRepository>,
}

impl SqliteWorkspaceQueryService {
    pub(crate) fn with_repository(
        repository: Arc<SqliteWorkspaceTreeRepository>,
        archives: Arc<dyn WorkflowExecutionArchiveRepository>,
    ) -> Arc<Self> {
        Arc::new(Self {
            repository,
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

    fn execution_records(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
        page: Option<WorkflowPageRequest>,
    ) -> Result<Vec<crate::domain::local_event::WorkflowExecutionMetadataRecord>, WorkflowError>
    {
        let backend = self.repository.fact_backend();
        let tree_ids = crate::adaptor::gateway::workflow::fact_log::list_tree_ids(
            &backend,
            workspace_identity.map(|identity| identity.as_str()),
        )
        .map_err(WorkflowError::external)?;
        let mut records = Vec::new();
        for tree_id in tree_ids {
            let Some((_, record)) = self.repository.folded_tree(&tree_id).map_err(query_error)?
            else {
                continue;
            };
            let keep = match status {
                Some(ExecutionStatusFilter::Active) => !record.status.is_finished(),
                Some(ExecutionStatusFilter::Terminal) => record.status.is_finished(),
                None => true,
            };
            if keep {
                records.push(record);
            }
        }
        // 旧一覧と同じ並び: active が先、次に更新時刻の新しい順、最後に id。
        records.sort_by(|left, right| {
            left.status
                .is_finished()
                .cmp(&right.status.is_finished())
                .then_with(|| {
                    f64::from_bits(right.updated_at_bits)
                        .total_cmp(&f64::from_bits(left.updated_at_bits))
                })
                .then_with(|| left.execution_id.cmp(&right.execution_id))
        });
        let (limit, offset) = sqlite_page_bounds(page);
        Ok(records
            .into_iter()
            .skip(usize::try_from(offset).unwrap_or(0))
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .collect())
    }
}

impl WorkspaceQueryService for SqliteWorkspaceQueryService {
    fn workspace_tree(
        &self,
        workspace_identity: &WorkspaceIdentity,
    ) -> Result<WorkspaceTreeSnapshotDto, WorkflowError> {
        let folded = self
            .repository
            .folded_workspace_trees(workspace_identity.as_str())
            .map_err(query_error)?;
        let tree = self
            .repository
            .workspace_tree_from_folded(workspace_identity.as_str(), &folded)
            .map_err(query_error)?
            .unwrap_or_else(|| WorkspaceTree::empty(workspace_identity.as_str()));
        let session_tree_ids = folded
            .iter()
            .filter_map(|(tree, _)| match &tree.root {
                TreeRootFact::Session(root)
                    if root.workspace_identity == workspace_identity.as_str() =>
                {
                    Some(tree.aggregate.id.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
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
            sessions: crate::adaptor::gateway::agent_session::workspace_session_items(
                &self.repository.fact_backend(),
                &session_tree_ids,
                workspace_identity.as_str(),
            )
            .map_err(|error| {
                WorkflowError::external(format!("workspace session query failed: {error:?}"))
            })?,
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
        self.repository
            .folded_tree(execution_id)
            .map_err(query_error)?
            .map(|(_, record)| execution_summary(record))
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
                WorkspaceNodeKind::Fanout | WorkspaceNodeKind::Sequence => {
                    WorkspaceTreeItemDto::Fanout(WorkspaceFanoutDto {
                        id: node.id.clone(),
                        title: node.title.clone(),
                        status: node.status.as_public_str().to_string(),
                        children: branch(Some(&node.id), children, hidden),
                        updated_at: node.updated_at(),
                    })
                }
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
                        can_retry: node.can_retry,
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
    let submit_received = matches!(
        node.completion_signals,
        crate::domain::workflow::NodeCompletionSignalState::SubmitReceived
            | crate::domain::workflow::NodeCompletionSignalState::Ready
    );
    let stop_received = matches!(
        node.completion_signals,
        crate::domain::workflow::NodeCompletionSignalState::StopReceived
            | crate::domain::workflow::NodeCompletionSignalState::Ready
    );
    let waiting_for = match node.completion_signals {
        crate::domain::workflow::NodeCompletionSignalState::SubmitReceived => Some("stop"),
        crate::domain::workflow::NodeCompletionSignalState::StopReceived => Some("submit"),
        crate::domain::workflow::NodeCompletionSignalState::Pending
        | crate::domain::workflow::NodeCompletionSignalState::Ready => None,
    };
    let content = match node.kind {
        WorkspaceNodeKind::WorkflowCommand => {
            WorkspaceNodeContentDto::Command(WorkspaceCommandNodeContentDto {
                display_command: node.display_command,
                result: node.command_result.map(|result| WorkspaceCommandResultDto {
                    exit_code: result.exit_code,
                    duration: result.duration,
                    stdout: result.stdout,
                    stderr: result.stderr,
                }),
            })
        }
        WorkspaceNodeKind::WorkflowSession => {
            WorkspaceNodeContentDto::AgentSession(WorkspaceSessionNodeContentDto {
                session_id: node.session_id,
            })
        }
        _ => WorkspaceNodeContentDto::Session(WorkspaceSessionNodeContentDto {
            session_id: node.session_id,
        }),
    };
    WorkspaceNodeDetailDto {
        id: node.id,
        title: node.title,
        status: node.status.as_public_str().to_string(),
        attempt: node.attempt,
        submit_received,
        stop_received,
        waiting_for,
        has_artifact: node.has_artifact,
        error_reason: node.error_reason,
        recovery_reason: node.recovery_owner_reason,
        capabilities: WorkspaceNodeCapabilitiesDto {
            can_approve: node.can_approve,
            can_retry: node.can_retry,
            can_close: node.can_close,
        },
        updated_at,
        content,
    }
}

fn sqlite_page_bounds(page: Option<WorkflowPageRequest>) -> (i64, i64) {
    page.map(|page| {
        (
            i64::try_from(page.limit).unwrap_or(i64::MAX),
            i64::try_from(page.offset).unwrap_or(0),
        )
    })
    .unwrap_or((i64::MAX, 0))
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
            completion_signals: Default::default(),
            has_artifact: false,
            session_id: None,
            can_approve: true,
            can_retry: false,
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
    fn test_workflow_session_node_detail_agent_session_surfaceを公開する() {
        let mut workflow_session = node();
        workflow_session.session_id = Some("agent-session-1".to_string());

        let detail = serde_json::to_value(node_detail(workflow_session)).unwrap();

        assert_eq!(
            detail["content"]["kind"],
            serde_json::Value::String("agentSession".to_string())
        );
        assert_eq!(
            detail["content"]["sessionId"],
            serde_json::Value::String("agent-session-1".to_string())
        );
    }

    #[test]
    fn workflow_node_detail_exposes_backend_owned_attempt_signal_and_capabilities() {
        let mut workflow_session = node();
        workflow_session.node_execution_id = Some("node-execution-1".to_string());
        workflow_session.execution_id = Some("execution-1".to_string());
        workflow_session.node_name = Some("Review".to_string());
        workflow_session.session_id = Some("agent-session-1".to_string());
        workflow_session.completion_signals =
            crate::domain::workflow::NodeCompletionSignalState::StopReceived;
        workflow_session.has_artifact = false;
        workflow_session.can_retry = true;

        let detail = serde_json::to_value(node_detail(workflow_session)).unwrap();

        assert_eq!(detail["attempt"], 1);
        assert_eq!(detail["submitReceived"], false);
        assert_eq!(detail["stopReceived"], true);
        assert_eq!(detail["waitingFor"], "submit");
        assert_eq!(detail["hasArtifact"], false);
        assert_eq!(detail["capabilities"]["canRetry"], true);
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

    #[test]
    fn unrepresentable_page_offset_falls_back_to_the_first_record() {
        assert_eq!(
            sqlite_page_bounds(Some(WorkflowPageRequest::new(usize::MAX, usize::MAX))),
            (i64::MAX, 0)
        );
        assert_eq!(sqlite_page_bounds(None), (i64::MAX, 0));
    }
}
