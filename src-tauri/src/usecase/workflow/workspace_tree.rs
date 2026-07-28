use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::command::ApprovalCommand;
use super::ports::WorkflowExecutionProjection;
use super::query_service::WorkflowQueryService;
use super::workspace_node_command::{
    CloseWorkspaceNodeError, WorkspaceNodeActionResolver, WorkspaceNodeCloseTarget,
};
use super::WorkflowUsecase;
use crate::domain::path::same_worktree_path;
use crate::domain::workflow::status_aggregation::{
    aggregate_representative_statuses, session_result, NodeProgress, RepresentativeStatus,
    SessionActivity,
};
use crate::domain::workflow::{
    ExecutionListFilter, ExecutionStatus, FanoutSpec, ItemsSource, NodeDefinition, NodeExecution,
    NodeExecutionStatus, NodeKindName, WorkflowDefinition, WorkflowError, WorkflowExecution,
    WorkflowExecutionId, WorkflowExecutionManualArchiveRecord, WorkflowExecutionSummary,
    WORKFLOW_ARCHIVE_REASON_MANUAL,
};

const DEFAULT_SESSION_TITLE: &str = "NewSession";
const DEFAULT_WORKFLOW_TITLE: &str = "Workflow";

fn unix_timestamp_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkspaceSessionState {
    Active,
    Idle,
    Done,
    Error,
    Closed,
    Archived,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WorkspaceSessionInput {
    pub id: String,
    pub worktree_path: String,
    pub state: WorkspaceSessionState,
    pub error_reason: Option<String>,
    pub updated_at: f64,
    pub first_message: String,
    /// Backend-only provenance used to keep closed workflow sessions available.
    /// It is deliberately absent from every public Workspace DTO.
    pub workflow_node_session: bool,
    /// Backend-only routing hint for id-based Workspace queries.
    /// It is never serialized into the Workspace tree or Node detail DTOs.
    pub workflow_execution_id: Option<String>,
    /// Backend-only durable recovery fence used to derive honest workflow
    /// resume capability.
    pub unresolved_recovery_reason: Option<String>,
}

pub(crate) trait WorkspaceSessionGateway: Send + Sync {
    fn list_active_sessions(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError>;

    fn list_closed_sessions(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError>;
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceTreeSnapshotDto {
    pub nodes: Vec<WorkspaceTreeItemDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceTreeSelectionSnapshotDto {
    pub snapshot: WorkspaceTreeSnapshotDto,
    pub reconciliation: WorkspaceSelectionReconciliationDto,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceSelectionReconciliationDto {
    pub selection_in_snapshot: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum WorkspaceTreeItemDto {
    Node(WorkspaceNodeDto),
    Workflow(WorkspaceWorkflowDto),
    Fanout(WorkspaceFanoutDto),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceNodeDto {
    pub id: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    pub content_kind: &'static str,
    pub capabilities: WorkspaceNodeCapabilitiesDto,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceNodeCapabilitiesDto {
    pub can_approve: bool,
    pub can_close: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowDto {
    pub id: String,
    pub title: String,
    pub status: String,
    pub capabilities: WorkspaceWorkflowCapabilitiesDto,
    pub children: Vec<WorkspaceTreeItemDto>,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowCapabilitiesDto {
    pub can_stop: bool,
    pub can_resume: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_unavailable_reason: Option<String>,
    pub can_abort: bool,
    pub can_archive: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceFanoutDto {
    pub id: String,
    pub title: String,
    pub status: String,
    pub children: Vec<WorkspaceTreeItemDto>,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceNodeDetailDto {
    pub id: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_reason: Option<String>,
    pub capabilities: WorkspaceNodeCapabilitiesDto,
    pub updated_at: f64,
    pub content: WorkspaceNodeContentDto,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum WorkspaceNodeContentDto {
    Session(WorkspaceSessionNodeContentDto),
    Command(WorkspaceCommandNodeContentDto),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceSessionNodeContentDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceCommandNodeContentDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<WorkspaceCommandResultDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceCommandResultDto {
    pub exit_code: i64,
    /// Milliseconds, matching the command Artifact reserved field.
    pub duration: u64,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowHistoryItemDto {
    pub execution_id: String,
    pub worktree_path: String,
    pub title: String,
    pub status: String,
    pub updated_at: f64,
    pub archived_at: f64,
    pub archive_reason: String,
}

#[derive(Debug, Clone)]
struct WorkspaceNodeApprovalTarget {
    execution_id: String,
    node_name: String,
    node_execution_id: String,
}

#[derive(Debug, Clone)]
struct WorkspaceNodeRecord {
    detail: WorkspaceNodeDetailDto,
    approval: Option<WorkspaceNodeApprovalTarget>,
    close: Option<WorkspaceNodeCloseTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkspaceProjectionTarget {
    /// Tree summary only. No selected-node content or session lookup index.
    Snapshot,
    /// Materialize detail and action target for this one opaque leaf ID.
    Node(String),
    /// Resolve this persisted Session ID to its opaque Workspace leaf ID.
    Session(String),
}

impl WorkspaceProjectionTarget {
    fn matches_node(&self, node_id: &str) -> bool {
        matches!(self, Self::Node(target) if target == node_id)
    }

    fn matches_session(&self, session_id: &str) -> bool {
        matches!(self, Self::Session(target) if target == session_id)
    }
}

#[derive(Default)]
struct WorkspaceProjectionIndex {
    records: HashMap<String, WorkspaceNodeRecord>,
    session_node_ids: HashMap<String, String>,
}

struct WorkspaceProjection {
    snapshot: WorkspaceTreeSnapshotDto,
    index: WorkspaceProjectionIndex,
}

#[derive(Clone)]
pub(crate) struct WorkspaceTreeQueryService {
    query: WorkflowQueryService,
    worktrees: Arc<dyn crate::domain::workflow::ManagedWorktreeGateway>,
    sessions: Arc<dyn WorkspaceSessionGateway>,
    execution_archives: Arc<dyn crate::domain::workflow::WorkflowExecutionArchiveRepository>,
}

impl WorkspaceTreeQueryService {
    pub(crate) fn new(
        query: WorkflowQueryService,
        worktrees: Arc<dyn crate::domain::workflow::ManagedWorktreeGateway>,
        sessions: Arc<dyn WorkspaceSessionGateway>,
        execution_archives: Arc<dyn crate::domain::workflow::WorkflowExecutionArchiveRepository>,
    ) -> Self {
        Self {
            query,
            worktrees,
            sessions,
            execution_archives,
        }
    }

    pub(crate) fn get_workspace_tree_selection_reconciliation(
        &self,
        worktree_path: &str,
        selected_node_id: &str,
    ) -> Result<WorkspaceTreeSelectionSnapshotDto, WorkflowError> {
        let worktree_path = self.worktrees.resolve(worktree_path)?;
        let mut sessions = self.sessions.list_active_sessions(&worktree_path)?;
        sessions.extend(
            self.sessions
                .list_closed_sessions(&worktree_path)?
                .into_iter()
                .filter(|session| session.workflow_node_session),
        );
        let archives = self.execution_archives.manual_archive_records()?;
        self.query
            .project_workspace(
                &worktree_path,
                sessions,
                &archives,
                WorkspaceProjectionTarget::Snapshot,
            )
            .map(|projection| {
                reconcile_workspace_tree_selection(projection.snapshot, selected_node_id)
            })
    }
}

impl WorkflowUsecase {
    pub(crate) fn collect_workspace_session_inputs(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
        // SessionStore and Workflow projections must receive the same managed-worktree
        // identity. In particular, libgit2 reports the main worktree with a trailing `/`,
        // while Workflow executions persist the canonical path without it.
        let worktree_path = self.resolve_worktree_path(worktree_path)?;
        let mut sessions = self.sessions.list_active_sessions(&worktree_path)?;
        sessions.extend(
            self.sessions
                .list_closed_sessions(&worktree_path)?
                .into_iter()
                .filter(|session| session.workflow_node_session),
        );
        Ok(sessions)
    }

    pub(crate) fn list_workspace_tree_nodes(
        &self,
        worktree_path: &str,
        sessions: Vec<WorkspaceSessionInput>,
    ) -> Result<WorkspaceTreeSnapshotDto, WorkflowError> {
        let worktree_path = self.resolve_worktree_path(worktree_path)?;
        let archives = self.execution_archives.manual_archive_records()?;
        self.query
            .project_workspace(
                &worktree_path,
                sessions,
                &archives,
                WorkspaceProjectionTarget::Snapshot,
            )
            .map(|projection| projection.snapshot)
    }

    pub(crate) fn list_workspace_workflow_history(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkflowError> {
        let worktree_path = self.resolve_worktree_path(worktree_path)?;
        let archives = self.execution_archives.manual_archive_records()?;
        self.query
            .list_workspace_workflow_history(&worktree_path, &archives)
    }

    pub(crate) fn get_workspace_node_detail(
        &self,
        worktree_path: &str,
        node_id: &str,
        sessions: Vec<WorkspaceSessionInput>,
    ) -> Result<Option<WorkspaceNodeDetailDto>, WorkflowError> {
        let worktree_path = self.resolve_worktree_path(worktree_path)?;
        let archives = self.execution_archives.manual_archive_records()?;
        self.query
            .project_workspace(
                &worktree_path,
                sessions,
                &archives,
                WorkspaceProjectionTarget::Node(node_id.to_string()),
            )
            .map(|mut projection| {
                projection
                    .index
                    .records
                    .remove(node_id)
                    .map(|record| record.detail)
            })
    }

    pub(crate) fn get_workspace_session_node_id(
        &self,
        worktree_path: &str,
        session_id: &str,
        sessions: Vec<WorkspaceSessionInput>,
    ) -> Result<Option<String>, WorkflowError> {
        let worktree_path = self.resolve_worktree_path(worktree_path)?;
        let archives = self.execution_archives.manual_archive_records()?;
        self.query
            .project_workspace(
                &worktree_path,
                sessions,
                &archives,
                WorkspaceProjectionTarget::Session(session_id.to_string()),
            )
            .map(|mut projection| projection.index.session_node_ids.remove(session_id))
    }

    pub(crate) fn resolve_workspace_node_approval(
        &self,
        worktree_path: &str,
        node_id: &str,
        sessions: Vec<WorkspaceSessionInput>,
    ) -> Result<ApprovalCommand, WorkflowError> {
        let worktree_path = self.resolve_worktree_path(worktree_path)?;
        let archives = self.execution_archives.manual_archive_records()?;
        let mut projection = self.query.project_workspace(
            &worktree_path,
            sessions,
            &archives,
            WorkspaceProjectionTarget::Node(node_id.to_string()),
        )?;
        let record = projection.index.records.remove(node_id).ok_or_else(|| {
            WorkflowError::external(format!("Workspace node not found: {node_id}"))
        })?;
        let target = record
            .approval
            .ok_or_else(|| WorkflowError::external("Workspace node is not waiting for approval"))?;
        Ok(ApprovalCommand {
            execution_id: target.execution_id,
            node_name: target.node_name,
            node_execution_id: Some(target.node_execution_id),
            comment: None,
        })
    }

    pub(crate) fn archive_workspace_workflow_execution(
        &self,
        worktree_path: &str,
        execution_id: &str,
    ) -> Result<(), WorkflowError> {
        let execution_id = WorkflowExecutionId::new(execution_id.to_string())?;
        if self
            .authorize_execution_summary_for_worktree(execution_id.as_str(), worktree_path)?
            .is_none()
        {
            return Err(WorkflowError::external(format!(
                "Workflow execution not found: {execution_id}"
            )));
        }
        self.execution_archives
            .archive_manual(&execution_id, unix_timestamp_seconds())
    }

    pub(crate) fn restore_workspace_workflow_execution(
        &self,
        worktree_path: &str,
        execution_id: &str,
    ) -> Result<(), WorkflowError> {
        let execution_id = WorkflowExecutionId::new(execution_id.to_string())?;
        if self
            .authorize_execution_summary_for_worktree(execution_id.as_str(), worktree_path)?
            .is_none()
        {
            return Err(WorkflowError::external(format!(
                "Workflow execution not found: {execution_id}"
            )));
        }
        self.execution_archives
            .restore_manual(&execution_id, unix_timestamp_seconds())
    }
}

impl WorkspaceNodeActionResolver for WorkflowUsecase {
    fn resolve_close_target(
        &self,
        worktree_path: &str,
        node_id: &str,
    ) -> Result<WorkspaceNodeCloseTarget, CloseWorkspaceNodeError> {
        let sessions = self
            .collect_workspace_session_inputs(worktree_path)
            .map_err(CloseWorkspaceNodeError::Resolution)?;
        let worktree_path = self
            .resolve_worktree_path(worktree_path)
            .map_err(CloseWorkspaceNodeError::Resolution)?;
        let archives = self
            .execution_archives
            .manual_archive_records()
            .map_err(CloseWorkspaceNodeError::Resolution)?;
        let mut projection = self
            .query
            .project_workspace(
                &worktree_path,
                sessions,
                &archives,
                WorkspaceProjectionTarget::Node(node_id.to_string()),
            )
            .map_err(CloseWorkspaceNodeError::Resolution)?;
        let record = projection.index.records.remove(node_id).ok_or_else(|| {
            CloseWorkspaceNodeError::NodeNotFound {
                node_id: node_id.to_string(),
            }
        })?;
        record
            .close
            .ok_or_else(|| CloseWorkspaceNodeError::CloseNotSupported {
                node_id: node_id.to_string(),
            })
    }
}

impl WorkflowQueryService {
    fn project_workspace(
        &self,
        worktree_path: &str,
        sessions: Vec<WorkspaceSessionInput>,
        archives: &[WorkflowExecutionManualArchiveRecord],
        target: WorkspaceProjectionTarget,
    ) -> Result<WorkspaceProjection, WorkflowError> {
        let mut summaries = self.list_executions(ExecutionListFilter {
            status: None,
            worktree_path: Some(worktree_path.to_string()),
        })?;
        restrict_summaries_to_projection_target(&mut summaries, &sessions, &target);
        let projections = self.execution_projections_for_summaries(&summaries, &target)?;
        Ok(project_workspace_tree(
            worktree_path,
            sessions,
            summaries,
            projections,
            archives,
            &target,
        ))
    }

    pub(in crate::usecase::workflow) fn list_workspace_workflow_history(
        &self,
        worktree_path: &str,
        archives: &[WorkflowExecutionManualArchiveRecord],
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkflowError> {
        let summaries = self.list_executions(ExecutionListFilter {
            status: None,
            worktree_path: Some(worktree_path.to_string()),
        })?;
        Ok(project_workspace_workflow_history(summaries, archives))
    }

    fn execution_projections_for_summaries(
        &self,
        summaries: &[WorkflowExecutionSummary],
        target: &WorkspaceProjectionTarget,
    ) -> Result<HashMap<String, WorkflowExecutionProjection>, WorkflowError> {
        let mut projections = HashMap::new();
        for summary in summaries {
            let projection = match target {
                // Selected Node detail may include its masked Command snapshot/result.
                // Backend-routable IDs restrict this full replay to one execution.
                WorkspaceProjectionTarget::Node(_) => {
                    self.get_execution_with_definition(&summary.execution_id)?
                }
                // Tree summaries and Session lookup never need request, Command,
                // or Artifact bodies and use the payload-stripped production path.
                WorkspaceProjectionTarget::Snapshot | WorkspaceProjectionTarget::Session(_) => {
                    self.get_workspace_execution_with_definition(&summary.execution_id)?
                }
            };
            if let Some(projection) = projection {
                projections.insert(summary.execution_id.clone(), projection);
            }
        }
        Ok(projections)
    }
}

fn project_workspace_tree(
    worktree_path: &str,
    sessions: Vec<WorkspaceSessionInput>,
    summaries: Vec<WorkflowExecutionSummary>,
    projections: HashMap<String, WorkflowExecutionProjection>,
    archives: &[WorkflowExecutionManualArchiveRecord],
    target: &WorkspaceProjectionTarget,
) -> WorkspaceProjection {
    let session_index = session_index(&sessions, worktree_path);
    let nested_session_ids = projections
        .values()
        .flat_map(|projection| projection.execution.node_executions.iter())
        .filter_map(|node_execution| node_execution.session_id.clone())
        .collect::<HashSet<_>>();
    let mut index = WorkspaceProjectionIndex::default();

    let mut direct_sessions = sessions
        .into_iter()
        .filter(|session| {
            same_worktree_path(&session.worktree_path, worktree_path)
                && !session.workflow_node_session
                && !nested_session_ids.contains(&session.id)
        })
        .collect::<Vec<_>>();
    direct_sessions.sort_by(|left, right| {
        compare_titles(&session_title(left, None), &session_title(right, None))
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut nodes = direct_sessions
        .into_iter()
        .map(|session| project_direct_session(session, target, &mut index))
        .collect::<Vec<_>>();

    let mut workflows = summaries
        .into_iter()
        .filter(|summary| {
            // Archiving removes a Workflow branch from the tree, but it must not
            // invalidate the Node that is already selected in the center view.
            // Targeted detail/session lookups are worktree-authorized above and
            // remain routable so the bound Session transcript and input survive
            // a concurrent archive refresh.
            !matches!(target, WorkspaceProjectionTarget::Snapshot)
                || !is_workflow_archived(&summary.execution_id, archives)
        })
        .collect::<Vec<_>>();
    workflows.sort_by(|left, right| {
        compare_titles(&workflow_title(left), &workflow_title(right))
            .then_with(|| left.execution_id.cmp(&right.execution_id))
    });
    nodes.extend(workflows.into_iter().map(|summary| {
        let projection = projections.get(&summary.execution_id);
        project_workflow(summary, projection, &session_index, target, &mut index)
    }));

    let preferred_node_id = preferred_node_id(&nodes);
    WorkspaceProjection {
        snapshot: WorkspaceTreeSnapshotDto {
            nodes,
            preferred_node_id,
        },
        index,
    }
}

fn reconcile_workspace_tree_selection(
    snapshot: WorkspaceTreeSnapshotDto,
    selected_node_id: &str,
) -> WorkspaceTreeSelectionSnapshotDto {
    let selection_in_snapshot = workspace_tree_contains_node(&snapshot.nodes, selected_node_id);
    WorkspaceTreeSelectionSnapshotDto {
        snapshot,
        reconciliation: WorkspaceSelectionReconciliationDto {
            selection_in_snapshot,
        },
    }
}

fn workspace_tree_contains_node(nodes: &[WorkspaceTreeItemDto], node_id: &str) -> bool {
    nodes.iter().any(|item| match item {
        WorkspaceTreeItemDto::Node(node) => node.id == node_id,
        WorkspaceTreeItemDto::Workflow(workflow) => {
            workspace_tree_contains_node(&workflow.children, node_id)
        }
        WorkspaceTreeItemDto::Fanout(fanout) => {
            workspace_tree_contains_node(&fanout.children, node_id)
        }
    })
}

fn restrict_summaries_to_projection_target(
    summaries: &mut Vec<WorkflowExecutionSummary>,
    sessions: &[WorkspaceSessionInput],
    target: &WorkspaceProjectionTarget,
) {
    let execution_id = match target {
        WorkspaceProjectionTarget::Snapshot => return,
        WorkspaceProjectionTarget::Node(node_id) => workflow_execution_id_from_node_id(node_id),
        WorkspaceProjectionTarget::Session(session_id) => {
            let Some(session) = sessions.iter().find(|session| session.id == *session_id) else {
                summaries.clear();
                return;
            };
            if !session.workflow_node_session {
                summaries.clear();
                return;
            }
            let Some(execution_id) = session.workflow_execution_id.clone() else {
                // Preserve lookup for legacy workflow Sessions whose persisted metadata
                // predates the execution routing hint.
                return;
            };
            Some(execution_id)
        }
    };

    match execution_id {
        Some(execution_id) => {
            summaries.retain(|summary| summary.execution_id == execution_id);
        }
        None => summaries.clear(),
    }
}

fn project_direct_session(
    session: WorkspaceSessionInput,
    target: &WorkspaceProjectionTarget,
    index: &mut WorkspaceProjectionIndex,
) -> WorkspaceTreeItemDto {
    let id = opaque_node_id(&format!("session\0{}", session.id));
    let title = session_title(&session, None);
    let status = standalone_session_status(&session.state)
        .as_str()
        .to_string();
    let capabilities = WorkspaceNodeCapabilitiesDto {
        can_approve: false,
        can_close: !matches!(
            session.state,
            WorkspaceSessionState::Closed | WorkspaceSessionState::Archived
        ),
    };
    let close = capabilities.can_close.then(|| WorkspaceNodeCloseTarget {
        session_id: session.id.clone(),
    });
    let node = WorkspaceNodeDto {
        id: id.clone(),
        title: title.clone(),
        status: status.clone(),
        error_reason: session.error_reason.clone(),
        content_kind: "session",
        capabilities: capabilities.clone(),
        updated_at: session.updated_at,
    };
    if target.matches_session(&session.id) {
        index
            .session_node_ids
            .insert(session.id.clone(), id.clone());
    }
    if target.matches_node(&id) {
        index.records.insert(
            id.clone(),
            WorkspaceNodeRecord {
                detail: WorkspaceNodeDetailDto {
                    id,
                    title,
                    status,
                    error_reason: session.error_reason,
                    capabilities,
                    updated_at: session.updated_at,
                    content: WorkspaceNodeContentDto::Session(WorkspaceSessionNodeContentDto {
                        session_id: Some(session.id),
                    }),
                },
                approval: None,
                close,
            },
        );
    }
    WorkspaceTreeItemDto::Node(node)
}

fn project_workflow(
    summary: WorkflowExecutionSummary,
    projection: Option<&WorkflowExecutionProjection>,
    sessions: &HashMap<String, WorkspaceSessionInput>,
    target: &WorkspaceProjectionTarget,
    index: &mut WorkspaceProjectionIndex,
) -> WorkspaceTreeItemDto {
    let execution = projection.map(|projection| &projection.execution);
    let children = projection
        .map(|projection| {
            project_workflow_children(
                &summary,
                &projection.execution,
                projection.definition.as_ref(),
                sessions,
                target,
                index,
            )
        })
        .unwrap_or_default();
    let status = execution
        .map(|execution| execution.status)
        .unwrap_or(summary.status);
    let updated_at = execution
        .map(|execution| execution.updated_at)
        .unwrap_or(summary.updated_at);
    let resume_unavailable_reason = status
        .can_resume()
        .then(|| {
            // HashMap iteration order is unstable; pick the lowest session id so
            // repeated projections surface the same reason deterministically.
            sessions
                .iter()
                .filter(|(_, session)| {
                    session.workflow_execution_id.as_deref() == Some(summary.execution_id.as_str())
                        && session.unresolved_recovery_reason.is_some()
                })
                .min_by_key(|(session_id, _)| session_id.as_str())
                .and_then(|(_, session)| session.unresolved_recovery_reason.clone())
        })
        .flatten();

    WorkspaceTreeItemDto::Workflow(WorkspaceWorkflowDto {
        // Execution IDs remain opaque transport identifiers. The Workspace UI
        // never parses or renders them, and existing workflow actions can reuse it.
        id: summary.execution_id.clone(),
        title: workflow_title(&summary),
        status: representative_status(status.as_str()).as_str().to_string(),
        capabilities: WorkspaceWorkflowCapabilitiesDto {
            can_stop: status.can_stop(),
            can_resume: status.can_resume() && resume_unavailable_reason.is_none(),
            resume_unavailable_reason,
            can_abort: status.can_abort(),
            can_archive: matches!(
                status,
                ExecutionStatus::Completed | ExecutionStatus::Failed | ExecutionStatus::Aborted
            ),
        },
        children,
        updated_at,
    })
}

fn project_workflow_children(
    summary: &WorkflowExecutionSummary,
    execution: &WorkflowExecution,
    definition: Option<&WorkflowDefinition>,
    sessions: &HashMap<String, WorkspaceSessionInput>,
    target: &WorkspaceProjectionTarget,
    index: &mut WorkspaceProjectionIndex,
) -> Vec<WorkspaceTreeItemDto> {
    let node_executions = execution.node_executions.as_slice();
    let mut fanout_children = HashMap::<(String, u32), Vec<&NodeExecution>>::new();
    for node_execution in node_executions {
        let Some(parent) = node_execution.fanout_parent.as_ref() else {
            continue;
        };
        fanout_children
            .entry((parent.parent_node.clone(), parent.parent_attempt))
            .or_default()
            .push(node_execution);
    }

    // NodeStarted append order is the canonical execution order. Do not sort by
    // timestamps: a fanout batch can legitimately give several occurrences the
    // same timestamp. Repeated definitions therefore remain repeated UI rows.
    let mut occurrence_counts = HashMap::<String, usize>::new();
    let mut children = Vec::new();
    for node_execution in node_executions
        .iter()
        .filter(|node_execution| node_execution.fanout_parent.is_none())
    {
        let occurrence_index = occurrence_counts
            .entry(node_execution.node_name.clone())
            .or_default();
        let current_occurrence = *occurrence_index;
        *occurrence_index += 1;

        let item = match node_execution.kind {
            NodeKindName::Fanout => {
                let fanout_spec = definition
                    .and_then(|definition| {
                        definition
                            .nodes
                            .iter()
                            .find(|node| node.name == node_execution.node_name)
                    })
                    .and_then(NodeDefinition::fanout);
                let actual_children = fanout_children
                    .get(&(node_execution.node_name.clone(), node_execution.attempt))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                project_fanout(
                    summary,
                    &node_execution.node_name,
                    fanout_spec,
                    node_execution,
                    current_occurrence,
                    actual_children,
                    sessions,
                    target,
                    index,
                )
            }
            NodeKindName::Session | NodeKindName::Command => project_workflow_node(
                summary,
                &workflow_node_occurrence_key(&node_execution.node_name, current_occurrence),
                node_execution,
                sessions,
                target,
                index,
            ),
        };
        children.push(item);
    }

    children
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct FanoutSlot {
    item_index: Option<usize>,
    child_index: usize,
    child_name: String,
}

#[allow(clippy::too_many_arguments)]
fn project_fanout(
    summary: &WorkflowExecutionSummary,
    fanout_name: &str,
    spec: Option<&FanoutSpec>,
    parent: &NodeExecution,
    parent_occurrence: usize,
    actual_children: &[&NodeExecution],
    sessions: &HashMap<String, WorkspaceSessionInput>,
    target: &WorkspaceProjectionTarget,
    index: &mut WorkspaceProjectionIndex,
) -> WorkspaceTreeItemDto {
    let mut slot_occurrences = BTreeMap::<FanoutSlot, usize>::new();
    let mut dynamic_child_occurrences = BTreeMap::<(usize, String), usize>::new();
    let mut children = Vec::new();
    let dynamic_items =
        spec.is_some_and(|spec| matches!(spec.items, Some(ItemsSource::ArtifactField { .. })));

    for &node_execution in actual_children {
        let reference = node_execution
            .fanout_parent
            .as_ref()
            .expect("filtered fanout child must have a parent");
        let slot = FanoutSlot {
            item_index: reference.item_index,
            child_index: reference.child_index,
            child_name: node_execution.node_name.clone(),
        };
        let semantic_key = if dynamic_items {
            let dynamic_key = (reference.child_index, node_execution.node_name.clone());
            let occurrence = dynamic_child_occurrences.entry(dynamic_key).or_default();
            let semantic_key = fanout_dynamic_child_occurrence_key(
                fanout_name,
                parent_occurrence,
                reference.child_index,
                &node_execution.node_name,
                *occurrence,
            );
            *occurrence += 1;
            semantic_key
        } else {
            let slot_occurrence = slot_occurrences.entry(slot.clone()).or_default();
            let semantic_key = fanout_child_occurrence_key(
                fanout_name,
                parent_occurrence,
                &slot,
                *slot_occurrence,
            );
            *slot_occurrence += 1;
            semantic_key
        };
        children.push(project_workflow_node(
            summary,
            &semantic_key,
            node_execution,
            sessions,
            target,
            index,
        ));
    }

    let child_statuses = children.iter().filter_map(tree_item_status);
    let parent_status = representative_status(parent.status.as_str());
    let status =
        aggregate_representative_statuses(child_statuses.chain(std::iter::once(parent_status)))
            .expect("fanout status aggregation always includes its execution");
    let child_updated = children.iter().map(tree_item_updated_at);
    let parent_updated = parent.completed_at.unwrap_or(parent.started_at);
    let updated_at = child_updated
        .chain(std::iter::once(parent_updated))
        .max_by(f64::total_cmp)
        .expect("fanout updated time aggregation always includes its execution");

    WorkspaceTreeItemDto::Fanout(WorkspaceFanoutDto {
        id: opaque_branch_id(&fanout_branch_occurrence_key(
            &summary.execution_id,
            fanout_name,
            parent_occurrence,
        )),
        title: fanout_name.to_string(),
        status: status.as_str().to_string(),
        children,
        updated_at,
    })
}

fn project_workflow_node(
    summary: &WorkflowExecutionSummary,
    semantic_key: &str,
    node_execution: &NodeExecution,
    sessions: &HashMap<String, WorkspaceSessionInput>,
    target: &WorkspaceProjectionTarget,
    index: &mut WorkspaceProjectionIndex,
) -> WorkspaceTreeItemDto {
    let id = opaque_workflow_node_id(&summary.execution_id, semantic_key);
    let session = node_execution
        .session_id
        .as_ref()
        .and_then(|session_id| sessions.get(session_id));
    let status = workflow_node_status(node_execution, session);
    let execution_updated = node_execution
        .completed_at
        .unwrap_or(node_execution.started_at);
    let updated_at = session
        .map(|session| session.updated_at.max(execution_updated))
        .unwrap_or(execution_updated);
    let capabilities = WorkspaceNodeCapabilitiesDto {
        can_approve: node_execution.status == NodeExecutionStatus::WaitingApproval,
        can_close: false,
    };
    let status_value = status.as_str().to_string();
    let content_kind = match node_execution.kind {
        NodeKindName::Command => "command",
        NodeKindName::Session | NodeKindName::Fanout => "session",
    };
    let node = WorkspaceNodeDto {
        id: id.clone(),
        title: node_execution.node_name.clone(),
        status: status_value.clone(),
        error_reason: session.and_then(|session| session.error_reason.clone()),
        content_kind,
        capabilities: capabilities.clone(),
        updated_at,
    };

    if let WorkspaceProjectionTarget::Session(session_id) = target {
        if node_execution.session_id.as_deref() == Some(session_id.as_str())
            && sessions.contains_key(session_id)
        {
            index
                .session_node_ids
                .insert(session_id.clone(), id.clone());
        }
    }
    if target.matches_node(&id) {
        // Content can retain command output, so it is materialized only for the
        // one explicitly selected leaf. Snapshot/Session projections never clone it.
        let content = match node_execution.kind {
            NodeKindName::Command => {
                WorkspaceNodeContentDto::Command(WorkspaceCommandNodeContentDto {
                    display_command: node_execution.display_command.clone(),
                    result: command_result(node_execution),
                })
            }
            NodeKindName::Session | NodeKindName::Fanout => {
                WorkspaceNodeContentDto::Session(WorkspaceSessionNodeContentDto {
                    // A replayed attachment is not a usable chat until the Session
                    // store can supply it for this worktree.
                    session_id: session.map(|session| session.id.clone()),
                })
            }
        };
        let approval = (node_execution.status == NodeExecutionStatus::WaitingApproval).then(|| {
            WorkspaceNodeApprovalTarget {
                execution_id: node_execution.execution_id.clone(),
                node_name: node_execution.node_name.clone(),
                node_execution_id: node_execution.id.clone(),
            }
        });
        index.records.insert(
            id.clone(),
            WorkspaceNodeRecord {
                detail: WorkspaceNodeDetailDto {
                    id,
                    title: node_execution.node_name.clone(),
                    status: status_value,
                    error_reason: session.and_then(|session| session.error_reason.clone()),
                    capabilities,
                    updated_at,
                    content,
                },
                approval,
                close: None,
            },
        );
    }
    WorkspaceTreeItemDto::Node(node)
}

fn workflow_node_occurrence_key(node_name: &str, occurrence: usize) -> String {
    if occurrence == 0 {
        format!("node\0{node_name}")
    } else {
        format!("node\0{node_name}\0occurrence\0{occurrence}")
    }
}

fn fanout_branch_occurrence_key(
    execution_id: &str,
    fanout_name: &str,
    occurrence: usize,
) -> String {
    if occurrence == 0 {
        format!("workflow\0{execution_id}\0fanout\0{fanout_name}")
    } else {
        format!("workflow\0{execution_id}\0fanout\0{fanout_name}\0occurrence\0{occurrence}")
    }
}

fn fanout_child_occurrence_key(
    fanout_name: &str,
    parent_occurrence: usize,
    slot: &FanoutSlot,
    slot_occurrence: usize,
) -> String {
    if parent_occurrence == 0 && slot_occurrence == 0 {
        format!(
            "fanout-child\0{}\0{:?}\0{}\0{}",
            fanout_name, slot.item_index, slot.child_index, slot.child_name
        )
    } else {
        format!(
            "fanout-child\0{}\0parent-occurrence\0{}\0{:?}\0{}\0{}\0occurrence\0{}",
            fanout_name,
            parent_occurrence,
            slot.item_index,
            slot.child_index,
            slot.child_name,
            slot_occurrence
        )
    }
}

fn fanout_dynamic_child_occurrence_key(
    fanout_name: &str,
    parent_occurrence: usize,
    child_index: usize,
    child_name: &str,
    occurrence: usize,
) -> String {
    // ArtifactField items are discovered only through concrete child executions.
    // Number those children in their NodeStarted order without exposing item coordinates.
    fanout_child_occurrence_key(
        fanout_name,
        parent_occurrence,
        &FanoutSlot {
            item_index: None,
            child_index,
            child_name: child_name.to_string(),
        },
        occurrence,
    )
}

fn command_result(node_execution: &NodeExecution) -> Option<WorkspaceCommandResultDto> {
    let object = node_execution.artifact.as_ref()?.value.as_object()?;
    let exit_code = object.get("exit_code")?.as_i64()?;
    let duration = object.get("duration")?.as_u64()?;
    let stdout = object.get("stdout")?.as_str()?.to_string();
    let stderr = object.get("stderr")?.as_str()?.to_string();
    Some(WorkspaceCommandResultDto {
        exit_code,
        duration,
        stdout,
        stderr,
    })
}

fn workflow_node_status(
    node_execution: &NodeExecution,
    session: Option<&WorkspaceSessionInput>,
) -> RepresentativeStatus {
    let node = NodeProgress::from_status_str(node_execution.status.as_str());
    session
        .map(|session| session_result(node, session_activity(&session.state)))
        .unwrap_or_else(|| representative_status(node_execution.status.as_str()))
}

fn standalone_session_status(state: &WorkspaceSessionState) -> RepresentativeStatus {
    match state {
        WorkspaceSessionState::Active => RepresentativeStatus::Running,
        WorkspaceSessionState::Idle => RepresentativeStatus::Waiting,
        WorkspaceSessionState::Done
        | WorkspaceSessionState::Closed
        | WorkspaceSessionState::Archived => RepresentativeStatus::Completed,
        WorkspaceSessionState::Error => RepresentativeStatus::Error,
    }
}

fn session_activity(state: &WorkspaceSessionState) -> SessionActivity {
    match state {
        WorkspaceSessionState::Active => SessionActivity::Running,
        WorkspaceSessionState::Idle => SessionActivity::Waiting,
        WorkspaceSessionState::Done
        | WorkspaceSessionState::Closed
        | WorkspaceSessionState::Archived => SessionActivity::Done,
        WorkspaceSessionState::Error => SessionActivity::Error,
    }
}

fn project_workspace_workflow_history(
    summaries: Vec<WorkflowExecutionSummary>,
    archives: &[WorkflowExecutionManualArchiveRecord],
) -> Vec<WorkspaceWorkflowHistoryItemDto> {
    let mut history = summaries
        .into_iter()
        .filter_map(|summary| {
            let record = archives
                .iter()
                .find(|record| record.execution_id == summary.execution_id)?;
            Some(WorkspaceWorkflowHistoryItemDto {
                execution_id: summary.execution_id.clone(),
                worktree_path: summary.worktree_path.clone(),
                title: workflow_title(&summary),
                status: representative_status(summary.status.as_str())
                    .as_str()
                    .to_string(),
                updated_at: summary.updated_at,
                archived_at: record.archived_at,
                archive_reason: WORKFLOW_ARCHIVE_REASON_MANUAL.to_string(),
            })
        })
        .collect::<Vec<_>>();
    history.sort_by(|left, right| {
        right
            .archived_at
            .total_cmp(&left.archived_at)
            .then_with(|| compare_titles(&left.title, &right.title))
            .then_with(|| left.execution_id.cmp(&right.execution_id))
    });
    history
}

fn session_index(
    sessions: &[WorkspaceSessionInput],
    worktree_path: &str,
) -> HashMap<String, WorkspaceSessionInput> {
    sessions
        .iter()
        .filter(|session| same_worktree_path(&session.worktree_path, worktree_path))
        .cloned()
        .map(|session| (session.id.clone(), session))
        .collect()
}

fn session_title(session: &WorkspaceSessionInput, node_name: Option<&str>) -> String {
    node_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let first_message = session.first_message.trim();
            (!first_message.is_empty()).then(|| first_message.to_string())
        })
        .unwrap_or_else(|| DEFAULT_SESSION_TITLE.to_string())
}

fn representative_status(status: &str) -> RepresentativeStatus {
    RepresentativeStatus::from_status_str(status)
}

fn workflow_title(summary: &WorkflowExecutionSummary) -> String {
    let workflow_name = summary.workflow_name.trim();
    if workflow_name.is_empty() {
        DEFAULT_WORKFLOW_TITLE.to_string()
    } else {
        workflow_name.to_string()
    }
}

fn is_workflow_archived(
    execution_id: &str,
    archives: &[WorkflowExecutionManualArchiveRecord],
) -> bool {
    archives
        .iter()
        .any(|record| record.execution_id == execution_id)
}

fn compare_titles(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
}

fn tree_item_status(item: &WorkspaceTreeItemDto) -> Option<RepresentativeStatus> {
    Some(representative_status(match item {
        WorkspaceTreeItemDto::Node(node) => &node.status,
        WorkspaceTreeItemDto::Workflow(workflow) => &workflow.status,
        WorkspaceTreeItemDto::Fanout(fanout) => &fanout.status,
    }))
}

fn tree_item_updated_at(item: &WorkspaceTreeItemDto) -> f64 {
    match item {
        WorkspaceTreeItemDto::Node(node) => node.updated_at,
        WorkspaceTreeItemDto::Workflow(workflow) => workflow.updated_at,
        WorkspaceTreeItemDto::Fanout(fanout) => fanout.updated_at,
    }
}

fn preferred_node_id(nodes: &[WorkspaceTreeItemDto]) -> Option<String> {
    let mut ordered = Vec::<&WorkspaceNodeDto>::new();
    collect_leaf_nodes(nodes, &mut ordered);
    ordered
        .iter()
        .find(|node| matches!(node.status.as_str(), "running" | "waiting"))
        .or_else(|| ordered.first())
        .map(|node| node.id.clone())
}

fn collect_leaf_nodes<'a>(
    items: &'a [WorkspaceTreeItemDto],
    nodes: &mut Vec<&'a WorkspaceNodeDto>,
) {
    for item in items {
        match item {
            WorkspaceTreeItemDto::Node(node) => nodes.push(node),
            WorkspaceTreeItemDto::Workflow(workflow) => {
                collect_leaf_nodes(&workflow.children, nodes)
            }
            WorkspaceTreeItemDto::Fanout(fanout) => collect_leaf_nodes(&fanout.children, nodes),
        }
    }
}

fn opaque_node_id(semantic_key: &str) -> String {
    opaque_id("node", semantic_key)
}

fn opaque_workflow_node_id(execution_id: &str, semantic_key: &str) -> String {
    let execution_id = uuid::Uuid::parse_str(execution_id)
        .expect("persisted Workflow execution IDs must be UUIDs");
    let digest = Sha256::digest(semantic_key.as_bytes());
    let mut encoded = format!("node-w-{}-", execution_id.simple());
    for byte in &digest[..16] {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn workflow_execution_id_from_node_id(node_id: &str) -> Option<String> {
    let encoded = node_id.strip_prefix("node-w-")?;
    let (execution_id, digest) = encoded.split_once('-')?;
    if digest.len() != 32 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    uuid::Uuid::parse_str(execution_id)
        .ok()
        .map(|execution_id| execution_id.to_string())
}

fn opaque_branch_id(semantic_key: &str) -> String {
    opaque_id("branch", semantic_key)
}

fn opaque_id(prefix: &str, semantic_key: &str) -> String {
    let digest = Sha256::digest(semantic_key.as_bytes());
    let mut encoded = String::with_capacity(prefix.len() + 1 + 32);
    encoded.push_str(prefix);
    encoded.push('-');
    for byte in &digest[..16] {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        Artifact, CommandSpec, ExecutionOrigin, NodeExecutionFailure, NodeExecutionFailureKind,
        NodeKind, TokenUsage,
    };

    fn summary() -> WorkflowExecutionSummary {
        WorkflowExecutionSummary {
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            workflow_name: "review".to_string(),
            status: ExecutionStatus::Running,
            worktree_path: "/repo".to_string(),
            current_node: Some("plan".to_string()),
            created_from: ExecutionOrigin::DesktopUi,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: TokenUsage::default(),
        }
    }

    fn node(
        id: &str,
        name: &str,
        kind: NodeKindName,
        attempt: u32,
        status: NodeExecutionStatus,
    ) -> NodeExecution {
        NodeExecution {
            id: id.to_string(),
            execution_id: summary().execution_id,
            node_name: name.to_string(),
            kind,
            attempt,
            status,
            session_id: None,
            display_command: None,
            result_summary: None,
            artifact: None,
            token_usage: None,
            failure: None,
            fanout_parent: None,
            started_at: f64::from(attempt),
            completed_at: None,
        }
    }

    fn execution(nodes: Vec<NodeExecution>) -> WorkflowExecution {
        WorkflowExecution {
            id: summary().execution_id,
            workflow_name: "review".to_string(),
            status: ExecutionStatus::Running,
            current_node: Some("plan".to_string()),
            created_from: ExecutionOrigin::DesktopUi,
            worktree_path: "/repo".to_string(),
            started_at: 1.0,
            updated_at: 3.0,
            completed_at: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: TokenUsage::default(),
            node_executions: nodes,
            artifacts: Vec::new(),
            fanouts: Vec::new(),
            approval_target: None,
        }
    }

    fn definition(nodes: Vec<NodeDefinition>) -> WorkflowDefinition {
        WorkflowDefinition {
            name: "review".to_string(),
            nodes,
            ..Default::default()
        }
    }

    fn session_definition(name: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Session(Default::default()),
            ..Default::default()
        }
    }

    fn command_definition(name: &str) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            kind: NodeKind::Command(CommandSpec {
                command: "ignored raw template".to_string(),
            }),
            ..Default::default()
        }
    }

    fn projection(
        definition: WorkflowDefinition,
        execution: WorkflowExecution,
    ) -> HashMap<String, WorkflowExecutionProjection> {
        HashMap::from([(
            summary().execution_id,
            WorkflowExecutionProjection {
                execution,
                definition: Some(definition),
            },
        )])
    }

    fn workflow_node_target(name: &str) -> WorkspaceProjectionTarget {
        workflow_node_target_at(name, 0)
    }

    fn workflow_node_target_at(name: &str, occurrence: usize) -> WorkspaceProjectionTarget {
        WorkspaceProjectionTarget::Node(opaque_workflow_node_id(
            &summary().execution_id,
            &workflow_node_occurrence_key(name, occurrence),
        ))
    }

    #[test]
    fn opaque_workflow_node_id_routes_to_one_execution_without_exposing_attempts() {
        let execution_id = summary().execution_id;
        let first =
            opaque_workflow_node_id(&execution_id, &workflow_node_occurrence_key("plan", 0));
        let retried =
            opaque_workflow_node_id(&execution_id, &workflow_node_occurrence_key("plan", 1));

        assert_ne!(first, retried);
        assert_eq!(
            workflow_execution_id_from_node_id(&first),
            Some(execution_id.clone())
        );
        assert_eq!(
            workflow_execution_id_from_node_id(&retried),
            Some(execution_id)
        );
        assert!(!first.contains("attempt"));
        assert!(!retried.contains("attempt"));
        assert_eq!(
            workflow_execution_id_from_node_id("node-not-routable"),
            None
        );
    }

    #[test]
    fn id_based_projection_targets_restrict_workflow_replay() {
        let mut other = summary();
        other.execution_id = "00000000-0000-4000-8000-000000000002".to_string();
        let mut summaries = vec![summary(), other.clone()];

        restrict_summaries_to_projection_target(&mut summaries, &[], &workflow_node_target("plan"));
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].execution_id, summary().execution_id);

        let mut direct = vec![summary(), other.clone()];
        restrict_summaries_to_projection_target(
            &mut direct,
            &[],
            &WorkspaceProjectionTarget::Node(opaque_node_id("session\0direct")),
        );
        assert!(direct.is_empty());

        let mut session_lookup = vec![summary(), other.clone()];
        restrict_summaries_to_projection_target(
            &mut session_lookup,
            &[WorkspaceSessionInput {
                id: "workflow-session".to_string(),
                worktree_path: "/repo".to_string(),
                state: WorkspaceSessionState::Idle,
                error_reason: None,
                updated_at: 1.0,
                first_message: String::new(),
                workflow_node_session: true,
                workflow_execution_id: Some(other.execution_id.clone()),
                unresolved_recovery_reason: None,
            }],
            &WorkspaceProjectionTarget::Session("workflow-session".to_string()),
        );
        assert_eq!(session_lookup, vec![other]);
    }

    #[test]
    fn direct_session_is_a_leaf_and_summary_is_an_allowlist() {
        let projected = project_workspace_tree(
            "/repo",
            vec![
                WorkspaceSessionInput {
                    id: "session-secret-id".to_string(),
                    worktree_path: "/repo/".to_string(),
                    state: WorkspaceSessionState::Active,
                    error_reason: None,
                    updated_at: 4.0,
                    first_message: "Investigate failure".to_string(),
                    workflow_node_session: false,
                    workflow_execution_id: None,
                    unresolved_recovery_reason: None,
                },
                WorkspaceSessionInput {
                    id: "foreign-session".to_string(),
                    worktree_path: "/repository".to_string(),
                    state: WorkspaceSessionState::Active,
                    error_reason: None,
                    updated_at: 5.0,
                    first_message: "must not appear".to_string(),
                    workflow_node_session: false,
                    workflow_execution_id: None,
                    unresolved_recovery_reason: None,
                },
            ],
            Vec::new(),
            HashMap::new(),
            &[],
            &WorkspaceProjectionTarget::Session("session-secret-id".to_string()),
        );
        let WorkspaceTreeItemDto::Node(node) = &projected.snapshot.nodes[0] else {
            panic!("expected node")
        };
        assert_eq!(node.title, "Investigate failure");
        assert_eq!(node.status, "running");
        assert_eq!(projected.snapshot.preferred_node_id, Some(node.id.clone()));
        assert_eq!(
            projected.index.session_node_ids.get("session-secret-id"),
            Some(&node.id),
            "NewSession lookup must return the opaque Workspace node ID"
        );
        assert!(
            projected.index.records.is_empty(),
            "Session lookup must not materialize node detail"
        );

        let value = serde_json::to_value(&projected.snapshot).unwrap();
        let serialized = value.to_string();
        assert!(!serialized.contains("session-secret-id"));
        assert!(!serialized.contains("firstMessage"));
        assert!(!serialized.contains("workflowNodeSession"));
        assert!(!serialized.contains("executionId"));
        assert!(!serialized.contains("attempt"));
        assert!(!serialized.contains("artifact"));
    }

    #[test]
    fn direct_session_close_target_is_materialized_only_for_its_opaque_node() {
        let node_id = opaque_node_id("session\0session-to-close");
        let projected = project_workspace_tree(
            "/repo",
            vec![WorkspaceSessionInput {
                id: "session-to-close".to_string(),
                worktree_path: "/repo".to_string(),
                state: WorkspaceSessionState::Idle,
                error_reason: None,
                updated_at: 4.0,
                first_message: "Close me".to_string(),
                workflow_node_session: false,
                workflow_execution_id: None,
                unresolved_recovery_reason: None,
            }],
            Vec::new(),
            HashMap::new(),
            &[],
            &WorkspaceProjectionTarget::Node(node_id.clone()),
        );

        let record = &projected.index.records[&node_id];
        assert!(record.detail.capabilities.can_close);
        assert_eq!(
            record
                .close
                .as_ref()
                .map(|target| target.session_id.as_str()),
            Some("session-to-close")
        );
    }

    #[test]
    fn error_reason_is_exposed_on_direct_session_badges_and_detail() {
        let node_id = opaque_node_id("session\0errored-session");
        let projected = project_workspace_tree(
            "/repo",
            vec![WorkspaceSessionInput {
                id: "errored-session".to_string(),
                worktree_path: "/repo".to_string(),
                state: WorkspaceSessionState::Error,
                error_reason: Some("app server stopped".to_string()),
                updated_at: 4.0,
                first_message: "Errored session".to_string(),
                workflow_node_session: false,
                workflow_execution_id: None,
                unresolved_recovery_reason: None,
            }],
            Vec::new(),
            HashMap::new(),
            &[],
            &WorkspaceProjectionTarget::Node(node_id.clone()),
        );

        let WorkspaceTreeItemDto::Node(node) = &projected.snapshot.nodes[0] else {
            panic!("expected session node")
        };
        assert_eq!(node.status, "error");
        assert_eq!(node.error_reason.as_deref(), Some("app server stopped"));
        assert_eq!(
            projected.index.records[&node_id]
                .detail
                .error_reason
                .as_deref(),
            Some("app server stopped")
        );
    }

    #[test]
    fn absent_definition_still_projects_actual_occurrences_without_queued_definitions() {
        let workflow_summary = summary();
        let projected = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![workflow_summary.clone()],
            HashMap::from([(
                workflow_summary.execution_id,
                WorkflowExecutionProjection {
                    execution: execution(vec![node(
                        "history-only",
                        "plan",
                        NodeKindName::Session,
                        1,
                        NodeExecutionStatus::Running,
                    )]),
                    definition: None,
                },
            )]),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        let WorkspaceTreeItemDto::Workflow(workflow) = &projected.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        assert_eq!(workflow.children.len(), 1);
        let WorkspaceTreeItemDto::Node(node) = &workflow.children[0] else {
            panic!("expected actual occurrence")
        };
        assert_eq!(node.title, "plan");
        assert_eq!(node.status, "running");
    }

    #[test]
    fn workflow_without_started_nodes_has_an_empty_branch_and_no_preferred_node() {
        let definition = definition(vec![
            command_definition("build"),
            session_definition("review"),
        ]);
        let projected = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(definition, execution(Vec::new())),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        let WorkspaceTreeItemDto::Workflow(workflow) = &projected.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        assert!(workflow.children.is_empty());
        assert_eq!(projected.snapshot.preferred_node_id, None);
        assert!(projected.index.records.is_empty());
        assert!(projected.index.session_node_ids.is_empty());
    }

    #[test]
    fn execution_occurrences_follow_event_order_without_unstarted_definitions() {
        let workflow_definition = definition(vec![
            session_definition("A"),
            command_definition("B"),
            command_definition("C"),
            session_definition("D"),
        ]);
        let mut occurrences = vec![
            node(
                "a-first-internal",
                "A",
                NodeKindName::Session,
                1,
                NodeExecutionStatus::Succeeded,
            ),
            node(
                "b-first-internal",
                "B",
                NodeKindName::Command,
                1,
                NodeExecutionStatus::Succeeded,
            ),
            node(
                "a-second-internal",
                "A",
                NodeKindName::Session,
                2,
                NodeExecutionStatus::Succeeded,
            ),
            node(
                "c-first-internal",
                "C",
                NodeKindName::Command,
                1,
                NodeExecutionStatus::Running,
            ),
        ];
        for occurrence in &mut occurrences {
            occurrence.started_at = 10.0;
        }
        let projected = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(workflow_definition, execution(occurrences)),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        let WorkspaceTreeItemDto::Workflow(workflow) = &projected.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        let leaves = workflow
            .children
            .iter()
            .map(|item| match item {
                WorkspaceTreeItemDto::Node(node) => node,
                _ => panic!("expected leaf"),
            })
            .collect::<Vec<_>>();

        assert_eq!(
            leaves
                .iter()
                .map(|node| node.title.as_str())
                .collect::<Vec<_>>(),
            vec!["A", "B", "A", "C"]
        );
        assert_ne!(leaves[0].id, leaves[2].id);
        assert_eq!(
            leaves
                .iter()
                .map(|node| &node.id)
                .collect::<HashSet<_>>()
                .len(),
            leaves.len()
        );
        let serialized = serde_json::to_string(&projected.snapshot).unwrap();
        for internal_id in [
            "a-first-internal",
            "b-first-internal",
            "a-second-internal",
            "c-first-internal",
        ] {
            assert!(!serialized.contains(internal_id));
        }
        assert!(!serialized.contains("attempt"));
        assert_eq!(
            projected.snapshot.preferred_node_id,
            Some(leaves[3].id.clone())
        );
    }

    #[test]
    fn terminal_workflows_hide_every_unstarted_leaf_and_branch() {
        let fanout = NodeDefinition {
            name: "unstarted-fanout".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec!["fanout-child".to_string()],
                items: None,
            }),
            ..Default::default()
        };
        let workflow_definition = definition(vec![
            session_definition("started"),
            session_definition("unstarted-leaf"),
            fanout,
            session_definition("fanout-child"),
        ]);
        let started = node(
            "started-execution",
            "started",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Succeeded,
        );

        for status in [
            ExecutionStatus::Completed,
            ExecutionStatus::Failed,
            ExecutionStatus::Aborted,
        ] {
            let mut workflow_summary = summary();
            workflow_summary.status = status;
            let mut runtime = execution(vec![started.clone()]);
            runtime.status = status;
            let projected = project_workspace_tree(
                "/repo",
                Vec::new(),
                vec![workflow_summary],
                projection(workflow_definition.clone(), runtime),
                &[],
                &WorkspaceProjectionTarget::Snapshot,
            );
            let WorkspaceTreeItemDto::Workflow(workflow) = &projected.snapshot.nodes[0] else {
                panic!("expected workflow")
            };
            assert_eq!(workflow.children.len(), 1);
            assert!(matches!(
                &workflow.children[0],
                WorkspaceTreeItemDto::Node(node) if node.title == "started"
            ));
        }
    }

    #[test]
    fn started_nodes_keep_every_execution_status() {
        let statuses = [
            (NodeExecutionStatus::Running, "running"),
            (NodeExecutionStatus::WaitingApproval, "waiting"),
            (NodeExecutionStatus::Succeeded, "completed"),
            (NodeExecutionStatus::Failed, "failed"),
            (NodeExecutionStatus::Aborted, "aborted"),
        ];
        let definitions = statuses
            .iter()
            .enumerate()
            .map(|(index, _)| session_definition(&format!("node-{index}")))
            .collect::<Vec<_>>();
        let executions = statuses
            .iter()
            .enumerate()
            .map(|(index, (status, _))| {
                node(
                    &format!("execution-{index}"),
                    &format!("node-{index}"),
                    NodeKindName::Session,
                    index as u32 + 1,
                    *status,
                )
            })
            .collect::<Vec<_>>();
        let projected = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(definition(definitions), execution(executions)),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        let WorkspaceTreeItemDto::Workflow(workflow) = &projected.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        let projected_statuses = workflow
            .children
            .iter()
            .map(|item| match item {
                WorkspaceTreeItemDto::Node(node) => node.status.as_str(),
                _ => panic!("expected leaf"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            projected_statuses,
            statuses
                .iter()
                .map(|(_, expected)| *expected)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn node_started_is_identical_in_live_and_reloaded_workspace_trees() {
        let workflow_definition = definition(vec![session_definition("plan")]);
        let mut live_execution = execution(Vec::new());
        let before = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(workflow_definition.clone(), live_execution.clone()),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        let WorkspaceTreeItemDto::Workflow(before_workflow) = &before.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        assert!(before_workflow.children.is_empty());

        let mut started = node(
            "plan-execution",
            "plan",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Running,
        );
        started.started_at = 2.0;
        live_execution.node_executions.push(started);
        live_execution.updated_at = 2.0;
        live_execution.current_node = Some("plan".to_string());
        let live = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(workflow_definition.clone(), live_execution),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );

        // Event replay lives outside the usecase layer. Model reload with an
        // independently reconstructed domain read model at this projection boundary.
        let mut reloaded_node = node(
            "plan-execution",
            "plan",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Running,
        );
        reloaded_node.started_at = 2.0;
        let mut reloaded_execution = execution(vec![reloaded_node]);
        reloaded_execution.updated_at = 2.0;
        reloaded_execution.current_node = Some("plan".to_string());
        let reloaded = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(workflow_definition, reloaded_execution),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );

        let WorkspaceTreeItemDto::Workflow(live_workflow) = &live.snapshot.nodes[0] else {
            panic!("expected live workflow")
        };
        assert_eq!(live_workflow.children.len(), 1);
        assert_eq!(live.snapshot, reloaded.snapshot);
    }

    #[test]
    fn definition_only_nodes_never_populate_tree_detail_or_session_indexes() {
        let fanout = NodeDefinition {
            name: "matrix".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec!["review".to_string()],
                items: Some(ItemsSource::Literal(vec![serde_json::json!("item")])),
            }),
            ..Default::default()
        };
        let expanded_definition = definition(vec![
            session_definition("plan"),
            fanout,
            session_definition("review"),
        ]);
        let stored_session = WorkspaceSessionInput {
            id: "stored-session".to_string(),
            worktree_path: "/repo".to_string(),
            state: WorkspaceSessionState::Active,
            error_reason: None,
            updated_at: 4.0,
            first_message: String::new(),
            workflow_node_session: true,
            workflow_execution_id: Some(summary().execution_id),
            unresolved_recovery_reason: None,
        };
        let empty = project_workspace_tree(
            "/repo",
            vec![stored_session.clone()],
            vec![summary()],
            projection(definition(Vec::new()), execution(Vec::new())),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        let definition_only = project_workspace_tree(
            "/repo",
            vec![stored_session.clone()],
            vec![summary()],
            projection(expanded_definition.clone(), execution(Vec::new())),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        assert_eq!(empty.snapshot, definition_only.snapshot);

        let detail_target = workflow_node_target("plan");
        let missing_detail = project_workspace_tree(
            "/repo",
            vec![stored_session.clone()],
            vec![summary()],
            projection(expanded_definition.clone(), execution(Vec::new())),
            &[],
            &detail_target,
        );
        assert!(missing_detail.index.records.is_empty());
        let missing_session = project_workspace_tree(
            "/repo",
            vec![stored_session.clone()],
            vec![summary()],
            projection(expanded_definition.clone(), execution(Vec::new())),
            &[],
            &WorkspaceProjectionTarget::Session("stored-session".to_string()),
        );
        assert!(missing_session.index.session_node_ids.is_empty());

        let mut started = node(
            "plan-execution",
            "plan",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Running,
        );
        started.session_id = Some("stored-session".to_string());
        let runtime = execution(vec![started]);
        let detail = project_workspace_tree(
            "/repo",
            vec![stored_session.clone()],
            vec![summary()],
            projection(expanded_definition.clone(), runtime.clone()),
            &[],
            &detail_target,
        );
        let WorkspaceProjectionTarget::Node(node_id) = detail_target else {
            unreachable!()
        };
        assert!(detail.index.records.contains_key(&node_id));
        let lookup = project_workspace_tree(
            "/repo",
            vec![stored_session],
            vec![summary()],
            projection(expanded_definition, runtime),
            &[],
            &WorkspaceProjectionTarget::Session("stored-session".to_string()),
        );
        assert_eq!(
            lookup.index.session_node_ids.get("stored-session"),
            Some(&node_id)
        );
    }

    #[test]
    fn fanout_occurrences_are_distinct_and_children_stay_nested_in_event_order() {
        let fanout = NodeDefinition {
            name: "reviews".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec!["review".to_string()],
                items: None,
            }),
            ..Default::default()
        };
        let mut first_parent = node(
            "fanout-parent-1",
            "reviews",
            NodeKindName::Fanout,
            1,
            NodeExecutionStatus::Succeeded,
        );
        first_parent.completed_at = Some(4.0);
        let mut first = node(
            "child-attempt-1",
            "review",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Failed,
        );
        first.fanout_parent = Some(crate::domain::workflow::FanoutParentRef {
            parent_node: "reviews".to_string(),
            parent_attempt: 1,
            item_index: None,
            child_index: 0,
        });
        let mut retry = node(
            "child-attempt-2",
            "review",
            NodeKindName::Session,
            2,
            NodeExecutionStatus::Succeeded,
        );
        retry.fanout_parent = Some(crate::domain::workflow::FanoutParentRef {
            parent_node: "reviews".to_string(),
            parent_attempt: 1,
            item_index: None,
            child_index: 0,
        });
        let second_parent = node(
            "fanout-parent-2",
            "reviews",
            NodeKindName::Fanout,
            2,
            NodeExecutionStatus::Running,
        );
        let mut second_child = node(
            "child-attempt-3",
            "review",
            NodeKindName::Session,
            3,
            NodeExecutionStatus::Running,
        );
        second_child.fanout_parent = Some(crate::domain::workflow::FanoutParentRef {
            parent_node: "reviews".to_string(),
            parent_attempt: 2,
            item_index: None,
            child_index: 0,
        });
        let projected = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(
                definition(vec![fanout, session_definition("review")]),
                execution(vec![
                    first_parent,
                    first,
                    retry,
                    second_parent,
                    second_child,
                ]),
            ),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        let WorkspaceTreeItemDto::Workflow(workflow) = &projected.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        assert_eq!(workflow.children.len(), 2);
        let WorkspaceTreeItemDto::Fanout(first_fanout) = &workflow.children[0] else {
            panic!("expected first fanout")
        };
        let WorkspaceTreeItemDto::Fanout(second_fanout) = &workflow.children[1] else {
            panic!("expected second fanout")
        };
        assert_ne!(first_fanout.id, second_fanout.id);
        assert_eq!(first_fanout.children.len(), 2);
        assert_eq!(second_fanout.children.len(), 1);
        let first_child_ids = first_fanout
            .children
            .iter()
            .map(|item| match item {
                WorkspaceTreeItemDto::Node(node) => {
                    assert_eq!(node.title, "review");
                    node.id.as_str()
                }
                _ => panic!("expected nested child"),
            })
            .collect::<Vec<_>>();
        assert_ne!(first_child_ids[0], first_child_ids[1]);
        let WorkspaceTreeItemDto::Node(second_child) = &second_fanout.children[0] else {
            panic!("expected nested second occurrence")
        };
        assert_eq!(second_child.status, "running");
        assert!(!workflow
            .children
            .iter()
            .any(|item| matches!(item, WorkspaceTreeItemDto::Node(_))));
        let serialized = serde_json::to_string(&projected.snapshot).unwrap();
        assert!(!serialized.contains("fanout-parent"));
        assert!(!serialized.contains("child-attempt"));
        assert!(!serialized.contains("parentAttempt"));
        assert!(!serialized.contains("itemIndex"));
        assert!(!serialized.contains("childIndex"));
    }

    #[test]
    fn literal_fanout_projects_only_started_children_in_event_order() {
        let fanout = NodeDefinition {
            name: "matrix".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec!["lint".to_string(), "test".to_string()],
                items: Some(ItemsSource::Literal(vec![
                    serde_json::json!({"package": "core"}),
                    serde_json::json!({"package": "ui"}),
                ])),
            }),
            ..Default::default()
        };
        let parent = node(
            "matrix-parent",
            "matrix",
            NodeKindName::Fanout,
            1,
            NodeExecutionStatus::Running,
        );
        let mut executions = vec![parent];
        for (item_index, child_name) in [(0, "lint"), (1, "test")] {
            let child_index = usize::from(child_name == "test");
            let mut child = node(
                &format!("{child_name}-{item_index}"),
                child_name,
                NodeKindName::Command,
                1,
                NodeExecutionStatus::Succeeded,
            );
            child.started_at += item_index as f64;
            child.fanout_parent = Some(crate::domain::workflow::FanoutParentRef {
                parent_node: "matrix".to_string(),
                parent_attempt: 1,
                item_index: Some(item_index),
                child_index,
            });
            executions.push(child);
        }
        let projected = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(
                definition(vec![
                    fanout,
                    command_definition("lint"),
                    command_definition("test"),
                ]),
                execution(executions),
            ),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        let WorkspaceTreeItemDto::Workflow(workflow) = &projected.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        assert_eq!(workflow.children.len(), 1, "children must not be flat");
        let WorkspaceTreeItemDto::Fanout(fanout) = &workflow.children[0] else {
            panic!("expected fanout")
        };
        let titles = fanout
            .children
            .iter()
            .map(|item| match item {
                WorkspaceTreeItemDto::Node(node) => node.title.as_str(),
                _ => panic!("expected leaf"),
            })
            .collect::<Vec<_>>();
        assert_eq!(titles, vec!["lint", "test"]);
        let ids = fanout
            .children
            .iter()
            .map(|item| match item {
                WorkspaceTreeItemDto::Node(node) => node.id.as_str(),
                _ => panic!("expected leaf"),
            })
            .collect::<HashSet<_>>();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn artifact_item_fanout_without_started_children_has_an_empty_branch() {
        let fanout = NodeDefinition {
            name: "matrix".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec!["review".to_string()],
                items: Some(ItemsSource::ArtifactField {
                    node: "prepare".to_string(),
                    field: "items".to_string(),
                }),
            }),
            ..Default::default()
        };
        let parent = node(
            "matrix-parent",
            "matrix",
            NodeKindName::Fanout,
            1,
            NodeExecutionStatus::Running,
        );
        let workflow_definition = definition(vec![fanout, session_definition("review")]);
        let empty = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(workflow_definition.clone(), execution(vec![parent.clone()])),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        let fanout_branch = |projection: &WorkspaceProjection| {
            let WorkspaceTreeItemDto::Workflow(workflow) = &projection.snapshot.nodes[0] else {
                panic!("expected workflow")
            };
            let WorkspaceTreeItemDto::Fanout(fanout) = &workflow.children[0] else {
                panic!("expected fanout")
            };
            fanout.clone()
        };
        assert!(fanout_branch(&empty).children.is_empty());

        let dynamic_child = |id: &str, item_index: usize| {
            let mut child = node(
                id,
                "review",
                NodeKindName::Session,
                1,
                NodeExecutionStatus::Running,
            );
            child.fanout_parent = Some(crate::domain::workflow::FanoutParentRef {
                parent_node: "matrix".to_string(),
                parent_attempt: 1,
                item_index: Some(item_index),
                child_index: 0,
            });
            child
        };
        let expanded = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(
                workflow_definition,
                execution(vec![
                    parent,
                    dynamic_child("review-item-1", 0),
                    dynamic_child("review-item-2", 1),
                ]),
            ),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );

        let children = &fanout_branch(&expanded).children;
        assert_eq!(children.len(), 2);
        let WorkspaceTreeItemDto::Node(first) = &children[0] else {
            panic!("expected first concrete child")
        };
        let WorkspaceTreeItemDto::Node(second) = &children[1] else {
            panic!("expected second concrete child")
        };
        assert_eq!(first.status, "running");
        assert_eq!(second.status, "running");
        assert_ne!(first.id, second.id);
    }

    #[test]
    fn branch_status_capabilities_and_session_activity_are_backend_aggregated() {
        let fanout = NodeDefinition {
            name: "checks".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec!["lint".to_string(), "test".to_string()],
                items: None,
            }),
            ..Default::default()
        };
        let mut plan = node(
            "plan-execution",
            "plan",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Succeeded,
        );
        plan.session_id = Some("plan-session".to_string());
        let mut parent = node(
            "fanout-parent",
            "checks",
            NodeKindName::Fanout,
            1,
            NodeExecutionStatus::Succeeded,
        );
        parent.completed_at = Some(3.0);
        let mut lint = node(
            "lint-execution",
            "lint",
            NodeKindName::Command,
            1,
            NodeExecutionStatus::Failed,
        );
        lint.fanout_parent = Some(crate::domain::workflow::FanoutParentRef {
            parent_node: "checks".to_string(),
            parent_attempt: 1,
            item_index: None,
            child_index: 0,
        });
        let mut test = node(
            "test-execution",
            "test",
            NodeKindName::Command,
            1,
            NodeExecutionStatus::WaitingApproval,
        );
        test.fanout_parent = Some(crate::domain::workflow::FanoutParentRef {
            parent_node: "checks".to_string(),
            parent_attempt: 1,
            item_index: None,
            child_index: 1,
        });
        let mut runtime = execution(vec![plan, parent, lint, test]);
        runtime.status = ExecutionStatus::WaitingApproval;
        let mut workflow_summary = summary();
        workflow_summary.status = ExecutionStatus::WaitingApproval;
        let projected = project_workspace_tree(
            "/repo",
            vec![WorkspaceSessionInput {
                id: "plan-session".to_string(),
                worktree_path: "/repo".to_string(),
                state: WorkspaceSessionState::Active,
                error_reason: None,
                updated_at: 5.0,
                first_message: "body excluded".to_string(),
                workflow_node_session: true,
                workflow_execution_id: Some(summary().execution_id),
                unresolved_recovery_reason: None,
            }],
            vec![workflow_summary],
            projection(
                definition(vec![
                    session_definition("plan"),
                    fanout,
                    command_definition("lint"),
                    command_definition("test"),
                ]),
                runtime,
            ),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        let WorkspaceTreeItemDto::Workflow(workflow) = &projected.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        assert_eq!(workflow.status, "waiting");
        assert!(workflow.capabilities.can_stop);
        assert!(!workflow.capabilities.can_resume);
        assert!(workflow.capabilities.can_abort);
        assert!(!workflow.capabilities.can_archive);

        let WorkspaceTreeItemDto::Node(plan) = &workflow.children[0] else {
            panic!("expected plan")
        };
        assert_eq!(
            plan.status, "running",
            "live session overrides completed attempt"
        );
        let WorkspaceTreeItemDto::Fanout(fanout) = &workflow.children[1] else {
            panic!("expected fanout")
        };
        assert_eq!(fanout.status, "failed");
        let WorkspaceTreeItemDto::Node(waiting) = &fanout.children[1] else {
            panic!("expected waiting child")
        };
        assert_eq!(waiting.status, "waiting");
        assert!(waiting.capabilities.can_approve);
    }

    #[test]
    fn resumable_workflow_exposes_the_durable_recovery_block_reason() {
        let execution_id = summary().execution_id;
        let mut workflow_summary = summary();
        workflow_summary.status = ExecutionStatus::Interrupted;
        let mut runtime = execution(vec![node(
            "plan-execution",
            "plan",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Running,
        )]);
        runtime.status = ExecutionStatus::Interrupted;
        let projected = project_workspace_tree(
            "/repo",
            vec![WorkspaceSessionInput {
                id: "plan-session".to_string(),
                worktree_path: "/repo".to_string(),
                state: WorkspaceSessionState::Done,
                error_reason: None,
                updated_at: 5.0,
                first_message: String::new(),
                workflow_node_session: true,
                workflow_execution_id: Some(execution_id),
                unresolved_recovery_reason: Some(
                    "Unresolved recovery recovery-1 must be resolved before resume.".to_string(),
                ),
            }],
            vec![workflow_summary],
            projection(definition(vec![session_definition("plan")]), runtime),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        let WorkspaceTreeItemDto::Workflow(workflow) = &projected.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        assert!(!workflow.capabilities.can_resume);
        assert_eq!(
            workflow.capabilities.resume_unavailable_reason.as_deref(),
            Some("Unresolved recovery recovery-1 must be resolved before resume.")
        );
    }

    #[test]
    fn command_detail_contains_only_masked_snapshot_and_standard_result() {
        let mut command = node(
            "command-attempt",
            "build",
            NodeKindName::Command,
            1,
            NodeExecutionStatus::Succeeded,
        );
        command.display_command = Some("echo ***".to_string());
        command.completed_at = Some(3.0);
        command.artifact = Some(Artifact {
            node_name: "build".to_string(),
            contract: Some("BuildOutput".to_string()),
            value: serde_json::json!({
                "ok": true,
                "exit_code": 0,
                "duration": 42,
                "stdout": "done",
                "stderr": "",
                "private_contract_field": "must not leak"
            }),
            produced_at: 3.0,
        });
        let projected = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(
                definition(vec![command_definition("build")]),
                execution(vec![command]),
            ),
            &[],
            &workflow_node_target("build"),
        );
        let WorkspaceTreeItemDto::Workflow(workflow) = &projected.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        let WorkspaceTreeItemDto::Node(node) = &workflow.children[0] else {
            panic!("expected node")
        };
        let detail = &projected.index.records[&node.id].detail;
        assert_eq!(projected.index.records.len(), 1);
        assert!(projected.index.session_node_ids.is_empty());
        let WorkspaceNodeContentDto::Command(content) = &detail.content else {
            panic!("expected command content")
        };
        assert_eq!(content.display_command.as_deref(), Some("echo ***"));
        assert_eq!(content.result.as_ref().unwrap().duration, 42);
        let detail_json = serde_json::to_string(detail).unwrap();
        assert!(!detail_json.contains("private_contract_field"));
        let summary_json = serde_json::to_string(&projected.snapshot).unwrap();
        assert!(!summary_json.contains("echo ***"));
        assert!(!summary_json.contains("done"));
    }

    #[test]
    fn command_detail_remains_bound_to_the_selected_occurrence() {
        let command_occurrence = |id: &str, attempt: u32, display_command: &str, stdout: &str| {
            let mut command = node(
                id,
                "build",
                NodeKindName::Command,
                attempt,
                NodeExecutionStatus::Succeeded,
            );
            command.display_command = Some(display_command.to_string());
            command.artifact = Some(Artifact {
                node_name: "build".to_string(),
                contract: None,
                value: serde_json::json!({
                    "exit_code": 0,
                    "duration": u64::from(attempt),
                    "stdout": stdout,
                    "stderr": ""
                }),
                produced_at: f64::from(attempt),
            });
            command
        };
        let first = command_occurrence("command-internal-1", 1, "echo first", "first");
        let second = command_occurrence("command-internal-2", 2, "echo second", "second");
        let workflow_definition = definition(vec![command_definition("build")]);
        let runtime = execution(vec![first, second]);

        let selected_first = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(workflow_definition.clone(), runtime.clone()),
            &[],
            &workflow_node_target_at("build", 0),
        );
        let selected_second = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(workflow_definition, runtime),
            &[],
            &workflow_node_target_at("build", 1),
        );
        let WorkspaceTreeItemDto::Workflow(workflow) = &selected_first.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        let node_ids = workflow
            .children
            .iter()
            .map(|item| match item {
                WorkspaceTreeItemDto::Node(node) => node.id.as_str(),
                _ => panic!("expected command node"),
            })
            .collect::<Vec<_>>();
        assert_eq!(node_ids.len(), 2);
        assert_ne!(node_ids[0], node_ids[1]);

        let WorkspaceNodeContentDto::Command(first_content) =
            &selected_first.index.records[node_ids[0]].detail.content
        else {
            panic!("expected first command content")
        };
        let WorkspaceNodeContentDto::Command(second_content) =
            &selected_second.index.records[node_ids[1]].detail.content
        else {
            panic!("expected second command content")
        };
        assert_eq!(first_content.display_command.as_deref(), Some("echo first"));
        assert_eq!(first_content.result.as_ref().unwrap().stdout, "first");
        assert_eq!(
            second_content.display_command.as_deref(),
            Some("echo second")
        );
        assert_eq!(second_content.result.as_ref().unwrap().stdout, "second");
    }

    #[test]
    fn snapshot_mode_never_materializes_command_payloads() {
        const DISPLAY_SENTINEL: &str = "SNAPSHOT_MUST_NOT_CLONE_DISPLAY_COMMAND";
        const OUTPUT_SENTINEL: &str = "SNAPSHOT_MUST_NOT_CLONE_COMMAND_OUTPUT";
        let mut command = node(
            "command-attempt",
            "build",
            NodeKindName::Command,
            1,
            NodeExecutionStatus::Succeeded,
        );
        command.display_command = Some(DISPLAY_SENTINEL.to_string());
        command.artifact = Some(Artifact {
            node_name: "build".to_string(),
            contract: None,
            value: serde_json::json!({
                "exit_code": 0,
                "duration": 1,
                "stdout": OUTPUT_SENTINEL,
                "stderr": OUTPUT_SENTINEL
            }),
            produced_at: 3.0,
        });
        let projected = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(
                definition(vec![command_definition("build")]),
                execution(vec![command]),
            ),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );

        assert!(projected.index.records.is_empty());
        assert!(projected.index.session_node_ids.is_empty());
        let summary_json = serde_json::to_string(&projected.snapshot).unwrap();
        assert!(!summary_json.contains(DISPLAY_SENTINEL));
        assert!(!summary_json.contains(OUTPUT_SENTINEL));
    }

    #[test]
    fn each_occurrence_keeps_its_detail_and_only_waiting_occurrence_can_approve() {
        let failed = node(
            "attempt-1",
            "review",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Failed,
        );
        let waiting = node(
            "attempt-2",
            "review",
            NodeKindName::Session,
            2,
            NodeExecutionStatus::WaitingApproval,
        );
        let first = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(
                definition(vec![session_definition("review")]),
                execution(vec![failed.clone()]),
            ),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );
        let retried_first_target = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(
                definition(vec![session_definition("review")]),
                execution(vec![failed.clone(), waiting.clone()]),
            ),
            &[],
            &workflow_node_target("review"),
        );
        let retried_second_target = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(
                definition(vec![session_definition("review")]),
                execution(vec![failed, waiting]),
            ),
            &[],
            &workflow_node_target_at("review", 1),
        );
        let leaf_ids = |projection: &WorkspaceProjection| {
            let WorkspaceTreeItemDto::Workflow(workflow) = &projection.snapshot.nodes[0] else {
                panic!("expected workflow")
            };
            workflow
                .children
                .iter()
                .map(|item| match item {
                    WorkspaceTreeItemDto::Node(node) => node.id.clone(),
                    _ => panic!("expected node"),
                })
                .collect::<Vec<_>>()
        };
        let first_id = leaf_ids(&first)[0].clone();
        let retried_ids = leaf_ids(&retried_first_target);
        assert_eq!(retried_ids.len(), 2);
        assert_eq!(first_id, retried_ids[0]);
        assert_ne!(retried_ids[0], retried_ids[1]);

        let first_record = &retried_first_target.index.records[&retried_ids[0]];
        assert_eq!(first_record.detail.status, "failed");
        assert!(!first_record.detail.capabilities.can_approve);
        assert!(first_record.approval.is_none());

        let second_record = &retried_second_target.index.records[&retried_ids[1]];
        assert_eq!(second_record.detail.status, "waiting");
        assert!(second_record.detail.capabilities.can_approve);
        assert_eq!(
            second_record.approval.as_ref().unwrap().node_execution_id,
            "attempt-2"
        );
    }

    #[test]
    fn missing_session_keeps_node_but_returns_no_unusable_session_id() {
        let mut running = node(
            "session-node",
            "plan",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Running,
        );
        running.session_id = Some("missing-session".to_string());
        let projected = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(
                definition(vec![session_definition("plan")]),
                execution(vec![running]),
            ),
            &[],
            &workflow_node_target("plan"),
        );
        let WorkspaceTreeItemDto::Workflow(workflow) = &projected.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        let WorkspaceTreeItemDto::Node(node) = &workflow.children[0] else {
            panic!("expected node")
        };
        assert_eq!(node.status, "running");
        let WorkspaceNodeContentDto::Session(content) =
            &projected.index.records[&node.id].detail.content
        else {
            panic!("expected session")
        };
        assert_eq!(content.session_id, None);
        assert!(projected.index.session_node_ids.is_empty());
    }

    #[test]
    fn stored_workflow_session_is_available_to_detail_and_opaque_lookup() {
        let mut running = node(
            "session-node",
            "plan",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Running,
        );
        running.session_id = Some("stored-session".to_string());
        let stored_session = WorkspaceSessionInput {
            id: "stored-session".to_string(),
            worktree_path: "/repo/".to_string(),
            state: WorkspaceSessionState::Active,
            error_reason: None,
            updated_at: 4.0,
            first_message: "stored body".to_string(),
            workflow_node_session: true,
            workflow_execution_id: Some(summary().execution_id),
            unresolved_recovery_reason: None,
        };
        let definition = definition(vec![session_definition("plan")]);
        let runtime = execution(vec![running]);
        let detail_projection = project_workspace_tree(
            "/repo",
            vec![stored_session.clone()],
            vec![summary()],
            projection(definition.clone(), runtime.clone()),
            &[],
            &workflow_node_target("plan"),
        );
        let node_id = workflow_node_target("plan");
        let WorkspaceProjectionTarget::Node(node_id) = node_id else {
            unreachable!()
        };
        let WorkspaceNodeContentDto::Session(content) =
            &detail_projection.index.records[&node_id].detail.content
        else {
            panic!("expected session")
        };
        assert_eq!(content.session_id.as_deref(), Some("stored-session"));

        let lookup_projection = project_workspace_tree(
            "/repo",
            vec![stored_session],
            vec![summary()],
            projection(definition, runtime),
            &[],
            &WorkspaceProjectionTarget::Session("stored-session".to_string()),
        );
        assert_eq!(
            lookup_projection
                .index
                .session_node_ids
                .get("stored-session"),
            Some(&node_id)
        );
        assert!(lookup_projection.index.records.is_empty());
    }

    #[test]
    fn repeated_session_occurrences_keep_distinct_session_detail_and_lookup() {
        let mut first = node(
            "session-node-1",
            "review",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Succeeded,
        );
        first.session_id = Some("stored-session-1".to_string());
        let mut second = node(
            "session-node-2",
            "review",
            NodeKindName::Session,
            2,
            NodeExecutionStatus::Running,
        );
        second.session_id = Some("stored-session-2".to_string());
        let sessions = vec![
            WorkspaceSessionInput {
                id: "stored-session-1".to_string(),
                worktree_path: "/repo".to_string(),
                state: WorkspaceSessionState::Done,
                error_reason: None,
                updated_at: 3.0,
                first_message: String::new(),
                workflow_node_session: true,
                workflow_execution_id: Some(summary().execution_id),
                unresolved_recovery_reason: None,
            },
            WorkspaceSessionInput {
                id: "stored-session-2".to_string(),
                worktree_path: "/repo".to_string(),
                state: WorkspaceSessionState::Active,
                error_reason: None,
                updated_at: 4.0,
                first_message: String::new(),
                workflow_node_session: true,
                workflow_execution_id: Some(summary().execution_id),
                unresolved_recovery_reason: None,
            },
        ];
        let workflow_definition = definition(vec![session_definition("review")]);
        let runtime = execution(vec![first, second]);

        let first_detail = project_workspace_tree(
            "/repo",
            sessions.clone(),
            vec![summary()],
            projection(workflow_definition.clone(), runtime.clone()),
            &[],
            &workflow_node_target_at("review", 0),
        );
        let second_detail = project_workspace_tree(
            "/repo",
            sessions.clone(),
            vec![summary()],
            projection(workflow_definition.clone(), runtime.clone()),
            &[],
            &workflow_node_target_at("review", 1),
        );
        let WorkspaceTreeItemDto::Workflow(workflow) = &first_detail.snapshot.nodes[0] else {
            panic!("expected workflow")
        };
        let node_ids = workflow
            .children
            .iter()
            .map(|item| match item {
                WorkspaceTreeItemDto::Node(node) => node.id.as_str(),
                _ => panic!("expected session node"),
            })
            .collect::<Vec<_>>();
        assert_eq!(node_ids.len(), 2);
        assert_ne!(node_ids[0], node_ids[1]);
        let session_id = |projection: &WorkspaceProjection, node_id: &str| {
            let WorkspaceNodeContentDto::Session(content) =
                &projection.index.records[node_id].detail.content
            else {
                panic!("expected Session content")
            };
            content.session_id.clone()
        };
        assert_eq!(
            session_id(&first_detail, node_ids[0]).as_deref(),
            Some("stored-session-1")
        );
        assert_eq!(
            session_id(&second_detail, node_ids[1]).as_deref(),
            Some("stored-session-2")
        );

        for (session_id, expected_node_id) in [
            ("stored-session-1", node_ids[0]),
            ("stored-session-2", node_ids[1]),
        ] {
            let lookup = project_workspace_tree(
                "/repo",
                sessions.clone(),
                vec![summary()],
                projection(workflow_definition.clone(), runtime.clone()),
                &[],
                &WorkspaceProjectionTarget::Session(session_id.to_string()),
            );
            assert_eq!(
                lookup.index.session_node_ids.get(session_id),
                Some(&expected_node_id.to_string())
            );
            assert!(lookup.index.records.is_empty());
        }
    }

    #[test]
    fn selection_reconciliation_keeps_a_leaf_that_remains_in_the_snapshot() {
        let started = node(
            "session-node",
            "plan",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Running,
        );
        let selected_node_id = match workflow_node_target("plan") {
            WorkspaceProjectionTarget::Node(node_id) => node_id,
            _ => unreachable!(),
        };
        let projected = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(
                definition(vec![session_definition("plan")]),
                execution(vec![started]),
            ),
            &[],
            &WorkspaceProjectionTarget::Snapshot,
        );

        let response = reconcile_workspace_tree_selection(projected.snapshot, &selected_node_id);

        assert!(response.reconciliation.selection_in_snapshot);
        assert_eq!(response.snapshot.preferred_node_id, Some(selected_node_id));
        assert_eq!(
            serde_json::to_value(&response).unwrap()["reconciliation"],
            serde_json::json!({ "selectionInSnapshot": true })
        );
    }

    #[test]
    fn archived_selection_reconciliation_uses_the_same_snapshot_preferred_leaf_or_null() {
        let started = node(
            "session-node",
            "plan",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Running,
        );
        let selected_node_id = match workflow_node_target("plan") {
            WorkspaceProjectionTarget::Node(node_id) => node_id,
            _ => unreachable!(),
        };
        let archives = vec![WorkflowExecutionManualArchiveRecord {
            execution_id: summary().execution_id,
            archived_at: 5.0,
        }];
        let direct_session = WorkspaceSessionInput {
            id: "direct-session".to_string(),
            worktree_path: "/repo".to_string(),
            state: WorkspaceSessionState::Active,
            error_reason: None,
            updated_at: 6.0,
            first_message: "fallback".to_string(),
            workflow_node_session: false,
            workflow_execution_id: None,
            unresolved_recovery_reason: None,
        };
        let project_snapshot = |sessions| {
            project_workspace_tree(
                "/repo",
                sessions,
                vec![summary()],
                projection(
                    definition(vec![session_definition("plan")]),
                    execution(vec![started.clone()]),
                ),
                &archives,
                &WorkspaceProjectionTarget::Snapshot,
            )
            .snapshot
        };

        let with_fallback = reconcile_workspace_tree_selection(
            project_snapshot(vec![direct_session]),
            &selected_node_id,
        );
        assert!(!with_fallback.reconciliation.selection_in_snapshot);
        assert!(with_fallback.snapshot.preferred_node_id.is_some());

        let without_fallback =
            reconcile_workspace_tree_selection(project_snapshot(Vec::new()), &selected_node_id);
        assert!(!without_fallback.reconciliation.selection_in_snapshot);
        assert_eq!(without_fallback.snapshot.preferred_node_id, None);
        assert_eq!(
            serde_json::to_value(&without_fallback).unwrap()["reconciliation"],
            serde_json::json!({ "selectionInSnapshot": false })
        );
    }

    #[test]
    fn archived_workflow_is_hidden_from_tree_but_selected_session_detail_remains_available() {
        let mut running = node(
            "session-node",
            "plan",
            NodeKindName::Session,
            1,
            NodeExecutionStatus::Running,
        );
        running.session_id = Some("stored-session".to_string());
        let stored_session = WorkspaceSessionInput {
            id: "stored-session".to_string(),
            worktree_path: "/repo".to_string(),
            state: WorkspaceSessionState::Active,
            error_reason: None,
            updated_at: 4.0,
            first_message: String::new(),
            workflow_node_session: true,
            workflow_execution_id: Some(summary().execution_id),
            unresolved_recovery_reason: None,
        };
        let archives = vec![WorkflowExecutionManualArchiveRecord {
            execution_id: summary().execution_id,
            archived_at: 5.0,
        }];

        let snapshot = project_workspace_tree(
            "/repo",
            vec![stored_session.clone()],
            vec![summary()],
            projection(
                definition(vec![session_definition("plan")]),
                execution(vec![running.clone()]),
            ),
            &archives,
            &WorkspaceProjectionTarget::Snapshot,
        );
        assert!(
            snapshot.snapshot.nodes.is_empty(),
            "archive still removes the Workflow branch from the tree"
        );

        let target = workflow_node_target("plan");
        let WorkspaceProjectionTarget::Node(node_id) = &target else {
            unreachable!()
        };
        let detail = project_workspace_tree(
            "/repo",
            vec![stored_session.clone()],
            vec![summary()],
            projection(
                definition(vec![session_definition("plan")]),
                execution(vec![running.clone()]),
            ),
            &archives,
            &target,
        );
        let WorkspaceNodeContentDto::Session(content) =
            &detail.index.records[node_id].detail.content
        else {
            panic!("expected session detail")
        };
        assert_eq!(content.session_id.as_deref(), Some("stored-session"));
        assert!(!detail.index.records[node_id].detail.capabilities.can_close);
        assert!(detail.index.records[node_id].close.is_none());

        let lookup = project_workspace_tree(
            "/repo",
            vec![stored_session],
            vec![summary()],
            projection(
                definition(vec![session_definition("plan")]),
                execution(vec![running]),
            ),
            &archives,
            &WorkspaceProjectionTarget::Session("stored-session".to_string()),
        );
        assert_eq!(
            lookup.index.session_node_ids.get("stored-session"),
            Some(node_id)
        );
    }

    #[test]
    fn failure_metadata_never_enters_workspace_summary_or_detail() {
        let mut failed = node(
            "internal-node-id",
            "build",
            NodeKindName::Command,
            1,
            NodeExecutionStatus::Failed,
        );
        failed.failure = Some(NodeExecutionFailure {
            reason: "raw internal failure".to_string(),
            kind: NodeExecutionFailureKind::InfrastructureCrash,
        });
        let projected = project_workspace_tree(
            "/repo",
            Vec::new(),
            vec![summary()],
            projection(
                definition(vec![command_definition("build")]),
                execution(vec![failed]),
            ),
            &[],
            &workflow_node_target("build"),
        );
        let serialized = serde_json::to_string(&projected.snapshot).unwrap();
        assert!(!serialized.contains("internal-node-id"));
        assert!(!serialized.contains("raw internal failure"));
        let details = projected
            .index
            .records
            .values()
            .map(|record| serde_json::to_string(&record.detail).unwrap())
            .collect::<String>();
        assert!(!details.contains("internal-node-id"));
        assert!(!details.contains("raw internal failure"));
    }
}
