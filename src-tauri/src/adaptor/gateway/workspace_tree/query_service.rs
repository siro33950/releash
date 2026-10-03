use std::sync::Arc;

use super::SqliteWorkspaceTreeRepository;
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::domain::workflow::{
    ExecutionStatusFilter, ExecutionTreeLaunch, WorkflowError, WorkflowExecutionSummary,
    WorkflowPageRequest,
};
use crate::domain::workspace_tree::{
    WorkspaceIdentity, WorkspaceNodeKind, WorkspaceTreeNode, WorkspaceTreeRepository,
};
use crate::usecase::workflow::{
    WorkspaceCommandNodeContentDto, WorkspaceCommandResultDto, WorkspaceNodeCapabilitiesDto,
    WorkspaceNodeContentDto, WorkspaceNodeDetailDto, WorkspaceSessionNodeContentDto,
};
use crate::usecase::workspace_tree::WorkspaceQueryService;

pub(crate) struct SqliteWorkspaceQueryService {
    repository: Arc<SqliteWorkspaceTreeRepository>,
}

impl SqliteWorkspaceQueryService {
    pub(crate) fn with_repository(repository: Arc<SqliteWorkspaceTreeRepository>) -> Arc<Self> {
        Arc::new(Self { repository })
    }

    pub(crate) fn new_read_only(store: Arc<LocalEventReadStore>) -> Arc<Self> {
        Arc::new(Self {
            repository: SqliteWorkspaceTreeRepository::new_read_only(store),
        })
    }

    async fn execution_records(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
        page: Option<WorkflowPageRequest>,
    ) -> Result<Vec<crate::domain::local_event::WorkflowExecutionMetadataRecord>, WorkflowError>
    {
        let tree_roots = self.repository.tree_roots().await.map_err(|error| {
            WorkflowError::from(
                crate::adaptor::gateway::workflow::fact_log::FactReadError::Query(error),
            )
        })?;
        let mut records = Vec::new();
        for (tree_id, root) in tree_roots {
            if root.launched_as != ExecutionTreeLaunch::Workflow
                || workspace_identity.is_some_and(|wanted| wanted.as_str() != root.worktree_path)
            {
                continue;
            }
            let Some(execution) = self
                .repository
                .folded_tree(&tree_id)
                .await
                .map_err(query_error)?
            else {
                continue;
            };
            let (folded, record) = &*execution;
            debug_assert_eq!(folded.root.launched_as, ExecutionTreeLaunch::Workflow);
            let keep = match status {
                Some(ExecutionStatusFilter::Active) => !record.status.is_finished(),
                Some(ExecutionStatusFilter::Terminal) => record.status.is_finished(),
                None => true,
            };
            if keep {
                records.push(record.clone());
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

#[async_trait::async_trait]
impl WorkspaceQueryService for SqliteWorkspaceQueryService {
    async fn node_detail(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError> {
        self.repository
            .load_node(workspace_identity, node_id)
            .await
            .map(|node| node.map(node_detail))
            .map_err(query_error)
    }

    async fn session_selection(
        &self,
        workspace_identity: &WorkspaceIdentity,
        session_id: &str,
    ) -> Result<Option<crate::usecase::workspace_tree::SessionNodeSelectionDto>, WorkflowError>
    {
        self.repository
            .session_selection(workspace_identity, session_id)
            .await
            .map_err(query_error)
    }

    async fn execution_summaries(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
        page: Option<WorkflowPageRequest>,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError> {
        self.execution_records(workspace_identity, status, page)
            .await?
            .into_iter()
            .map(execution_summary)
            .collect()
    }

    async fn execution_summary(
        &self,
        execution_id: &str,
    ) -> Result<Option<WorkflowExecutionSummary>, WorkflowError> {
        self.repository
            .folded_tree(execution_id)
            .await
            .map_err(query_error)?
            .filter(|execution| execution.0.root.launched_as == ExecutionTreeLaunch::Workflow)
            .map(|execution| execution_summary(execution.1.clone()))
            .transpose()
    }
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
        process_presence: node.process_presence.as_str(),
        worktree: node
            .worktree
            .map(|worktree| crate::usecase::workflow::NodeWorktreeDto {
                branch: worktree.branch,
                path: worktree.path,
            }),
        id: node.id,
        title: node.title,
        status: node.status.as_public_str().to_string(),
        submit_received,
        stop_received,
        waiting_for,
        has_artifact: node.has_artifact,
        error_reason: node.error_reason,
        capabilities: WorkspaceNodeCapabilitiesDto {
            can_rename: node.can_rename,
            can_approve: node.can_approve,
            can_retry: node.can_retry,
            can_resume_session: node.can_resume_session,
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
        total_token_usage: record.total_token_usage,
    })
}

pub(super) fn query_error(
    error: crate::domain::local_event::LocalEventQueryError,
) -> WorkflowError {
    use crate::domain::local_event::LocalEventQueryError;

    match error {
        LocalEventQueryError::Technical(stopped) => WorkflowError::Technical(stopped),
        error @ (LocalEventQueryError::StorageUnavailable { .. }
        | LocalEventQueryError::QueryBusy) => {
            let message = error.to_string();
            WorkflowError::storage(error, message)
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
