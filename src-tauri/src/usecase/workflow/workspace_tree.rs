use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::query_service::WorkflowQueryService;
use super::WorkflowUsecase;
use crate::domain::workflow::status_aggregation::RepresentativeStatus;
use crate::domain::workflow::{
    Artifact, ExecutionListFilter, FanoutParentRef, NodeExecution, NodeExecutionStatus,
    WorkflowError, WorkflowExecution, WorkflowExecutionId, WorkflowExecutionManualArchiveRecord,
    WorkflowExecutionSummary, WORKFLOW_ARCHIVE_REASON_MANUAL,
};

const DEFAULT_SESSION_TITLE: &str = "NewSession";

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
    pub updated_at: f64,
    pub first_message: String,
    pub workflow_node_session: bool,
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
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum WorkspaceTreeNodeDto {
    Session(WorkspaceSessionNodeDto),
    Workflow(WorkspaceWorkflowExecutionDto),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceSessionNodeDto {
    pub id: String,
    pub worktree_path: String,
    pub title: String,
    pub state: WorkspaceSessionState,
    pub updated_at: f64,
    pub workflow_node_session: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowExecutionDto {
    pub execution_id: String,
    pub worktree_path: String,
    pub workflow_name: String,
    pub title: String,
    pub status: String,
    pub can_stop: bool,
    pub can_resume: bool,
    pub can_abort: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interruption_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_from_node: Option<String>,
    pub updated_at: f64,
    pub node_executions: Vec<WorkspaceWorkflowNodeExecutionDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowNodeExecutionDto {
    pub kind: &'static str,
    pub execution_id: String,
    pub worktree_path: String,
    pub title: String,
    pub node_name: String,
    pub node_kind: &'static str,
    pub status: String,
    pub node_execution_status: String,
    pub can_approve: bool,
    pub updated_at: f64,
    pub attempt: u32,
    pub node_execution_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<WorkspaceArtifactDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fanout_parent: Option<WorkspaceFanoutParentDto>,
    pub sessions: Vec<WorkspaceSessionNodeDto>,
}

/// WorkflowView 向けの選択 NodeExecution read model。
///
/// execution-level action permission と interrupted checkpoint は同じ node が tree に
/// 複数並ぶ場合もあるため、一覧 DTO には複製せず detail response にだけ載せる。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowNodeDetailDto {
    #[serde(flatten)]
    pub node_execution: WorkspaceWorkflowNodeExecutionDto,
    pub execution_status: String,
    pub can_stop: bool,
    pub can_resume: bool,
    pub can_abort: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interruption_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_from_node: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceArtifactDto {
    pub node_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    pub value: serde_json::Value,
    pub produced_at: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceFanoutParentDto {
    pub parent_node: String,
    pub parent_attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_index: Option<usize>,
    pub child_index: usize,
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

impl WorkflowUsecase {
    pub(crate) fn collect_workspace_session_inputs(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
        let mut sessions = self.sessions.list_active_sessions(worktree_path)?;
        sessions.extend(
            self.sessions
                .list_closed_sessions(worktree_path)?
                .into_iter()
                .filter(|session| session.workflow_node_session),
        );
        Ok(sessions)
    }

    pub(crate) fn list_workspace_tree_nodes(
        &self,
        worktree_path: &str,
        sessions: Vec<WorkspaceSessionInput>,
    ) -> Result<Vec<WorkspaceTreeNodeDto>, WorkflowError> {
        let worktree_path = self.resolve_worktree_path(worktree_path)?;
        let archives = self.execution_archives.manual_archive_records()?;
        self.query
            .list_workspace_tree_nodes(&worktree_path, sessions, &archives)
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

    pub(crate) fn get_workspace_workflow_node_detail(
        &self,
        worktree_path: &str,
        execution_id: &str,
        node_execution_id: &str,
        sessions: Vec<WorkspaceSessionInput>,
    ) -> Result<Option<WorkspaceWorkflowNodeDetailDto>, WorkflowError> {
        let worktree_path = self.resolve_worktree_path(worktree_path)?;
        self.query.get_workspace_workflow_node_detail(
            &worktree_path,
            execution_id,
            node_execution_id,
            sessions,
        )
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

impl WorkflowQueryService {
    pub(in crate::usecase::workflow) fn list_workspace_tree_nodes(
        &self,
        worktree_path: &str,
        sessions: Vec<WorkspaceSessionInput>,
        archives: &[WorkflowExecutionManualArchiveRecord],
    ) -> Result<Vec<WorkspaceTreeNodeDto>, WorkflowError> {
        let summaries = self.list_executions(ExecutionListFilter {
            status: None,
            worktree_path: Some(worktree_path.to_string()),
        })?;
        let executions = self.executions_for_summaries(&summaries)?;
        Ok(project_workspace_tree_nodes(
            sessions, summaries, executions, archives,
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

    pub(in crate::usecase::workflow) fn get_workspace_workflow_node_detail(
        &self,
        worktree_path: &str,
        execution_id: &str,
        node_execution_id: &str,
        sessions: Vec<WorkspaceSessionInput>,
    ) -> Result<Option<WorkspaceWorkflowNodeDetailDto>, WorkflowError> {
        let Some(summary) = self.get_execution(execution_id)? else {
            return Ok(None);
        };
        if summary.worktree_path != worktree_path {
            return Ok(None);
        }
        let Some(execution) = self.get_execution_state(execution_id)? else {
            return Ok(None);
        };
        let Some(node_execution) = execution
            .node_executions
            .iter()
            .find(|node_execution| node_execution.id == node_execution_id)
        else {
            return Ok(None);
        };
        let sessions = session_index(sessions);
        Ok(Some(workspace_node_detail(
            &summary,
            &execution,
            node_execution,
            &sessions,
        )))
    }

    fn executions_for_summaries(
        &self,
        summaries: &[WorkflowExecutionSummary],
    ) -> Result<HashMap<String, WorkflowExecution>, WorkflowError> {
        let mut executions = HashMap::new();
        for summary in summaries {
            if let Some(execution) = self.get_execution_state(&summary.execution_id)? {
                executions.insert(summary.execution_id.clone(), execution);
            }
        }
        Ok(executions)
    }
}

fn project_workspace_tree_nodes(
    sessions: Vec<WorkspaceSessionInput>,
    summaries: Vec<WorkflowExecutionSummary>,
    executions: HashMap<String, WorkflowExecution>,
    archives: &[WorkflowExecutionManualArchiveRecord],
) -> Vec<WorkspaceTreeNodeDto> {
    let nested_session_ids = executions
        .values()
        .flat_map(|execution| execution.node_executions.iter())
        .filter_map(|node_execution| node_execution.session_id.clone())
        .collect::<HashSet<_>>();
    let session_index = session_index(sessions.clone());

    let mut direct_sessions = sessions
        .into_iter()
        .filter(|session| {
            !session.workflow_node_session && !nested_session_ids.contains(&session.id)
        })
        .map(|session| session_node(session, None, None, None))
        .collect::<Vec<_>>();
    direct_sessions.sort_by(compare_session_nodes);

    let mut workflow_executions = summaries
        .into_iter()
        .filter(|summary| !is_workflow_archived(&summary.execution_id, archives))
        .map(|summary| {
            let execution = executions.get(&summary.execution_id);
            workspace_workflow_execution(summary, execution, &session_index)
        })
        .collect::<Vec<_>>();
    workflow_executions.sort_by(|left, right| compare_titles(&left.title, &right.title));

    direct_sessions
        .into_iter()
        .map(WorkspaceTreeNodeDto::Session)
        .chain(
            workflow_executions
                .into_iter()
                .map(WorkspaceTreeNodeDto::Workflow),
        )
        .collect()
}

fn workspace_workflow_execution(
    summary: WorkflowExecutionSummary,
    execution: Option<&WorkflowExecution>,
    sessions: &HashMap<String, WorkspaceSessionInput>,
) -> WorkspaceWorkflowExecutionDto {
    let status = execution
        .map(|execution| execution.status)
        .unwrap_or(summary.status);
    let updated_at = execution
        .map(|execution| execution.updated_at)
        .unwrap_or(summary.updated_at);
    let interruption_reason = execution
        .and_then(|execution| execution.interruption_reason.as_ref())
        .or(summary.interruption_reason.as_ref())
        .map(|reason| reason.as_str().to_string());
    let resume_from_node = execution
        .and_then(|execution| execution.resume_from_node.clone())
        .or_else(|| summary.resume_from_node.clone());
    let node_executions = execution
        .map(|execution| {
            execution
                .node_executions
                .iter()
                .map(|node_execution| {
                    workspace_node_execution(&summary, execution, node_execution, sessions)
                })
                .collect()
        })
        .unwrap_or_default();

    WorkspaceWorkflowExecutionDto {
        execution_id: summary.execution_id.clone(),
        worktree_path: summary.worktree_path.clone(),
        workflow_name: summary.workflow_name.clone(),
        title: workflow_title(&summary),
        status: representative_status(status.as_str()),
        can_stop: status.can_stop(),
        can_resume: status.can_resume(),
        can_abort: status.can_abort(),
        interruption_reason,
        resume_from_node,
        updated_at,
        node_executions,
    }
}

fn workspace_node_execution(
    summary: &WorkflowExecutionSummary,
    execution: &WorkflowExecution,
    node_execution: &NodeExecution,
    sessions: &HashMap<String, WorkspaceSessionInput>,
) -> WorkspaceWorkflowNodeExecutionDto {
    let nested_sessions = node_execution
        .session_id
        .as_ref()
        .and_then(|session_id| sessions.get(session_id))
        .cloned()
        .map(|session| {
            vec![session_node(
                session,
                Some(node_execution.id.clone()),
                Some(node_execution.node_name.clone()),
                Some(node_execution.attempt),
            )]
        })
        .unwrap_or_default();

    WorkspaceWorkflowNodeExecutionDto {
        kind: "node",
        execution_id: node_execution.execution_id.clone(),
        worktree_path: summary.worktree_path.clone(),
        title: node_execution.node_name.clone(),
        node_name: node_execution.node_name.clone(),
        node_kind: node_execution.kind.as_str(),
        status: representative_status(node_execution.status.as_str()),
        node_execution_status: node_execution.status.as_str().to_string(),
        can_approve: node_execution.status == NodeExecutionStatus::WaitingApproval,
        updated_at: node_execution
            .completed_at
            .unwrap_or_else(|| execution.updated_at.max(node_execution.started_at)),
        attempt: node_execution.attempt,
        node_execution_id: node_execution.id.clone(),
        session_id: node_execution.session_id.clone(),
        artifact: node_execution.artifact.as_ref().map(workspace_artifact),
        fanout_parent: node_execution
            .fanout_parent
            .as_ref()
            .map(workspace_fanout_parent),
        sessions: nested_sessions,
    }
}

fn workspace_node_detail(
    summary: &WorkflowExecutionSummary,
    execution: &WorkflowExecution,
    node_execution: &NodeExecution,
    sessions: &HashMap<String, WorkspaceSessionInput>,
) -> WorkspaceWorkflowNodeDetailDto {
    let status = execution.status;
    WorkspaceWorkflowNodeDetailDto {
        node_execution: workspace_node_execution(summary, execution, node_execution, sessions),
        execution_status: representative_status(status.as_str()),
        can_stop: status.can_stop(),
        can_resume: status.can_resume(),
        can_abort: status.can_abort(),
        interruption_reason: execution
            .interruption_reason
            .as_ref()
            .map(|reason| reason.as_str().to_string()),
        resume_from_node: execution.resume_from_node.clone(),
    }
}

fn workspace_artifact(artifact: &Artifact) -> WorkspaceArtifactDto {
    WorkspaceArtifactDto {
        node_name: artifact.node_name.clone(),
        contract: artifact.contract.clone(),
        value: artifact.value.clone(),
        produced_at: artifact.produced_at,
    }
}

fn workspace_fanout_parent(parent: &FanoutParentRef) -> WorkspaceFanoutParentDto {
    WorkspaceFanoutParentDto {
        parent_node: parent.parent_node.clone(),
        parent_attempt: parent.parent_attempt,
        item_index: parent.item_index,
        child_index: parent.child_index,
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
                status: representative_status(summary.status.as_str()),
                updated_at: summary.updated_at,
                archived_at: record.archived_at,
                archive_reason: WORKFLOW_ARCHIVE_REASON_MANUAL.to_string(),
            })
        })
        .collect::<Vec<_>>();
    history.sort_by(|left, right| {
        right
            .archived_at
            .partial_cmp(&left.archived_at)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| compare_titles(&left.title, &right.title))
            .then_with(|| left.execution_id.cmp(&right.execution_id))
    });
    history
}

fn session_index(sessions: Vec<WorkspaceSessionInput>) -> HashMap<String, WorkspaceSessionInput> {
    sessions
        .into_iter()
        .map(|session| (session.id.clone(), session))
        .collect()
}

fn session_node(
    session: WorkspaceSessionInput,
    node_execution_id: Option<String>,
    node_name: Option<String>,
    attempt: Option<u32>,
) -> WorkspaceSessionNodeDto {
    let title = node_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            let first_message = session.first_message.trim();
            (!first_message.is_empty()).then(|| first_message.to_string())
        })
        .unwrap_or_else(|| DEFAULT_SESSION_TITLE.to_string());

    WorkspaceSessionNodeDto {
        id: session.id,
        worktree_path: session.worktree_path,
        title,
        state: session.state,
        updated_at: session.updated_at,
        workflow_node_session: session.workflow_node_session,
        node_execution_id,
        node_name,
        attempt,
    }
}

fn representative_status(status: &str) -> String {
    RepresentativeStatus::from_status_str(status)
        .as_str()
        .to_string()
}

fn workflow_title(summary: &WorkflowExecutionSummary) -> String {
    let workflow_name = summary.workflow_name.trim();
    if workflow_name.is_empty() {
        summary.execution_id.clone()
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

fn compare_session_nodes(
    left: &WorkspaceSessionNodeDto,
    right: &WorkspaceSessionNodeDto,
) -> std::cmp::Ordering {
    compare_titles(&left.title, &right.title).then_with(|| left.id.cmp(&right.id))
}

fn compare_titles(left: &str, right: &str) -> std::cmp::Ordering {
    left.to_lowercase()
        .cmp(&right.to_lowercase())
        .then_with(|| left.cmp(right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        ExecutionInterruptionReason, ExecutionOrigin, ExecutionStatus, NodeExecutionStatus,
        NodeKindName, TokenUsage,
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

    fn execution() -> WorkflowExecution {
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
            node_executions: vec![NodeExecution {
                id: "node-execution-plan".to_string(),
                execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
                node_name: "plan".to_string(),
                kind: NodeKindName::Session,
                attempt: 1,
                status: NodeExecutionStatus::Running,
                session_id: Some("session-plan".to_string()),
                result_summary: None,
                artifact: None,
                token_usage: None,
                failure: None,
                fanout_parent: None,
                started_at: 2.0,
                completed_at: None,
            }],
            artifacts: Vec::new(),
            fanouts: Vec::new(),
            approval_target: None,
        }
    }

    fn node_execution(
        id: &str,
        node_name: &str,
        status: NodeExecutionStatus,
        fanout_parent: Option<FanoutParentRef>,
    ) -> NodeExecution {
        NodeExecution {
            id: id.to_string(),
            execution_id: "00000000-0000-4000-8000-000000000001".to_string(),
            node_name: node_name.to_string(),
            kind: NodeKindName::Session,
            attempt: 1,
            status,
            session_id: None,
            result_summary: None,
            artifact: None,
            token_usage: None,
            failure: None,
            fanout_parent,
            started_at: 2.0,
            completed_at: None,
        }
    }

    #[test]
    fn workspace_tree_projects_only_canonical_node_executions() {
        let summary = summary();
        let execution = execution();
        let nodes = project_workspace_tree_nodes(
            vec![WorkspaceSessionInput {
                id: "session-plan".to_string(),
                worktree_path: "/repo".to_string(),
                state: WorkspaceSessionState::Active,
                updated_at: 3.0,
                first_message: "Plan".to_string(),
                workflow_node_session: true,
            }],
            vec![summary.clone()],
            HashMap::from([(summary.execution_id.clone(), execution)]),
            &[],
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow execution")
        };
        assert_eq!(workflow.node_executions.len(), 1);
        assert_eq!(
            workflow.node_executions[0].node_execution_id,
            "node-execution-plan"
        );
        let value = serde_json::to_value(workflow).unwrap();
        assert!(value.get("steps").is_none());
        assert!(value["nodeExecutions"][0].get("stepName").is_none());
        assert!(value["nodeExecutions"][0].get("runIndex").is_none());
    }

    #[test]
    fn missing_execution_projection_does_not_reconstruct_legacy_nodes() {
        let nodes = project_workspace_tree_nodes(Vec::new(), vec![summary()], HashMap::new(), &[]);
        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow execution")
        };
        assert!(workflow.node_executions.is_empty());
    }

    #[test]
    fn interrupted_execution_projects_checkpoint_and_backend_owned_actions() {
        let summary = WorkflowExecutionSummary {
            status: ExecutionStatus::Interrupted,
            current_node: None,
            interruption_reason: Some(ExecutionInterruptionReason::Stop),
            resume_from_node: Some("review".to_string()),
            ..summary()
        };
        let nodes =
            project_workspace_tree_nodes(Vec::new(), vec![summary.clone()], HashMap::new(), &[]);

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow execution")
        };
        assert_eq!(workflow.status, "interrupted");
        assert!(!workflow.can_stop);
        assert!(workflow.can_resume);
        assert!(workflow.can_abort);
        assert_eq!(workflow.interruption_reason.as_deref(), Some("stop"));
        assert_eq!(workflow.resume_from_node.as_deref(), Some("review"));

        let value = serde_json::to_value(workflow).unwrap();
        assert_eq!(value["interruptionReason"], "stop");
        assert_eq!(value["resumeFromNode"], "review");
        assert_eq!(value["canStop"], false);
        assert_eq!(value["canResume"], true);
        assert_eq!(value["canAbort"], true);

        let execution = WorkflowExecution {
            status: ExecutionStatus::Interrupted,
            current_node: None,
            interruption_reason: Some(ExecutionInterruptionReason::Stop),
            resume_from_node: Some("review".to_string()),
            node_executions: vec![node_execution(
                "node-execution-review",
                "review",
                NodeExecutionStatus::Aborted,
                None,
            )],
            ..execution()
        };
        let detail = workspace_node_detail(
            &summary,
            &execution,
            &execution.node_executions[0],
            &HashMap::new(),
        );
        assert_eq!(detail.execution_status, "interrupted");
        assert!(!detail.can_stop);
        assert!(detail.can_resume);
        assert!(detail.can_abort);
        assert_eq!(detail.interruption_reason.as_deref(), Some("stop"));
        assert_eq!(detail.resume_from_node.as_deref(), Some("review"));

        let value = serde_json::to_value(detail).unwrap();
        assert_eq!(value["executionStatus"], "interrupted");
        assert_eq!(value["interruptionReason"], "stop");
        assert_eq!(value["resumeFromNode"], "review");
        assert_eq!(value["nodeExecutionId"], "node-execution-review");
        assert!(value.get("nodeExecution").is_none());
    }

    #[test]
    fn node_detail_does_not_fall_back_to_stale_summary_checkpoint() {
        let stale_summary = WorkflowExecutionSummary {
            status: ExecutionStatus::Interrupted,
            current_node: None,
            interruption_reason: Some(ExecutionInterruptionReason::Stop),
            resume_from_node: Some("review".to_string()),
            ..summary()
        };
        let execution = execution();

        let detail = workspace_node_detail(
            &stale_summary,
            &execution,
            &execution.node_executions[0],
            &HashMap::new(),
        );

        assert_eq!(detail.execution_status, "running");
        assert_eq!(detail.interruption_reason, None);
        assert_eq!(detail.resume_from_node, None);
    }

    #[test]
    fn multiple_waiting_fanout_children_are_individually_approvable() {
        let summary = summary();
        let execution = WorkflowExecution {
            status: ExecutionStatus::WaitingApproval,
            current_node: Some("reviews".to_string()),
            node_executions: vec![
                node_execution("parent", "reviews", NodeExecutionStatus::Running, None),
                node_execution(
                    "child-1",
                    "review",
                    NodeExecutionStatus::WaitingApproval,
                    Some(FanoutParentRef {
                        parent_node: "reviews".to_string(),
                        parent_attempt: 1,
                        item_index: Some(0),
                        child_index: 0,
                    }),
                ),
                node_execution(
                    "child-2",
                    "review",
                    NodeExecutionStatus::WaitingApproval,
                    Some(FanoutParentRef {
                        parent_node: "reviews".to_string(),
                        parent_attempt: 1,
                        item_index: Some(1),
                        child_index: 0,
                    }),
                ),
            ],
            approval_target: None,
            ..execution()
        };
        let nodes = project_workspace_tree_nodes(
            Vec::new(),
            vec![summary.clone()],
            HashMap::from([(summary.execution_id.clone(), execution)]),
            &[],
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow execution")
        };
        let approvable = workflow
            .node_executions
            .iter()
            .filter(|node| node.can_approve)
            .map(|node| node.node_execution_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(approvable, vec!["child-1", "child-2"]);
    }

    #[test]
    fn single_waiting_approval_still_sets_can_approve() {
        let summary = summary();
        let execution = WorkflowExecution {
            status: ExecutionStatus::WaitingApproval,
            node_executions: vec![node_execution(
                "node-execution-plan",
                "plan",
                NodeExecutionStatus::WaitingApproval,
                None,
            )],
            approval_target: Some(crate::domain::workflow::ApprovalTarget {
                node_execution_id: "node-execution-plan".to_string(),
                node_name: "plan".to_string(),
                session_id: None,
            }),
            ..execution()
        };
        let nodes = project_workspace_tree_nodes(
            Vec::new(),
            vec![summary.clone()],
            HashMap::from([(summary.execution_id.clone(), execution)]),
            &[],
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow execution")
        };
        assert!(workflow.node_executions[0].can_approve);
    }
}
