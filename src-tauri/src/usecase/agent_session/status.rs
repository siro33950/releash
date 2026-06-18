use crate::usecase::agent_session::session::SessionState;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Running,
    Done,
    Error,
    Waiting,
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
    pub last_activity_at: f64,
    pub workflow_step: Option<String>,
    pub workflow_execution_state: Option<String>,
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
}

impl AgentStatusChanges {
    pub fn is_empty(&self) -> bool {
        self.session.is_none() && self.workspace.is_none() && self.agent_state.is_none()
    }
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
    /// key: worktree_path（= worktree_id と同値で運用）
    workflows: RwLock<HashMap<String, WorkflowAggSnapshot>>,
}

impl AgentStatusCenter {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            workspaces: RwLock::new(HashMap::new()),
            workflows: RwLock::new(HashMap::new()),
        }
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
            && a.workflow_step == b.workflow_step
            && a.workflow_execution_state == b.workflow_execution_state
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

    /// Workspace の集約規則: Error > Waiting > Running > Done。
    /// `SessionState::Closed`（タブを閉じた Session）はオープン中でないため集約対象から除外する。
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
            .filter(|s| s.session_state != SessionState::Closed)
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

        let aggregated_state = if error_count > 0 {
            AgentState::Error
        } else if waiting_count > 0 {
            AgentState::Waiting
        } else if running_count > 0 {
            AgentState::Running
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
            .filter(|s| s.session_state != SessionState::Closed)
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

    /// Session 状態を更新する。
    /// 1. dedup（前回と等価なら何もしない）
    /// 2. sessions マップに反映
    /// 3. 同 worktree の全 SessionStatus から WorkspaceStatus を再計算
    /// 4. 呼び出し側が transport 層で通知できるよう変更結果を返す
    pub fn update_session(&self, status: SessionStatus) -> AgentStatusChanges {
        // 1. dedup
        {
            let prev = self.sessions.read().get(&status.chat_session_id).cloned();
            if let Some(prev) = prev {
                if Self::is_session_state_equivalent(&prev, &status) {
                    return AgentStatusChanges::default();
                }
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
        let new_snapshot = WorkflowAggSnapshot {
            execution_id: execution_id.to_string(),
            agent_state: agent_state.clone(),
            last_activity_at,
        };

        // 1. dedup（execution_id / agent_state が変わらなければスキップ）
        {
            let workflows = self.workflows.read();
            if let Some(prev) = workflows.get(worktree_path) {
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
            workflows.insert(worktree_path.to_string(), new_snapshot);
        }

        // 3. 再集約
        AgentStatusChanges {
            workspace: self.reaggregate_workspace(worktree_path, last_activity_at),
            ..Default::default()
        }
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
        let workflow_snapshot = self.workflow_agg_snapshot_for(worktree_path);
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
                .unwrap_or_else(|| worktree_path.to_string());
            let workspace = if same_workspace.is_empty()
                && workflow_snapshot
                    .as_ref()
                    .is_none_or(|snapshot| snapshot.agent_state.is_none())
            {
                None
            } else {
                Some(Self::aggregate_with_workflow_snapshot(
                    &worktree_id,
                    worktree_path,
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
                        worktree_path: worktree_path.to_string(),
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
    /// Closed への遷移は `aggregate` 段階で集約対象から外れ、Closed → 他状態の
    /// 復帰では再び集約対象に戻る。
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
        let normalize_to_idle = matches!(new_state, SessionState::Closed | SessionState::Idle);
        let (turn_phase_repr, pending_permission) = if normalize_to_idle {
            (TurnPhaseRepr::Idle, false)
        } else {
            (existing.turn_phase, existing.pending_permission)
        };
        let agent_state = Self::derive_agent_state(turn_phase_repr.into(), new_state.clone());
        Some(SessionStatus {
            session_state: new_state,
            agent_state,
            turn_phase: turn_phase_repr,
            pending_permission,
            last_activity_at,
            ..existing.clone()
        })
    }

    /// Session を削除し、worktree を再集約する。
    pub fn get_session(&self, chat_session_id: &str) -> Option<SessionStatus> {
        self.sessions.read().get(chat_session_id).cloned()
    }

    pub fn get_workspace(&self, worktree_id: &str) -> Option<WorkspaceStatus> {
        self.workspaces.read().get(worktree_id).cloned()
    }

    pub fn list_workspaces(&self) -> Vec<WorkspaceStatus> {
        self.workspaces.read().values().cloned().collect()
    }

    pub fn list_sessions(&self) -> Vec<SessionStatus> {
        self.sessions.read().values().cloned().collect()
    }
}

pub fn current_timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
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
            last_activity_at: 0.0,
            workflow_step: None,
            workflow_execution_state: None,
        }
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
    fn aggregate_waiting_dominates_running() {
        let s1 = mk_session("a", "/repo", TurnPhase::Streaming, SessionState::Active);
        let s2 = mk_session(
            "b",
            "/repo",
            TurnPhase::WaitingPermission,
            SessionState::Active,
        );
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[&s1, &s2], None, 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Waiting);
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
    fn aggregate_error_dominates_all() {
        let s1 = mk_session("a", "/repo", TurnPhase::Streaming, SessionState::Active);
        let s2 = mk_session(
            "b",
            "/repo",
            TurnPhase::WaitingPermission,
            SessionState::Active,
        );
        let s3 = mk_session("c", "/repo", TurnPhase::Idle, SessionState::Error);
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[&s1, &s2, &s3], None, 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Error);
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
        let updated =
            AgentStatusCenter::build_state_transition(&streaming, SessionState::Closed, 42.0)
                .expect("transition should produce updated session");
        assert_eq!(updated.session_state, SessionState::Closed);
        assert_eq!(updated.turn_phase, TurnPhaseRepr::Idle);
        assert!(!updated.pending_permission);
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
        let updated = AgentStatusCenter::build_state_transition(&closed, SessionState::Idle, 0.0)
            .expect("transition should produce updated session");
        assert_eq!(updated.session_state, SessionState::Idle);
        assert_eq!(updated.turn_phase, TurnPhaseRepr::Idle);
        assert!(!updated.pending_permission);
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
                    reason: "boom".into()
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
    fn aggregate_with_workflow_waiting_outranks_running_sessions() {
        // Session=Running、Workflow=Waiting → Workspace=Waiting（Waiting 優先）
        let s = mk_session("a", "/repo", TurnPhase::Streaming, SessionState::Active);
        let ws =
            AgentStatusCenter::aggregate("/repo", "/repo", &[&s], Some(AgentState::Waiting), 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Waiting);
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
}
