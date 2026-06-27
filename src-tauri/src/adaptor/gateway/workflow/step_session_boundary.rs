use std::sync::Arc;

use tokio::sync::Mutex;

use super::runtime_events as workflow_runtime_events;
use super::runtime_session as workflow_runtime_session;
use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::state::WorkflowState;
use crate::adaptor::gateway::workflow::step_settings::WorkflowDefaults;
use crate::domain::workflow::{NodeType, WorkflowStepContext};
use crate::infrastructure::agent_session::runtime::AgentProcessMap;
use crate::infrastructure::agent_session::runtime::AgentRuntimeError;
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::session::SessionStore;

/// AgentSession 開始呼び出しを抽象化するトレイト。
/// production では `start_agent_session_internal` を呼ぶ `RealSessionStartGate` を使い、
/// テストでは引数を記録するテストダブルに差し替えて、合成された `system_prompt` が
/// バックエンドへ受け渡される経路を検証する。
#[async_trait::async_trait]
pub(crate) trait SessionStartGate: Send + Sync {
    async fn start_session(
        &self,
        session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
        workflow_instruction: Option<String>,
    ) -> Result<(), AgentRuntimeError>;
}

/// production 用の `SessionStartGate` 実装。`start_agent_session_internal` をそのまま呼び出す。
struct RealSessionStartGate<'a, R: tauri::Runtime> {
    app: &'a tauri::AppHandle<R>,
    branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    handles: &'a Arc<Mutex<AgentProcessMap>>,
    session_store: &'a Arc<SessionStore>,
}

#[async_trait::async_trait]
impl<'a, R: tauri::Runtime> SessionStartGate for RealSessionStartGate<'a, R> {
    async fn start_session(
        &self,
        session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
        workflow_instruction: Option<String>,
    ) -> Result<(), AgentRuntimeError> {
        crate::infrastructure::agent_session::runtime::start_agent_session_internal(
            self.app,
            self.branch_diff_context.clone(),
            self.handles,
            self.session_store,
            session_id,
            worktree_path,
            permission_mode,
            false,
            system_prompt,
            workflow_instruction.into_iter().collect(),
        )
        .await
    }
}

/// `start_step_session` 内でファセット合成（純粋関数）後に実行される副作用境界を
/// まとめて抽象化するトレイト。
#[async_trait::async_trait]
pub(crate) trait StepSessionDeps: Send + Sync {
    /// ステップ用 ChatSession を生成し、IDと permission_mode を返す。
    ///
    /// `workflow_defaults` は workflow 開始時に capture された継承デフォルト。
    /// 各 step は `step_model` / `step_permission` で上書きできる。
    async fn create_step_session(
        &self,
        worktree_path: &str,
        step_model: Option<String>,
        step_permission: Option<String>,
        workflow_defaults: WorkflowDefaults,
        workflow_step_context: WorkflowStepContext,
        node_kind: NodeType,
    ) -> Result<StepSessionInfo, WorkflowEngineError>;

    /// 合成済み `system_prompt` を AgentSession 開始経路へ受け渡す。
    async fn dispatch_session_start(
        &self,
        step_session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
        workflow_instruction: Option<String>,
    ) -> Result<(), WorkflowEngineError>;

    async fn mark_step_tab_open(&self, step_session_id: &str);

    /// ワークフロー状態をブロードキャストする（best-effort）。
    async fn broadcast_state(&self, worktree_path: &str, snapshot: WorkflowState);

    /// step session と node の紐付きを event log に確定する。
    async fn append_node_session_started(
        &self,
        snapshot: &WorkflowState,
    ) -> Result<(), WorkflowEngineError>;

    /// Runtime lock acquired by the caller variant.
    async fn start_agent_turn_locked(
        &self,
        step_session_id: &str,
        worktree_path: &str,
        permission_mode: &str,
        prompt: &str,
        system_prompt: Option<String>,
        workflow_instruction: Option<String>,
    ) -> Result<(), WorkflowEngineError>;
}

/// `StepSessionDeps::create_step_session` の戻り値。
#[derive(Clone, Debug)]
pub(crate) struct StepSessionInfo {
    pub(crate) id: String,
    pub(crate) permission_mode: String,
}

