use crate::domain::path::to_canonical_forward_slash;
use crate::domain::workflow::services::session_projection::StepSessionProjection;
use crate::domain::workflow::status_aggregation::{
    aggregate_representative_statuses, session_result, RepresentativeStatus, SessionActivity,
    StepProgress,
};
use crate::usecase::agent_session::session::SessionState;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Running,
    Done,
    Error,
    Waiting,
}

impl From<AgentState> for SessionActivity {
    fn from(value: AgentState) -> Self {
        match value {
            AgentState::Running => Self::Running,
            AgentState::Done => Self::Done,
            AgentState::Error => Self::Error,
            AgentState::Waiting => Self::Waiting,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnPhase {
    Idle,
    Streaming,
    WaitingPermission,
}

/// 1 つの ChatSession に対する状態スナップショット。
/// AgentState は turn_phase / session_state から算出した派生値。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionStatus {
    pub chat_session_id: String,
    pub worktree_id: String,
    pub worktree_path: String,
    pub pty_id: Option<String>,
    pub agent_state: AgentState,
    pub turn_phase: TurnPhaseRepr,
    pub session_state: SessionState,
    pub pending_permission: bool,
    pub pending_permission_request: Option<serde_json::Value>,
    pub last_activity_at: f64,
    pub workflow_step: Option<String>,
    pub workflow_execution_state: Option<String>,
    #[serde(skip_serializing)]
    pub workflow_execution_id: Option<String>,
    #[serde(skip_serializing)]
    pub workflow_run_index: Option<u32>,
    #[serde(skip_serializing)]
    pub workflow_step_progress: Option<StepProgress>,
}

/// `TurnPhase` は `agent_sdk` 側で `Copy + Serialize` だが `Eq` を持たないため、
/// SessionStatus のフィールド比較用に同型の列挙を持つ。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnPhaseRepr {
    Idle,
    Streaming,
    WaitingPermission,
}

impl From<TurnPhase> for TurnPhaseRepr {
    fn from(value: TurnPhase) -> Self {
        match value {
            TurnPhase::Idle => Self::Idle,
            TurnPhase::Streaming => Self::Streaming,
            TurnPhase::WaitingPermission => Self::WaitingPermission,
        }
    }
}

impl From<TurnPhaseRepr> for TurnPhase {
    fn from(value: TurnPhaseRepr) -> Self {
        match value {
            TurnPhaseRepr::Idle => Self::Idle,
            TurnPhaseRepr::Streaming => Self::Streaming,
            TurnPhaseRepr::WaitingPermission => Self::WaitingPermission,
        }
    }
}

/// Workspace 全体の集約状態。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkspaceStatus {
    pub worktree_id: String,
    pub worktree_path: String,
    pub aggregated_state: AgentState,
    pub running_count: usize,
    pub waiting_count: usize,
    pub error_count: usize,
    pub session_count: usize,
    pub last_activity_at: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WorkflowStepKey {
    worktree_path: String,
    execution_id: String,
    step_name: String,
    run_index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepRepresentative {
    pub execution_id: String,
    pub step_name: String,
    pub run_index: Option<u32>,
    pub representative: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRepresentative {
    pub execution_id: String,
    pub representative: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeStepStatusView {
    pub worktree_path: String,
    pub version: u64,
    pub steps: Vec<WorkflowStepRepresentative>,
    pub workflows: Vec<WorkflowRepresentative>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkflowStatusEntry {
    representative: RepresentativeStatus,
}

#[derive(Debug, Default)]
struct WorkflowStepStatusState {
    steps: HashMap<WorkflowStepKey, WorkflowStatusEntry>,
    baselines: HashMap<WorkflowStepKey, RepresentativeStatus>,
    views: HashMap<String, WorktreeStepStatusView>,
}

#[derive(Debug, Default)]
struct WorkflowStepStatusUpdate {
    views: Vec<WorktreeStepStatusView>,
}

impl WorkflowStepStatusUpdate {
    fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkflowStepSessionStatusInput {
    step_name: String,
    run_index: Option<u32>,
    progress: StepProgress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingWorkflowStepSessionStatus {
    worktree_path: String,
    execution_id: String,
    workflow_execution_state: String,
    input: WorkflowStepSessionStatusInput,
}

/// 各 worktree の「アクティブな Workflow 実行」の集約用スナップショット。
/// Workspace 集約に Workflow 自体の状態（WaitingApproval 等）を反映するために保持する。
/// 現状 1 worktree = 1 active run 前提のためフラット HashMap で管理する。
#[derive(Debug, Clone, PartialEq)]
struct WorkflowAggSnapshot {
    execution_id: String,
    agent_state: Option<AgentState>,
    last_activity_at: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentStatusChanges {
    pub session: Option<SessionStatus>,
    pub workspace: Option<WorkspaceStatus>,
    pub agent_state: Option<AgentStateChange>,
    pub workflow_step_views: Vec<WorktreeStepStatusView>,
}

impl AgentStatusChanges {
    pub fn is_empty(&self) -> bool {
        self.session.is_none()
            && self.workspace.is_none()
            && self.agent_state.is_none()
            && self.workflow_step_views.is_empty()
    }
}

pub trait AgentStatusNotifier: Send + Sync {
    fn status_changed(&self, changes: AgentStatusChanges);
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentStateChange {
    pub worktree_path: String,
    pub state: AgentState,
    pub timestamp: f64,
    pub session_id: Option<String>,
    pub pty_id: Option<String>,
}

/// Session / Workspace の状態を集中管理し、フロント・WS にブロードキャストする中央管理。
pub struct AgentStatusCenter {
    sessions: RwLock<HashMap<String, SessionStatus>>,
    workspaces: RwLock<HashMap<String, WorkspaceStatus>>,
    workflow_step_status: RwLock<WorkflowStepStatusState>,
    pending_workflow_step_sessions: RwLock<HashMap<String, PendingWorkflowStepSessionStatus>>,
    workflow_status_version: AtomicU64,
    /// key: worktree_path（= worktree_id と同値で運用）
    workflows: RwLock<HashMap<String, WorkflowAggSnapshot>>,
}

impl AgentStatusCenter {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            workspaces: RwLock::new(HashMap::new()),
            workflow_step_status: RwLock::new(WorkflowStepStatusState::default()),
            pending_workflow_step_sessions: RwLock::new(HashMap::new()),
            workflow_status_version: AtomicU64::new(0),
            workflows: RwLock::new(HashMap::new()),
        }
    }

    fn canonical_worktree_path(path: &str) -> String {
        to_canonical_forward_slash(path)
    }

    fn normalize_session_paths(status: &mut SessionStatus) {
        status.worktree_id = Self::canonical_worktree_path(&status.worktree_id);
        status.worktree_path = Self::canonical_worktree_path(&status.worktree_path);
    }

    /// `WorkflowExecutionState` を Workspace 集約に寄与する `AgentState` にマップする。
    /// `Aborted` はユーザー意図で停止したものとして「集約対象外（None）」扱いにする。
    pub fn workflow_execution_state_to_agent_state(
        state: &crate::domain::workflow::WorkflowExecutionState,
    ) -> Option<AgentState> {
        use crate::domain::workflow::WorkflowExecutionState;
        match state {
            WorkflowExecutionState::Running => Some(AgentState::Running),
            WorkflowExecutionState::WaitingApproval => Some(AgentState::Waiting),
            WorkflowExecutionState::Failed { .. } => Some(AgentState::Error),
            WorkflowExecutionState::Completed => Some(AgentState::Done),
            WorkflowExecutionState::Aborted => None,
        }
    }

    /// AgentState は turn_phase / session_state から派生する。
    /// Spec line 110 の集約規則対応 (Error > Idle > Active > Done) に揃え、
    /// turn_phase が Idle の場合は session_state=Error → Error、Idle → Waiting、
    /// それ以外 → Done として個別 Session の agent_state を決定する。
    pub fn derive_agent_state(turn_phase: TurnPhase, session_state: SessionState) -> AgentState {
        match turn_phase {
            TurnPhase::Streaming => AgentState::Running,
            TurnPhase::WaitingPermission => AgentState::Waiting,
            TurnPhase::Idle => match session_state {
                SessionState::Error => AgentState::Error,
                SessionState::Idle => AgentState::Waiting,
                _ => AgentState::Done,
            },
        }
    }

    /// dedup 用: タイムスタンプを除いた状態フィールドのみで等価判定。
    fn is_session_state_equivalent(a: &SessionStatus, b: &SessionStatus) -> bool {
        a.chat_session_id == b.chat_session_id
            && a.worktree_id == b.worktree_id
            && a.worktree_path == b.worktree_path
            && a.pty_id == b.pty_id
            && a.agent_state == b.agent_state
            && a.turn_phase == b.turn_phase
            && a.session_state == b.session_state
            && a.pending_permission == b.pending_permission
            && a.pending_permission_request == b.pending_permission_request
            && a.workflow_step == b.workflow_step
            && a.workflow_execution_state == b.workflow_execution_state
            && a.workflow_execution_id == b.workflow_execution_id
            && a.workflow_run_index == b.workflow_run_index
            && a.workflow_step_progress == b.workflow_step_progress
    }

    fn is_workspace_state_equivalent(a: &WorkspaceStatus, b: &WorkspaceStatus) -> bool {
        a.worktree_id == b.worktree_id
            && a.worktree_path == b.worktree_path
            && a.aggregated_state == b.aggregated_state
            && a.running_count == b.running_count
            && a.waiting_count == b.waiting_count
            && a.error_count == b.error_count
            && a.session_count == b.session_count
    }

    /// Workspace の集約規則: Running > Error > Waiting > Done。
    /// Closed/Archived Session はオープン中でないため集約対象から除外する。
    /// 全てフィルタされて 0 件になった場合は `aggregated_state: Done` / `session_count: 0` で表現する
    /// （= 集約対象なし）。
    ///
    /// `workflow_state` は、当該 worktree でアクティブな Workflow の状態
    /// （`WorkflowExecutionState` を `AgentState` にマップしたもの）。
    /// 集約優先度ロジックには寄与するが、`session_count` には含めない。
    fn aggregate(
        worktree_id: &str,
        worktree_path: &str,
        sessions: &[&SessionStatus],
        workflow_state: Option<AgentState>,
        last_activity_at: f64,
    ) -> WorkspaceStatus {
        let open_sessions: Vec<&SessionStatus> = sessions
            .iter()
            .copied()
            .filter(|s| Self::is_live_session_state(&s.session_state))
            .collect();

        let mut running_count = 0usize;
        let mut waiting_count = 0usize;
        let mut error_count = 0usize;

        for s in &open_sessions {
            match s.agent_state {
                AgentState::Running => running_count += 1,
                AgentState::Waiting => waiting_count += 1,
                AgentState::Error => error_count += 1,
                AgentState::Done => {}
            }
        }

        if let Some(wf_state) = workflow_state {
            match wf_state {
                AgentState::Running => running_count += 1,
                AgentState::Waiting => waiting_count += 1,
                AgentState::Error => error_count += 1,
                AgentState::Done => {}
            }
        }

        let aggregated_state = if running_count > 0 {
            AgentState::Running
        } else if error_count > 0 {
            AgentState::Error
        } else if waiting_count > 0 {
            AgentState::Waiting
        } else {
            AgentState::Done
        };

        WorkspaceStatus {
            worktree_id: worktree_id.to_string(),
            worktree_path: worktree_path.to_string(),
            aggregated_state,
            running_count,
            waiting_count,
            error_count,
            session_count: open_sessions.len(),
            last_activity_at,
        }
    }

    fn is_live_session_state(state: &SessionState) -> bool {
        !matches!(state, SessionState::Closed | SessionState::Archived)
    }

    /// Workspace 集約時の `last_activity_at` は、Open session と集約対象 Workflow の最大値にする。
    /// `agent_state=None` の Workflow snapshot（aborted 等）は集約にも timestamp にも寄与させない。
    fn aggregate_with_workflow_snapshot(
        worktree_id: &str,
        worktree_path: &str,
        sessions: &[&SessionStatus],
        workflow_snapshot: Option<&WorkflowAggSnapshot>,
        fallback_last_activity_at: f64,
    ) -> WorkspaceStatus {
        let open_session_last_activity_at = sessions
            .iter()
            .copied()
            .filter(|s| Self::is_live_session_state(&s.session_state))
            .map(|s| s.last_activity_at)
            .reduce(f64::max);
        let workflow_last_activity_at = workflow_snapshot
            .filter(|snapshot| snapshot.agent_state.is_some())
            .map(|snapshot| snapshot.last_activity_at);
        let last_activity_at = open_session_last_activity_at
            .into_iter()
            .chain(workflow_last_activity_at)
            .reduce(f64::max)
            .unwrap_or(fallback_last_activity_at);
        let workflow_state = workflow_snapshot.and_then(|snapshot| snapshot.agent_state.clone());

        Self::aggregate(
            worktree_id,
            worktree_path,
            sessions,
            workflow_state,
            last_activity_at,
        )
    }

    /// 当該 worktree でアクティブな Workflow の集約用 snapshot を取り出す。
    fn workflow_agg_snapshot_for(&self, worktree_path: &str) -> Option<WorkflowAggSnapshot> {
        self.workflows.read().get(worktree_path).cloned()
    }

    fn session_workflow_step_key(status: &SessionStatus) -> Option<WorkflowStepKey> {
        Some(WorkflowStepKey {
            worktree_path: status.worktree_path.clone(),
            execution_id: status.workflow_execution_id.clone()?,
            step_name: status.workflow_step.clone()?,
            run_index: status.workflow_run_index,
        })
    }

    fn next_workflow_status_version(&self) -> u64 {
        self.workflow_status_version.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn is_same_workflow(left: &WorkflowStepKey, right: &WorkflowStepKey) -> bool {
        left.worktree_path == right.worktree_path && left.execution_id == right.execution_id
    }

    fn workflow_has_live_status(
        workflow_key: &WorkflowStepKey,
        live_steps: &HashMap<WorkflowStepKey, WorkflowStatusEntry>,
    ) -> bool {
        live_steps
            .keys()
            .any(|key| Self::is_same_workflow(key, workflow_key))
    }

    fn workflow_representative_for_key(
        workflow_key: &WorkflowStepKey,
        live_steps: &HashMap<WorkflowStepKey, WorkflowStatusEntry>,
        baselines: &HashMap<WorkflowStepKey, RepresentativeStatus>,
    ) -> Option<RepresentativeStatus> {
        if !Self::workflow_has_live_status(workflow_key, live_steps) {
            return None;
        }

        let baseline_statuses = baselines
            .iter()
            .filter(|(key, _)| Self::is_same_workflow(key, workflow_key))
            .map(|(key, baseline)| {
                live_steps
                    .get(key)
                    .map(|entry| entry.representative)
                    .unwrap_or(*baseline)
            });
        let live_only_statuses = live_steps
            .iter()
            .filter(|(key, _)| {
                Self::is_same_workflow(key, workflow_key) && !baselines.contains_key(*key)
            })
            .map(|(_, entry)| entry.representative);

        aggregate_representative_statuses(baseline_statuses.chain(live_only_statuses))
    }

    fn worktree_step_status_view_from_maps(
        worktree_path: &str,
        version: u64,
        step_statuses: &HashMap<WorkflowStepKey, WorkflowStatusEntry>,
        baselines: &HashMap<WorkflowStepKey, RepresentativeStatus>,
    ) -> WorktreeStepStatusView {
        let worktree_path = Self::canonical_worktree_path(worktree_path);
        let mut steps = Vec::new();
        let mut workflow_keys: HashMap<String, WorkflowStepKey> = HashMap::new();

        for (key, entry) in step_statuses {
            if key.worktree_path != worktree_path {
                continue;
            }
            steps.push(WorkflowStepRepresentative {
                execution_id: key.execution_id.clone(),
                step_name: key.step_name.clone(),
                run_index: key.run_index,
                representative: entry.representative.as_str().to_string(),
            });
            workflow_keys
                .entry(key.execution_id.clone())
                .or_insert_with(|| key.clone());
        }

        steps.sort_by(|left, right| {
            (
                &left.execution_id,
                &left.step_name,
                left.run_index.unwrap_or(1),
            )
                .cmp(&(
                    &right.execution_id,
                    &right.step_name,
                    right.run_index.unwrap_or(1),
                ))
        });

        let mut workflows = workflow_keys
            .into_values()
            .filter_map(|key| {
                Self::workflow_representative_for_key(&key, step_statuses, baselines).map(
                    |representative| WorkflowRepresentative {
                        execution_id: key.execution_id,
                        representative: representative.as_str().to_string(),
                    },
                )
            })
            .collect::<Vec<_>>();
        workflows.sort_by(|left, right| left.execution_id.cmp(&right.execution_id));

        WorktreeStepStatusView {
            worktree_path,
            version,
            steps,
            workflows,
        }
    }

    fn update_worktree_step_status_view(
        state: &mut WorkflowStepStatusState,
        worktree_path: &str,
        version: u64,
    ) -> WorktreeStepStatusView {
        let worktree_path = Self::canonical_worktree_path(worktree_path);
        if let Some(view) = state.views.get(&worktree_path) {
            if version < view.version {
                return view.clone();
            }
        }
        let view = Self::worktree_step_status_view_from_maps(
            &worktree_path,
            version,
            &state.steps,
            &state.baselines,
        );
        state.views.insert(worktree_path, view.clone());
        view
    }

    fn worktree_step_status_view(&self, worktree_path: &str) -> WorktreeStepStatusView {
        let worktree_path = Self::canonical_worktree_path(worktree_path);
        let state = self.workflow_step_status.read();
        state.views.get(&worktree_path).cloned().unwrap_or_else(|| {
            Self::worktree_step_status_view_from_maps(
                &worktree_path,
                0,
                &state.steps,
                &state.baselines,
            )
        })
    }

    fn strip_worktree_step_views(changes: &mut [AgentStatusChanges], worktree_path: &str) -> bool {
        let worktree_path = Self::canonical_worktree_path(worktree_path);
        let mut removed = false;
        for change in changes {
            let before = change.workflow_step_views.len();
            change
                .workflow_step_views
                .retain(|view| view.worktree_path != worktree_path.as_str());
            removed |= change.workflow_step_views.len() != before;
        }
        removed
    }

    fn update_workflow_step_baselines(
        &self,
        worktree_path: &str,
        execution_id: &str,
        projections: &[StepSessionProjection],
    ) -> WorkflowStepStatusUpdate {
        let mut grouped: HashMap<WorkflowStepKey, Vec<RepresentativeStatus>> = HashMap::new();
        for projection in projections {
            let key = WorkflowStepKey {
                worktree_path: worktree_path.to_string(),
                execution_id: execution_id.to_string(),
                step_name: projection.group_step_name.clone(),
                run_index: projection.group_run_index,
            };
            grouped
                .entry(key)
                .or_default()
                .push(projection.representative);
        }
        let next_baselines = grouped
            .into_iter()
            .filter_map(|(key, statuses)| {
                aggregate_representative_statuses(statuses)
                    .map(|representative| (key, representative))
            })
            .collect::<HashMap<_, _>>();

        let mut state = self.workflow_step_status.write();
        let previous_keys = state
            .baselines
            .keys()
            .filter(|key| key.worktree_path == worktree_path && key.execution_id == execution_id)
            .cloned()
            .collect::<Vec<_>>();
        let mut changed = false;
        for key in previous_keys {
            if next_baselines.contains_key(&key) {
                continue;
            }
            state.baselines.remove(&key);
            changed = true;
        }
        for (key, representative) in next_baselines {
            if state.baselines.insert(key, representative) != Some(representative) {
                changed = true;
            }
        }

        if !changed {
            return WorkflowStepStatusUpdate::default();
        }

        let has_visible_workflow = state
            .steps
            .keys()
            .any(|key| key.worktree_path == worktree_path && key.execution_id == execution_id);
        if !has_visible_workflow {
            return WorkflowStepStatusUpdate::default();
        }

        let version = self.next_workflow_status_version();
        let view = Self::update_worktree_step_status_view(&mut state, worktree_path, version);
        WorkflowStepStatusUpdate { views: vec![view] }
    }

    fn reaggregate_workflow_steps_for_worktree(
        &self,
        worktree_path: &str,
    ) -> WorkflowStepStatusUpdate {
        let mut grouped: HashMap<WorkflowStepKey, Vec<RepresentativeStatus>> = HashMap::new();
        let sessions = self.sessions.read();
        for status in sessions.values() {
            if status.worktree_path != worktree_path {
                continue;
            }
            if !Self::is_live_session_state(&status.session_state) {
                continue;
            }
            let Some(key) = Self::session_workflow_step_key(status) else {
                continue;
            };
            let Some(step_progress) = status.workflow_step_progress else {
                continue;
            };
            grouped.entry(key).or_default().push(session_result(
                step_progress,
                SessionActivity::from(status.agent_state.clone()),
            ));
        }

        let next_steps = grouped
            .into_iter()
            .filter_map(|(key, statuses)| {
                aggregate_representative_statuses(statuses)
                    .map(|representative| (key, representative))
            })
            .collect::<HashMap<_, _>>();

        let mut state = self.workflow_step_status.write();
        let mut changed = false;
        let previous_step_keys = state
            .steps
            .keys()
            .filter(|key| key.worktree_path == worktree_path)
            .cloned()
            .collect::<Vec<_>>();
        for key in &previous_step_keys {
            if next_steps.contains_key(key) {
                continue;
            }
            changed = true;
        }
        for (key, representative) in &next_steps {
            let previous = state.steps.get(key).map(|entry| entry.representative);
            if previous != Some(*representative) {
                changed = true;
            }
        }
        if !changed {
            return WorkflowStepStatusUpdate::default();
        }

        let version = self.next_workflow_status_version();
        for key in previous_step_keys {
            if !next_steps.contains_key(&key) {
                state.steps.remove(&key);
            }
        }
        for (key, representative) in next_steps {
            state
                .steps
                .insert(key, WorkflowStatusEntry { representative });
        }

        let view = Self::update_worktree_step_status_view(&mut state, worktree_path, version);
        WorkflowStepStatusUpdate { views: vec![view] }
    }

    fn apply_workflow_step_session_status(
        status: &mut SessionStatus,
        execution_id: &str,
        workflow_execution_state: &str,
        input: &WorkflowStepSessionStatusInput,
    ) {
        status.workflow_step = Some(input.step_name.clone());
        status.workflow_execution_state = Some(workflow_execution_state.to_string());
        status.workflow_execution_id = Some(execution_id.to_string());
        status.workflow_run_index = input.run_index;
        status.workflow_step_progress = Some(input.progress);
    }

    fn clear_workflow_step_session_status(status: &mut SessionStatus) {
        status.workflow_step = None;
        status.workflow_execution_state = None;
        status.workflow_execution_id = None;
        status.workflow_run_index = None;
        status.workflow_step_progress = None;
    }

    fn apply_pending_workflow_step_session_status(&self, status: &mut SessionStatus) {
        let pending = {
            let mut pending = self.pending_workflow_step_sessions.write();
            pending.remove(&status.chat_session_id)
        };
        let Some(pending) = pending else {
            return;
        };
        if pending.worktree_path != status.worktree_path || status.workflow_execution_id.is_some() {
            return;
        }
        Self::apply_workflow_step_session_status(
            status,
            &pending.execution_id,
            &pending.workflow_execution_state,
            &pending.input,
        );
    }

    fn update_pending_workflow_step_session_statuses(
        &self,
        worktree_path: &str,
        execution_id: &str,
        workflow_execution_state: &str,
        inputs: &HashMap<String, WorkflowStepSessionStatusInput>,
        existing_session_ids: &HashSet<String>,
    ) {
        let mut pending = self.pending_workflow_step_sessions.write();
        pending.retain(|session_id, pending| {
            !(pending.worktree_path == worktree_path
                && pending.execution_id == execution_id
                && !inputs.contains_key(session_id))
        });
        for (session_id, input) in inputs {
            if existing_session_ids.contains(session_id) {
                if pending.get(session_id).is_some_and(|pending| {
                    pending.worktree_path == worktree_path && pending.execution_id == execution_id
                }) {
                    pending.remove(session_id);
                }
                continue;
            }
            pending.insert(
                session_id.clone(),
                PendingWorkflowStepSessionStatus {
                    worktree_path: worktree_path.to_string(),
                    execution_id: execution_id.to_string(),
                    workflow_execution_state: workflow_execution_state.to_string(),
                    input: input.clone(),
                },
            );
        }
    }

    /// Session 状態を更新する。
    /// 1. dedup（前回と等価なら何もしない）
    /// 2. sessions マップに反映
    /// 3. 同 worktree の全 SessionStatus から WorkspaceStatus を再計算
    /// 4. 呼び出し側が transport 層で通知できるよう変更結果を返す
    pub fn update_session(&self, mut status: SessionStatus) -> AgentStatusChanges {
        Self::normalize_session_paths(&mut status);
        let prev = self.sessions.read().get(&status.chat_session_id).cloned();
        if prev.is_none() {
            self.apply_pending_workflow_step_session_status(&mut status);
        }

        // 1. dedup
        if let Some(prev) = prev {
            if Self::is_session_state_equivalent(&prev, &status) {
                return AgentStatusChanges::default();
            }
        }

        let worktree_id = status.worktree_id.clone();
        let worktree_path = status.worktree_path.clone();
        let last_activity_at = status.last_activity_at;
        let chat_session_id = status.chat_session_id.clone();
        let pty_id = status.pty_id.clone();
        let agent_state = status.agent_state.clone();

        // 2. sessions マップ反映
        {
            let mut sessions = self.sessions.write();
            sessions.insert(chat_session_id.clone(), status.clone());
        }

        // 3. workspace 再集約（aggregate 内で Closed は集約対象から除外される）
        let workflow_snapshot = self.workflow_agg_snapshot_for(&worktree_path);
        let new_workspace = {
            let sessions = self.sessions.read();
            let same_workspace: Vec<&SessionStatus> = sessions
                .values()
                .filter(|s| s.worktree_id == worktree_id)
                .collect();
            Self::aggregate_with_workflow_snapshot(
                &worktree_id,
                &worktree_path,
                &same_workspace,
                workflow_snapshot.as_ref(),
                last_activity_at,
            )
        };

        let workspace_changed = {
            let mut workspaces = self.workspaces.write();
            let prev_ws = workspaces.get(&worktree_id).cloned();
            let changed = prev_ws
                .as_ref()
                .map(|p| !Self::is_workspace_state_equivalent(p, &new_workspace))
                .unwrap_or(true);
            workspaces.insert(worktree_id.clone(), new_workspace.clone());
            changed
        };
        let workflow_step_update = self.reaggregate_workflow_steps_for_worktree(&worktree_path);

        AgentStatusChanges {
            session: Some(status),
            workspace: workspace_changed.then_some(new_workspace),
            agent_state: Some(AgentStateChange {
                worktree_path,
                state: agent_state,
                timestamp: last_activity_at,
                session_id: Some(chat_session_id),
                pty_id,
            }),
            workflow_step_views: workflow_step_update.views,
        }
    }

    /// Workflow の状態スナップショットを更新する。
    /// `agent_state` が `None` の場合は集約対象外（pending と等価）として扱う。
    /// dedup（前回と等価ならスキップ）の上、当該 worktree の WorkspaceStatus を
    /// 再集約し変化があれば返す。
    pub fn update_workflow_snapshot(
        &self,
        worktree_path: &str,
        execution_id: &str,
        agent_state: Option<AgentState>,
        last_activity_at: f64,
    ) -> AgentStatusChanges {
        let worktree_path = Self::canonical_worktree_path(worktree_path);
        let new_snapshot = WorkflowAggSnapshot {
            execution_id: execution_id.to_string(),
            agent_state: agent_state.clone(),
            last_activity_at,
        };

        // 1. dedup（execution_id / agent_state が変わらなければスキップ）
        {
            let workflows = self.workflows.read();
            if let Some(prev) = workflows.get(&worktree_path) {
                if prev.execution_id == new_snapshot.execution_id
                    && prev.agent_state == new_snapshot.agent_state
                {
                    return AgentStatusChanges::default();
                }
            }
        }

        // 2. workflows マップ反映
        {
            let mut workflows = self.workflows.write();
            workflows.insert(worktree_path.clone(), new_snapshot);
        }

        // 3. 再集約
        AgentStatusChanges {
            workspace: self.reaggregate_workspace(&worktree_path, last_activity_at),
            ..Default::default()
        }
    }

    pub fn sync_workflow_step_session_statuses(
        &self,
        worktree_path: &str,
        execution_id: &str,
        workflow_execution_state: &str,
        projections: Vec<StepSessionProjection>,
    ) -> Vec<AgentStatusChanges> {
        let worktree_path = Self::canonical_worktree_path(worktree_path);
        let baseline_update =
            self.update_workflow_step_baselines(&worktree_path, execution_id, &projections);
        let inputs = projections
            .into_iter()
            .filter_map(|projection| {
                let session_id = projection.session_id?;
                Some((
                    session_id,
                    WorkflowStepSessionStatusInput {
                        step_name: projection.group_step_name,
                        run_index: projection.group_run_index,
                        progress: projection.progress,
                    },
                ))
            })
            .collect::<HashMap<_, _>>();

        let mut changes = Vec::new();
        let sessions = self.list_sessions();
        let existing_session_ids = sessions
            .iter()
            .filter(|status| status.worktree_path == worktree_path)
            .map(|status| status.chat_session_id.clone())
            .collect::<HashSet<_>>();
        self.update_pending_workflow_step_session_statuses(
            &worktree_path,
            execution_id,
            workflow_execution_state,
            &inputs,
            &existing_session_ids,
        );

        for status in sessions {
            if status.worktree_path != worktree_path {
                continue;
            }
            let mut updated = status;
            if let Some(input) = inputs.get(&updated.chat_session_id) {
                Self::apply_workflow_step_session_status(
                    &mut updated,
                    execution_id,
                    workflow_execution_state,
                    input,
                );
            } else if updated.workflow_execution_id.as_deref() == Some(execution_id) {
                Self::clear_workflow_step_session_status(&mut updated);
            } else {
                continue;
            }

            let update_changes = self.update_session(updated);
            if !update_changes.is_empty() {
                changes.push(update_changes);
            }
        }

        let had_live_step_view = Self::strip_worktree_step_views(&mut changes, &worktree_path);
        if had_live_step_view || !baseline_update.is_empty() {
            let latest_view = self.worktree_step_status_view(&worktree_path);
            changes.push(AgentStatusChanges {
                workflow_step_views: vec![latest_view],
                ..Default::default()
            });
        }

        changes
    }

    /// 当該 worktree の WorkspaceStatus を再集約し、前回と差分があれば
    /// 差分があれば更新後の `WorkspaceStatus` を返す。
    /// Session も Workflow も無い空状態なら workspaces から entry を削除し
    /// 空 WorkspaceStatus を 1 度だけ送る（`remove_session` と同じ振る舞い）。
    fn reaggregate_workspace(
        &self,
        worktree_path: &str,
        last_activity_at: f64,
    ) -> Option<WorkspaceStatus> {
        let worktree_path = Self::canonical_worktree_path(worktree_path);
        let workflow_snapshot = self.workflow_agg_snapshot_for(&worktree_path);
        let (worktree_id, new_workspace) = {
            let sessions = self.sessions.read();
            let same_workspace: Vec<&SessionStatus> = sessions
                .values()
                .filter(|s| s.worktree_path == worktree_path)
                .collect();
            // worktree_id は worktree_path と同値で運用されている。
            // session が無い場合のフォールバックとしても worktree_path を使う。
            let worktree_id = same_workspace
                .first()
                .map(|s| s.worktree_id.clone())
                .unwrap_or_else(|| worktree_path.clone());
            let workspace = if same_workspace.is_empty()
                && workflow_snapshot
                    .as_ref()
                    .is_none_or(|snapshot| snapshot.agent_state.is_none())
            {
                None
            } else {
                Some(Self::aggregate_with_workflow_snapshot(
                    &worktree_id,
                    &worktree_path,
                    &same_workspace,
                    workflow_snapshot.as_ref(),
                    last_activity_at,
                ))
            };
            (worktree_id, workspace)
        };

        match new_workspace {
            Some(ws) => {
                let changed = {
                    let mut workspaces = self.workspaces.write();
                    let prev = workspaces.get(&worktree_id).cloned();
                    let changed = prev
                        .as_ref()
                        .map(|p| !Self::is_workspace_state_equivalent(p, &ws))
                        .unwrap_or(true);
                    workspaces.insert(worktree_id.clone(), ws.clone());
                    changed
                };
                if changed {
                    return Some(ws);
                }
            }
            None => {
                let removed_ws = {
                    let mut workspaces = self.workspaces.write();
                    workspaces.remove(&worktree_id)
                };
                if removed_ws.is_some() {
                    let empty = WorkspaceStatus {
                        worktree_id: worktree_id.clone(),
                        worktree_path: worktree_path.clone(),
                        aggregated_state: AgentState::Done,
                        running_count: 0,
                        waiting_count: 0,
                        error_count: 0,
                        session_count: 0,
                        last_activity_at,
                    };
                    return Some(empty);
                }
            }
        }
        None
    }

    /// SessionStore からの `SessionState` 変更通知を受け取り、保持している
    /// `SessionStatus` の `session_state` を最新化した上で再集約する。
    /// Closed/Archived への遷移は `aggregate` 段階で集約対象から外れ、
    /// 復帰時は再び集約対象に戻る。
    pub fn on_session_state_changed(
        &self,
        chat_session_id: &str,
        new_state: SessionState,
    ) -> AgentStatusChanges {
        let existing = self.sessions.read().get(chat_session_id).cloned();
        let Some(existing) = existing else {
            return AgentStatusChanges::default();
        };
        if let Some(updated) =
            Self::build_state_transition(&existing, new_state, current_timestamp())
        {
            return self.update_session(updated);
        }
        AgentStatusChanges::default()
    }

    /// `on_session_state_changed` の中核ロジック。
    /// Closed / Idle への tab lifecycle 遷移では、閉じる前の Streaming や
    /// WaitingPermission の `turn_phase` / `pending_permission` を引きずらないよう
    /// `turn_phase=Idle`, `pending_permission=false` に正規化してから
    /// `agent_state` を再算出する。状態が変わらない場合は `None`。
    fn build_state_transition(
        existing: &SessionStatus,
        new_state: SessionState,
        last_activity_at: f64,
    ) -> Option<SessionStatus> {
        if existing.session_state == new_state {
            return None;
        }
        let normalize_to_idle = matches!(
            new_state,
            SessionState::Closed | SessionState::Archived | SessionState::Idle
        );
        let (turn_phase_repr, pending_permission, pending_permission_request) = if normalize_to_idle
        {
            (TurnPhaseRepr::Idle, false, None)
        } else {
            (
                existing.turn_phase,
                existing.pending_permission,
                existing.pending_permission_request.clone(),
            )
        };
        let agent_state = Self::derive_agent_state(turn_phase_repr.into(), new_state.clone());
        Some(SessionStatus {
            session_state: new_state,
            agent_state,
            turn_phase: turn_phase_repr,
            pending_permission,
            pending_permission_request,
            last_activity_at,
            ..existing.clone()
        })
    }

    /// Session を削除し、worktree を再集約する。
    pub fn get_session(&self, chat_session_id: &str) -> Option<SessionStatus> {
        self.sessions.read().get(chat_session_id).cloned()
    }

    pub fn get_workspace(&self, worktree_id: &str) -> Option<WorkspaceStatus> {
        self.workspaces
            .read()
            .get(&Self::canonical_worktree_path(worktree_id))
            .cloned()
    }

    pub fn list_workspaces(&self) -> Vec<WorkspaceStatus> {
        self.workspaces.read().values().cloned().collect()
    }

    pub fn list_sessions(&self) -> Vec<SessionStatus> {
        self.sessions.read().values().cloned().collect()
    }

    pub fn query_worktree_step_statuses(&self, worktree_path: &str) -> WorktreeStepStatusView {
        self.worktree_step_status_view(worktree_path)
    }
}

pub fn current_timestamp() -> f64 {
    crate::other::utils::unix_timestamp_seconds()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_session(
        id: &str,
        worktree: &str,
        turn_phase: TurnPhase,
        session_state: SessionState,
    ) -> SessionStatus {
        let agent_state = AgentStatusCenter::derive_agent_state(turn_phase, session_state.clone());
        SessionStatus {
            chat_session_id: id.to_string(),
            worktree_id: worktree.to_string(),
            worktree_path: worktree.to_string(),
            pty_id: None,
            agent_state,
            turn_phase: TurnPhaseRepr::from(turn_phase),
            session_state,
            pending_permission: matches!(turn_phase, TurnPhase::WaitingPermission),
            pending_permission_request: None,
            last_activity_at: 0.0,
            workflow_step: None,
            workflow_execution_state: None,
            workflow_execution_id: None,
            workflow_run_index: None,
            workflow_step_progress: None,
        }
    }

    fn workflow_session(
        id: &str,
        step: &str,
        run_index: Option<u32>,
        progress: StepProgress,
        agent_state: AgentState,
    ) -> SessionStatus {
        let mut session = mk_session("unused", "/repo", TurnPhase::Idle, SessionState::Done);
        session.chat_session_id = id.to_string();
        session.agent_state = agent_state;
        session.workflow_execution_id = Some("exec-1".to_string());
        session.workflow_step = Some(step.to_string());
        session.workflow_run_index = run_index;
        session.workflow_step_progress = Some(progress);
        session
    }

    fn projection(
        session_id: Option<&str>,
        step_name: &str,
        run_index: Option<u32>,
        representative: RepresentativeStatus,
        progress: StepProgress,
    ) -> StepSessionProjection {
        StepSessionProjection {
            session_id: session_id.map(str::to_string),
            step_name: step_name.to_string(),
            run_index,
            group_step_name: step_name.to_string(),
            group_run_index: run_index,
            progress,
            representative,
            order: 0,
        }
    }

    #[test]
    fn update_session_normalizes_worktree_paths_before_storing() {
        let center = AgentStatusCenter::new();
        let changes = center.update_session(mk_session(
            "s1",
            r"C:\repo\wt",
            TurnPhase::Streaming,
            SessionState::Active,
        ));

        let session = changes.session.unwrap();
        assert_eq!(session.worktree_id, "C:/repo/wt");
        assert_eq!(session.worktree_path, "C:/repo/wt");
        let workspace = changes.workspace.unwrap();
        assert_eq!(workspace.worktree_id, "C:/repo/wt");
        assert_eq!(workspace.worktree_path, "C:/repo/wt");
        assert!(center.get_workspace("C:/repo/wt").is_some());
        assert!(center.get_workspace(r"C:\repo\wt").is_some());
    }

    #[test]
    fn update_session_preserves_unc_prefix_before_storing() {
        let center = AgentStatusCenter::new();
        let changes = center.update_session(mk_session(
            "s1",
            r"\\server\share\wt",
            TurnPhase::Streaming,
            SessionState::Active,
        ));

        let session = changes.session.unwrap();
        assert_eq!(session.worktree_id, "//server/share/wt");
        assert_eq!(session.worktree_path, "//server/share/wt");
        let workspace = changes.workspace.unwrap();
        assert_eq!(workspace.worktree_id, "//server/share/wt");
        assert_eq!(workspace.worktree_path, "//server/share/wt");
        assert!(center.get_workspace("//server/share/wt").is_some());
        assert!(center.get_workspace(r"\\server\share\wt").is_some());
    }

    // ---- derive_agent_state ----

    #[test]
    fn streaming_maps_to_running_regardless_of_session_state() {
        for ss in [
            SessionState::Active,
            SessionState::Idle,
            SessionState::Done,
            SessionState::Error,
            SessionState::Closed,
        ] {
            assert_eq!(
                AgentStatusCenter::derive_agent_state(TurnPhase::Streaming, ss),
                AgentState::Running
            );
        }
    }

    #[test]
    fn waiting_permission_maps_to_waiting_regardless_of_session_state() {
        for ss in [
            SessionState::Active,
            SessionState::Idle,
            SessionState::Done,
            SessionState::Error,
            SessionState::Closed,
        ] {
            assert_eq!(
                AgentStatusCenter::derive_agent_state(TurnPhase::WaitingPermission, ss),
                AgentState::Waiting
            );
        }
    }

    #[test]
    fn idle_with_error_session_maps_to_error() {
        assert_eq!(
            AgentStatusCenter::derive_agent_state(TurnPhase::Idle, SessionState::Error),
            AgentState::Error
        );
    }

    #[test]
    fn idle_with_non_error_non_idle_session_maps_to_done() {
        for ss in [
            SessionState::Active,
            SessionState::Done,
            SessionState::Closed,
        ] {
            assert_eq!(
                AgentStatusCenter::derive_agent_state(TurnPhase::Idle, ss),
                AgentState::Done
            );
        }
    }

    #[test]
    fn idle_with_idle_session_maps_to_waiting() {
        assert_eq!(
            AgentStatusCenter::derive_agent_state(TurnPhase::Idle, SessionState::Idle),
            AgentState::Waiting
        );
    }

    // ---- aggregate ----

    #[test]
    fn aggregate_empty_sessions_is_done() {
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[], None, 100.0);
        assert_eq!(ws.aggregated_state, AgentState::Done);
        assert_eq!(ws.session_count, 0);
        assert_eq!(ws.running_count, 0);
        assert_eq!(ws.waiting_count, 0);
        assert_eq!(ws.error_count, 0);
        assert_eq!(ws.last_activity_at, 100.0);
    }

    #[test]
    fn aggregate_only_done_is_done() {
        let s1 = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Done);
        let s2 = mk_session("b", "/repo", TurnPhase::Idle, SessionState::Done);
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[&s1, &s2], None, 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Done);
        assert_eq!(ws.session_count, 2);
    }

    #[test]
    fn aggregate_running_dominates_done() {
        let s1 = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Done);
        let s2 = mk_session("b", "/repo", TurnPhase::Streaming, SessionState::Active);
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[&s1, &s2], None, 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Running);
        assert_eq!(ws.running_count, 1);
    }

    #[test]
    fn aggregate_running_dominates_waiting() {
        let s1 = mk_session("a", "/repo", TurnPhase::Streaming, SessionState::Active);
        let s2 = mk_session(
            "b",
            "/repo",
            TurnPhase::WaitingPermission,
            SessionState::Active,
        );
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[&s1, &s2], None, 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Running);
        assert_eq!(ws.waiting_count, 1);
        assert_eq!(ws.running_count, 1);
    }

    #[test]
    fn aggregate_excludes_closed_sessions() {
        // Closed Session（過去に Error → Closed になったタブを想定）は集約に寄与しない。
        // 残るオープン中 Session の状態で判定される。
        let open_done = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Done);
        let mut closed_with_error_history =
            mk_session("b", "/repo", TurnPhase::Idle, SessionState::Closed);
        // 過去に Error だった「痕跡」: agent_state は Error のまま残っていても、
        // session_state=Closed なのでフィルタされる。
        closed_with_error_history.agent_state = AgentState::Error;
        let ws = AgentStatusCenter::aggregate(
            "/repo",
            "/repo",
            &[&open_done, &closed_with_error_history],
            None,
            0.0,
        );
        assert_eq!(ws.aggregated_state, AgentState::Done);
        assert_eq!(ws.error_count, 0);
        assert_eq!(ws.session_count, 1);
    }

    #[test]
    fn aggregate_excludes_archived_sessions() {
        let open_done = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Done);
        let mut archived_with_error_history =
            mk_session("b", "/repo", TurnPhase::Idle, SessionState::Archived);
        archived_with_error_history.agent_state = AgentState::Error;
        let ws = AgentStatusCenter::aggregate(
            "/repo",
            "/repo",
            &[&open_done, &archived_with_error_history],
            None,
            0.0,
        );

        assert_eq!(ws.aggregated_state, AgentState::Done);
        assert_eq!(ws.error_count, 0);
        assert_eq!(ws.session_count, 1);
    }

