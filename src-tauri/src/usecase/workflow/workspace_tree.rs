use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::usecase::workflow::query_service::WorkflowQueryService;
use crate::{
    domain::workflow::{
        NodeType, RunId, RunListFilter, RunStatus, WorkflowError, WorkflowRunManualArchiveRecord,
        WorkflowRunSummary, WorkflowStateSnapshot, WorkflowStepContext, STEP_STATE_ABORTED,
        STEP_STATE_COMPLETED, STEP_STATE_FAILED, STEP_STATE_PENDING, STEP_STATE_RUNNING,
        STEP_STATE_WAITING_APPROVAL, WORKFLOW_ARCHIVE_REASON_MANUAL,
    },
    other::utils::unix_timestamp_seconds,
};

use super::WorkflowUsecase;

const DEFAULT_SESSION_TITLE: &str = "NewSession";

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
    pub workflow_step_session: bool,
    pub workflow_step_context: Option<WorkflowStepContext>,
}

impl WorkspaceSessionInput {
    fn is_workflow_step_session(&self) -> bool {
        self.workflow_step_session || self.workflow_step_context.is_some()
    }
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
    Workflow(WorkspaceWorkflowNodeDto),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceSessionNodeDto {
    pub id: String,
    pub worktree_path: String,
    pub title: String,
    pub state: WorkspaceSessionState,
    pub updated_at: f64,
    pub workflow_step_session: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowNodeDto {
    pub run_id: String,
    pub worktree_path: String,
    pub workflow_name: String,
    pub title: String,
    pub status: String,
    pub updated_at: f64,
    pub steps: Vec<WorkspaceWorkflowStepNodeDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowStepNodeDto {
    pub kind: &'static str,
    pub id: String,
    pub run_id: String,
    pub worktree_path: String,
    pub title: String,
    pub status: String,
    pub step_type: &'static str,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub can_reject: Option<bool>,
    pub updated_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_index: Option<u32>,
    pub sessions: Vec<WorkspaceSessionNodeDto>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceWorkflowHistoryItemDto {
    pub run_id: String,
    pub worktree_path: String,
    pub title: String,
    pub status: String,
    pub updated_at: f64,
    pub archived_at: f64,
    pub archive_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StepSessionRef {
    session_id: Option<String>,
    step_name: String,
    run_index: Option<u32>,
    group_step_name: String,
    group_run_index: Option<u32>,
    state: String,
    order: usize,
}

impl WorkflowUsecase {
    pub(crate) fn collect_workspace_session_inputs(
        &self,
        sessions: &dyn WorkspaceSessionGateway,
        worktree_path: &str,
    ) -> Result<Vec<WorkspaceSessionInput>, WorkflowError> {
        let mut active_sessions = sessions.list_active_sessions(worktree_path)?;
        active_sessions.extend(
            sessions
                .list_closed_sessions(worktree_path)?
                .into_iter()
                .filter(WorkspaceSessionInput::is_workflow_step_session),
        );
        Ok(active_sessions)
    }

    pub(crate) fn list_workspace_tree_nodes(
        &self,
        worktree_path: &str,
        sessions: Vec<WorkspaceSessionInput>,
    ) -> Result<Vec<WorkspaceTreeNodeDto>, WorkflowError> {
        let worktree_path = self.resolve_worktree_path(worktree_path)?;
        let archives = self.archive_runs.manual_archive_records()?;
        self.query
            .list_workspace_tree_nodes(&worktree_path, sessions, &archives)
    }

    pub(crate) fn list_workspace_workflow_history(
        &self,
        worktree_path: &str,
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkflowError> {
        let worktree_path = self.resolve_worktree_path(worktree_path)?;
        let archives = self.archive_runs.manual_archive_records()?;
        self.query
            .list_workspace_workflow_history(&worktree_path, &archives)
    }

    pub(crate) fn get_workspace_workflow_step_detail(
        &self,
        worktree_path: &str,
        run_id: &str,
        step_id: &str,
        sessions: Vec<WorkspaceSessionInput>,
    ) -> Result<Option<WorkspaceWorkflowStepNodeDto>, WorkflowError> {
        let worktree_path = self.resolve_worktree_path(worktree_path)?;
        self.query
            .get_workspace_workflow_step_detail(&worktree_path, run_id, step_id, sessions)
    }

    pub(crate) fn archive_workspace_workflow_run(
        &self,
        worktree_path: &str,
        run_id: &str,
    ) -> Result<(), WorkflowError> {
        let run_id = RunId::new(run_id.to_string())?;
        let Some(_) = self.authorize_run_summary_for_worktree(run_id.as_str(), worktree_path)?
        else {
            return Err(WorkflowError::external(format!(
                "Workflow run not found: {run_id}"
            )));
        };
        self.archive_runs
            .archive_manual(&run_id, unix_timestamp_seconds())
    }

    pub(crate) fn restore_workspace_workflow_run(
        &self,
        worktree_path: &str,
        run_id: &str,
    ) -> Result<(), WorkflowError> {
        let run_id = RunId::new(run_id.to_string())?;
        let Some(_) = self.authorize_run_summary_for_worktree(run_id.as_str(), worktree_path)?
        else {
            return Err(WorkflowError::external(format!(
                "Workflow run not found: {run_id}"
            )));
        };
        self.archive_runs
            .restore_manual(&run_id, unix_timestamp_seconds())
    }
}

impl WorkflowQueryService {
    pub(in crate::usecase::workflow) fn list_workspace_tree_nodes(
        &self,
        worktree_path: &str,
        sessions: Vec<WorkspaceSessionInput>,
        archives: &[WorkflowRunManualArchiveRecord],
    ) -> Result<Vec<WorkspaceTreeNodeDto>, WorkflowError> {
        let runs = self.list_runs(RunListFilter {
            status: None,
            worktree_path: Some(worktree_path.to_string()),
        })?;
        let states = self.states_for_runs(&runs)?;
        Ok(project_workspace_tree_nodes(
            sessions, runs, states, archives,
        ))
    }

    pub(in crate::usecase::workflow) fn list_workspace_workflow_history(
        &self,
        worktree_path: &str,
        archives: &[WorkflowRunManualArchiveRecord],
    ) -> Result<Vec<WorkspaceWorkflowHistoryItemDto>, WorkflowError> {
        let runs = self.list_runs(RunListFilter {
            status: None,
            worktree_path: Some(worktree_path.to_string()),
        })?;
        Ok(project_workspace_workflow_history(runs, archives))
    }

    pub(in crate::usecase::workflow) fn get_workspace_workflow_step_detail(
        &self,
        worktree_path: &str,
        run_id: &str,
        step_id: &str,
        sessions: Vec<WorkspaceSessionInput>,
    ) -> Result<Option<WorkspaceWorkflowStepNodeDto>, WorkflowError> {
        let run_id = RunId::new(run_id.to_string())?;
        let Some(run) = self.get_run(run_id.as_str())? else {
            return Ok(None);
        };
        if run.worktree_path != worktree_path {
            return Ok(None);
        }
        let state = self.get_run_state(run_id.as_str())?;
        let workflow_sessions = sessions
            .into_iter()
            .filter(|session| session.is_workflow_step_session())
            .map(|session| (session.id.clone(), session))
            .collect::<HashMap<_, _>>();
        Ok(workflow_step_detail_node(
            run,
            &workflow_sessions,
            state.as_ref(),
            step_id,
        ))
    }

    fn states_for_runs(
        &self,
        runs: &[WorkflowRunSummary],
    ) -> Result<HashMap<String, Option<WorkflowStateSnapshot>>, WorkflowError> {
        let mut states = HashMap::new();
        for run in runs {
            states.insert(run.run_id.clone(), self.get_run_state(&run.run_id)?);
        }
        Ok(states)
    }
}

fn project_workspace_tree_nodes(
    sessions: Vec<WorkspaceSessionInput>,
    runs: Vec<WorkflowRunSummary>,
    states: HashMap<String, Option<WorkflowStateSnapshot>>,
    archives: &[WorkflowRunManualArchiveRecord],
) -> Vec<WorkspaceTreeNodeDto> {
    let mut direct_sessions = Vec::new();
    let mut workflow_sessions: HashMap<String, WorkspaceSessionInput> = HashMap::new();

    for session in sessions {
        if session.is_workflow_step_session() {
            workflow_sessions.insert(session.id.clone(), session);
        } else {
            direct_sessions.push(session_node(session, None, None));
        }
    }

    direct_sessions.sort_by(compare_session_nodes);

    let mut workflow_nodes: Vec<WorkspaceWorkflowNodeDto> = runs
        .into_iter()
        .filter_map(|run| {
            if is_workflow_archived(&run.run_id, archives) {
                return None;
            }
            let state = states.get(&run.run_id).and_then(|state| state.as_ref());
            let step_refs = step_refs_for_run(&run.run_id, &workflow_sessions, state);
            Some(workflow_node(run, &workflow_sessions, step_refs, state))
        })
        .collect();

    workflow_nodes.sort_by(|a, b| compare_titles(&a.title, &b.title));

    direct_sessions
        .into_iter()
        .map(WorkspaceTreeNodeDto::Session)
        .chain(
            workflow_nodes
                .into_iter()
                .map(WorkspaceTreeNodeDto::Workflow),
        )
        .collect()
}

fn workflow_node(
    run: WorkflowRunSummary,
    workflow_sessions: &HashMap<String, WorkspaceSessionInput>,
    step_refs: Vec<StepSessionRef>,
    state: Option<&WorkflowStateSnapshot>,
) -> WorkspaceWorkflowNodeDto {
    let steps = workflow_steps(&run, workflow_sessions, &step_refs, state);

    WorkspaceWorkflowNodeDto {
        run_id: run.run_id.clone(),
        worktree_path: run.worktree_path.clone(),
        workflow_name: run.workflow_name.clone(),
        title: workflow_title(&run),
        status: run_status_label(run.status).to_string(),
        updated_at: run.updated_at,
        steps,
    }
}

fn workflow_step_detail_node(
    run: WorkflowRunSummary,
    workflow_sessions: &HashMap<String, WorkspaceSessionInput>,
    state: Option<&WorkflowStateSnapshot>,
    step_id: &str,
) -> Option<WorkspaceWorkflowStepNodeDto> {
    let step_refs = step_refs_for_run(&run.run_id, workflow_sessions, state);
    workflow_node(run, workflow_sessions, step_refs, state)
        .steps
        .into_iter()
        .find(|step| step.id == step_id)
}

fn step_refs_for_run(
    run_id: &str,
    workflow_sessions: &HashMap<String, WorkspaceSessionInput>,
    state: Option<&WorkflowStateSnapshot>,
) -> Vec<StepSessionRef> {
    let state_refs = state.map(collect_step_session_refs).unwrap_or_default();
    let mut refs = Vec::new();
    let mut explicit_session_ids = HashSet::new();

    for session in workflow_sessions.values() {
        let Some(context) = session.workflow_step_context.as_ref() else {
            continue;
        };
        if context.run_id != run_id {
            continue;
        }
        explicit_session_ids.insert(session.id.clone());
        refs.push(context_step_session_ref(
            session,
            context,
            state_ref_for_session(&state_refs, &session.id),
        ));
    }

    for state_ref in state_refs {
        let Some(session_id) = state_ref.session_id.as_ref() else {
            refs.push(state_ref);
            continue;
        };
        if explicit_session_ids.contains(session_id) {
            continue;
        }
        match workflow_sessions.get(session_id) {
            Some(session) if session.workflow_step_context.is_none() => refs.push(state_ref),
            Some(_) => {}
            None => refs.push(state_ref),
        }
    }

    retain_unique_step_refs(&mut refs);
    refs
}

fn context_step_session_ref(
    session: &WorkspaceSessionInput,
    context: &WorkflowStepContext,
    state_ref: Option<&StepSessionRef>,
) -> StepSessionRef {
    StepSessionRef {
        session_id: Some(session.id.clone()),
        step_name: context.step_name.clone(),
        run_index: Some(context.run_index),
        group_step_name: context.group_step_name().to_string(),
        group_run_index: Some(context.group_run_index()),
        state: state_ref
            .map(|step_ref| step_ref.state.clone())
            .unwrap_or_else(|| step_status_from_session_lifecycle(&session.state).to_string()),
        order: context.order as usize,
    }
}

fn state_ref_for_session<'a>(
    refs: &'a [StepSessionRef],
    session_id: &str,
) -> Option<&'a StepSessionRef> {
    refs.iter()
        .find(|step_ref| step_ref.session_id.as_deref() == Some(session_id))
}

fn step_status_from_session_lifecycle(state: &WorkspaceSessionState) -> &'static str {
    match state {
        WorkspaceSessionState::Active => STEP_STATE_RUNNING,
        WorkspaceSessionState::Error => STEP_STATE_FAILED,
        WorkspaceSessionState::Idle
        | WorkspaceSessionState::Done
        | WorkspaceSessionState::Closed
        | WorkspaceSessionState::Archived => STEP_STATE_COMPLETED,
    }
}

fn project_workspace_workflow_history(
    runs: Vec<WorkflowRunSummary>,
    archives: &[WorkflowRunManualArchiveRecord],
) -> Vec<WorkspaceWorkflowHistoryItemDto> {
    let mut history = runs
        .into_iter()
        .filter_map(|run| {
            let record = archives.iter().find(|record| record.run_id == run.run_id)?;
            Some(WorkspaceWorkflowHistoryItemDto {
                run_id: run.run_id.clone(),
                worktree_path: run.worktree_path.clone(),
                title: workflow_title(&run),
                status: run_status_label(run.status).to_string(),
                updated_at: run.updated_at,
                archived_at: record.archived_at,
                archive_reason: WORKFLOW_ARCHIVE_REASON_MANUAL.to_string(),
            })
        })
        .collect::<Vec<_>>();

    history.sort_by(|a, b| {
        b.archived_at
            .partial_cmp(&a.archived_at)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| compare_titles(&a.title, &b.title))
            .then_with(|| a.run_id.cmp(&b.run_id))
    });
    history
}

fn is_workflow_archived(run_id: &str, archives: &[WorkflowRunManualArchiveRecord]) -> bool {
    archives.iter().any(|record| record.run_id == run_id)
}

fn session_node(
    session: WorkspaceSessionInput,
    step_name: Option<String>,
    run_index: Option<u32>,
) -> WorkspaceSessionNodeDto {
    let title = step_name
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
        workflow_step_session: session.workflow_step_session,
        step_name,
        run_index,
    }
}

fn workflow_title(run: &WorkflowRunSummary) -> String {
    run.task
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let workflow_name = run.workflow_name.trim();
            if workflow_name.is_empty() {
                run.run_id.clone()
            } else {
                workflow_name.to_string()
            }
        })
}

