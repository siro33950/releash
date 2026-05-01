use crate::agent_sdk::TurnPhase;
use crate::protocol::{AgentState, AgentStateSync, WsMessage};
use crate::session::SessionState;
use crate::ws_bridge::WsBroadcaster;
use parking_lot::RwLock;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, State};

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

/// Session / Workspace の状態を集中管理し、フロント・WS にブロードキャストする中央管理。
pub struct AgentStatusCenter {
    sessions: RwLock<HashMap<String, SessionStatus>>,
    workspaces: RwLock<HashMap<String, WorkspaceStatus>>,
    app_handle: AppHandle,
    broadcaster: Arc<WsBroadcaster>,
}

impl AgentStatusCenter {
    pub fn new(app_handle: AppHandle, broadcaster: Arc<WsBroadcaster>) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            workspaces: RwLock::new(HashMap::new()),
            app_handle,
            broadcaster,
        }
    }

    /// AgentState は turn_phase / session_state から派生する。
    /// frontend `deriveAgentState` と同じ規則。
    pub fn derive_agent_state(turn_phase: TurnPhase, session_state: SessionState) -> AgentState {
        match turn_phase {
            TurnPhase::Streaming => AgentState::Running,
            TurnPhase::WaitingPermission => AgentState::Waiting,
            TurnPhase::Idle => match session_state {
                SessionState::Error => AgentState::Error,
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

    /// Workspace の集約規則: Error > Waiting > Running > Done
    fn aggregate(
        worktree_id: &str,
        worktree_path: &str,
        sessions: &[&SessionStatus],
        last_activity_at: f64,
    ) -> WorkspaceStatus {
        let mut running_count = 0usize;
        let mut waiting_count = 0usize;
        let mut error_count = 0usize;

        for s in sessions {
            match s.agent_state {
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
            session_count: sessions.len(),
            last_activity_at,
        }
    }

    /// Session 状態を更新する。
    /// 1. dedup（前回と等価なら何もしない）
    /// 2. sessions マップに反映
    /// 3. 同 worktree の全 SessionStatus から WorkspaceStatus を再計算
    /// 4. session-status-changed / workspace-status-changed / agent-state-changed を emit
    /// 5. WsMessage::AgentStateSync を broadcast
    pub fn update_session(&self, status: SessionStatus) {
        // 1. dedup
        {
            let prev = self.sessions.read().get(&status.chat_session_id).cloned();
            if let Some(prev) = prev {
                if Self::is_session_state_equivalent(&prev, &status) {
                    return;
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

        // 3. workspace 再集約
        let new_workspace = {
            let sessions = self.sessions.read();
            let same_workspace: Vec<&SessionStatus> = sessions
                .values()
                .filter(|s| s.worktree_id == worktree_id)
                .collect();
            Self::aggregate(
                &worktree_id,
                &worktree_path,
                &same_workspace,
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

        // 4. emit
        self.emit_session_changed(&status);
        if workspace_changed {
            self.emit_workspace_changed(&new_workspace);
        }
        self.emit_agent_state_changed_compat(&worktree_path, &agent_state, pty_id.as_deref());

        // 5. WS broadcast (legacy 互換)
        self.broadcast_agent_state_sync(&worktree_path, &agent_state, last_activity_at, pty_id);
    }

    /// Session を削除し、worktree を再集約する。
    #[allow(dead_code)] // 将来 AgentProcess 明示削除時に使用予定
    pub fn remove_session(&self, chat_session_id: &str) {
        let removed = {
            let mut sessions = self.sessions.write();
            sessions.remove(chat_session_id)
        };

        let Some(removed) = removed else { return };

        let worktree_id = removed.worktree_id.clone();
        let worktree_path = removed.worktree_path.clone();
        let last_activity_at = current_timestamp();

        let new_workspace = {
            let sessions = self.sessions.read();
            let same_workspace: Vec<&SessionStatus> = sessions
                .values()
                .filter(|s| s.worktree_id == worktree_id)
                .collect();
            if same_workspace.is_empty() {
                None
            } else {
                Some(Self::aggregate(
                    &worktree_id,
                    &worktree_path,
                    &same_workspace,
                    last_activity_at,
                ))
            }
        };

        match new_workspace {
            Some(ws) => {
                let workspace_changed = {
                    let mut workspaces = self.workspaces.write();
                    let prev_ws = workspaces.get(&worktree_id).cloned();
                    let changed = prev_ws
                        .as_ref()
                        .map(|p| !Self::is_workspace_state_equivalent(p, &ws))
                        .unwrap_or(true);
                    workspaces.insert(worktree_id.clone(), ws.clone());
                    changed
                };
                if workspace_changed {
                    self.emit_workspace_changed(&ws);
                }
            }
            None => {
                let removed_ws = {
                    let mut workspaces = self.workspaces.write();
                    workspaces.remove(&worktree_id)
                };
                if removed_ws.is_some() {
                    // workspace が空になった: aggregated_state=Done で送る
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
                    self.emit_workspace_changed(&empty);
                }
            }
        }

        // session-status-changed: 削除も通知（agent_state を Done にしたスナップショットを送る）
        let mut closed = removed.clone();
        closed.agent_state = AgentState::Done;
        closed.last_activity_at = last_activity_at;
        self.emit_session_changed(&closed);
    }

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

    fn emit_session_changed(&self, status: &SessionStatus) {
        use tauri::Emitter;
        let _ = self.app_handle.emit("session-status-changed", status);
    }

    fn emit_workspace_changed(&self, status: &WorkspaceStatus) {
        use tauri::Emitter;
        let _ = self.app_handle.emit("workspace-status-changed", status);
    }

    /// ワークフロー状態変更を通知する（Tauriイベント + WebSocket）。
    pub fn emit_workflow_state_changed(
        &self,
        worktree_path: &str,
        workflow_state: &crate::session::WorkflowState,
    ) {
        use tauri::Emitter;

        // Tauriイベント（デスクトップUI向け）
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> {
            worktree_path: &'a str,
            workflow_state: &'a crate::session::WorkflowState,
        }
        let payload = Payload {
            worktree_path,
            workflow_state,
        };
        let _ = self.app_handle.emit("workflow-state-changed", &payload);

        // WebSocketブロードキャスト（リモートアプリ向け）
        let ws_sync = crate::protocol::WorkflowStateSync {
            worktree_path: worktree_path.to_string(),
            workflow_state: workflow_state.clone(),
        };
        self.broadcaster
            .try_send(WsMessage::WorkflowStateSync(Box::new(ws_sync)));
    }

    /// 既存 frontend / remote が購読している `agent-state-changed` を維持する。
    fn emit_agent_state_changed_compat(
        &self,
        worktree_path: &str,
        state: &AgentState,
        pty_id: Option<&str>,
    ) {
        use tauri::Emitter;
        let payload = AgentStateSync {
            worktree_path: worktree_path.to_string(),
            state: state.clone(),
            exit_code: None,
            timestamp: current_timestamp(),
            session_id: None,
            pty_id: pty_id.map(|s| s.to_string()),
        };
        let _ = self.app_handle.emit("agent-state-changed", &payload);
    }

    fn broadcast_agent_state_sync(
        &self,
        worktree_path: &str,
        state: &AgentState,
        timestamp: f64,
        pty_id: Option<String>,
    ) {
        let msg = WsMessage::AgentStateSync(AgentStateSync {
            worktree_path: worktree_path.to_string(),
            state: state.clone(),
            exit_code: None,
            timestamp,
            session_id: None,
            pty_id,
        });
        self.broadcaster.try_send(msg);
    }
}

pub fn current_timestamp() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[tauri::command]
pub fn get_session_status(
    chat_session_id: String,
    center: State<'_, Arc<AgentStatusCenter>>,
) -> Option<SessionStatus> {
    center.get_session(&chat_session_id)
}

#[tauri::command]
pub fn get_workspace_status(
    worktree_id: String,
    center: State<'_, Arc<AgentStatusCenter>>,
) -> Option<WorkspaceStatus> {
    center.get_workspace(&worktree_id)
}

#[tauri::command]
pub fn list_workspace_statuses(center: State<'_, Arc<AgentStatusCenter>>) -> Vec<WorkspaceStatus> {
    center.list_workspaces()
}

#[tauri::command]
pub fn list_session_statuses(center: State<'_, Arc<AgentStatusCenter>>) -> Vec<SessionStatus> {
    center.list_sessions()
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
    fn idle_with_non_error_session_maps_to_done() {
        for ss in [
            SessionState::Active,
            SessionState::Idle,
            SessionState::Done,
            SessionState::Closed,
        ] {
            assert_eq!(
                AgentStatusCenter::derive_agent_state(TurnPhase::Idle, ss),
                AgentState::Done
            );
        }
    }

    // ---- aggregate ----

    #[test]
    fn aggregate_empty_sessions_is_done() {
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[], 100.0);
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
        let s2 = mk_session("b", "/repo", TurnPhase::Idle, SessionState::Idle);
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[&s1, &s2], 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Done);
        assert_eq!(ws.session_count, 2);
    }

    #[test]
    fn aggregate_running_dominates_done() {
        let s1 = mk_session("a", "/repo", TurnPhase::Idle, SessionState::Done);
        let s2 = mk_session("b", "/repo", TurnPhase::Streaming, SessionState::Active);
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[&s1, &s2], 0.0);
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
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[&s1, &s2], 0.0);
        assert_eq!(ws.aggregated_state, AgentState::Waiting);
        assert_eq!(ws.waiting_count, 1);
        assert_eq!(ws.running_count, 1);
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
        let ws = AgentStatusCenter::aggregate("/repo", "/repo", &[&s1, &s2, &s3], 0.0);
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
        let mut a = AgentStatusCenter::aggregate("/r", "/r", &[], 100.0);
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
}
