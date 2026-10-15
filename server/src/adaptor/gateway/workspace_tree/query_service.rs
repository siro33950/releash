use std::sync::Arc;

use super::SqliteWorkspaceTreeRepository;
use crate::domain::workflow::{
    ExecutionStatusFilter, ExecutionTreeLaunch, WorkflowError, WorkflowExecutionSummary,
};
use crate::domain::workspace_tree::{
    WorkspaceIdentity, WorkspaceNodeKind, WorkspacePublicRoot, WorkspaceTreeNode,
};
use crate::usecase::workflow::{
    WorkspaceCommandNodeContentDto, WorkspaceCommandResultDto, WorkspaceNodeCapabilitiesDto,
    WorkspaceNodeContentDto, WorkspaceNodeDetailDto, WorkspaceSessionNodeContentDto,
};
use crate::usecase::workspace_tree::WorkspaceQueryService;

pub struct SqliteWorkspaceQueryService {
    repository: Arc<SqliteWorkspaceTreeRepository>,
}

impl SqliteWorkspaceQueryService {
    pub fn with_repository(repository: Arc<SqliteWorkspaceTreeRepository>) -> Arc<Self> {
        Arc::new(Self { repository })
    }

    pub async fn execution_records(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
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
        Ok(records)
    }
}

#[async_trait::async_trait]
impl WorkspaceQueryService for SqliteWorkspaceQueryService {
    async fn node_detail(
        &self,
        workspace_identity: &WorkspaceIdentity,
        node_id: &str,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError> {
        let workspace = workspace_identity.as_str();
        let trees = self
            .repository
            .folded_workspace_trees(workspace)
            .await
            .map_err(query_error)?;
        for execution in trees {
            let (folded, record) = &*execution;
            let nodes = self
                .repository
                .tree_nodes(workspace, folded, record)
                .map_err(query_error)?;
            let node = WorkspacePublicRoot::resolve_node(&nodes, node_id);
            let Some(node) = node else { continue };
            let attempts = node
                .past_attempt_ids
                .iter()
                .filter_map(|id| nodes.iter().find(|attempt| &attempt.id == id))
                .map(
                    |attempt| crate::usecase::workflow::WorkspaceNodeAttemptDto {
                        id: attempt.id.clone(),
                        title: attempt.title.clone(),
                        updated_at: attempt.updated_at(),
                        error_reason: attempt.failure_reason(),
                    },
                )
                .collect();
            let workflow = nodes.iter().find(|node| {
                node.kind == WorkspaceNodeKind::Workflow && node.id == folded.aggregate.id
            });
            let mut detail = node_detail_with_workflow(node, workflow);
            detail.past_attempts = attempts;
            return Ok(Some(detail));
        }
        Ok(None)
    }

    async fn node_artifact(
        &self,
        node_execution_id: &str,
    ) -> Result<crate::usecase::workflow::WorkflowGetOutputResult, WorkflowError> {
        use crate::domain::workflow::NodeFact;
        use crate::usecase::workflow::WorkflowGetOutputResult;
        let record = crate::adaptor::gateway::workflow::fact_log::read_latest_record_for_node_with_event_types(
            &self.repository.fact_backend(), node_execution_id, &["artifact_produced"],
        ).await.map_err(WorkflowError::from)?;
        Ok(match record {
            Some(record) => match record.fact {
                NodeFact::ArtifactProduced(artifact) => WorkflowGetOutputResult::Submitted {
                    contract: artifact.contract,
                    structured_output: artifact.value,
                    submitted_at: Some(record.timestamp_ms as f64 / 1000.0),
                    request_id: artifact.request_id,
                    timestamp: record.timestamp_ms as f64 / 1000.0,
                },
                _ => {
                    return Err(WorkflowError::CorruptStoredState(
                        "Expected ArtifactProduced fact".into(),
                    ))
                }
            },
            None => WorkflowGetOutputResult::NotSubmitted,
        })
    }

    async fn execution_summaries(
        &self,
        workspace_identity: Option<&WorkspaceIdentity>,
        status: Option<ExecutionStatusFilter>,
    ) -> Result<Vec<WorkflowExecutionSummary>, WorkflowError> {
        self.execution_records(workspace_identity, status)
            .await?
            .into_iter()
            .map(execution_summary)
            .collect()
    }
}

fn node_detail_with_workflow(
    node: WorkspaceTreeNode,
    workflow: Option<&WorkspaceTreeNode>,
) -> WorkspaceNodeDetailDto {
    node_detail(node, workflow.is_some_and(|workflow| workflow.can_abort))
}

pub fn node_detail(node: WorkspaceTreeNode, can_abort: bool) -> WorkspaceNodeDetailDto {
    let updated_at = node.updated_at();
    let error_reason = node.failure_reason();
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
                shell: node.shell,
                cwd: node.cwd,
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
        classification: node.status_classification.as_public_str().into(),
        node_execution_id: node.node_execution_id,
        node_name: node.node_name,
        model: node.model,
        past_attempts: Vec::new(),
        execution_id: node.execution_id,
        can_abort,
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
        error_reason,
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

pub fn execution_summary(
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