fn collect_step_session_refs(state: &WorkflowStateSnapshot) -> Vec<StepSessionRef> {
    let mut refs = Vec::new();
    let mut next_order = 0usize;
    for entry in &state.step_history {
        let order = next_order;
        next_order += 1;
        refs.push(StepSessionRef {
            session_id: entry.session_id.clone(),
            step_name: entry.step_name.clone(),
            run_index: Some(entry.run_index),
            group_step_name: entry.step_name.clone(),
            group_run_index: Some(entry.run_index),
            state: normalize_step_status(&entry.state).to_string(),
            order,
        });
        if let Some(children) = entry.child_outputs.as_ref() {
            refs.extend(children.iter().map(|child| StepSessionRef {
                session_id: child.session_id.clone(),
                step_name: child.step_name.clone(),
                run_index: Some(child.run_index),
                group_step_name: entry.step_name.clone(),
                group_run_index: Some(entry.run_index),
                state: normalize_step_status(&child.state).to_string(),
                order,
            }));
        }
    }
    if state.current_session_id.is_some()
        || matches!(
            state.state,
            crate::domain::workflow::WorkflowExecutionState::Running
                | crate::domain::workflow::WorkflowExecutionState::WaitingApproval
        )
    {
        let order = next_order;
        let run_index = state
            .step_execution_counts
            .get(&state.current_step_name)
            .copied()
            .or(Some(1));
        refs.push(StepSessionRef {
            session_id: state.current_session_id.clone(),
            step_name: state.current_step_name.clone(),
            run_index,
            group_step_name: state.current_step_name.clone(),
            group_run_index: run_index,
            state: workflow_execution_state_label(&state.state).to_string(),
            order,
        });
        refs.extend(
            state
                .active_parallel_steps
                .iter()
                .map(|step| StepSessionRef {
                    session_id: step.session_id.clone(),
                    step_name: step.step_name.clone(),
                    run_index: Some(step.run_index),
                    group_step_name: state.current_step_name.clone(),
                    group_run_index: run_index,
                    state: normalize_step_status(&step.state).to_string(),
                    order,
                }),
        );
    }
    retain_unique_step_refs(&mut refs);
    refs
}