/// production 用の `StepSessionDeps` 実装。
pub(crate) struct RealStepSessionDeps<'a, R: tauri::Runtime> {
    pub(crate) app: &'a tauri::AppHandle<R>,
    pub(crate) branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    pub(crate) handles: &'a Arc<Mutex<AgentProcessMap>>,
    pub(crate) session_store: &'a Arc<SessionStore>,
}

#[async_trait::async_trait]
impl<'a, R: tauri::Runtime> StepSessionDeps for RealStepSessionDeps<'a, R> {
    async fn create_step_session(
        &self,
        worktree_path: &str,
        step_model: Option<String>,
        step_permission: Option<String>,
        workflow_defaults: WorkflowDefaults,
        workflow_step_context: WorkflowStepContext,
        node_kind: NodeType,
    ) -> Result<StepSessionInfo, WorkflowEngineError> {
        let data_dir = crate::app_data_dir::resolve_data_dir(self.app)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;
        let step_session = workflow_runtime_session::create_step_session_with_settings(
            self.app,
            self.session_store,
            &data_dir,
            worktree_path,
            step_model,
            step_permission,
            &workflow_defaults,
            workflow_step_context,
            node_kind,
        )
        .await?;
        Ok(StepSessionInfo {
            id: step_session.id,
            permission_mode: step_session.permission_mode,
        })
    }

    async fn dispatch_session_start(
        &self,
        step_session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
        workflow_instruction: Option<String>,
    ) -> Result<(), WorkflowEngineError> {
        let gate = RealSessionStartGate {
            app: self.app,
            branch_diff_context: self.branch_diff_context.clone(),
            handles: self.handles,
            session_store: self.session_store,
        };
        dispatch_session_start(
            &gate,
            step_session_id,
            worktree_path,
            permission_mode,
            system_prompt,
            workflow_instruction,
        )
        .await
    }

    async fn mark_step_tab_open(&self, step_session_id: &str) {
        crate::adaptor::gateway::workflow::mark_started_step_tab_open(self.app, step_session_id);
    }

    async fn broadcast_state(&self, worktree_path: &str, snapshot: WorkflowState) {
        workflow_runtime_session::broadcast_state(self.app, worktree_path, snapshot).await;
    }

    async fn append_node_session_started(
        &self,
        snapshot: &WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
        let Some(event) =
            workflow_runtime_events::node_session_started_event_for_snapshot(snapshot)
        else {
            return Ok(());
        };
        crate::adaptor::gateway::workflow::event_log_writer::append_required_events_for_app(
            self.app,
            &[event],
        )
        .map_err(|e| {
            WorkflowEngineError::SessionStore(format!("append NodeSessionStarted failed: {e}"))
        })
    }

    async fn start_agent_turn_locked(
        &self,
        step_session_id: &str,
        worktree_path: &str,
        permission_mode: &str,
        prompt: &str,
        system_prompt: Option<String>,
        workflow_instruction: Option<String>,
    ) -> Result<(), WorkflowEngineError> {
        crate::infrastructure::agent_session::runtime::start_agent_turn_internal_locked(
            self.app,
            self.branch_diff_context.clone(),
            self.handles,
            self.session_store,
            step_session_id,
            worktree_path,
            permission_mode,
            prompt,
            system_prompt,
            workflow_instruction.into_iter().collect(),
        )
        .await
        .map_err(WorkflowEngineError::from)
    }
}

/// `SessionStartGate` 経由で AgentSession を開始する。
/// production からは `RealSessionStartGate` を、テストからは記録用テストダブルを渡す。
pub(crate) async fn dispatch_session_start<G: SessionStartGate + ?Sized>(
    gate: &G,
    session_id: &str,
    worktree_path: &str,
    permission_mode: Option<String>,
    system_prompt: Option<String>,
    workflow_instruction: Option<String>,
) -> Result<(), WorkflowEngineError> {
    gate.start_session(
        session_id,
        worktree_path,
        permission_mode,
        system_prompt,
        workflow_instruction,
    )
    .await
    .map_err(|error| {
        WorkflowEngineError::with_agent_runtime_context(
            format!("Failed to start AgentSession '{session_id}'"),
            error,
        )
    })
}
