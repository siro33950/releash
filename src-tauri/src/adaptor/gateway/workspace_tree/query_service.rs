use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::SqliteWorkspaceTreeRepository;
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::domain::workflow::{
    ExecutionStatusFilter, TreeRootFact, WorkflowError, WorkflowExecutionArchiveRepository,
    WorkflowExecutionSummary, WorkflowPageRequest, WORKFLOW_ARCHIVE_REASON_MANUAL,
};
use crate::domain::workspace_tree::{
    WorkspaceIdentity, WorkspaceNodeKind, WorkspacePublicRoot, WorkspaceTree, WorkspaceTreeNode,
    WorkspaceTreeRepository, WorkspaceTreeVisibilityPolicy,
};
use crate::usecase::agent_session::{AgentSessionItemDto, AgentSessionLifecycleDto};
use crate::usecase::workflow::{
    WorkspaceCommandNodeContentDto, WorkspaceCommandResultDto, WorkspaceFanoutDto,
    WorkspaceNodeCapabilitiesDto, WorkspaceNodeContentDto, WorkspaceNodeDetailDto,
    WorkspaceNodeDto, WorkspaceSequenceDto, WorkspaceSessionCapabilitiesDto,
    WorkspaceSessionNodeContentDto, WorkspaceTreeItemDto, WorkspaceTreeSnapshotDto,
    WorkspaceWorkflowCapabilitiesDto, WorkspaceWorkflowHistoryItemDto,
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
        let tree_roots = crate::adaptor::gateway::workflow::fact_log::list_tree_roots(
            &backend,
            workspace_identity.map(|identity| identity.as_str()),
        )
        .map_err(WorkflowError::external)?;
        let mut records = Vec::new();
        for (tree_id, root) in tree_roots {
            if !matches!(root, TreeRootFact::Workflow(_)) {
                continue;
            }
            let Some((folded, record)) =
                self.repository.folded_tree(&tree_id).map_err(query_error)?
            else {
                continue;
            };
            debug_assert!(matches!(folded.root, TreeRootFact::Workflow(_)));
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
        let session_items = crate::adaptor::gateway::agent_session::workspace_session_items(
            &self.repository.fact_backend(),
            &session_tree_ids,
            workspace_identity.as_str(),
        )
        .map_err(|error| {
            WorkflowError::external(format!("workspace session query failed: {error:?}"))
        })?;
        let workflow_execution_ids = folded
            .iter()
            .filter(|(tree, _)| matches!(tree.root, TreeRootFact::Workflow(_)))
            .map(|(tree, _)| tree.aggregate.id.clone())
            .collect::<HashSet<_>>();
        let execution_ids = workflow_execution_ids.iter().cloned().collect::<Vec<_>>();
        let archive = self.archives.manual_archive_snapshot_for(&execution_ids)?;
        let mut hidden = WorkspaceTreeVisibilityPolicy::hidden_branch_ids(
            &tree,
            archive
                .records
                .iter()
                .map(|record| record.execution_id.as_str()),
        );
        let archived_sessions = session_items
            .iter()
            .filter(|session| session.lifecycle == AgentSessionLifecycleDto::Archived)
            .cloned()
            .collect::<Vec<_>>();
        for session in &archived_sessions {
            if let Some(execution_id) = tree
                .nodes()
                .iter()
                .find(|node| node.session_id.as_deref() == Some(session.id.as_str()))
                .and_then(|node| node.execution_id.clone())
            {
                hidden.insert(execution_id);
            }
        }
        let preferred_node_id = tree
            .preferred_node_id(&hidden)
            .map(|node_id| public_node_id(&tree, &node_id));
        let nodes = project_tree(&tree, &hidden, &workflow_execution_ids, &session_items);
        Ok(WorkspaceTreeSnapshotDto {
            nodes,
            archived_sessions,
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
            .filter(|(folded, _)| matches!(folded.root, TreeRootFact::Workflow(_)))
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

fn public_node_id(tree: &WorkspaceTree, node_id: &str) -> String {
    WorkspacePublicRoot::for_node(tree.nodes(), node_id)
        .map_or_else(|| node_id.to_string(), |root| root.public_id().to_string())
}

fn workflow_capabilities(node: &WorkspaceTreeNode) -> WorkspaceWorkflowCapabilitiesDto {
    WorkspaceWorkflowCapabilitiesDto {
        can_stop: node.can_stop,
        can_resume: node.can_resume,
        resume_unavailable_reason: node.resume_unavailable_reason.clone(),
        can_abort: node.can_abort,
        can_archive: node.can_archive,
    }
}

fn project_tree(
    tree: &WorkspaceTree,
    hidden: &HashSet<String>,
    workflow_execution_ids: &HashSet<String>,
    session_items: &[AgentSessionItemDto],
) -> Vec<WorkspaceTreeItemDto> {
    let mut children: HashMap<Option<&str>, Vec<&WorkspaceTreeNode>> = HashMap::new();
    let by_id = tree
        .nodes()
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let sessions = session_items
        .iter()
        .map(|session| (session.id.as_str(), session))
        .collect::<HashMap<_, _>>();
    for node in tree.nodes() {
        if !node.is_internal_rule_record() && !node.is_retry_history {
            children
                .entry(node.parent_id.as_deref())
                .or_default()
                .push(node);
        }
    }
    for siblings in children.values_mut() {
        siblings.sort_by_key(|node| (node.sibling_order, node.id.as_str()));
    }
    fn node_dto(
        node: &WorkspaceTreeNode,
        public_id: String,
        workflow_capabilities: Option<WorkspaceWorkflowCapabilitiesDto>,
        session_capabilities: Option<WorkspaceSessionCapabilitiesDto>,
        by_id: &HashMap<&str, &WorkspaceTreeNode>,
    ) -> WorkspaceNodeDto {
        let past_attempts = node
            .past_attempt_ids
            .iter()
            .filter_map(|id| by_id.get(id.as_str()).copied())
            .map(|past| node_dto(past, past.id.clone(), None, None, by_id))
            .collect::<Vec<_>>();
        WorkspaceNodeDto {
            id: public_id,
            title: node.title.clone(),
            status: node.status_classification.as_public_str().to_string(),
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
            workflow_capabilities,
            session_capabilities,
            past_attempts_collapsed: !past_attempts.is_empty(),
            past_attempts,
            updated_at: node.updated_at(),
        }
    }

    #[derive(Clone)]
    struct RootProjection {
        public_id: String,
        workflow_capabilities: Option<WorkspaceWorkflowCapabilitiesDto>,
        session_capabilities: Option<WorkspaceSessionCapabilitiesDto>,
    }

    let root_projections = WorkspacePublicRoot::all(tree.nodes())
        .into_iter()
        .map(|root| {
            let is_workflow = workflow_execution_ids.contains(root.public_id());
            let root_session = (!is_workflow)
                .then(|| root.node().session_id.as_deref())
                .flatten()
                .and_then(|session_id| sessions.get(session_id).copied());
            (
                root.node().id.as_str(),
                RootProjection {
                    public_id: root.public_id().to_string(),
                    workflow_capabilities: is_workflow.then(|| workflow_capabilities(root.owner())),
                    session_capabilities: root_session.map(|session| {
                        WorkspaceSessionCapabilitiesDto {
                            session_ref: session.id.clone(),
                            can_archive: session.operations.can_archive,
                            can_delete: session.operations.can_delete,
                        }
                    }),
                },
            )
        })
        .collect::<HashMap<_, _>>();

    fn branch(
        parent: Option<&str>,
        children: &HashMap<Option<&str>, Vec<&WorkspaceTreeNode>>,
        hidden: &HashSet<String>,
        by_id: &HashMap<&str, &WorkspaceTreeNode>,
        root_projections: &HashMap<&str, RootProjection>,
    ) -> Vec<WorkspaceTreeItemDto> {
        children
            .get(&parent)
            .into_iter()
            .flatten()
            .filter(|node| !hidden.contains(&node.id))
            .flat_map(|node| match node.kind {
                WorkspaceNodeKind::Workflow => {
                    branch(Some(&node.id), children, hidden, by_id, root_projections)
                }
                WorkspaceNodeKind::Fanout => {
                    let root = root_projections.get(node.id.as_str());
                    vec![WorkspaceTreeItemDto::Fanout(WorkspaceFanoutDto {
                        id: root.map_or_else(|| node.id.clone(), |root| root.public_id.clone()),
                        title: node.title.clone(),
                        status: node.status_classification.as_public_str().to_string(),
                        workflow_capabilities: root
                            .and_then(|root| root.workflow_capabilities.clone()),
                        children: branch(Some(&node.id), children, hidden, by_id, root_projections),
                        updated_at: node.updated_at(),
                    })]
                }
                WorkspaceNodeKind::Sequence => {
                    let root = root_projections.get(node.id.as_str());
                    vec![WorkspaceTreeItemDto::Sequence(WorkspaceSequenceDto {
                        id: root.map_or_else(|| node.id.clone(), |root| root.public_id.clone()),
                        title: node.title.clone(),
                        status: node.status_classification.as_public_str().to_string(),
                        workflow_capabilities: root
                            .and_then(|root| root.workflow_capabilities.clone()),
                        children: branch(Some(&node.id), children, hidden, by_id, root_projections),
                        updated_at: node.updated_at(),
                    })]
                }
                _ => {
                    let root = root_projections.get(node.id.as_str());
                    vec![WorkspaceTreeItemDto::Node(node_dto(
                        node,
                        root.map_or_else(|| node.id.clone(), |root| root.public_id.clone()),
                        root.and_then(|root| root.workflow_capabilities.clone()),
                        root.and_then(|root| root.session_capabilities.clone()),
                        by_id,
                    ))]
                }
            })
            .collect()
    }
    branch(None, &children, hidden, &by_id, &root_projections)
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
        _ => WorkspaceNodeContentDto::Session(WorkspaceSessionNodeContentDto {
            session_id: node.session_id,
        }),
    };
    WorkspaceNodeDetailDto {
        id: node.id,
        title: node.title,
        status: node.status.as_public_str().to_string(),
        status_classification: node.status_classification.as_public_str().to_string(),
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
#[path = "query_service_test.rs"]
mod query_service_tests;