fn retain_unique_step_refs(refs: &mut Vec<StepSessionRef>) {
    let mut seen = HashSet::new();
    refs.retain(|step_ref| {
        seen.insert((
            step_ref.session_id.clone(),
            step_ref.step_name.clone(),
            step_ref.run_index,
            step_ref.group_step_name.clone(),
            step_ref.group_run_index,
        ))
    });
}

fn workflow_steps(
    run: &WorkflowRunSummary,
    workflow_sessions: &HashMap<String, WorkspaceSessionInput>,
    step_refs: &[StepSessionRef],
    state: Option<&WorkflowStateSnapshot>,
) -> Vec<WorkspaceWorkflowStepNodeDto> {
    let mut grouped_refs: BTreeMap<(String, Option<u32>), Vec<&StepSessionRef>> = BTreeMap::new();
    for step_ref in step_refs {
        grouped_refs
            .entry((step_ref.group_step_name.clone(), step_ref.group_run_index))
            .or_default()
            .push(step_ref);
    }

    let mut steps = grouped_refs
        .into_iter()
        .map(|((step_name, run_index), refs)| {
            let order = refs
                .iter()
                .map(|step_ref| step_ref.order)
                .min()
                .unwrap_or(usize::MAX);
            let mut sessions = refs
                .iter()
                .filter_map(|step_ref| {
                    step_ref
                        .session_id
                        .as_ref()
                        .and_then(|session_id| workflow_sessions.get(session_id))
                        .cloned()
                        .map(|session| {
                            session_node(
                                session,
                                Some(step_ref.step_name.clone()),
                                step_ref.run_index,
                            )
                        })
                })
                .collect::<Vec<_>>();
            sessions.sort_by(compare_session_nodes);
            let updated_at = sessions
                .iter()
                .map(|session| session.updated_at)
                .fold(run.updated_at, f64::max);
            (
                order,
                WorkspaceWorkflowStepNodeDto {
                    kind: "step",
                    id: workflow_step_id(&run.run_id, &step_name, run_index),
                    run_id: run.run_id.clone(),
                    worktree_path: run.worktree_path.clone(),
                    title: step_name.clone(),
                    status: step_status_for_group(&step_name, run_index, &refs, state).to_string(),
                    step_type: step_type_for_group(&step_name, state),
                    can_reject: step_can_reject(&step_name, run_index, state),
                    updated_at,
                    run_index,
                    sessions,
                },
            )
        })
        .collect::<Vec<_>>();
    steps.sort_by(|(a_order, a), (b_order, b)| {
        a_order
            .cmp(b_order)
            .then_with(|| a.run_index.cmp(&b.run_index))
            .then_with(|| a.id.cmp(&b.id))
    });
    steps.into_iter().map(|(_, step)| step).collect()
}