    #[test]
    fn aggregate_all_closed_yields_no_aggregation_target() {
        // オープン中 Session が 0 件のとき: session_count=0 で「集約対象なし」を示す
        let closed_a = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Closed);
        let closed_b = mk_session("b", "/repo", TurnPhase::Idle, SessionState::Closed);
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[&closed_a, &closed_b], None, 0.0);
        assert_eq!(ws.session_count, 0);
        assert_eq!(ws.error_count, 0);
        assert_eq!(ws.aggregated_state, AgentState::Done);
    }

    #[test]
    fn aggregate_keeps_error_when_multiple_open_errors_exist() {
        // 同じ worktree に Error が複数オープン中ならエラー集約は維持される
        let open_error_a = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Error);
        let open_error_b = mk_session("b", "/repo", TurnPhase::Idle, SessionState::Error);
        let closed = mk_session("c", "/repo", TurnPhase::Idle, SessionState::Closed);
        let ws = AgentStatusCenter::aggregate(
            "/repo",
            "/repo",
            &[&open_error_a, &open_error_b, &closed],
            None,
            0.0,
        );
        assert_eq!(ws.aggregated_state, AgentState::Error);
        assert_eq!(ws.error_count, 2);
        assert_eq!(ws.session_count, 2);
    }

    #[test]
    fn aggregate_running_dominates_error_and_waiting() {
        let s1 = mk_session("a", "/repo", TurnPhase::Streaming, SessionState::Active);
        let s2 = mk_session(
            "b",
            "/repo",
            TurnPhase::WaitingPermission,
            SessionState::Active,
        );
        let s3 = mk_session("c", "/repo", TurnPhase::Idle, SessionState::Error);
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[&s1, &s2, &s3], None, 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Running);
        assert_eq!(ws.error_count, 1);
        assert_eq!(ws.waiting_count, 1);
        assert_eq!(ws.running_count, 1);
    }

    // ---- dedup helpers ----

    #[test]
    fn dedup_ignores_last_activity_at() {
        let mut a = mk_session("s", "/repo", TurnPhase::Streaming, SessionState::Active);
        let mut b = a.clone();
        a.last_activity_at = 100.0;
        b.last_activity_at = 999.0;
        assert!(AgentStatusCenter::is_session_state_equivalent(&a, &b));
    }

    #[test]
    fn dedup_detects_state_change() {
        let a = mk_session("s", "/repo", TurnPhase::Streaming, SessionState::Active);
        let b = mk_session("s", "/repo", TurnPhase::Idle, SessionState::Done);
        assert!(!AgentStatusCenter::is_session_state_equivalent(&a, &b));
    }

    #[test]
    fn workspace_dedup_ignores_last_activity_at() {
        let mut a = AgentStatusCenter::aggregate("/r", "/r", &[], None, 100.0);
        let mut b = a.clone();
        a.last_activity_at = 100.0;
        b.last_activity_at = 999.0;
        assert!(AgentStatusCenter::is_workspace_state_equivalent(&a, &b));
    }

    #[test]
    fn turn_phase_repr_roundtrip() {
        for tp in [
            TurnPhase::Idle,
            TurnPhase::Streaming,
            TurnPhase::WaitingPermission,
        ] {
            let repr = TurnPhaseRepr::from(tp);
            let back = TurnPhase::from(repr);
            assert_eq!(tp, back);
        }
    }

    #[test]
    fn session_status_serialization_omits_internal_workflow_aggregation_inputs() {
        let status = workflow_session(
            "step-a",
            "build",
            Some(3),
            StepProgress::Running,
            AgentState::Running,
        );

        let value = serde_json::to_value(status).expect("session status serializes");

        assert!(value.get("workflow_step").is_some());
        assert!(value.get("workflow_execution_state").is_some());
        assert!(value.get("workflow_execution_id").is_none());
        assert!(value.get("workflow_run_index").is_none());
        assert!(value.get("workflow_step_progress").is_none());
    }

    // ---- on_session_state_changed core (build_state_transition) ----

    #[test]
    fn build_state_transition_returns_none_when_state_unchanged() {
        let s = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Idle);
        assert!(AgentStatusCenter::build_state_transition(&s, SessionState::Idle, 0.0).is_none());
    }

    #[test]
    fn build_state_transition_closes_streaming_session_with_idle_normalization() {
        // 閉じる前に Streaming だった SessionStatus を Closed に遷移させると
        // turn_phase / pending_permission を引きずらず正規化される
        let mut streaming = mk_session("a", "/repo", TurnPhase::Streaming, SessionState::Active);
        streaming.pending_permission = true;
        streaming.pending_permission_request = Some(serde_json::json!({
            "request_id": "req-1",
            "tool_name": "Edit",
            "input": {},
            "tool_use_id": "toolu-1"
        }));
        let updated =
            AgentStatusCenter::build_state_transition(&streaming, SessionState::Closed, 42.0)
                .expect("transition should produce updated session");
        assert_eq!(updated.session_state, SessionState::Closed);
        assert_eq!(updated.turn_phase, TurnPhaseRepr::Idle);
        assert!(!updated.pending_permission);
        assert!(updated.pending_permission_request.is_none());
        assert_eq!(updated.last_activity_at, 42.0);
    }

    #[test]
    fn build_state_transition_restores_to_idle_normalizes_turn_phase() {
        // Closed Session を Idle に復帰させる際も turn_phase が Idle に正規化され
        // agent_state は Waiting として再寄与する
        let mut closed = mk_session(
            "a",
            "/repo",
            TurnPhase::WaitingPermission,
            SessionState::Closed,
        );
        closed.pending_permission = true;
        closed.pending_permission_request = Some(serde_json::json!({
            "request_id": "req-1",
            "tool_name": "Edit",
            "input": {},
            "tool_use_id": "toolu-1"
        }));
        let updated = AgentStatusCenter::build_state_transition(&closed, SessionState::Idle, 0.0)
            .expect("transition should produce updated session");
        assert_eq!(updated.session_state, SessionState::Idle);
        assert_eq!(updated.turn_phase, TurnPhaseRepr::Idle);
        assert!(!updated.pending_permission);
        assert!(updated.pending_permission_request.is_none());
        assert_eq!(updated.agent_state, AgentState::Waiting);
    }

    #[test]
    fn build_state_transition_preserves_turn_phase_for_non_lifecycle_states() {
        // Active / Done / Error への遷移では turn_phase はそのまま再計算に使われる
        let streaming = mk_session("a", "/repo", TurnPhase::Streaming, SessionState::Active);
        let updated =
            AgentStatusCenter::build_state_transition(&streaming, SessionState::Error, 0.0)
                .expect("transition should produce updated session");
        // Streaming のまま再計算されるので derive 結果は Running
        assert_eq!(updated.turn_phase, TurnPhaseRepr::Streaming);
        assert_eq!(updated.agent_state, AgentState::Running);
        assert_eq!(updated.session_state, SessionState::Error);
    }

    #[test]
    fn close_error_session_yields_done_aggregate_with_session_count_one() {
        // Spec 中核挙動: Error + Done 登録後に Error 側を Closed に遷移させると
        // aggregate は Done / session_count=1 になる
        let mut error_session = mk_session("err", "/repo", TurnPhase::Idle, SessionState::Error);
        let done_session = mk_session("done", "/repo", TurnPhase::Idle, SessionState::Done);

        let initial = AgentStatusCenter::aggregate(
            "/repo",
            "/repo",
            &[&error_session, &done_session],
            None,
            0.0,
        );
        assert_eq!(initial.aggregated_state, AgentState::Error);
        assert_eq!(initial.session_count, 2);

        let closed =
            AgentStatusCenter::build_state_transition(&error_session, SessionState::Closed, 0.0)
                .expect("transition should produce updated session");
        error_session = closed;

        let after = AgentStatusCenter::aggregate(
            "/repo",
            "/repo",
            &[&error_session, &done_session],
            None,
            0.0,
        );
        assert_eq!(after.aggregated_state, AgentState::Done);
        assert_eq!(after.session_count, 1);
        assert_eq!(after.error_count, 0);
    }

    #[test]
    fn restore_closed_session_to_idle_recontributes_to_aggregate() {
        // Closed → Idle で再び集約対象に戻り、aggregate に Waiting として寄与する
        let mut closed = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Closed);
        let done_session = mk_session("b", "/repo", TurnPhase::Idle, SessionState::Done);

        let before =
            AgentStatusCenter::aggregate("/repo", "/repo", &[&closed, &done_session], None, 0.0);
        assert_eq!(before.session_count, 1);
        assert_eq!(before.aggregated_state, AgentState::Done);

        let restored = AgentStatusCenter::build_state_transition(&closed, SessionState::Idle, 0.0)
            .expect("transition should produce updated session");
        closed = restored;

        let after =
            AgentStatusCenter::aggregate("/repo", "/repo", &[&closed, &done_session], None, 0.0);
        assert_eq!(after.session_count, 2);
        assert_eq!(after.aggregated_state, AgentState::Waiting);
        assert_eq!(after.waiting_count, 1);
    }

    fn mk_center() -> AgentStatusCenter {
        AgentStatusCenter::new()
    }

    #[test]
    fn on_session_state_changed_closes_error_session_and_reaggregates_workspace() {
        // Spec Scenario「エラー Session を閉じれば Workspace のエラー表示は解消する」
        // および「閉じた Session を復帰させれば再寄与する」の本経路を担保する。
        // build_state_transition + aggregate の単体合成ではなく、
        // on_session_state_changed -> update_session -> workspace 再集約の経路を通す。
        let center = mk_center();
        let error_session = mk_session("err", "/repo", TurnPhase::Idle, SessionState::Error);
        let done_session = mk_session("done", "/repo", TurnPhase::Idle, SessionState::Done);
        center.update_session(error_session.clone());
        center.update_session(done_session.clone());

        let initial = center.get_workspace("/repo").expect("workspace registered");
        assert_eq!(initial.aggregated_state, AgentState::Error);
        assert_eq!(initial.session_count, 2);

        // Error Session を Closed に遷移させると Workspace は Done / session_count=1 に再集約される
        center.on_session_state_changed("err", SessionState::Closed);
        let after_close = center
            .get_workspace("/repo")
            .expect("workspace still tracked");
        assert_eq!(after_close.aggregated_state, AgentState::Done);
        assert_eq!(after_close.session_count, 1);
        assert_eq!(after_close.error_count, 0);

        // Closed → Idle で復帰させると Waiting / session_count=2 として再寄与する
        center.on_session_state_changed("err", SessionState::Idle);
        let after_restore = center
            .get_workspace("/repo")
            .expect("workspace still tracked");
        assert_eq!(after_restore.aggregated_state, AgentState::Waiting);
        assert_eq!(after_restore.session_count, 2);
        assert_eq!(after_restore.waiting_count, 1);
    }

    // ---- Workflow 状態の Workspace 集約への寄与 ----

    #[test]
    fn workflow_execution_state_to_agent_state_maps_each_variant() {
        use crate::domain::workflow::WorkflowExecutionState;
        assert_eq!(
            AgentStatusCenter::workflow_execution_state_to_agent_state(
                &WorkflowExecutionState::Running
            ),
            Some(AgentState::Running)
        );
        assert_eq!(
            AgentStatusCenter::workflow_execution_state_to_agent_state(
                &WorkflowExecutionState::WaitingApproval
            ),
            Some(AgentState::Waiting)
        );
        assert_eq!(
            AgentStatusCenter::workflow_execution_state_to_agent_state(
                &WorkflowExecutionState::Failed {
                    reason: "boom".into(),
                    kind: crate::domain::workflow::WorkflowStepFailureKind::InfrastructureCrash,
                    retry_count: None,
                }
            ),
            Some(AgentState::Error)
        );
        assert_eq!(
            AgentStatusCenter::workflow_execution_state_to_agent_state(
                &WorkflowExecutionState::Completed
            ),
            Some(AgentState::Done)
        );
        assert_eq!(
            AgentStatusCenter::workflow_execution_state_to_agent_state(
                &WorkflowExecutionState::Aborted
            ),
            None
        );
    }

    #[test]
    fn aggregate_with_running_session_outranks_workflow_waiting() {
        // Session=Running、Workflow=Waiting → Workspace=Running
        let s = mk_session("a", "/repo", TurnPhase::Streaming, SessionState::Active);
        let ws =
            AgentStatusCenter::aggregate("/repo", "/repo", &[&s], Some(AgentState::Waiting), 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Running);
        assert_eq!(ws.running_count, 1);
        assert_eq!(ws.waiting_count, 1);
        assert_eq!(ws.session_count, 1);
    }

    #[test]
    fn aggregate_with_workflow_error_outranks_done_sessions() {
        // Session=Done、Workflow=Error → Workspace=Error
        let s = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Done);
        let ws =
            AgentStatusCenter::aggregate("/repo", "/repo", &[&s], Some(AgentState::Error), 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Error);
        assert_eq!(ws.error_count, 1);
        // session_count は Workflow を含めない
        assert_eq!(ws.session_count, 1);
    }

    #[test]
    fn aggregate_with_workflow_aborted_is_excluded() {
        // Workflow=Aborted（None）の場合、Session 状態のみで集約される
        let s = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Done);
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[&s], None, 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Done);
        assert_eq!(ws.session_count, 1);
        assert_eq!(ws.waiting_count, 0);
    }

    #[test]
    fn aggregate_with_only_workflow_waiting_yields_waiting_with_zero_sessions() {
        // Session が無く Workflow=Waiting のとき、Workspace=Waiting / session_count=0
        let ws =
            AgentStatusCenter::aggregate("/repo", "/repo", &[], Some(AgentState::Waiting), 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Waiting);
        assert_eq!(ws.session_count, 0);
        assert_eq!(ws.waiting_count, 1);
    }

    #[test]
    fn update_workflow_snapshot_returns_workspace_change_when_only_workflow_changes() {
        // Session=Done で Workspace=Done の状態から、Workflow=Waiting に更新すると
        // session 不変でも Workspace 集約は Waiting に変化する。
        let center = mk_center();
        let done = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Done);
        center.update_session(done);

        let before = center.get_workspace("/repo").expect("workspace registered");
        assert_eq!(before.aggregated_state, AgentState::Done);

        center.update_workflow_snapshot("/repo", "exec-1", Some(AgentState::Waiting), 1.0);

        let after = center
            .get_workspace("/repo")
            .expect("workspace still tracked");
        assert_eq!(after.aggregated_state, AgentState::Waiting);
        assert_eq!(after.waiting_count, 1);
        assert_eq!(after.session_count, 1);
    }

    #[test]
    fn update_workflow_snapshot_dedup_does_not_change_workspace_when_unchanged() {
        // 同じ execution_id / agent_state の二度目の update は WorkspaceStatus を変えない。
        let center = mk_center();
        let done = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Done);
        center.update_session(done);

        center.update_workflow_snapshot("/repo", "exec-1", Some(AgentState::Waiting), 1.0);
        let first = center.get_workspace("/repo").expect("first snapshot");

        // 同一 snapshot → workspace は不変
        center.update_workflow_snapshot("/repo", "exec-1", Some(AgentState::Waiting), 2.0);
        let second = center.get_workspace("/repo").expect("second snapshot");

        // last_activity_at 以外のフィールドは等しい
        assert_eq!(first.aggregated_state, second.aggregated_state);
        assert_eq!(first.waiting_count, second.waiting_count);
        assert_eq!(first.last_activity_at, second.last_activity_at);
    }

    #[test]
    fn update_workflow_snapshot_to_aborted_releases_workflow_contribution() {
        // 一度 Waiting にした後、Aborted（None）に更新すると Workflow 寄与が外れる。
        let center = mk_center();
        let done = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Done);
        center.update_session(done);

        center.update_workflow_snapshot("/repo", "exec-1", Some(AgentState::Waiting), 1.0);
        assert_eq!(
            center.get_workspace("/repo").unwrap().aggregated_state,
            AgentState::Waiting
        );

        center.update_workflow_snapshot("/repo", "exec-1", None, 2.0);
        let after = center.get_workspace("/repo").expect("workspace tracked");
        assert_eq!(after.aggregated_state, AgentState::Done);
        assert_eq!(after.waiting_count, 0);
    }

    #[test]
    fn workflow_update_then_older_session_update_does_not_rewind_workspace_last_activity() {
        let center = mk_center();
        let mut session = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Done);
        session.last_activity_at = 60.0;
        center.update_session(session);
        center.update_workflow_snapshot("/repo", "exec-1", Some(AgentState::Running), 90.0);

        let mut older_session_update =
            mk_session("a", "/repo", TurnPhase::Streaming, SessionState::Active);
        older_session_update.last_activity_at = 50.0;
        center.update_session(older_session_update);

        let ws = center.get_workspace("/repo").expect("workspace tracked");
        assert_eq!(ws.aggregated_state, AgentState::Running);
        assert_eq!(ws.last_activity_at, 90.0);
    }

    #[test]
    fn aggregate_excludes_workflow_timestamp_when_aborted() {
        let center = mk_center();
        let mut session = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Done);
        session.last_activity_at = 80.0;
        center.update_session(session);

        center.update_workflow_snapshot("/repo", "exec-1", None, 999.0);

        let ws = center.get_workspace("/repo").expect("workspace tracked");
        assert_eq!(ws.aggregated_state, AgentState::Done);
        assert_eq!(ws.last_activity_at, 80.0);
    }

    #[test]
    fn aggregate_excludes_closed_session_timestamp() {
        let center = mk_center();
        let mut open_session = mk_session("open", "/repo", TurnPhase::Idle, SessionState::Done);
        open_session.last_activity_at = 80.0;
        center.update_session(open_session);

        let mut closed_session =
            mk_session("closed", "/repo", TurnPhase::Idle, SessionState::Closed);
        closed_session.last_activity_at = 200.0;
        center.update_session(closed_session);

        let ws = center.get_workspace("/repo").expect("workspace tracked");
        assert_eq!(ws.session_count, 1);
        assert_eq!(ws.last_activity_at, 80.0);
    }

    #[test]
    fn aggregate_excludes_archived_session_timestamp() {
        let center = mk_center();
        let mut open_session = mk_session("open", "/repo", TurnPhase::Idle, SessionState::Done);
        open_session.last_activity_at = 80.0;
        center.update_session(open_session);

        let mut archived_session =
            mk_session("archived", "/repo", TurnPhase::Idle, SessionState::Archived);
        archived_session.last_activity_at = 200.0;
        center.update_session(archived_session);

        let ws = center.get_workspace("/repo").expect("workspace tracked");
        assert_eq!(ws.session_count, 1);
        assert_eq!(ws.last_activity_at, 80.0);
    }

    #[test]
    fn update_workflow_snapshot_without_any_sessions_creates_workspace_entry() {
        // Session が一切登録されていない worktree でも Workflow 状態から WorkspaceStatus を作る。
        let center = mk_center();
        center.update_workflow_snapshot("/repo", "exec-1", Some(AgentState::Running), 5.0);

        let ws = center
            .get_workspace("/repo")
            .expect("workspace from workflow");
        assert_eq!(ws.aggregated_state, AgentState::Running);
        assert_eq!(ws.running_count, 1);
        assert_eq!(ws.session_count, 0);
        assert_eq!(ws.worktree_id, "/repo");
        assert_eq!(ws.worktree_path, "/repo");
    }

    #[test]
    fn update_workflow_snapshot_normalizes_worktree_path_without_sessions() {
        let center = mk_center();
        let changes = center.update_workflow_snapshot(
            r"C:\repo\wt",
            "exec-1",
            Some(AgentState::Running),
            5.0,
        );

        let changed = changes
            .workspace
            .expect("workspace from workflow-only snapshot");
        assert_eq!(changed.worktree_id, "C:/repo/wt");
        assert_eq!(changed.worktree_path, "C:/repo/wt");
        assert_eq!(changed.aggregated_state, AgentState::Running);
        assert_eq!(changed.running_count, 1);
        assert_eq!(changed.session_count, 0);

        let stored = center
            .get_workspace(r"C:\repo\wt")
            .expect("raw backslash path resolves to stored workspace");
        assert_eq!(stored, changed);
        assert!(center.get_workspace("C:/repo/wt").is_some());
    }

    #[test]
    fn workflow_step_representative_updates_when_session_starts_streaming() {
        let center = mk_center();
        let queued = workflow_session(
            "step-a",
            "build",
            Some(1),
            StepProgress::Queued,
            AgentState::Done,
        );
        let initial = center.update_session(queued);
        assert_eq!(
            initial.workflow_step_views[0].steps[0].representative,
            RepresentativeStatus::Queued.as_str()
        );
        let initial_version = initial.workflow_step_views[0].version;

        let running = workflow_session(
            "step-a",
            "build",
            Some(1),
            StepProgress::Queued,
            AgentState::Running,
        );
        let changes = center.update_session(running);

        assert_eq!(
            changes.workflow_step_views[0].steps[0].representative,
            RepresentativeStatus::Running.as_str()
        );
        assert!(changes.workflow_step_views[0].version > initial_version);
        assert_eq!(changes.workflow_step_views.len(), 1);
        assert_eq!(changes.workflow_step_views[0].worktree_path, "/repo");
        assert_eq!(changes.workflow_step_views[0].steps.len(), 1);
        assert_eq!(
            changes.workflow_step_views[0].steps[0].representative,
            RepresentativeStatus::Running.as_str()
        );
    }

    #[test]
    fn workflow_step_representative_is_removed_when_session_is_archived() {
        let center = mk_center();
        center.update_session(workflow_session(
            "step-a",
            "build",
            Some(1),
            StepProgress::Queued,
            AgentState::Done,
        ));

        let changes = center.on_session_state_changed("step-a", SessionState::Archived);

        assert_eq!(changes.workflow_step_views.len(), 1);
        assert!(changes.workflow_step_views[0].steps.is_empty());
        assert!(changes.workflow_step_views[0].workflows.is_empty());
        assert!(changes.workflow_step_views[0].version > 0);
        let queried = center.query_worktree_step_statuses("/repo");
        assert!(queried.steps.is_empty());
        assert!(queried.workflows.is_empty());
    }

    #[test]
    fn pending_workflow_projection_applies_to_first_session_status() {
        let center = mk_center();
        let sync_changes = center.sync_workflow_step_session_statuses(
            "/repo",
            "exec-1",
            "running",
            vec![projection(
                Some("step-late"),
                "build",
                Some(2),
                RepresentativeStatus::Running,
                StepProgress::Running,
            )],
        );
        assert!(sync_changes.is_empty());

        let mut initial = mk_session(
            "step-late",
            "/repo",
            TurnPhase::Streaming,
            SessionState::Active,
        );
        initial.last_activity_at = 10.0;
        let changes = center.update_session(initial);

        let session = changes.session.expect("session change emitted");
        assert_eq!(session.workflow_step.as_deref(), Some("build"));
        assert_eq!(session.workflow_execution_state.as_deref(), Some("running"));
        assert_eq!(session.workflow_execution_id.as_deref(), Some("exec-1"));
        assert_eq!(session.workflow_run_index, Some(2));
        assert_eq!(session.workflow_step_progress, Some(StepProgress::Running));
        assert_eq!(changes.workflow_step_views.len(), 1);
        let view = &changes.workflow_step_views[0];
        assert_eq!(view.steps.len(), 1);
        assert_eq!(
            view.steps[0].representative,
            RepresentativeStatus::Running.as_str()
        );
        assert_eq!(view.workflows.len(), 1);
        assert_eq!(
            view.workflows[0].representative,
            RepresentativeStatus::Running.as_str()
        );
        assert_eq!(
            center
                .get_session("step-late")
                .expect("session registered")
                .workflow_execution_id
                .as_deref(),
            Some("exec-1")
        );
    }

    #[test]
    fn pending_workflow_projection_is_removed_when_snapshot_no_longer_references_session() {
        let center = mk_center();
        center.sync_workflow_step_session_statuses(
            "/repo",
            "exec-1",
            "running",
            vec![projection(
                Some("step-late"),
                "build",
                Some(2),
                RepresentativeStatus::Queued,
                StepProgress::Queued,
            )],
        );
        center.sync_workflow_step_session_statuses("/repo", "exec-1", "running", Vec::new());

        let changes = center.update_session(mk_session(
            "step-late",
            "/repo",
            TurnPhase::Idle,
            SessionState::Done,
        ));

        let session = changes.session.expect("session change emitted");
        assert_eq!(session.workflow_step, None);
        assert_eq!(session.workflow_execution_state, None);
        assert_eq!(session.workflow_execution_id, None);
        assert!(changes.workflow_step_views.is_empty());
    }

    #[test]
    fn parallel_step_running_session_dominates_stopped_sessions() {
        let center = mk_center();
        center.update_session(workflow_session(
            "step-a",
            "parallel-review",
            Some(1),
            StepProgress::Completed,
            AgentState::Done,
        ));
        let changes = center.update_session(workflow_session(
            "step-b",
            "parallel-review",
            Some(1),
            StepProgress::Completed,
            AgentState::Running,
        ));

        assert_eq!(
            changes.workflow_step_views[0].steps[0].representative,
            RepresentativeStatus::Running.as_str()
        );
    }

    #[test]
    fn query_worktree_step_statuses_returns_backend_aggregated_view() {
        let center = mk_center();
        center.update_session(workflow_session(
            "step-a",
            "plan",
            Some(1),
            StepProgress::Completed,
            AgentState::Done,
        ));
        center.update_session(workflow_session(
            "step-b",
            "test",
            Some(1),
            StepProgress::WaitingApproval,
            AgentState::Error,
        ));
        center.update_session(SessionStatus {
            chat_session_id: "other-session".to_string(),
            worktree_id: "/other".to_string(),
            worktree_path: "/other".to_string(),
            pty_id: None,
            agent_state: AgentState::Running,
            turn_phase: TurnPhaseRepr::Streaming,
            session_state: SessionState::Active,
            pending_permission: false,
            pending_permission_request: None,
            last_activity_at: 0.0,
            workflow_step: Some("deploy".to_string()),
            workflow_execution_state: Some("running".to_string()),
            workflow_execution_id: Some("exec-other".to_string()),
            workflow_run_index: Some(1),
            workflow_step_progress: Some(StepProgress::Running),
        });

        let view = center.query_worktree_step_statuses("/repo");

        assert_eq!(view.worktree_path, "/repo");
        assert!(view.version > 0);
        assert_eq!(view.steps.len(), 2);
        assert!(view.steps.iter().any(|step| {
            step.execution_id == "exec-1"
                && step.step_name == "plan"
                && step.representative == RepresentativeStatus::Completed.as_str()
        }));
        assert!(view.steps.iter().any(|step| {
            step.execution_id == "exec-1"
                && step.step_name == "test"
                && step.representative == RepresentativeStatus::Error.as_str()
        }));
        assert_eq!(view.workflows.len(), 1);
        assert_eq!(view.workflows[0].execution_id, "exec-1");
        assert_eq!(
            view.workflows[0].representative,
            RepresentativeStatus::Error.as_str()
        );

        let other = center.query_worktree_step_statuses("/other");
        assert!(other.version > 0);
        assert_eq!(other.steps.len(), 1);
        assert_eq!(other.steps[0].execution_id, "exec-other");
    }

    #[test]
    fn query_worktree_step_statuses_keeps_monotonic_version_after_empty_snapshot() {
        let center = mk_center();
        let initial = center.update_session(workflow_session(
            "step-a",
            "build",
            Some(1),
            StepProgress::Queued,
            AgentState::Done,
        ));
        let initial_version = initial.workflow_step_views[0].version;

        let removed = center.on_session_state_changed("step-a", SessionState::Archived);

        assert_eq!(removed.workflow_step_views.len(), 1);
        assert!(removed.workflow_step_views[0].steps.is_empty());
        assert!(removed.workflow_step_views[0].workflows.is_empty());
        assert!(removed.workflow_step_views[0].version > initial_version);

        let queried = center.query_worktree_step_statuses("/repo");
        assert!(queried.steps.is_empty());
        assert!(queried.workflows.is_empty());
        assert_eq!(queried.version, removed.workflow_step_views[0].version);
    }

    #[test]
    fn workflow_step_status_event_view_matches_query_snapshot() {
        let center = mk_center();
        let changes = center.update_session(workflow_session(
            "step-a",
            "build",
            Some(1),
            StepProgress::Running,
            AgentState::Running,
        ));

        assert_eq!(changes.workflow_step_views.len(), 1);
        let emitted = &changes.workflow_step_views[0];
        let queried = center.query_worktree_step_statuses("/repo");

        assert_eq!(emitted, &queried);
    }

    #[test]
    fn workflow_step_status_view_refresh_does_not_rewind_stored_snapshot() {
        let key = WorkflowStepKey {
            worktree_path: "/repo".to_string(),
            execution_id: "exec-1".to_string(),
            step_name: "build".to_string(),
            run_index: Some(1),
        };
        let mut state = WorkflowStepStatusState::default();
        state.steps.insert(
            key.clone(),
            WorkflowStatusEntry {
                representative: RepresentativeStatus::Running,
            },
        );
        let initial = AgentStatusCenter::update_worktree_step_status_view(&mut state, "/repo", 10);

        state.steps.insert(
            key,
            WorkflowStatusEntry {
                representative: RepresentativeStatus::Error,
            },
        );
        let stale = AgentStatusCenter::update_worktree_step_status_view(&mut state, "/repo", 5);

        assert_eq!(stale, initial);
        assert_eq!(state.views.get("/repo"), Some(&initial));
    }

    #[test]
    fn workflow_representative_aggregates_live_steps_with_snapshot_baselines() {
        let center = mk_center();
        center.sync_workflow_step_session_statuses(
            "/repo",
            "exec-1",
            "running",
            vec![
                projection(
                    Some("step-live"),
                    "live",
                    Some(1),
                    RepresentativeStatus::Queued,
                    StepProgress::Queued,
                ),
                projection(
                    None,
                    "failed-history",
                    Some(1),
                    RepresentativeStatus::Failed,
                    StepProgress::Failed,
                ),
            ],
        );

        let live_waiting = workflow_session(
            "step-live",
            "live",
            Some(1),
            StepProgress::Queued,
            AgentState::Waiting,
        );
        let changes = center.update_session(live_waiting);

        assert_eq!(
            changes.workflow_step_views[0].steps[0].representative,
            RepresentativeStatus::Waiting.as_str()
        );
        assert_eq!(
            changes.workflow_step_views[0].workflows[0].representative,
            RepresentativeStatus::Failed.as_str()
        );
    }

    #[test]
    fn sync_workflow_step_statuses_returns_current_view_after_live_reaggregation() {
        let center = mk_center();
        center.update_session(workflow_session(
            "step-live",
            "live",
            Some(1),
            StepProgress::Queued,
            AgentState::Done,
        ));

        let changes = center.sync_workflow_step_session_statuses(
            "/repo",
            "exec-1",
            "running",
            vec![projection(
                Some("step-live"),
                "live",
                Some(1),
                RepresentativeStatus::Queued,
                StepProgress::Running,
            )],
        );

        let emitted_views = changes
            .iter()
            .flat_map(|change| change.workflow_step_views.iter())
            .collect::<Vec<_>>();
        assert_eq!(emitted_views.len(), 1);
        let emitted = emitted_views[0];
        let queried = center.query_worktree_step_statuses("/repo");
        assert_eq!(emitted, &queried);
        assert_eq!(
            emitted.steps[0].representative,
            RepresentativeStatus::Running.as_str()
        );
    }

    #[test]
    fn workflow_representative_clears_when_last_live_step_is_removed() {
        let center = mk_center();
        center.sync_workflow_step_session_statuses(
            "/repo",
            "exec-1",
            "running",
            vec![projection(
                Some("step-live"),
                "live",
                Some(1),
                RepresentativeStatus::Queued,
                StepProgress::Queued,
            )],
        );
        center.update_session(workflow_session(
            "step-live",
            "live",
            Some(1),
            StepProgress::Queued,
            AgentState::Running,
        ));

        let changes = center.on_session_state_changed("step-live", SessionState::Archived);

        assert_eq!(changes.workflow_step_views.len(), 1);
        assert!(changes.workflow_step_views[0].steps.is_empty());
        assert!(changes.workflow_step_views[0].workflows.is_empty());
    }
}
