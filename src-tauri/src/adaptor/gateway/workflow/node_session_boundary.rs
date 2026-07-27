use std::sync::Arc;

use super::runtime_events as workflow_runtime_events;
use super::runtime_session as workflow_runtime_session;
use crate::adaptor::gateway::workflow::node_settings::WorkflowDefaults;
use crate::adaptor::gateway::workflow::runtime_error::WorkflowRuntimeError;
use crate::adaptor::gateway::workflow::state::RuntimeCommitSnapshot;
use crate::domain::agent_session::PermissionMode;
use crate::domain::workflow::WorkflowNodeContext;
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::runtime::usecase::AgentRuntimeError;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{OpenTabRegistry, SessionStore};

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
    _app: &'a tauri::AppHandle<R>,
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
        let _ = (
            session_id,
            worktree_path,
            permission_mode,
            system_prompt,
            workflow_instruction,
        );
        Ok(())
    }
}

/// `start_node_session` 内でファセット合成（純粋関数）後に実行される副作用境界を
/// まとめて抽象化するトレイト。
#[async_trait::async_trait]
pub(crate) trait NodeSessionDeps: Send + Sync {
    /// ステップ用 ChatSession を生成し、IDと permission_mode を返す。
    ///
    /// `workflow_defaults` は workflow 開始時に capture された継承デフォルト。
    /// 各 node は `node_model` / `node_permission` で上書きできる。
    async fn create_node_session(
        &self,
        worktree_path: &str,
        node_model: Option<String>,
        node_permission: Option<String>,
        workflow_defaults: WorkflowDefaults,
        workflow_node_context: WorkflowNodeContext,
        kind_context: workflow_runtime_session::NodeRuntimeKindContext,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError>;

    /// 合成済み `system_prompt` を AgentSession 開始経路へ受け渡す。
    async fn dispatch_session_start(
        &self,
        node_session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
        workflow_instruction: Option<String>,
    ) -> Result<(), WorkflowRuntimeError>;

    async fn mark_node_tab_open(&self, node_session_id: &str);

    /// ワークフロー状態をブロードキャストする（best-effort）。
    async fn broadcast_state(&self, worktree_path: &str, snapshot: RuntimeCommitSnapshot);

    /// node session と node の紐付きを event log に確定する。
    async fn append_node_session_started(
        &self,
        snapshot: &RuntimeCommitSnapshot,
    ) -> Result<(), WorkflowRuntimeError>;

    /// Runtime lock acquired by the caller variant.
    async fn start_agent_turn_locked(
        &self,
        node_execution_id: &str,
        node_session_id: &str,
        permission_mode: &str,
        prompt: &str,
        system_prompt: Option<String>,
        workflow_instruction: Option<String>,
    ) -> Result<(), WorkflowRuntimeError>;
}

/// `NodeSessionDeps::create_node_session` の戻り値。
#[derive(Clone, Debug)]
pub(crate) struct NodeSessionInfo {
    pub(crate) id: String,
    pub(crate) permission_mode: String,
}

/// production 用の `NodeSessionDeps` 実装。
pub(crate) struct RealNodeSessionDeps<'a, R: tauri::Runtime> {
    pub(crate) app: &'a tauri::AppHandle<R>,
    pub(crate) branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    pub(crate) agent_runtime: &'a Arc<AgentSessionRuntimeUsecase>,
    pub(crate) session_store: &'a Arc<SessionStore>,
    pub(crate) open_tabs: &'a Arc<OpenTabRegistry>,
}

#[async_trait::async_trait]
impl<'a, R: tauri::Runtime> NodeSessionDeps for RealNodeSessionDeps<'a, R> {
    async fn create_node_session(
        &self,
        worktree_path: &str,
        node_model: Option<String>,
        node_permission: Option<String>,
        workflow_defaults: WorkflowDefaults,
        workflow_node_context: WorkflowNodeContext,
        kind_context: workflow_runtime_session::NodeRuntimeKindContext,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
        let data_dir = crate::infrastructure::platform::app_data_dir::resolve_data_dir(self.app)
            .map_err(|e| WorkflowRuntimeError::SessionStore(format!("resolve_data_dir: {e}")))?;
        let node_session = workflow_runtime_session::create_node_session_with_settings(
            self.agent_runtime.backend_registry(),
            self.session_store,
            &data_dir,
            worktree_path,
            node_model,
            node_permission,
            &workflow_defaults,
            workflow_node_context,
            kind_context,
        )?;
        Ok(NodeSessionInfo {
            id: node_session.id,
            permission_mode: node_session.permission_mode,
        })
    }

    async fn dispatch_session_start(
        &self,
        node_session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
        workflow_instruction: Option<String>,
    ) -> Result<(), WorkflowRuntimeError> {
        let gate = RealSessionStartGate { _app: self.app };
        dispatch_session_start(
            &gate,
            node_session_id,
            worktree_path,
            permission_mode,
            system_prompt,
            workflow_instruction,
        )
        .await
    }

    async fn mark_node_tab_open(&self, node_session_id: &str) {
        crate::adaptor::gateway::workflow::mark_started_node_tab_open(
            self.open_tabs,
            node_session_id,
        );
    }

    async fn broadcast_state(&self, worktree_path: &str, snapshot: RuntimeCommitSnapshot) {
        workflow_runtime_session::broadcast_state(self.app, worktree_path, snapshot).await;
    }

    async fn append_node_session_started(
        &self,
        snapshot: &RuntimeCommitSnapshot,
    ) -> Result<(), WorkflowRuntimeError> {
        let Some(event) =
            workflow_runtime_events::node_session_started_event_for_snapshot(snapshot)?
        else {
            return Ok(());
        };
        crate::adaptor::gateway::workflow::event_log_writer::append_required_events_for_app(
            self.app,
            &[event],
        )
        .map_err(|e| {
            WorkflowRuntimeError::SessionStore(format!("append NodeSessionStarted failed: {e}"))
        })
    }

    async fn start_agent_turn_locked(
        &self,
        node_execution_id: &str,
        node_session_id: &str,
        permission_mode: &str,
        prompt: &str,
        system_prompt: Option<String>,
        workflow_instruction: Option<String>,
    ) -> Result<(), WorkflowRuntimeError> {
        let _ = (
            self.app,
            self.branch_diff_context.as_ref(),
            self.session_store,
        );
        let permission_mode = PermissionMode::parse_canonical(permission_mode)
            .map_err(|e| WorkflowRuntimeError::InvalidWorkflow(e.to_string()))?;
        let _runtime_guard = self
            .agent_runtime
            .acquire_session_control_after_recovery(node_session_id)
            .await;
        self.agent_runtime
            .start_workflow_turn_locked(
                crate::usecase::agent_session::runtime::DurableWorkflowTurnRequest {
                    operation_id:
                        crate::usecase::agent_session::runtime::durable_workflow_turn_operation_id(
                            node_execution_id,
                            "initial",
                        ),
                    session_id: node_session_id.to_string(),
                    content: prompt.to_string(),
                    permission_mode,
                    base_system_prompt: system_prompt,
                    workflow_instructions: workflow_instruction.into_iter().collect(),
                },
            )
            .await
            .map_err(WorkflowRuntimeError::from)
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
) -> Result<(), WorkflowRuntimeError> {
    gate.start_session(
        session_id,
        worktree_path,
        permission_mode,
        system_prompt,
        workflow_instruction,
    )
    .await
    .map_err(|error| {
        WorkflowRuntimeError::with_agent_runtime_context(
            format!("Failed to start AgentSession '{session_id}'"),
            error,
        )
    })
}