fn workflow_step_id(run_id: &str, step_name: &str, run_index: Option<u32>) -> String {
    format!("{run_id}:{step_name}:{}", run_index.unwrap_or(1))
}

fn step_status_for_group(
    step_name: &str,
    run_index: Option<u32>,
    refs: &[&StepSessionRef],
    state: Option<&WorkflowStateSnapshot>,
) -> &'static str {
    if let Some(state) = state {
        if state.current_step_name == step_name {
            if current_run_index(state) == run_index {
                return workflow_execution_state_label(&state.state);
            }
        } else if let Some(step_state) = state.step_states.get(step_name) {
            return normalize_step_status(step_state);
        }
    }
    if refs
        .iter()
        .any(|step_ref| step_ref.state == STEP_STATE_FAILED)
    {
        return STEP_STATE_FAILED;
    }
    if refs
        .iter()
        .any(|step_ref| step_ref.state == STEP_STATE_ABORTED)
    {
        return STEP_STATE_ABORTED;
    }
    if refs
        .iter()
        .any(|step_ref| step_ref.state == STEP_STATE_WAITING_APPROVAL)
    {
        return STEP_STATE_WAITING_APPROVAL;
    }
    if refs
        .iter()
        .any(|step_ref| step_ref.state == STEP_STATE_RUNNING)
    {
        return STEP_STATE_RUNNING;
    }
    if !refs.is_empty() {
        return STEP_STATE_COMPLETED;
    }
    "queued"
}

fn step_type_for_group(step_name: &str, state: Option<&WorkflowStateSnapshot>) -> &'static str {
    state
        .and_then(|state| {
            state
                .workflow_definition
                .nodes
                .iter()
                .find(|node| node.name == step_name)
        })
        .map(|node| node_type_label(node.node_type))
        .unwrap_or("agent")
}

fn node_type_label(node_type: NodeType) -> &'static str {
    match node_type {
        NodeType::Agent => "agent",
        NodeType::Bash => "bash",
        NodeType::Approval => "approval",
        NodeType::Parallel => "parallel",
    }
}

fn step_can_reject(
    step_name: &str,
    run_index: Option<u32>,
    state: Option<&WorkflowStateSnapshot>,
) -> Option<bool> {
    let state = state?;
    if state.current_step_name != step_name {
        return None;
    }
    if current_run_index(state) != run_index {
        return None;
    }
    if !matches!(
        state.state,
        crate::domain::workflow::WorkflowExecutionState::WaitingApproval
    ) {
        return None;
    }
    Some(
        state
            .approval_operations
            .as_ref()
            .is_some_and(|operations| operations.can_reject),
    )
}

fn current_run_index(state: &WorkflowStateSnapshot) -> Option<u32> {
    state
        .step_execution_counts
        .get(&state.current_step_name)
        .copied()
        .or(Some(1))
}

fn workflow_execution_state_label(
    state: &crate::domain::workflow::WorkflowExecutionState,
) -> &'static str {
    match state {
        crate::domain::workflow::WorkflowExecutionState::Running => STEP_STATE_RUNNING,
        crate::domain::workflow::WorkflowExecutionState::WaitingApproval => {
            STEP_STATE_WAITING_APPROVAL
        }
        crate::domain::workflow::WorkflowExecutionState::Completed => STEP_STATE_COMPLETED,
        crate::domain::workflow::WorkflowExecutionState::Failed { .. } => STEP_STATE_FAILED,
        crate::domain::workflow::WorkflowExecutionState::Aborted => STEP_STATE_ABORTED,
    }
}

fn normalize_step_status(status: &str) -> &'static str {
    match status {
        STEP_STATE_RUNNING => STEP_STATE_RUNNING,
        STEP_STATE_WAITING_APPROVAL => STEP_STATE_WAITING_APPROVAL,
        STEP_STATE_COMPLETED => STEP_STATE_COMPLETED,
        STEP_STATE_FAILED => STEP_STATE_FAILED,
        STEP_STATE_ABORTED => STEP_STATE_ABORTED,
        STEP_STATE_PENDING => "queued",
        _ => "queued",
    }
}

fn compare_session_nodes(
    a: &WorkspaceSessionNodeDto,
    b: &WorkspaceSessionNodeDto,
) -> std::cmp::Ordering {
    compare_titles(&a.title, &b.title).then_with(|| a.id.cmp(&b.id))
}

