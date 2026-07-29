pub use crate::domain::agent_session::value_objects::TurnPhase;
use crate::domain::repository::normalize_repo_path;
use crate::domain::workflow::services::node_session_projection::NodeSessionProjection;
use crate::domain::workflow::status_aggregation::{
    aggregate_representative_statuses, session_result, NodeProgress, RepresentativeStatus,
    SessionActivity,
};
use crate::usecase::agent_session::session::{PermissionRequestMsg, SessionState};
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
pub enum SessionNoticeKind {
    PersistFailure,
    EventLogRecovered,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNotice {
    pub session_id: String,
    pub kind: SessionNoticeKind,
    pub message: String,
    pub created_at: f64,
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
    pub pending_permission_request: Option<PermissionRequestMsg>,
    pub last_activity_at: f64,
    pub workflow_node: Option<String>,
    pub workflow_execution_status: Option<String>,
    pub workflow_execution_id: Option<String>,
    pub node_execution_id: Option<String>,
    pub workflow_attempt: Option<u32>,
    pub notice: Option<SessionNotice>,
    #[serde(skip_serializing)]
    pub workflow_node_progress: Option<NodeProgress>,
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
struct WorkflowNodeExecutionKey {
    worktree_path: String,
    execution_id: String,
    node_execution_id: String,
    node_name: String,
    attempt: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowNodeRepresentative {
    pub execution_id: String,
    pub node_execution_id: String,
    pub node_name: String,
    pub attempt: Option<u32>,
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
pub struct WorktreeNodeStatusView {
    pub worktree_path: String,
    pub version: u64,
    pub node_executions: Vec<WorkflowNodeRepresentative>,
    pub workflow_executions: Vec<WorkflowRepresentative>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WorkflowStatusEntry {
    representative: RepresentativeStatus,
}

#[derive(Debug, Default)]
struct WorkflowNodeStatusState {
    node_executions: HashMap<WorkflowNodeExecutionKey, WorkflowStatusEntry>,
    baselines: HashMap<WorkflowNodeExecutionKey, RepresentativeStatus>,
    views: HashMap<String, WorktreeNodeStatusView>,
}

#[derive(Debug, Default)]
struct WorkflowNodeStatusUpdate {
    views: Vec<WorktreeNodeStatusView>,
}

impl WorkflowNodeStatusUpdate {
    fn is_empty(&self) -> bool {
        self.views.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkflowNodeSessionStatusInput {
    node_execution_id: String,
    node_name: String,
    attempt: Option<u32>,
    progress: NodeProgress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingWorkflowNodeSessionStatus {
    worktree_path: String,
    execution_id: String,
    workflow_execution_status: String,
    input: WorkflowNodeSessionStatusInput,
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
    pub workflow_node_views: Vec<WorktreeNodeStatusView>,
}

impl AgentStatusChanges {
    pub fn is_empty(&self) -> bool {
        self.session.is_none()
            && self.workspace.is_none()
            && self.agent_state.is_none()
            && self.workflow_node_views.is_empty()
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
    notices: RwLock<HashMap<String, SessionNotice>>,
    #[cfg(test)]
    update_session_notice_sync_hook:
        RwLock<Option<std::sync::Arc<dyn Fn() + Send + Sync + 'static>>>,
    session_state_revisions: RwLock<HashMap<String, u64>>,
    workspaces: RwLock<HashMap<String, WorkspaceStatus>>,
    workflow_node_status: RwLock<WorkflowNodeStatusState>,
    pending_workflow_node_sessions: RwLock<HashMap<String, PendingWorkflowNodeSessionStatus>>,
    workflow_node_status_version: AtomicU64,
    /// key: worktree_path（= worktree_id と同値で運用）
    workflows: RwLock<HashMap<String, WorkflowAggSnapshot>>,
}

impl AgentStatusCenter {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            notices: RwLock::new(HashMap::new()),
            #[cfg(test)]
            update_session_notice_sync_hook: RwLock::new(None),
            session_state_revisions: RwLock::new(HashMap::new()),
            workspaces: RwLock::new(HashMap::new()),
            workflow_node_status: RwLock::new(WorkflowNodeStatusState::default()),
            pending_workflow_node_sessions: RwLock::new(HashMap::new()),
            workflow_node_status_version: AtomicU64::new(0),
            workflows: RwLock::new(HashMap::new()),
        }
    }

    fn canonical_worktree_path(path: &str) -> String {
        normalize_repo_path(path)
    }

    fn normalize_session_paths(status: &mut SessionStatus) {
        status.worktree_id = Self::canonical_worktree_path(&status.worktree_id);
        status.worktree_path = Self::canonical_worktree_path(&status.worktree_path);
    }

    /// `RuntimeExecutionState` を Workspace 集約に寄与する `AgentState` にマップする。
    /// `Aborted` はユーザー意図で停止したものとして「集約対象外（None）」扱いにする。
    pub fn workflow_execution_status_to_agent_state(
        state: &crate::domain::workflow::RuntimeExecutionState,
    ) -> Option<AgentState> {
        use crate::domain::workflow::RuntimeExecutionState;
        match state {
            RuntimeExecutionState::Running => Some(AgentState::Running),
            RuntimeExecutionState::WaitingApproval => Some(AgentState::Waiting),
            RuntimeExecutionState::Failed { .. } => Some(AgentState::Error),
            RuntimeExecutionState::Completed => Some(AgentState::Done),
            RuntimeExecutionState::Aborted | RuntimeExecutionState::Interrupted => None,
        }
    }

    /// AgentState は turn_phase / session_state から派生する。
    /// Spec line 110 の集約規則対応 (Error > Idle > Active > Done) に揃え、
    /// turn_phase が Idle の場合は session_state=Error → Error、Idle → Waiting、
    /// それ以外 → Done として個別 Session の agent_state を決定する。
    pub fn derive_agent_state(turn_phase: TurnPhase, session_state: SessionState) -> AgentState {
        match crate::domain::agent_session::services::classify_session_activity(
            turn_phase,
            session_state,
        ) {
            crate::domain::agent_session::services::SessionActivity::Running => AgentState::Running,
            crate::domain::agent_session::services::SessionActivity::Waiting => AgentState::Waiting,
            crate::domain::agent_session::services::SessionActivity::Error => AgentState::Error,
            crate::domain::agent_session::services::SessionActivity::Done => AgentState::Done,
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
            && a.workflow_node == b.workflow_node
            && a.workflow_execution_status == b.workflow_execution_status
            && a.workflow_execution_id == b.workflow_execution_id
            && a.node_execution_id == b.node_execution_id
            && a.workflow_attempt == b.workflow_attempt
            && a.notice == b.notice
            && a.workflow_node_progress == b.workflow_node_progress
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
    /// （`RuntimeExecutionState` を `AgentState` にマップしたもの）。
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
        state.is_open()
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

    fn session_workflow_node_key(status: &SessionStatus) -> Option<WorkflowNodeExecutionKey> {
        Some(WorkflowNodeExecutionKey {
            worktree_path: status.worktree_path.clone(),
            execution_id: status.workflow_execution_id.clone()?,
            node_execution_id: status.node_execution_id.clone()?,
            node_name: status.workflow_node.clone()?,
            attempt: status.workflow_attempt,
        })
    }

    fn next_workflow_node_status_version(&self) -> u64 {
        self.workflow_node_status_version
            .fetch_add(1, Ordering::Relaxed)
            + 1
    }

    fn is_same_workflow(left: &WorkflowNodeExecutionKey, right: &WorkflowNodeExecutionKey) -> bool {
        left.worktree_path == right.worktree_path && left.execution_id == right.execution_id
    }

    fn workflow_has_live_status(
        workflow_key: &WorkflowNodeExecutionKey,
        live_nodes: &HashMap<WorkflowNodeExecutionKey, WorkflowStatusEntry>,
    ) -> bool {
        live_nodes
            .keys()
            .any(|key| Self::is_same_workflow(key, workflow_key))
    }

    fn workflow_representative_for_key(
        workflow_key: &WorkflowNodeExecutionKey,
        live_nodes: &HashMap<WorkflowNodeExecutionKey, WorkflowStatusEntry>,
        baselines: &HashMap<WorkflowNodeExecutionKey, RepresentativeStatus>,
    ) -> Option<RepresentativeStatus> {
        if !Self::workflow_has_live_status(workflow_key, live_nodes) {
            return None;
        }

        let baseline_statuses = baselines
            .iter()
            .filter(|(key, _)| Self::is_same_workflow(key, workflow_key))
            .map(|(key, baseline)| {
                live_nodes
                    .get(key)
                    .map(|entry| entry.representative)
                    .unwrap_or(*baseline)
            });
        let live_only_statuses = live_nodes
            .iter()
            .filter(|(key, _)| {
                Self::is_same_workflow(key, workflow_key) && !baselines.contains_key(*key)
            })
            .map(|(_, entry)| entry.representative);

        aggregate_representative_statuses(baseline_statuses.chain(live_only_statuses))
    }

    fn worktree_node_status_view_from_maps(
        worktree_path: &str,
        version: u64,
        node_statuses: &HashMap<WorkflowNodeExecutionKey, WorkflowStatusEntry>,
        baselines: &HashMap<WorkflowNodeExecutionKey, RepresentativeStatus>,
    ) -> WorktreeNodeStatusView {
        let worktree_path = Self::canonical_worktree_path(worktree_path);
        let mut node_executions = Vec::new();
        let mut workflow_keys: HashMap<String, WorkflowNodeExecutionKey> = HashMap::new();

        for (key, entry) in node_statuses {
            if key.worktree_path != worktree_path {
                continue;
            }
            node_executions.push(WorkflowNodeRepresentative {
                execution_id: key.execution_id.clone(),
                node_execution_id: key.node_execution_id.clone(),
                node_name: key.node_name.clone(),
                attempt: key.attempt,
                representative: entry.representative.as_str().to_string(),
            });
            workflow_keys
                .entry(key.execution_id.clone())
                .or_insert_with(|| key.clone());
        }

        node_executions.sort_by(|left, right| {
            (
                &left.execution_id,
                &left.node_name,
                left.attempt,
                &left.node_execution_id,
            )
                .cmp(&(
                    &right.execution_id,
                    &right.node_name,
                    right.attempt,
                    &right.node_execution_id,
                ))
        });

        let mut workflow_executions = workflow_keys
            .into_values()
            .filter_map(|key| {
                Self::workflow_representative_for_key(&key, node_statuses, baselines).map(
                    |representative| WorkflowRepresentative {
                        execution_id: key.execution_id,
                        representative: representative.as_str().to_string(),
                    },
                )
            })
            .collect::<Vec<_>>();
        workflow_executions.sort_by(|left, right| left.execution_id.cmp(&right.execution_id));

        WorktreeNodeStatusView {
            worktree_path,
            version,
            node_executions,
            workflow_executions,
        }
    }

    fn update_worktree_node_status_view(
        state: &mut WorkflowNodeStatusState,
        worktree_path: &str,
        version: u64,
    ) -> WorktreeNodeStatusView {
        let worktree_path = Self::canonical_worktree_path(worktree_path);
        if let Some(view) = state.views.get(&worktree_path) {
            if version < view.version {
                return view.clone();
            }
        }
        let view = Self::worktree_node_status_view_from_maps(
            &worktree_path,
            version,
            &state.node_executions,
            &state.baselines,
        );
        state.views.insert(worktree_path, view.clone());
        view
    }

    fn worktree_node_status_view(&self, worktree_path: &str) -> WorktreeNodeStatusView {
        let worktree_path = Self::canonical_worktree_path(worktree_path);
        let state = self.workflow_node_status.read();
        state.views.get(&worktree_path).cloned().unwrap_or_else(|| {
            Self::worktree_node_status_view_from_maps(
                &worktree_path,
                0,
                &state.node_executions,
                &state.baselines,
            )
        })
    }

    fn strip_worktree_node_views(changes: &mut [AgentStatusChanges], worktree_path: &str) -> bool {
        let worktree_path = Self::canonical_worktree_path(worktree_path);
        let mut removed = false;
        for change in changes {
            let before = change.workflow_node_views.len();
            change
                .workflow_node_views
                .retain(|view| view.worktree_path != worktree_path.as_str());
            removed |= change.workflow_node_views.len() != before;
        }
        removed
    }

    fn update_workflow_node_baselines(
        &self,
        worktree_path: &str,
        execution_id: &str,
        projections: &[NodeSessionProjection],
    ) -> WorkflowNodeStatusUpdate {
        let mut grouped: HashMap<WorkflowNodeExecutionKey, Vec<RepresentativeStatus>> =
            HashMap::new();
        for projection in projections {
            let Some(node_execution_id) = projection.node_execution_id.clone() else {
                continue;
            };
            let key = WorkflowNodeExecutionKey {
                worktree_path: worktree_path.to_string(),
                execution_id: execution_id.to_string(),
                node_execution_id,
                node_name: projection.node_name.clone(),
                attempt: projection.attempt,
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

        let mut state = self.workflow_node_status.write();
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
            return WorkflowNodeStatusUpdate::default();
        }

        let has_visible_workflow = state
            .node_executions
            .keys()
            .any(|key| key.worktree_path == worktree_path && key.execution_id == execution_id);
        if !has_visible_workflow {
            return WorkflowNodeStatusUpdate::default();
        }

        let version = self.next_workflow_node_status_version();
        let view = Self::update_worktree_node_status_view(&mut state, worktree_path, version);
        WorkflowNodeStatusUpdate { views: vec![view] }
    }

    fn reaggregate_workflow_nodes_for_worktree(
        &self,
        worktree_path: &str,
    ) -> WorkflowNodeStatusUpdate {
        let mut grouped: HashMap<WorkflowNodeExecutionKey, Vec<RepresentativeStatus>> =
            HashMap::new();
        let sessions = self.sessions.read();
        for status in sessions.values() {
            if status.worktree_path != worktree_path {
                continue;
            }
            if !Self::is_live_session_state(&status.session_state) {
                continue;
            }
            let Some(key) = Self::session_workflow_node_key(status) else {
                continue;
            };
            let Some(node_progress) = status.workflow_node_progress else {
                continue;
            };
            grouped.entry(key).or_default().push(session_result(
                node_progress,
                SessionActivity::from(status.agent_state.clone()),
            ));
        }

        let next_nodes = grouped
            .into_iter()
            .filter_map(|(key, statuses)| {
                aggregate_representative_statuses(statuses)
                    .map(|representative| (key, representative))
            })
            .collect::<HashMap<_, _>>();

        let mut state = self.workflow_node_status.write();
        let mut changed = false;
        let previous_node_keys = state
            .node_executions
            .keys()
            .filter(|key| key.worktree_path == worktree_path)
            .cloned()
            .collect::<Vec<_>>();
        for key in &previous_node_keys {
            if next_nodes.contains_key(key) {
                continue;
            }
            changed = true;
        }
        for (key, representative) in &next_nodes {
            let previous = state
                .node_executions
                .get(key)
                .map(|entry| entry.representative);
            if previous != Some(*representative) {
                changed = true;
            }
        }
        if !changed {
            return WorkflowNodeStatusUpdate::default();
        }

        let version = self.next_workflow_node_status_version();
        for key in previous_node_keys {
            if !next_nodes.contains_key(&key) {
                state.node_executions.remove(&key);
            }
        }
        for (key, representative) in next_nodes {
            state
                .node_executions
                .insert(key, WorkflowStatusEntry { representative });
        }

        let view = Self::update_worktree_node_status_view(&mut state, worktree_path, version);
        WorkflowNodeStatusUpdate { views: vec![view] }
    }

    fn apply_workflow_node_session_status(
        status: &mut SessionStatus,
        execution_id: &str,
        workflow_execution_status: &str,
        input: &WorkflowNodeSessionStatusInput,
    ) {
        status.workflow_node = Some(input.node_name.clone());
        status.workflow_execution_status = Some(workflow_execution_status.to_string());
        status.workflow_execution_id = Some(execution_id.to_string());
        status.node_execution_id = Some(input.node_execution_id.clone());
        status.workflow_attempt = input.attempt;
        status.workflow_node_progress = Some(input.progress);
    }

    fn clear_workflow_node_session_status(status: &mut SessionStatus) {
        status.workflow_node = None;
        status.workflow_execution_status = None;
        status.workflow_execution_id = None;
        status.node_execution_id = None;
        status.workflow_attempt = None;
        status.workflow_node_progress = None;
    }

    fn apply_pending_workflow_node_session_status(&self, status: &mut SessionStatus) {
        let pending = {
            let mut pending = self.pending_workflow_node_sessions.write();
            pending.remove(&status.chat_session_id)
        };
        let Some(pending) = pending else {
            return;
        };
        if pending.worktree_path != status.worktree_path || status.workflow_execution_id.is_some() {
            return;
        }
        Self::apply_workflow_node_session_status(
            status,
            &pending.execution_id,
            &pending.workflow_execution_status,
            &pending.input,
        );
    }

    fn update_pending_workflow_node_session_statuses(
        &self,
        worktree_path: &str,
        execution_id: &str,
        workflow_execution_status: &str,
        inputs: &HashMap<String, WorkflowNodeSessionStatusInput>,
        existing_session_ids: &HashSet<String>,
    ) {
        let mut pending = self.pending_workflow_node_sessions.write();
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
                PendingWorkflowNodeSessionStatus {
                    worktree_path: worktree_path.to_string(),
                    execution_id: execution_id.to_string(),
                    workflow_execution_status: workflow_execution_status.to_string(),
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
        {
            // notices is the source of truth. Hold its read lock through the session
            // insert so record/clear always observes the same notices -> sessions order.
            let notices = self.notices.read();
            status.notice = notices.get(&status.chat_session_id).cloned();
            let prev = self.sessions.read().get(&status.chat_session_id).cloned();
            if prev.is_none() {
                self.apply_pending_workflow_node_session_status(&mut status);
            }

            // 1. dedup
            if let Some(prev) = prev {
                if Self::is_session_state_equivalent(&prev, &status) {
                    return AgentStatusChanges::default();
                }
            }

            #[cfg(test)]
            if let Some(hook) = self.update_session_notice_sync_hook.read().clone() {
                hook();
            }

            // 2. sessions マップ反映
            let mut sessions = self.sessions.write();
            sessions.insert(status.chat_session_id.clone(), status.clone());
        }

        let worktree_id = status.worktree_id.clone();
        let worktree_path = status.worktree_path.clone();
        let last_activity_at = status.last_activity_at;
        let chat_session_id = status.chat_session_id.clone();
        let pty_id = status.pty_id.clone();
        let agent_state = status.agent_state.clone();

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
        let workflow_node_update = self.reaggregate_workflow_nodes_for_worktree(&worktree_path);

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
            workflow_node_views: workflow_node_update.views,
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

    pub fn sync_workflow_node_session_statuses(
        &self,
        worktree_path: &str,
        execution_id: &str,
        workflow_execution_status: &str,
        projections: Vec<NodeSessionProjection>,
    ) -> Vec<AgentStatusChanges> {
        let worktree_path = Self::canonical_worktree_path(worktree_path);
        let baseline_update =
            self.update_workflow_node_baselines(&worktree_path, execution_id, &projections);
        let inputs = projections
            .into_iter()
            .filter_map(|projection| {
                let session_id = projection.session_id?;
                let node_execution_id = projection.node_execution_id?;
                Some((
                    session_id,
                    WorkflowNodeSessionStatusInput {
                        node_execution_id,
                        node_name: projection.node_name,
                        attempt: projection.attempt,
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
        self.update_pending_workflow_node_session_statuses(
            &worktree_path,
            execution_id,
            workflow_execution_status,
            &inputs,
            &existing_session_ids,
        );

        for status in sessions {
            if status.worktree_path != worktree_path {
                continue;
            }
            let mut updated = status;
            if let Some(input) = inputs.get(&updated.chat_session_id) {
                Self::apply_workflow_node_session_status(
                    &mut updated,
                    execution_id,
                    workflow_execution_status,
                    input,
                );
            } else if updated.workflow_execution_id.as_deref() == Some(execution_id) {
                Self::clear_workflow_node_session_status(&mut updated);
            } else {
                continue;
            }

            let update_changes = self.update_session(updated);
            if !update_changes.is_empty() {
                changes.push(update_changes);
            }
        }

        let had_live_node_view = Self::strip_worktree_node_views(&mut changes, &worktree_path);
        if had_live_node_view || !baseline_update.is_empty() {
            let latest_view = self.worktree_node_status_view(&worktree_path);
            changes.push(AgentStatusChanges {
                workflow_node_views: vec![latest_view],
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

    /// SessionStore からの state projection 変更通知を受け取り、保持している
    /// `SessionStatus` の `session_state` を最新化した上で再集約する。
    /// 同一 state の通知は backend read model 再取得用に session snapshot を再配信する。
    /// Closed/Archived への遷移は `aggregate` 段階で集約対象から外れ、
    /// 復帰時は再び集約対象に戻る。
    pub fn on_session_state_changed(
        &self,
        chat_session_id: &str,
        new_state: SessionState,
        state_revision: u64,
    ) -> AgentStatusChanges {
        let mut revisions = self.session_state_revisions.write();
        if revisions
            .get(chat_session_id)
            .is_some_and(|current| state_revision < *current)
        {
            return AgentStatusChanges::default();
        }
        revisions.insert(chat_session_id.to_string(), state_revision);
        let existing = self.sessions.read().get(chat_session_id).cloned();
        let Some(existing) = existing else {
            return AgentStatusChanges::default();
        };
        if existing.session_state == new_state {
            return AgentStatusChanges {
                session: Some(existing),
                ..Default::default()
            };
        }
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
        let normalize_to_idle = new_state.normalizes_turn_phase_to_idle();
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
        let agent_state = Self::derive_agent_state(turn_phase_repr.into(), new_state);
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

    fn replace_session_notice(
        &self,
        session_id: &str,
        notice: Option<SessionNotice>,
        expected_kind: Option<SessionNoticeKind>,
    ) -> AgentStatusChanges {
        let mut notices = self.notices.write();
        if expected_kind.is_some_and(|kind| {
            !notices
                .get(session_id)
                .is_some_and(|notice| notice.kind == kind)
        }) {
            return AgentStatusChanges::default();
        }
        match &notice {
            Some(notice) => {
                notices.insert(session_id.to_string(), notice.clone());
            }
            None => {
                notices.remove(session_id);
            }
        }
        let session = self.sessions.write().get_mut(session_id).map(|status| {
            status.notice = notice;
            status.clone()
        });
        AgentStatusChanges {
            session,
            ..Default::default()
        }
    }

    pub fn record_session_notice(&self, notice: SessionNotice) -> AgentStatusChanges {
        let session_id = notice.session_id.clone();
        self.replace_session_notice(&session_id, Some(notice), None)
    }

    pub fn clear_session_notice(
        &self,
        session_id: &str,
        kind: SessionNoticeKind,
    ) -> AgentStatusChanges {
        self.replace_session_notice(session_id, None, Some(kind))
    }

    #[cfg(test)]
    fn set_update_session_notice_sync_hook_for_test(
        &self,
        hook: std::sync::Arc<dyn Fn() + Send + Sync + 'static>,
    ) {
        *self.update_session_notice_sync_hook.write() = Some(hook);
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

    pub fn query_worktree_node_statuses(&self, worktree_path: &str) -> WorktreeNodeStatusView {
        self.worktree_node_status_view(worktree_path)
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
            workflow_node: None,
            workflow_execution_status: None,
            workflow_execution_id: None,
            node_execution_id: None,
            workflow_attempt: None,
            notice: None,
            workflow_node_progress: None,
        }
    }

    fn workflow_session(
        id: &str,
        node_name: &str,
        attempt: Option<u32>,
        progress: NodeProgress,
        agent_state: AgentState,
    ) -> SessionStatus {
        let mut session = mk_session("unused", "/repo", TurnPhase::Idle, SessionState::Done);
        session.chat_session_id = id.to_string();
        session.agent_state = agent_state;
        session.workflow_execution_id = Some("exec-1".to_string());
        session.node_execution_id = Some(id.to_string());
        session.workflow_node = Some(node_name.to_string());
        session.workflow_attempt = attempt;
        session.workflow_node_progress = Some(progress);
        session
    }

    fn permission_request_fixture() -> PermissionRequestMsg {
        PermissionRequestMsg {
            id: "req-1".to_string(),
            tool_use_id: Some("toolu-1".to_string()),
            tool_name: "Edit".to_string(),
            kind: crate::usecase::agent_session::session::PermissionRequestKindMsg::ToolApproval,
            input: Some(serde_json::json!({})),
            plan: None,
            allowed_prompts: Vec::new(),
            questions: Vec::new(),
            title: None,
            display_name: None,
            description: None,
            decision_reason: None,
        }
    }

    fn projection(
        session_id: Option<&str>,
        node_name: &str,
        attempt: Option<u32>,
        representative: RepresentativeStatus,
        progress: NodeProgress,
    ) -> NodeSessionProjection {
        NodeSessionProjection {
            node_execution_id: Some(format!("{node_name}-{}", attempt.unwrap_or(1))),
            session_id: session_id.map(str::to_string),
            node_name: node_name.to_string(),
            attempt,
            group_node_name: node_name.to_string(),
            group_attempt: attempt,
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
    fn update_session_trims_trailing_worktree_slash_before_storing() {
        let center = AgentStatusCenter::new();
        let changes = center.update_session(mk_session(
            "s1",
            "/repo/wt/",
            TurnPhase::Streaming,
            SessionState::Active,
        ));

        let session = changes.session.unwrap();
        assert_eq!(session.worktree_id, "/repo/wt");
        assert_eq!(session.worktree_path, "/repo/wt");
        assert!(center.get_workspace("/repo/wt").is_some());
        assert!(center.get_workspace("/repo/wt/").is_some());
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
    fn session_status_serialization_exposes_canonical_execution_and_node_ids_only() {
        let status = workflow_session(
            "step-a",
            "build",
            Some(3),
            NodeProgress::Running,
            AgentState::Running,
        );

        let value = serde_json::to_value(status).expect("session status serializes");

        assert!(value.get("workflow_node").is_some());
        assert!(value.get("workflow_execution_status").is_some());
        assert_eq!(value["workflow_execution_id"], "exec-1");
        assert_eq!(value["node_execution_id"], "step-a");
        assert_eq!(value["workflow_attempt"], 3);
        assert!(value.get("workflow_node_execution_id").is_none());
        assert!(value.get("workflow_node_progress").is_none());
    }

    #[test]
    fn session_notice_serialization_uses_frontend_wire_shape() {
        for (kind, expected_kind) in [
            (SessionNoticeKind::PersistFailure, "persist_failure"),
            (SessionNoticeKind::EventLogRecovered, "event_log_recovered"),
        ] {
            let value = serde_json::to_value(SessionNotice {
                session_id: "session-1".to_string(),
                kind,
                message: "Notice message".to_string(),
                created_at: 42.0,
            })
            .expect("session notice serializes");

            assert_eq!(
                value,
                serde_json::json!({
                    "sessionId": "session-1",
                    "kind": expected_kind,
                    "message": "Notice message",
                    "createdAt": 42.0,
                })
            );
        }
    }

    #[test]
    fn session_status_serialization_nests_notice_wire_shape() {
        let mut status = mk_session("session-1", "/repo", TurnPhase::Idle, SessionState::Idle);
        status.notice = Some(SessionNotice {
            session_id: "session-1".to_string(),
            kind: SessionNoticeKind::EventLogRecovered,
            message: "Recovered".to_string(),
            created_at: 7.0,
        });

        let value = serde_json::to_value(status).expect("session status serializes");

        assert_eq!(
            value["notice"],
            serde_json::json!({
                "sessionId": "session-1",
                "kind": "event_log_recovered",
                "message": "Recovered",
                "createdAt": 7.0,
            })
        );
    }

    #[test]
    fn session_notice_is_retained_in_status_snapshot() {
        let center = mk_center();
        center.update_session(mk_session(
            "session-1",
            "/repo",
            TurnPhase::Idle,
            SessionState::Idle,
        ));
        center.record_session_notice(SessionNotice {
            session_id: "session-1".to_string(),
            kind: SessionNoticeKind::PersistFailure,
            message: "Persistence failed".to_string(),
            created_at: 42.0,
        });

        let status = center.get_session("session-1").expect("session status");

        assert_eq!(
            status.notice,
            Some(SessionNotice {
                session_id: "session-1".to_string(),
                kind: SessionNoticeKind::PersistFailure,
                message: "Persistence failed".to_string(),
                created_at: 42.0,
            })
        );
    }

    #[test]
    fn session_notice_recorded_before_status_is_applied_to_first_snapshot() {
        let center = mk_center();
        center.record_session_notice(SessionNotice {
            session_id: "session-1".to_string(),
            kind: SessionNoticeKind::EventLogRecovered,
            message: "Recovered".to_string(),
            created_at: 7.0,
        });

        let changes = center.update_session(mk_session(
            "session-1",
            "/repo",
            TurnPhase::Idle,
            SessionState::Idle,
        ));

        assert_eq!(
            changes
                .session
                .and_then(|status| status.notice)
                .map(|notice| notice.kind),
            Some(SessionNoticeKind::EventLogRecovered)
        );
    }

    #[test]
    fn session_notice_clear_updates_backend_snapshot() {
        let center = mk_center();
        center.update_session(mk_session(
            "session-1",
            "/repo",
            TurnPhase::Idle,
            SessionState::Idle,
        ));
        center.record_session_notice(SessionNotice {
            session_id: "session-1".to_string(),
            kind: SessionNoticeKind::PersistFailure,
            message: "Persistence failed".to_string(),
            created_at: 42.0,
        });

        let changes = center.clear_session_notice("session-1", SessionNoticeKind::PersistFailure);

        assert_eq!(
            changes
                .session
                .as_ref()
                .and_then(|status| status.notice.as_ref()),
            None
        );
        assert_eq!(
            center
                .get_session("session-1")
                .and_then(|status| status.notice),
            None
        );
    }

    #[test]
    fn session_notice_clear_keeps_a_different_notice_kind() {
        let center = mk_center();
        center.update_session(mk_session(
            "session-1",
            "/repo",
            TurnPhase::Idle,
            SessionState::Idle,
        ));
        let notice = SessionNotice {
            session_id: "session-1".to_string(),
            kind: SessionNoticeKind::EventLogRecovered,
            message: "Recovered".to_string(),
            created_at: 7.0,
        };
        center.record_session_notice(notice.clone());

        let changes = center.clear_session_notice("session-1", SessionNoticeKind::PersistFailure);

        assert_eq!(changes, AgentStatusChanges::default());
        assert_eq!(
            center
                .get_session("session-1")
                .and_then(|status| status.notice),
            Some(notice)
        );
    }

    #[test]
    fn update_session_and_record_notice_keep_both_maps_consistent() {
        let center = std::sync::Arc::new(mk_center());
        center.update_session(mk_session(
            "session-1",
            "/repo",
            TurnPhase::Idle,
            SessionState::Idle,
        ));
        let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
        let release = std::sync::Arc::new(std::sync::Barrier::new(2));
        center.set_update_session_notice_sync_hook_for_test({
            let entered = entered.clone();
            let release = release.clone();
            std::sync::Arc::new(move || {
                entered.wait();
                release.wait();
            })
        });
        let updater = {
            let center = center.clone();
            std::thread::spawn(move || {
                let mut status =
                    mk_session("session-1", "/repo", TurnPhase::Idle, SessionState::Idle);
                status.pty_id = Some("pty-1".to_string());
                center.update_session(status);
            })
        };
        entered.wait();
        let notice = SessionNotice {
            session_id: "session-1".to_string(),
            kind: SessionNoticeKind::PersistFailure,
            message: "Persistence failed".to_string(),
            created_at: 42.0,
        };
        let (completed_tx, completed_rx) = std::sync::mpsc::channel();
        let recorder = {
            let center = center.clone();
            let notice = notice.clone();
            std::thread::spawn(move || {
                center.record_session_notice(notice);
                completed_tx.send(()).unwrap();
            })
        };

        let record_was_blocked = completed_rx
            .recv_timeout(std::time::Duration::from_millis(50))
            .is_err();
        release.wait();
        updater.join().unwrap();
        recorder.join().unwrap();

        assert!(record_was_blocked);
        assert_eq!(center.notices.read().get("session-1"), Some(&notice));
        assert_eq!(
            center
                .sessions
                .read()
                .get("session-1")
                .and_then(|status| status.notice.as_ref()),
            Some(&notice)
        );
    }

    #[test]
    fn update_session_and_clear_notice_keep_both_maps_consistent() {
        let center = std::sync::Arc::new(mk_center());
        center.update_session(mk_session(
            "session-1",
            "/repo",
            TurnPhase::Idle,
            SessionState::Idle,
        ));
        center.record_session_notice(SessionNotice {
            session_id: "session-1".to_string(),
            kind: SessionNoticeKind::PersistFailure,
            message: "Persistence failed".to_string(),
            created_at: 42.0,
        });
        let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
        let release = std::sync::Arc::new(std::sync::Barrier::new(2));
        center.set_update_session_notice_sync_hook_for_test({
            let entered = entered.clone();
            let release = release.clone();
            std::sync::Arc::new(move || {
                entered.wait();
                release.wait();
            })
        });
        let updater = {
            let center = center.clone();
            std::thread::spawn(move || {
                let mut status =
                    mk_session("session-1", "/repo", TurnPhase::Idle, SessionState::Idle);
                status.pty_id = Some("pty-1".to_string());
                center.update_session(status);
            })
        };
        entered.wait();
        let (completed_tx, completed_rx) = std::sync::mpsc::channel();
        let clearer = {
            let center = center.clone();
            std::thread::spawn(move || {
                center.clear_session_notice("session-1", SessionNoticeKind::PersistFailure);
                completed_tx.send(()).unwrap();
            })
        };

        let clear_was_blocked = completed_rx
            .recv_timeout(std::time::Duration::from_millis(50))
            .is_err();
        release.wait();
        updater.join().unwrap();
        clearer.join().unwrap();

        assert!(clear_was_blocked);
        assert!(!center.notices.read().contains_key("session-1"));
        assert_eq!(
            center
                .sessions
                .read()
                .get("session-1")
                .and_then(|status| status.notice.as_ref()),
            None
        );
    }

    #[test]
    fn worktree_node_status_view_serializes_canonical_execution_collections() {
        let center = mk_center();
        center.update_session(workflow_session(
            "node-execution-build-1",
            "build",
            Some(1),
            NodeProgress::Running,
            AgentState::Running,
        ));

        let value = serde_json::to_value(center.query_worktree_node_statuses("/repo"))
            .expect("worktree node status view serializes");

        assert!(value["nodeExecutions"].is_array());
        assert!(value["workflowExecutions"].is_array());
        assert!(value.get("nodes").is_none());
        assert!(value.get("executions").is_none());
        assert!(value.get("workflows").is_none());
    }

    // ---- on_session_state_changed core (build_state_transition) ----

    #[test]
    fn build_state_transition_returns_none_when_state_unchanged() {
        let s = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Idle);
        assert!(AgentStatusCenter::build_state_transition(&s, SessionState::Idle, 0.0).is_none());
    }

    #[test]
    fn same_state_projection_notification_republishes_session_snapshot() {
        let center = AgentStatusCenter::new();
        center.update_session(mk_session(
            "a",
            "/repo",
            TurnPhase::Idle,
            SessionState::Error,
        ));

        let changes = center.on_session_state_changed("a", SessionState::Error, 1);

        assert_eq!(
            changes
                .session
                .as_ref()
                .map(|session| session.chat_session_id.as_str()),
            Some("a")
        );
        assert!(changes.workspace.is_none());
        assert!(changes.agent_state.is_none());
        assert!(changes.workflow_node_views.is_empty());
    }

    #[test]
    fn stale_session_state_revision_cannot_overwrite_newer_state() {
        let center = AgentStatusCenter::new();
        center.update_session(mk_session(
            "a",
            "/repo",
            TurnPhase::Idle,
            SessionState::Active,
        ));

        let closed = center.on_session_state_changed("a", SessionState::Closed, 2);
        let stale = center.on_session_state_changed("a", SessionState::Error, 1);

        assert_eq!(
            closed.session.as_ref().map(|status| &status.session_state),
            Some(&SessionState::Closed)
        );
        assert!(stale.is_empty());
        assert_eq!(
            center.get_session("a").map(|status| status.session_state),
            Some(SessionState::Closed)
        );
    }

    #[test]
    fn build_state_transition_closes_streaming_session_with_idle_normalization() {
        // 閉じる前に Streaming だった SessionStatus を Closed に遷移させると
        // turn_phase / pending_permission を引きずらず正規化される
        let mut streaming = mk_session("a", "/repo", TurnPhase::Streaming, SessionState::Active);
        streaming.pending_permission = true;
        streaming.pending_permission_request = Some(permission_request_fixture());
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
        closed.pending_permission_request = Some(permission_request_fixture());
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
        center.on_session_state_changed("err", SessionState::Closed, 1);
        let after_close = center
            .get_workspace("/repo")
            .expect("workspace still tracked");
        assert_eq!(after_close.aggregated_state, AgentState::Done);
        assert_eq!(after_close.session_count, 1);
        assert_eq!(after_close.error_count, 0);

        // Closed → Idle で復帰させると Waiting / session_count=2 として再寄与する
        center.on_session_state_changed("err", SessionState::Idle, 2);
        let after_restore = center
            .get_workspace("/repo")
            .expect("workspace still tracked");
        assert_eq!(after_restore.aggregated_state, AgentState::Waiting);
        assert_eq!(after_restore.session_count, 2);
        assert_eq!(after_restore.waiting_count, 1);
    }

    // ---- Workflow 状態の Workspace 集約への寄与 ----

    #[test]
    fn workflow_execution_status_to_agent_state_maps_each_variant() {
        use crate::domain::workflow::RuntimeExecutionState;
        assert_eq!(
            AgentStatusCenter::workflow_execution_status_to_agent_state(
                &RuntimeExecutionState::Running
            ),
            Some(AgentState::Running)
        );
        assert_eq!(
            AgentStatusCenter::workflow_execution_status_to_agent_state(
                &RuntimeExecutionState::WaitingApproval
            ),
            Some(AgentState::Waiting)
        );
        assert_eq!(
            AgentStatusCenter::workflow_execution_status_to_agent_state(
                &RuntimeExecutionState::Failed {
                    reason: "boom".into(),
                    kind: crate::domain::workflow::NodeExecutionFailureKind::InfrastructureCrash,
                    retry_count: None,
                }
            ),
            Some(AgentState::Error)
        );
        assert_eq!(
            AgentStatusCenter::workflow_execution_status_to_agent_state(
                &RuntimeExecutionState::Completed
            ),
            Some(AgentState::Done)
        );
        assert_eq!(
            AgentStatusCenter::workflow_execution_status_to_agent_state(
                &RuntimeExecutionState::Aborted
            ),
            None
        );
        assert_eq!(
            AgentStatusCenter::workflow_execution_status_to_agent_state(
                &RuntimeExecutionState::Interrupted
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
    fn workflow_node_representative_updates_when_session_starts_streaming() {
        let center = mk_center();
        let queued = workflow_session(
            "step-a",
            "build",
            Some(1),
            NodeProgress::Queued,
            AgentState::Done,
        );
        let initial = center.update_session(queued);
        assert_eq!(
            initial.workflow_node_views[0].node_executions[0].representative,
            RepresentativeStatus::Queued.as_str()
        );
        let initial_version = initial.workflow_node_views[0].version;

        let running = workflow_session(
            "step-a",
            "build",
            Some(1),
            NodeProgress::Queued,
            AgentState::Running,
        );
        let changes = center.update_session(running);

        assert_eq!(
            changes.workflow_node_views[0].node_executions[0].representative,
            RepresentativeStatus::Running.as_str()
        );
        assert!(changes.workflow_node_views[0].version > initial_version);
        assert_eq!(changes.workflow_node_views.len(), 1);
        assert_eq!(changes.workflow_node_views[0].worktree_path, "/repo");
        assert_eq!(changes.workflow_node_views[0].node_executions.len(), 1);
        assert_eq!(
            changes.workflow_node_views[0].node_executions[0].representative,
            RepresentativeStatus::Running.as_str()
        );
    }

    #[test]
    fn workflow_node_representative_is_removed_when_session_is_archived() {
        let center = mk_center();
        center.update_session(workflow_session(
            "step-a",
            "build",
            Some(1),
            NodeProgress::Queued,
            AgentState::Done,
        ));

        let changes = center.on_session_state_changed("step-a", SessionState::Archived, 1);

        assert_eq!(changes.workflow_node_views.len(), 1);
        assert!(changes.workflow_node_views[0].node_executions.is_empty());
        assert!(changes.workflow_node_views[0]
            .workflow_executions
            .is_empty());
        assert!(changes.workflow_node_views[0].version > 0);
        let queried = center.query_worktree_node_statuses("/repo");
        assert!(queried.node_executions.is_empty());
        assert!(queried.workflow_executions.is_empty());
    }

    #[test]
    fn pending_workflow_projection_applies_to_first_session_status() {
        let center = mk_center();
        let sync_changes = center.sync_workflow_node_session_statuses(
            "/repo",
            "exec-1",
            "running",
            vec![projection(
                Some("step-late"),
                "build",
                Some(2),
                RepresentativeStatus::Running,
                NodeProgress::Running,
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
        assert_eq!(session.workflow_node.as_deref(), Some("build"));
        assert_eq!(
            session.workflow_execution_status.as_deref(),
            Some("running")
        );
        assert_eq!(session.workflow_execution_id.as_deref(), Some("exec-1"));
        assert_eq!(session.workflow_attempt, Some(2));
        assert_eq!(session.workflow_node_progress, Some(NodeProgress::Running));
        assert_eq!(changes.workflow_node_views.len(), 1);
        let view = &changes.workflow_node_views[0];
        assert_eq!(view.node_executions.len(), 1);
        assert_eq!(
            view.node_executions[0].representative,
            RepresentativeStatus::Running.as_str()
        );
        assert_eq!(view.workflow_executions.len(), 1);
        assert_eq!(
            view.workflow_executions[0].representative,
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
        center.sync_workflow_node_session_statuses(
            "/repo",
            "exec-1",
            "running",
            vec![projection(
                Some("step-late"),
                "build",
                Some(2),
                RepresentativeStatus::Queued,
                NodeProgress::Queued,
            )],
        );
        center.sync_workflow_node_session_statuses("/repo", "exec-1", "running", Vec::new());

        let changes = center.update_session(mk_session(
            "step-late",
            "/repo",
            TurnPhase::Idle,
            SessionState::Done,
        ));

        let session = changes.session.expect("session change emitted");
        assert_eq!(session.workflow_node, None);
        assert_eq!(session.workflow_execution_status, None);
        assert_eq!(session.workflow_execution_id, None);
        assert!(changes.workflow_node_views.is_empty());
    }

    #[test]
    fn fanout_running_node_execution_dominates_workflow_representative() {
        let center = mk_center();
        center.update_session(mk_session(
            "completed-session",
            "/repo",
            TurnPhase::Idle,
            SessionState::Done,
        ));
        center.update_session(mk_session(
            "running-session",
            "/repo",
            TurnPhase::Streaming,
            SessionState::Active,
        ));
        center.sync_workflow_node_session_statuses(
            "/repo",
            "execution-1",
            "running",
            vec![
                NodeSessionProjection {
                    node_execution_id: Some("node-execution-review-a".to_string()),
                    session_id: Some("completed-session".to_string()),
                    node_name: "review-a".to_string(),
                    attempt: Some(1),
                    group_node_name: "review-fanout".to_string(),
                    group_attempt: Some(1),
                    progress: NodeProgress::Completed,
                    representative: RepresentativeStatus::Completed,
                    order: 0,
                },
                NodeSessionProjection {
                    node_execution_id: Some("node-execution-review-b".to_string()),
                    session_id: Some("running-session".to_string()),
                    node_name: "review-b".to_string(),
                    attempt: Some(1),
                    group_node_name: "review-fanout".to_string(),
                    group_attempt: Some(1),
                    progress: NodeProgress::Running,
                    representative: RepresentativeStatus::Running,
                    order: 1,
                },
            ],
        );

        let view = center.query_worktree_node_statuses("/repo");
        assert_eq!(view.node_executions.len(), 2);
        assert_eq!(view.workflow_executions.len(), 1);
        assert_eq!(view.workflow_executions[0].execution_id, "execution-1");
        assert_eq!(
            view.workflow_executions[0].representative,
            RepresentativeStatus::Running.as_str()
        );
    }

    #[test]
    fn query_worktree_node_statuses_returns_backend_aggregated_view() {
        let center = mk_center();
        center.update_session(workflow_session(
            "step-a",
            "plan",
            Some(1),
            NodeProgress::Completed,
            AgentState::Done,
        ));
        center.update_session(workflow_session(
            "step-b",
            "test",
            Some(1),
            NodeProgress::WaitingApproval,
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
            workflow_node: Some("deploy".to_string()),
            workflow_execution_status: Some("running".to_string()),
            workflow_execution_id: Some("exec-other".to_string()),
            node_execution_id: Some("deploy-1".to_string()),
            workflow_attempt: Some(1),
            notice: None,
            workflow_node_progress: Some(NodeProgress::Running),
        });

        let view = center.query_worktree_node_statuses("/repo");

        assert_eq!(view.worktree_path, "/repo");
        assert!(view.version > 0);
        assert_eq!(view.node_executions.len(), 2);
        assert!(view.node_executions.iter().any(|step| {
            step.execution_id == "exec-1"
                && step.node_name == "plan"
                && step.representative == RepresentativeStatus::Completed.as_str()
        }));
        assert!(view.node_executions.iter().any(|step| {
            step.execution_id == "exec-1"
                && step.node_name == "test"
                && step.representative == RepresentativeStatus::Error.as_str()
        }));
        assert_eq!(view.workflow_executions.len(), 1);
        assert_eq!(view.workflow_executions[0].execution_id, "exec-1");
        assert_eq!(
            view.workflow_executions[0].representative,
            RepresentativeStatus::Error.as_str()
        );

        let other = center.query_worktree_node_statuses("/other");
        assert!(other.version > 0);
        assert_eq!(other.node_executions.len(), 1);
        assert_eq!(other.node_executions[0].execution_id, "exec-other");
    }

    #[test]
    fn query_worktree_node_statuses_keeps_monotonic_version_after_empty_snapshot() {
        let center = mk_center();
        let initial = center.update_session(workflow_session(
            "step-a",
            "build",
            Some(1),
            NodeProgress::Queued,
            AgentState::Done,
        ));
        let initial_version = initial.workflow_node_views[0].version;

        let removed = center.on_session_state_changed("step-a", SessionState::Archived, 1);

        assert_eq!(removed.workflow_node_views.len(), 1);
        assert!(removed.workflow_node_views[0].node_executions.is_empty());
        assert!(removed.workflow_node_views[0]
            .workflow_executions
            .is_empty());
        assert!(removed.workflow_node_views[0].version > initial_version);

        let queried = center.query_worktree_node_statuses("/repo");
        assert!(queried.node_executions.is_empty());
        assert!(queried.workflow_executions.is_empty());
        assert_eq!(queried.version, removed.workflow_node_views[0].version);
    }

    #[test]
    fn workflow_node_status_event_view_matches_query_snapshot() {
        let center = mk_center();
        let changes = center.update_session(workflow_session(
            "step-a",
            "build",
            Some(1),
            NodeProgress::Running,
            AgentState::Running,
        ));

        assert_eq!(changes.workflow_node_views.len(), 1);
        let emitted = &changes.workflow_node_views[0];
        let queried = center.query_worktree_node_statuses("/repo");

        assert_eq!(emitted, &queried);
    }

    #[test]
    fn workflow_node_status_view_sorts_none_attempt_before_some() {
        let center = mk_center();
        center.update_session(workflow_session(
            "step-some",
            "build",
            Some(1),
            NodeProgress::Running,
            AgentState::Running,
        ));
        center.update_session(workflow_session(
            "step-none",
            "build",
            None,
            NodeProgress::Running,
            AgentState::Running,
        ));

        let queried = center.query_worktree_node_statuses("/repo");
        let attempts = queried
            .node_executions
            .iter()
            .map(|step| step.attempt)
            .collect::<Vec<_>>();

        assert_eq!(attempts, vec![None, Some(1)]);
    }

    #[test]
    fn workflow_node_status_view_refresh_does_not_rewind_stored_snapshot() {
        let key = WorkflowNodeExecutionKey {
            worktree_path: "/repo".to_string(),
            execution_id: "exec-1".to_string(),
            node_execution_id: "build-1".to_string(),
            node_name: "build".to_string(),
            attempt: Some(1),
        };
        let mut state = WorkflowNodeStatusState::default();
        state.node_executions.insert(
            key.clone(),
            WorkflowStatusEntry {
                representative: RepresentativeStatus::Running,
            },
        );
        let initial = AgentStatusCenter::update_worktree_node_status_view(&mut state, "/repo", 10);

        state.node_executions.insert(
            key,
            WorkflowStatusEntry {
                representative: RepresentativeStatus::Error,
            },
        );
        let stale = AgentStatusCenter::update_worktree_node_status_view(&mut state, "/repo", 5);

        assert_eq!(stale, initial);
        assert_eq!(state.views.get("/repo"), Some(&initial));
    }

    #[test]
    fn workflow_representative_aggregates_live_nodes_with_snapshot_baselines() {
        let center = mk_center();
        center.sync_workflow_node_session_statuses(
            "/repo",
            "exec-1",
            "running",
            vec![
                projection(
                    Some("step-live"),
                    "live",
                    Some(1),
                    RepresentativeStatus::Queued,
                    NodeProgress::Queued,
                ),
                projection(
                    None,
                    "failed-history",
                    Some(1),
                    RepresentativeStatus::Failed,
                    NodeProgress::Failed,
                ),
            ],
        );

        let live_waiting = workflow_session(
            "step-live",
            "live",
            Some(1),
            NodeProgress::Queued,
            AgentState::Waiting,
        );
        let changes = center.update_session(live_waiting);

        assert_eq!(
            changes.workflow_node_views[0].node_executions[0].representative,
            RepresentativeStatus::Waiting.as_str()
        );
        assert_eq!(
            changes.workflow_node_views[0].workflow_executions[0].representative,
            RepresentativeStatus::Failed.as_str()
        );
    }

    #[test]
    fn sync_workflow_node_statuses_returns_current_view_after_live_reaggregation() {
        let center = mk_center();
        center.update_session(workflow_session(
            "step-live",
            "live",
            Some(1),
            NodeProgress::Queued,
            AgentState::Done,
        ));

        let changes = center.sync_workflow_node_session_statuses(
            "/repo",
            "exec-1",
            "running",
            vec![projection(
                Some("step-live"),
                "live",
                Some(1),
                RepresentativeStatus::Queued,
                NodeProgress::Running,
            )],
        );

        let emitted_views = changes
            .iter()
            .flat_map(|change| change.workflow_node_views.iter())
            .collect::<Vec<_>>();
        assert_eq!(emitted_views.len(), 1);
        let emitted = emitted_views[0];
        let queried = center.query_worktree_node_statuses("/repo");
        assert_eq!(emitted, &queried);
        assert_eq!(
            emitted.node_executions[0].representative,
            RepresentativeStatus::Running.as_str()
        );
    }

    #[test]
    fn workflow_representative_clears_when_last_live_step_is_removed() {
        let center = mk_center();
        center.sync_workflow_node_session_statuses(
            "/repo",
            "exec-1",
            "running",
            vec![projection(
                Some("step-live"),
                "live",
                Some(1),
                RepresentativeStatus::Queued,
                NodeProgress::Queued,
            )],
        );
        center.update_session(workflow_session(
            "step-live",
            "live",
            Some(1),
            NodeProgress::Queued,
            AgentState::Running,
        ));

        let changes = center.on_session_state_changed("step-live", SessionState::Archived, 1);

        assert_eq!(changes.workflow_node_views.len(), 1);
        assert!(changes.workflow_node_views[0].node_executions.is_empty());
        assert!(changes.workflow_node_views[0]
            .workflow_executions
            .is_empty());
    }
}