fn compare_titles(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

fn run_status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => STEP_STATE_RUNNING,
        RunStatus::WaitingApproval => "waiting_approval",
        RunStatus::Completed => "completed",
        RunStatus::Failed => "failed",
        RunStatus::Aborted => "aborted",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{
        ApprovalOperations, ChildOutputSnapshot, NodeDefinition, NodeType, ParallelStepState,
        StepHistoryEntry, TriggerSource, WorkflowDefinition, WorkflowExecutionState,
        STEP_STATE_ABORTED, STEP_STATE_COMPLETED, STEP_STATE_FAILED, STEP_STATE_RUNNING,
        STEP_STATE_WAITING_APPROVAL,
    };

    fn session(id: &str, title: &str, workflow_step_session: bool) -> WorkspaceSessionInput {
        WorkspaceSessionInput {
            id: id.to_string(),
            worktree_path: "/repo/wt".to_string(),
            state: WorkspaceSessionState::Active,
            updated_at: 2.0,
            first_message: title.to_string(),
            workflow_step_session,
            workflow_step_context: None,
        }
    }

    fn context(
        run_id: &str,
        step_name: &str,
        run_index: u32,
        parent: Option<(&str, u32)>,
        order: u32,
    ) -> WorkflowStepContext {
        WorkflowStepContext {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            step_name: step_name.to_string(),
            run_index,
            parent_step_name: parent.map(|(name, _)| name.to_string()),
            parent_run_index: parent.map(|(_, run_index)| run_index),
            order,
        }
    }

    fn session_with_context(
        id: &str,
        title: &str,
        context: WorkflowStepContext,
    ) -> WorkspaceSessionInput {
        let mut session = session(id, title, true);
        session.workflow_step_context = Some(context);
        session
    }

    fn run(run_id: &str, task: &str) -> WorkflowRunSummary {
        run_with_status(run_id, task, RunStatus::Running)
    }

    fn run_with_status(run_id: &str, task: &str, status: RunStatus) -> WorkflowRunSummary {
        WorkflowRunSummary {
            run_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            task: Some(task.to_string()),
            status,
            worktree_path: "/repo/wt".to_string(),
            current_node_name: Some("build".to_string()),
            trigger_source: TriggerSource::DesktopUi,
            started_at: 1.0,
            updated_at: 3.0,
            completed_at: None,
            error_reason: None,
        }
    }

    fn manual_archive_record(run_id: &str, archived_at: f64) -> WorkflowRunManualArchiveRecord {
        WorkflowRunManualArchiveRecord {
            run_id: run_id.to_string(),
            archived_at,
        }
    }

    fn state(run_id: &str) -> WorkflowStateSnapshot {
        WorkflowStateSnapshot {
            execution_id: run_id.to_string(),
            workflow_name: "wf".to_string(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            current_step_name: "build".to_string(),
            current_session_id: Some("step-build".to_string()),
            total_steps: 2,
            step_history: vec![crate::domain::workflow::StepHistoryEntry {
                step_name: "plan".to_string(),
                completed_at: 1.0,
                result: None,
                session_id: Some("step-plan".to_string()),
                token_usage: None,
                structured_output: None,
                run_index: 1,
                child_outputs: Some(vec![ChildOutputSnapshot {
                    step_name: "child-review".to_string(),
                    session_id: Some("step-child".to_string()),
                    result: None,
                    run_index: 2,
                    completed_at: 2.0,
                    structured_output: None,
                    output_contract: None,
                    state: STEP_STATE_COMPLETED.to_string(),
                }]),
                state: STEP_STATE_COMPLETED.to_string(),
            }],
            step_execution_counts: HashMap::from([("build".to_string(), 3)]),
            workflow_definition: WorkflowDefinition {
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                variables: HashMap::new(),
                nodes: Vec::new(),
            },
            total_token_usage: Default::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![ParallelStepState {
                step_name: "parallel-lint".to_string(),
                state: STEP_STATE_RUNNING.to_string(),
                session_id: Some("step-parallel".to_string()),
                result: None,
                run_index: 1,
                completed_at: None,
                structured_output: None,
                output_contract: None,
            }],
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 1.0,
            updated_at: 2.0,
        }
    }

    #[test]
    fn projects_direct_sessions_before_workflows_and_sorts_by_title() {
        let archives = Vec::new();
        let nodes = project_workspace_tree_nodes(
            vec![
                session("b", "Zulu", false),
                session("a", "Alpha", false),
                session("step-build", "ignored", true),
            ],
            vec![run("run-1", "Implement")],
            HashMap::from([("run-1".to_string(), Some(state("run-1")))]),
            &archives,
        );

        assert!(matches!(&nodes[0], WorkspaceTreeNodeDto::Session(node) if node.title == "Alpha"));
        assert!(matches!(&nodes[1], WorkspaceTreeNodeDto::Session(node) if node.title == "Zulu"));
        assert!(
            matches!(&nodes[2], WorkspaceTreeNodeDto::Workflow(node) if node.title == "Implement" && node.workflow_name == "wf")
        );
    }

    #[test]
    fn empty_direct_session_uses_default_new_session_title() {
        let archives = Vec::new();
        let nodes = project_workspace_tree_nodes(
            vec![session("empty", "", false)],
            vec![],
            HashMap::new(),
            &archives,
        );

        assert!(
            matches!(&nodes[0], WorkspaceTreeNodeDto::Session(node) if node.title == DEFAULT_SESSION_TITLE)
        );
    }

    #[test]
    fn maps_workflow_step_sessions_to_their_parent_run_with_step_titles() {
        let archives = Vec::new();
        let nodes = project_workspace_tree_nodes(
            vec![
                session("step-plan", "old plan title", true),
                session("step-build", "old build title", true),
                session("step-child", "old child title", true),
                session("step-parallel", "old parallel title", true),
                session("unmatched-step", "orphan", true),
            ],
            vec![run("run-1", "Implement")],
            HashMap::from([("run-1".to_string(), Some(state("run-1")))]),
            &archives,
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow node");
        };
        assert_eq!(
            workflow
                .steps
                .iter()
                .map(|step| step.title.as_str())
                .collect::<Vec<_>>(),
            vec!["plan", "build"]
        );
        let sessions = workflow
            .steps
            .iter()
            .flat_map(|step| step.sessions.iter())
            .collect::<Vec<_>>();
        let mut titles = sessions
            .iter()
            .map(|session| session.title.as_str())
            .collect::<Vec<_>>();
        titles.sort();
        assert_eq!(
            titles,
            vec!["build", "child-review", "parallel-lint", "plan"]
        );
        assert!(sessions
            .iter()
            .any(|session| session.id == "step-build" && session.run_index == Some(3)));
        assert!(!sessions
            .iter()
            .any(|session| session.id == "unmatched-step"));
    }

    #[test]
    fn explicit_workflow_step_context_is_primary_parentage() {
        let archives = Vec::new();
        let mut snapshot = state("run-1");
        snapshot.current_step_name = "wrong-state-step".to_string();
        snapshot.current_session_id = Some("ctx-plan".to_string());
        snapshot.step_history = Vec::new();
        snapshot.active_parallel_steps = Vec::new();

        let nodes = project_workspace_tree_nodes(
            vec![session_with_context(
                "ctx-plan",
                "old title",
                context("run-1", "plan", 1, None, 0),
            )],
            vec![run("run-1", "Implement")],
            HashMap::from([("run-1".to_string(), Some(snapshot))]),
            &archives,
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow node");
        };
        assert_eq!(workflow.steps.len(), 1);
        assert_eq!(workflow.steps[0].id, "run-1:plan:1");
        assert_eq!(workflow.steps[0].title, "plan");
        assert_eq!(workflow.steps[0].sessions[0].id, "ctx-plan");
        assert_eq!(workflow.steps[0].sessions[0].title, "plan");
    }

    #[test]
    fn explicit_parallel_child_context_groups_sessions_under_parent_step() {
        let archives = Vec::new();
        let mut snapshot = state("run-1");
        snapshot.state = WorkflowExecutionState::Completed;
        snapshot.current_session_id = None;
        snapshot.step_history = Vec::new();
        snapshot.active_parallel_steps = Vec::new();
        snapshot.workflow_definition.nodes = vec![NodeDefinition {
            name: "parallel-review".to_string(),
            node_type: NodeType::Parallel,
            ..Default::default()
        }];

        let nodes = project_workspace_tree_nodes(
            vec![
                session_with_context(
                    "child-a",
                    "old a",
                    context("run-1", "review-opus", 1, Some(("parallel-review", 2)), 4),
                ),
                session_with_context(
                    "child-b",
                    "old b",
                    context("run-1", "review-gpt55", 1, Some(("parallel-review", 2)), 4),
                ),
            ],
            vec![run("run-1", "Implement")],
            HashMap::from([("run-1".to_string(), Some(snapshot))]),
            &archives,
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow node");
        };
        assert_eq!(workflow.steps.len(), 1);
        let step = &workflow.steps[0];
        assert_eq!(step.id, "run-1:parallel-review:2");
        assert_eq!(step.title, "parallel-review");
        assert_eq!(step.run_index, Some(2));
        assert_eq!(step.step_type, "parallel");
        assert_eq!(
            step.sessions
                .iter()
                .map(|session| (session.id.as_str(), session.title.as_str()))
                .collect::<Vec<_>>(),
            vec![("child-b", "review-gpt55"), ("child-a", "review-opus")]
        );
    }

    #[test]
    fn explicit_context_distinguishes_repeated_step_runs_with_same_name() {
        let archives = Vec::new();
        let nodes = project_workspace_tree_nodes(
            vec![
                session_with_context(
                    "review-1",
                    "old review 1",
                    context("run-1", "review", 1, None, 0),
                ),
                session_with_context(
                    "review-2",
                    "old review 2",
                    context("run-1", "review", 2, None, 1),
                ),
            ],
            vec![run("run-1", "Implement")],
            HashMap::new(),
            &archives,
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow node");
        };
        assert_eq!(
            workflow
                .steps
                .iter()
                .map(|step| (step.id.as_str(), step.title.as_str(), step.run_index))
                .collect::<Vec<_>>(),
            vec![
                ("run-1:review:1", "review", Some(1)),
                ("run-1:review:2", "review", Some(2)),
            ]
        );
    }

    #[test]
    fn explicit_context_does_not_drop_missing_session_state_step_refs() {
        let archives = Vec::new();
        let mut snapshot = state("run-1");
        snapshot.state = WorkflowExecutionState::Completed;
        snapshot.current_session_id = None;
        snapshot.step_history = vec![crate::domain::workflow::StepHistoryEntry {
            step_name: "legacy-sessionless".to_string(),
            completed_at: 1.0,
            result: None,
            session_id: Some("missing-legacy-session".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: None,
            state: STEP_STATE_COMPLETED.to_string(),
        }];
        snapshot.active_parallel_steps = Vec::new();

        let nodes = project_workspace_tree_nodes(
            vec![session_with_context(
                "ctx-review",
                "old review",
                context("run-1", "review", 1, None, 0),
            )],
            vec![run("run-1", "Implement")],
            HashMap::from([("run-1".to_string(), Some(snapshot))]),
            &archives,
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow node");
        };
        assert!(workflow
            .steps
            .iter()
            .any(|step| step.title == "review" && step.sessions.len() == 1));
        assert!(workflow
            .steps
            .iter()
            .any(|step| step.title == "legacy-sessionless" && step.sessions.is_empty()));
    }

    #[test]
    fn workflow_steps_keep_closed_step_sessions_in_their_parent_run() {
        let archives = Vec::new();
        let mut closed_step_session = session("step-build", "old build title", true);
        closed_step_session.state = WorkspaceSessionState::Closed;
        let nodes = project_workspace_tree_nodes(
            vec![closed_step_session],
            vec![run("run-1", "Implement")],
            HashMap::from([("run-1".to_string(), Some(state("run-1")))]),
            &archives,
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow node");
        };
        let build = workflow
            .steps
            .iter()
            .find(|step| step.title == "build")
            .expect("build step");

        assert_eq!(build.sessions.len(), 1);
        assert_eq!(build.sessions[0].id, "step-build");
        assert_eq!(build.sessions[0].state, WorkspaceSessionState::Closed);
    }

    #[test]
    fn closed_explicit_step_session_stays_attached_to_parent_step() {
        let archives = Vec::new();
        let mut closed_step_session = session_with_context(
            "closed-review",
            "old review title",
            context("run-1", "review", 1, None, 0),
        );
        closed_step_session.state = WorkspaceSessionState::Closed;

        let nodes = project_workspace_tree_nodes(
            vec![closed_step_session],
            vec![run("run-1", "Implement")],
            HashMap::new(),
            &archives,
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow node");
        };
        assert_eq!(workflow.steps.len(), 1);
        assert_eq!(workflow.steps[0].title, "review");
        assert_eq!(workflow.steps[0].sessions[0].id, "closed-review");
        assert_eq!(
            workflow.steps[0].sessions[0].state,
            WorkspaceSessionState::Closed
        );
    }

    #[test]
    fn explicit_context_without_state_ref_uses_session_lifecycle_for_step_status() {
        for (session_state, expected_status) in [
            (WorkspaceSessionState::Error, STEP_STATE_FAILED),
            (WorkspaceSessionState::Active, STEP_STATE_RUNNING),
            (WorkspaceSessionState::Closed, STEP_STATE_COMPLETED),
        ] {
            let archives = Vec::new();
            let mut step_session = session_with_context(
                expected_status,
                "old review title",
                context("run-1", "review", 1, None, 0),
            );
            step_session.state = session_state;

            let nodes = project_workspace_tree_nodes(
                vec![step_session],
                vec![run("run-1", "Implement")],
                HashMap::new(),
                &archives,
            );

            let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
                panic!("expected workflow node");
            };
            assert_eq!(workflow.steps.len(), 1);
            assert_eq!(workflow.steps[0].status, expected_status);
        }
    }

    #[test]
    fn step_detail_uses_explicit_context_without_state_session_ref() {
        let session = session_with_context(
            "detail-review",
            "old detail title",
            context("run-1", "review", 7, None, 3),
        );
        let workflow_sessions = HashMap::from([(session.id.clone(), session)]);

        let detail = workflow_step_detail_node(
            run("run-1", "Implement"),
            &workflow_sessions,
            None,
            "run-1:review:7",
        )
        .expect("step detail should come from explicit context");

        assert_eq!(detail.title, "review");
        assert_eq!(detail.run_index, Some(7));
        assert_eq!(detail.sessions.len(), 1);
        assert_eq!(detail.sessions[0].id, "detail-review");
    }

    #[test]
    fn workflow_steps_group_sessions_and_scope_current_status_to_run_index() {
        let mut snapshot = state("run-1");
        snapshot.current_step_name = "review".to_string();
        snapshot.current_session_id = Some("review-2".to_string());
        snapshot.state = WorkflowExecutionState::WaitingApproval;
        snapshot.step_execution_counts = HashMap::from([("review".to_string(), 2)]);
        snapshot.step_states = HashMap::from([(
            "review".to_string(),
            STEP_STATE_WAITING_APPROVAL.to_string(),
        )]);
        snapshot.approval_operations = Some(ApprovalOperations { can_reject: true });
        snapshot.workflow_definition.nodes = vec![NodeDefinition {
            name: "review".to_string(),
            node_type: NodeType::Approval,
            ..Default::default()
        }];
        snapshot.step_history = vec![crate::domain::workflow::StepHistoryEntry {
            step_name: "review".to_string(),
            completed_at: 1.0,
            result: None,
            session_id: Some("review-1".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: None,
            state: STEP_STATE_COMPLETED.to_string(),
        }];
        snapshot.active_parallel_steps = Vec::new();

        let archives = Vec::new();
        let nodes = project_workspace_tree_nodes(
            vec![
                session("review-1", "old review", true),
                session("review-2", "current review", true),
            ],
            vec![run("run-1", "Implement")],
            HashMap::from([("run-1".to_string(), Some(snapshot))]),
            &archives,
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow node");
        };
        assert_eq!(
            workflow
                .steps
                .iter()
                .map(|step| (step.title.as_str(), step.run_index))
                .collect::<Vec<_>>(),
            vec![("review", Some(1)), ("review", Some(2))]
        );
        let previous = workflow
            .steps
            .iter()
            .find(|step| step.run_index == Some(1))
            .expect("previous step");
        let current = workflow
            .steps
            .iter()
            .find(|step| step.run_index == Some(2))
            .expect("current step");

        assert_eq!(previous.status, STEP_STATE_COMPLETED);
        assert_eq!(previous.step_type, "approval");
        assert_eq!(previous.can_reject, None);
        assert_eq!(previous.sessions[0].id, "review-1");
        assert_eq!(current.status, STEP_STATE_WAITING_APPROVAL);
        assert_eq!(current.step_type, "approval");
        assert_eq!(current.can_reject, Some(true));
        assert_eq!(current.sessions[0].id, "review-2");
    }

    #[test]
    fn workflow_steps_include_sessionless_active_and_history_steps_only() {
        let mut snapshot = state("run-1");
        snapshot.current_step_name = "script".to_string();
        snapshot.current_session_id = None;
        snapshot.step_execution_counts = HashMap::from([("script".to_string(), 1)]);
        snapshot.step_history = vec![crate::domain::workflow::StepHistoryEntry {
            step_name: "lint".to_string(),
            completed_at: 1.0,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: None,
            state: STEP_STATE_COMPLETED.to_string(),
        }];
        snapshot.active_parallel_steps = Vec::new();
        snapshot.workflow_definition.nodes = vec![
            NodeDefinition {
                name: "script".to_string(),
                node_type: NodeType::Bash,
                ..Default::default()
            },
            NodeDefinition {
                name: "lint".to_string(),
                node_type: NodeType::Bash,
                ..Default::default()
            },
            NodeDefinition {
                name: "parallel-review".to_string(),
                node_type: NodeType::Parallel,
                ..Default::default()
            },
        ];

        let archives = Vec::new();
        let nodes = project_workspace_tree_nodes(
            vec![],
            vec![run("run-1", "Implement")],
            HashMap::from([("run-1".to_string(), Some(snapshot))]),
            &archives,
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow node");
        };
        assert!(workflow.steps.iter().any(|step| {
            step.title == "script"
                && step.status == STEP_STATE_RUNNING
                && step.step_type == "bash"
                && step.sessions.is_empty()
        }));
        assert!(workflow.steps.iter().any(|step| {
            step.title == "lint"
                && step.status == STEP_STATE_COMPLETED
                && step.step_type == "bash"
                && step.sessions.is_empty()
        }));
        assert!(!workflow
            .steps
            .iter()
            .any(|step| step.title == "parallel-review"));
    }

    fn projected_step_status_for_ref_states(states: &[&str]) -> String {
        let mut snapshot = state("run-1");
        snapshot.state = WorkflowExecutionState::Completed;
        snapshot.current_step_name = "other".to_string();
        snapshot.current_session_id = None;
        snapshot.step_execution_counts = HashMap::new();
        snapshot.step_states = HashMap::new();
        snapshot.active_parallel_steps = Vec::new();
        snapshot.step_history = vec![StepHistoryEntry {
            step_name: "priority".to_string(),
            completed_at: 1.0,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: Some(
                states
                    .iter()
                    .enumerate()
                    .map(|(index, state)| ChildOutputSnapshot {
                        step_name: format!("child-{index}"),
                        session_id: None,
                        result: None,
                        run_index: index as u32 + 1,
                        completed_at: index as f64 + 2.0,
                        structured_output: None,
                        output_contract: None,
                        state: (*state).to_string(),
                    })
                    .collect(),
            ),
            state: STEP_STATE_COMPLETED.to_string(),
        }];

        let archives = Vec::new();
        let nodes = project_workspace_tree_nodes(
            vec![],
            vec![run("run-1", "Implement")],
            HashMap::from([("run-1".to_string(), Some(snapshot))]),
            &archives,
        );

        let WorkspaceTreeNodeDto::Workflow(workflow) = &nodes[0] else {
            panic!("expected workflow node");
        };
        workflow
            .steps
            .iter()
            .find(|step| step.title == "priority")
            .expect("priority step")
            .status
            .clone()
    }

    #[test]
    fn workflow_step_status_uses_ref_state_priority_for_mixed_refs() {
        for (states, expected_status) in [
            (
                vec![
                    STEP_STATE_FAILED,
                    STEP_STATE_ABORTED,
                    STEP_STATE_WAITING_APPROVAL,
                    STEP_STATE_RUNNING,
                ],
                STEP_STATE_FAILED,
            ),
            (
                vec![
                    STEP_STATE_ABORTED,
                    STEP_STATE_WAITING_APPROVAL,
                    STEP_STATE_RUNNING,
                ],
                STEP_STATE_ABORTED,
            ),
            (
                vec![STEP_STATE_WAITING_APPROVAL, STEP_STATE_RUNNING],
                STEP_STATE_WAITING_APPROVAL,
            ),
            (vec![STEP_STATE_RUNNING], STEP_STATE_RUNNING),
        ] {
            assert_eq!(
                projected_step_status_for_ref_states(&states),
                expected_status
            );
        }
    }

    #[test]
    fn terminal_workflow_without_sessions_stays_visible() {
        let archives = Vec::new();
        let nodes = project_workspace_tree_nodes(
            vec![],
            vec![run_with_status(
                "run-1",
                "Done without sessions",
                RunStatus::Completed,
            )],
            HashMap::new(),
            &archives,
        );

        assert!(
            matches!(&nodes[0], WorkspaceTreeNodeDto::Workflow(node) if node.run_id == "run-1")
        );
        assert!(archives.is_empty());
    }

    #[test]
    fn restored_terminal_workflow_without_sessions_stays_visible() {
        let archives = Vec::new();
        let nodes = project_workspace_tree_nodes(
            vec![],
            vec![run_with_status("run-1", "Restored", RunStatus::Completed)],
            HashMap::new(),
            &archives,
        );

        assert!(
            matches!(&nodes[0], WorkspaceTreeNodeDto::Workflow(node) if node.run_id == "run-1")
        );
    }

    #[test]
    fn manual_archive_hides_workflow_until_restored() {
        let archives = vec![manual_archive_record("run-1", 15.0)];
        let nodes = project_workspace_tree_nodes(
            vec![session("step-build", "old build title", true)],
            vec![run("run-1", "Manual")],
            HashMap::from([("run-1".to_string(), Some(state("run-1")))]),
            &archives,
        );

        assert!(nodes.is_empty());
        let history = project_workspace_workflow_history(vec![run("run-1", "Manual")], &archives);
        assert_eq!(history[0].run_id, "run-1");
        assert_eq!(history[0].archive_reason, WORKFLOW_ARCHIVE_REASON_MANUAL);
    }

    #[test]
    fn normalize_step_status_keeps_known_and_unknown_mappings() {
        assert_eq!(
            normalize_step_status(STEP_STATE_RUNNING),
            STEP_STATE_RUNNING
        );
        assert_eq!(
            normalize_step_status(STEP_STATE_WAITING_APPROVAL),
            STEP_STATE_WAITING_APPROVAL
        );
        assert_eq!(
            normalize_step_status(STEP_STATE_COMPLETED),
            STEP_STATE_COMPLETED
        );
        assert_eq!(normalize_step_status(STEP_STATE_FAILED), STEP_STATE_FAILED);
        assert_eq!(
            normalize_step_status(STEP_STATE_ABORTED),
            STEP_STATE_ABORTED
        );
        assert_eq!(normalize_step_status(STEP_STATE_PENDING), "queued");
        assert_eq!(normalize_step_status("queued"), "queued");
        assert_eq!(normalize_step_status("unexpected"), "queued");
    }
}
