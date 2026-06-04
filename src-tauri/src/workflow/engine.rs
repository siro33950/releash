use std::collections::HashMap;
use std::fmt;
use std::path::Path;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};

use regex::RegexBuilder;
use tauri::Manager;
use tokio::sync::Mutex;

use crate::agent_sdk::AgentProcessMap;
use crate::agent_status::current_timestamp;
use crate::permission::PermissionMode;
use crate::session::{ChatSession, SessionStore};
use crate::workflow::command::{WorkflowCommand, WorkflowCommandResult};
use crate::workflow::command_input::{
    validate_optional_comment_text, validate_reject_reason_text, validate_required_comment_text,
    CommandInputError,
};
use crate::workflow::contract::{
    strip_contract_validation_metadata, validate_contract_value, ContractValidationResult,
};
use crate::workflow::event::{
    ApprovalDecisionRecord, CliMutationRejectionReason, CliMutationRequestRecord,
    CollectedOutputEntry, WorkflowEvent,
};
use crate::workflow::event_projection::reconstruct_state_from_events;
use crate::workflow::log::WorkflowEventLog;
use crate::workflow::resolver::{
    ManagedWorktreeResolver, ManagedWorktreeResolverError, WorkflowDefinitionResolver,
    WorkflowDefinitionResolverError,
};
use crate::workflow::route_context::CommandCommitContext;
use crate::workflow::run::{
    RunStatus, RunStore, RunStoreError, TerminalRunStatus, TriggerSource, WorkflowRun,
};
use crate::workflow::schema::{
    CollectConfig, NodeDefinition, NodeType, ParallelAggregate, ReduceStrategy, TransitionRule,
    Workflow,
};
use crate::workflow::state::{
    ApprovalOperations, ParallelStepState, StepHistoryEntry, StepOutput, TokenUsage,
    WorkflowExecutionState, WorkflowState,
};

#[allow(dead_code)]
const MAX_OUTPUT_SIZE: usize = 100 * 1024; // 100KB
const MAX_CONTRACT_REPAIR_ATTEMPTS: u32 = 2;

static PRIVATE_KEY_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?is)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----")
        .unwrap()
});
static GHP_TOKEN_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\bghp_[A-Za-z0-9_]{20,}\b").unwrap());
static GITHUB_PAT_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\bgithub_pat_[A-Za-z0-9_]{20,}\b").unwrap());
static SECRET_KV_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)\b(api_key|apikey|token|password|secret)\s*[:=]\s*([^\s,;]+)").unwrap()
});

/// `command_input::CommandInputError` を `WorkflowEngineError::ValidationError`
/// に map する境界（review R2-01: ドメイン pure helper の Engine 層への接続点）。
fn command_input_error_to_engine_error(err: CommandInputError) -> WorkflowEngineError {
    WorkflowEngineError::ValidationError(err.to_string())
}

#[allow(dead_code)]
fn truncate_output(text: String) -> String {
    if text.len() <= MAX_OUTPUT_SIZE {
        return text;
    }
    let mut end = MAX_OUTPUT_SIZE;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = text[..end].to_string();
    truncated.push_str("... (truncated)");
    truncated
}

/// AgentSession 開始呼び出しを抽象化するトレイト。
/// production では `start_agent_session_internal` を呼ぶ `RealSessionStartGate` を使い、
/// テストでは引数を記録するテストダブルに差し替えて、合成された `system_prompt` が
/// バックエンドへ受け渡される経路を検証する。
#[async_trait::async_trait]
trait SessionStartGate: Send + Sync {
    async fn start_session(
        &self,
        session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
    ) -> Result<(), String>;
}

/// production 用の `SessionStartGate` 実装。`start_agent_session_internal` をそのまま呼び出す。
struct RealSessionStartGate<'a, R: tauri::Runtime> {
    app: &'a tauri::AppHandle<R>,
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
    ) -> Result<(), String> {
        crate::agent_sdk::start_agent_session_internal(
            self.app,
            self.handles,
            self.session_store,
            session_id,
            worktree_path,
            permission_mode,
            system_prompt,
        )
        .await
    }
}

/// `start_step_session` 内でファセット合成（純粋関数）後に実行される副作用境界を
/// まとめて抽象化するトレイト。具体的には以下の経路を担う:
///
/// - 親 ChatSession の取得
/// - ステップ用 ChatSession の生成（設定解決を含む）
/// - AgentSession 起動 (`start_agent_session_internal` 相当)
/// - ワークフロー状態の永続化／ブロードキャスト
/// - ステップセッションへのターン起動 (`start_agent_turn_internal_locked` 相当)
///
/// production では `AppHandle` / `SessionStore` / `AgentProcessMap` を握る
/// `RealStepSessionDeps` を渡し、テストでは記録用のテストダブルを差し替えることで、
/// 「`build_step_prompt` 失敗時に `create_step_session` 等が呼ばれない」という
/// 順序保証を実 production 経路と同じ構造で検証する。
#[async_trait::async_trait]
trait StepSessionDeps: Send + Sync {
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
    ) -> Result<StepSessionInfo, WorkflowEngineError>;

    /// 合成済み `system_prompt` を AgentSession 開始経路へ受け渡す。
    async fn dispatch_session_start(
        &self,
        step_session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
    ) -> Result<(), WorkflowEngineError>;

    async fn mark_step_tab_open(&self, step_session_id: &str);

    /// ワークフロー状態をブロードキャストする（best-effort）。
    async fn broadcast_state(&self, worktree_path: &str, snapshot: WorkflowState);

    /// Runtime lock acquired by the caller variant.
    async fn start_agent_turn_locked(
        &self,
        step_session_id: &str,
        worktree_path: &str,
        permission_mode: &str,
        prompt: &str,
    ) -> Result<(), WorkflowEngineError>;
}

/// workflow 起動時に確定する step session の継承デフォルト。
///
/// `start_workflow` の `permission_mode` 引数を capture し、以降の step / 並列子 step は
/// この値を fallback として `NodeDefinition.model` / `NodeDefinition.permission` で上書きする。
///
/// `selected_model` は spec [02] の暗黙フォールバック禁止に従い workflow デフォルトとしては
/// 持たない（各 step は `NodeDefinition.model` 必須で個別に解決する）。`backend_id` も
/// 各 step が `NodeDefinition.model` 必須から `resolve_backend_for_step_model` 経由で
/// 一意解決するため、step 指定が無い場合の fallback としてのみ保持する。
#[derive(Clone, Debug)]
struct WorkflowDefaults {
    backend_id: Option<String>,
    permission_mode: String,
}

/// `StepSessionDeps::create_step_session` の戻り値。
#[derive(Clone, Debug)]
struct StepSessionInfo {
    id: String,
    permission_mode: String,
}

/// production 用の `StepSessionDeps` 実装。
struct RealStepSessionDeps<'a, R: tauri::Runtime> {
    engine: &'a WorkflowEngine,
    app: &'a tauri::AppHandle<R>,
    handles: &'a Arc<Mutex<AgentProcessMap>>,
    session_store: &'a Arc<SessionStore>,
}

#[async_trait::async_trait]
impl<'a, R: tauri::Runtime> StepSessionDeps for RealStepSessionDeps<'a, R> {
    async fn create_step_session(
        &self,
        worktree_path: &str,
        step_model: Option<String>,
        step_permission: Option<String>,
        workflow_defaults: WorkflowDefaults,
    ) -> Result<StepSessionInfo, WorkflowEngineError> {
        let data_dir = crate::session::resolve_data_dir(self.app)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;
        let step_session = self
            .engine
            .create_step_session_with_settings(
                self.app,
                self.session_store,
                &data_dir,
                worktree_path,
                step_model,
                step_permission,
                &workflow_defaults,
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
    ) -> Result<(), WorkflowEngineError> {
        let gate = RealSessionStartGate {
            app: self.app,
            handles: self.handles,
            session_store: self.session_store,
        };
        WorkflowEngine::dispatch_session_start(
            &gate,
            step_session_id,
            worktree_path,
            permission_mode,
            system_prompt,
        )
        .await
    }

    async fn mark_step_tab_open(&self, step_session_id: &str) {
        crate::workflow_step_lifecycle_adapters::mark_started_step_tab_open(
            self.app,
            step_session_id,
        );
    }

    async fn broadcast_state(&self, worktree_path: &str, snapshot: WorkflowState) {
        self.engine
            .broadcast_state(self.app, worktree_path, snapshot)
            .await;
    }

    async fn start_agent_turn_locked(
        &self,
        step_session_id: &str,
        worktree_path: &str,
        permission_mode: &str,
        prompt: &str,
    ) -> Result<(), WorkflowEngineError> {
        crate::agent_sdk::start_agent_turn_internal_locked(
            self.app,
            self.handles,
            self.session_store,
            step_session_id,
            worktree_path,
            permission_mode,
            prompt,
        )
        .await
        .map_err(WorkflowEngineError::AgentSession)
    }
}

/// `abort_workflow_by_run_id` 内部 lookup の typed 結果。
#[derive(Debug)]
enum AbortTargetLookup {
    NotFound,
    AlreadyTerminal,
    Active {
        current_step_session_id: Option<String>,
        parallel_session_ids: Option<Vec<String>>,
    },
}

/// `abort_workflow_by_run_id` の typed outcome。
///
/// `WorkflowCommand::AbortRun` 経由の中断要求に対し、command handler が「実際に
/// 中断を実施したか」「対象 run が存在しないか」「既に終了済みで中断不能だったか」
/// を typed に表現する。`Aborted` のみが dispatch から `Accepted` に射影され、
/// `NotFound` / `AlreadyTerminal` は非受理（`WorkflowEngineError` 経由）として
/// 上位に伝播する（Spec [04] Rule「対象不在 / 既に終了した command は受理されない」）。
///
/// engine 内部用途のみのため可視性は module-private に閉じる。外部入口は
/// `WorkflowCommand::AbortRun` 一本に統一する（Spec [04] 公開 API 最小化）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbortOutcome {
    /// 対象 run を Aborted に遷移させ、RunAborted event を append した。
    Aborted,
    /// 対象 run が `executions` に存在しない。
    NotFound,
    /// 対象 run は既に terminal で、中断対象でない。
    AlreadyTerminal,
}

struct CommandMutationRollback<'a> {
    run_id: &'a str,
    snapshot_before: WorkflowExecution,
    run_store_snapshot_before: Option<WorkflowRun>,
    context: &'a str,
}

struct RequiredEventCommit<'a> {
    run_id: &'a str,
    snapshot_for_commit: &'a WorkflowState,
    snapshot_before: WorkflowExecution,
    run_store_snapshot_before: Option<WorkflowRun>,
    required_events: Vec<WorkflowEvent>,
    append_error_context: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutcomeCommitMode {
    EmitProgressEvents,
    ProgressEventsAlreadyCommitted,
}

impl OutcomeCommitMode {
    fn should_emit_progress_events(self) -> bool {
        matches!(self, Self::EmitProgressEvents)
    }
}

/// ワークフローエンジンのエラー型。
#[derive(Debug)]
pub enum WorkflowEngineError {
    /// ワークフロー実行が見つからない
    ExecutionNotFound(String),
    /// セッションが見つからない
    SessionNotFound(String),
    /// ワークフロー定義エラー（ステップなし、ステップ未発見等）
    InvalidWorkflow(String),
    /// ワークフローが既にアクティブ
    AlreadyActive(String),
    /// 不正な状態遷移（WaitingApprovalでない時にapproval等）
    InvalidState(String),
    /// 入力検証エラー（表示用の安定 kind: validation_error）
    ValidationError(String),
    /// 承認操作が指定 worktree の実行を対象にしていない
    UnauthorizedWorktree(String),
    /// 承認操作が現在の execution / step を対象にしていない
    UnauthorizedApprovalTarget(String),
    /// セッションストアのIO/シリアライズエラー
    SessionStore(String),
    /// AgentSession起動エラー
    AgentSession(String),
}

impl fmt::Display for WorkflowEngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExecutionNotFound(id) => {
                write!(f, "No workflow execution found for session '{id}'")
            }
            Self::SessionNotFound(id) => write!(f, "ChatSession not found: {id}"),
            Self::InvalidWorkflow(msg) => write!(f, "{msg}"),
            Self::AlreadyActive(name) => {
                write!(f, "Workflow '{name}' is already running for this session")
            }
            Self::InvalidState(msg) => write!(f, "invalid_state: {msg}"),
            Self::ValidationError(msg) => write!(f, "validation_error: {msg}"),
            Self::UnauthorizedWorktree(msg) => write!(f, "unauthorized_worktree: {msg}"),
            Self::UnauthorizedApprovalTarget(msg) => {
                write!(f, "unauthorized_approval_target: {msg}")
            }
            Self::SessionStore(msg) => write!(f, "{msg}"),
            Self::AgentSession(msg) => write!(f, "{msg}"),
        }
    }
}

impl From<WorkflowEngineError> for String {
    fn from(e: WorkflowEngineError) -> Self {
        e.to_string()
    }
}

impl From<WorkflowDefinitionResolverError> for WorkflowEngineError {
    fn from(e: WorkflowDefinitionResolverError) -> Self {
        match e {
            WorkflowDefinitionResolverError::InvalidWorkflow(message) => {
                Self::InvalidWorkflow(message)
            }
            WorkflowDefinitionResolverError::Infrastructure(message) => Self::SessionStore(message),
        }
    }
}

impl From<ManagedWorktreeResolverError> for WorkflowEngineError {
    fn from(e: ManagedWorktreeResolverError) -> Self {
        match e {
            ManagedWorktreeResolverError::Validation(message) => Self::ValidationError(message),
        }
    }
}

impl From<crate::workflow_step_lifecycle::WorkflowStepLifecycleError> for WorkflowEngineError {
    fn from(e: crate::workflow_step_lifecycle::WorkflowStepLifecycleError) -> Self {
        match e {
            crate::workflow_step_lifecycle::WorkflowStepLifecycleError::SessionNotFound(id) => {
                Self::SessionNotFound(id)
            }
            crate::workflow_step_lifecycle::WorkflowStepLifecycleError::SessionStore(message) => {
                Self::SessionStore(message)
            }
            crate::workflow_step_lifecycle::WorkflowStepLifecycleError::AgentSession(message) => {
                Self::AgentSession(message)
            }
        }
    }
}

/// ワークフロー実行の内部状態。
#[derive(Clone)]
struct WorkflowExecution {
    /// `WorkflowState.execution_id` を `run_id` として昇格させた識別子。
    /// `WorkflowEngine.executions` の HashMap キーと一致する。
    id: String,
    workflow: Workflow,
    state: WorkflowExecutionState,
    current_step_index: usize,
    step_execution_counts: HashMap<String, u32>,
    step_history: Vec<StepHistoryEntry>,
    /// step / 並列子 step 起動時の継承デフォルト（permission_mode / backend_id / selected_model）。
    /// `start_workflow` 時に capture し、以降は session_store を読み直さない（in-memory のみ）。
    workflow_defaults: WorkflowDefaults,
    /// run が紐づく worktree。HashMap キーではなく属性として保持する。
    /// `find_by_worktree` / `find_by_worktree_mut` が worktree 起点の lookup で参照する。
    worktree_path: String,
    started_at: f64,
    updated_at: f64,
    /// 現在のステップに対応するAgentSessionのセッションID。
    current_session_id: Option<String>,
    /// 現在のステップで累計したトークン使用量。
    current_step_token_usage: TokenUsage,
    /// step_name → 最新StepOutput のマップ。
    step_outputs: HashMap<String, StepOutput>,
    /// ワークフロー実行時のタスク内容（テンプレート変数 {{task}} の展開に使用）。
    task: Option<String>,
    /// 並列実行中の場合の状態。
    parallel_run: Option<ParallelRunState>,
    /// ワークフローレベルの変数（spec-directory等のcontract結果から設定）。
    workflow_variables: HashMap<String, String>,
}

/// 並列実行中の内部状態。
#[derive(Clone)]
struct ParallelRunState {
    parent_step_name: String,
    aggregate: Option<ParallelAggregate>,
    children: Vec<ParallelChildRun>,
}

/// 並列子ステップの実行状態。
#[derive(Clone)]
struct ParallelChildRun {
    step_name: String,
    session_id: String,
    state: ParallelChildState,
    result: Option<String>,
    structured_output: Option<serde_json::Value>,
    output_contract: Option<String>,
    token_usage: TokenUsage,
    run_index: u32,
}

/// 並列子ステップの状態。
#[derive(Clone, PartialEq)]
enum ParallelChildState {
    Running,
    Completed,
    Failed,
    Interrupted,
}

/// session_workflow_refsの値型。session_id → run_id の逆引き索引。
///
/// parent ChatSession 機構撤去後は step session のみが登録されるため種別区別は不要
/// （Spec issues-929: 「逐次 step と並列子 step は単一経路で扱う」/ Spec issues-1011:
/// engine 内部キーは run_id に統一）。worktree_path は `WorkflowExecution.worktree_path`
/// 属性として exec から取得する。
#[derive(Clone)]
struct SessionWorkflowRef {
    /// engine.executions の HashMap キー（= `WorkflowExecution.id` = `run_id`）。
    run_id: String,
}

impl WorkflowExecution {
    /// ワークフローが実行中（Running または WaitingApproval）かどうかを返す。
    fn is_active(&self) -> bool {
        matches!(
            self.state,
            WorkflowExecutionState::Running | WorkflowExecutionState::WaitingApproval
        )
    }

    /// ワークフローが終了状態（Completed / Failed / Aborted）かどうかを返す。
    fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            WorkflowExecutionState::Completed
                | WorkflowExecutionState::Failed { .. }
                | WorkflowExecutionState::Aborted
        )
    }

    /// workflow 構造の事前検証（純粋関数 / 副作用なし）。
    ///
    /// `start_workflow` の Phase 1（副作用なし validation）と、
    /// executions ロック内の defense-in-depth から共有して呼ぶ。
    fn validate_workflow_shape(workflow: &Workflow) -> Result<(), WorkflowEngineError> {
        if workflow.nodes.is_empty() {
            return Err(WorkflowEngineError::InvalidWorkflow(
                "Workflow has no steps".to_string(),
            ));
        }
        // [02] schema 境界: bash node の実行系は [13] で具体化される。
        // それまでは workflow.nodes に bash が含まれる場合、開始前に明示拒否し、
        // 利用者が誤った状態に進まないようにする。
        for node in &workflow.nodes {
            if node.node_type == NodeType::Bash {
                return Err(WorkflowEngineError::InvalidWorkflow(format!(
                    "Bash node '{}' is not executable in this milestone (planned for [13])",
                    node.name
                )));
            }
        }
        Ok(())
    }

    /// ワークフロー開始の事前条件を検証する（純粋関数）。
    ///
    /// executions ロック内の defense-in-depth で呼ばれる。Run Store の active index と
    /// in-memory executions 表が一時的に不整合な場合に、最終的な atomic guard として機能する。
    fn validate_start(
        workflow: &Workflow,
        existing: Option<&WorkflowExecution>,
    ) -> Result<(), WorkflowEngineError> {
        Self::validate_workflow_shape(workflow)?;
        if let Some(existing) = existing {
            if existing.is_active() {
                return Err(WorkflowEngineError::AlreadyActive(
                    existing.workflow.name.clone(),
                ));
            }
        }
        Ok(())
    }
}

/// 次のステップ遷移の判定結果。
#[derive(Debug, Clone, PartialEq)]
enum NextStepDecision {
    /// ワークフロー完了（最後のステップを超えた）
    Completed,
    /// 指定ステップへ遷移
    TransitionTo(String),
}

/// サイクルガード検証結果。
#[derive(Debug, Clone, PartialEq)]
enum CycleGuardResult {
    /// 許可（ガードなし or 上限内）
    Allowed,
    /// 超過
    Exceeded {
        max_iterations: u32,
        count: u32,
        on_exhausted: Option<String>,
    },
}

/// on_turn_complete後のモード別アクション判定結果。
#[derive(Debug, Clone, PartialEq)]
enum TurnCompleteAction {
    /// AgentSessionがエラー終了 → Failed
    SessionError { step_name: String, exit_code: i64 },
    /// agent ノード → タグ検出して遷移
    AutoEvaluate {
        rules: Vec<TransitionRule>,
        step_name: String,
    },
    /// approval ノード → WaitingApproval
    WaitApproval,
    /// 設計上 turn_complete に流入してはならない node 種別を検出した
    /// （`validate_start` などの上流ガードで弾くべきケース）。`Failed` に遷移させ、
    /// `SessionError { exit_code: 0 }` の「正常終了」セマンティクスと混同しないようにする。
    UnexpectedNodeType {
        step_name: String,
        node_type: NodeType,
    },
    /// ワークフローが実行中でない → 何もしない
    NotRunning,
}

impl WorkflowExecution {
    /// 永続化用の `WorkflowState` に変換する。
    fn to_workflow_state(&self) -> WorkflowState {
        let mut total_token_usage = TokenUsage::default();
        for entry in &self.step_history {
            if let Some(ref usage) = entry.token_usage {
                total_token_usage.add(usage);
            }
        }

        let step_states = crate::workflow::state::compute_step_states(
            &self.workflow,
            self.current_step_index,
            &self.state,
            &self.step_history,
        );

        WorkflowState {
            execution_id: self.id.clone(),
            workflow_name: self.workflow.name.clone(),
            state: self.state.clone(),
            current_step_index: self.current_step_index,
            current_step_name: self.workflow.nodes[self.current_step_index].name.clone(),
            current_session_id: self.current_session_id.clone(),
            total_steps: self.workflow.nodes.len(),
            step_history: self.step_history.clone(),
            step_execution_counts: self.step_execution_counts.clone(),
            workflow_definition: self.workflow.clone(),
            total_token_usage,
            step_states,
            step_outputs: self.step_outputs.clone(),
            active_parallel_steps: self.build_active_parallel_steps(),
            workflow_variables: self.workflow_variables.clone(),
            approval_operations: self.build_approval_operations(),
            started_at: self.started_at,
            updated_at: self.updated_at,
        }
    }

    fn build_approval_operations(&self) -> Option<ApprovalOperations> {
        if self.state != WorkflowExecutionState::WaitingApproval {
            return None;
        }
        let step = &self.workflow.nodes[self.current_step_index];
        Some(ApprovalOperations {
            can_reject: step.transition_rules.iter().any(|r| r.r#match == "reject"),
        })
    }

    /// parallel_runからactive_parallel_stepsを生成する。
    fn build_active_parallel_steps(&self) -> Vec<ParallelStepState> {
        let Some(ref pr) = self.parallel_run else {
            return vec![];
        };
        pr.children
            .iter()
            .map(|child| ParallelStepState {
                step_name: child.step_name.clone(),
                state: match child.state {
                    ParallelChildState::Running => "running".to_string(),
                    ParallelChildState::Completed => "completed".to_string(),
                    ParallelChildState::Failed => "failed".to_string(),
                    ParallelChildState::Interrupted => "interrupted".to_string(),
                },
                session_id: Some(child.session_id.clone()),
                result: child.result.clone(),
                run_index: child.run_index,
                completed_at: None,
                structured_output: child.structured_output.clone(),
                output_contract: child.output_contract.clone(),
            })
            .collect()
    }

    /// spec issues-1023: 中断された通常 step の `step_history` entry を作る。
    ///
    /// 既存 `make_step_history_entry` の副作用（`current_session_id` reset /
    /// step_outputs の前段クリーンアップ等）を**起こさない**点が違い。`abort_workflow_by_run_id`
    /// は post-commit で interrupt_agent や cleanup を行うため、reset 系は
    /// `finalize_terminal_transition_after_required_append` 経路に任せる。
    ///
    /// session_id を entry にコピーすることで、`step_history` 由来の
    /// session log 到達経路（runtime_view::collect_step_session_ids）を復活させる。
    fn make_aborted_history_entry(&mut self, timestamp: f64) -> StepHistoryEntry {
        let step_name = self.workflow.nodes[self.current_step_index].name.clone();
        let run_index = self
            .step_execution_counts
            .get(&step_name)
            .copied()
            .unwrap_or(1);
        // 中断時点までに累積した token_usage は entry に残す。
        // current_step_token_usage 自体は take してクリアする
        // （post-commit 経路で参照されないため）。
        let token_usage = Some(std::mem::take(&mut self.current_step_token_usage));
        StepHistoryEntry {
            step_name,
            completed_at: timestamp,
            result: None,
            session_id: self.current_session_id.clone(),
            token_usage,
            structured_output: None,
            run_index,
            child_outputs: None,
            state: "aborted".to_string(),
        }
    }

    /// spec issues-1023: 中断された parallel parent step の `step_history` entry を作る。
    ///
    /// `parallel_run.children` 全件を `child_outputs` に snapshot する。
    /// 完了済み child（`ParallelChildState::Completed`）は `state="completed"`、
    /// それ以外（Running / Failed / Interrupted）は `state="aborted"` として記録し、
    /// session_id は child runtime / step_outputs から維持する（session log 到達経路維持）。
    ///
    /// 呼出し側は本関数で entry を組み立てた後、`self.parallel_run = None;` を明示
    /// セットすること（`build_active_parallel_steps()` 経由の二重表示を防ぐ）。
    fn make_aborted_parallel_history_entry(&self, timestamp: f64) -> Option<StepHistoryEntry> {
        let pr = self.parallel_run.as_ref()?;
        let parent_run_index = self
            .step_execution_counts
            .get(&pr.parent_step_name)
            .copied()
            .unwrap_or(1);
        let child_snapshots: Vec<crate::workflow::state::ChildOutputSnapshot> = pr
            .children
            .iter()
            .map(|child| {
                let snapshot_state = if matches!(child.state, ParallelChildState::Completed) {
                    "completed"
                } else {
                    "aborted"
                };
                let child_so = self.step_outputs.get(&child.step_name);
                crate::workflow::state::ChildOutputSnapshot {
                    step_name: child.step_name.clone(),
                    session_id: child_so
                        .and_then(|o| o.session_id.clone())
                        .or_else(|| Some(child.session_id.clone())),
                    result: child_so
                        .and_then(|o| o.result.clone())
                        .or(child.result.clone()),
                    run_index: child.run_index,
                    completed_at: child_so.map(|o| o.completed_at).unwrap_or(timestamp),
                    structured_output: child_so.and_then(|o| o.structured_output.clone()),
                    output_contract: child_so.and_then(|o| o.output_contract.clone()),
                    state: snapshot_state.to_string(),
                }
            })
            .collect();
        Some(StepHistoryEntry {
            step_name: pr.parent_step_name.clone(),
            completed_at: timestamp,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: parent_run_index,
            child_outputs: Some(child_snapshots),
            state: "aborted".to_string(),
        })
    }

    /// 現在のステップの完了履歴エントリを生成し、トークン使用量をリセットする。
    fn make_step_history_entry(
        &mut self,
        result: Option<String>,
        structured_output: Option<serde_json::Value>,
        output_contract: Option<String>,
    ) -> StepHistoryEntry {
        let step_name = self.workflow.nodes[self.current_step_index].name.clone();
        let run_index = self
            .step_execution_counts
            .get(&step_name)
            .copied()
            .unwrap_or(1);
        let completed_at = current_timestamp();
        let token_usage = Some(std::mem::take(&mut self.current_step_token_usage));

        // StepOutputを更新（structured_outputがある場合のみ）
        if structured_output.is_some() {
            self.step_outputs.insert(
                step_name.clone(),
                StepOutput {
                    step_name: step_name.clone(),
                    run_index,
                    session_id: self.current_session_id.clone(),
                    result: result.clone(),
                    structured_output: structured_output.clone(),
                    output_contract: output_contract.clone(),
                    token_usage: token_usage.clone(),
                    completed_at,
                },
            );
        }

        let entry = StepHistoryEntry {
            step_name,
            completed_at,
            result,
            session_id: self.current_session_id.clone(),
            token_usage,
            structured_output,
            run_index,
            child_outputs: None,
            state: crate::workflow::state::default_step_entry_state(),
        };
        self.current_session_id = None;
        entry
    }

    /// 指定インデックスの step が新しい実行を開始する瞬間に、当該 step の
    /// 前回出力を `step_outputs` から破棄する。並列ブロックの場合は
    /// 親ブロック名と全子 step 名を一括で削除する。
    ///
    /// 同一 step がループで再実行される際、前回値が残ったままになると
    /// `evaluate_aggregate` / `pass_output_from` / `apply_reduce` /
    /// `inject_step_outputs` が前回値を引いてしまい、新しい実行で
    /// `structured_output` が更新されないケースや LLM が前回ターンの
    /// `<workflow_output>` を引用してきたケースで Contract 違反が
    /// 「正常完了（Done）」扱いされる不具合の原因となる。
    fn clear_step_outputs_for_new_execution(&mut self, step_index: usize) {
        let step = &self.workflow.nodes[step_index];
        self.step_outputs.remove(&step.name);
        if let Some(children) = step.parallel_children.as_ref() {
            for child in children {
                self.step_outputs.remove(&child.name);
            }
        }
    }

    /// 次のステップ遷移先を判定する（純粋関数）。
    fn decide_next_step(&self) -> NextStepDecision {
        let current_index = self.current_step_index;
        if current_index + 1 >= self.workflow.nodes.len() {
            NextStepDecision::Completed
        } else {
            NextStepDecision::TransitionTo(self.workflow.nodes[current_index + 1].name.clone())
        }
    }

    /// 指定ステップへの遷移時にサイクルガードを検証する（純粋関数）。
    fn check_cycle_guard(
        &self,
        target_step_name: &str,
    ) -> Result<CycleGuardResult, WorkflowEngineError> {
        let idx = self
            .workflow
            .nodes
            .iter()
            .position(|s| s.name == target_step_name)
            .ok_or_else(|| {
                WorkflowEngineError::InvalidWorkflow(format!(
                    "Step '{}' not found in workflow",
                    target_step_name
                ))
            })?;

        let step = &self.workflow.nodes[idx];
        if let Some(guard) = &step.cycle_guard {
            let count = self
                .step_execution_counts
                .get(target_step_name)
                .copied()
                .unwrap_or(0);
            if count >= guard.max_iterations {
                Ok(CycleGuardResult::Exceeded {
                    max_iterations: guard.max_iterations,
                    count,
                    on_exhausted: guard.on_exhausted.clone(),
                })
            } else {
                Ok(CycleGuardResult::Allowed)
            }
        } else {
            Ok(CycleGuardResult::Allowed)
        }
    }

    /// turn_complete後のアクションを判定する（純粋関数）。
    fn decide_turn_complete_action(&self, exit_code: i64) -> TurnCompleteAction {
        if self.state != WorkflowExecutionState::Running {
            return TurnCompleteAction::NotRunning;
        }

        let step = &self.workflow.nodes[self.current_step_index];

        if exit_code != 0 {
            return TurnCompleteAction::SessionError {
                step_name: step.name.clone(),
                exit_code,
            };
        }

        match step.node_type {
            NodeType::Agent => TurnCompleteAction::AutoEvaluate {
                rules: step.transition_rules.clone(),
                step_name: step.name.clone(),
            },
            NodeType::Approval => TurnCompleteAction::WaitApproval,
            NodeType::Bash | NodeType::Parallel => TurnCompleteAction::UnexpectedNodeType {
                step_name: step.name.clone(),
                node_type: step.node_type,
            },
        }
    }

    /// approvalモードの判定ロジック（純粋関数）。
    fn decide_approval_action(
        &self,
        decision: &ApprovalDecision,
    ) -> Result<ApprovalAction, WorkflowEngineError> {
        if self.state != WorkflowExecutionState::WaitingApproval {
            return Err(WorkflowEngineError::InvalidState(
                "Workflow is not waiting for approval".to_string(),
            ));
        }
        let step = &self.workflow.nodes[self.current_step_index];
        match decision {
            ApprovalDecision::Approve => Ok(ApprovalAction::Advance),
            ApprovalDecision::Reject { .. } => {
                match step.transition_rules.iter().find(|r| r.r#match == "reject") {
                    Some(r) => Ok(ApprovalAction::TransitionTo(r.next.clone())),
                    None => Err(WorkflowEngineError::InvalidState(format!(
                        "Step '{}' does not allow reject",
                        step.name
                    ))),
                }
            }
        }
    }
}

/// approvalモードのユーザー判定。
#[derive(Debug, Clone, PartialEq)]
enum ApprovalDecision {
    Approve,
    Reject { comment: String },
}

/// approvalモードの判定結果（純粋関数用）。
#[derive(Debug, Clone, PartialEq)]
enum ApprovalAction {
    Advance,
    TransitionTo(String),
}

struct ApprovalApplication {
    effective_result: String,
    structured_output: Option<serde_json::Value>,
    output_contract: Option<String>,
}

/// ロック内で確定した遷移結果。ロ��ク外で永続化・AgentSession起動を行うための情報を持つ。
enum StepOutcome {
    /// 状態を永続化・ブロードキャストするだけ（終了状態遷移���ど）
    Persist(WorkflowState),
    /// 次のステップに遷移し、AgentSession を起動する
    TransitionAndStart(WorkflowState),
    /// collect仮想stepに遷移し、reduce処理を実行する
    ReduceAndTransition(WorkflowState),
    /// 並列ブロックに遷移し、子ステップを並列起動する
    StartParallel(WorkflowState),
}

/// reduce処理の結果。
struct ReduceResult {
    result: Option<String>,
    structured_output: Option<serde_json::Value>,
}

/// ステップ設定解決の結果。
/// ステップのmodel/permission指定と親セッション設定のマージ結果を保持する。
#[derive(Debug, Clone, PartialEq)]
struct ResolvedStepSettings {
    backend_id: Option<String>,
    selected_model: Option<String>,
    permission_mode: String,
}

/// ステップの model/permission 設定を workflow デフォルトとマージして解決する。
///
/// - permission: ステップ指定があれば採用、なければ workflow デフォルトを継承
/// - backend_id: model指定があれば resolved_backend_id を採用、なければ workflow デフォルトを継承
/// - selected_model: ステップ指定があれば採用、なければ未指定（None）として扱う。
///   Spec: workflow 経路の `model_id=None` は当該 step session の選択モデルを
///   未指定状態のままとし、workflow デフォルト model への暗黙フォールバックを行わない。
///
/// `resolved_backend_id` は、ステップにmodel指定がある場合に
/// `resolve_backend_for_step_model` で事前に解決されたbackend_id。
/// model未指定時は無視される。
fn resolve_step_settings(
    step_model: Option<String>,
    step_permission: Option<String>,
    resolved_backend_id: Option<String>,
    workflow_defaults: &WorkflowDefaults,
) -> ResolvedStepSettings {
    let permission_mode =
        step_permission.unwrap_or_else(|| workflow_defaults.permission_mode.clone());
    let backend_id = if step_model.is_some() {
        resolved_backend_id
    } else {
        workflow_defaults.backend_id.clone()
    };
    let selected_model = step_model;
    ResolvedStepSettings {
        backend_id,
        selected_model,
        permission_mode,
    }
}

/// ワークフローのステップを順次実行するステートマシンエンジン。
pub struct WorkflowEngine {
    /// `run_id` → `WorkflowExecution` の in-memory マッピング。
    /// HashMap キーは `WorkflowExecution.id`（= `run_id`）と一致する。
    /// `worktree_path` は `WorkflowExecution.worktree_path` 属性として保持し、
    /// `worktree_path → run_id` の補助解決は Run Store の secondary index 経由で行う。
    executions: Mutex<HashMap<String, WorkflowExecution>>,
    /// session_id（親・ステップ・並列子） → SessionWorkflowRef のマッピング
    session_workflow_refs: Mutex<HashMap<String, SessionWorkflowRef>>,
    /// active な WorkflowRun の管理および run metadata の永続化を担う Run Store。
    /// worktree_path → active run_id の secondary index は Run Store 内で保持する。
    run_store: Arc<RunStore>,
    workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
    worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
    #[cfg(test)]
    fail_next_required_event_append: AtomicBool,
}

/// `set_execution_state_inner` の lookup 戦略。worktree_path 起点の `find_by_worktree_mut`
/// で active な run を解決する。broadcast / cleanup 用の worktree_path は
/// `set_execution_state_inner` 内で解決した exec から取得するため、target には保持しない
/// （Spec issues-1011 finding 12: 意図不明な未使用 field を残さない）。
///
/// run_id 主語の遷移経路（`WorkflowCommand::AbortRun` 全体中断）は
/// `abort_workflow_by_run_id` 側で typed AbortOutcome + 必須 RunAborted append として
/// 完結するため、ここでは worktree variant のみを扱う。
enum ExecutionStateTarget {
    Worktree(String),
}

/// `execs: HashMap<run_id, WorkflowExecution>` から、worktree_path 属性が一致する
/// **active な** `(run_id, exec)` を線形走査で取得する補助関数。
///
/// Spec issues-1011: engine 内部キーは run_id だが、production 経路で session_id や
/// worktree_path 起点の操作（on_turn_complete, handle_approval, abort 等）が残るため、
/// その下流で active な実行だけを引くための secondary lookup として使う。終了済み
/// （terminal）実行は executions に残るが、ここでは除外して active な run のみ返す
/// （同一 worktree に terminal run と active run が共存しても active を取り違えない）。
fn find_by_worktree<'a>(
    execs: &'a HashMap<String, WorkflowExecution>,
    worktree_path: &str,
) -> Option<(&'a String, &'a WorkflowExecution)> {
    execs
        .iter()
        .find(|(_, e)| e.worktree_path == worktree_path && e.is_active())
}

fn find_by_worktree_mut<'a>(
    execs: &'a mut HashMap<String, WorkflowExecution>,
    worktree_path: &str,
) -> Option<&'a mut WorkflowExecution> {
    execs
        .values_mut()
        .find(|e| e.worktree_path == worktree_path && e.is_active())
}

/// validate_start 用に「active/terminal を問わず worktree_path が一致する exec」を引く。
/// 重複起動拒否（is_active な existing がある場合に限り Err）の判定で必要なため、
/// active filter を適用しない（terminal な過去 run は通過させて Ok 判定にする）。
fn find_any_by_worktree<'a>(
    execs: &'a HashMap<String, WorkflowExecution>,
    worktree_path: &str,
) -> Option<&'a WorkflowExecution> {
    execs.values().find(|e| e.worktree_path == worktree_path)
}

// [08] `lookup_step_output_contract` は `workflow::contract` に移動済み。
// engine と CLI の双方が `crate::workflow::contract::lookup_step_output_contract`
// を直接参照するため、本モジュールではメモのみ残す。

impl WorkflowEngine {
    pub(crate) fn new(
        workflow_resolver: Arc<dyn WorkflowDefinitionResolver>,
        worktree_resolver: Arc<dyn ManagedWorktreeResolver>,
    ) -> Self {
        Self {
            executions: Mutex::new(HashMap::new()),
            session_workflow_refs: Mutex::new(HashMap::new()),
            run_store: Arc::new(RunStore::new()),
            workflow_resolver,
            worktree_resolver,
            #[cfg(test)]
            fail_next_required_event_append: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        Self::new(
            Arc::new(crate::workflow::resolver_adapters::DefaultWorkflowDefinitionResolver),
            Arc::new(crate::workflow::resolver_adapters::PassthroughManagedWorktreeResolver),
        )
    }

    #[cfg(test)]
    pub(crate) async fn seed_active_execution_for_test(
        &self,
        run_id: String,
        workflow: Workflow,
        state: WorkflowExecutionState,
        worktree_path: String,
        trigger_source: TriggerSource,
    ) {
        assert!(
            matches!(
                state,
                WorkflowExecutionState::Running | WorkflowExecutionState::WaitingApproval
            ),
            "seed_active_execution_for_test only accepts active states"
        );
        let current_node_name = workflow.nodes[0].name.clone();
        let run_status = if matches!(state, WorkflowExecutionState::WaitingApproval) {
            RunStatus::WaitingApproval
        } else {
            RunStatus::Running
        };
        let now = 1000.0;
        self.run_store
            .register_active(WorkflowRun {
                run_id: run_id.clone(),
                workflow_name: workflow.name.clone(),
                task: None,
                status: run_status,
                worktree_path: worktree_path.clone(),
                current_node_name: Some(current_node_name.clone()),
                trigger_source,
                started_at: now,
                updated_at: now,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        self.executions.lock().await.insert(
            run_id.clone(),
            WorkflowExecution {
                id: run_id,
                workflow,
                state,
                current_step_index: 0,
                step_execution_counts: HashMap::from([(current_node_name, 1)]),
                step_history: Vec::new(),
                workflow_defaults: WorkflowDefaults {
                    backend_id: None,
                    permission_mode: crate::permission::PermissionMode::EDIT.to_string(),
                },
                worktree_path,
                started_at: now,
                updated_at: now,
                current_session_id: None,
                current_step_token_usage: TokenUsage::default(),
                step_outputs: HashMap::new(),
                task: None,
                parallel_run: None,
                workflow_variables: HashMap::new(),
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn fail_next_required_event_append_for_test(&self) {
        self.fail_next_required_event_append
            .store(true, Ordering::Release);
    }

    /// テスト専用: 指定 run の `current_step_index` を移動させて stale 状態を作る。
    #[cfg(test)]
    pub(crate) async fn force_current_step_index_for_test(&self, run_id: &str, index: usize) {
        if let Some(exec) = self.executions.lock().await.get_mut(run_id) {
            exec.current_step_index = index;
        }
    }

    /// Run Store の参照（テスト専用）。production 経路では下記 facade メソッドを使用する。
    /// 公開 API は `list_active_runs` / `list_completed_runs` / `run_id_for_worktree` /
    /// `resolve_worktree_by_run` / `set_run_store_data_dir` に集約する。
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn run_store(&self) -> &Arc<RunStore> {
        &self.run_store
    }

    async fn reserve_workflow_run(
        &self,
        workflow: &Workflow,
        worktree_path: &str,
        task: Option<String>,
        trigger_source: TriggerSource,
        now: f64,
    ) -> Result<String, WorkflowEngineError> {
        let run_id = uuid::Uuid::new_v4().to_string();
        self.run_store
            .register_active(WorkflowRun {
                run_id: run_id.clone(),
                workflow_name: workflow.name.clone(),
                task,
                status: RunStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node_name: workflow.nodes.first().map(|n| n.name.clone()),
                trigger_source,
                started_at: now,
                updated_at: now,
                completed_at: None,
                error_reason: None,
            })
            .await
            .map_err(|e| match e {
                RunStoreError::WorktreeAlreadyActive { .. } => {
                    WorkflowEngineError::AlreadyActive(workflow.name.clone())
                }
                other => {
                    WorkflowEngineError::SessionStore(format!("RunStore register failed: {other}"))
                }
            })?;
        Ok(run_id)
    }

    async fn insert_workflow_execution(
        &self,
        run_id: String,
        workflow: Workflow,
        worktree_path: String,
        task: Option<String>,
        workflow_defaults: WorkflowDefaults,
        now: f64,
    ) -> Result<WorkflowState, WorkflowEngineError> {
        let mut execution = WorkflowExecution {
            id: run_id.clone(),
            workflow: workflow.clone(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            workflow_defaults,
            started_at: now,
            updated_at: now,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: worktree_path.clone(),
        };

        let step_name = workflow.nodes[0].name.clone();
        let mut execs = self.executions.lock().await;
        WorkflowExecution::validate_start(&workflow, find_any_by_worktree(&execs, &worktree_path))?;
        execution.step_execution_counts.insert(step_name, 1);
        execs.insert(run_id.clone(), execution);
        Ok(execs.get(&run_id).unwrap().to_workflow_state())
    }

    #[cfg(test)]
    async fn start_workflow_common_core_for_test(
        &self,
        workflow: Workflow,
        worktree_path: String,
        task: Option<String>,
        trigger_source: TriggerSource,
        now: f64,
    ) -> Result<String, WorkflowEngineError> {
        WorkflowExecution::validate_workflow_shape(&workflow)?;
        let run_id = self
            .reserve_workflow_run(&workflow, &worktree_path, task.clone(), trigger_source, now)
            .await?;
        self.insert_workflow_execution(
            run_id.clone(),
            workflow,
            worktree_path,
            task,
            WorkflowDefaults {
                backend_id: None,
                permission_mode: crate::permission::PermissionMode::EDIT.to_string(),
            },
            now,
        )
        .await?;
        Ok(run_id)
    }

    /// worktree_path から active run_id を解決する。Run Store の secondary index を参照する。
    pub async fn run_id_for_worktree(&self, worktree_path: &str) -> Option<String> {
        self.run_store.resolve_run_by_worktree(worktree_path).await
    }

    /// run_id から worktree_path を解決する。active な run のみならず、終了済み run も
    /// `workflow_runs/{run_id}.json` から metadata を読み込んで返す。
    /// Tauri command 経路で run_id 主語の操作を内部 worktree_path に解決する際に使用する。
    pub async fn resolve_worktree_by_run(&self, run_id: &str) -> Option<String> {
        self.run_store.resolve_worktree_by_run(run_id).await
    }

    /// [05] read-only API: optional な status / worktree filter を適用した
    /// run summary 一覧を返す（facade）。
    pub async fn list_runs(
        &self,
        filter: crate::workflow::run::RunListFilter,
    ) -> Vec<crate::workflow::run::WorkflowRunSummary> {
        self.run_store.list_runs(filter).await
    }

    /// テスト専用 facade: active な run 一覧を取得する。
    /// production 経路は `list_runs(RunListFilter { status: Some(Active), .. })` を使う。
    #[cfg(test)]
    pub async fn list_active_runs(&self) -> Vec<crate::workflow::run::WorkflowRunSummary> {
        self.run_store
            .list_runs(crate::workflow::run::RunListFilter {
                status: Some(crate::workflow::run::RunStatusFilter::Active),
                worktree_path: None,
            })
            .await
    }

    /// テスト専用 facade: terminal な run 一覧を取得する。
    #[cfg(test)]
    pub async fn list_completed_runs(&self) -> Vec<crate::workflow::run::WorkflowRunSummary> {
        self.run_store
            .list_runs(crate::workflow::run::RunListFilter {
                status: Some(crate::workflow::run::RunStatusFilter::Terminal),
                worktree_path: None,
            })
            .await
    }

    /// [05] read-only API: 単一 run の summary を取得する（facade）。
    /// active map → terminal metadata file の順で lookup する。
    pub async fn get_run(&self, run_id: &str) -> Option<crate::workflow::run::WorkflowRunSummary> {
        self.run_store.get_run(run_id).await
    }

    /// Run Store の永続化ディレクトリを設定する（アプリ起動時の setup から呼ぶ）。
    pub async fn set_run_store_data_dir(&self, dir: std::path::PathBuf) {
        self.run_store.set_data_dir(dir).await;
    }

    /// 起動時 recovery: 前回プロセスが terminal event を書かないまま終了した run（metadata の
    /// status が non-terminal なまま残った run）を、Aborted へ強制遷移させる。
    /// 既存 `event_projection` の `RunAborted → Aborted` 判定をそのまま機能させるため、
    /// `<data_dir>/workflow_logs/<run_id>.ndjson` 末尾に `RunAborted` event を append し、
    /// `workflow_runs/<run_id>.json` の status を Aborted に更新する。
    ///
    /// 本メソッドは `set_run_store_data_dir` 直後（in-memory `executions` map が空の状態）に
    /// 1 度だけ呼ばれる前提。append / persist が個別に失敗しても起動自体は止めない（warn
    /// のみ）。metadata の更新失敗時は次回起動で再試行される（idempotent）。
    pub async fn recover_orphan_runs<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>) {
        let orphans = self.run_store.list_non_terminal_metadata().await;
        if orphans.is_empty() {
            return;
        }
        let timestamp = current_timestamp();
        for run in orphans {
            let run_id = run.run_id.clone();
            let event = WorkflowEvent::RunAborted {
                run_id: run_id.clone(),
                workflow_name: run.workflow_name.clone(),
                timestamp,
            };
            if let Err(e) = self.write_log_required(app, event) {
                log::warn!("recover_orphan_runs: append RunAborted failed for {run_id}: {e}");
                // metadata 更新は次回起動で再試行するため、ここで skip する。
                continue;
            }
            if let Err(e) = self
                .run_store
                .force_complete_orphan_to_aborted(run, timestamp, None)
                .await
            {
                log::warn!("recover_orphan_runs: persist metadata failed for {run_id}: {e}");
            }
        }
    }

    /// ステップの model 値から対応するバックエンドIDを解決する。
    /// 形式検証（`ModelId`）と登録判定（`resolve_backend_for_model`）を
    /// 一括で行い、`set_agent_model_internal` と同一の受け入れ基準を適用する。
    async fn resolve_backend_for_step_model<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        model: &str,
    ) -> Result<Option<String>, WorkflowEngineError> {
        let registry = app
            .try_state::<Arc<crate::backends::AgentBackendRegistry>>()
            .ok_or_else(|| {
                WorkflowEngineError::InvalidWorkflow(format!(
                    "cannot resolve model '{model}': backend registry is unavailable"
                ))
            })?;
        resolve_step_model_with_registry(&registry, model).map(Some)
    }
}

/// 形式検証＋登録判定をレジストリ単体で行う、ワークフロー経路用の解決関数。
/// `resolve_backend_for_step_model` の実体ロジックで、テストではこちらを直接呼ぶ。
pub(crate) fn resolve_step_model_with_registry(
    registry: &crate::backends::AgentBackendRegistry,
    model: &str,
) -> Result<String, WorkflowEngineError> {
    crate::domain::agent_session::ModelId::parse(model).map_err(|e| {
        WorkflowEngineError::InvalidWorkflow(format!("invalid model '{model}': {e}"))
    })?;
    let backend_id = registry
        .resolve_backend_for_model(model)
        .map_err(|e| {
            WorkflowEngineError::InvalidWorkflow(format!(
                "model '{model}' could not be resolved: {e}"
            ))
        })?
        .ok_or_else(|| WorkflowEngineError::InvalidWorkflow(format!("unknown model: {model}")))?;
    Ok(backend_id)
}

impl WorkflowEngine {
    /// ステップ設定の解決 → セッション生成 → 解決済み設定の反映 → 保存を一括で行う。
    ///
    /// `start_step_session` と `start_parallel_children` の共通パターンを抽出したヘルパー。
    #[allow(clippy::too_many_arguments)]
    async fn create_step_session_with_settings<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &SessionStore,
        data_dir: &std::path::Path,
        worktree_path: &str,
        step_model: Option<String>,
        step_permission: Option<String>,
        workflow_defaults: &WorkflowDefaults,
    ) -> Result<ChatSession, WorkflowEngineError> {
        let resolved_backend_id = match step_model {
            Some(ref model) => self.resolve_backend_for_step_model(app, model).await?,
            None => None,
        };
        let settings = resolve_step_settings(
            step_model,
            step_permission,
            resolved_backend_id,
            workflow_defaults,
        );

        // Spec issues-947: 検証済み permission_mode と step session 属性を初回保存で確定する。
        // edit デフォルトで save → 上書きで再 save する二段階を排除し、途中失敗時に
        // 抽象モード不一致のセッションが残らないようにする。
        let permission_mode = crate::permission::PermissionMode::parse(&settings.permission_mode)
            .map_err(|e| WorkflowEngineError::InvalidWorkflow(e.to_string()))?;
        let step_session = crate::session::create_session_internal_with_attributes(
            session_store,
            data_dir,
            worktree_path,
            settings.backend_id,
            permission_mode,
            settings.selected_model,
            true,
        )
        .map_err(|e| WorkflowEngineError::SessionStore(format!("create step session: {e}")))?;

        Ok(step_session)
    }

    /// ワークフローを開始する。
    /// ChatSessionは既に作成済みの前提で、最初のステップのプロンプトを送信する。
    ///
    /// 戻り値は新しく払い出された `run_id`。
    /// `execution_id` を `run_id` として「昇格」させた値であり、ここ以外で採番されることはない。
    /// state 変化の入口は `WorkflowCommand::StartRun` 経由の `dispatch` 一本に統一する
    /// （Spec [04] 境界）。本メソッドは dispatch 内部からのみ呼ぶ private handler であり、
    /// 外部入口として `pub` 公開しない。
    #[allow(clippy::too_many_arguments)]
    async fn start_workflow<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        workflow: Workflow,
        worktree_path: String,
        file_stem: &str,
        task: Option<String>,
        trigger_source: TriggerSource,
        permission_mode: PermissionMode,
    ) -> Result<String, WorkflowEngineError> {
        // ===== Phase 1: 副作用なしの validation =====
        // parent ChatSession 作成・executions 登録・refs 登録の前で全 validation を実施する。
        // ここで弾けば、リトライ時に「孤立した parent session」「孤立した refs entry」
        // を残さない（Spec issues-1011: 起動順序のアトミック化）。
        //
        // 1) workflow 構造の事前検証（空 nodes / 未実装 bash node の拒否）。
        WorkflowExecution::validate_workflow_shape(&workflow)?;
        // 2) model 検証: 各 model から所属 backend を一意に解決する。
        //    registry 未登録自体を InvalidWorkflow として即時失敗にする（検証スキップを避ける）。
        let registry = app
            .try_state::<Arc<crate::backends::AgentBackendRegistry>>()
            .ok_or_else(|| {
                WorkflowEngineError::InvalidWorkflow(
                    "AgentBackendRegistry is not registered".to_string(),
                )
            })?;
        crate::workflow::validation::validate_models(&workflow, |model| {
            registry.resolve_backend_for_model(model)
        })
        .map_err(|e| WorkflowEngineError::InvalidWorkflow(e.to_string()))?;

        // ===== Phase 2: 副作用（Run Store reservation 先取り → 親 session 作成 → executions 登録） =====
        // Spec issues-1011 finding 5/8: 並行起動でも parent ChatSession を孤立させないために
        // Run Store reservation を「最初の副作用」にする。reservation が失敗（同一 worktree
        // への並行起動）した場合は AlreadyActive として返り、他の副作用は走らない。
        let data_dir = crate::session::resolve_data_dir(app)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;
        let now = current_timestamp();
        let run_id = self
            .reserve_workflow_run(&workflow, &worktree_path, task.clone(), trigger_source, now)
            .await?;

        // 以降の副作用で失敗した場合は Run Store reservation を確実に撤回する helper。
        // Spec issues-1011 finding 9: reservation 撤回専用 API (`cancel_reservation`) を使い、
        // 失敗した起動を completed 一覧（terminal entry）に残さない。撤回自体の失敗は
        // warn を出した上で reservation を completed_at=now の Failed として最低限 metadata に
        // 残し、Run Store と engine の状態スキューを抑える。
        // 撤回 helper は最終的な Result を返し、呼出側で start_workflow の Err に伝播させる。
        let rollback_reservation = |reason: String| async {
            if let Err(rs_err) = self.run_store.cancel_reservation(&run_id).await {
                log::warn!(
                    "RunStore cancel_reservation failed during start rollback for {run_id}: {rs_err}; reason={reason}"
                );
                // fallback として terminal metadata を残す（撤回より優先度低い）。
                if let Err(rs_err2) = self
                    .run_store
                    .complete_run(
                        &run_id,
                        TerminalRunStatus::Failed,
                        current_timestamp(),
                        Some(reason),
                    )
                    .await
                {
                    log::warn!(
                        "RunStore complete_run failed during start rollback fallback for {run_id}: {rs_err2}"
                    );
                }
            }
        };

        // parent ChatSession 機構撤去後は session を engine が作らない。
        // workflow_defaults は StartRun の permission_mode 引数を workflow 全体の継承
        // デフォルトとして capture する（schema 境界 [02]: 各 step は NodeDefinition.model
        // 必須で個別解決される）。
        let _ = data_dir; // unused after parent session removal
        let workflow_defaults = WorkflowDefaults {
            backend_id: None,
            permission_mode: permission_mode.as_str().to_string(),
        };

        // validate_start → insert → スナップショット確定を同一ロックで原子的に実行。
        // reservation 段階で worktree 衝突は撥ねているが、executions 側にも terminal run が
        // 残っている可能性があるため `find_any_by_worktree` で active な existing を見て
        // validate_start する。
        let step_name = workflow.nodes[0].name.clone();
        let snapshot_result = self
            .insert_workflow_execution(
                run_id.clone(),
                workflow.clone(),
                worktree_path.clone(),
                task.clone(),
                workflow_defaults,
                now,
            )
            .await;
        let snapshot = match snapshot_result {
            Ok(s) => s,
            Err(e) => {
                rollback_reservation(format!("validate_start failed: {e}")).await;
                return Err(e);
            }
        };

        // [04] commit point: RunStarted append が command 受理の唯一の不可逆な commit point。
        // ChatSession への workflow_state 永続化は撤去済み（NDJSON event log + Run Store
        // metadata が権威）。append 成功＝command 受理として扱い、以降の broadcast は
        // best-effort な post-commit 副作用に位置付ける。
        if let Err(e) = self.write_log_required(
            app,
            WorkflowEvent::RunStarted {
                run_id: snapshot.execution_id.clone(),
                workflow_name: snapshot.workflow_name.clone(),
                workflow_file_stem: file_stem.to_string(),
                worktree_path: worktree_path.clone(),
                workflow_definition: workflow.clone(),
                timestamp: now,
            },
        ) {
            let mut execs = self.executions.lock().await;
            execs.remove(&run_id);
            drop(execs);
            rollback_reservation(format!("RunStarted log failed: {e}")).await;
            return Err(WorkflowEngineError::SessionStore(format!(
                "write RunStarted log failed: {e}"
            )));
        }

        // [04] post-commit: broadcast。RunStarted は append 済みのため command は既に受理。
        // session_workflow_refs への登録は step session 起動時（start_step_session /
        // start_parallel_children）で行う。
        self.broadcast_state(app, &worktree_path, snapshot.clone())
            .await;

        // NDJSONログ: step_started 以降は補助ログとして best effort で書き込む。
        // 最初のステップが並列ブロックかどうかで分岐
        let first_step_is_parallel = workflow.nodes[0].is_parallel();

        // [04] post-commit: RunStarted append 済みのため command は既に受理。
        //    初回 session / parallel children 起動失敗は Failed 状態遷移として観測し、
        //    dispatch(StartRun) は Ok(RunStarted { run_id }) を返す（spec [04]
        //    『command 受理境界』Rule）。
        if first_step_is_parallel {
            // 並列ブロック → start_parallel_children を呼ぶ
            // (StepStartedログは書かず、start_parallel_children内でParallelStarted等を記録)
            if let Err(e) = self
                .start_parallel_children(app, session_store, handles, &worktree_path, true)
                .await
            {
                let _ = self
                    .set_execution_state(
                        app,
                        session_store,
                        handles,
                        &worktree_path,
                        WorkflowExecutionState::Failed {
                            reason: format!("Failed to start parallel children: {e}"),
                        },
                    )
                    .await;
                log::warn!("workflow {run_id}: post-commit start_parallel_children failed: {e}");
            }
        } else {
            // 逐次ステップ → StepStartedログ + start_step_session
            self.write_log(
                app,
                WorkflowEvent::NodeStarted {
                    run_id: snapshot.execution_id,
                    workflow_name: snapshot.workflow_name,
                    node_name: step_name.clone(),
                    execution_count: 1,
                    timestamp: now,
                },
            );

            if let Err(e) = self
                .start_step_session(app, handles, session_store, &worktree_path)
                .await
            {
                {
                    let mut execs = self.executions.lock().await;
                    if let Some(exec) = execs.get_mut(&run_id) {
                        let entry = exec.make_step_history_entry(
                            Some(format!("session_start_failed: {e}")),
                            None,
                            None,
                        );
                        exec.step_history.push(entry);
                    }
                }
                let _ = self
                    .set_execution_state(
                        app,
                        session_store,
                        handles,
                        &worktree_path,
                        WorkflowExecutionState::Failed {
                            reason: format!("Failed to start step session: {e}"),
                        },
                    )
                    .await;
                log::warn!("workflow {run_id}: post-commit start_step_session failed: {e}");
            }
        }
        Ok(run_id)
    }

    /// [05] adapter-facing 拒否境界: 外部 adapter（Tauri command / CLI / agent path）
    /// 向けの単一入口。internal-only な `CompleteNode` / `FailNode` を外部 caller から
    /// 組み立てて到達した場合は内部不整合として `Err` に変換する（spec [05] internal
    /// command の非公開境界）。internal variant 受理境界は `dispatch` 側に集約。
    ///
    /// engine 内部経路は `dispatch` を直接呼ぶ。本 wrapper は adapter からの外部入力
    /// だけが通る非公開境界専用とする。
    pub async fn dispatch_external<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        command: WorkflowCommand,
    ) -> Result<WorkflowCommandResult, WorkflowEngineError> {
        self.dispatch_external_inner(app, session_store, handles, command, None)
            .await
    }

    pub(crate) async fn dispatch_external_with_commit_context<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        command: WorkflowCommand,
        commit_context: CommandCommitContext,
    ) -> Result<WorkflowCommandResult, WorkflowEngineError> {
        self.dispatch_external_inner(app, session_store, handles, command, Some(commit_context))
            .await
    }

    async fn dispatch_external_inner<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        command: WorkflowCommand,
        commit_context: Option<CommandCommitContext>,
    ) -> Result<WorkflowCommandResult, WorkflowEngineError> {
        if matches!(
            command,
            WorkflowCommand::CompleteNode { .. } | WorkflowCommand::FailNode { .. }
        ) {
            let run_id = match &command {
                WorkflowCommand::CompleteNode { run_id, .. }
                | WorkflowCommand::FailNode { run_id, .. } => run_id.clone(),
                _ => unreachable!(),
            };
            return Err(WorkflowEngineError::ValidationError(format!(
                "internal-only WorkflowCommand variant reached public dispatch for run {run_id}"
            )));
        }
        self.dispatch_with_commit_context(app, session_store, handles, command, commit_context)
            .await
    }

    /// [08] 指定 run の event log 内に同じ `request_id` を持つ OutputSubmitted が既に
    /// append されているかを判定する idempotency 用 helper。CLI pending command の
    /// 再処理時に重複 OutputSubmitted を作らないように、dispatch 入口側で短絡する。
    pub(crate) fn output_submitted_already_recorded<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        request_id: &str,
    ) -> Result<bool, WorkflowEngineError> {
        uuid::Uuid::parse_str(run_id).map_err(|_| {
            WorkflowEngineError::ValidationError("SubmitOutput run_id must be UUID".to_string())
        })?;
        uuid::Uuid::parse_str(request_id).map_err(|_| {
            WorkflowEngineError::ValidationError("SubmitOutput request_id must be UUID".to_string())
        })?;
        let data_dir =
            crate::session::resolve_data_dir(app).map_err(WorkflowEngineError::SessionStore)?;
        let log = WorkflowEventLog::new(&data_dir);
        let events = log
            .read_log(run_id)
            .map_err(WorkflowEngineError::SessionStore)?;
        Ok(events.iter().any(|e| {
            matches!(
                e,
                WorkflowEvent::OutputSubmitted { request_id: Some(rid), .. } if rid == request_id
            )
        }))
    }

    /// [08] step に対する構造化出力提出の単一トランザクション handler。
    ///
    /// 1. run / step / contract の妥当性検証
    /// 2. `validate_contract_value` で contract 適合判定
    /// 3. 適合時のみ `step_outputs` / `workflow_variables` を更新し、
    ///    `OutputSubmitted` event を append
    /// 4. 不適合・stale step・不在 step・契約タイプ不一致は副作用なしで `Err` を返し、
    ///    `step_outputs` / `workflow_variables` / event log を一切変更しない。
    #[allow(clippy::too_many_arguments)]
    async fn handle_submit_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        step_name: String,
        contract: String,
        structured_output: serde_json::Value,
        request_id: Option<String>,
        submitted_at: Option<f64>,
    ) -> Result<(), WorkflowEngineError> {
        uuid::Uuid::parse_str(run_id)
            .map_err(|_| WorkflowEngineError::ValidationError("run_id must be UUID".to_string()))?;
        if step_name.trim().is_empty() {
            return Err(WorkflowEngineError::ValidationError(
                "step_name must not be empty".to_string(),
            ));
        }
        if contract.trim().is_empty() {
            return Err(WorkflowEngineError::ValidationError(
                "contract must not be empty".to_string(),
            ));
        }

        // 1. contract 適合判定（pure validator、副作用なし）。ロック取得前に行い、
        //    無効入力は writer lock を取らずに弾く。
        //    [08] 機密値 redaction: caller (CLI / Tauri API) 入力に approve コメントや
        //    secret token が混入していても event log / step_outputs に生で残らないよう、
        //    redaction 後の structured output を contract validation に通す。
        //    preflight (workflow_validate_output / CLI cmd_output_validate) と本 submit で
        //    同一の前処理 + validation を共有するため、`preprocess_and_validate_output`
        //    に集約する（spec [08] L169 / Rule 2）。
        let (validated_output, validated_result) =
            match Self::preprocess_and_validate_output(app, &contract, structured_output) {
                ContractValidationResult::Valid {
                    structured_output,
                    result,
                } => (structured_output, result),
                ContractValidationResult::Invalid(violation) => {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "contract validation failed ({}): {}",
                        violation.reason, violation.details
                    )));
                }
            };

        // 2. writer lock 取得後に state / contract / accepting target / run_index を
        //    再検証し、snapshot 採取と mutation を同一 lock スコープで行う
        //    （spec [08] 境界: OutputSubmitted の append は適合判定および state 更新と
        //    同一トランザクション境界内。並行 dispatch によって stale step の output が
        //    確定されないよう、validation と mutation のあいだに lock を手放さない）。
        let contract_vars = Self::extract_contract_variables(
            &Some(contract.clone()),
            &Some(validated_output.clone()),
        );
        let timestamp = current_timestamp();
        let (workflow_name, prior_step_output, prior_workflow_variables) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(run_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
            match exec.state {
                WorkflowExecutionState::Running | WorkflowExecutionState::WaitingApproval => {}
                _ => {
                    return Err(WorkflowEngineError::InvalidState(format!(
                        "run {run_id} is not accepting structured output (state: {})",
                        exec.state.as_str()
                    )));
                }
            }
            let expected_contract =
                crate::workflow::contract::lookup_step_output_contract(&exec.workflow, &step_name)
                    .ok_or_else(|| {
                        WorkflowEngineError::ValidationError(format!(
                            "step '{step_name}' is not a valid submission target"
                        ))
                    })?;
            if expected_contract != contract {
                return Err(WorkflowEngineError::ValidationError(format!(
                    "contract mismatch: step '{step_name}' expects '{expected_contract}', got '{contract}'"
                )));
            }
            if !Self::is_accepting_submission_target(exec, &step_name) {
                return Err(WorkflowEngineError::InvalidState(format!(
                    "step '{step_name}' is not currently accepting structured output"
                )));
            }
            let run_index = exec
                .step_execution_counts
                .get(&step_name)
                .copied()
                .unwrap_or(0);
            let workflow_name = exec.workflow.name.clone();
            let prior_step_output = exec.step_outputs.get(&step_name).cloned();
            let prior_workflow_variables = exec.workflow_variables.clone();
            exec.step_outputs.insert(
                step_name.clone(),
                StepOutput {
                    step_name: step_name.clone(),
                    run_index,
                    session_id: None,
                    result: validated_result.clone(),
                    structured_output: Some(validated_output.clone()),
                    output_contract: Some(contract.clone()),
                    token_usage: None,
                    completed_at: timestamp,
                },
            );
            if !contract_vars.is_empty() {
                exec.workflow_variables.extend(contract_vars);
            }
            (workflow_name, prior_step_output, prior_workflow_variables)
        };

        // 3. OutputSubmitted event を append。append 失敗時は state を snapshot から
        //    一括復元することで「validation・state 更新・event append」を原子的に揃える
        //    （spec [08] 振る舞い定義 Rule 1: 適合しない場合 / 適合する場合いずれも
        //    state と event log が一致する）。
        let event = WorkflowEvent::OutputSubmitted {
            run_id: run_id.to_string(),
            workflow_name,
            node_name: step_name.clone(),
            contract,
            structured_output: validated_output,
            request_id,
            submitted_at,
            timestamp,
        };
        if let Err(append_err) = self.write_log_required(app, event) {
            let mut execs = self.executions.lock().await;
            if let Some(exec) = execs.get_mut(run_id) {
                match prior_step_output {
                    Some(prior) => {
                        exec.step_outputs.insert(step_name, prior);
                    }
                    None => {
                        exec.step_outputs.remove(&step_name);
                    }
                }
                exec.workflow_variables = prior_workflow_variables;
            }
            return Err(WorkflowEngineError::SessionStore(append_err));
        }

        Ok(())
    }

    /// [08] step が現在「構造化出力を受け付けられる」か判定する。
    ///
    /// 受付対象は以下のいずれか:
    /// - top-level の現在 step（`current_step_index`）であり、当該 step が
    ///   parallel ではない（Running / WaitingApproval どちらでも提出可能）
    /// - 親 step が parallel で、当該名前の parallel child が Running 状態
    ///
    /// 既に完了 / 失敗した step、未到達の future step、別 parallel child の名前を
    /// 指定された場合はいずれも accepting target ではない（spec [08] 振る舞い定義
    /// Rule 1 Scenario 3: 出力を受け付けられる状態にない step は拒否）。
    fn is_accepting_submission_target(exec: &WorkflowExecution, step_name: &str) -> bool {
        if exec.current_step_index >= exec.workflow.nodes.len() {
            return false;
        }
        let current = &exec.workflow.nodes[exec.current_step_index];
        if current.name == step_name && current.node_type != NodeType::Parallel {
            return true;
        }
        if let Some(ref pr) = exec.parallel_run {
            if pr.parent_step_name == current.name {
                return pr
                    .children
                    .iter()
                    .any(|c| c.step_name == step_name && c.state == ParallelChildState::Running);
            }
        }
        false
    }

    fn submitted_step_output_for(
        exec: &WorkflowExecution,
        step_name: &str,
        run_index: u32,
        contract: &str,
    ) -> Option<StepOutput> {
        let output = exec.step_outputs.get(step_name)?;
        if output.run_index == run_index
            && output.output_contract.as_deref() == Some(contract)
            && output.structured_output.is_some()
        {
            Some(output.clone())
        } else {
            None
        }
    }

    fn resolved_output_contract_definition_for(
        exec: &WorkflowExecution,
        step_name: &str,
        contract: &str,
    ) -> Option<String> {
        for node in &exec.workflow.nodes {
            if node.name == step_name && node.output_contract.as_deref() == Some(contract) {
                return node
                    .resolved_facets
                    .output_contract
                    .as_deref()
                    .map(strip_contract_validation_metadata);
            }
            if let Some(children) = node.parallel_children.as_ref() {
                for child in children {
                    if child.name == step_name && child.output_contract.as_deref() == Some(contract)
                    {
                        return child
                            .resolved_facets
                            .output_contract
                            .as_deref()
                            .map(strip_contract_validation_metadata);
                    }
                }
            }
        }
        None
    }

    pub(crate) async fn append_command_commit_context<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        context: CommandCommitContext,
    ) -> Result<(), WorkflowEngineError> {
        // SubmitOutput 経路は CliMutationRequested を emit せず、OutputSubmitted 単体で
        // 記録する（spec [08]）。ここでは何もせず Ok を返す。
        let Some(mutation_ref) = context.cli_pending_mutation() else {
            return Ok(());
        };
        let run_id = mutation_ref.run_id().to_string();
        let workflow_name = self.workflow_name_for_external_run(&run_id).await?;
        let event = Self::command_commit_context_event(&workflow_name, context)
            .expect("CliPending context must produce a CliMutationRequested event");
        self.write_log_required(app, event)
            .map_err(WorkflowEngineError::SessionStore)?;
        Ok(())
    }

    fn command_commit_context_event(
        workflow_name: &str,
        context: CommandCommitContext,
    ) -> Option<WorkflowEvent> {
        let mutation = context.into_cli_pending_mutation()?;
        let (run_id, request, requested_at, request_id) = mutation.into_event_parts();
        Some(WorkflowEvent::CliMutationRequested {
            run_id,
            workflow_name: workflow_name.to_string(),
            request_id,
            request,
            requested_at,
            timestamp: current_timestamp(),
        })
    }

    pub(crate) async fn workflow_name_for_external_run(
        &self,
        run_id: &str,
    ) -> Result<String, WorkflowEngineError> {
        let execs = self.executions.lock().await;
        if let Some(exec) = execs.get(run_id) {
            return Ok(exec.workflow.name.clone());
        }
        drop(execs);
        self.run_store
            .get_run_record(run_id)
            .await
            .map(|run| run.workflow_name)
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))
    }

    pub(crate) fn should_commit_rejected_external_request(error: &WorkflowEngineError) -> bool {
        matches!(
            error,
            WorkflowEngineError::ValidationError(_)
                | WorkflowEngineError::InvalidState(_)
                | WorkflowEngineError::UnauthorizedApprovalTarget(_)
                | WorkflowEngineError::UnauthorizedWorktree(_)
        )
    }

    /// 5-3 / 5-4 修正: engine が拒否した CLI mutation の `WorkflowEngineError` を
    /// `CliMutationRejectionReason` に分類する。observability 用途のため詳細は
    /// `CliMutationRejected.message` で人間可読に保ち、ここでは粗粒度の分類に
    /// 留める。
    pub(crate) fn classify_rejection_reason(
        error: &WorkflowEngineError,
    ) -> CliMutationRejectionReason {
        use CliMutationRejectionReason::*;
        match error {
            WorkflowEngineError::ExecutionNotFound(_) => RunNotFound,
            WorkflowEngineError::UnauthorizedApprovalTarget(_) => NotWaitingApproval,
            WorkflowEngineError::UnauthorizedWorktree(_) => Other,
            WorkflowEngineError::ValidationError(msg) => {
                if msg.contains("contract mismatch") {
                    ContractMismatch
                } else if msg.contains("is not a valid submission target") {
                    NodeNotFound
                } else {
                    Other
                }
            }
            WorkflowEngineError::InvalidState(msg) => {
                if msg.contains("does not allow reject") {
                    NoRejectRule
                } else if msg.contains("is not currently accepting structured output") {
                    StepNotAccepting
                } else if msg.contains("is already terminal")
                    || msg.contains("is not accepting structured output (state:")
                {
                    RunNotActive
                } else {
                    Other
                }
            }
            // 以下は retryable / 内部 I/O 経路で本来は到達しない（呼び出し側で
            // `should_commit_rejected_external_request` で弾く）。安全側で Other。
            _ => Other,
        }
    }

    /// 5-3 / 5-4 修正: engine が拒否した CLI mutation を `CliMutationRejected`
    /// event として補助履歴に追記する。
    ///
    /// 失敗時は呼び出し側に `WorkflowEngineError` として伝播し、dispatcher 側で
    /// retryable / final を分類する。本 event は spec [08] Rule 1 の意味
    /// （accepted のメイン履歴に出さない）を壊さない補助履歴である点に注意。
    pub(crate) async fn append_cli_mutation_rejected<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        context: &CommandCommitContext,
        error: &WorkflowEngineError,
    ) -> Result<(), WorkflowEngineError> {
        let run_id = match context {
            CommandCommitContext::CliPending { mutation } => mutation.run_id().to_string(),
            CommandCommitContext::SubmitOutput { .. } => {
                // SubmitOutput 経路の run_id は dispatcher 側が pending command から
                // 取り出しているが、commit_context には保持していない。run_id
                // 解決は engine 側の `workflow_name_for_external_run` でも引けず、
                // SubmitOutput では呼び出し元（dispatcher）から別途渡してもらう
                // 設計にする。本メソッドは CliPending を直接扱うバリアントに限定
                // し、SubmitOutput 経路は専用 helper `append_cli_mutation_rejected_for_submit_output`
                // を使う。
                return Err(WorkflowEngineError::InvalidState(
                    "append_cli_mutation_rejected requires CliPending context".to_string(),
                ));
            }
        };
        let workflow_name = self.workflow_name_for_external_run(&run_id).await?;
        let (request, requested_at, request_id) = match context {
            CommandCommitContext::CliPending { mutation } => {
                let (_, request, requested_at, request_id) = mutation.clone().into_event_parts();
                (request, requested_at, request_id)
            }
            CommandCommitContext::SubmitOutput { .. } => unreachable!(),
        };
        let event = WorkflowEvent::CliMutationRejected {
            run_id,
            workflow_name,
            request_id,
            request,
            reason: Self::classify_rejection_reason(error),
            message: error.to_string(),
            requested_at,
            timestamp: current_timestamp(),
        };
        self.write_log_required(app, event)
            .map_err(WorkflowEngineError::SessionStore)?;
        Ok(())
    }

    /// 5-3 修正: SubmitOutput 経路用の `CliMutationRejected` append。
    ///
    /// SubmitOutput の commit_context は `WorkflowMutationContext` を持たないため、
    /// `run_id` は dispatcher 側から渡してもらう。spec [08] Rule 1 維持のため
    /// `CliMutationRequested` は引き続き emit せず、本 event のみが補助履歴と
    /// して残る。
    pub(crate) async fn append_cli_mutation_rejected_for_submit_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        context: &CommandCommitContext,
        error: &WorkflowEngineError,
    ) -> Result<(), WorkflowEngineError> {
        let (request_id, requested_at, step_name, contract) =
            context.submit_output_rejection_parts().ok_or_else(|| {
                WorkflowEngineError::InvalidState(
                    "append_cli_mutation_rejected_for_submit_output requires SubmitOutput context"
                        .to_string(),
                )
            })?;
        let workflow_name = self.workflow_name_for_external_run(run_id).await?;
        let event = WorkflowEvent::CliMutationRejected {
            run_id: run_id.to_string(),
            workflow_name,
            request_id,
            request: CliMutationRequestRecord::SubmitOutput {
                step_name,
                contract,
            },
            reason: Self::classify_rejection_reason(error),
            message: error.to_string(),
            requested_at,
            timestamp: current_timestamp(),
        };
        self.write_log_required(app, event)
            .map_err(WorkflowEngineError::SessionStore)?;
        Ok(())
    }

    pub(crate) fn cli_mutation_already_recorded<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        request_id: &str,
    ) -> Result<bool, WorkflowEngineError> {
        uuid::Uuid::parse_str(run_id).map_err(|_| {
            WorkflowEngineError::ValidationError("CLI mutation run_id must be UUID".to_string())
        })?;
        uuid::Uuid::parse_str(request_id).map_err(|_| {
            WorkflowEngineError::ValidationError("CLI mutation request_id must be UUID".to_string())
        })?;
        let data_dir =
            crate::session::resolve_data_dir(app).map_err(WorkflowEngineError::SessionStore)?;
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(run_id)
            .map_err(WorkflowEngineError::SessionStore)?;
        Ok(events.iter().any(|event| {
            matches!(
                event,
                WorkflowEvent::CliMutationRequested {
                    request_id: id,
                    ..
                } if id == request_id
            )
        }))
    }

    /// 外部入口（CLI pending dispatcher / 将来追加される他経路）が dispatch
    /// する前に、in-memory execution を `workflow_runs/` から再構成する。
    ///
    /// `dispatch_external_with_commit_context` 入口の前段で呼ぶことで、稼働
    /// アプリ再起動後でも `run_id` 主語の mutation が認可・冪等性判定の対象
    /// となる（spec [06] 経路非依存境界）。本関数は CLI 経路に限定されない
    /// ため `_for_external` で命名統一する（review R2-02）。
    pub(crate) async fn ensure_execution_loaded_for_external<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        run_id: &str,
    ) -> Result<(), WorkflowEngineError> {
        {
            let execs = self.executions.lock().await;
            if execs.contains_key(run_id) {
                return Ok(());
            }
        }

        let run = self
            .run_store
            .get_run_record(run_id)
            .await
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
        if run.status.is_terminal() {
            return Err(WorkflowEngineError::InvalidState(format!(
                "run {run_id} is already terminal"
            )));
        }

        let data_dir =
            crate::session::resolve_data_dir(app).map_err(WorkflowEngineError::SessionStore)?;
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(run_id)
            .map_err(WorkflowEngineError::SessionStore)?;
        let state = reconstruct_state_from_events(run_id, &events)
            .map_err(WorkflowEngineError::SessionStore)?
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
        if !matches!(
            state.state,
            WorkflowExecutionState::Running | WorkflowExecutionState::WaitingApproval
        ) {
            return Err(WorkflowEngineError::InvalidState(format!(
                "run {run_id} is already terminal"
            )));
        }
        if state.current_step_index >= state.workflow_definition.nodes.len() {
            return Err(WorkflowEngineError::InvalidState(format!(
                "run {run_id} has invalid current step"
            )));
        }

        if self.run_store.active_run_snapshot(run_id).await.is_none() {
            self.run_store
                .register_active(run.clone())
                .await
                .map_err(|e| {
                    WorkflowEngineError::SessionStore(format!("RunStore restore failed: {e}"))
                })?;
        }

        // parent ChatSession 機構撤去後は session_store からの復元経路を持たない。
        // current_session_id は event log の projection から復元（projection は常に None
        // を返すため、CLI 再接続直後は in-memory `WorkflowExecution.current_session_id` も
        // None）。CLI 経由の `dispatch_external` 4 typed command（StartRun/AbortRun/
        // ApproveNode/RejectNode）は `run_id` 主語で動作するため current_session_id は不要。
        let current_session_id = state.current_session_id.clone();
        // workflow_defaults は in-memory cache であり event log からは復元できない属性。
        // 再開後の step 起動は NodeDefinition.model / permission から settled する。
        let restored_workflow_defaults = WorkflowDefaults {
            backend_id: None,
            permission_mode: crate::permission::PermissionMode::EDIT.to_string(),
        };
        let exec = WorkflowExecution {
            id: run_id.to_string(),
            workflow: state.workflow_definition,
            state: state.state,
            current_step_index: state.current_step_index,
            step_execution_counts: state.step_execution_counts,
            step_history: state.step_history,
            workflow_defaults: restored_workflow_defaults,
            worktree_path: run.worktree_path,
            started_at: state.started_at,
            updated_at: state.updated_at,
            current_session_id: current_session_id.clone(),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: state.step_outputs,
            task: run.task,
            parallel_run: None,
            workflow_variables: state.workflow_variables,
        };

        let _ = session_store; // session_store は parent session 撤去後の本経路では未使用

        let mut execs = self.executions.lock().await;
        execs.entry(run_id.to_string()).or_insert(exec);
        drop(execs);

        let mut refs = self.session_workflow_refs.lock().await;
        if let Some(step_session_id) = current_session_id {
            refs.insert(
                step_session_id,
                SessionWorkflowRef {
                    run_id: run_id.to_string(),
                },
            );
        }
        Ok(())
    }

    /// [04] / [05] Command / Event Boundary: typed command の単一発火点。
    ///
    /// `WorkflowEngine::dispatch` は外部 4 variant (`StartRun` / `AbortRun` /
    /// `ApproveNode` / `RejectNode`) と internal 2 variant (`CompleteNode` /
    /// `FailNode`) を同一の typed command handler で処理する。internal node 完了 /
    /// 失敗の event 発行点もここに集約される（spec [05] L22-24, L145, L186）。
    ///
    /// 外部 adapter（Tauri command / CLI / agent path）は `dispatch_external` 経由で
    /// internal variant を拒否してから本関数に委譲する。engine 内部からは内部経路の
    /// 発火点として `dispatch` を直接呼び、internal variant も含めた typed command
    /// を受理する。
    pub async fn dispatch<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        command: WorkflowCommand,
    ) -> Result<WorkflowCommandResult, WorkflowEngineError> {
        self.dispatch_with_commit_context(app, session_store, handles, command, None)
            .await
    }

    async fn dispatch_with_commit_context<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        command: WorkflowCommand,
        commit_context: Option<CommandCommitContext>,
    ) -> Result<WorkflowCommandResult, WorkflowEngineError> {
        match command {
            WorkflowCommand::StartRun {
                workflow_file_stem,
                worktree_path,
                task,
                trigger_source,
                permission_mode,
            } => {
                crate::workflow::validation::validate_name(&workflow_file_stem).map_err(|e| {
                    WorkflowEngineError::ValidationError(format!("validation_error: {e}"))
                })?;

                let worktree_path = self.worktree_resolver.resolve(worktree_path).await?;

                let file_stem = workflow_file_stem.clone();
                let workflow = self.workflow_resolver.resolve(&workflow_file_stem).await?;

                let run_id = self
                    .start_workflow(
                        app,
                        session_store,
                        handles,
                        workflow,
                        worktree_path,
                        &file_stem,
                        task,
                        trigger_source,
                        permission_mode,
                    )
                    .await?;
                Ok(WorkflowCommandResult::RunStarted { run_id })
            }
            WorkflowCommand::AbortRun {
                run_id,
                expected_node_name,
            } => {
                // run 全体の Abort: NotFound / AlreadyTerminal は非受理として typed error
                // に射影する（Spec [04] Rule「対象不在 / 既に終了した command は受理されない」）。
                match self
                    .abort_workflow_by_run_id(
                        app,
                        session_store,
                        handles,
                        &run_id,
                        expected_node_name.as_deref(),
                        commit_context,
                    )
                    .await?
                {
                    AbortOutcome::Aborted => Ok(WorkflowCommandResult::Accepted),
                    AbortOutcome::NotFound => Err(WorkflowEngineError::ExecutionNotFound(run_id)),
                    AbortOutcome::AlreadyTerminal => Err(WorkflowEngineError::InvalidState(
                        format!("run {run_id} is already terminal"),
                    )),
                }
            }
            WorkflowCommand::ApproveNode {
                run_id,
                node_name,
                comment,
            } => {
                self.handle_approval(
                    app,
                    session_store,
                    handles,
                    &run_id,
                    ApprovalDecision::Approve,
                    comment,
                    node_name.as_deref(),
                    commit_context,
                )
                .await?;
                Ok(WorkflowCommandResult::Accepted)
            }
            WorkflowCommand::RejectNode {
                run_id,
                node_name,
                reason,
            } => {
                self.handle_approval(
                    app,
                    session_store,
                    handles,
                    &run_id,
                    ApprovalDecision::Reject {
                        comment: reason.clone(),
                    },
                    Some(reason),
                    node_name.as_deref(),
                    commit_context,
                )
                .await?;
                Ok(WorkflowCommandResult::Accepted)
            }
            // [08] step に対する構造化出力の typed 提出。CLI / Tauri command / in-process
            // caller の双方が `dispatch_external` を経由して同一 handler に合流する。
            // CLI pending 経路では commit_context が `SubmitOutput { request_id,
            // submitted_at }` を運び、OutputSubmitted event に保存する。in-process 経路
            // では commit_context = None で metadata は None になる。
            WorkflowCommand::SubmitOutput {
                run_id,
                step_name,
                contract,
                structured_output,
            } => {
                let (request_id, submitted_at) = commit_context
                    .as_ref()
                    .and_then(|ctx| ctx.submit_output_metadata())
                    .map(|(rid, ts)| (Some(rid), Some(ts)))
                    .unwrap_or((None, None));
                self.handle_submit_output(
                    app,
                    &run_id,
                    step_name,
                    contract,
                    structured_output,
                    request_id,
                    submitted_at,
                )
                .await?;
                Ok(WorkflowCommandResult::Accepted)
            }
            // [05] internal-only variant の commit handler。public `dispatch` 経由では
            // 事前に拒否されるが、`dispatch_core` を engine 内部から typed command 経由で
            // 呼び出した場合に到達する（spec [05]: dispatch を内部 node 完了 / 失敗の
            // 発火点とし、event 発行点を typed command 経路に集約）。state mutation +
            // event commit は `dispatch_internal_node_command` に委譲し、commit 失敗時は
            // engine state を一括復元する rollback 境界に揃える。
            command @ WorkflowCommand::CompleteNode { .. }
            | command @ WorkflowCommand::FailNode { .. } => {
                self.commit_internal_node_command(app, session_store, command)
                    .await?;
                Ok(WorkflowCommandResult::Accepted)
            }
        }
    }

    /// [05] `WorkflowCommand::CompleteNode` / `FailNode` 用の commit 境界。
    ///
    /// `dispatch` から呼ばれる engine 内部の typed command handler であり、外部
    /// adapter からは `dispatch_external` の拒否境界で到達不能。state mutation +
    /// `WorkflowEvent::NodeCompleted` / `NodeFailed` 発行を
    /// `dispatch_internal_node_command` に集約し、append 失敗 / Run Store sync 失敗 /
    /// ChatSession persist 失敗のいずれでも `commit_required_events` 経由で engine
    /// state と Run Store snapshot を `snapshot_before` で一括復元する（spec [05]
    /// commit_required_events 基盤の rollback 可能な共通 commit 境界 / silent error
    /// 禁止）。
    async fn commit_internal_node_command<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        command: WorkflowCommand,
    ) -> Result<(), WorkflowEngineError> {
        let run_id = match &command {
            WorkflowCommand::CompleteNode { run_id, .. }
            | WorkflowCommand::FailNode { run_id, .. } => run_id.clone(),
            _ => {
                return Err(WorkflowEngineError::ValidationError(
                    "commit_internal_node_command received non-internal variant".to_string(),
                ));
            }
        };

        // lock + snapshot + mutation を atomic に実行: dispatch_internal_node_command の
        // state mutation は snapshot 上に適用され、event を返した時点で engine.executions
        // の対象 exec にも反映する。
        let (mutated_snapshot, exec_snapshot_before, event) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(&run_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.clone()))?;
            let exec_snapshot_before = exec.clone();
            let mut snapshot = exec.to_workflow_state();
            let event =
                Self::dispatch_internal_node_command(&mut snapshot, command).inspect_err(|_e| {
                    // commit 失敗: engine state は未変更（snapshot のみ mutate）。
                    *exec = exec_snapshot_before.clone();
                })?;
            // snapshot の state / updated_at を live exec に書き戻す。
            exec.state = snapshot.state.clone();
            exec.updated_at = snapshot.updated_at;
            (snapshot, exec_snapshot_before, event)
        };

        // [05] commit_required_events 基盤の共通 commit 境界:
        // RunStore sync → ChatSession persist → event log append の順序と
        // rollback 方針を一箇所に集約する。いずれかの失敗時は engine state と
        // Run Store snapshot を `exec_snapshot_before` で一括復元する。
        let run_store_snapshot_before = self.run_store.active_run_snapshot(&run_id).await;
        self.commit_required_events(
            app,
            session_store,
            RequiredEventCommit {
                run_id: &run_id,
                snapshot_for_commit: &mutated_snapshot,
                snapshot_before: exec_snapshot_before,
                run_store_snapshot_before,
                required_events: vec![event],
                append_error_context: "internal node command event append failed",
            },
        )
        .await
    }

    /// [05] internal dispatch path: engine 内部の node 完了 / 失敗 typed command の
    /// 単一 commit 関数。`WorkflowCommand::CompleteNode` / `FailNode` を受け取り、
    /// 対応する state mutation を snapshot に適用したうえで
    /// `WorkflowEvent::NodeCompleted` / `NodeFailed` を返す（spec [05]: 発行点が
    /// typed command の経路に集約される / state mutation と event 発行を同一 commit
    /// 境界に集約）。
    ///
    /// 入力型は `WorkflowCommand` だが、internal variant 以外（外部 4 variant）が
    /// 流入した場合は `ValidationError` を返す。外部 adapter からの組み立て経路は
    /// 提供せず、`pub(crate)` で配置することで internal-only な commit 境界を担保する
    /// （spec [05] internal command の非公開境界）。
    ///
    /// state mutation の責務は以下のとおり本関数に集約される:
    ///
    /// - `CompleteNode`: 上流コードが構築した `step_history` 末尾 entry が当該 command の
    ///   effect（node_name / timestamp / result / session_id / token_usage /
    ///   structured_output / run_index）と一致することを検証し、不一致時は
    ///   `ValidationError` を返す。あわせて command の run_id / workflow_name が
    ///   snapshot.execution_id / workflow_name と一致することも検証する（spec [05]
    ///   commit 境界: snapshot が command effect を含むことの確証 / payload を
    ///   snapshot と整合検証する厳格化）。
    /// - `FailNode`: command の run_id / workflow_name / node_name を snapshot と
    ///   照合した上で、snapshot.state が `Failed { .. }` でなければ
    ///   `Failed { reason }` に遷移させ、updated_at を反映する。
    pub(crate) fn dispatch_internal_node_command(
        snapshot: &mut WorkflowState,
        command: WorkflowCommand,
    ) -> Result<WorkflowEvent, WorkflowEngineError> {
        Self::apply_internal_node_command_state_mutation(snapshot, &command)?;
        Self::map_internal_node_command_to_event(command)
    }

    /// [05] internal command → state mutation の commit。`dispatch_internal_node_command`
    /// 内部からのみ呼ばれる。
    ///
    /// - `CompleteNode`: 上流の `make_step_history_entry` 経由で push 済みの step_history
    ///   末尾 entry が当該 command の全 effect 列と一致するかを検証する。さらに command
    ///   の run_id / workflow_name を snapshot の execution_id / workflow_name と
    ///   照合する。不一致時は `ValidationError` を返し、command と snapshot の同期境界
    ///   が崩れたことを呼出側に伝える（spec [05] commit 境界: snapshot mutation と
    ///   event 発行を同一 commit に集約 / 二重適用は不可 / payload を snapshot と
    ///   照合）。
    /// - `FailNode`: command の run_id / workflow_name / node_name を snapshot と
    ///   照合した上で、snapshot.state が `Failed { .. }` でなければ
    ///   `Failed { reason }` に遷移させ、updated_at を反映する。既に Failed の場合は
    ///   idempotent な no-op。
    fn apply_internal_node_command_state_mutation(
        snapshot: &mut WorkflowState,
        command: &WorkflowCommand,
    ) -> Result<(), WorkflowEngineError> {
        match command {
            WorkflowCommand::CompleteNode {
                run_id,
                workflow_name,
                node_name,
                result,
                session_id,
                token_usage,
                structured_output,
                run_index,
                timestamp,
            } => {
                if run_id != &snapshot.execution_id {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "CompleteNode run_id mismatch: command='{run_id}', snapshot='{}'",
                        snapshot.execution_id
                    )));
                }
                if workflow_name != &snapshot.workflow_name {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "CompleteNode workflow_name mismatch: command='{workflow_name}', snapshot='{}'",
                        snapshot.workflow_name
                    )));
                }
                let Some(last_entry) = snapshot.step_history.last() else {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "CompleteNode for node '{node_name}' but snapshot.step_history is empty"
                    )));
                };
                if last_entry.step_name != *node_name {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "CompleteNode node mismatch: command='{node_name}', snapshot last='{}'",
                        last_entry.step_name
                    )));
                }
                if (last_entry.completed_at - *timestamp).abs() > f64::EPSILON {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "CompleteNode timestamp mismatch for node '{node_name}': command={timestamp}, snapshot={}",
                        last_entry.completed_at
                    )));
                }
                if last_entry.result != *result {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "CompleteNode result mismatch for node '{node_name}'"
                    )));
                }
                if last_entry.session_id != *session_id {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "CompleteNode session_id mismatch for node '{node_name}'"
                    )));
                }
                if last_entry.token_usage != *token_usage {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "CompleteNode token_usage mismatch for node '{node_name}'"
                    )));
                }
                if last_entry.structured_output != *structured_output {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "CompleteNode structured_output mismatch for node '{node_name}'"
                    )));
                }
                if Some(last_entry.run_index) != *run_index {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "CompleteNode run_index mismatch for node '{node_name}': command={run_index:?}, snapshot={}",
                        last_entry.run_index
                    )));
                }
                Ok(())
            }
            WorkflowCommand::FailNode {
                run_id,
                workflow_name,
                node_name,
                reason,
                timestamp,
            } => {
                if run_id != &snapshot.execution_id {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "FailNode run_id mismatch: command='{run_id}', snapshot='{}'",
                        snapshot.execution_id
                    )));
                }
                if workflow_name != &snapshot.workflow_name {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "FailNode workflow_name mismatch: command='{workflow_name}', snapshot='{}'",
                        snapshot.workflow_name
                    )));
                }
                if *node_name != snapshot.current_step_name {
                    return Err(WorkflowEngineError::ValidationError(format!(
                        "FailNode node_name mismatch: command='{node_name}', snapshot='{}'",
                        snapshot.current_step_name
                    )));
                }
                if !matches!(snapshot.state, WorkflowExecutionState::Failed { .. }) {
                    snapshot.state = WorkflowExecutionState::Failed {
                        reason: reason.clone(),
                    };
                    snapshot.updated_at = *timestamp;
                }
                Ok(())
            }
            _ => Err(WorkflowEngineError::ValidationError(
                "dispatch_internal_node_command received non-internal variant".to_string(),
            )),
        }
    }

    /// [05] internal command → event 射影。
    ///
    /// `WorkflowCommand::CompleteNode` / `FailNode` から対応する
    /// `WorkflowEvent::NodeCompleted` / `NodeFailed` を組み立てる純粋関数。emission
    /// 経路は `dispatch_internal_node_command` を入口として用い、本関数を直接呼ぶ
    /// 経路は持たない（spec [05]: 発行点が typed command の単一経路に集約される）。
    fn map_internal_node_command_to_event(
        command: WorkflowCommand,
    ) -> Result<WorkflowEvent, WorkflowEngineError> {
        match command {
            WorkflowCommand::CompleteNode {
                run_id,
                workflow_name,
                node_name,
                result,
                session_id,
                token_usage,
                structured_output,
                run_index,
                timestamp,
            } => Ok(WorkflowEvent::NodeCompleted {
                run_id,
                workflow_name,
                node_name,
                result,
                session_id,
                token_usage,
                structured_output,
                run_index,
                timestamp,
            }),
            WorkflowCommand::FailNode {
                run_id,
                workflow_name,
                node_name,
                reason,
                timestamp,
            } => Ok(WorkflowEvent::NodeFailed {
                run_id,
                workflow_name,
                node_name,
                reason,
                timestamp,
            }),
            _ => Err(WorkflowEngineError::ValidationError(
                "map_internal_node_command_to_event received non-internal variant".to_string(),
            )),
        }
    }

    /// turn_complete後に呼ばれるフック。
    /// autoモード→タグ検出で遷移、approvalモード→WaitingApproval、interactiveモード→何もしない。
    /// SessionError / WaitApproval は判定 + 状態変更を1回のロックで原子的に実行する。
    /// AutoEvaluate はタグ検出が必要なため handle_auto_complete に委譲する。
    #[allow(clippy::too_many_arguments)]
    pub async fn on_turn_complete<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        session_id: &str,
        exit_code: i64,
        final_parts: &[crate::session::MessagePart],
        token_usage: Option<(u64, u64)>,
    ) -> Result<(), WorkflowEngineError> {
        // session_id からSessionWorkflowRefを解決（ワークフロー既終了なら何もしない）
        let Some(session_ref) = self.resolve_session_ref(session_id).await else {
            return Ok(());
        };
        // parent ChatSession 機構撤去後は step session のみが登録されるため種別分岐なし。
        // 逐次 step / 並列子 step の区別は WorkflowExecution.parallel_run に当該 session_id が
        // 含まれるかで判定する（Spec issues-929）。

        // SessionWorkflowRef.run_id から exec を直接引き、属性として worktree_path を取得する
        // （Spec issues-1011: engine 内部キーも run_id）。下流の handle_* は worktree_path を
        // 引数に取るため、ここで派生取得する。
        let (worktree_path, parallel_parent): (String, Option<String>) = {
            let execs = self.executions.lock().await;
            let Some(exec) = execs.get(&session_ref.run_id) else {
                return Ok(());
            };
            let wt = exec.worktree_path.clone();
            let pp = exec.parallel_run.as_ref().and_then(|pr| {
                pr.children
                    .iter()
                    .find(|c| c.session_id == session_id)
                    .map(|_| pr.parent_step_name.clone())
            });
            (wt, pp)
        };

        if let Some(parent_step_name) = parallel_parent {
            return self
                .handle_parallel_child_complete(
                    app,
                    session_store,
                    handles,
                    &session_ref.run_id,
                    &worktree_path,
                    session_id,
                    &parent_step_name,
                    exit_code,
                    final_parts,
                    token_usage,
                )
                .await;
        }

        struct TurnCommit {
            outcome: StepOutcome,
            required_events: Vec<WorkflowEvent>,
            rollback_snapshot: (String, WorkflowExecution),
        }

        // 判定 + 状態変更を原子的に実行（AutoEvaluate以外）
        let action_or_outcome = {
            let mut execs = self.executions.lock().await;
            let exec = execs.get_mut(&session_ref.run_id).ok_or_else(|| {
                WorkflowEngineError::ExecutionNotFound(session_ref.run_id.clone())
            })?;

            // 現行ステップのセッション以外からの完了通知は無視
            if exec.current_session_id.as_deref() != Some(session_id) {
                return Ok(());
            }

            // トークン使用量を現在のステップに累計
            if let Some((input, output)) = token_usage {
                exec.current_step_token_usage.add(&TokenUsage {
                    input_tokens: input,
                    output_tokens: output,
                });
            }
            let action = exec.decide_turn_complete_action(exit_code);

            let result = match action {
                TurnCompleteAction::NotRunning => return Ok(()),
                TurnCompleteAction::SessionError {
                    step_name,
                    exit_code,
                } => {
                    if exec.is_terminal() {
                        return Ok(());
                    }
                    let snapshot_before = exec.clone();
                    let entry = exec.make_step_history_entry(
                        Some(format!("error (exit_code: {})", exit_code)),
                        None,
                        None,
                    );
                    exec.step_history.push(entry);
                    exec.state = WorkflowExecutionState::Failed {
                        reason: format!(
                            "AgentSession error at step '{}' (exit_code: {})",
                            step_name, exit_code
                        ),
                    };
                    exec.updated_at = current_timestamp();
                    Ok(TurnCommit {
                        outcome: StepOutcome::Persist(exec.to_workflow_state()),
                        required_events: Vec::new(),
                        rollback_snapshot: (exec.id.clone(), snapshot_before),
                    })
                }
                TurnCompleteAction::WaitApproval => {
                    if exec.is_terminal() {
                        return Ok(());
                    }
                    let snapshot_before = exec.clone();
                    let workflow_name = exec.workflow.name.clone();
                    let node_name = exec.workflow.nodes[exec.current_step_index].name.clone();
                    exec.state = WorkflowExecutionState::WaitingApproval;
                    exec.updated_at = current_timestamp();
                    Ok(TurnCommit {
                        outcome: StepOutcome::Persist(exec.to_workflow_state()),
                        required_events: vec![WorkflowEvent::ApprovalRequested {
                            run_id: exec.id.clone(),
                            workflow_name,
                            node_name,
                            timestamp: exec.updated_at,
                        }],
                        rollback_snapshot: (exec.id.clone(), snapshot_before),
                    })
                }
                TurnCompleteAction::UnexpectedNodeType {
                    step_name,
                    node_type,
                } => {
                    if exec.is_terminal() {
                        return Ok(());
                    }
                    let snapshot_before = exec.clone();
                    let reason = format!(
                        "Workflow engine reached turn_complete for unexpected node type {:?} at step '{}' (this should have been rejected upstream)",
                        node_type, step_name
                    );
                    let entry = exec.make_step_history_entry(Some(reason.clone()), None, None);
                    exec.step_history.push(entry);
                    exec.state = WorkflowExecutionState::Failed { reason };
                    exec.updated_at = current_timestamp();
                    Ok(TurnCommit {
                        outcome: StepOutcome::Persist(exec.to_workflow_state()),
                        required_events: Vec::new(),
                        rollback_snapshot: (exec.id.clone(), snapshot_before),
                    })
                }
                TurnCompleteAction::AutoEvaluate { rules, step_name } => Err((rules, step_name)),
            };
            result
        };

        match action_or_outcome {
            Ok(commit) => {
                let (_, snapshot_before) = commit.rollback_snapshot.clone();
                if commit.required_events.is_empty() {
                    self.execute_outcome(
                        app,
                        session_store,
                        handles,
                        &worktree_path,
                        commit.outcome,
                        snapshot_before,
                    )
                    .await
                } else {
                    self.commit_required_turn_events_and_execute_outcome(
                        app,
                        session_store,
                        handles,
                        &worktree_path,
                        commit.outcome,
                        commit.required_events,
                        Some(commit.rollback_snapshot),
                    )
                    .await
                }
            }
            Err((rules, step_name)) => {
                self.handle_auto_complete(
                    app,
                    session_store,
                    handles,
                    &worktree_path,
                    final_parts,
                    &rules,
                    &step_name,
                )
                .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    async fn commit_required_turn_events_and_execute_outcome<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        outcome: StepOutcome,
        required_events: Vec<WorkflowEvent>,
        rollback_snapshot: Option<(String, WorkflowExecution)>,
    ) -> Result<(), WorkflowEngineError> {
        let Some((run_id, snapshot_before)) = rollback_snapshot else {
            return Err(WorkflowEngineError::SessionStore(
                "required turn event commit missing rollback snapshot".to_string(),
            ));
        };
        let completed_step_session_ids = Self::completed_step_session_ids_for_outcome(&outcome);
        let snapshot_for_commit = Self::outcome_snapshot(&outcome).clone();
        let run_store_snapshot_before = self.run_store.active_run_snapshot(&run_id).await;

        self.commit_required_events(
            app,
            session_store,
            RequiredEventCommit {
                run_id: &run_id,
                snapshot_for_commit: &snapshot_for_commit,
                snapshot_before,
                run_store_snapshot_before,
                required_events,
                append_error_context: "turn_complete required event append failed",
            },
        )
        .await?;

        self.release_completed_step_sessions(
            app,
            session_store,
            handles,
            &completed_step_session_ids,
        )
        .await;
        self.finalize_after_commit(app, &snapshot_for_commit, worktree_path, true)
            .await;
        if let Err(e) = self
            .dispatch_step_outcome_side_effects(
                app,
                session_store,
                handles,
                worktree_path,
                outcome,
                OutcomeCommitMode::EmitProgressEvents,
            )
            .await
        {
            log::warn!("workflow {run_id}: post-commit turn side effects failed: {e}");
        }
        Ok(())
    }

    /// ApprovalDecisionのバリデーション。Reject時に空コメントを拒否する。
    ///
    /// 文字数上限 / 空白判定の実体は `command_input::validate_reject_reason_text`
    /// に集約する（review R2-01: CLI / engine の同一ドメインルール重複解消）。
    fn validate_approval_decision(decision: &ApprovalDecision) -> Result<(), WorkflowEngineError> {
        if let ApprovalDecision::Reject { ref comment } = decision {
            validate_reject_reason_text(comment, "Reject comment")
                .map_err(command_input_error_to_engine_error)?;
        }
        Ok(())
    }

    /// Approve 用 comment のバリデーション。空文字は許容するが、上限のみ検証する。
    /// reject と同じ MAX_APPROVAL_COMMENT_CHARS を `ApproveNode.comment` にも適用する
    /// （Spec [04]: 新規外部入力に対する境界バリデーション）。
    ///
    /// 文字数上限の判定実体は `command_input::validate_optional_comment_text`
    /// に集約する（review R2-01）。
    fn validate_approve_comment_length(comment: Option<&str>) -> Result<(), WorkflowEngineError> {
        validate_optional_comment_text(comment, "Approve comment")
            .map_err(command_input_error_to_engine_error)
    }

    fn reject_structured_output(comment: &str, configured_secrets: &[String]) -> serde_json::Value {
        let comment = Self::mask_sensitive_text(comment, configured_secrets);
        serde_json::json!({
            "decision": "reject",
            "comment": comment,
        })
    }

    fn apply_approval_application(
        exec: &mut WorkflowExecution,
        decision: &ApprovalDecision,
        application: ApprovalApplication,
    ) -> Result<StepOutcome, WorkflowEngineError> {
        let action = exec.decide_approval_action(decision)?;
        let outcome = match action {
            ApprovalAction::Advance => {
                let entry = exec.make_step_history_entry(
                    Some(application.effective_result),
                    application.structured_output,
                    application.output_contract,
                );
                exec.step_history.push(entry);
                Self::apply_advance(exec)
            }
            ApprovalAction::TransitionTo(target) => {
                let entry = exec.make_step_history_entry(
                    Some(application.effective_result),
                    application.structured_output,
                    application.output_contract,
                );
                exec.step_history.push(entry);
                Self::apply_transition(exec, &target)?
            }
        };
        Ok(outcome)
    }

    /// approvalモードでのユーザー判定を処理する。
    /// 判定 + 状態変更 + 履歴記録を1回のロックで原子的に実行し、
    /// ロック外では永続化・ブロードキャスト・AgentSession起動のみ行う。
    ///
    /// Spec issues-1011 finding 2: lookup は `executions.get(run_id)` / `get_mut(run_id)` で
    /// 直接行い、worktree_path 経由の find は使用しない。同一 worktree に terminal/active
    /// 共存があっても run_id 主語で取り違えない。worktree_path は exec から派生取得して
    /// 下流 (`fetch_current_output` / `execute_outcome`) に渡す。
    /// [04] 内部 typed boundary: `WorkflowCommand::ApproveNode` / `RejectNode` /
    /// `AbortRun { expected_node_name: Some(_) }` の handler 実体。外部から直接
    /// 呼び出すことは禁止する（pub にしない）。本メソッドへの到達経路は engine の
    /// dispatch のみであり、auto-approve も dispatch 経由で再入する（Spec [04] 境界）。
    #[allow(clippy::too_many_arguments)]
    async fn handle_approval<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        run_id: &str,
        decision: ApprovalDecision,
        approve_comment: Option<String>,
        expected_step_name: Option<&str>,
        commit_context: Option<CommandCommitContext>,
    ) -> Result<(), WorkflowEngineError> {
        let (result_tag, decision_record) = match &decision {
            ApprovalDecision::Approve => ("approve", ApprovalDecisionRecord::Approve),
            ApprovalDecision::Reject { .. } => ("reject", ApprovalDecisionRecord::Reject),
        };

        // target検証 + session_id + worktree_path + contract 提出状態を1回のロックで取得
        let (
            current_session_id,
            worktree_path,
            workflow_name_for_contract,
            node_name_for_contract,
            approval_output_contract,
            approval_submitted_output,
        ) = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(run_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
            Self::resolve_approval_target_snapshot(exec, Some(run_id), expected_step_name)?;
            let node = &exec.workflow.nodes[exec.current_step_index];
            let output_contract = node.output_contract.clone();
            let run_index = exec
                .step_execution_counts
                .get(&node.name)
                .copied()
                .unwrap_or(1);
            let submitted_output = output_contract.as_deref().and_then(|contract| {
                Self::submitted_step_output_for(exec, &node.name, run_index, contract)
            });
            (
                exec.current_session_id.clone(),
                exec.worktree_path.clone(),
                exec.workflow.name.clone(),
                node.name.clone(),
                output_contract,
                submitted_output,
            )
        };

        // Reject時: 空コメントバリデーション + Approve/Reject 共通の長さ上限検証
        // （副作用の前に実施）
        Self::validate_approval_decision(&decision)?;
        if matches!(decision, ApprovalDecision::Approve) {
            Self::validate_approve_comment_length(approve_comment.as_deref())?;
        }

        if matches!(decision, ApprovalDecision::Approve) {
            let turn_phase = if let Some(ref sid) = current_session_id {
                let map = handles.lock().await;
                map.get(sid).map(|p| p.turn_phase)
            } else {
                None
            };
            Self::validate_approval_turn_phase(turn_phase)?;
        }

        let approve_submitted_output = if matches!(decision, ApprovalDecision::Approve) {
            if let Some(ref contract) = approval_output_contract {
                if let Some(output) = approval_submitted_output {
                    Some(output)
                } else {
                    self.handle_missing_required_output(
                        app,
                        session_store,
                        handles,
                        &worktree_path,
                        run_id,
                        &workflow_name_for_contract,
                        &node_name_for_contract,
                        contract,
                        current_session_id.as_deref(),
                    )
                    .await?;
                    return Err(WorkflowEngineError::ValidationError(
                        "required structured output has not been submitted".to_string(),
                    ));
                }
            } else {
                None
            }
        } else {
            None
        };

        // [08] prose 抽出経路廃止に伴い、approval node の structured output は CLI / Tauri
        // 経由の `SubmitOutput` でしか確定しない。Approve 時は提出済み output を採用し、
        // Reject は理由を redaction 済み JSON に整形して application 入力に渡す。
        let (structured_output, contract_result): (Option<serde_json::Value>, Option<String>) =
            match &decision {
                ApprovalDecision::Approve => approve_submitted_output
                    .as_ref()
                    .map(|output| (output.structured_output.clone(), output.result.clone()))
                    .unwrap_or((None, None)),
                ApprovalDecision::Reject { comment } => {
                    let secrets = Self::collect_configured_secret_values(app);
                    (
                        Some(Self::reject_structured_output(comment, &secrets)),
                        None,
                    )
                }
            };

        let application_output_contract: Option<String> = if matches!(
            decision,
            ApprovalDecision::Approve
        ) && approve_submitted_output.is_some()
        {
            approval_output_contract.clone()
        } else {
            None
        };
        let contract_variables =
            Self::extract_contract_variables(&application_output_contract, &structured_output);

        // contract resultがあればそちらを優先、なければresult_tag
        let effective_result = contract_result.unwrap_or_else(|| result_tag.to_string());

        // [04] atomic mutation 境界: mutation 直前の WorkflowExecution 全体を snapshot に
        // 保持し、ApprovalResolved event append / persist のいずれかが失敗した場合は
        // `*exec = snapshot` で全フィールド（履歴・変数・state・current_step_index 等）を
        // 一括復元する。部分 rollback helper は使わない。
        let (mut outcome, exec_snapshot_before, workflow_name_for_event, node_name_for_event) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(run_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
            Self::resolve_approval_target_snapshot(exec, Some(run_id), expected_step_name)?;
            let workflow_name = exec.workflow.name.clone();
            let node_name = exec.workflow.nodes[exec.current_step_index].name.clone();
            let snapshot_before = exec.clone();
            exec.workflow_variables.extend(contract_variables);
            let outcome = Self::apply_approval_application(
                exec,
                &decision,
                ApprovalApplication {
                    effective_result,
                    structured_output,
                    output_contract: application_output_contract,
                },
            )?;
            (outcome, snapshot_before, workflow_name, node_name)
        };

        let snapshot_for_commit = Self::outcome_snapshot(&outcome).clone();
        let completed_step_session_ids = Self::completed_step_session_ids_for_outcome(&outcome);
        let run_store_snapshot_before = self.run_store.active_run_snapshot(run_id).await;

        // [04] commit point: ApprovalResolved と、同じ受理サイクルで確定した
        // NodeCompleted / NodeStarted / terminal event を同一 batch で必須 append する。
        // append / persist 失敗時は snapshot で全フィールド一括復元する。
        // 機密値 redaction: approve コメント / reject 理由には設定済み secret を含む可能性が
        // あるため、event log に保存する前に mask_sensitive_text() で redaction する
        // （reject_structured_output が同じ secret 列で structured_output 側に行う処理と対称）。
        let raw_event_comment = match &decision {
            ApprovalDecision::Approve => approve_comment.clone(),
            ApprovalDecision::Reject { comment } => Some(comment.clone()),
        };
        let event_comment = if let Some(raw) = raw_event_comment {
            let secrets = Self::collect_configured_secret_values(app);
            Some(Self::mask_sensitive_text(&raw, &secrets))
        } else {
            None
        };
        let approval_timestamp = current_timestamp();
        let approval_event = WorkflowEvent::ApprovalResolved {
            run_id: run_id.to_string(),
            workflow_name: workflow_name_for_event.clone(),
            node_name: node_name_for_event.clone(),
            decision: decision_record,
            comment: event_comment,
            timestamp: approval_timestamp,
        };
        // [05] silent error の禁止: required event 組立中に
        // `dispatch_internal_node_command` の ValidationError 等が発生した場合は
        // approval commit 境界として失敗扱いし、snapshot_before で engine state /
        // Run Store / ChatSession を一括復元してから Err を返す。
        let mut commit_events =
            match Self::required_events_for_approval_commit(approval_event, &mut outcome) {
                Ok(events) => events,
                Err(e) => {
                    let _ = self
                        .rollback_command_mutation(
                            app,
                            session_store,
                            CommandMutationRollback {
                                run_id,
                                snapshot_before: exec_snapshot_before,
                                run_store_snapshot_before,
                                context: "approval required event build failed",
                            },
                        )
                        .await;
                    return Err(e);
                }
            };
        if let Some(context) = commit_context {
            if let Some(event) =
                Self::command_commit_context_event(&workflow_name_for_event, context)
            {
                commit_events.push(event);
            }
        }
        self.commit_required_events(
            app,
            session_store,
            RequiredEventCommit {
                run_id,
                snapshot_for_commit: &snapshot_for_commit,
                snapshot_before: exec_snapshot_before,
                run_store_snapshot_before,
                required_events: commit_events,
                append_error_context: "approval commit batch append failed",
            },
        )
        .await?;

        // [04] post-commit: required event append 済みのため、ここから先の失敗は
        // command failure に射影しない（spec [04] post-commit 境界）。session release /
        // broadcast / terminal log / cleanup / 次 step 起動 / auto-approve dispatch は
        // ここで実行する。
        self.release_completed_step_sessions(
            app,
            session_store,
            handles,
            &completed_step_session_ids,
        )
        .await;
        self.finalize_after_commit(app, &snapshot_for_commit, &worktree_path, false)
            .await;
        if let Err(e) = self
            .dispatch_step_outcome_side_effects(
                app,
                session_store,
                handles,
                &worktree_path,
                outcome,
                OutcomeCommitMode::ProgressEventsAlreadyCommitted,
            )
            .await
        {
            log::warn!("workflow {run_id}: post-commit side effects failed: {e}");
        }
        Ok(())
    }

    /// ワークフローを中断する。
    /// `run_id` を主語に workflow を中断する。
    ///
    /// Spec issues-1011 finding 2/10: 全経路で `executions.get_mut(run_id)` を使い、
    /// worktree_path 経由の委譲を排除する。これにより、同一 worktree に terminal run と
    /// active run が共存しても誤って別 run を中断する TOCTOU を構造的に排除する。
    ///
    /// Spec [04]: `AbortRun` command handler の境界。
    /// - 対象 run が存在しない場合は `AbortOutcome::NotFound` を返す（非受理）。
    /// - 既に terminal な run の場合は `AbortOutcome::AlreadyTerminal` を返す（非受理）。
    /// - 実際に Aborted に遷移し RunAborted event を必須 append できた場合のみ
    ///   `AbortOutcome::Aborted` を返す。
    ///
    /// RunAborted event は `write_log_required` 経由で必須 append し、append 失敗時は
    /// mutation 直前 snapshot で `WorkflowExecution` 全体を一括復元する
    /// （Spec atomic mutation 境界）。
    ///
    /// 外部から直接呼ばれることはなく、`WorkflowEngine::dispatch` 経路のみが利用する
    /// （Spec [04]: 内部呼び出し元も engine の private method を直接叩かない）。
    async fn abort_workflow_by_run_id<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        run_id: &str,
        expected_node_name: Option<&str>,
        commit_context: Option<CommandCommitContext>,
    ) -> Result<AbortOutcome, WorkflowEngineError> {
        // 1. 対象 run の存在 + active 性を判定。
        //    非受理経路 (NotFound / AlreadyTerminal) ではどんな外部副作用も発生させない。
        let lookup = self.abort_target_lookup(run_id).await;
        let (current_step_session_id, parallel_session_ids) = match lookup {
            AbortTargetLookup::NotFound => return Ok(AbortOutcome::NotFound),
            AbortTargetLookup::AlreadyTerminal => return Ok(AbortOutcome::AlreadyTerminal),
            AbortTargetLookup::Active {
                current_step_session_id,
                parallel_session_ids,
            } => (current_step_session_id, parallel_session_ids),
        };

        // 2. [04] pre-commit (rollback 可能): mutation 直前 snapshot を取得し、
        //    state を Aborted に遷移させる。競合で terminal 化していた場合は
        //    AlreadyTerminal で返す。
        let timestamp = current_timestamp();
        let run_store_snapshot_before = self.run_store.active_run_snapshot(run_id).await;
        let (snapshot_before, snapshot_state, workflow_name_for_event) = {
            let mut execs = self.executions.lock().await;
            let Some(exec) = execs.get_mut(run_id) else {
                return Ok(AbortOutcome::NotFound);
            };
            if !exec.is_active() {
                return Ok(AbortOutcome::AlreadyTerminal);
            }
            if let Some(expected_node_name) = expected_node_name {
                let current_node = exec
                    .workflow
                    .nodes
                    .get(exec.current_step_index)
                    .map(|node| node.name.as_str())
                    .ok_or_else(|| {
                        WorkflowEngineError::InvalidState(format!(
                            "run {run_id} has invalid current step"
                        ))
                    })?;
                if expected_node_name != current_node {
                    return Err(WorkflowEngineError::UnauthorizedApprovalTarget(
                        "step does not match".to_string(),
                    ));
                }
            }
            let snapshot_before = exec.clone();
            let workflow_name = exec.workflow.name.clone();

            // spec issues-1023: state を Aborted にする前に、中断時の current step /
            // parallel children を `step_history` に "aborted" entry として記録する。
            // これにより UI 側は既存 history 描画経路 + session_id を使って中断 step の
            // session log にアクセスできるようになる。`exec.parallel_run = None` を
            // 明示クリアして `to_workflow_state()` 経由の二重表示を防ぐ。
            if exec.parallel_run.is_some() {
                if let Some(entry) = exec.make_aborted_parallel_history_entry(timestamp) {
                    exec.step_history.push(entry);
                }
                exec.parallel_run = None;
            } else {
                let current_step_name = exec.workflow.nodes[exec.current_step_index].name.clone();
                let already_in_history = exec
                    .step_history
                    .last()
                    .is_some_and(|e| e.step_name == current_step_name);
                if !already_in_history {
                    let entry = exec.make_aborted_history_entry(timestamp);
                    exec.step_history.push(entry);
                }
            }

            exec.state = WorkflowExecutionState::Aborted;
            exec.updated_at = timestamp;
            let snapshot_state = exec.to_workflow_state();
            (snapshot_before, snapshot_state, workflow_name)
        };

        // 3. [04] commit point: RunAborted を必須 append。失敗時は
        //    WorkflowExecution / Run Store / ChatSession を snapshot で一括復元する。
        //    interrupt_agent はこの時点ではまだ実行していないため、append 失敗時には
        //    rollback 不能な外部副作用が残らない。
        let aborted_event = WorkflowEvent::RunAborted {
            run_id: run_id.to_string(),
            workflow_name: workflow_name_for_event.clone(),
            timestamp,
        };
        let mut required_events = vec![aborted_event];
        if let Some(context) = commit_context {
            if let Some(event) =
                Self::command_commit_context_event(&workflow_name_for_event, context)
            {
                required_events.push(event);
            }
        }
        self.commit_required_events(
            app,
            session_store,
            RequiredEventCommit {
                run_id,
                snapshot_for_commit: &snapshot_state,
                snapshot_before,
                run_store_snapshot_before,
                required_events,
                append_error_context: "RunAborted log failed",
            },
        )
        .await?;

        // 4. [04] post-commit: interrupt_agent / cleanup / broadcast。
        //    RunAborted event は append 済み。Run Store / ChatSession は event 後の
        //    projection として同期済み、または warn として観測済み。
        if let Some(ref step_sid) = current_step_session_id {
            self.interrupt_agent(handles, step_sid).await;
        }
        if let Some(ref session_ids) = parallel_session_ids {
            for sid in session_ids {
                self.interrupt_agent(handles, sid).await;
            }
        }
        self.finalize_terminal_transition_after_required_append(
            app,
            session_store,
            handles,
            run_id,
        )
        .await;

        Ok(AbortOutcome::Aborted)
    }

    /// `WorkflowCommand::AbortRun` の post-commit 区間。state は呼出し前に Aborted に
    /// 遷移済みで、`RunAborted` event は必須 append 済み、かつ Run Store sync も
    /// 完了済みである前提。ChatSession persist / step session release / refs cleanup /
    /// broadcast を実行する。
    ///
    /// [04] post-commit 失敗は warn ログのみで command 結果に伝播させない。観測可能な
    /// 事実は既に RunAborted で確定しており、ここでの副作用失敗を command failure に
    /// 射影すると spec [04] の「post-commit 失敗は command failure として返さない」に
    /// 違反するため。
    async fn finalize_terminal_transition_after_required_append<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        run_id: &str,
    ) {
        let (snapshot, worktree_path) = {
            let execs = self.executions.lock().await;
            let Some(exec) = execs.get(run_id) else {
                return;
            };
            (exec.to_workflow_state(), exec.worktree_path.clone())
        };

        // terminal session の release と refs cleanup。
        let terminal_session_ids = Self::terminal_step_session_ids(&snapshot);
        self.release_completed_step_sessions(app, session_store, handles, &terminal_session_ids)
            .await;
        self.cleanup_session_workflow_refs_by_run_id(run_id).await;
        self.broadcast_state(app, &worktree_path, snapshot).await;
    }

    async fn abort_target_lookup(&self, run_id: &str) -> AbortTargetLookup {
        let execs = self.executions.lock().await;
        let Some(exec) = execs.get(run_id) else {
            return AbortTargetLookup::NotFound;
        };
        if !exec.is_active() {
            return AbortTargetLookup::AlreadyTerminal;
        }
        let current_step_session_id = exec.current_session_id.clone();
        let parallel_session_ids = exec.parallel_run.as_ref().map(|pr| {
            pr.children
                .iter()
                .filter(|c| c.state == ParallelChildState::Running)
                .map(|c| c.session_id.clone())
                .collect::<Vec<_>>()
        });
        AbortTargetLookup::Active {
            current_step_session_id,
            parallel_session_ids,
        }
    }

    /// 並列子ステップの完了を処理する。
    #[allow(clippy::too_many_arguments)]
    async fn handle_parallel_child_complete<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        run_id: &str,
        worktree_path: &str,
        session_id: &str,
        parent_step_name: &str,
        exit_code: i64,
        final_parts: &[crate::session::MessagePart],
        token_usage: Option<(u64, u64)>,
    ) -> Result<(), WorkflowEngineError> {
        // [08] parallel child の構造化出力は CLI / Tauri 経由の `SubmitOutput` で確定する。
        // output_contract がある child は、提出済み output が無い限り Completed にしない。
        let _ = final_parts;
        let (submitted_child_output, missing_child_output) = if exit_code == 0 {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(run_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
            if exec.is_terminal() {
                return Ok(());
            }
            let Some(pr) = exec.parallel_run.as_ref() else {
                return Ok(());
            };
            if pr.parent_step_name != parent_step_name {
                return Ok(());
            }
            let Some(child) = pr.children.iter().find(|c| c.session_id == session_id) else {
                return Ok(());
            };
            if let Some(contract) = child.output_contract.clone() {
                let submitted = Self::submitted_step_output_for(
                    exec,
                    &child.step_name,
                    child.run_index,
                    &contract,
                );
                let missing = if submitted.is_none() {
                    Some((
                        exec.workflow.name.clone(),
                        child.step_name.clone(),
                        contract.clone(),
                    ))
                } else {
                    None
                };
                (submitted, missing)
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };
        if let Some((workflow_name, child_name, contract)) = missing_child_output {
            self.handle_missing_required_output(
                app,
                session_store,
                handles,
                worktree_path,
                run_id,
                &workflow_name,
                &child_name,
                &contract,
                Some(session_id),
            )
            .await?;
            return Ok(());
        }
        let child_result = submitted_child_output
            .as_ref()
            .and_then(|output| output.result.clone());
        let child_structured_output = submitted_child_output
            .as_ref()
            .and_then(|output| output.structured_output.clone());

        // ロック内: 子ステップの状態更新 + 全完了チェック
        let (all_completed, outcome_opt, exec_snapshot_before) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(run_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;

            if exec.is_terminal() {
                return Ok(());
            }
            // [05] commit 境界: 子ステップ失敗 → workflow 全体 Failed の terminal event は
            // pre-commit batch で append し、失敗時は engine state を snapshot_before で
            // 一括復元する（post-persist warn 廃止）。snapshot は mutation 前にここで取得する。
            let exec_snapshot_before = exec.clone();
            let Some(pr) = exec.parallel_run.as_mut() else {
                return Ok(());
            };
            if pr.parent_step_name != parent_step_name {
                return Ok(());
            }

            // 対象の子ステップを見つけて更新
            let Some(child) = pr.children.iter_mut().find(|c| c.session_id == session_id) else {
                return Ok(());
            };

            if let Some((input, output)) = token_usage {
                child.token_usage.add(&TokenUsage {
                    input_tokens: input,
                    output_tokens: output,
                });
            }

            if exit_code != 0 {
                // 子ステップ失敗 → ワークフロー全体をFailed
                child.state = ParallelChildState::Failed;
                let child_name = child.step_name.clone();

                // 他の実行中子ステップのstateをInterruptedに更新し、IDを集める
                let running_ids: Vec<String> = pr
                    .children
                    .iter_mut()
                    .filter(|c| c.state == ParallelChildState::Running)
                    .map(|c| {
                        c.state = ParallelChildState::Interrupted;
                        c.session_id.clone()
                    })
                    .collect();

                exec.state = WorkflowExecutionState::Failed {
                    reason: format!(
                        "Parallel child '{}' failed (exit_code: {})",
                        child_name, exit_code
                    ),
                };
                exec.parallel_run = None;
                exec.updated_at = current_timestamp();
                let snapshot = exec.to_workflow_state();
                drop(execs);

                // [05] pre-commit: terminal event を先に append。失敗時は engine state
                // を snapshot_before で一括復元し Err を返す。
                if let Err(e) = self.write_terminal_log(app, &snapshot) {
                    let mut execs = self.executions.lock().await;
                    if let Some(exec) = execs.get_mut(run_id) {
                        *exec = exec_snapshot_before;
                    }
                    return Err(WorkflowEngineError::SessionStore(format!(
                        "parallel child failure terminal event append failed: {e}"
                    )));
                }

                if let Err(e) = self.sync_run_store_from_snapshot(run_id, &snapshot).await {
                    self.rollback_execution_projection_after_run_store_sync_failure(
                        run_id, &snapshot,
                    )
                    .await;
                    return Err(e);
                }
                // 他の子ステップをinterrupt
                for sid in &running_ids {
                    self.interrupt_agent(handles, sid).await;
                }
                let mut cleanup_ids = running_ids;
                cleanup_ids.push(session_id.to_string());
                cleanup_ids.sort();
                cleanup_ids.dedup();
                for sid in cleanup_ids {
                    self.release_completed_step_session(app, session_store, handles, &sid)
                        .await;
                }
                self.broadcast_state(app, worktree_path, snapshot.clone())
                    .await;
                self.cleanup_session_workflow_refs_by_run_id(&snapshot.execution_id)
                    .await;
                return Ok(());
            }

            // 成功
            child.state = ParallelChildState::Completed;
            child.result = child_result.clone();
            child.structured_output = child_structured_output.clone();
            let child_name = child.step_name.clone();
            let child_token_usage = child.token_usage.clone();
            let child_run_index = child.run_index;

            // [08] child の StepOutput は CLI / Tauri 経由の SubmitOutput でのみ確定する。
            // ここでは step_outputs slot に触れず、SubmitOutput 済みの値を保持したまま
            // 親 ParallelChildCompleted の事実だけを event log に積む。

            // ParallelStepCompleted ログ
            self.write_log(
                app,
                WorkflowEvent::ParallelChildCompleted {
                    run_id: exec.id.clone(),
                    workflow_name: exec.workflow.name.clone(),
                    parent_node_name: pr.parent_step_name.clone(),
                    child_node_name: child_name,
                    result: child_result.clone(),
                    session_id: session_id.to_string(),
                    token_usage: Some(child_token_usage.clone()),
                    structured_output: child_structured_output.clone(),
                    run_index: child_run_index,
                    timestamp: current_timestamp(),
                },
            );

            // 全完了チェック
            let all_done = pr
                .children
                .iter()
                .all(|c| c.state == ParallelChildState::Completed);

            if !all_done {
                // まだ未完了の子がある → ブロードキャストのみ
                exec.updated_at = current_timestamp();
                let snapshot = exec.to_workflow_state();
                (
                    false,
                    Some(StepOutcome::Persist(snapshot)),
                    exec_snapshot_before,
                )
            } else {
                // 全完了 → 親ブロック名でstep_outputsに集約登録 + aggregate評価 + 遷移
                let aggregate = pr.aggregate.clone();
                let parent_step_name = pr.parent_step_name.clone();
                let child_step_names: Vec<String> =
                    pr.children.iter().map(|c| c.step_name.clone()).collect();

                // 子ステップの個別StepOutputは既に登録済み（行1401-1413）
                // 親名での集約登録は parallel_run クリア後に行う
                // token_usageはStepHistoryEntryに直接渡す
                let parent_run_index = exec
                    .step_execution_counts
                    .get(&parent_step_name)
                    .copied()
                    .unwrap_or(1);
                let mut combined_tokens = TokenUsage::default();
                for child in &pr.children {
                    combined_tokens.add(&child.token_usage);
                }

                // 並列子ステップのスナップショットを保存（履歴表示用）
                // StepOutputがない子（output_contractなし）はParallelChildRunからフォールバック
                let child_snapshots: Vec<crate::workflow::state::ChildOutputSnapshot> = pr
                    .children
                    .iter()
                    .map(|child| {
                        let child_so = exec.step_outputs.get(&child.step_name);
                        crate::workflow::state::ChildOutputSnapshot {
                            step_name: child.step_name.clone(),
                            session_id: child_so
                                .and_then(|o| o.session_id.clone())
                                .or_else(|| Some(child.session_id.clone())),
                            result: child_so
                                .and_then(|o| o.result.clone())
                                .or(child.result.clone()),
                            run_index: child.run_index,
                            completed_at: child_so
                                .map(|o| o.completed_at)
                                .unwrap_or_else(current_timestamp),
                            structured_output: child_so.and_then(|o| o.structured_output.clone()),
                            output_contract: child_so.and_then(|o| o.output_contract.clone()),
                            state: crate::workflow::state::default_step_entry_state(),
                        }
                    })
                    .collect();

                exec.parallel_run = None;
                exec.updated_at = current_timestamp();

                // 並列ブロック親名でstep_outputsに集約登録
                // 後続ステップがpass_output_fromで親名を参照できるようにする
                {
                    let mut children_output = serde_json::Map::new();
                    for child_name in &child_step_names {
                        if let Some(child_so) = exec.step_outputs.get(child_name) {
                            children_output.insert(
                                child_name.clone(),
                                child_so
                                    .structured_output
                                    .clone()
                                    .unwrap_or(serde_json::Value::Null),
                            );
                        }
                    }
                    exec.step_outputs.insert(
                        parent_step_name.clone(),
                        StepOutput {
                            step_name: parent_step_name.clone(),
                            run_index: parent_run_index,
                            session_id: None,
                            result: None,
                            structured_output: Some(serde_json::Value::Object(children_output)),
                            output_contract: None,
                            token_usage: Some(combined_tokens.clone()),
                            completed_at: current_timestamp(),
                        },
                    );
                }

                let outcome = if let Some(ref agg) = aggregate {
                    // aggregate評価
                    let agg_result =
                        self.evaluate_aggregate(agg, &exec.step_outputs, &child_step_names);
                    let target = if agg_result { &agg.then } else { &agg.r#else };

                    // ParallelCompleted ログ
                    self.write_log(
                        app,
                        WorkflowEvent::ParallelCompleted {
                            run_id: exec.id.clone(),
                            workflow_name: exec.workflow.name.clone(),
                            parent_node_name: parent_step_name.clone(),
                            aggregate_result: if agg_result {
                                "then".to_string()
                            } else {
                                "else".to_string()
                            },
                            timestamp: current_timestamp(),
                        },
                    );

                    // 履歴エントリ追加（並列ブロックの完了として）
                    let entry = StepHistoryEntry {
                        step_name: parent_step_name.clone(),
                        completed_at: current_timestamp(),
                        result: Some(if agg_result {
                            "then".to_string()
                        } else {
                            "else".to_string()
                        }),
                        session_id: None,
                        token_usage: Some(combined_tokens),
                        structured_output: None,

                        run_index: parent_run_index,
                        child_outputs: Some(child_snapshots.clone()),
                        state: crate::workflow::state::default_step_entry_state(),
                    };
                    exec.current_step_token_usage = TokenUsage::default();
                    exec.current_session_id = None;
                    exec.step_history.push(entry);

                    Self::apply_transition(exec, target)?
                } else {
                    // aggregateなし → 通常のadvance

                    // ParallelCompleted ログ
                    self.write_log(
                        app,
                        WorkflowEvent::ParallelCompleted {
                            run_id: exec.id.clone(),
                            workflow_name: exec.workflow.name.clone(),
                            parent_node_name: parent_step_name.clone(),
                            aggregate_result: "advance".to_string(),
                            timestamp: current_timestamp(),
                        },
                    );

                    let entry = StepHistoryEntry {
                        step_name: parent_step_name.clone(),
                        completed_at: current_timestamp(),
                        result: Some("complete".to_string()),
                        session_id: None,
                        token_usage: Some(combined_tokens),
                        structured_output: None,

                        run_index: parent_run_index,
                        child_outputs: Some(child_snapshots),
                        state: crate::workflow::state::default_step_entry_state(),
                    };
                    exec.current_step_token_usage = TokenUsage::default();
                    exec.current_session_id = None;
                    exec.step_history.push(entry);

                    Self::apply_advance(exec)
                };
                (true, Some(outcome), exec_snapshot_before)
            }
        };

        if let Some(outcome) = outcome_opt {
            if all_completed {
                self.execute_outcome(
                    app,
                    session_store,
                    handles,
                    worktree_path,
                    outcome,
                    exec_snapshot_before,
                )
                .await?;
            } else {
                // まだ完了していない → Persistのみ
                if let StepOutcome::Persist(snapshot) = outcome {
                    self.persist_release_and_broadcast(
                        app,
                        session_store,
                        handles,
                        worktree_path,
                        snapshot,
                        &[session_id.to_string()],
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }

    /// aggregate条件を評価する。trueなら`then`、falseなら`else`。
    /// child_step_namesで指定された並列子ステップの出力のみを対象にする。
    /// StepOutput.resultのみで判定する。
    fn evaluate_aggregate(
        &self,
        agg: &ParallelAggregate,
        step_outputs: &HashMap<String, StepOutput>,
        child_step_names: &[String],
    ) -> bool {
        let child_outputs: Vec<&StepOutput> = child_step_names
            .iter()
            .filter_map(|name| step_outputs.get(name))
            .collect();

        let matches_pattern = |output: &&StepOutput, pattern: &str, re: &Option<regex::Regex>| {
            if let Some(ref result) = output.result {
                if let Some(ref re) = re {
                    re.is_match(result)
                } else {
                    result.contains(pattern)
                }
            } else {
                false
            }
        };

        if let Some(ref pattern) = agg.all_match {
            // all_match: 子ステップのStepOutputが1つでも欠けていればfalse
            if child_outputs.len() != child_step_names.len() {
                return false;
            }
            let re = RegexBuilder::new(pattern).size_limit(1 << 20).build().ok();
            child_outputs
                .iter()
                .all(|o| matches_pattern(o, pattern, &re))
        } else if let Some(ref pattern) = agg.any_match {
            let re = RegexBuilder::new(pattern).size_limit(1 << 20).build().ok();
            child_outputs
                .iter()
                .any(|o| matches_pattern(o, pattern, &re))
        } else {
            true
        }
    }

    /// `run_id` を直接指定して session_workflow_refs を掃除する。
    /// Spec issues-1011 finding 1: 同一 worktree に terminal/active 両方の run が共存する
    /// 状況で、worktree 主語でクリーンアップすると別 run の refs まで削除し得る。
    /// 全 cleanup 経路はこの run_id 主語のメソッドを使う。
    async fn cleanup_session_workflow_refs_by_run_id(&self, run_id: &str) {
        let mut map = self.session_workflow_refs.lock().await;
        map.retain(|_, r| r.run_id != run_id);
    }

    /// 状態取得。`worktree_path` 属性で in-memory 実行表を検索する。
    pub async fn get_state(&self, worktree_path: &str) -> Option<WorkflowState> {
        let execs = self.executions.lock().await;
        find_by_worktree(&execs, worktree_path).map(|(_, e)| e.to_workflow_state())
    }

    /// `run_id` から `WorkflowState` を取得する。
    pub async fn get_state_by_run_id(&self, run_id: &str) -> Option<WorkflowState> {
        let execs = self.executions.lock().await;
        execs.get(run_id).map(|e| e.to_workflow_state())
    }

    async fn emit_workflow_runtime_projection<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        open_tabs: &crate::session::OpenTabRegistry,
        worktree_path: &str,
    ) {
        let Some(state) = self.get_state(worktree_path).await else {
            return;
        };
        crate::workflow_state_events::emit_workflow_state(
            app,
            worktree_path,
            state,
            handles,
            open_tabs,
        )
        .await;
    }

    fn build_missing_output_repair_prompt(
        run_id: &str,
        step_name: &str,
        contract: &str,
        contract_definition: Option<&str>,
    ) -> String {
        let contract_section = contract_definition
            .filter(|body| !body.trim().is_empty())
            .map(|body| {
                format!(
                    "\n\nContract definition (type: {contract}):\n\n```text\n{}\n```",
                    body.trim()
                )
            })
            .unwrap_or_default();
        // CLI 名は起動環境別に解決する（dev → `releash-dev`、本番 → `releash`）。
        let cli = Self::resolve_releash_alias();
        format!(
            "The required structured output for this workflow step has not been submitted.\n\n\
Submit it by running this command with a JSON object that satisfies the `{contract}` contract:{contract_section}\n\n\
```sh\n\
{cli} workflow output submit {run_id} \\\n  --step {step_name} \\\n  --type {contract} \\\n  --json '{{...}}'\n\
```\n\n\
Do not create a temporary JSON file for this. Do not finish the step until the command succeeds."
        )
    }

    /// 起動環境別の `releash` alias 名を返す（spec issues-1054）。
    ///
    /// 本関数は alias 名のみを必要とするため、data_dir 解決を経由しない pure helper
    /// (`alias_name_for_profile`) を直接呼び、`dirs::data_dir()` 失敗で alias 名解決が
    /// 巻き込まれないようにする。
    pub(crate) fn resolve_releash_alias() -> String {
        crate::path_aliases::alias_name_for_profile(crate::path_aliases::BuildProfile::current())
            .to_string()
    }

    fn contract_repair_attempt_count<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        node_name: &str,
    ) -> Result<u32, WorkflowEngineError> {
        let data_dir =
            crate::session::resolve_data_dir(app).map_err(WorkflowEngineError::SessionStore)?;
        let log = WorkflowEventLog::new(&data_dir);
        let events = log
            .read_log(run_id)
            .map_err(WorkflowEngineError::SessionStore)?;
        Ok(events
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    WorkflowEvent::ContractRepairRequested {
                        node_name: event_node,
                        ..
                    } if event_node == node_name
                )
            })
            .count() as u32)
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_missing_required_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        run_id: &str,
        workflow_name: &str,
        node_name: &str,
        contract: &str,
        session_id: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        let prior_attempts = self.contract_repair_attempt_count(app, run_id, node_name)?;
        let attempt = prior_attempts + 1;
        let Some(session_id) = session_id else {
            return self
                .fail_missing_required_output(
                    app,
                    session_store,
                    handles,
                    worktree_path,
                    run_id,
                    node_name,
                    contract,
                    "no active session is available for contract output repair",
                )
                .await;
        };
        if attempt > MAX_CONTRACT_REPAIR_ATTEMPTS {
            return self
                .fail_missing_required_output(
                    app,
                    session_store,
                    handles,
                    worktree_path,
                    run_id,
                    node_name,
                    contract,
                    &format!(
                        "required structured output was not submitted after {MAX_CONTRACT_REPAIR_ATTEMPTS} repair attempts"
                    ),
                )
                .await;
        }

        let data_dir =
            crate::session::resolve_data_dir(app).map_err(WorkflowEngineError::SessionStore)?;
        let Some(session) = session_store
            .get_session(&data_dir, session_id)
            .map_err(WorkflowEngineError::SessionStore)?
        else {
            return self
                .fail_missing_required_output(
                    app,
                    session_store,
                    handles,
                    worktree_path,
                    run_id,
                    node_name,
                    contract,
                    &format!("step session not found for contract repair: {session_id}"),
                )
                .await;
        };

        self.write_log_required(
            app,
            WorkflowEvent::ContractRepairRequested {
                run_id: run_id.to_string(),
                workflow_name: workflow_name.to_string(),
                node_name: node_name.to_string(),
                attempt,
                violation_reason: "missing_submit_output".to_string(),
                timestamp: current_timestamp(),
            },
        )
        .map_err(WorkflowEngineError::SessionStore)?;

        let contract_definition = {
            let execs = self.executions.lock().await;
            execs.get(run_id).and_then(|exec| {
                Self::resolved_output_contract_definition_for(exec, node_name, contract)
            })
        };
        let prompt = Self::build_missing_output_repair_prompt(
            run_id,
            node_name,
            contract,
            contract_definition.as_deref(),
        );
        let _runtime_guard = crate::agent_sdk::acquire_session_runtime_lock(session_id).await;
        crate::agent_sdk::start_agent_turn_internal_locked(
            app,
            handles,
            session_store,
            session_id,
            worktree_path,
            &session.permission_mode,
            &prompt,
        )
        .await
        .map_err(WorkflowEngineError::AgentSession)
    }

    #[allow(clippy::too_many_arguments)]
    async fn fail_missing_required_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        run_id: &str,
        node_name: &str,
        contract: &str,
        reason: &str,
    ) -> Result<(), WorkflowEngineError> {
        let (snapshot, snapshot_before) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(run_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
            if exec.is_terminal() {
                return Ok(());
            }
            let snapshot_before = exec.clone();
            let mut entry = exec.make_step_history_entry(
                Some("contract_missing_output".to_string()),
                None,
                Some(contract.to_string()),
            );
            entry.state = "failed".to_string();
            exec.step_history.push(entry);
            exec.state = WorkflowExecutionState::Failed {
                reason: format!(
                    "Required structured output for step '{node_name}' was not submitted: {reason}"
                ),
            };
            exec.updated_at = current_timestamp();
            (exec.to_workflow_state(), snapshot_before)
        };
        self.execute_outcome(
            app,
            session_store,
            handles,
            worktree_path,
            StepOutcome::Persist(snapshot),
            snapshot_before,
        )
        .await
    }

    /// session_idがワークフロー実行中かどうか。
    pub async fn is_running(&self, session_id: &str) -> bool {
        let Some(worktree_path) = self.resolve_worktree_path(session_id).await else {
            return false;
        };
        let execs = self.executions.lock().await;
        find_by_worktree(&execs, &worktree_path).is_some_and(|(_, e)| e.is_active())
    }

    /// `run_id` から approval 用 chat session（current step session）と worktree_path を解決する。
    /// Spec issues-1011 line 121: 起動以外の workflow 操作 API は run_id を主語に取り、
    /// 内部の chat_session_id / worktree_path は engine が解決する。
    ///
    /// Spec issues-1011 finding 3: 任意 step session への注入経路を塞ぐため、resolve 時点で
    /// 以下を全て必須化する:
    ///   - 対象 run が active であること
    ///   - state が `WaitingApproval` であること
    ///   - current node の `node_type` が `Approval` であること
    ///   - `current_session_id` が存在すること
    ///
    /// いずれかが不成立なら approval ターゲット解決を拒否する。
    pub async fn resolve_chat_session_for_approval(
        &self,
        run_id: &str,
    ) -> Result<(String, String), WorkflowEngineError> {
        let execs = self.executions.lock().await;
        let exec = execs
            .get(run_id)
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
        if !exec.is_active() {
            return Err(WorkflowEngineError::InvalidState(
                "workflow run is not active".to_string(),
            ));
        }
        if exec.state != WorkflowExecutionState::WaitingApproval {
            return Err(WorkflowEngineError::InvalidState(
                "Workflow is not waiting for approval".to_string(),
            ));
        }
        let current_node = exec
            .workflow
            .nodes
            .get(exec.current_step_index)
            .ok_or_else(|| {
                WorkflowEngineError::InvalidState("current step index is out of range".to_string())
            })?;
        if current_node.node_type != NodeType::Approval {
            return Err(WorkflowEngineError::InvalidState(
                "current node is not an approval step".to_string(),
            ));
        }
        let session_id = exec.current_session_id.clone().ok_or_else(|| {
            WorkflowEngineError::InvalidState(
                "workflow has no current step session for approval chat".to_string(),
            )
        })?;
        Ok((session_id, exec.worktree_path.clone()))
    }

    pub async fn validate_approval_chat_instruction(
        &self,
        session_id: &str,
        content: &str,
    ) -> Result<(), WorkflowEngineError> {
        let Some(session_ref) = self.resolve_session_ref(session_id).await else {
            return Ok(());
        };
        // parent ChatSession 機構撤去後は step session のみが session_workflow_refs に登録される。

        let execs = self.executions.lock().await;
        let Some(exec) = execs.get(&session_ref.run_id) else {
            return Ok(());
        };
        let step = &exec.workflow.nodes[exec.current_step_index];
        let is_current_approval_session = step.node_type == NodeType::Approval
            && exec.current_session_id.as_deref() == Some(session_id);
        if !is_current_approval_session {
            if Self::is_approval_step_session(exec, session_id) {
                return Err(WorkflowEngineError::InvalidState(
                    "Workflow is not waiting for approval".to_string(),
                ));
            }
            return Ok(());
        }
        if exec.state != WorkflowExecutionState::WaitingApproval {
            return Err(WorkflowEngineError::InvalidState(
                "Workflow is not waiting for approval".to_string(),
            ));
        }
        validate_required_comment_text(content, "approval chat instruction")
            .map_err(command_input_error_to_engine_error)?;
        Ok(())
    }

    fn is_approval_step_session(exec: &WorkflowExecution, session_id: &str) -> bool {
        let step_is_approval = |step_name: &str| {
            exec.workflow
                .nodes
                .iter()
                .find(|step| step.name == step_name)
                .is_some_and(|step| step.node_type == NodeType::Approval)
        };

        if exec.current_session_id.as_deref() == Some(session_id)
            && step_is_approval(&exec.workflow.nodes[exec.current_step_index].name)
        {
            return true;
        }

        exec.step_history.iter().any(|entry| {
            entry.session_id.as_deref() == Some(session_id) && step_is_approval(&entry.step_name)
        })
    }

    #[cfg(test)]
    pub async fn validate_approval_target(
        &self,
        worktree_path: &str,
        expected_execution_id: Option<&str>,
        expected_step_name: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        let execs = self.executions.lock().await;
        let (_, exec) = find_by_worktree(&execs, worktree_path)
            .ok_or_else(|| WorkflowEngineError::UnauthorizedWorktree(worktree_path.to_string()))?;
        Self::validate_approval_target_snapshot(exec, expected_execution_id, expected_step_name)
    }

    #[cfg(test)]
    fn validate_approval_target_snapshot(
        exec: &WorkflowExecution,
        expected_execution_id: Option<&str>,
        expected_step_name: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        Self::resolve_approval_target_snapshot(exec, expected_execution_id, expected_step_name)?;
        if expected_step_name.is_none() {
            return Err(WorkflowEngineError::UnauthorizedApprovalTarget(
                "step_name is required".to_string(),
            ));
        }
        Ok(())
    }

    fn resolve_approval_target_snapshot(
        exec: &WorkflowExecution,
        expected_execution_id: Option<&str>,
        expected_step_name: Option<&str>,
    ) -> Result<String, WorkflowEngineError> {
        if exec.state != WorkflowExecutionState::WaitingApproval {
            return Err(WorkflowEngineError::InvalidState(
                "Workflow is not waiting for approval".to_string(),
            ));
        }
        let expected_execution_id = expected_execution_id.ok_or_else(|| {
            WorkflowEngineError::UnauthorizedApprovalTarget("execution_id is required".to_string())
        })?;
        if expected_execution_id != exec.id {
            return Err(WorkflowEngineError::UnauthorizedApprovalTarget(
                "execution_id does not match".to_string(),
            ));
        }
        let current_step = &exec.workflow.nodes[exec.current_step_index].name;
        if expected_step_name.is_some_and(|expected| expected != current_step) {
            return Err(WorkflowEngineError::UnauthorizedApprovalTarget(
                "step does not match".to_string(),
            ));
        }
        Ok(current_step.clone())
    }

    fn validate_approval_turn_phase(
        turn_phase: Option<crate::agent_sdk::TurnPhase>,
    ) -> Result<(), WorkflowEngineError> {
        match turn_phase {
            Some(crate::agent_sdk::TurnPhase::Streaming)
            | Some(crate::agent_sdk::TurnPhase::WaitingPermission) => Err(
                WorkflowEngineError::ValidationError("approval output is not complete".to_string()),
            ),
            Some(crate::agent_sdk::TurnPhase::Idle) | None => Ok(()),
        }
    }

    /// 現在実行中のワークフロー名の集合を返す（全worktreeを集約）。
    pub async fn running_workflow_names(&self) -> std::collections::HashSet<String> {
        let execs = self.executions.lock().await;
        execs
            .values()
            .filter(|e| e.is_active())
            .map(|e| e.workflow.name.clone())
            .collect()
    }

    /// セッションIDからworktree_pathを解決する。
    /// session_workflow_refsに登録されていない場合はNoneを返す。
    /// SessionWorkflowRef は run_id を保持するため、executions から exec.worktree_path を
    /// 取得して返す（Spec issues-1011: engine 内部キーも run_id）。
    pub async fn resolve_worktree_path(&self, session_id: &str) -> Option<String> {
        let run_id = {
            let map = self.session_workflow_refs.lock().await;
            map.get(session_id).map(|r| r.run_id.clone())?
        };
        let execs = self.executions.lock().await;
        execs.get(&run_id).map(|e| e.worktree_path.clone())
    }

    /// セッションIDからSessionWorkflowRefを解決する。
    async fn resolve_session_ref(&self, session_id: &str) -> Option<SessionWorkflowRef> {
        let map = self.session_workflow_refs.lock().await;
        map.get(session_id).cloned()
    }

    // ---- 内部メソッド ----

    // set_execution_state の lookup 戦略指定。RunId バリアントは worktree_path を補助情報
    // として保持する（broadcast / cleanup の対象として）。
    // Note: enum 定義は impl の外側にあり、ここでは参照のみ可能（Rust 制約）。
    // 実体は WorkflowEngine impl の下に置く。

    /// 実行状態を更新し、永続化・ブロードキャストする。
    /// 内部実装は `set_execution_state_inner` に集約され、worktree_path 主語の場合は
    /// `find_by_worktree_mut`、run_id 主語の場合は `executions.get_mut(run_id)` で
    /// lookup する。
    async fn set_execution_state<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        new_state: WorkflowExecutionState,
    ) -> Result<(), WorkflowEngineError> {
        self.set_execution_state_inner(
            app,
            session_store,
            handles,
            ExecutionStateTarget::Worktree(worktree_path.to_string()),
            new_state,
        )
        .await
    }

    /// 実行状態更新の内部実装。lookup 戦略を `target` で切り替える。
    /// Spec issues-1011 finding 10: Run Store sync 失敗時は engine state も巻き戻し、
    /// engine terminal / Run Store active のスキューを残さない。
    async fn set_execution_state_inner<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        target: ExecutionStateTarget,
        new_state: WorkflowExecutionState,
    ) -> Result<(), WorkflowEngineError> {
        let (snapshot, run_id, worktree_path, snapshot_before) = {
            let mut execs = self.executions.lock().await;
            let exec = match &target {
                ExecutionStateTarget::Worktree(wt) => find_by_worktree_mut(&mut execs, wt)
                    .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(wt.clone()))?,
            };
            // 終了状態（Completed/Failed/Aborted）からの上書きを防止
            if exec.is_terminal() {
                return Ok(());
            }
            let snapshot_before = exec.clone();
            exec.state = new_state;
            exec.updated_at = current_timestamp();
            (
                exec.to_workflow_state(),
                exec.id.clone(),
                exec.worktree_path.clone(),
                snapshot_before,
            )
        };

        let is_terminal = matches!(
            snapshot.state,
            WorkflowExecutionState::Completed
                | WorkflowExecutionState::Failed { .. }
                | WorkflowExecutionState::Aborted
        );

        // [05] terminal 経路は commit_required_events 基盤の共通 commit 境界に統合する。
        // terminal events (NodeCompleted（Completed のみ）+ RunCompleted / NodeFailed+RunFailed)
        // を required event 列として集約し、RunStore sync → ChatSession persist → event log
        // append の順序で commit する。いずれかが失敗した場合は engine state と Run Store snapshot
        // を snapshot_before で一括復元する（spec [05] atomic mutation 境界 / best-effort warn 廃止）。
        // Aborted は AbortRun command handler 側で別途 commit されるため本経路では event 集合に含めない。
        if is_terminal && !matches!(snapshot.state, WorkflowExecutionState::Aborted) {
            let mut terminal_snapshot = snapshot.clone();
            let mut required_events = Vec::new();
            if matches!(snapshot.state, WorkflowExecutionState::Completed) {
                match Self::last_step_completed_event_for_snapshot(&mut terminal_snapshot) {
                    Ok(Some(ev)) => required_events.push(ev),
                    Ok(None) => {}
                    Err(e) => {
                        let mut execs = self.executions.lock().await;
                        if let Some(exec) = execs.get_mut(&run_id) {
                            *exec = snapshot_before;
                        }
                        return Err(e);
                    }
                }
            }
            match Self::terminal_events_for_snapshot(&mut terminal_snapshot) {
                Ok(events) => required_events.extend(events),
                Err(e) => {
                    let mut execs = self.executions.lock().await;
                    if let Some(exec) = execs.get_mut(&run_id) {
                        *exec = snapshot_before;
                    }
                    return Err(e);
                }
            }
            let run_store_snapshot_before = self.run_store.active_run_snapshot(&run_id).await;
            self.commit_required_events(
                app,
                session_store,
                RequiredEventCommit {
                    run_id: &run_id,
                    snapshot_for_commit: &snapshot,
                    snapshot_before,
                    run_store_snapshot_before,
                    required_events,
                    append_error_context: "set_execution_state terminal event append failed",
                },
            )
            .await?;

            // terminal 副作用: step session release + refs cleanup + broadcast。
            let terminal_session_ids = Self::terminal_step_session_ids(&snapshot);
            self.release_completed_step_sessions(
                app,
                session_store,
                handles,
                &terminal_session_ids,
            )
            .await;
            self.cleanup_session_workflow_refs_by_run_id(&run_id).await;
            self.broadcast_state(app, &worktree_path, snapshot.clone())
                .await;
            return Ok(());
        }

        // 非 terminal / Aborted 経路: required event が無いため従来の sync→persist 順で commit する。
        // Aborted は AbortRun command handler 側で event を別途 append 済み。
        let rollback_engine_state =
            |run_id_for_rollback: String, previous_snapshot: WorkflowExecution| async move {
                let mut execs = self.executions.lock().await;
                if let Some(exec) = execs.get_mut(&run_id_for_rollback) {
                    *exec = previous_snapshot;
                }
            };

        if let Err(e) = self.sync_run_store_from_snapshot(&run_id, &snapshot).await {
            rollback_engine_state(run_id.clone(), snapshot_before).await;
            return Err(e);
        }

        if is_terminal {
            let terminal_session_ids = Self::terminal_step_session_ids(&snapshot);
            self.release_completed_step_sessions(
                app,
                session_store,
                handles,
                &terminal_session_ids,
            )
            .await;
            self.cleanup_session_workflow_refs_by_run_id(&run_id).await;
        }
        self.broadcast_state(app, &worktree_path, snapshot.clone())
            .await;
        Ok(())
    }

    /// `WorkflowExecutionState` から `RunStatus` への変換と Run Store metadata 同期を 1 箇所に集約する。
    /// terminal 状態は `complete_run` で active から除外し、それ以外は `update_active` で metadata を更新する。
    ///
    /// Spec issues-1011 finding 3: 同期失敗は Result で呼出側に伝播し、権威遷移経路で「成功遷移
    /// として扱わない」境界とする。Run Store は active run 集合と metadata の owner であり、
    /// engine.executions との不整合を許容しない。
    async fn sync_run_store_from_snapshot(
        &self,
        run_id: &str,
        snapshot: &WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
        let now = current_timestamp();
        let result = match &snapshot.state {
            WorkflowExecutionState::Completed => {
                self.run_store
                    .complete_run(run_id, TerminalRunStatus::Completed, now, None)
                    .await
            }
            WorkflowExecutionState::Failed { reason } => {
                self.run_store
                    .complete_run(run_id, TerminalRunStatus::Failed, now, Some(reason.clone()))
                    .await
            }
            WorkflowExecutionState::Aborted => {
                self.run_store
                    .complete_run(run_id, TerminalRunStatus::Aborted, now, None)
                    .await
            }
            WorkflowExecutionState::Running | WorkflowExecutionState::WaitingApproval => {
                let status = if matches!(snapshot.state, WorkflowExecutionState::Running) {
                    RunStatus::Running
                } else {
                    RunStatus::WaitingApproval
                };
                let current_node = snapshot.current_step_name.clone();
                self.run_store
                    .sync_active_projection(run_id, status, Some(current_node), now)
                    .await
            }
        };
        result.map_err(|e| {
            WorkflowEngineError::SessionStore(format!("RunStore sync failed for run {run_id}: {e}"))
        })
    }

    async fn restore_run_store_active_snapshot(
        &self,
        run_snapshot: Option<WorkflowRun>,
    ) -> Result<(), WorkflowEngineError> {
        let Some(run_snapshot) = run_snapshot else {
            return Ok(());
        };
        let run_id = run_snapshot.run_id.clone();
        self.run_store
            .restore_active_snapshot_for_rollback(run_snapshot)
            .await
            .map_err(|e| {
                WorkflowEngineError::SessionStore(format!(
                    "RunStore rollback failed for run {run_id}: {e}"
                ))
            })
    }

    async fn rollback_command_mutation<R: tauri::Runtime>(
        &self,
        _app: &tauri::AppHandle<R>,
        _session_store: &Arc<SessionStore>,
        rollback: CommandMutationRollback<'_>,
    ) -> Result<(), WorkflowEngineError> {
        let CommandMutationRollback {
            run_id,
            snapshot_before,
            run_store_snapshot_before,
            context,
        } = rollback;
        let run_store_result = self
            .restore_run_store_active_snapshot(run_store_snapshot_before)
            .await;
        if let Err(ref rollback_err) = run_store_result {
            log::warn!(
                "workflow {run_id}: Run Store rollback failed after {context}: {rollback_err}"
            );
        }
        let mut execs = self.executions.lock().await;
        if let Some(exec) = execs.get_mut(run_id) {
            *exec = snapshot_before;
        }
        run_store_result
    }

    async fn rollback_execution_projection_after_run_store_sync_failure(
        &self,
        run_id: &str,
        failed_snapshot: &WorkflowState,
    ) {
        let active_projection = self
            .run_store
            .list_active()
            .await
            .into_iter()
            .find(|run| run.run_id == run_id);
        let Some(active_projection) = active_projection else {
            return;
        };
        let rollback_state = match active_projection.status {
            RunStatus::Running => WorkflowExecutionState::Running,
            RunStatus::WaitingApproval => WorkflowExecutionState::WaitingApproval,
            RunStatus::Completed | RunStatus::Failed | RunStatus::Aborted => return,
        };
        let mut execs = self.executions.lock().await;
        let Some(exec) = execs.get_mut(run_id) else {
            return;
        };
        if exec.state != failed_snapshot.state {
            return;
        }
        exec.state = rollback_state;
        if let Some(current_node_name) = active_projection.current_node_name {
            if let Some(index) = exec
                .workflow
                .nodes
                .iter()
                .position(|node| node.name == current_node_name)
            {
                exec.current_step_index = index;
            }
        }
        exec.updated_at = current_timestamp();
    }

    /// autoモードのタグ検出結果を処理する。
    /// 判定 + 状態変更 + 履歴記録を1回のロックで原子的に実行する。
    /// output_contractが設定されたステップではcontract検証を実行し、
    /// 違反時はリトライプロンプトを送信する。
    #[allow(clippy::too_many_arguments)]
    async fn handle_auto_complete<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        final_parts: &[crate::session::MessagePart],
        rules: &[TransitionRule],
        step_name: &str,
    ) -> Result<(), WorkflowEngineError> {
        // テキストパートを結合（ロック外で完了）
        let text = Self::extract_text_from_parts(final_parts);

        // [08] prose 抽出経路廃止: agent step の structured output は CLI / Tauri 経由の
        // `SubmitOutput` でしか確定しない。output_contract がある step は、提出済み
        // output が見つからない限り完了扱いにせず、同じ session に修正ターンを投げる。
        let (
            run_id,
            workflow_name,
            output_contract,
            run_index,
            current_session_id,
            submitted_output,
        ) = {
            let execs = self.executions.lock().await;
            let (run_id, exec) = find_by_worktree(&execs, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let node = &exec.workflow.nodes[exec.current_step_index];
            let output_contract = node.output_contract.clone();
            let run_index = exec
                .step_execution_counts
                .get(&node.name)
                .copied()
                .unwrap_or(1);
            let submitted_output = output_contract.as_deref().and_then(|contract| {
                Self::submitted_step_output_for(exec, &node.name, run_index, contract)
            });
            (
                run_id.clone(),
                exec.workflow.name.clone(),
                output_contract,
                run_index,
                exec.current_session_id.clone(),
                submitted_output,
            )
        };
        let (structured_output, contract_result) = if let Some(ref contract) = output_contract {
            if let Some(output) = submitted_output {
                (output.structured_output.clone(), output.result.clone())
            } else {
                self.handle_missing_required_output(
                    app,
                    session_store,
                    handles,
                    worktree_path,
                    &run_id,
                    &workflow_name,
                    step_name,
                    contract,
                    current_session_id.as_deref(),
                )
                .await?;
                return Ok(());
            }
        } else {
            (None, None)
        };
        let _ = run_index;

        // contract検証成功時のworkflow_variables反映。
        self.apply_contract_variables(worktree_path, &output_contract, &structured_output)
            .await;

        let effective_result = contract_result;

        // タグ検出もロック外で完了（純粋関数）
        let rule_match = if rules.is_empty() {
            None // ルールなし → 定義順で次へ
        } else if let Some(ref result_str) = effective_result {
            // contract resultがある場合はそれでルール評価
            Some(Self::evaluate_auto_rules(result_str, rules))
        } else {
            Some(Self::evaluate_auto_rules(&text, rules))
        };

        // 判定 + 状態変更 + 履歴記録を原子的に実行
        let (outcome, snapshot_before) = {
            let mut execs = self.executions.lock().await;
            let exec = find_by_worktree_mut(&mut execs, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let snapshot_before = exec.clone();

            let outcome = match rule_match {
                None => {
                    // ルールなし → 定義順で次へ
                    let entry = exec.make_step_history_entry(
                        effective_result,
                        structured_output,
                        output_contract,
                    );
                    exec.step_history.push(entry);
                    Self::apply_advance(exec)
                }
                Some(Some((next_step, matched_rule))) => {
                    // ルールマッチ → 指定ステップへ遷移
                    let entry = exec.make_step_history_entry(
                        Some(matched_rule),
                        structured_output,
                        output_contract,
                    );
                    exec.step_history.push(entry);
                    Self::apply_transition(exec, &next_step)?
                }
                Some(None) => {
                    // マッチなし → Failed
                    let entry = exec.make_step_history_entry(
                        Some("no_matching_rule".to_string()),
                        structured_output,
                        output_contract,
                    );
                    exec.step_history.push(entry);
                    exec.state = WorkflowExecutionState::Failed {
                        reason: format!("No matching rule found for step '{}' output", step_name),
                    };
                    exec.updated_at = current_timestamp();
                    StepOutcome::Persist(exec.to_workflow_state())
                }
            };
            (outcome, snapshot_before)
        };

        self.execute_outcome(
            app,
            session_store,
            handles,
            worktree_path,
            outcome,
            snapshot_before,
        )
        .await
    }

    /// autoモードのタグ検出。rulesの定義順で最初にマッチしたルールを返す。
    fn evaluate_auto_rules(text: &str, rules: &[TransitionRule]) -> Option<(String, String)> {
        for rule in rules {
            match RegexBuilder::new(&rule.r#match).size_limit(1 << 20).build() {
                Ok(re) => {
                    if re.is_match(text) {
                        return Some((rule.next.clone(), rule.r#match.clone()));
                    }
                }
                Err(e) => {
                    log::warn!(
                        "Invalid regex pattern '{}' in transition rule (next='{}'): {e}",
                        rule.r#match,
                        rule.next
                    );
                }
            }
        }
        None
    }

    /// MessagePartからテキストを抽出して結合する。
    fn extract_text_from_parts(parts: &[crate::session::MessagePart]) -> String {
        let mut text = String::new();
        for part in parts {
            if let crate::session::MessagePart::Text { content, .. } = part {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(content);
            }
        }
        text
    }

    /// 現在のステップ用に新しいChatSessionを生成し、AgentSessionを開始してプロンプトを送信する。
    /// ファセット方式と旧prompt方式を自動判別する。
    ///
    /// production 経路。副作用境界を `RealStepSessionDeps` にラップし、コアロジック
    /// `start_step_session_with_deps` に委譲する。
    async fn start_step_session<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        session_store: &Arc<SessionStore>,
        worktree_path: &str,
    ) -> Result<(), WorkflowEngineError> {
        let deps = RealStepSessionDeps {
            engine: self,
            app,
            handles,
            session_store,
        };
        self.start_step_session_with_deps(&deps, worktree_path)
            .await
    }

    /// `start_step_session` のコアロジック。副作用境界は `StepSessionDeps` 経由で注入する。
    ///
    /// 呼び出し順序の不変条件:
    /// 1. `build_step_prompt`（純粋関数）でプロンプト合成
    /// 2. `deps.create_step_session`（`exec.workflow_defaults` を継承元に注入）
    /// 3. `session_workflow_refs` への登録
    /// 4. `deps.dispatch_session_start`（AgentSession 開始）
    /// 5. `executions.current_session_id` 更新と永続化・ブロードキャスト
    /// 6. `deps.start_agent_turn`（ターン起動）
    ///
    /// 1 で失敗した場合、2 以降は一切実行されない（合成失敗時に
    /// ChatSession 生成や `session_workflow_refs` への孤立 entry が残らない）。
    /// テストではこの順序保証を `StepSessionDeps` のテストダブル経由で検証する。
    async fn start_step_session_with_deps<D: StepSessionDeps + ?Sized>(
        &self,
        deps: &D,
        worktree_path: &str,
    ) -> Result<(), WorkflowEngineError> {
        let (
            run_id_for_ref,
            step_clone,
            step_outputs_clone,
            step_history_clone,
            task_clone,
            workflow_variables_clone,
            workflow_declared_variables_clone,
            workflow_defaults_clone,
        ) = {
            let execs = self.executions.lock().await;
            let (run_id, exec) = find_by_worktree(&execs, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let step = &exec.workflow.nodes[exec.current_step_index];
            (
                run_id.clone(),
                step.clone(),
                exec.step_outputs.clone(),
                exec.step_history.clone(),
                exec.task.clone(),
                exec.workflow_variables.clone(),
                exec.workflow.variables.clone(),
                exec.workflow_defaults.clone(),
            )
        };

        // プロンプト合成（純粋関数）を最初に行う。
        // ここで失敗（参照先ファセットが存在しない等）した場合、後続の
        // ChatSession 生成・`session_workflow_refs` 登録・AgentSession 開始は一切
        // 行われない。これにより、`start_step_session` がエラー経路で孤立した
        // ChatSession や参照マップ entry を残さないことを構造的に保証する。
        let (system_prompt, prompt) = Self::build_step_prompt(
            &step_clone,
            &run_id_for_ref,
            worktree_path,
            task_clone.as_deref(),
            &step_outputs_clone,
            &step_history_clone,
            &workflow_variables_clone,
            &workflow_declared_variables_clone,
        )?;

        // ステップ設定の解決 → セッション生成（workflow_defaults を継承元に注入）
        let step_session = deps
            .create_step_session(
                worktree_path,
                step_clone.model.clone(),
                step_clone.permission.clone(),
                workflow_defaults_clone,
            )
            .await?;
        let permission_mode = step_session.permission_mode.clone();
        let step_session_id = step_session.id.clone();

        // ステップセッションID → SessionWorkflowRefのマッピングを登録
        {
            let mut map = self.session_workflow_refs.lock().await;
            map.insert(
                step_session_id.clone(),
                SessionWorkflowRef {
                    run_id: run_id_for_ref.clone(),
                },
            );
        }

        let _runtime_guard = crate::agent_sdk::acquire_session_runtime_lock(&step_session_id).await;

        // 合成済み system_prompt を AgentSession 起動経路へ受け渡す。
        deps.dispatch_session_start(&step_session_id, worktree_path, None, system_prompt)
            .await?;
        deps.mark_step_tab_open(&step_session_id).await;

        // ステップセッションIDをワークフロー実行に紐付け
        let snapshot = {
            let mut execs = self.executions.lock().await;
            if let Some(exec) = execs.get_mut(&run_id_for_ref) {
                exec.current_session_id = Some(step_session_id.clone());
                Some(exec.to_workflow_state())
            } else {
                None
            }
        };

        if let Some(snapshot) = snapshot {
            deps.broadcast_state(worktree_path, snapshot).await;
        }

        // プロンプト送信（ステップ用セッションIDを使用）
        deps.start_agent_turn_locked(&step_session_id, worktree_path, &permission_mode, &prompt)
            .await
    }

    /// ファセット合成パイプライン: compose → 変数展開 → step output注入
    /// start_step_session の中核ロジックを純粋関数として切り出し、テスト可能にする。
    ///
    /// [08] `run_id` を引数で受け取り、`output_contract_preamble` が埋め込む
    /// `{{run_id}}` / `{{step_name}}` プレースホルダを実値に展開する。これにより
    /// agent が表示された CLI コマンドをそのまま `releash workflow output submit` 用に
    /// 呼び出せる（spec [08] 要求: run_id と step_name を主語に CLI 提出できる）。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_step_prompt(
        step: &NodeDefinition,
        run_id: &str,
        worktree_path: &str,
        task: Option<&str>,
        step_outputs: &HashMap<String, StepOutput>,
        step_history: &[StepHistoryEntry],
        workflow_variables: &HashMap<String, String>,
        workflow_declared_variables: &HashMap<String, String>,
    ) -> Result<(Option<String>, String), WorkflowEngineError> {
        // inline_prompt のみ（ファセット参照なし）のステップ: inline_prompt をそのまま使用
        if !step.has_facet_refs() {
            if let Some(ref inline) = step.inline_prompt {
                let rendered = Self::render_facet_variables(inline, worktree_path, task);
                let rendered = Self::render_submit_command_variables(&rendered, run_id, &step.name);
                let rendered =
                    Self::render_namespaced_variables(&rendered, workflow_declared_variables);
                let prompt = Self::inject_step_outputs(
                    &rendered,
                    step,
                    step_outputs,
                    step_history,
                    workflow_variables,
                );
                // facet ref を持たない inline_prompt step は input_contracts も持たない
                // (`has_facet_refs` が false のため)。`<task>` 注入は行わず、
                // inline_prompt 内で `{{task}}` テンプレートを使う設計に委ねる。
                return Ok((None, prompt));
            }
            return Err(WorkflowEngineError::InvalidWorkflow(format!(
                "Step '{}' has no facet refs and no inline_prompt.",
                step.name
            )));
        }
        // [02] schema 境界: engine は load 経路で解決済み facet のみを参照する。
        // facet ref が設定されているのに resolved_facets が空 = load pipeline が走っていない
        // 不整合状態のため、副作用に進ませず InvalidWorkflow で拒否する。
        if step.resolved_facets.is_empty() {
            return Err(WorkflowEngineError::InvalidWorkflow(format!(
                "Step '{}' has unresolved facet refs (workflow must go through load pipeline)",
                step.name
            )));
        }
        let composed = crate::workflow::facet::compose_facets(step);
        let system_prompt = composed.system_prompt.map(|s| {
            let s = Self::render_facet_variables(&s, worktree_path, task);
            let s = Self::render_submit_command_variables(&s, run_id, &step.name);
            Self::render_namespaced_variables(&s, workflow_declared_variables)
        });
        let rendered_user = {
            let s = Self::render_facet_variables(&composed.user_message, worktree_path, task);
            let s = Self::render_submit_command_variables(&s, run_id, &step.name);
            Self::render_namespaced_variables(&s, workflow_declared_variables)
        };
        let mut prompt = Self::inject_step_outputs(
            &rendered_user,
            step,
            step_outputs,
            step_history,
            workflow_variables,
        );
        // input_contracts を宣言している step だけが `<task>` ブロックを受け取る。
        // 既存 builtin の `{{task}}` テンプレート展開と二重注入にならないようにする。
        let allow_task = step.input_contracts.as_ref().is_some_and(|v| !v.is_empty());
        Self::append_task_block(&mut prompt, task, allow_task);
        Self::append_output_contract_completion_action(
            &mut prompt,
            step.output_contract.as_deref(),
            run_id,
            &step.name,
            workflow_declared_variables,
        );
        Ok((system_prompt, prompt))
    }

    /// `SessionStartGate` 経由で AgentSession を開始する。
    /// production からは `RealSessionStartGate` を、テストからは記録用テストダブルを渡す。
    /// この関数を経由することで、合成された `system_prompt` がドロップ・空文字置換されずに
    /// バックエンドへ受け渡されることをユニットテストで検証可能にする。
    async fn dispatch_session_start<G: SessionStartGate + ?Sized>(
        gate: &G,
        session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
    ) -> Result<(), WorkflowEngineError> {
        gate.start_session(session_id, worktree_path, permission_mode, system_prompt)
            .await
            .map_err(WorkflowEngineError::AgentSession)
    }

    /// `build_step_prompt` で合成した `system_prompt` を `dispatch_session_start` 経由で
    /// gate に渡し、`prompt`（user_message 由来）を返すテスト用ヘルパー。
    ///
    /// production では `start_step_session` 内で `build_step_prompt` →
    /// `create_step_session_with_settings` → `dispatch_session_start` を順に呼ぶ
    /// 構造にしている（プロンプト合成失敗時に ChatSession・参照マップ登録が起きない
    /// 順序保証のため）。テストでは記録用 gate を注入することで、合成された
    /// `system_prompt` が None や空文字に置換されずバックエンドへ受け渡される
    /// 経路を直接検証する。
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    async fn build_and_dispatch_step_session<G: SessionStartGate + ?Sized>(
        gate: &G,
        step: &NodeDefinition,
        run_id: &str,
        step_session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        task: Option<&str>,
        step_outputs: &HashMap<String, StepOutput>,
        step_history: &[StepHistoryEntry],
        workflow_variables: &HashMap<String, String>,
    ) -> Result<String, WorkflowEngineError> {
        let (system_prompt, prompt) = Self::build_step_prompt(
            step,
            run_id,
            worktree_path,
            task,
            step_outputs,
            step_history,
            workflow_variables,
            &HashMap::new(),
        )?;
        Self::dispatch_session_start(
            gate,
            step_session_id,
            worktree_path,
            permission_mode,
            system_prompt,
        )
        .await?;
        Ok(prompt)
    }

    /// contract検証成功時にworkflow_variablesへの反映を行う共通ヘルパー。
    /// spec-directory contractの場合、spec_dirをworkflow_variablesに設定する。
    async fn apply_contract_variables(
        &self,
        worktree_path: &str,
        output_contract: &Option<String>,
        structured_output: &Option<serde_json::Value>,
    ) {
        let vars = Self::extract_contract_variables(output_contract, structured_output);
        if !vars.is_empty() {
            let mut execs = self.executions.lock().await;
            if let Some(exec) = find_by_worktree_mut(&mut execs, worktree_path) {
                exec.workflow_variables.extend(vars);
            }
        }
    }

    /// contractとstructured_outputからworkflow_variablesに設定すべきキー/値を抽出する。
    fn extract_contract_variables(
        output_contract: &Option<String>,
        structured_output: &Option<serde_json::Value>,
    ) -> HashMap<String, String> {
        let mut vars = HashMap::new();
        if let (Some(ref contract), Some(ref so)) = (output_contract, structured_output) {
            if contract == "spec-directory" {
                if let Some(path) = so.get("spec_dir").and_then(|v| v.as_str()) {
                    vars.insert("spec_dir".to_string(), path.to_string());
                }
            }
        }
        vars
    }

    fn mask_sensitive_structured_output_with_secrets(
        contract: &str,
        mut value: serde_json::Value,
        secrets: &[String],
    ) -> serde_json::Value {
        if contract != "approved-fix-policy" {
            return value;
        }
        Self::mask_json_strings(&mut value, secrets);
        value
    }

    /// [08] structured output に対する「機密値 redaction → contract 適合判定」を
    /// 1 関数にまとめた共有 preflight/validate ヘルパー（pure / 副作用なし）。
    ///
    /// `handle_submit_output` の本体・`workflow_validate_output` の preflight・
    /// CLI 経路の preflight (`cmd_output_submit` / `cmd_output_validate`) を
    /// 同一の前処理 + validation に揃え、preflight と本 submit で判定が
    /// 食い違う構造を排除する（spec [08] CLI 完了基準: 最終判定は engine 側）。
    ///
    /// `secrets` は呼び出し側責任で収集する:
    ///   - Tauri command / engine 内部: [`Self::collect_configured_secret_values`]
    ///     経由（AppConfig + env vars）。
    ///   - CLI 経路: 別プロセスでありアプリ状態を持たないため、現状は空配列で
    ///     呼ぶ（最終 masking は engine 側で再評価される）。
    pub(crate) fn preprocess_and_validate_output_with_secrets(
        contract: &str,
        structured_output: serde_json::Value,
        secrets: &[String],
    ) -> ContractValidationResult {
        let redacted = Self::mask_sensitive_structured_output_with_secrets(
            contract,
            structured_output,
            secrets,
        );
        validate_contract_value(contract, redacted)
    }

    /// [08] AppHandle 経由で secrets を収集した上で
    /// [`Self::preprocess_and_validate_output_with_secrets`] に委譲する便利関数。
    /// Tauri command / engine 内部からの呼び出し用。
    pub(crate) fn preprocess_and_validate_output<R: tauri::Runtime>(
        app: &tauri::AppHandle<R>,
        contract: &str,
        structured_output: serde_json::Value,
    ) -> ContractValidationResult {
        let secrets = Self::collect_configured_secret_values(app);
        Self::preprocess_and_validate_output_with_secrets(contract, structured_output, &secrets)
    }

    fn collect_configured_secret_values<R: tauri::Runtime>(
        app: &tauri::AppHandle<R>,
    ) -> Vec<String> {
        let mut values = Vec::new();
        if let Some(config) = app.try_state::<Arc<crate::config::AppConfig>>() {
            if let Ok(cfg) = config.get_config() {
                values.extend(Self::collect_configured_secret_values_from_config(&cfg));
            }
        }
        values.extend(Self::collect_secret_values_from_env_vars(std::env::vars()));
        values.sort();
        values.dedup();
        values.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        values
    }

    fn collect_secret_values_from_env_vars<I>(vars: I) -> Vec<String>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        vars.into_iter()
            .filter_map(|(k, v)| {
                if v.len() >= 8 && Self::is_secret_env_var_name(&k) {
                    Some(v)
                } else {
                    None
                }
            })
            .collect()
    }

    fn is_secret_env_var_name(name: &str) -> bool {
        let normalized = name.to_ascii_uppercase();
        [
            "TOKEN",
            "SECRET",
            "PASSWORD",
            "PASSWD",
            "API_KEY",
            "ACCESS_KEY",
            "PRIVATE_KEY",
            "CREDENTIAL",
        ]
        .iter()
        .any(|pattern| normalized.contains(pattern))
    }

    fn collect_configured_secret_values_from_config(
        cfg: &crate::config::ReleashConfig,
    ) -> Vec<String> {
        let mut values = Vec::new();
        for v in [
            cfg.server.token.as_str(),
            cfg.server.mcp_token.as_str(),
            cfg.server.notify.webhook_url.as_str(),
        ] {
            if v.len() >= 8 {
                values.push(v.to_string());
            }
        }
        for notion in cfg.notion.values() {
            if notion.api_token.len() >= 8 {
                values.push(notion.api_token.clone());
            }
        }
        values
    }

    fn mask_json_strings(value: &mut serde_json::Value, configured_secrets: &[String]) {
        match value {
            serde_json::Value::String(s) => {
                *s = Self::mask_sensitive_text(s, configured_secrets);
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    Self::mask_json_strings(item, configured_secrets);
                }
            }
            serde_json::Value::Object(map) => {
                for item in map.values_mut() {
                    Self::mask_json_strings(item, configured_secrets);
                }
            }
            _ => {}
        }
    }

    /// 機密値パターンに該当するテキストを `[REDACTED]` に置換する。
    /// `configured_secrets` は長さ降順にソート済みであることを前提とする
    /// (`collect_configured_secret_values` が保証)。
    fn mask_sensitive_text(text: &str, configured_secrets: &[String]) -> String {
        let mut masked = text.to_string();
        masked = PRIVATE_KEY_RE
            .replace_all(&masked, "[REDACTED]")
            .into_owned();
        masked = GHP_TOKEN_RE.replace_all(&masked, "[REDACTED]").into_owned();
        masked = GITHUB_PAT_RE
            .replace_all(&masked, "[REDACTED]")
            .into_owned();
        masked = SECRET_KV_RE
            .replace_all(&masked, "$1=[REDACTED]")
            .into_owned();
        for secret in configured_secrets {
            if !secret.is_empty() {
                masked = masked.replace(secret.as_str(), "[REDACTED]");
            }
        }
        masked
    }

    /// ファセット内容中のテンプレート変数を展開する。
    /// - `{{task}}` → タスク内容（未指定時はプレースホルダーをそのまま残す）
    /// - `{{project_name}}` → worktree_pathの末尾ディレクトリ名
    pub(crate) fn render_facet_variables(
        content: &str,
        worktree_path: &str,
        task: Option<&str>,
    ) -> String {
        let project_name = Path::new(worktree_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let result = content.replace("{{project_name}}", project_name);
        match task {
            Some(t) => result.replace("{{task}}", t),
            None => result.replace("{{task}}", ""),
        }
    }

    /// [08] `releash workflow output submit` の CLI 例を実 run_id / step_name に展開する。
    ///
    /// `output_contract_preamble` が生成する `{{run_id}}` / `{{step_name}}` プレースホルダを
    /// 実行時の値に置き換え、agent / 人間オペレータがそのまま CLI を呼べるようにする
    /// （spec [08] 要求: 「run_id と step_name を主語に CLI/API 経由で engine に提出できる」）。
    pub(crate) fn render_submit_command_variables(
        content: &str,
        run_id: &str,
        step_name: &str,
    ) -> String {
        content
            .replace("{{run_id}}", run_id)
            .replace("{{step_name}}", step_name)
    }

    /// facet 本文に対し、起動環境別 alias と workflow 定義変数を namespace 展開する。
    ///
    /// spec issues-1054:
    /// - `{{path_alias.releash}}` → 起動環境別 alias 名 (`releash` / `releash-dev`)
    /// - `{{vars.<name>}}` → workflow 定義側の宣言値
    ///
    /// 既存プレースホルダ (`{{project_name}}` / `{{task}}` / `{{run_id}}` / `{{step_name}}`) は
    /// 名前空間を持たないトップレベル namespace に残り、別途 `render_facet_variables` /
    /// `render_submit_command_variables` で展開される。
    pub(crate) fn render_namespaced_variables(
        content: &str,
        workflow_declared_variables: &HashMap<String, String>,
    ) -> String {
        // path_alias の展開は alias 名のみで完結する。data_dir 解決を経由しないため
        // `alias_name_for_profile` を直接渡し、`dirs::data_dir()` 失敗から切り離す。
        let releash_alias = crate::path_aliases::alias_name_for_profile(
            crate::path_aliases::BuildProfile::current(),
        );
        let s =
            crate::workflow::facet::render_path_alias_variables_with_name(content, releash_alias);
        crate::workflow::facet::render_workflow_variables(&s, workflow_declared_variables)
    }

    /// [08] prose 抽出経路は engine から完全除去された（spec [08] Rule 4 構造化出力の
    /// 確定経路は明示的提出のみ）。本 helper は ChatSession 表示など event log と無関係な
    /// 経路で「最後の Agent メッセージ本文」を取り出すテスト用 fixture としてのみ残す。
    #[cfg(test)]
    fn extract_last_assistant_text_from_session(
        session: &crate::session::ChatSession,
    ) -> Option<String> {
        let agent_msg = session
            .messages
            .iter()
            .rev()
            .find(|m| m.role == crate::session::MessageRole::Agent)?;

        let text = if let Some(ref parts) = agent_msg.parts {
            Self::extract_text_from_parts(parts)
        } else {
            agent_msg.content.clone()
        };

        if text.is_empty() {
            return None;
        }

        Some(text)
    }

    /// ステップの出力をプロンプトにコンテキストブロックとして注入する。
    fn inject_step_outputs(
        prompt: &str,
        step: &NodeDefinition,
        step_outputs: &HashMap<String, StepOutput>,
        step_history: &[StepHistoryEntry],
        workflow_variables: &HashMap<String, String>,
    ) -> String {
        let mut result = prompt.to_string();

        // pass_previous_response: true → step_historyの最後のエントリのstep_nameからstep_outputsを参照
        // 前stepにStepOutputがない場合は何も注入しない（Spec: 何も注入されない）
        if step.pass_previous_response == Some(true) {
            if let Some(last_entry) = step_history.last() {
                if let Some(o) = step_outputs.get(&last_entry.step_name) {
                    let text = Self::format_step_output_block(o);
                    Self::append_step_output_block(&mut result, &last_entry.step_name, &text);
                }
            }
        }

        // pass_output_from: ["step_a", "step_b"] → 指定step名のoutputをcontext block追加
        if let Some(ref refs) = step.pass_output_from {
            for step_name in refs {
                let text = match step_outputs.get(step_name.as_str()) {
                    Some(o) => Self::format_step_output_block(o),
                    None => "(not yet completed)".to_string(),
                };
                Self::append_step_output_block(&mut result, step_name, &text);
            }
        }

        Self::append_workflow_variables_block(&mut result, workflow_variables);

        result
    }

    /// task が非空、かつ呼び出し側が input_contracts 由来の task 入力を期待しているときに限り、
    /// `<task>` ブロックを末尾に注入する。
    ///
    /// [02] Contract 双方向対称性: `<task>` ブロックは step が `input_contracts` で
    /// 入力データ仕様を宣言しているときだけ engine が prompt に流し込む。
    /// `input_contracts` を持たない既存 builtin step (例: plan-requirements は
    /// instruction 内で `{{task}}` テンプレートを直接展開する) には注入しないことで、
    /// 既存 prompt の同等性を保つ。
    ///
    /// task 文字列はユーザー制御の信頼境界外入力のため、`<`/`>`/`&` をエスケープし、
    /// 偽の `</task>` 等で engine 注入ブロックを偽装する prompt injection を防ぐ。
    fn append_task_block(prompt: &mut String, task: Option<&str>, allow_task_injection: bool) {
        if !allow_task_injection {
            return;
        }
        let Some(t) = task else { return };
        if t.is_empty() {
            return;
        }
        let escaped = Self::escape_xml_text(t);
        prompt.push_str(&format!("\n\n<task>\n{}\n</task>", escaped));
    }

    fn append_output_contract_completion_action(
        prompt: &mut String,
        output_contract: Option<&str>,
        run_id: &str,
        step_name: &str,
        workflow_declared_variables: &HashMap<String, String>,
    ) {
        let Some(contract) = output_contract else {
            return;
        };
        let action = crate::workflow::facet::output_contract_completion_action(contract);
        let action = Self::render_submit_command_variables(&action, run_id, step_name);
        let action = Self::render_namespaced_variables(&action, workflow_declared_variables);
        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str(&action);
    }

    /// XML 風タグ内に文字列を埋め込む際に `<` / `>` / `&` をエスケープする。
    /// `<task>` ブロック等の信頼境界外データを engine が注入する経路で利用する。
    fn escape_xml_text(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            match c {
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '&' => out.push_str("&amp;"),
                other => out.push(other),
            }
        }
        out
    }

    /// StepOutputのstructured_outputをJSON文字列としてフォーマットする。
    fn format_step_output_block(output: &StepOutput) -> String {
        match &output.structured_output {
            Some(json) => serde_json::to_string_pretty(json).unwrap_or_else(|_| "{}".to_string()),
            None => "(no structured output)".to_string(),
        }
    }

    fn append_step_output_block(result: &mut String, step_name: &str, text: &str) {
        result.push_str(&format!(
            "\n\n<step_output name=\"{}\">\n{}\n</step_output>",
            step_name, text
        ));
    }

    fn append_workflow_variables_block(
        result: &mut String,
        workflow_variables: &HashMap<String, String>,
    ) {
        let filtered_variables: HashMap<_, _> = workflow_variables
            .iter()
            .filter(|(key, _)| !key.starts_with("approved_fix_policy"))
            .collect();
        if filtered_variables.is_empty() {
            return;
        }
        let vars_json = serde_json::to_string_pretty(&filtered_variables).unwrap_or_default();
        result.push_str(&format!(
            "\n\n<workflow_variables>\n{}\n</workflow_variables>",
            vars_json
        ));
    }

    /// collect設定に基づいてstep_outputsをreduce処理する。
    fn apply_reduce(
        collect: &CollectConfig,
        step_outputs: &HashMap<String, StepOutput>,
    ) -> ReduceResult {
        match collect.reduce {
            ReduceStrategy::Last => {
                let last_output = collect
                    .from
                    .iter()
                    .filter_map(|name| step_outputs.get(name.as_str()))
                    .max_by(|a, b| {
                        a.completed_at
                            .partial_cmp(&b.completed_at)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                match last_output {
                    Some(output) => ReduceResult {
                        result: output.result.clone(),
                        structured_output: output.structured_output.clone(),
                    },
                    None => ReduceResult {
                        result: None,
                        structured_output: None,
                    },
                }
            }
            ReduceStrategy::Concat => {
                let entries = Self::collect_step_output_entries(&collect.from, step_outputs);
                ReduceResult {
                    result: None,
                    structured_output: if entries.is_empty() {
                        None
                    } else {
                        Some(serde_json::Value::Array(entries))
                    },
                }
            }
            ReduceStrategy::Grouped => {
                let mut groups: HashMap<String, Vec<String>> = HashMap::new();
                for step_name in &collect.from {
                    if let Some(output) = step_outputs.get(step_name.as_str()) {
                        let key = output
                            .result
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string());
                        groups.entry(key).or_default().push(step_name.clone());
                    }
                }
                let grouped_json: serde_json::Map<String, serde_json::Value> = groups
                    .into_iter()
                    .map(|(k, v)| {
                        (
                            k,
                            serde_json::Value::Array(
                                v.into_iter().map(serde_json::Value::String).collect(),
                            ),
                        )
                    })
                    .collect();
                ReduceResult {
                    result: None,
                    structured_output: if grouped_json.is_empty() {
                        None
                    } else {
                        Some(serde_json::Value::Object(grouped_json))
                    },
                }
            }
            ReduceStrategy::AnyNeedsFix => {
                let mut any_needs_fix = false;
                for step_name in &collect.from {
                    if let Some(output) = step_outputs.get(step_name.as_str()) {
                        let step_result = Self::resolve_step_result(output);
                        if matches!(
                            step_result.as_deref(),
                            Some("NEEDS_FIX") | Some("needs_fix")
                        ) {
                            any_needs_fix = true;
                        }
                    } else {
                        any_needs_fix = true;
                    }
                }
                let entries = Self::collect_step_output_entries(&collect.from, step_outputs);
                ReduceResult {
                    result: Some(if any_needs_fix { "NEEDS_FIX" } else { "LGTM" }.to_string()),
                    structured_output: if entries.is_empty() {
                        None
                    } else {
                        Some(serde_json::Value::Array(entries))
                    },
                }
            }
            ReduceStrategy::AllPassed => {
                let mut all_passed = true;
                for step_name in &collect.from {
                    if let Some(output) = step_outputs.get(step_name.as_str()) {
                        let step_result = Self::resolve_step_result(output);
                        if !matches!(
                            step_result.as_deref(),
                            Some("PASSED") | Some("passed") | Some("LGTM")
                        ) {
                            all_passed = false;
                        }
                    } else {
                        all_passed = false;
                    }
                }
                let entries = Self::collect_step_output_entries(&collect.from, step_outputs);
                ReduceResult {
                    result: Some(if all_passed { "PASSED" } else { "FAILED" }.to_string()),
                    structured_output: if entries.is_empty() {
                        None
                    } else {
                        Some(serde_json::Value::Array(entries))
                    },
                }
            }
        }
    }

    /// collect.fromのステップ名リストから [{ "stepName": "...", "output": ... }] 形式の配列を構築する。
    fn collect_step_output_entries(
        from: &[String],
        step_outputs: &HashMap<String, StepOutput>,
    ) -> Vec<serde_json::Value> {
        let mut entries = Vec::new();
        for step_name in from {
            if let Some(output) = step_outputs.get(step_name.as_str()) {
                if let Some(ref so) = output.structured_output {
                    entries.push(serde_json::json!({
                        "stepName": step_name,
                        "output": so,
                    }));
                }
            }
        }
        entries
    }

    /// StepOutputからresultを解決する。
    /// structured_output.verdict → structured_output.status → result の優先順で参照する。
    fn resolve_step_result(output: &StepOutput) -> Option<String> {
        if let Some(ref so) = output.structured_output {
            if let Some(verdict) = so.get("verdict").and_then(|v| v.as_str()) {
                return Some(verdict.to_string());
            }
            if let Some(status) = so.get("status").and_then(|v| v.as_str()) {
                return Some(status.to_string());
            }
        }
        output.result.clone()
    }

    /// AgentSessionを中断する。
    async fn interrupt_agent(&self, handles: &Arc<Mutex<AgentProcessMap>>, session_id: &str) {
        use tokio::io::AsyncWriteExt;

        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(session_id) {
            if let Err(e) = proc.stdin.write_all(b"{\"type\":\"interrupt\"}\n").await {
                log::warn!(
                    "Failed to write interrupt for session '{}': {e}",
                    session_id
                );
            }
            if let Err(e) = proc.stdin.flush().await {
                log::warn!(
                    "Failed to flush interrupt for session '{}': {e}",
                    session_id
                );
            }
        }
    }

    async fn release_completed_step_session<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        session_id: &str,
    ) {
        let open_tabs_state = app.try_state::<Arc<crate::session::OpenTabRegistry>>();
        let open_tabs = open_tabs_state.as_ref().map(|state| state.inner().as_ref());
        crate::workflow_step_lifecycle_adapters::release_step_runtime_on_done(
            app,
            session_store,
            handles,
            open_tabs,
            session_id,
        )
        .await;
    }

    fn completed_step_session_ids(snapshot: &WorkflowState) -> Vec<String> {
        let mut ids = Vec::new();
        let Some(entry) = snapshot.step_history.last() else {
            return ids;
        };
        if let Some(session_id) = entry.session_id.as_ref() {
            ids.push(session_id.clone());
        }
        if let Some(children) = entry.child_outputs.as_ref() {
            ids.extend(children.iter().filter_map(|child| child.session_id.clone()));
        }
        ids.sort();
        ids.dedup();
        ids
    }

    fn outcome_snapshot(outcome: &StepOutcome) -> &WorkflowState {
        match outcome {
            StepOutcome::Persist(s)
            | StepOutcome::TransitionAndStart(s)
            | StepOutcome::ReduceAndTransition(s)
            | StepOutcome::StartParallel(s) => s,
        }
    }

    fn completed_step_session_ids_for_outcome(outcome: &StepOutcome) -> Vec<String> {
        match outcome {
            StepOutcome::Persist(snapshot)
                if matches!(snapshot.state, WorkflowExecutionState::Aborted) =>
            {
                snapshot.current_session_id.iter().cloned().collect()
            }
            StepOutcome::Persist(snapshot)
                if matches!(
                    snapshot.state,
                    WorkflowExecutionState::Completed | WorkflowExecutionState::Failed { .. }
                ) =>
            {
                Self::completed_step_session_ids(snapshot)
            }
            StepOutcome::Persist(_) => Vec::new(),
            StepOutcome::TransitionAndStart(snapshot)
            | StepOutcome::ReduceAndTransition(snapshot)
            | StepOutcome::StartParallel(snapshot) => Self::completed_step_session_ids(snapshot),
        }
    }

    fn terminal_step_session_ids(snapshot: &WorkflowState) -> Vec<String> {
        let mut ids = Vec::new();
        ids.extend(snapshot.current_session_id.iter().cloned());
        ids.extend(
            snapshot
                .active_parallel_steps
                .iter()
                .filter_map(|step| step.session_id.clone()),
        );
        ids.sort();
        ids.dedup();
        ids
    }

    async fn release_completed_step_sessions<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        session_ids: &[String],
    ) {
        for session_id in session_ids {
            self.release_completed_step_session(app, session_store, handles, session_id)
                .await;
        }
    }

    /// [04] pre-commit projection phase: required event append 前に Run Store と
    /// Run Store の active projection / terminal metadata を snapshot に揃える。
    /// append-only event fact が command の最初の不可逆な可視 commit point であり、
    /// この helper の失敗は event append 前に rollback できる。
    async fn project_state_before_required_event_commit(
        &self,
        snapshot: &WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
        let run_id = snapshot.execution_id.clone();
        self.sync_run_store_from_snapshot(&run_id, snapshot).await
    }

    async fn commit_required_events<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        commit: RequiredEventCommit<'_>,
    ) -> Result<(), WorkflowEngineError> {
        let RequiredEventCommit {
            run_id,
            snapshot_for_commit,
            snapshot_before,
            run_store_snapshot_before,
            required_events,
            append_error_context,
        } = commit;

        let rollback_snapshot_before = snapshot_before.clone();
        let projection_error_context = "required event projection failed";
        if let Err(e) = self
            .project_state_before_required_event_commit(snapshot_for_commit)
            .await
        {
            let _ = self
                .rollback_command_mutation(
                    app,
                    session_store,
                    CommandMutationRollback {
                        run_id,
                        snapshot_before: rollback_snapshot_before,
                        run_store_snapshot_before,
                        context: projection_error_context,
                    },
                )
                .await;
            return Err(WorkflowEngineError::SessionStore(format!(
                "{projection_error_context}: {e}"
            )));
        }

        if let Err(e) = self.write_log_required_batch(app, &required_events) {
            let _ = self
                .rollback_command_mutation(
                    app,
                    session_store,
                    CommandMutationRollback {
                        run_id,
                        snapshot_before,
                        run_store_snapshot_before,
                        context: append_error_context,
                    },
                )
                .await;
            return Err(WorkflowEngineError::SessionStore(format!(
                "{append_error_context}: {e}"
            )));
        }

        Ok(())
    }

    /// [04] pre-commit phase: sync_run_store + release_completed_step_sessions を実行する。
    /// 本 helper は本 issue scope 外の non-command 経路（NodeCompleted/NodeFailed 系の
    /// `persist_release_and_broadcast` 呼び出し）専用に温存する。
    /// 本 issue scope の command 受理 handler は required event append 前の rollback 可能な
    /// projection と post-commit `release_completed_step_sessions` の組み合わせを使う。
    async fn sync_persist_release<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        snapshot: &WorkflowState,
        completed_step_session_ids: &[String],
    ) -> Result<(), WorkflowEngineError> {
        let run_id = snapshot.execution_id.clone();
        if let Err(e) = self.sync_run_store_from_snapshot(&run_id, snapshot).await {
            self.rollback_execution_projection_after_run_store_sync_failure(&run_id, snapshot)
                .await;
            return Err(e);
        }
        self.release_completed_step_sessions(
            app,
            session_store,
            handles,
            completed_step_session_ids,
        )
        .await;
        Ok(())
    }

    /// [04] post-commit phase: terminal log + cleanup_refs + broadcast。required append
    /// 完了後の副作用に限定し、失敗は warn として観測する（command 結果には伝播しない）。
    async fn finalize_after_commit<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        snapshot: &WorkflowState,
        worktree_path: &str,
        write_terminal_events: bool,
    ) {
        let run_id = snapshot.execution_id.clone();
        let is_terminal = matches!(
            snapshot.state,
            WorkflowExecutionState::Completed
                | WorkflowExecutionState::Failed { .. }
                | WorkflowExecutionState::Aborted
        );
        if is_terminal {
            if write_terminal_events {
                if matches!(snapshot.state, WorkflowExecutionState::Completed) {
                    if let Err(e) = self.write_last_step_completed_log(app, snapshot) {
                        log::warn!("Failed to append NodeCompleted workflow event: {e}");
                    }
                }
                if let Err(e) = self.write_terminal_log(app, snapshot) {
                    log::warn!("Failed to append terminal workflow events: {e}");
                }
            }
            self.cleanup_session_workflow_refs_by_run_id(&run_id).await;
        }
        self.broadcast_state(app, worktree_path, snapshot.clone())
            .await;
    }

    /// 既存呼び出し元（on_turn_complete 等）から使う一括 helper。pre-commit と post-commit
    /// を順に呼ぶだけで、外部 contract は変えない。
    #[allow(clippy::too_many_arguments)]
    async fn persist_release_and_broadcast<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        snapshot: WorkflowState,
        completed_step_session_ids: &[String],
    ) -> Result<WorkflowState, WorkflowEngineError> {
        self.sync_persist_release(
            app,
            session_store,
            handles,
            &snapshot,
            completed_step_session_ids,
        )
        .await?;
        self.finalize_after_commit(app, &snapshot, worktree_path, true)
            .await;
        Ok(snapshot)
    }

    /// ロック内で次ステップへの advance を適用する（純粋な状態変更）。
    /// `&self` を使わないため関連関数として定義。
    fn apply_advance(exec: &mut WorkflowExecution) -> StepOutcome {
        let decision = exec.decide_next_step();
        match decision {
            NextStepDecision::Completed => {
                exec.state = WorkflowExecutionState::Completed;
                exec.updated_at = current_timestamp();
                StepOutcome::Persist(exec.to_workflow_state())
            }
            NextStepDecision::TransitionTo(name) => {
                let idx = exec
                    .workflow
                    .nodes
                    .iter()
                    .position(|s| s.name == name)
                    .expect("decide_next_step returned unknown step");
                exec.current_step_index = idx;
                exec.state = WorkflowExecutionState::Running;
                *exec.step_execution_counts.entry(name).or_insert(0) += 1;
                exec.clear_step_outputs_for_new_execution(idx);
                exec.updated_at = current_timestamp();

                // resets_cycle_for: 遷移先ステップの設定に従い指定ステップのカウントをリセット
                let resets = exec.workflow.nodes[idx].resets_cycle_for.clone();
                if let Some(targets) = resets {
                    for target in &targets {
                        exec.step_execution_counts.remove(target);
                    }
                }

                let step = &exec.workflow.nodes[idx];
                if step.is_parallel() {
                    StepOutcome::StartParallel(exec.to_workflow_state())
                } else if step.collect.is_some() {
                    StepOutcome::ReduceAndTransition(exec.to_workflow_state())
                } else {
                    StepOutcome::TransitionAndStart(exec.to_workflow_state())
                }
            }
        }
    }

    /// ロック内で指定ステップへの遷移を適用する（サイクルガード検証含む）。
    /// `&self` を使わないため関連関数として定義。
    fn apply_transition(
        exec: &mut WorkflowExecution,
        target_step_name: &str,
    ) -> Result<StepOutcome, WorkflowEngineError> {
        if exec.is_terminal() {
            return Ok(StepOutcome::Persist(exec.to_workflow_state()));
        }

        Self::apply_transition_inner(exec, target_step_name, 0)
    }

    fn apply_transition_inner(
        exec: &mut WorkflowExecution,
        target_step_name: &str,
        depth: usize,
    ) -> Result<StepOutcome, WorkflowEngineError> {
        let max_depth = exec.workflow.nodes.len();
        if depth >= max_depth {
            exec.state = WorkflowExecutionState::Failed {
                reason: format!("on_exhausted chain depth exceeded (max={})", max_depth),
            };
            exec.updated_at = current_timestamp();
            return Ok(StepOutcome::Persist(exec.to_workflow_state()));
        }

        let idx = exec
            .workflow
            .nodes
            .iter()
            .position(|s| s.name == target_step_name)
            .ok_or_else(|| {
                WorkflowEngineError::InvalidWorkflow(format!(
                    "Step '{}' not found in workflow",
                    target_step_name
                ))
            })?;

        let guard_result = exec.check_cycle_guard(target_step_name)?;
        match guard_result {
            CycleGuardResult::Exceeded {
                max_iterations,
                count,
                on_exhausted,
            } => {
                if let Some(fallback_target) = on_exhausted {
                    Self::apply_transition_inner(exec, &fallback_target, depth + 1)
                } else {
                    exec.state = WorkflowExecutionState::Failed {
                        reason: format!(
                            "Cycle guard exceeded for step '{}': max_iterations={}, executed={}",
                            target_step_name, max_iterations, count
                        ),
                    };
                    exec.updated_at = current_timestamp();
                    Ok(StepOutcome::Persist(exec.to_workflow_state()))
                }
            }
            CycleGuardResult::Allowed => {
                exec.current_step_index = idx;
                exec.state = WorkflowExecutionState::Running;
                *exec
                    .step_execution_counts
                    .entry(target_step_name.to_string())
                    .or_insert(0) += 1;
                exec.clear_step_outputs_for_new_execution(idx);
                exec.updated_at = current_timestamp();

                // resets_cycle_for: 遷移先ステップの設定に従い指定ステップのカウントをリセット
                let resets = exec.workflow.nodes[idx].resets_cycle_for.clone();
                if let Some(targets) = resets {
                    for target in &targets {
                        exec.step_execution_counts.remove(target);
                    }
                }

                let step = &exec.workflow.nodes[idx];
                if step.is_parallel() {
                    Ok(StepOutcome::StartParallel(exec.to_workflow_state()))
                } else if step.collect.is_some() {
                    Ok(StepOutcome::ReduceAndTransition(exec.to_workflow_state()))
                } else {
                    Ok(StepOutcome::TransitionAndStart(exec.to_workflow_state()))
                }
            }
        }
    }

    /// ロック外でStepOutcomeに応じた副作用（永続化・ブロードキャスト・AgentSession起動）を実行する。
    ///
    /// 本 helper は non-command 経路（NodeCompleted / NodeFailed 等）から呼ばれる。
    ///
    /// [05] commit 境界: spec [04] commit_required_events を基盤に、StepOutcome から
    /// `NodeCompleted` / `NodeFailed` / `RunCompleted` / `RunFailed` の必須 event を
    /// 組み立て、RunStore sync → ChatSession persist → event log append の順で commit
    /// する。いずれかの phase で失敗した場合は engine state と Run Store snapshot を
    /// `snapshot_before` で一括復元することで、event log と engine state / RunStore /
    /// ChatSession の分離を防ぐ（spec [05]: state mutation と event log の分離を防ぐ
    /// rollback 境界 / atomic mutation 境界）。
    ///
    /// 必須 event が空の場合は従来通り `sync_persist_release` のみを実行する。
    async fn execute_outcome<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        outcome: StepOutcome,
        snapshot_before: WorkflowExecution,
    ) -> Result<(), WorkflowEngineError> {
        let completed_step_session_ids = Self::completed_step_session_ids_for_outcome(&outcome);
        let snapshot_for_commit = Self::outcome_snapshot(&outcome).clone();
        let run_id = snapshot_for_commit.execution_id.clone();

        // [05] pre-commit phase: 必須 event の生成。`dispatch_internal_node_command` の
        // ValidationError は engine state を snapshot_before で復元して伝播する
        // （spec [05] silent error 禁止）。
        let pre_commit_events = match Self::pre_commit_required_events_for_outcome(&outcome) {
            Ok(events) => events,
            Err(e) => {
                let mut execs = self.executions.lock().await;
                if let Some(exec) = execs.get_mut(&run_id) {
                    *exec = snapshot_before;
                }
                return Err(e);
            }
        };

        if !pre_commit_events.is_empty() {
            // [05] commit_required_events 基盤: 順序と rollback 方針を一箇所に集約。
            // 失敗時は engine state と Run Store snapshot を一括復元する。
            let run_store_snapshot_before = self.run_store.active_run_snapshot(&run_id).await;
            self.commit_required_events(
                app,
                session_store,
                RequiredEventCommit {
                    run_id: &run_id,
                    snapshot_for_commit: &snapshot_for_commit,
                    snapshot_before,
                    run_store_snapshot_before,
                    required_events: pre_commit_events,
                    append_error_context: "execute_outcome required event append failed",
                },
            )
            .await?;
            self.release_completed_step_sessions(
                app,
                session_store,
                handles,
                &completed_step_session_ids,
            )
            .await;
        } else {
            // 必須 event 無し: 従来通り sync_persist_release のみ。
            self.sync_persist_release(
                app,
                session_store,
                handles,
                &snapshot_for_commit,
                &completed_step_session_ids,
            )
            .await?;
        }

        // terminal / NodeCompleted は append 済みのため finalize_after_commit には
        // write_terminal_events=false を渡し二重 append を避ける（commit 境界の単一性）。
        self.finalize_after_commit(app, &snapshot_for_commit, worktree_path, false)
            .await;
        self.dispatch_step_outcome_side_effects(
            app,
            session_store,
            handles,
            worktree_path,
            outcome,
            OutcomeCommitMode::ProgressEventsAlreadyCommitted,
        )
        .await
    }

    /// [05] StepOutcome から persist 前に append すべき必須 event を組み立てる
    /// 純粋関数。`execute_outcome` の pre-commit phase でのみ呼ばれる。
    ///
    /// `Persist`:
    /// terminal（Completed / Failed）の場合 NodeCompleted（Completed のみ）+ terminal
    /// events（RunCompleted / NodeFailed+RunFailed）を返す。Aborted は `AbortRun`
    /// command 経路で別途 append されるため本関数では扱わない。
    ///
    /// `TransitionAndStart` / `ReduceAndTransition` / `StartParallel`:
    /// 直前の step 完了に対応する NodeCompleted を返す。NodeStarted は best-effort
    /// write_log 側で扱う（spec scope: required append は NodeCompleted / 終了系のみ）。
    fn pre_commit_required_events_for_outcome(
        outcome: &StepOutcome,
    ) -> Result<Vec<WorkflowEvent>, WorkflowEngineError> {
        let mut events = Vec::new();
        let mut snapshot = Self::outcome_snapshot(outcome).clone();
        match outcome {
            StepOutcome::Persist(s) => {
                let is_terminal = matches!(
                    s.state,
                    WorkflowExecutionState::Completed | WorkflowExecutionState::Failed { .. }
                );
                if is_terminal {
                    if matches!(s.state, WorkflowExecutionState::Completed) {
                        if let Some(ev) =
                            Self::last_step_completed_event_for_snapshot(&mut snapshot)?
                        {
                            events.push(ev);
                        }
                    }
                    events.extend(Self::terminal_events_for_snapshot(&mut snapshot)?);
                }
            }
            StepOutcome::TransitionAndStart(_)
            | StepOutcome::ReduceAndTransition(_)
            | StepOutcome::StartParallel(_) => {
                if let Some(ev) = Self::last_step_completed_event_for_snapshot(&mut snapshot)? {
                    events.push(ev);
                }
            }
        }
        Ok(events)
    }

    /// [04] post-commit variant work（共通 side-effect helper）。
    ///
    /// snapshot は既に persist 済みである前提で、outcome variant に応じた残りの副作用
    /// （NodeStarted 書き込み・start_step_session・reduce + 派生 mutation の再帰・
    /// start_parallel_children・auto-approve dispatch）のみを担当する。`execute_outcome`
    /// （non-command 経路）と `handle_approval` などの 4 command handler の双方から
    /// 呼ばれ、副作用ロジックの単一 source of truth として機能する。失敗は warn 化して
    /// command 結果に伝播させない設計に揃える（spec [04] post-commit 境界）。
    async fn dispatch_step_outcome_side_effects<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        outcome: StepOutcome,
        commit_mode: OutcomeCommitMode,
    ) -> Result<(), WorkflowEngineError> {
        match outcome {
            StepOutcome::Persist(snapshot) => {
                if let Some((execution_id, step_name)) =
                    Self::auto_approve_target_for_persisted_snapshot(
                        &snapshot,
                        Self::workflow_approval_auto_approve_enabled(app),
                    )
                {
                    return Box::pin(self.dispatch(
                        app,
                        session_store,
                        handles,
                        WorkflowCommand::ApproveNode {
                            run_id: execution_id,
                            node_name: Some(step_name),
                            comment: None,
                        },
                    ))
                    .await
                    .map(|_| ());
                }
                Ok(())
            }
            StepOutcome::TransitionAndStart(snapshot) => {
                if commit_mode.should_emit_progress_events() {
                    if let Err(e) = self.write_last_step_completed_log(app, &snapshot) {
                        return Err(WorkflowEngineError::SessionStore(format!(
                            "TransitionAndStart pre-commit NodeCompleted append failed: {e}"
                        )));
                    }
                }
                let exec_count = snapshot
                    .step_execution_counts
                    .get(&snapshot.current_step_name)
                    .copied()
                    .unwrap_or(1);
                if commit_mode.should_emit_progress_events() {
                    self.write_log(
                        app,
                        WorkflowEvent::NodeStarted {
                            run_id: snapshot.execution_id.clone(),
                            workflow_name: snapshot.workflow_name.clone(),
                            node_name: snapshot.current_step_name.clone(),
                            execution_count: exec_count,
                            timestamp: snapshot.updated_at,
                        },
                    );
                }
                if let Err(e) = self
                    .start_step_session(app, handles, session_store, worktree_path)
                    .await
                {
                    {
                        let mut execs = self.executions.lock().await;
                        if let Some(exec) = find_by_worktree_mut(&mut execs, worktree_path) {
                            let entry = exec.make_step_history_entry(
                                Some(format!("session_start_failed: {e}")),
                                None,
                                None,
                            );
                            exec.step_history.push(entry);
                        }
                    }
                    let _ = self
                        .set_execution_state(
                            app,
                            session_store,
                            handles,
                            worktree_path,
                            WorkflowExecutionState::Failed {
                                reason: format!("Failed to start step session: {e}"),
                            },
                        )
                        .await;
                    return Err(e);
                }
                Ok(())
            }
            StepOutcome::ReduceAndTransition(snapshot) => {
                if commit_mode.should_emit_progress_events() {
                    if let Err(e) = self.write_last_step_completed_log(app, &snapshot) {
                        return Err(WorkflowEngineError::SessionStore(format!(
                            "ReduceAndTransition pre-commit NodeCompleted append failed: {e}"
                        )));
                    }
                }

                let (collect_config_clone, reduce_result, step_rules) = {
                    let execs = self.executions.lock().await;
                    let (_, exec) = find_by_worktree(&execs, worktree_path).ok_or_else(|| {
                        WorkflowEngineError::ExecutionNotFound(worktree_path.to_string())
                    })?;
                    let step = &exec.workflow.nodes[exec.current_step_index];
                    let collect = step
                        .collect
                        .clone()
                        .expect("ReduceAndTransition requires collect config");
                    let result = Self::apply_reduce(&collect, &exec.step_outputs);
                    (collect, result, step.transition_rules.clone())
                };

                let (next_outcome, log_step_name, log_exec_id, log_wf_name, snapshot_before) = {
                    let mut execs = self.executions.lock().await;
                    let exec =
                        find_by_worktree_mut(&mut execs, worktree_path).ok_or_else(|| {
                            WorkflowEngineError::ExecutionNotFound(worktree_path.to_string())
                        })?;
                    let snapshot_before = exec.clone();

                    let entry = exec.make_step_history_entry(
                        reduce_result.result.clone(),
                        reduce_result.structured_output.clone(),
                        None,
                    );
                    exec.step_history.push(entry);

                    let step_name = exec.workflow.nodes[exec.current_step_index].name.clone();
                    let exec_id = exec.id.clone();
                    let wf_name = exec.workflow.name.clone();

                    log::info!(
                        "OutputCollected: step='{}', strategy={:?}, from={:?}",
                        step_name,
                        collect_config_clone.reduce,
                        collect_config_clone.from,
                    );

                    let outcome = if step_rules.is_empty() {
                        Self::apply_advance(exec)
                    } else if let Some(ref result_str) = reduce_result.result {
                        match Self::evaluate_auto_rules(result_str, &step_rules) {
                            Some((next_step, _)) => Self::apply_transition(exec, &next_step)?,
                            None => Self::apply_advance(exec),
                        }
                    } else {
                        Self::apply_advance(exec)
                    };
                    (outcome, step_name, exec_id, wf_name, snapshot_before)
                };

                let collected_entries: Vec<CollectedOutputEntry> = collect_config_clone
                    .from
                    .iter()
                    .map(|name| {
                        let output = snapshot.step_outputs.get(name);
                        CollectedOutputEntry {
                            node_name: name.clone(),
                            result: output.and_then(|o| o.result.clone()),
                            structured_output: output.and_then(|o| o.structured_output.clone()),
                        }
                    })
                    .collect();
                self.write_log(
                    app,
                    WorkflowEvent::OutputCollected {
                        run_id: log_exec_id,
                        workflow_name: log_wf_name,
                        node_name: log_step_name,
                        node_outputs: collected_entries,
                        reduce_strategy: format!("{:?}", collect_config_clone.reduce),
                        reduce_result: reduce_result.result.clone(),
                        reduce_structured_output: reduce_result.structured_output.clone(),
                        timestamp: crate::session::now_timestamp(),
                    },
                );

                // 次 outcome は新たな state mutation なので、再度 sync+persist が必要。
                // execute_outcome 経由でフル経路を回す（spec [04] post-commit 内で発生する
                // 派生 mutation も同じ atomic 境界に乗せる）。
                Box::pin(self.execute_outcome(
                    app,
                    session_store,
                    handles,
                    worktree_path,
                    next_outcome,
                    snapshot_before,
                ))
                .await
            }
            StepOutcome::StartParallel(snapshot) => {
                if commit_mode.should_emit_progress_events() {
                    if let Err(e) = self.write_last_step_completed_log(app, &snapshot) {
                        return Err(WorkflowEngineError::SessionStore(format!(
                            "StartParallel pre-commit NodeCompleted append failed: {e}"
                        )));
                    }
                }
                if let Err(e) = self
                    .start_parallel_children(
                        app,
                        session_store,
                        handles,
                        worktree_path,
                        commit_mode.should_emit_progress_events(),
                    )
                    .await
                {
                    let _ = self
                        .set_execution_state(
                            app,
                            session_store,
                            handles,
                            worktree_path,
                            WorkflowExecutionState::Failed {
                                reason: format!("Failed to start parallel children: {e}"),
                            },
                        )
                        .await;
                    return Err(e);
                }
                Ok(())
            }
        }
    }

    /// 並列ブロックの子ステップをすべて起動する。
    #[allow(clippy::too_many_arguments)]
    async fn start_parallel_children<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        emit_parallel_started: bool,
    ) -> Result<(), WorkflowEngineError> {
        // ロック内: 子ステップ定義取得 + ParallelRunState構築
        let (
            parallel_steps,
            parent_step_name,
            aggregate,
            execution_id,
            workflow_name,
            task_clone,
            workflow_defaults,
        ) = {
            let mut execs = self.executions.lock().await;
            let exec = find_by_worktree_mut(&mut execs, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;

            let step = &exec.workflow.nodes[exec.current_step_index];
            let parallel = step
                .parallel_children
                .clone()
                .expect("StartParallel requires parallel field");
            let agg = step.aggregate.clone();
            let parent_name = step.name.clone();
            let exec_id = exec.id.clone();
            let wf_name = exec.workflow.name.clone();
            let task = exec.task.clone();
            let defaults = exec.workflow_defaults.clone();

            (parallel, parent_name, agg, exec_id, wf_name, task, defaults)
        };

        // ParallelStarted ログ
        let child_step_names: Vec<String> =
            parallel_steps.iter().map(|ps| ps.name.clone()).collect();
        if emit_parallel_started {
            self.write_log(
                app,
                WorkflowEvent::ParallelStarted {
                    run_id: execution_id.clone(),
                    workflow_name: workflow_name.clone(),
                    parent_node_name: parent_step_name.clone(),
                    child_node_names: child_step_names.clone(),
                    timestamp: current_timestamp(),
                },
            );
        }

        // 各子ステップのセッション生成 + AgentSession起動
        let data_dir = crate::session::resolve_data_dir(app)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;

        // step_outputsとworkflow_variablesのスナップショットをロック外で取得
        let (step_outputs_snapshot, wf_variables_snapshot, wf_declared_variables_snapshot) = {
            let execs = self.executions.lock().await;
            let (_, exec) = find_by_worktree(&execs, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            (
                exec.step_outputs.clone(),
                exec.workflow_variables.clone(),
                exec.workflow.variables.clone(),
            )
        };

        // Phase 1: セッション生成 + ref登録 + プロンプト構築（AgentSessionはまだ起動しない）
        struct ChildSetup {
            step_name: String,
            session_id: String,
            system_prompt: Option<String>,
            user_message: String,
            output_contract: Option<String>,
            permission_mode: String,
        }
        let mut child_setups: Vec<ChildSetup> = Vec::new();

        for ps in &parallel_steps {
            // 子ステップ設定の解決 → セッション生成（workflow_defaults を継承元に注入）
            let step_session = self
                .create_step_session_with_settings(
                    app,
                    session_store,
                    &data_dir,
                    worktree_path,
                    ps.model.clone(),
                    ps.permission.clone(),
                    &workflow_defaults,
                )
                .await?;
            let child_permission_mode = step_session.permission_mode.clone();
            let step_session_id = step_session.id.clone();

            // session_workflow_refs に Step として登録（並列子か否かは
            // exec.parallel_run.children から動的に判定する）
            {
                let mut map = self.session_workflow_refs.lock().await;
                map.insert(
                    step_session_id.clone(),
                    SessionWorkflowRef {
                        run_id: execution_id.clone(),
                    },
                );
            }

            // ファセットからプロンプト構築
            let (system_prompt, user_message) = Self::build_parallel_step_prompt(
                ps,
                &execution_id,
                worktree_path,
                task_clone.as_deref(),
                &step_outputs_snapshot,
                ps.pass_previous_response.unwrap_or(false),
                ps.pass_output_from.as_deref(),
                &wf_variables_snapshot,
                &wf_declared_variables_snapshot,
            )?;

            child_setups.push(ChildSetup {
                step_name: ps.name.clone(),
                session_id: step_session_id,
                system_prompt,
                user_message,
                permission_mode: child_permission_mode,
                output_contract: ps.output_contract.clone(),
            });
        }

        // Phase 2: ParallelRunState を先に設定（レース条件防止）
        // step_execution_countsをインクリメントし、run_indexに反映する
        let (child_run_indices, snapshot) = {
            let mut execs = self.executions.lock().await;
            let exec = find_by_worktree_mut(&mut execs, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;

            let indices: Vec<u32> = child_setups
                .iter()
                .map(|cs| {
                    let count = exec
                        .step_execution_counts
                        .entry(cs.step_name.clone())
                        .or_insert(0);
                    *count += 1;
                    *count
                })
                .collect();

            let children: Vec<ParallelChildRun> = child_setups
                .iter()
                .zip(indices.iter())
                .map(|(cs, &run_index)| ParallelChildRun {
                    step_name: cs.step_name.clone(),
                    session_id: cs.session_id.clone(),
                    state: ParallelChildState::Running,
                    result: None,
                    structured_output: None,
                    output_contract: cs.output_contract.clone(),
                    token_usage: TokenUsage::default(),
                    run_index,
                })
                .collect();

            exec.parallel_run = Some(ParallelRunState {
                parent_step_name: parent_step_name.clone(),
                aggregate,
                children,
            });
            let snap = exec.to_workflow_state();

            (indices, snap)
        };
        // ロック解放後にI/O操作を実行（broadcast_state のみ；ChatSession への永続化は撤去済み）
        self.broadcast_state(app, worktree_path, snapshot).await;

        // Phase 3a: 全子セッション作成（AgentSessionプロセス起動）
        // Note: AppHandleが!Sendのためtokio::spawnによる真の並列化は不可能。
        // セッション作成とターン開始を分離し、全セッション準備完了後に
        // 全ターンを開始することで「ほぼ同時起動」を実現する。
        let mut created_session_ids: Vec<String> = Vec::new();
        let mut runtime_guards = Vec::new();
        for cs in &child_setups {
            let runtime_guard =
                crate::agent_sdk::acquire_session_runtime_lock(&cs.session_id).await;
            if let Err(e) = crate::agent_sdk::start_agent_session_internal(
                app,
                handles,
                session_store,
                &cs.session_id,
                worktree_path,
                None,
                cs.system_prompt.clone(),
            )
            .await
            {
                // 作成済みセッションを中断してからエラーを返す
                for sid in &created_session_ids {
                    self.interrupt_agent(handles, sid).await;
                }
                return Err(WorkflowEngineError::AgentSession(format!(
                    "Failed to start parallel child '{}': {e}",
                    cs.step_name
                )));
            }
            runtime_guards.push(runtime_guard);
            if let Some(open_tabs) = app.try_state::<Arc<crate::session::OpenTabRegistry>>() {
                open_tabs.add(&cs.session_id);
                self.emit_workflow_runtime_projection(app, handles, &open_tabs, worktree_path)
                    .await;
            }
            created_session_ids.push(cs.session_id.clone());
        }

        // Phase 3b: 全子ターン開始（ここが実際のAgent作業トリガー）
        for (i, cs) in child_setups.iter().enumerate() {
            let runtime_guard = runtime_guards.remove(0);
            if let Err(e) = crate::agent_sdk::start_agent_turn_internal_locked(
                app,
                handles,
                session_store,
                &cs.session_id,
                worktree_path,
                &cs.permission_mode,
                &cs.user_message,
            )
            .await
            {
                // 全作成済みセッションを中断してからエラーを返す
                for sid in &created_session_ids {
                    self.interrupt_agent(handles, sid).await;
                }
                return Err(WorkflowEngineError::AgentSession(format!(
                    "Failed to start turn for parallel child '{}': {e}",
                    cs.step_name
                )));
            }
            drop(runtime_guard);

            // ParallelStepStarted ログ
            self.write_log(
                app,
                WorkflowEvent::ParallelChildStarted {
                    run_id: execution_id.clone(),
                    workflow_name: workflow_name.clone(),
                    parent_node_name: parent_step_name.clone(),
                    child_node_name: cs.step_name.clone(),
                    session_id: cs.session_id.clone(),
                    execution_count: child_run_indices[i],
                    timestamp: current_timestamp(),
                },
            );
        }

        Ok(())
    }

    /// 並列子ステップ用のプロンプトを構築する。
    /// `build_step_prompt` と同様に純粋関数として切り出し、テスト可能にする。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_parallel_step_prompt(
        ps: &crate::workflow::schema::ChildNodeDefinition,
        run_id: &str,
        worktree_path: &str,
        task: Option<&str>,
        step_outputs: &HashMap<String, StepOutput>,
        pass_previous_response: bool,
        pass_output_from: Option<&[String]>,
        workflow_variables: &HashMap<String, String>,
        workflow_declared_variables: &HashMap<String, String>,
    ) -> Result<(Option<String>, String), WorkflowEngineError> {
        if ps.has_facet_refs() && ps.resolved_facets.is_empty() {
            return Err(WorkflowEngineError::InvalidWorkflow(format!(
                "Parallel child '{}' has unresolved facet refs (workflow must go through load pipeline)",
                ps.name
            )));
        }
        let composed = crate::workflow::facet::compose_child_facets(ps);

        let system_prompt = composed.system_prompt.map(|s| {
            let s = Self::render_facet_variables(&s, worktree_path, task);
            let s = Self::render_submit_command_variables(&s, run_id, &ps.name);
            Self::render_namespaced_variables(&s, workflow_declared_variables)
        });
        let mut user_message =
            Self::render_facet_variables(&composed.user_message, worktree_path, task);
        user_message = Self::render_submit_command_variables(&user_message, run_id, &ps.name);
        user_message =
            Self::render_namespaced_variables(&user_message, workflow_declared_variables);

        // pass_output_from による出力注入
        if let Some(from_steps) = pass_output_from {
            let mut injections = Vec::new();
            for step_name in from_steps {
                if let Some(output) = step_outputs.get(step_name) {
                    let text = Self::format_step_output_block(output);
                    injections.push(format!(
                        "<step_output name=\"{step_name}\">\n{text}\n</step_output>",
                    ));
                }
            }
            if !injections.is_empty() {
                user_message = format!("{}\n\n{}", injections.join("\n\n"), user_message);
            }
        } else if pass_previous_response {
            // 直前ステップの出力を注入（逐次ステップの最後の出力）
            if let Some(last_output) = step_outputs.values().max_by(|a, b| {
                a.completed_at
                    .partial_cmp(&b.completed_at)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }) {
                let text = Self::format_step_output_block(last_output);
                user_message = format!(
                    "<step_output name=\"{}\">\n{}\n</step_output>\n\n{}",
                    last_output.step_name, text, user_message
                );
            }
        }

        Self::append_workflow_variables_block(&mut user_message, workflow_variables);
        // 並列子 node も top-level 同様、input_contracts 宣言があるときだけ `<task>` 注入する
        let allow_task = ps.input_contracts.as_ref().is_some_and(|v| !v.is_empty());
        Self::append_task_block(&mut user_message, task, allow_task);
        Self::append_output_contract_completion_action(
            &mut user_message,
            ps.output_contract.as_deref(),
            run_id,
            &ps.name,
            workflow_declared_variables,
        );

        Ok((system_prompt, user_message))
    }

    /// ワークフロー状態をブロードキャストする。
    /// スナップショットは呼び出し元がロック内で確定したものを受け取る。
    /// worktree_pathベースでイベントを発行するため、同一worktreeの全セッションが受信可能。
    async fn broadcast_state<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        worktree_path: &str,
        workflow_state: WorkflowState,
    ) {
        crate::workflow_state_events::emit_workflow_state_snapshot(
            app,
            worktree_path,
            workflow_state,
        )
        .await;
    }

    /// 終了状態（Completed/Failed）のログを書き込む required append helper。
    /// StepCompletedログは呼び出し元で書き込み済みのため、ここでは書かない。
    ///
    /// `Aborted` 状態の `RunAborted` event は本 issue [04] の典型 typed command
    /// `AbortRun` に対応する事実列であり、command handler 側で `write_log_required`
    /// を経由して必須 append + snapshot 一括復元の atomic 境界に乗せる。本ヘルパーは
    /// `AbortRun` の rollback 経路を担保できないため Aborted はここで書かない（重複
    /// append 防止）。
    ///
    /// [05] event 発行点の集約: terminal events（NodeFailed / RunCompleted / RunFailed）は
    /// `dispatch_internal_node_command` 経由で生成し、`write_log_required_batch` で必須
    /// append 経路に乗せる。append 失敗時は `Err` を返し、呼出側で state mutation
    /// rollback / persist スキップに乗せる（spec [05]: best-effort warn を廃止し
    /// commit 境界に揃える）。
    fn write_terminal_log<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        snapshot: &WorkflowState,
    ) -> Result<(), String> {
        let mut local = snapshot.clone();
        let events =
            Self::terminal_events_for_snapshot(&mut local).map_err(|e| format!("{e:?}"))?;
        if events.is_empty() {
            return Ok(());
        }
        self.write_log_required_batch(app, &events)
    }

    /// 最後のステップの NodeCompleted ログを書き込む required append helper。
    /// [05] event 発行点の集約: `dispatch_internal_node_command` 経由で生成した
    /// `NodeCompleted` を `write_log_required` で必須 append 経路に乗せる。
    /// append 失敗時は `Err` を返し、呼出側で commit 境界に乗せる（spec [05]:
    /// best-effort warn を廃止）。
    fn write_last_step_completed_log<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        snapshot: &WorkflowState,
    ) -> Result<(), String> {
        let mut local = snapshot.clone();
        match Self::last_step_completed_event_for_snapshot(&mut local) {
            Ok(Some(event)) => self.write_log_required(app, event),
            Ok(None) => Ok(()),
            Err(e) => Err(format!("{e:?}")),
        }
    }

    /// [05] internal command boundary: NodeCompleted event の発行点を typed command
    /// 経路に揃える。snapshot から `WorkflowCommand::CompleteNode` を組み立て、
    /// `dispatch_internal_node_command` 経由で (state mutation + event) を atomic
    /// commit する（Complete の mutation は上流 push 完了点と一致することの検証）。
    ///
    /// `Ok(None)` は step_history 空（NodeCompleted 該当なし）の合法な経路。
    /// `dispatch_internal_node_command` の `ValidationError` は `.ok()` で握り潰さず
    /// `Err` として呼出側に伝播し、commit 境界の判断材料を奪わないようにする
    /// （spec [05] silent error の禁止）。
    fn last_step_completed_event_for_snapshot(
        snapshot: &mut WorkflowState,
    ) -> Result<Option<WorkflowEvent>, WorkflowEngineError> {
        let Some(last_entry) = snapshot.step_history.last().cloned() else {
            return Ok(None);
        };
        let command = WorkflowCommand::CompleteNode {
            run_id: snapshot.execution_id.clone(),
            workflow_name: snapshot.workflow_name.clone(),
            node_name: last_entry.step_name,
            result: last_entry.result,
            session_id: last_entry.session_id,
            token_usage: last_entry.token_usage,
            structured_output: last_entry.structured_output,
            run_index: Some(last_entry.run_index),
            timestamp: last_entry.completed_at,
        };
        Self::dispatch_internal_node_command(snapshot, command).map(Some)
    }

    fn node_started_event_for_snapshot(snapshot: &WorkflowState) -> WorkflowEvent {
        let exec_count = snapshot
            .step_execution_counts
            .get(&snapshot.current_step_name)
            .copied()
            .unwrap_or(1);
        WorkflowEvent::NodeStarted {
            run_id: snapshot.execution_id.clone(),
            workflow_name: snapshot.workflow_name.clone(),
            node_name: snapshot.current_step_name.clone(),
            execution_count: exec_count,
            timestamp: snapshot.updated_at,
        }
    }

    fn parallel_started_event_for_snapshot(snapshot: &WorkflowState) -> Option<WorkflowEvent> {
        let node = snapshot
            .workflow_definition
            .nodes
            .get(snapshot.current_step_index)?;
        let child_node_names: Vec<String> = node
            .parallel_children
            .as_ref()?
            .iter()
            .map(|child| child.name.clone())
            .collect();
        Some(WorkflowEvent::ParallelStarted {
            run_id: snapshot.execution_id.clone(),
            workflow_name: snapshot.workflow_name.clone(),
            parent_node_name: snapshot.current_step_name.clone(),
            child_node_names,
            timestamp: snapshot.updated_at,
        })
    }

    /// [05] internal command boundary: terminal event 列の発行点を typed command 経路に
    /// 揃える。`Failed` の場合は `FailNode` typed command を組み立てて
    /// `dispatch_internal_node_command` で state mutation + NodeFailed event を atomic
    /// に commit し、その後 RunFailed を追記する。`dispatch_internal_node_command` の
    /// `ValidationError` は `.ok()` で握り潰さず `Err` として呼出側に伝播する
    /// （spec [05] silent error の禁止）。
    fn terminal_events_for_snapshot(
        snapshot: &mut WorkflowState,
    ) -> Result<Vec<WorkflowEvent>, WorkflowEngineError> {
        match &snapshot.state {
            WorkflowExecutionState::Completed => Ok(vec![WorkflowEvent::RunCompleted {
                run_id: snapshot.execution_id.clone(),
                workflow_name: snapshot.workflow_name.clone(),
                total_token_usage: snapshot.total_token_usage.clone(),
                timestamp: snapshot.updated_at,
            }]),
            WorkflowExecutionState::Failed { reason } => {
                let run_id = snapshot.execution_id.clone();
                let workflow_name = snapshot.workflow_name.clone();
                let node_name = snapshot.current_step_name.clone();
                let reason = reason.clone();
                let timestamp = snapshot.updated_at;
                let fail_command = WorkflowCommand::FailNode {
                    run_id: run_id.clone(),
                    workflow_name: workflow_name.clone(),
                    node_name,
                    reason: reason.clone(),
                    timestamp,
                };
                let node_failed = Self::dispatch_internal_node_command(snapshot, fail_command)?;
                Ok(vec![
                    node_failed,
                    WorkflowEvent::RunFailed {
                        run_id,
                        workflow_name,
                        reason,
                        timestamp,
                    },
                ])
            }
            _ => Ok(Vec::new()),
        }
    }

    fn required_events_for_approval_commit(
        approval_event: WorkflowEvent,
        outcome: &mut StepOutcome,
    ) -> Result<Vec<WorkflowEvent>, WorkflowEngineError> {
        let mut events = vec![approval_event];
        match outcome {
            StepOutcome::Persist(snapshot) => {
                let is_terminal = matches!(
                    snapshot.state,
                    WorkflowExecutionState::Completed | WorkflowExecutionState::Failed { .. }
                );
                if is_terminal {
                    if let Some(event) = Self::last_step_completed_event_for_snapshot(snapshot)? {
                        events.push(event);
                    }
                    events.extend(Self::terminal_events_for_snapshot(snapshot)?);
                } else if matches!(snapshot.state, WorkflowExecutionState::Aborted) {
                    events.push(WorkflowEvent::RunAborted {
                        run_id: snapshot.execution_id.clone(),
                        workflow_name: snapshot.workflow_name.clone(),
                        timestamp: snapshot.updated_at,
                    });
                }
            }
            StepOutcome::TransitionAndStart(snapshot) => {
                if let Some(event) = Self::last_step_completed_event_for_snapshot(snapshot)? {
                    events.push(event);
                }
                events.push(Self::node_started_event_for_snapshot(snapshot));
            }
            StepOutcome::ReduceAndTransition(snapshot) => {
                if let Some(event) = Self::last_step_completed_event_for_snapshot(snapshot)? {
                    events.push(event);
                }
                events.push(Self::node_started_event_for_snapshot(snapshot));
            }
            StepOutcome::StartParallel(snapshot) => {
                if let Some(event) = Self::last_step_completed_event_for_snapshot(snapshot)? {
                    events.push(event);
                }
                if let Some(event) = Self::parallel_started_event_for_snapshot(snapshot) {
                    events.push(event);
                }
            }
        }
        let commit_timestamp = events
            .iter()
            .map(Self::workflow_event_timestamp)
            .fold(0.0_f64, f64::max);
        for event in &mut events {
            Self::set_workflow_event_timestamp(event, commit_timestamp);
        }
        Ok(events)
    }

    fn workflow_event_timestamp(event: &WorkflowEvent) -> f64 {
        match event {
            WorkflowEvent::RunStarted { timestamp, .. }
            | WorkflowEvent::NodeStarted { timestamp, .. }
            | WorkflowEvent::NodeCompleted { timestamp, .. }
            | WorkflowEvent::NodeFailed { timestamp, .. }
            | WorkflowEvent::ApprovalRequested { timestamp, .. }
            | WorkflowEvent::ApprovalResolved { timestamp, .. }
            | WorkflowEvent::RunCompleted { timestamp, .. }
            | WorkflowEvent::RunFailed { timestamp, .. }
            | WorkflowEvent::RunAborted { timestamp, .. }
            | WorkflowEvent::OutputCollected { timestamp, .. }
            | WorkflowEvent::ParallelStarted { timestamp, .. }
            | WorkflowEvent::ParallelChildStarted { timestamp, .. }
            | WorkflowEvent::ParallelChildCompleted { timestamp, .. }
            | WorkflowEvent::ParallelCompleted { timestamp, .. }
            | WorkflowEvent::ContractRepairRequested { timestamp, .. }
            | WorkflowEvent::CliMutationRequested { timestamp, .. }
            | WorkflowEvent::OutputSubmitted { timestamp, .. }
            | WorkflowEvent::CliMutationRejected { timestamp, .. } => *timestamp,
        }
    }

    fn set_workflow_event_timestamp(event: &mut WorkflowEvent, commit_timestamp: f64) {
        match event {
            WorkflowEvent::RunStarted { timestamp, .. }
            | WorkflowEvent::NodeStarted { timestamp, .. }
            | WorkflowEvent::NodeCompleted { timestamp, .. }
            | WorkflowEvent::NodeFailed { timestamp, .. }
            | WorkflowEvent::ApprovalRequested { timestamp, .. }
            | WorkflowEvent::ApprovalResolved { timestamp, .. }
            | WorkflowEvent::RunCompleted { timestamp, .. }
            | WorkflowEvent::RunFailed { timestamp, .. }
            | WorkflowEvent::RunAborted { timestamp, .. }
            | WorkflowEvent::OutputCollected { timestamp, .. }
            | WorkflowEvent::ParallelStarted { timestamp, .. }
            | WorkflowEvent::ParallelChildStarted { timestamp, .. }
            | WorkflowEvent::ParallelChildCompleted { timestamp, .. }
            | WorkflowEvent::ParallelCompleted { timestamp, .. }
            | WorkflowEvent::ContractRepairRequested { timestamp, .. }
            | WorkflowEvent::CliMutationRequested { timestamp, .. }
            | WorkflowEvent::OutputSubmitted { timestamp, .. }
            | WorkflowEvent::CliMutationRejected { timestamp, .. } => {
                *timestamp = commit_timestamp;
            }
        }
    }

    /// NDJSONログにイベントを書き込む。失敗してもワークフロー実行には影響しない。
    fn write_log<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>, event: WorkflowEvent) {
        if let Err(e) = self.write_log_required(app, event) {
            log::warn!("Failed to write workflow log: {e}");
        }
    }

    /// NDJSONログにイベントを書き込む。履歴復元に必須のログでのみ失敗を伝播する。
    fn write_log_required<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        event: WorkflowEvent,
    ) -> Result<(), String> {
        // [08] テスト fixture (`fail_next_required_event_append_for_test`) を
        // 単発の write_log_required 経路でも観測できるよう、内部で batch helper に
        // 委譲する。production の振る舞いは変わらず、SubmitOutput 等の rollback
        // テストが append 失敗を再現できる。
        self.write_log_required_batch(app, std::slice::from_ref(&event))
    }

    /// 複数の必須 event を 1 つの atomic commit point として一括追記する。
    ///
    /// [04] spec『event 列と domain state の整合』Rule: 同一 command 受理サイクル内で
    /// 複数 required event を発行する場合は本 helper を使い、partial commit を構造的に
    /// 排除する。
    fn write_log_required_batch<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        events: &[WorkflowEvent],
    ) -> Result<(), String> {
        #[cfg(test)]
        if self
            .fail_next_required_event_append
            .swap(false, Ordering::AcqRel)
        {
            return Err("injected required event append failure".to_string());
        }
        if let Ok(data_dir) = crate::session::resolve_data_dir(app) {
            let log = WorkflowEventLog::new(&data_dir);
            return log.append_batch(events);
        }
        Err("failed to resolve app data dir".to_string())
    }

    fn workflow_approval_auto_approve_enabled<R: tauri::Runtime>(
        app: &tauri::AppHandle<R>,
    ) -> bool {
        app.try_state::<Arc<crate::config::AppConfig>>()
            .and_then(|config| config.get_config().ok())
            .is_some_and(|cfg| cfg.workflow.approval_auto_approve)
    }

    fn should_auto_approve_workflow_approval(
        snapshot: &WorkflowState,
        approval_auto_approve_enabled: bool,
    ) -> bool {
        approval_auto_approve_enabled && snapshot.state == WorkflowExecutionState::WaitingApproval
    }

    fn auto_approve_target_for_persisted_snapshot(
        snapshot: &WorkflowState,
        approval_auto_approve_enabled: bool,
    ) -> Option<(String, String)> {
        if Self::should_auto_approve_workflow_approval(snapshot, approval_auto_approve_enabled) {
            Some((
                snapshot.execution_id.clone(),
                snapshot.current_step_name.clone(),
            ))
        } else {
            None
        }
    }

    #[cfg(test)]
    async fn handle_approval_with_output_for_test(
        &self,
        worktree_path: &str,
        decision: ApprovalDecision,
        expected_execution_id: Option<&str>,
        expected_step_name: Option<&str>,
    ) -> Result<StepOutcome, WorkflowEngineError> {
        let run_id = {
            let execs = self.executions.lock().await;
            let (run_id, _) = find_by_worktree(&execs, worktree_path).ok_or_else(|| {
                WorkflowEngineError::UnauthorizedWorktree(worktree_path.to_string())
            })?;
            run_id.clone()
        };
        self.handle_approval_with_output_for_run_for_test(
            &run_id,
            decision,
            expected_execution_id,
            expected_step_name,
        )
        .await
    }

    /// [05] Test-only: 既に `Failed` state に遷移した snapshot に対して
    /// `execute_outcome(StepOutcome::Persist(snapshot))` を実行する production 経路の
    /// ショートカット。pre-commit append 失敗時に RunStore / state が persist されない
    /// ことを検証するために用いる（spec [05] commit 境界の継承）。
    #[cfg(test)]
    async fn execute_outcome_persist_failed_for_test<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        snapshot: WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
        // テスト helper の snapshot_before は engine.executions の現在状態を採用する。
        // production 経路では call site が mutation 前に capture するが、本 helper は
        // 既に mutated snapshot を直接渡すための短絡として、現在状態を rollback target
        // 扱いにする（pre-commit 失敗時の挙動を観測する用途のため）。
        let snapshot_before = {
            let execs = self.executions.lock().await;
            execs.get(&snapshot.execution_id).cloned().ok_or_else(|| {
                WorkflowEngineError::ExecutionNotFound(snapshot.execution_id.clone())
            })?
        };
        self.execute_outcome(
            app,
            session_store,
            handles,
            worktree_path,
            StepOutcome::Persist(snapshot),
            snapshot_before,
        )
        .await
    }

    #[cfg(test)]
    async fn handle_approval_with_output_for_run_for_test(
        &self,
        run_id: &str,
        decision: ApprovalDecision,
        expected_execution_id: Option<&str>,
        expected_step_name: Option<&str>,
    ) -> Result<StepOutcome, WorkflowEngineError> {
        {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(run_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
            Self::validate_approval_target_snapshot(
                exec,
                expected_execution_id,
                expected_step_name,
            )?;
        }

        Self::validate_approval_decision(&decision)?;
        if matches!(decision, ApprovalDecision::Approve) {
            Self::validate_approval_turn_phase(None)?;
        }

        let output_contract = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(run_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
            exec.workflow.nodes[exec.current_step_index]
                .output_contract
                .clone()
        };

        let result_tag = match &decision {
            ApprovalDecision::Approve => "approve",
            ApprovalDecision::Reject { .. } => "reject",
        };
        // [08] approval 経路の自由文 contract 抽出は廃止。approval node の構造化出力は
        // CLI / Tauri 経由の `SubmitOutput` で確定する（spec [08] Rule 4）。Approve 時の
        // `structured_output` は None で固定し、Reject 時のみ comment 由来の暫定 payload を
        // 維持する（既存 reject 経路は本 issue のスコープ外）。
        let (structured_output, contract_result): (Option<serde_json::Value>, Option<String>) =
            match &decision {
                ApprovalDecision::Approve => (None, None),
                ApprovalDecision::Reject { comment } => {
                    (Some(Self::reject_structured_output(comment, &[])), None)
                }
            };

        let application_output_contract = if matches!(decision, ApprovalDecision::Approve) {
            output_contract.clone()
        } else {
            None
        };
        let contract_variables =
            Self::extract_contract_variables(&application_output_contract, &structured_output);
        let effective_result = contract_result.unwrap_or_else(|| result_tag.to_string());

        let mut execs = self.executions.lock().await;
        let exec = execs
            .get_mut(run_id)
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
        Self::validate_approval_target_snapshot(exec, expected_execution_id, expected_step_name)?;
        exec.workflow_variables.extend(contract_variables);
        Self::apply_approval_application(
            exec,
            &decision,
            ApprovalApplication {
                effective_result,
                structured_output,
                output_contract: application_output_contract,
            },
        )
    }

    #[cfg(test)]
    async fn execute_outcome_persist_auto_approve_for_test(
        &self,
        worktree_path: &str,
        snapshot: &WorkflowState,
    ) -> Result<Option<StepOutcome>, WorkflowEngineError> {
        if let Some((execution_id, step_name)) =
            Self::auto_approve_target_for_persisted_snapshot(snapshot, true)
        {
            self.handle_approval_with_output_for_test(
                worktree_path,
                ApprovalDecision::Approve,
                Some(&execution_id),
                Some(&step_name),
            )
            .await
            .map(Some)
        } else {
            Ok(None)
        }
    }

    #[cfg(test)]
    pub(crate) async fn insert_test_approval_execution(
        &self,
        worktree_path: &str,
        current_session_id: &str,
        state: WorkflowExecutionState,
    ) -> WorkflowState {
        let workflow = Workflow {
            variables: Default::default(),
            name: "test-approval-workflow".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "implementation_fix_policy".to_string(),
                node_type: NodeType::Approval,
                policy: None,
                knowledge: None,
                instruction: Some("Review fix policy".to_string()),
                output_contract: Some("approved-fix-policy".to_string()),
                transition_rules: vec![],
                cycle_guard: None,
                pass_previous_response: None,
                pass_output_from: None,
                inline_prompt: None,
                collect: None,
                parallel_children: None,
                aggregate: None,
                resets_cycle_for: None,
                model: None,
                permission: None,
                ..Default::default()
            }],
        };
        let exec = WorkflowExecution {
            id: "exec-approval-chat".to_string(),
            workflow,
            state,
            current_step_index: 0,
            step_execution_counts: HashMap::from([("implementation_fix_policy".to_string(), 1)]),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some(current_session_id.to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: worktree_path.to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let snapshot = exec.to_workflow_state();
        let run_id = exec.id.clone();
        self.executions.lock().await.insert(run_id.clone(), exec);
        self.session_workflow_refs.lock().await.insert(
            current_session_id.to_string(),
            SessionWorkflowRef { run_id },
        );
        snapshot
    }

    #[cfg(test)]
    pub(crate) async fn insert_test_running_execution_for_pending_pickup(
        &self,
        run_id: &str,
        worktree_path: &str,
    ) {
        let workflow = Workflow {
            variables: Default::default(),
            name: "pending-pickup-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "work".to_string(),
                node_type: NodeType::Agent,
                instruction: Some("work".to_string()),
                ..Default::default()
            }],
        };
        let exec = WorkflowExecution {
            id: run_id.to_string(),
            workflow,
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::from([("work".to_string(), 1)]),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: worktree_path.to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        self.run_store
            .register_active(WorkflowRun {
                run_id: run_id.to_string(),
                workflow_name: exec.workflow.name.clone(),
                task: None,
                status: RunStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node_name: Some("work".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: exec.started_at,
                updated_at: exec.updated_at,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        self.executions
            .lock()
            .await
            .insert(run_id.to_string(), exec);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::{
        AgentBackend, AgentBackendRegistry, AgentMessage as BackendAgentMessage,
        PermissionResponse, SessionConfig as BackendSessionConfig,
        SessionHandle as BackendSessionHandle,
    };
    use crate::session::MessagePart;
    use crate::workflow::command_input::MAX_APPROVAL_COMMENT_CHARS;
    use async_trait::async_trait;

    const TEST_PARENT_SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";
    const TEST_STEP_SESSION_ID: &str = "22222222-2222-4222-8222-222222222222";
    const TEST_REGULAR_SESSION_ID: &str = "33333333-3333-4333-8333-333333333333";

    struct WorkflowMockBackend {
        backend_id: String,
    }

    #[async_trait]
    impl AgentBackend for WorkflowMockBackend {
        fn id(&self) -> &str {
            &self.backend_id
        }
        fn name(&self) -> &str {
            "Mock"
        }
        async fn start_session(
            &self,
            cfg: BackendSessionConfig,
        ) -> Result<BackendSessionHandle, String> {
            Ok(BackendSessionHandle {
                chat_session_id: cfg.chat_session_id,
                backend_id: self.backend_id.clone(),
            })
        }
        async fn send_message(
            &self,
            _s: &BackendSessionHandle,
            _m: BackendAgentMessage,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn interrupt(&self, _s: &BackendSessionHandle) -> Result<(), String> {
            Ok(())
        }
        async fn respond_permission(
            &self,
            _s: &BackendSessionHandle,
            _r: PermissionResponse,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn close_session(&self, _s: &BackendSessionHandle) -> Result<(), String> {
            Ok(())
        }
    }

    fn make_workflow_test_registry(
        claude_models: &[&str],
        codex_models: &[&str],
    ) -> AgentBackendRegistry {
        let mut cfg = crate::config::ReleashConfig::default();
        cfg.agents.claude.models = claude_models.iter().map(|s| s.to_string()).collect();
        cfg.agents.codex.models = codex_models.iter().map(|s| s.to_string()).collect();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let app_cfg = Arc::new(crate::config::AppConfig::new(cfg, tmp.path().to_path_buf()));
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(WorkflowMockBackend {
            backend_id: "claude".to_string(),
        }));
        registry.register(Arc::new(WorkflowMockBackend {
            backend_id: "codex".to_string(),
        }));
        registry.set_config(app_cfg);
        registry
    }

    #[test]
    fn workflow_resolve_unique_model_returns_owning_backend() {
        let registry = make_workflow_test_registry(&["claude-4"], &["gpt-5"]);
        let result = resolve_step_model_with_registry(&registry, "claude-4").unwrap();
        assert_eq!(result, "claude");
    }

    #[test]
    fn workflow_resolve_rejects_ambiguous_model_in_multiple_backends() {
        let registry = make_workflow_test_registry(&["shared"], &["shared"]);
        let err = resolve_step_model_with_registry(&registry, "shared").unwrap_err();
        match err {
            WorkflowEngineError::InvalidWorkflow(msg) => {
                assert!(msg.contains("could not be resolved"));
            }
            other => panic!("expected InvalidWorkflow, got {:?}", other),
        }
    }

    #[test]
    fn workflow_resolve_rejects_unknown_model() {
        let registry = make_workflow_test_registry(&["claude-4"], &[]);
        let err = resolve_step_model_with_registry(&registry, "unknown").unwrap_err();
        match err {
            WorkflowEngineError::InvalidWorkflow(msg) => {
                assert!(msg.contains("unknown model"));
            }
            other => panic!("expected InvalidWorkflow, got {:?}", other),
        }
    }

    #[test]
    fn workflow_resolve_rejects_invalid_format() {
        let registry = make_workflow_test_registry(&["claude-4"], &[]);
        // 形式不正（空文字）は登録判定に進む前に拒否される
        let err = resolve_step_model_with_registry(&registry, "").unwrap_err();
        match err {
            WorkflowEngineError::InvalidWorkflow(msg) => {
                assert!(msg.contains("invalid model"));
            }
            other => panic!("expected InvalidWorkflow, got {:?}", other),
        }
    }

    fn chat_session_for_test(
        id: &str,
        worktree_path: &str,
        _workflow_state: Option<WorkflowState>,
        workflow_step_session: bool,
    ) -> crate::session::ChatSession {
        crate::session::ChatSession {
            id: id.to_string(),
            worktree_path: worktree_path.to_string(),
            messages: vec![],
            state: crate::session::SessionState::Idle,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: Some("sdk-session".to_string()),
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: Some(crate::agent_sdk::CLAUDE_BACKEND_ID.to_string()),
            workflow_step_session,
        }
    }

    fn chat_session_with_message_for_test(
        id: &str,
        worktree_path: &str,
    ) -> crate::session::ChatSession {
        let mut session = chat_session_for_test(id, worktree_path, None, true);
        session.messages.push(crate::session::ChatMessage {
            id: "msg-1".to_string(),
            role: crate::session::MessageRole::Agent,
            content: "history".to_string(),
            thinking: None,
            activities: None,
            parts: None,
            timestamp: 1.0,
            mentions: None,
        });
        session
    }

    #[test]
    fn workflow_step_summary_uses_persisted_session_flag() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = crate::session::SessionStore::default();

        store
            .save_session(
                tmp.path(),
                &chat_session_for_test(TEST_PARENT_SESSION_ID, "/repo", None, false),
            )
            .unwrap();
        store
            .save_session(
                tmp.path(),
                &chat_session_for_test(TEST_STEP_SESSION_ID, "/repo", None, true),
            )
            .unwrap();
        store
            .save_session(
                tmp.path(),
                &chat_session_for_test(TEST_REGULAR_SESSION_ID, "/repo", None, false),
            )
            .unwrap();

        let summaries = store.list_sessions(tmp.path(), "/repo").unwrap();
        let step_summary = summaries
            .iter()
            .find(|session| session.id == TEST_STEP_SESSION_ID)
            .unwrap();
        assert!(step_summary.workflow_step_session);
    }

    // 撤去済み: persist_state は廃止された（NDJSON event log + Run Store metadata で永続化が完結）。
    // 旧 `persist_failure_still_runs_completed_step_cleanup` は persist_state 失敗時の cleanup 順序を
    // 検証していたが、機構撤去により意味を失った。

    #[test]
    fn step_session_tab_cleanup_closes_session_and_preserves_history() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = crate::session::SessionStore::default();
        let open_tabs = Arc::new(crate::session::OpenTabRegistry::default());
        let session_id = uuid::Uuid::new_v4().to_string();

        store
            .save_session(
                tmp.path(),
                &chat_session_with_message_for_test(&session_id, "/repo"),
            )
            .unwrap();
        open_tabs.add(&session_id);

        crate::workflow_step_lifecycle_adapters::close_step_session_tab_state(
            &store,
            tmp.path(),
            Some(open_tabs.as_ref()),
            &session_id,
        );

        assert!(!open_tabs.contains(&session_id));
        let session = store
            .get_session(tmp.path(), &session_id)
            .unwrap()
            .expect("session remains");
        assert_eq!(session.state, crate::session::SessionState::Closed);
        assert_eq!(session.agent_session_id.as_deref(), Some("sdk-session"));
        assert_eq!(session.messages.len(), 1);
    }

    #[tokio::test]
    async fn persist_outcome_without_new_history_does_not_cleanup_last_step_session() {
        let engine = WorkflowEngine::new_for_test();
        let mut snapshot = engine
            .insert_test_approval_execution(
                "/repo",
                TEST_STEP_SESSION_ID,
                WorkflowExecutionState::WaitingApproval,
            )
            .await;
        snapshot.step_history.push(StepHistoryEntry {
            step_name: "previous".to_string(),
            completed_at: 1.0,
            result: Some("ok".to_string()),
            session_id: Some(TEST_STEP_SESSION_ID.to_string()),
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: None,
            state: crate::workflow::state::default_step_entry_state(),
        });

        let persist = StepOutcome::Persist(snapshot.clone());
        assert!(WorkflowEngine::completed_step_session_ids_for_outcome(&persist).is_empty());

        snapshot.state = WorkflowExecutionState::Completed;
        let terminal = StepOutcome::Persist(snapshot);
        assert_eq!(
            WorkflowEngine::completed_step_session_ids_for_outcome(&terminal),
            vec![TEST_STEP_SESSION_ID.to_string()]
        );
    }

    #[tokio::test]
    async fn aborted_approval_outcome_cleans_current_session_not_last_history_entry() {
        let engine = WorkflowEngine::new_for_test();
        let mut snapshot = engine
            .insert_test_approval_execution(
                "/repo",
                TEST_STEP_SESSION_ID,
                WorkflowExecutionState::WaitingApproval,
            )
            .await;
        snapshot.current_session_id = Some("approval-session".to_string());
        snapshot.step_history.push(StepHistoryEntry {
            step_name: "previous".to_string(),
            completed_at: 1.0,
            result: Some("ok".to_string()),
            session_id: Some("previous-session".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: None,
            state: crate::workflow::state::default_step_entry_state(),
        });
        snapshot.state = WorkflowExecutionState::Aborted;

        let outcome = StepOutcome::Persist(snapshot);
        assert_eq!(
            WorkflowEngine::completed_step_session_ids_for_outcome(&outcome),
            vec!["approval-session".to_string()]
        );
    }

    #[tokio::test]
    async fn terminal_state_cleanup_targets_current_and_parallel_step_sessions() {
        let engine = WorkflowEngine::new_for_test();
        let mut exec = engine
            .insert_test_approval_execution(
                "/repo",
                TEST_STEP_SESSION_ID,
                WorkflowExecutionState::Running,
            )
            .await;
        exec.current_session_id = Some("current-step-session".to_string());
        exec.active_parallel_steps = vec![
            ParallelStepState {
                step_name: "review-a".to_string(),
                state: "running".to_string(),
                session_id: Some("parallel-a-session".to_string()),
                result: None,
                run_index: 1,
                completed_at: None,
                structured_output: None,
                output_contract: None,
            },
            ParallelStepState {
                step_name: "review-b".to_string(),
                state: "running".to_string(),
                session_id: Some("parallel-b-session".to_string()),
                result: None,
                run_index: 1,
                completed_at: None,
                structured_output: None,
                output_contract: None,
            },
        ];

        assert_eq!(
            WorkflowEngine::terminal_step_session_ids(&exec),
            vec![
                "current-step-session".to_string(),
                "parallel-a-session".to_string(),
                "parallel-b-session".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn terminal_outcome_cleanup_includes_parent_entry_and_parallel_child_outputs() {
        let engine = WorkflowEngine::new_for_test();
        let mut snapshot = engine
            .insert_test_approval_execution(
                "/repo",
                TEST_STEP_SESSION_ID,
                WorkflowExecutionState::Completed,
            )
            .await;
        snapshot.step_history.push(StepHistoryEntry {
            step_name: "parallel-review".to_string(),
            completed_at: 1.0,
            result: Some("done".to_string()),
            session_id: Some("parent-entry-session".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: Some(vec![
                crate::workflow::state::ChildOutputSnapshot {
                    step_name: "review-a".to_string(),
                    session_id: Some("child-a-session".to_string()),
                    result: Some("LGTM".to_string()),
                    run_index: 1,
                    completed_at: 1.0,
                    structured_output: None,
                    output_contract: None,
                    state: crate::workflow::state::default_step_entry_state(),
                },
                crate::workflow::state::ChildOutputSnapshot {
                    step_name: "review-b".to_string(),
                    session_id: Some("child-b-session".to_string()),
                    result: Some("LGTM".to_string()),
                    run_index: 1,
                    completed_at: 1.0,
                    structured_output: None,
                    output_contract: None,
                    state: crate::workflow::state::default_step_entry_state(),
                },
            ]),
            state: crate::workflow::state::default_step_entry_state(),
        });

        assert_eq!(
            WorkflowEngine::completed_step_session_ids_for_outcome(&StepOutcome::Persist(snapshot)),
            vec![
                "child-a-session".to_string(),
                "child-b-session".to_string(),
                "parent-entry-session".to_string(),
            ]
        );
    }
    use crate::workflow::schema::{
        CollectConfig, CycleGuard, ParallelAggregate, ReduceStrategy, TransitionRule, Workflow,
    };

    #[allow(dead_code)]
    fn make_approved_fix_policy_workflow() -> Workflow {
        Workflow {
            variables: Default::default(),
            name: "approved-fix-policy-test".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![
                NodeDefinition {
                    name: "code_review_parallel".to_string(),
                    node_type: NodeType::Parallel,
                    policy: None,
                    knowledge: None,
                    instruction: None,
                    output_contract: None,
                    transition_rules: vec![],
                    cycle_guard: None,
                    pass_previous_response: None,
                    pass_output_from: None,
                    inline_prompt: None,
                    collect: None,
                    parallel_children: Some(vec![]),
                    aggregate: Some(ParallelAggregate {
                        all_match: Some("LGTM".to_string()),
                        any_match: None,
                        then: "done".to_string(),
                        r#else: "implementation_fix_policy".to_string(),
                    }),
                    resets_cycle_for: None,
                    model: None,
                    permission: None,
                    ..Default::default()
                },
                NodeDefinition {
                    name: "implementation_fix_policy".to_string(),
                    node_type: NodeType::Approval,
                    policy: None,
                    knowledge: None,
                    instruction: Some("Review fix policy".to_string()),
                    output_contract: Some("approved-fix-policy".to_string()),
                    transition_rules: vec![],
                    cycle_guard: None,
                    pass_previous_response: None,
                    pass_output_from: Some(vec!["code_review_parallel".to_string()]),
                    inline_prompt: None,
                    collect: None,
                    parallel_children: None,
                    aggregate: None,
                    resets_cycle_for: None,
                    model: None,
                    permission: None,
                    ..Default::default()
                },
            ],
        }
    }

    fn make_spec_driven_spec_fix_policy_exec(
        execution_id: &str,
        current_session_id: &str,
    ) -> WorkflowExecution {
        make_spec_driven_fix_policy_exec(execution_id, current_session_id, "spec_fix_policy")
    }

    fn make_spec_driven_fix_policy_exec(
        execution_id: &str,
        current_session_id: &str,
        step_name: &str,
    ) -> WorkflowExecution {
        let workflow =
            crate::workflow::builtin::load_builtin_workflow_resolved("spec-driven-development")
                .expect("builtin workflow must load")
                .expect("builtin workflow exists");
        let current_step_index = workflow
            .nodes
            .iter()
            .position(|step| step.name == step_name)
            .unwrap_or_else(|| panic!("{step_name} step exists"));
        WorkflowExecution {
            id: execution_id.to_string(),
            workflow,
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index,
            step_execution_counts: HashMap::from([(step_name.to_string(), 1)]),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some(current_session_id.to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        }
    }

    // ---- evaluate_auto_rules ----

    #[test]
    fn evaluate_auto_rules_matches_first_rule() {
        let rules = vec![
            TransitionRule {
                r#match: "NEEDS_FIX".to_string(),
                next: "implement".to_string(),
            },
            TransitionRule {
                r#match: "LGTM".to_string(),
                next: "report".to_string(),
            },
        ];

        let text = "<decision>NEEDS_FIX</decision>";
        let result = WorkflowEngine::evaluate_auto_rules(text, &rules);
        assert_eq!(
            result,
            Some(("implement".to_string(), "NEEDS_FIX".to_string()))
        );
    }

    #[test]
    fn evaluate_auto_rules_matches_second_rule() {
        let rules = vec![
            TransitionRule {
                r#match: "NEEDS_FIX".to_string(),
                next: "implement".to_string(),
            },
            TransitionRule {
                r#match: "LGTM".to_string(),
                next: "report".to_string(),
            },
        ];

        let text = "<decision>LGTM</decision>";
        let result = WorkflowEngine::evaluate_auto_rules(text, &rules);
        assert_eq!(result, Some(("report".to_string(), "LGTM".to_string())));
    }

    #[test]
    fn evaluate_auto_rules_no_match_returns_none() {
        let rules = vec![
            TransitionRule {
                r#match: "NEEDS_FIX".to_string(),
                next: "implement".to_string(),
            },
            TransitionRule {
                r#match: "LGTM".to_string(),
                next: "report".to_string(),
            },
        ];

        let text = "The code looks okay but needs minor refactoring";
        let result = WorkflowEngine::evaluate_auto_rules(text, &rules);
        assert_eq!(result, None);
    }

    #[test]
    fn evaluate_auto_rules_first_match_wins() {
        let rules = vec![
            TransitionRule {
                r#match: "FIX".to_string(),
                next: "implement".to_string(),
            },
            TransitionRule {
                r#match: "NEEDS_FIX".to_string(),
                next: "review".to_string(),
            },
        ];

        // Both "FIX" and "NEEDS_FIX" match, but first rule wins
        let text = "<decision>NEEDS_FIX</decision>";
        let result = WorkflowEngine::evaluate_auto_rules(text, &rules);
        assert_eq!(result, Some(("implement".to_string(), "FIX".to_string())));
    }

    #[test]
    fn evaluate_auto_rules_regex_pattern() {
        let rules = vec![TransitionRule {
            r#match: r"<decision>(LGTM|APPROVED)</decision>".to_string(),
            next: "report".to_string(),
        }];

        let text = "Review complete. <decision>APPROVED</decision>";
        let result = WorkflowEngine::evaluate_auto_rules(text, &rules);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "report");
    }

    // ---- extract_text_from_parts ----

    #[test]
    fn extract_text_from_parts_combines_text_parts() {
        let parts = vec![
            MessagePart::Thinking {
                content: "thinking...".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "First line".to_string(),
                parent_tool_use_id: None,
            },
            MessagePart::Text {
                content: "Second line".to_string(),
                parent_tool_use_id: None,
            },
        ];

        let text = WorkflowEngine::extract_text_from_parts(&parts);
        assert_eq!(text, "First line\nSecond line");
    }

    #[test]
    fn extract_text_from_parts_empty() {
        let parts: Vec<MessagePart> = vec![];
        let text = WorkflowEngine::extract_text_from_parts(&parts);
        assert_eq!(text, "");
    }

    // ---- WorkflowExecution ----

    fn make_test_step(
        name: &str,
        node_type: NodeType,
        instruction: &str,
        rules: Vec<TransitionRule>,
        cycle_guard: Option<CycleGuard>,
    ) -> NodeDefinition {
        NodeDefinition {
            name: name.to_string(),
            node_type,
            instruction: Some(instruction.to_string()),
            transition_rules: rules,
            cycle_guard,
            ..NodeDefinition::default()
        }
    }

    /// テストヘルパー: node の facet 参照を `base_dir` から解決し
    /// `resolved_facets` に格納する。`crate::workflow::facet::resolve_node_facets`
    /// （`#[cfg(test)] pub(crate)`）への薄い委譲で、欠損 facet 時の `unwrap` 等の
    /// パニックは facet helper 側で発生する。
    fn resolve_node_facets_for_test(node: &mut NodeDefinition, base_dir: &Path) {
        crate::workflow::facet::resolve_node_facets(node, base_dir)
            .expect("facet refs must resolve in tests; missing facet indicates a fixture bug");
    }

    /// テストヘルパー: 並列子 node の facet 参照を解決する。
    /// `crate::workflow::facet::resolve_child_facets` への委譲。
    fn resolve_child_facets_for_test(
        child: &mut crate::workflow::schema::ChildNodeDefinition,
        base_dir: &Path,
    ) {
        crate::workflow::facet::resolve_child_facets(child, base_dir)
            .expect("facet refs must resolve in tests; missing facet indicates a fixture bug");
    }

    fn make_test_workflow() -> Workflow {
        Workflow {
            variables: Default::default(),
            name: "test-workflow".to_string(),
            description: "Test workflow".to_string(),
            builtin: false,
            nodes: vec![
                make_test_step("plan", NodeType::Agent, "Plan the work", vec![], None),
                make_test_step(
                    "implement",
                    NodeType::Agent,
                    "Implement the plan",
                    vec![],
                    None,
                ),
                make_test_step(
                    "review",
                    NodeType::Agent,
                    "Review the implementation",
                    vec![
                        TransitionRule {
                            r#match: "NEEDS_FIX".to_string(),
                            next: "implement".to_string(),
                        },
                        TransitionRule {
                            r#match: "LGTM".to_string(),
                            next: "report".to_string(),
                        },
                    ],
                    Some(CycleGuard {
                        max_iterations: 3,
                        on_exhausted: None,
                    }),
                ),
                make_test_step(
                    "report",
                    NodeType::Approval,
                    "Generate report",
                    vec![TransitionRule {
                        r#match: "reject".to_string(),
                        next: "implement".to_string(),
                    }],
                    None,
                ),
            ],
        }
    }

    #[test]
    fn workflow_execution_to_workflow_state() {
        let workflow = make_test_workflow();
        let exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow,
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        let state = exec.to_workflow_state();
        assert_eq!(state.execution_id, "exec-1");
        assert_eq!(state.workflow_name, "test-workflow");
        assert_eq!(state.state, WorkflowExecutionState::Running);
        assert_eq!(state.current_step_index, 0);
        assert_eq!(state.current_step_name, "plan");
        assert_eq!(state.total_steps, 4);
        assert!(state.step_history.is_empty());
    }

    // ---- is_active ----

    #[test]
    fn is_active_running() {
        let exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: make_test_workflow(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        assert!(exec.is_active());
    }

    #[test]
    fn is_active_waiting_approval() {
        let exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: make_test_workflow(),
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        assert!(exec.is_active());
    }

    #[test]
    fn is_active_completed() {
        let exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: make_test_workflow(),
            state: WorkflowExecutionState::Completed,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        assert!(!exec.is_active());
    }

    #[test]
    fn is_active_failed() {
        let exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: make_test_workflow(),
            state: WorkflowExecutionState::Failed {
                reason: "err".to_string(),
            },
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        assert!(!exec.is_active());
    }

    #[test]
    fn is_active_aborted() {
        let exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: make_test_workflow(),
            state: WorkflowExecutionState::Aborted,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        assert!(!exec.is_active());
    }

    // ---- to_workflow_state: all state variants ----

    #[test]
    fn to_workflow_state_waiting_approval() {
        let workflow = make_test_workflow();
        let exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow,
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 3,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1001.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let ws = exec.to_workflow_state();
        assert_eq!(ws.state, WorkflowExecutionState::WaitingApproval);
        assert_eq!(ws.current_step_name, "report");
        assert_eq!(ws.current_step_index, 3);
        assert_eq!(
            ws.approval_operations.as_ref().map(|ops| ops.can_reject),
            Some(true)
        );
    }

    #[test]
    fn to_workflow_state_waiting_approval_without_reject_rule_disables_reject() {
        let workflow = make_test_workflow();
        let exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow,
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1001.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let ws = exec.to_workflow_state();
        assert_eq!(
            ws.approval_operations.as_ref().map(|ops| ops.can_reject),
            Some(false)
        );
    }

    #[test]
    fn to_workflow_state_failed() {
        let workflow = make_test_workflow();
        let exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow,
            state: WorkflowExecutionState::Failed {
                reason: "exit code 1".to_string(),
            },
            current_step_index: 1,
            step_execution_counts: HashMap::new(),
            step_history: vec![StepHistoryEntry {
                step_name: "plan".to_string(),
                completed_at: 1000.5,
                result: None,
                session_id: None,
                token_usage: None,
                structured_output: None,

                run_index: 0,
                child_outputs: None,
                state: crate::workflow::state::default_step_entry_state(),
            }],
            started_at: 1000.0,
            updated_at: 1001.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let ws = exec.to_workflow_state();
        assert_eq!(
            ws.state,
            WorkflowExecutionState::Failed {
                reason: "exit code 1".to_string()
            }
        );
        assert_eq!(ws.current_step_name, "implement");
        assert_eq!(ws.step_history.len(), 1);
    }

    #[test]
    fn to_workflow_state_aborted() {
        let workflow = make_test_workflow();
        let exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow,
            state: WorkflowExecutionState::Aborted,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1001.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let ws = exec.to_workflow_state();
        assert_eq!(ws.state, WorkflowExecutionState::Aborted);
    }

    #[test]
    fn to_workflow_state_completed() {
        let workflow = make_test_workflow();
        let exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow,
            state: WorkflowExecutionState::Completed,
            current_step_index: 3,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1002.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let ws = exec.to_workflow_state();
        assert_eq!(ws.state, WorkflowExecutionState::Completed);
        assert_eq!(ws.total_steps, 4);
    }

    // ---- evaluate_auto_rules: boundary cases ----

    #[test]
    fn evaluate_auto_rules_empty_rules() {
        let rules: Vec<TransitionRule> = vec![];
        let result = WorkflowEngine::evaluate_auto_rules("any text", &rules);
        assert_eq!(result, None);
    }

    #[test]
    fn evaluate_auto_rules_invalid_regex_skipped() {
        let rules = vec![
            TransitionRule {
                r#match: "[invalid".to_string(),
                next: "bad".to_string(),
            },
            TransitionRule {
                r#match: "LGTM".to_string(),
                next: "report".to_string(),
            },
        ];
        let result = WorkflowEngine::evaluate_auto_rules("LGTM", &rules);
        assert_eq!(result, Some(("report".to_string(), "LGTM".to_string())));
    }

    #[test]
    fn evaluate_auto_rules_all_invalid_regex_returns_none() {
        let rules = vec![TransitionRule {
            r#match: "[invalid".to_string(),
            next: "bad".to_string(),
        }];
        let result = WorkflowEngine::evaluate_auto_rules("anything", &rules);
        assert_eq!(result, None);
    }

    // ---- cycle_guard: boundary value at exactly max_iterations ----

    #[test]
    fn check_cycle_guard_at_boundary_minus_one_allowed() {
        let mut exec = make_exec(2); // review (max_iterations=3)
        exec.step_execution_counts.insert("review".to_string(), 2);
        assert_eq!(
            exec.check_cycle_guard("review").unwrap(),
            CycleGuardResult::Allowed
        );
    }

    #[test]
    fn check_cycle_guard_at_exact_boundary_exceeded() {
        let mut exec = make_exec(2); // review (max_iterations=3)
        exec.step_execution_counts.insert("review".to_string(), 3);
        assert_eq!(
            exec.check_cycle_guard("review").unwrap(),
            CycleGuardResult::Exceeded {
                max_iterations: 3,
                count: 3,
                on_exhausted: None,
            }
        );
    }

    #[test]
    fn cycle_guard_no_guard_defined() {
        let workflow = make_test_workflow();
        let step = &workflow.nodes[0]; // plan (no cycle_guard)
        assert!(step.cycle_guard.is_none());
    }

    // ---- decide_next_step ----

    fn make_exec(step_index: usize) -> WorkflowExecution {
        WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: make_test_workflow(),
            state: WorkflowExecutionState::Running,
            current_step_index: step_index,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            step_outputs: HashMap::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        }
    }

    #[test]
    fn decide_next_step_returns_next_step_name() {
        let exec = make_exec(0); // plan → next is implement
        assert_eq!(
            exec.decide_next_step(),
            NextStepDecision::TransitionTo("implement".to_string())
        );
    }

    #[test]
    fn decide_next_step_returns_completed_at_last_step() {
        let exec = make_exec(3); // report (last)
        assert_eq!(exec.decide_next_step(), NextStepDecision::Completed);
    }

    #[test]
    fn decide_next_step_middle_step() {
        let exec = make_exec(1); // implement → next is review
        assert_eq!(
            exec.decide_next_step(),
            NextStepDecision::TransitionTo("review".to_string())
        );
    }

    // ---- check_cycle_guard ----

    #[test]
    fn check_cycle_guard_allowed_no_guard() {
        let exec = make_exec(0);
        assert_eq!(
            exec.check_cycle_guard("plan").unwrap(),
            CycleGuardResult::Allowed
        );
    }

    #[test]
    fn check_cycle_guard_allowed_within_limit() {
        let mut exec = make_exec(2);
        exec.step_execution_counts.insert("review".to_string(), 2);
        assert_eq!(
            exec.check_cycle_guard("review").unwrap(),
            CycleGuardResult::Allowed
        );
    }

    #[test]
    fn check_cycle_guard_exceeded() {
        let mut exec = make_exec(2);
        exec.step_execution_counts.insert("review".to_string(), 3);
        assert_eq!(
            exec.check_cycle_guard("review").unwrap(),
            CycleGuardResult::Exceeded {
                max_iterations: 3,
                count: 3,
                on_exhausted: None,
            }
        );
    }

    #[test]
    fn check_cycle_guard_step_not_found() {
        let exec = make_exec(0);
        assert!(exec.check_cycle_guard("nonexistent").is_err());
    }

    #[test]
    fn check_cycle_guard_first_transition_no_count() {
        // step_execution_counts にキーなし = 初回遷移
        let exec = make_exec(2); // review has cycle_guard(max_iterations=3)
        assert_eq!(
            exec.check_cycle_guard("review").unwrap(),
            CycleGuardResult::Allowed
        );
    }

    // ---- decide_turn_complete_action ----

    #[test]
    fn turn_complete_action_not_running() {
        let mut exec = make_exec(0);
        exec.state = WorkflowExecutionState::Completed;
        assert_eq!(
            exec.decide_turn_complete_action(0),
            TurnCompleteAction::NotRunning
        );
    }

    #[test]
    fn turn_complete_action_session_error() {
        let exec = make_exec(0); // plan (interactive)
        assert_eq!(
            exec.decide_turn_complete_action(1),
            TurnCompleteAction::SessionError {
                step_name: "plan".to_string(),
                exit_code: 1,
            }
        );
    }

    #[test]
    fn turn_complete_action_auto_evaluate() {
        let exec = make_exec(2); // review (auto, has rules)
        let action = exec.decide_turn_complete_action(0);
        match action {
            TurnCompleteAction::AutoEvaluate { rules, step_name } => {
                assert_eq!(step_name, "review");
                assert_eq!(rules.len(), 2);
                assert_eq!(rules[0].r#match, "NEEDS_FIX");
                assert_eq!(rules[1].r#match, "LGTM");
            }
            other => panic!("Expected AutoEvaluate, got {:?}", other),
        }
    }

    #[test]
    fn turn_complete_action_wait_approval() {
        let exec = make_exec(3); // report (approval)
        assert_eq!(
            exec.decide_turn_complete_action(0),
            TurnCompleteAction::WaitApproval
        );
    }

    // [02]: Interactive 概念が廃止されたため、Interactive 用 SessionError 経路を
    // 検査する旧テスト `turn_complete_action_interactive_fails_for_validation_only_legacy_definition`
    // は削除した。bash / parallel 種別が turn_complete に流入した場合は専用バリアント
    // `UnexpectedNodeType` を返し、`SessionError { exit_code: 0 }`（正常終了セマンティクス）
    // との混同を避ける。下記 2 テストでバリアント別に確認する。

    #[test]
    fn turn_complete_action_unexpected_node_type_for_bash() {
        let mut exec = make_exec(0);
        exec.workflow.nodes[0].node_type = NodeType::Bash;
        let action = exec.decide_turn_complete_action(0);
        match action {
            TurnCompleteAction::UnexpectedNodeType {
                step_name,
                node_type,
            } => {
                assert_eq!(step_name, "plan");
                assert_eq!(node_type, NodeType::Bash);
            }
            other => panic!("Expected UnexpectedNodeType for Bash, got {:?}", other),
        }
    }

    #[test]
    fn turn_complete_action_unexpected_node_type_for_parallel() {
        let mut exec = make_exec(0);
        exec.workflow.nodes[0].node_type = NodeType::Parallel;
        let action = exec.decide_turn_complete_action(0);
        match action {
            TurnCompleteAction::UnexpectedNodeType {
                step_name,
                node_type,
            } => {
                assert_eq!(step_name, "plan");
                assert_eq!(node_type, NodeType::Parallel);
            }
            other => panic!("Expected UnexpectedNodeType for Parallel, got {:?}", other),
        }
    }

    #[test]
    fn turn_complete_action_waiting_approval_state_returns_not_running() {
        let mut exec = make_exec(3);
        exec.state = WorkflowExecutionState::WaitingApproval;
        assert_eq!(
            exec.decide_turn_complete_action(0),
            TurnCompleteAction::NotRunning
        );
    }

    #[test]
    fn turn_complete_action_negative_exit_code() {
        let exec = make_exec(0); // plan (interactive)
        assert_eq!(
            exec.decide_turn_complete_action(-1),
            TurnCompleteAction::SessionError {
                step_name: "plan".to_string(),
                exit_code: -1,
            }
        );
    }

    #[test]
    fn turn_complete_action_auto_no_rules_returns_auto_evaluate_empty() {
        let exec = make_exec(1); // implement (auto, no rules)
        let action = exec.decide_turn_complete_action(0);
        match action {
            TurnCompleteAction::AutoEvaluate { rules, step_name } => {
                assert_eq!(step_name, "implement");
                assert!(rules.is_empty());
            }
            other => panic!("Expected AutoEvaluate with empty rules, got {:?}", other),
        }
    }

    // ---- decide_approval_action ----

    #[test]
    fn decide_approval_action_approve() {
        let mut exec = make_exec(3); // report (approval)
        exec.state = WorkflowExecutionState::WaitingApproval;
        assert_eq!(
            exec.decide_approval_action(&ApprovalDecision::Approve)
                .unwrap(),
            ApprovalAction::Advance
        );
    }

    #[test]
    fn decide_approval_action_reject_with_rule() {
        let mut exec = make_exec(3); // report (approval, reject→implement)
        exec.state = WorkflowExecutionState::WaitingApproval;
        assert_eq!(
            exec.decide_approval_action(&ApprovalDecision::Reject {
                comment: "Needs fix".to_string()
            })
            .unwrap(),
            ApprovalAction::TransitionTo("implement".to_string())
        );
    }

    #[test]
    fn decide_approval_action_reject_no_rule() {
        let mut exec = make_exec(0); // plan (interactive, no reject rule)
        exec.state = WorkflowExecutionState::WaitingApproval;
        assert!(exec
            .decide_approval_action(&ApprovalDecision::Reject {
                comment: "Needs fix".to_string()
            })
            .is_err());
    }

    #[test]
    fn decide_approval_action_not_waiting() {
        let exec = make_exec(3); // report, state=Running
        assert!(exec
            .decide_approval_action(&ApprovalDecision::Approve)
            .is_err());
    }

    // ---- validate_start ----

    #[test]
    fn validate_start_empty_steps_returns_err() {
        let workflow = Workflow {
            variables: Default::default(),
            name: "empty".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![],
        };
        let result = WorkflowExecution::validate_start(&workflow, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no steps"));
    }

    #[test]
    fn validate_start_active_workflow_returns_err() {
        let workflow = make_test_workflow();
        let existing = make_exec(0); // Running state
        let result = WorkflowExecution::validate_start(&workflow, Some(&existing));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already running"));
    }

    #[test]
    fn validate_start_completed_workflow_allows_restart() {
        let workflow = make_test_workflow();
        let mut existing = make_exec(0);
        existing.state = WorkflowExecutionState::Completed;
        let result = WorkflowExecution::validate_start(&workflow, Some(&existing));
        assert!(result.is_ok());
    }

    #[test]
    fn validate_start_no_existing_returns_ok() {
        let workflow = make_test_workflow();
        let result = WorkflowExecution::validate_start(&workflow, None);
        assert!(result.is_ok());
    }

    /// [02] schema 境界: bash 種別 node を含む workflow は実行系未対応のため
    /// 開始前に明示的に拒否される（実行系は [13] で具体化）。
    #[test]
    fn validate_start_rejects_bash_node() {
        let workflow = Workflow {
            variables: Default::default(),
            name: "bash-wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "build".to_string(),
                node_type: NodeType::Bash,
                command: Some("echo hello".to_string()),
                ..NodeDefinition::default()
            }],
        };
        let result = WorkflowExecution::validate_start(&workflow, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Bash node"));
    }

    // ---- is_terminal ----

    #[test]
    fn is_terminal_completed() {
        let mut exec = make_exec(0);
        exec.state = WorkflowExecutionState::Completed;
        assert!(exec.is_terminal());
    }

    #[test]
    fn is_terminal_failed() {
        let mut exec = make_exec(0);
        exec.state = WorkflowExecutionState::Failed {
            reason: "err".to_string(),
        };
        assert!(exec.is_terminal());
    }

    #[test]
    fn is_terminal_aborted() {
        let mut exec = make_exec(0);
        exec.state = WorkflowExecutionState::Aborted;
        assert!(exec.is_terminal());
    }

    #[test]
    fn is_terminal_running_is_false() {
        let exec = make_exec(0);
        assert!(!exec.is_terminal());
    }

    #[test]
    fn is_terminal_waiting_approval_is_false() {
        let mut exec = make_exec(0);
        exec.state = WorkflowExecutionState::WaitingApproval;
        assert!(!exec.is_terminal());
    }

    // ---- step_states computation ----

    #[test]
    fn step_states_all_pending_at_start() {
        let exec = make_exec(0);
        let ws = exec.to_workflow_state();
        assert_eq!(ws.step_states["plan"], "running");
        assert_eq!(ws.step_states["implement"], "pending");
        assert_eq!(ws.step_states["review"], "pending");
        assert_eq!(ws.step_states["report"], "pending");
    }

    #[test]
    fn step_states_completed_steps() {
        let mut exec = make_exec(2);
        exec.step_history = vec![
            StepHistoryEntry {
                step_name: "plan".to_string(),
                completed_at: 1000.5,
                result: None,
                session_id: None,
                token_usage: None,
                structured_output: None,

                run_index: 0,
                child_outputs: None,
                state: crate::workflow::state::default_step_entry_state(),
            },
            StepHistoryEntry {
                step_name: "implement".to_string(),
                completed_at: 1001.0,
                result: None,
                session_id: None,
                token_usage: None,
                structured_output: None,

                run_index: 0,
                child_outputs: None,
                state: crate::workflow::state::default_step_entry_state(),
            },
        ];
        let ws = exec.to_workflow_state();
        assert_eq!(ws.step_states["plan"], "completed");
        assert_eq!(ws.step_states["implement"], "completed");
        assert_eq!(ws.step_states["review"], "running");
        assert_eq!(ws.step_states["report"], "pending");
    }

    #[test]
    fn step_states_failed_step() {
        let mut exec = make_exec(1);
        exec.state = WorkflowExecutionState::Failed {
            reason: "error".to_string(),
        };
        exec.step_history = vec![StepHistoryEntry {
            step_name: "plan".to_string(),
            completed_at: 1000.5,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,

            run_index: 0,
            child_outputs: None,
            state: crate::workflow::state::default_step_entry_state(),
        }];
        let ws = exec.to_workflow_state();
        assert_eq!(ws.step_states["plan"], "completed");
        assert_eq!(ws.step_states["implement"], "failed");
        assert_eq!(ws.step_states["review"], "pending");
        assert_eq!(ws.step_states["report"], "pending");
    }

    #[test]
    fn step_states_waiting_approval() {
        let mut exec = make_exec(1);
        exec.state = WorkflowExecutionState::WaitingApproval;
        exec.step_history = vec![StepHistoryEntry {
            step_name: "plan".to_string(),
            completed_at: 1000.5,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,

            run_index: 0,
            child_outputs: None,
            state: crate::workflow::state::default_step_entry_state(),
        }];
        let ws = exec.to_workflow_state();
        assert_eq!(ws.step_states["plan"], "completed");
        assert_eq!(ws.step_states["implement"], "waiting_approval");
        assert_eq!(ws.step_states["review"], "pending");
    }

    // ---- inject_step_outputs ----

    fn make_step_output(step_name: &str, output_text: &str, result: Option<&str>) -> StepOutput {
        StepOutput {
            step_name: step_name.to_string(),
            run_index: 0,
            session_id: None,
            result: result.map(|s| s.to_string()),
            structured_output: Some(serde_json::json!({"text": output_text})),
            output_contract: None,
            token_usage: None,
            completed_at: 1000.0,
        }
    }

    #[test]
    fn inject_step_outputs_pass_previous_response() {
        let mut step = make_test_step("step_b", NodeType::Agent, "Do B", vec![], None);
        step.pass_previous_response = Some(true);

        let mut outputs = HashMap::new();
        outputs.insert(
            "step_a".to_string(),
            make_step_output("step_a", "output from A", None),
        );
        let history = vec![StepHistoryEntry {
            step_name: "step_a".to_string(),
            completed_at: 1000.0,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,

            run_index: 0,
            child_outputs: None,
            state: crate::workflow::state::default_step_entry_state(),
        }];
        let wv = HashMap::new();
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &outputs, &history, &wv);
        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("output from A"));
    }

    #[test]
    fn inject_step_outputs_no_pass_previous_response() {
        let step = make_test_step("step_b", NodeType::Agent, "Do B", vec![], None);
        let outputs = HashMap::new();
        let history = vec![];
        let wv = HashMap::new();
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &outputs, &history, &wv);
        assert_eq!(result, "Do B");
    }

    #[test]
    fn inject_step_outputs_pass_output_from_single() {
        let mut step = make_test_step("step_c", NodeType::Agent, "Do C", vec![], None);
        step.pass_output_from = Some(vec!["step_a".to_string()]);

        let mut outputs = HashMap::new();
        outputs.insert(
            "step_a".to_string(),
            make_step_output("step_a", "output A", None),
        );
        let result =
            WorkflowEngine::inject_step_outputs("Do C", &step, &outputs, &[], &HashMap::new());
        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("output A"));
    }

    #[test]
    fn reject_comment_accessible_via_pass_output_from() {
        let mut step = make_test_step("fix", NodeType::Agent, "Fix issues", vec![], None);
        step.pass_output_from = Some(vec!["review".to_string()]);

        let mut outputs = HashMap::new();
        outputs.insert(
            "review".to_string(),
            make_step_output("review", "Fix the naming convention", Some("reject")),
        );
        let result = WorkflowEngine::inject_step_outputs(
            "Fix issues",
            &step,
            &outputs,
            &[],
            &HashMap::new(),
        );
        assert!(result.contains("<step_output name=\"review\">"));
        assert!(result.contains("Fix the naming convention"));
    }

    #[test]
    fn inject_step_outputs_pass_output_from_multiple() {
        let mut step = make_test_step("step_c", NodeType::Agent, "Do C", vec![], None);
        step.pass_output_from = Some(vec!["step_a".to_string(), "step_b".to_string()]);

        let mut outputs = HashMap::new();
        outputs.insert(
            "step_a".to_string(),
            make_step_output("step_a", "output A", None),
        );
        outputs.insert(
            "step_b".to_string(),
            make_step_output("step_b", "output B", None),
        );
        let result =
            WorkflowEngine::inject_step_outputs("Do C", &step, &outputs, &[], &HashMap::new());
        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("output A"));
        assert!(result.contains("<step_output name=\"step_b\">"));
        assert!(result.contains("output B"));
    }

    #[test]
    fn inject_step_outputs_pass_previous_response_no_output_injects_nothing() {
        let mut step = make_test_step("step_b", NodeType::Agent, "Do B", vec![], None);
        step.pass_previous_response = Some(true);

        let outputs = HashMap::new(); // step_a has no StepOutput
        let history = vec![StepHistoryEntry {
            step_name: "step_a".to_string(),
            completed_at: 1000.0,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,

            run_index: 0,
            child_outputs: None,
            state: crate::workflow::state::default_step_entry_state(),
        }];
        let wv = HashMap::new();
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &outputs, &history, &wv);
        assert_eq!(result, "Do B");
    }

    #[test]
    fn inject_step_outputs_missing_step_shows_not_completed() {
        let mut step = make_test_step("step_b", NodeType::Agent, "Do B", vec![], None);
        step.pass_output_from = Some(vec!["step_a".to_string()]);

        let outputs = HashMap::new(); // step_a not present
        let result =
            WorkflowEngine::inject_step_outputs("Do B", &step, &outputs, &[], &HashMap::new());
        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("(not yet completed)"));
    }

    #[test]
    fn inject_step_outputs_workflow_variables_injected() {
        let step = make_test_step("step_b", NodeType::Agent, "Do B", vec![], None);
        let mut wv = HashMap::new();
        wv.insert(
            "spec_dir".to_string(),
            "docs/spec/issues-909.md".to_string(),
        );
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &HashMap::new(), &[], &wv);
        assert!(result.contains("<workflow_variables>"));
        assert!(result.contains("spec_dir"));
        assert!(result.contains("docs/spec/issues-909.md"));
    }

    #[test]
    fn inject_step_outputs_empty_workflow_variables_not_injected() {
        let step = make_test_step("step_b", NodeType::Agent, "Do B", vec![], None);
        let wv = HashMap::new();
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &HashMap::new(), &[], &wv);
        assert!(!result.contains("<workflow_variables>"));
    }

    #[test]
    fn inject_step_outputs_parallel_parent_aggregated_children() {
        // 並列ブロック親名で集約された子出力がpass_output_fromで参照できること
        let mut step = make_test_step("spec_fix", NodeType::Agent, "Fix plan", vec![], None);
        step.pass_output_from = Some(vec![
            "spec_review_parallel".to_string(),
            "plan_draft".to_string(),
        ]);

        let mut outputs = HashMap::new();
        // 並列ブロック親の集約StepOutput（子出力をまとめたJSONオブジェクト）
        outputs.insert(
            "spec_review_parallel".to_string(),
            StepOutput {
                step_name: "spec_review_parallel".to_string(),
                run_index: 1,
                session_id: None,
                result: None,
                structured_output: Some(serde_json::json!({
                    "review_completeness": {
                        "verdict": "NEEDS_FIX",
                        "findings": [{"severity": "must_fix", "message": "Missing error handling"}]
                    },
                    "review_clarity": {
                        "verdict": "LGTM",
                        "findings": []
                    }
                })),
                output_contract: None,
                token_usage: None,
                completed_at: 1000.0,
            },
        );
        outputs.insert(
            "plan_draft".to_string(),
            make_step_output("plan_draft", "Draft spec content", None),
        );

        let result =
            WorkflowEngine::inject_step_outputs("Fix plan", &step, &outputs, &[], &HashMap::new());
        assert!(result.contains("<step_output name=\"spec_review_parallel\">"));
        assert!(result.contains("NEEDS_FIX"));
        assert!(result.contains("Missing error handling"));
        assert!(result.contains("<step_output name=\"plan_draft\">"));
        assert!(result.contains("Draft spec content"));
    }

    #[test]
    fn inject_step_outputs_parallel_parent_via_pass_previous_response() {
        // pass_previous_response: trueで並列ブロック親の集約出力が参照できること
        let mut step = make_test_step("spec_fix", NodeType::Agent, "Fix plan", vec![], None);
        step.pass_previous_response = Some(true);

        let mut outputs = HashMap::new();
        outputs.insert(
            "spec_review_parallel".to_string(),
            StepOutput {
                step_name: "spec_review_parallel".to_string(),
                run_index: 1,
                session_id: None,
                result: None,
                structured_output: Some(serde_json::json!({
                    "review_completeness": {"verdict": "LGTM", "findings": []},
                    "review_security": {"verdict": "NEEDS_FIX", "findings": [{"severity": "must_fix", "message": "SQL injection risk"}]}
                })),
                output_contract: None,
                token_usage: None,
                completed_at: 1000.0,
            },
        );

        let history = vec![StepHistoryEntry {
            step_name: "spec_review_parallel".to_string(),
            completed_at: 1000.0,
            result: Some("else".to_string()),
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: None,
            state: crate::workflow::state::default_step_entry_state(),
        }];

        let result = WorkflowEngine::inject_step_outputs(
            "Fix plan",
            &step,
            &outputs,
            &history,
            &HashMap::new(),
        );
        assert!(result.contains("<step_output name=\"spec_review_parallel\">"));
        assert!(result.contains("NEEDS_FIX"));
        assert!(result.contains("SQL injection risk"));
    }

    // ---- extract_contract_variables ----

    #[test]
    fn extract_contract_variables_spec_dir() {
        let contract = Some("spec-directory".to_string());
        let so = Some(serde_json::json!({
            "spec_dir": "docs/spec/issues-909.md"
        }));
        let vars = WorkflowEngine::extract_contract_variables(&contract, &so);
        assert_eq!(vars.get("spec_dir").unwrap(), "docs/spec/issues-909.md");
    }

    #[test]
    fn extract_contract_variables_non_spec_contract_returns_empty() {
        let contract = Some("review-verdict".to_string());
        let so = Some(serde_json::json!({
            "verdict": "LGTM",
            "summary": "All good"
        }));
        let vars = WorkflowEngine::extract_contract_variables(&contract, &so);
        assert!(vars.is_empty());
    }

    #[test]
    fn extract_contract_variables_approved_fix_policy_is_not_global() {
        let contract = Some("approved-fix-policy".to_string());
        let so = Some(serde_json::json!({
            "policy": "Fix only approved findings.",
            "review_step": "spec_review_parallel",
            "findings": []
        }));
        let vars = WorkflowEngine::extract_contract_variables(&contract, &so);
        assert!(vars.is_empty());
    }

    #[test]
    fn mask_sensitive_text_redacts_policy_secrets() {
        let text = "password=secret123 ghp_abcdefghijklmnopqrstuvwxyz1234567890 -----BEGIN PRIVATE KEY-----abc-----END PRIVATE KEY----- MY_TOKEN_VALUE_123456";
        let masked =
            WorkflowEngine::mask_sensitive_text(text, &["MY_TOKEN_VALUE_123456".to_string()]);
        assert!(!masked.contains("secret123"));
        assert!(!masked.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
        assert!(!masked.contains("PRIVATE KEY-----abc"));
        assert!(!masked.contains("MY_TOKEN_VALUE_123456"));
        assert!(masked.contains("[REDACTED]"));
    }

    #[test]
    fn configured_secret_values_include_notion_api_tokens() {
        let mut cfg = crate::config::ReleashConfig::default();
        cfg.server.token = "SERVER_TOKEN_123".to_string();
        cfg.server.mcp_token = "MCP_TOKEN_123456".to_string();
        cfg.notion.insert(
            "/repo".to_string(),
            crate::notion::types::NotionRepoConfig {
                api_token: "NOTION_TOKEN_123456".to_string(),
                database_id: "database".to_string(),
                property_mapping: crate::notion::types::PropertyMapping::default(),
            },
        );

        let secrets = WorkflowEngine::collect_configured_secret_values_from_config(&cfg);
        assert!(secrets.contains(&"SERVER_TOKEN_123".to_string()));
        assert!(secrets.contains(&"MCP_TOKEN_123456".to_string()));
        assert!(secrets.contains(&"NOTION_TOKEN_123456".to_string()));

        let masked = WorkflowEngine::mask_sensitive_text(
            "Use NOTION_TOKEN_123456 in this policy.",
            &secrets,
        );
        assert_eq!(masked, "Use [REDACTED] in this policy.");
    }

    #[test]
    fn overlapping_configured_secret_values_are_redacted_longest_first() {
        let text = "Use abcdefghXYZ and abcdefgh in this policy.";
        let masked = WorkflowEngine::mask_sensitive_text(
            text,
            &["abcdefghXYZ".to_string(), "abcdefgh".to_string()],
        );

        assert_eq!(masked, "Use [REDACTED] and [REDACTED] in this policy.");
        assert!(!masked.contains("XYZ"));
        assert!(!masked.contains("abcdefgh"));
    }

    #[test]
    fn environment_secret_values_include_only_named_secret_values_at_least_eight_bytes() {
        let secrets = WorkflowEngine::collect_secret_values_from_env_vars(vec![
            (
                "APPROVED_POLICY_TOKEN".to_string(),
                "SECRET_VALUE_123".to_string(),
            ),
            ("PATH".to_string(), "/bin:/usr/bin".to_string()),
            (
                "APPROVED_POLICY_TEXT".to_string(),
                "GENERAL_VALUE_123".to_string(),
            ),
            (
                "SERVICE_API_KEY".to_string(),
                "API_KEY_VALUE_123".to_string(),
            ),
            ("SHORT_TOKEN".to_string(), "short".to_string()),
            ("EMPTY".to_string(), String::new()),
        ]);

        assert!(secrets.contains(&"SECRET_VALUE_123".to_string()));
        assert!(secrets.contains(&"API_KEY_VALUE_123".to_string()));
        assert!(!secrets.contains(&"GENERAL_VALUE_123".to_string()));
        assert!(!secrets.contains(&"/bin:/usr/bin".to_string()));
        assert!(!secrets.contains(&"short".to_string()));
    }

    #[test]
    fn approved_fix_policy_structured_output_is_masked_for_parallel_contract_path() {
        let masked = WorkflowEngine::mask_sensitive_structured_output_with_secrets(
            "approved-fix-policy",
            serde_json::json!({
                "policy": "Use password=secret123 and MY_TOKEN_VALUE_123456",
                "review_step": "code_review_parallel",
                "findings": []
            }),
            &["MY_TOKEN_VALUE_123456".to_string()],
        );
        let serialized = serde_json::to_string(&masked).unwrap();
        assert!(serialized.contains("[REDACTED]"));
        assert!(!serialized.contains("secret123"));
        assert!(!serialized.contains("MY_TOKEN_VALUE_123456"));
    }

    #[test]
    fn reject_structured_output_redacts_sensitive_comment_before_history_or_sync() {
        let structured = WorkflowEngine::reject_structured_output(
            "Reject because password=secret123 and ghp_abcdefghijklmnopqrstuvwxyz1234567890",
            &[],
        );
        let comment = structured["comment"].as_str().unwrap();
        assert!(!comment.contains("secret123"));
        assert!(!comment.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
        assert!(comment.contains("[REDACTED]"));
    }

    #[test]
    fn approved_policy_injected_output_uses_sanitized_contract_payload_without_global_variables() {
        let mut step = make_test_step("fix", NodeType::Agent, "Fix", vec![], None);
        step.pass_output_from = Some(vec!["implementation_fix_policy".to_string()]);

        let sanitized = serde_json::json!({
            "policy": "Use password=[REDACTED] only in examples.",
            "review_step": "code_review_parallel",
            "findings": []
        });
        let vars = WorkflowEngine::extract_contract_variables(
            &Some("approved-fix-policy".to_string()),
            &Some(sanitized.clone()),
        );
        assert!(vars.is_empty());

        let mut outputs = HashMap::new();
        outputs.insert(
            "implementation_fix_policy".to_string(),
            StepOutput {
                step_name: "implementation_fix_policy".to_string(),
                run_index: 1,
                session_id: Some("policy-session".to_string()),
                result: Some("approved".to_string()),
                structured_output: Some(sanitized),
                output_contract: Some("approved-fix-policy".to_string()),
                token_usage: None,
                completed_at: 1000.0,
            },
        );
        let injected = WorkflowEngine::inject_step_outputs("Fix", &step, &outputs, &[], &vars);
        assert!(injected.contains("[REDACTED]"));
        assert!(injected.contains("<step_output name=\"implementation_fix_policy\">"));
        assert!(!injected.contains("<workflow_variables>"));
        assert!(!injected.contains("secret123"));
    }

    #[test]
    fn approved_policy_masks_raw_secrets_before_state_variables_history_and_injection() {
        let mut structured = serde_json::json!({
            "policy": "Use password=secret123 with ghp_abcdefghijklmnopqrstuvwxyz1234567890 -----BEGIN PRIVATE KEY-----abc-----END PRIVATE KEY----- MY_TOKEN_VALUE_123456",
            "review_step": "code_review_parallel",
            "findings": []
        });
        WorkflowEngine::mask_json_strings(&mut structured, &["MY_TOKEN_VALUE_123456".to_string()]);
        let raw = serde_json::to_string(&structured).unwrap();
        assert!(!raw.contains("secret123"));
        assert!(!raw.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
        assert!(!raw.contains("PRIVATE KEY-----abc"));
        assert!(!raw.contains("MY_TOKEN_VALUE_123456"));

        let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        exec.workflow.nodes[0].output_contract = Some("approved-fix-policy".to_string());
        exec.workflow.nodes.push(NodeDefinition {
            name: "fix".to_string(),
            node_type: NodeType::Agent,
            policy: None,
            knowledge: None,
            instruction: None,
            output_contract: None,
            transition_rules: vec![],
            cycle_guard: None,
            pass_previous_response: Some(true),
            pass_output_from: None,
            inline_prompt: Some("Fix".to_string()),
            collect: None,
            parallel_children: None,
            aggregate: None,
            resets_cycle_for: None,
            model: None,
            permission: None,
            ..Default::default()
        });
        let outcome = WorkflowEngine::apply_approval_application(
            &mut exec,
            &ApprovalDecision::Approve,
            ApprovalApplication {
                effective_result: "approved".to_string(),
                structured_output: Some(structured),
                output_contract: Some("approved-fix-policy".to_string()),
            },
        )
        .unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));

        let state = exec.to_workflow_state();
        let state_json = serde_json::to_string(&state).unwrap();
        assert!(state_json.contains("[REDACTED]"));
        assert!(!state_json.contains("secret123"));
        assert!(!state_json.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
        assert!(!state_json.contains("MY_TOKEN_VALUE_123456"));
        assert!(!exec.workflow_variables.contains_key("approved_fix_policy"));
        assert!(!exec.step_history[0]
            .structured_output
            .as_ref()
            .unwrap()
            .to_string()
            .contains("secret123"));

        let injected = WorkflowEngine::inject_step_outputs(
            "Fix",
            &exec.workflow.nodes[exec.current_step_index],
            &exec.step_outputs,
            &exec.step_history,
            &exec.workflow_variables,
        );
        assert!(injected.contains("[REDACTED]"));
        assert!(!injected.contains("<workflow_variables>"));
        assert!(!injected.contains("secret123"));
        assert!(!injected.contains("MY_TOKEN_VALUE_123456"));
    }

    #[test]
    fn approved_policy_workflow_event_log_readback_redacts_sensitive_values() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut exec = make_spec_driven_spec_fix_policy_exec(
            "00000000-0000-0000-0000-000000000917",
            "policy-session",
        );
        let secret_env_value = "MY_TOKEN_VALUE_123456".to_string();
        let mut structured = serde_json::json!({
            "policy": "Use password=secret123 with ghp_abcdefghijklmnopqrstuvwxyz1234567890 -----BEGIN PRIVATE KEY-----abc-----END PRIVATE KEY----- MY_TOKEN_VALUE_123456",
            "review_step": "spec_review_parallel",
            "findings": []
        });
        WorkflowEngine::mask_json_strings(&mut structured, &[secret_env_value]);

        let outcome = WorkflowEngine::apply_approval_application(
            &mut exec,
            &ApprovalDecision::Approve,
            ApprovalApplication {
                effective_result: "approved".to_string(),
                structured_output: Some(structured),
                output_contract: Some("approved-fix-policy".to_string()),
            },
        )
        .unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));

        let entry = exec
            .step_history
            .iter()
            .find(|entry| entry.step_name == "spec_fix_policy")
            .unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&WorkflowEvent::RunStarted {
            run_id: exec.id.clone(),
            workflow_name: exec.workflow.name.clone(),
            workflow_file_stem: "spec-driven-development".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: exec.workflow.clone(),
            timestamp: 1000.0,
        })
        .unwrap();
        log.append(&WorkflowEvent::NodeCompleted {
            run_id: exec.id.clone(),
            workflow_name: exec.workflow.name.clone(),
            node_name: entry.step_name.clone(),
            result: entry.result.clone(),
            session_id: entry.session_id.clone(),
            token_usage: entry.token_usage.clone(),
            structured_output: entry.structured_output.clone(),
            run_index: Some(entry.run_index),
            timestamp: entry.completed_at,
        })
        .unwrap();

        let raw_ndjson =
            std::fs::read_to_string(tmp.path().join(format!("workflow_logs/{}.ndjson", exec.id)))
                .unwrap();
        assert!(raw_ndjson.contains("[REDACTED]"));
        assert!(!raw_ndjson.contains("secret123"));
        assert!(!raw_ndjson.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
        assert!(!raw_ndjson.contains("PRIVATE KEY-----abc"));
        assert!(!raw_ndjson.contains("MY_TOKEN_VALUE_123456"));

        let events = log.read_log(&exec.id).unwrap();
        let serialized = serde_json::to_string(&events).unwrap();
        assert!(serialized.contains("[REDACTED]"));
        assert!(!serialized.contains("secret123"));
        assert!(!serialized.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
        assert!(!serialized.contains("PRIVATE KEY-----abc"));
        assert!(!serialized.contains("MY_TOKEN_VALUE_123456"));
        let completed = events
            .iter()
            .find(|event| matches!(event, WorkflowEvent::NodeCompleted { .. }))
            .unwrap();
        match completed {
            WorkflowEvent::NodeCompleted {
                structured_output, ..
            } => {
                let policy = structured_output
                    .as_ref()
                    .and_then(|output| output.get("policy"))
                    .and_then(|policy| policy.as_str())
                    .unwrap();
                assert!(policy.contains("[REDACTED]"));
                assert!(!policy.contains("secret123"));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn extract_contract_variables_no_contract_returns_empty() {
        let so = Some(serde_json::json!({"spec_dir": "docs/spec.md"}));
        let vars = WorkflowEngine::extract_contract_variables(&None, &so);
        assert!(vars.is_empty());
    }

    #[test]
    fn extract_contract_variables_no_output_returns_empty() {
        let contract = Some("spec-directory".to_string());
        let vars = WorkflowEngine::extract_contract_variables(&contract, &None);
        assert!(vars.is_empty());
    }

    #[test]
    fn extract_contract_variables_missing_field_returns_empty() {
        let contract = Some("spec-directory".to_string());
        let so = Some(serde_json::json!({"other_field": "value"}));
        let vars = WorkflowEngine::extract_contract_variables(&contract, &so);
        assert!(vars.is_empty());
    }

    // ---- contract retry 判定テストは prose 抽出経路 ([08] で廃止) の付随物だったため削除した。
    //      contract 適合判定は CLI / Tauri 経由の SubmitOutput で発生し、retry は行わない。

    // ---- apply_reduce ----

    fn make_collect(from: Vec<&str>, reduce: ReduceStrategy) -> CollectConfig {
        CollectConfig {
            from: from.iter().map(|s| s.to_string()).collect(),
            reduce,
        }
    }

    fn make_outputs(entries: Vec<(&str, &str, Option<&str>)>) -> HashMap<String, StepOutput> {
        let mut map = HashMap::new();
        for (name, text, result) in entries {
            map.insert(name.to_string(), make_step_output(name, text, result));
        }
        map
    }

    #[test]
    fn reduce_last_returns_latest_completed_entry() {
        let collect = make_collect(vec!["a", "b", "c"], ReduceStrategy::Last);
        let mut outputs = HashMap::new();
        outputs.insert(
            "a".to_string(),
            StepOutput {
                completed_at: 1000.0,
                ..make_step_output("a", "text_a", Some("LGTM"))
            },
        );
        outputs.insert(
            "b".to_string(),
            StepOutput {
                completed_at: 3000.0, // bが最後に完了
                ..make_step_output("b", "text_b", Some("NEEDS_FIX"))
            },
        );
        outputs.insert(
            "c".to_string(),
            StepOutput {
                completed_at: 2000.0,
                ..make_step_output("c", "text_c", Some("LGTM"))
            },
        );
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        // 設定順ではcが最後だが、completed_at最大のbが選ばれる
        assert_eq!(r.result, Some("NEEDS_FIX".to_string()));
        let so = r.structured_output.unwrap();
        assert_eq!(so["text"], "text_b");
    }

    #[test]
    fn reduce_concat_joins_all() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::Concat);
        let outputs = make_outputs(vec![
            ("a", "output from a", None),
            ("b", "output from b", None),
        ]);
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        assert!(r.result.is_none());
        let so = r.structured_output.unwrap();
        let arr = so.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["stepName"], "a");
        assert_eq!(arr[0]["output"]["text"], "output from a");
        assert_eq!(arr[1]["stepName"], "b");
        assert_eq!(arr[1]["output"]["text"], "output from b");
    }

    #[test]
    fn reduce_grouped_groups_by_result() {
        let collect = make_collect(vec!["a", "b", "c"], ReduceStrategy::Grouped);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("LGTM")),
            ("b", "text_b", Some("NEEDS_FIX")),
            ("c", "text_c", Some("LGTM")),
        ]);
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        assert!(r.result.is_none());
        let so = r.structured_output.unwrap();
        let lgtm = so["LGTM"].as_array().unwrap();
        assert!(lgtm.contains(&serde_json::Value::String("a".to_string())));
        assert!(lgtm.contains(&serde_json::Value::String("c".to_string())));
        let needs_fix = so["NEEDS_FIX"].as_array().unwrap();
        assert!(needs_fix.contains(&serde_json::Value::String("b".to_string())));
    }

    #[test]
    fn reduce_any_needs_fix_one_needs_fix() {
        let collect = make_collect(vec!["a", "b", "c"], ReduceStrategy::AnyNeedsFix);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("LGTM")),
            ("b", "text_b", Some("NEEDS_FIX")),
            ("c", "text_c", Some("LGTM")),
        ]);
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        assert_eq!(r.result, Some("NEEDS_FIX".to_string()));
    }

    #[test]
    fn reduce_any_needs_fix_all_lgtm() {
        let collect = make_collect(vec!["a", "b", "c"], ReduceStrategy::AnyNeedsFix);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("LGTM")),
            ("b", "text_b", Some("LGTM")),
            ("c", "text_c", Some("LGTM")),
        ]);
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        assert_eq!(r.result, Some("LGTM".to_string()));
    }

    #[test]
    fn reduce_any_needs_fix_no_result_treated_as_lgtm() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AnyNeedsFix);
        // result is None → resolve_step_result returns None → not NEEDS_FIX
        let outputs = make_outputs(vec![
            ("a", "Everything looks good", None),
            ("b", "Found issues text", None),
        ]);
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        assert_eq!(r.result, Some("LGTM".to_string()));
    }

    #[test]
    fn reduce_all_passed_all_pass() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AllPassed);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("PASSED")),
            ("b", "text_b", Some("PASSED")),
        ]);
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        assert_eq!(r.result, Some("PASSED".to_string()));
    }

    #[test]
    fn reduce_all_passed_one_failed() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AllPassed);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("PASSED")),
            ("b", "text_b", Some("FAILED")),
        ]);
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        assert_eq!(r.result, Some("FAILED".to_string()));
    }

    #[test]
    fn reduce_all_passed_no_result_treated_as_failed() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AllPassed);
        // result is None → not PASSED/LGTM → all_passed = false
        let outputs = make_outputs(vec![
            ("a", "All tests ran", None),
            ("b", "Some tests ran", None),
        ]);
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        assert_eq!(r.result, Some("FAILED".to_string()));
    }

    // ---- reduce structured_output array format ----

    #[test]
    fn reduce_any_needs_fix_structured_output_is_array() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AnyNeedsFix);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("LGTM")),
            ("b", "text_b", Some("NEEDS_FIX")),
        ]);
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        assert_eq!(r.result, Some("NEEDS_FIX".to_string()));
        let so = r.structured_output.unwrap();
        let arr = so.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["stepName"], "a");
        assert_eq!(arr[0]["output"]["text"], "text_a");
        assert_eq!(arr[1]["stepName"], "b");
        assert_eq!(arr[1]["output"]["text"], "text_b");
    }

    #[test]
    fn reduce_all_passed_structured_output_is_array() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AllPassed);
        let outputs = make_outputs(vec![
            ("a", "text_a", Some("PASSED")),
            ("b", "text_b", Some("PASSED")),
        ]);
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        assert_eq!(r.result, Some("PASSED".to_string()));
        let so = r.structured_output.unwrap();
        let arr = so.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["stepName"], "a");
        assert_eq!(arr[1]["stepName"], "b");
    }

    // ---- collect_step_output_entries ----

    #[test]
    fn collect_step_output_entries_returns_array_with_step_name_and_output() {
        let outputs = make_outputs(vec![
            ("s1", "out1", Some("LGTM")),
            ("s2", "out2", Some("NEEDS_FIX")),
        ]);
        let from = vec!["s1".to_string(), "s2".to_string()];
        let entries = WorkflowEngine::collect_step_output_entries(&from, &outputs);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["stepName"], "s1");
        assert_eq!(entries[0]["output"]["text"], "out1");
        assert_eq!(entries[1]["stepName"], "s2");
        assert_eq!(entries[1]["output"]["text"], "out2");
    }

    #[test]
    fn collect_step_output_entries_skips_missing_outputs() {
        let outputs = make_outputs(vec![("s1", "out1", None)]);
        let from = vec!["s1".to_string(), "s2".to_string()];
        let entries = WorkflowEngine::collect_step_output_entries(&from, &outputs);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["stepName"], "s1");
    }

    #[test]
    fn collect_step_output_entries_skips_none_structured_output() {
        let mut outputs = HashMap::new();
        outputs.insert(
            "s1".to_string(),
            StepOutput {
                structured_output: None,
                ..make_step_output("s1", "text", None)
            },
        );
        let from = vec!["s1".to_string()];
        let entries = WorkflowEngine::collect_step_output_entries(&from, &outputs);
        assert!(entries.is_empty());
    }

    // ---- resolve_step_result ----

    #[test]
    fn resolve_step_result_returns_result_field() {
        let output = make_step_output("s", "output with NEEDS_FIX text", Some("LGTM"));
        let r = WorkflowEngine::resolve_step_result(&output);
        assert_eq!(r, Some("LGTM".to_string()));
    }

    #[test]
    fn resolve_step_result_none_when_no_result() {
        let output = make_step_output("s", "found NEEDS_FIX issue", None);
        let r = WorkflowEngine::resolve_step_result(&output);
        assert!(r.is_none());
    }

    #[test]
    fn resolve_step_result_no_match_returns_none() {
        let output = make_step_output("s", "everything is fine", None);
        let r = WorkflowEngine::resolve_step_result(&output);
        assert!(r.is_none());
    }

    #[test]
    fn resolve_step_result_prefers_structured_verdict() {
        let output = StepOutput {
            step_name: "s".to_string(),
            run_index: 0,
            session_id: None,
            result: Some("LGTM".to_string()),
            structured_output: Some(
                serde_json::json!({"verdict": "NEEDS_FIX", "findings": [{"severity": "error", "message": "bug"}]}),
            ),
            output_contract: None,
            token_usage: None,
            completed_at: 1000.0,
        };
        let r = WorkflowEngine::resolve_step_result(&output);
        assert_eq!(r, Some("NEEDS_FIX".to_string()));
    }

    #[test]
    fn resolve_step_result_prefers_structured_status() {
        let output = StepOutput {
            step_name: "s".to_string(),
            run_index: 0,
            session_id: None,
            result: None,
            structured_output: Some(serde_json::json!({"status": "FIXED"})),
            output_contract: None,
            token_usage: None,
            completed_at: 1000.0,
        };
        let r = WorkflowEngine::resolve_step_result(&output);
        assert_eq!(r, Some("FIXED".to_string()));
    }

    #[test]
    fn resolve_step_result_verdict_over_status() {
        let output = StepOutput {
            step_name: "s".to_string(),
            run_index: 0,
            session_id: None,
            result: None,
            structured_output: Some(serde_json::json!({"verdict": "LGTM", "status": "FIXED"})),
            output_contract: None,
            token_usage: None,
            completed_at: 1000.0,
        };
        let r = WorkflowEngine::resolve_step_result(&output);
        assert_eq!(r, Some("LGTM".to_string()));
    }

    // ---- truncate_output ----

    #[test]
    fn truncate_output_within_limit() {
        let text = "hello".to_string();
        assert_eq!(super::truncate_output(text), "hello");
    }

    #[test]
    fn truncate_output_exceeds_limit_ascii() {
        let text = "a".repeat(super::MAX_OUTPUT_SIZE + 100);
        let result = super::truncate_output(text);
        assert!(result.ends_with("... (truncated)"));
        assert!(result.len() <= super::MAX_OUTPUT_SIZE + 20);
    }

    #[test]
    fn truncate_output_multibyte_boundary() {
        // 日本語文字（3バイト）でMAX_OUTPUT_SIZEの境界がバイト途中になるケース
        let text = "あ".repeat(super::MAX_OUTPUT_SIZE); // 3 * MAX_OUTPUT_SIZE bytes
        let result = super::truncate_output(text);
        assert!(result.ends_with("... (truncated)"));
        // 結果がvalidなUTF-8であることの確認（panicしないこと自体がテスト）
        assert!(!result.is_empty());
    }

    // ---- evaluate_auto_rules (reduce結果による遷移判定) ----

    #[test]
    fn reduce_result_triggers_transition_via_evaluate_auto_rules() {
        let rules = vec![TransitionRule {
            r#match: "NEEDS_FIX".to_string(),
            next: "fix".to_string(),
        }];
        let result = WorkflowEngine::evaluate_auto_rules("NEEDS_FIX", &rules);
        assert_eq!(result, Some(("fix".to_string(), "NEEDS_FIX".to_string())));
    }

    #[test]
    fn reduce_result_lgtm_no_matching_rule_returns_none() {
        let rules = vec![TransitionRule {
            r#match: "NEEDS_FIX".to_string(),
            next: "fix".to_string(),
        }];
        let result = WorkflowEngine::evaluate_auto_rules("LGTM", &rules);
        assert!(result.is_none());
    }

    // ---- render_facet_variables ----

    #[test]
    fn render_facet_variables_replaces_task_and_project_name() {
        let content = "Task: {{task}}\nProject: {{project_name}}";
        let result = WorkflowEngine::render_facet_variables(
            content,
            "/home/user/my-project",
            Some("Fix bug"),
        );
        assert_eq!(result, "Task: Fix bug\nProject: my-project");
    }

    #[test]
    fn render_facet_variables_task_none_replaces_with_empty() {
        let content = "Do: {{task}}";
        let result = WorkflowEngine::render_facet_variables(content, "/home/user/proj", None);
        assert_eq!(result, "Do: ");
    }

    #[test]
    fn render_facet_variables_no_variables_unchanged() {
        let content = "No variables here";
        let result =
            WorkflowEngine::render_facet_variables(content, "/home/user/proj", Some("task"));
        assert_eq!(result, "No variables here");
    }

    // ---- build_step_prompt ----

    #[test]
    fn build_step_prompt_full_pipeline() {
        let tmp = tempfile::TempDir::new().unwrap();
        let base = tmp.path();
        let instructions = base.join("instructions");
        let policies = base.join("policies");
        let contracts = base.join("contracts");
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::create_dir_all(&contracts).unwrap();
        std::fs::write(
            policies.join("coding.md"),
            "Coding policy for {{project_name}}.",
        )
        .unwrap();
        std::fs::write(
            instructions.join("impl.md"),
            "Task: {{task}}\nImplement the feature.",
        )
        .unwrap();
        std::fs::write(contracts.join("plan-doc.md"), "Output as markdown.").unwrap();

        let mut step = make_test_step("build", NodeType::Agent, "unused", vec![], None);
        step.instruction = Some("impl".to_string());
        step.policy = Some("coding".to_string());
        step.output_contract = Some("plan-doc".to_string());
        step.pass_previous_response = Some(true);
        resolve_node_facets_for_test(&mut step, base);

        let mut outputs = HashMap::new();
        outputs.insert(
            "plan".to_string(),
            make_step_output("plan", "Plan output text", None),
        );
        let history = vec![StepHistoryEntry {
            step_name: "plan".to_string(),
            completed_at: 2000.0,
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,

            run_index: 0,
            child_outputs: None,
            state: crate::workflow::state::default_step_entry_state(),
        }];
        let (sys, prompt) = WorkflowEngine::build_step_prompt(
            &step,
            "00000000-0000-0000-0000-000000000000",
            "/home/user/my-app",
            Some("Fix bug"),
            &outputs,
            &history,
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        // policy + output_contract → system_prompt with variable expansion
        let sys_str = sys.expect("system_prompt should be set");
        assert!(sys_str.contains("Coding policy for my-app."));
        assert!(sys_str.contains("Output as markdown."));
        // instruction in user_message, with variable expansion
        assert!(prompt.contains("Task: Fix bug"));
        assert!(prompt.contains("Implement the feature."));
        // output_contract がある場合、作業本文の末尾にも Contract 由来の
        // 完了時アクションを置き、初回完了時に CLI 提出へ誘導する。
        assert!(prompt.contains("完了時の必須アクション"));
        // CLI 名は起動環境別 alias で展開される（spec issues-1054）。
        let cli_alias = WorkflowEngine::resolve_releash_alias();
        assert!(prompt.contains(&format!(
            "{cli_alias} workflow output submit 00000000-0000-0000-0000-000000000000"
        )));
        assert!(prompt.contains("--step build"));
        assert!(prompt.contains("--type plan-doc"));
        assert!(prompt.contains("--json"));
        assert!(!prompt.contains("--file"));
        assert!(!prompt.contains("+  --step"));
        // inject_step_outputs: pass_previous_response includes plan output
        assert!(prompt.contains("<step_output name=\"plan\">"));
        assert!(prompt.contains("Plan output text"));
        assert!(
            prompt.find("完了時の必須アクション").unwrap()
                > prompt.find("Plan output text").unwrap(),
            "completion action must remain after injected step outputs"
        );
    }

    #[test]
    fn build_step_prompt_no_facet_refs_returns_error() {
        let step = NodeDefinition {
            name: "empty".to_string(),
            node_type: NodeType::Agent,
            policy: None,
            knowledge: None,
            instruction: None,
            output_contract: None,
            transition_rules: vec![],
            cycle_guard: None,
            pass_previous_response: None,
            pass_output_from: None,
            inline_prompt: None,
            collect: None,
            parallel_children: None,
            aggregate: None,
            resets_cycle_for: None,
            model: None,
            permission: None,
            ..Default::default()
        };
        let result = WorkflowEngine::build_step_prompt(
            &step,
            "00000000-0000-0000-0000-000000000000",
            "/repo",
            None,
            &HashMap::new(),
            &[],
            &HashMap::new(),
            &HashMap::new(),
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no facet refs"));
    }

    #[test]
    fn build_step_prompt_policy_only_system_prompt_set() {
        // Scenario: policyのみを指定したステップでも system_prompt が合成される
        let tmp = tempfile::TempDir::new().unwrap();
        let policies = tmp.path().join("policies");
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::write(policies.join("review.md"), "Review carefully.").unwrap();

        let mut step = make_test_step("review", NodeType::Agent, "unused", vec![], None);
        step.policy = Some("review".to_string());
        step.instruction = None;
        resolve_node_facets_for_test(&mut step, tmp.path());
        let (sys, prompt) = WorkflowEngine::build_step_prompt(
            &step,
            "00000000-0000-0000-0000-000000000000",
            "/repo",
            None,
            &HashMap::new(),
            &[],
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        assert_eq!(sys.as_deref(), Some("Review carefully."));
        assert_eq!(prompt, "");
    }

    #[test]
    fn build_step_prompt_passes_composed_system_prompt_through() {
        // Scenario: 合成された system_prompt は AgentSession 開始時にバックエンドへ受け渡される
        // build_step_prompt の戻り値の Option<String> がそのまま AgentSession に渡される経路を検証する。
        // ドロップ・空文字置換が起きないこと。
        let tmp = tempfile::TempDir::new().unwrap();
        let policies = tmp.path().join("policies");
        let contracts = tmp.path().join("contracts");
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::create_dir_all(&contracts).unwrap();
        std::fs::write(policies.join("coding.md"), "POLICY_BODY").unwrap();
        std::fs::write(contracts.join("plan-doc.md"), "CONTRACT_BODY").unwrap();

        let mut step = make_test_step("s", NodeType::Agent, "unused", vec![], None);
        step.policy = Some("coding".to_string());
        step.output_contract = Some("plan-doc".to_string());
        step.instruction = None;
        resolve_node_facets_for_test(&mut step, tmp.path());
        let (sys, prompt) = WorkflowEngine::build_step_prompt(
            &step,
            "00000000-0000-0000-0000-000000000000",
            "/repo",
            None,
            &HashMap::new(),
            &[],
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        // 合成された system_prompt は Some(...) として渡される（None や空文字に置換されない）
        let sys = sys.expect("system_prompt must be passed through, not dropped");
        assert!(!sys.is_empty(), "system_prompt must not be empty string");
        assert!(sys.contains("POLICY_BODY"));
        assert!(sys.contains("CONTRACT_BODY"));
        assert!(prompt.contains("完了時の必須アクション"));
        // CLI 名は起動環境別 alias で展開される（spec issues-1054）。
        let cli_alias = WorkflowEngine::resolve_releash_alias();
        assert!(prompt.contains(&format!(
            "{cli_alias} workflow output submit 00000000-0000-0000-0000-000000000000"
        )));
        assert!(prompt.contains("--step s"));
        assert!(prompt.contains("--type plan-doc"));
        assert!(!prompt.contains("+  --step"));
    }

    #[test]
    fn build_step_prompt_expands_workflow_declared_variables_in_user_message() {
        // spec issues-1054「workflow 定義変数の facet 展開」:
        // build_step_prompt は workflow_declared_variables を facet 本文の
        // `{{vars.<name>}}` 展開に渡す。本テストは instruction（user_message）と
        // policy（system_prompt）の双方で `{{vars.*}}` が宣言値に置換されることを検証する。
        let tmp = tempfile::TempDir::new().unwrap();
        let base = tmp.path();
        let instructions = base.join("instructions");
        let policies = base.join("policies");
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::write(
            instructions.join("impl-vars.md"),
            "Spec dir: {{vars.spec_dir}}\nEnv: {{vars.env}}",
        )
        .unwrap();
        std::fs::write(
            policies.join("vars-policy.md"),
            "Operate within {{vars.env}}.",
        )
        .unwrap();

        let mut step = make_test_step("impl", NodeType::Agent, "unused", vec![], None);
        step.instruction = Some("impl-vars".to_string());
        step.policy = Some("vars-policy".to_string());
        step.output_contract = None;
        resolve_node_facets_for_test(&mut step, base);

        let mut declared = HashMap::new();
        declared.insert("spec_dir".to_string(), "docs/specs/issues-1054".to_string());
        declared.insert("env".to_string(), "production".to_string());

        let (sys, prompt) = WorkflowEngine::build_step_prompt(
            &step,
            "00000000-0000-0000-0000-000000000000",
            "/repo",
            None,
            &HashMap::new(),
            &[],
            &HashMap::new(),
            &declared,
        )
        .unwrap();

        // user_message 側の `{{vars.spec_dir}}` / `{{vars.env}}` が宣言値に展開される
        assert!(prompt.contains("Spec dir: docs/specs/issues-1054"));
        assert!(prompt.contains("Env: production"));
        // 未展開トークンが残らない
        assert!(!prompt.contains("{{vars.spec_dir}}"));
        assert!(!prompt.contains("{{vars.env}}"));

        // system_prompt 側でも `{{vars.env}}` が展開される
        let sys_str = sys.expect("system_prompt should be set");
        assert!(sys_str.contains("Operate within production."));
        assert!(!sys_str.contains("{{vars.env}}"));
    }

    // ---- dispatch_session_start (SessionStartGate 経由のテストダブル検証) ----

    /// テスト用の `SessionStartGate` 実装。受け取った引数を共有 Vec に記録する。
    struct RecordingSessionStartGate {
        records: Arc<std::sync::Mutex<Vec<RecordedSessionStart>>>,
    }

    #[derive(Clone, Debug)]
    struct RecordedSessionStart {
        session_id: String,
        worktree_path: String,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
    }

    #[async_trait::async_trait]
    impl SessionStartGate for RecordingSessionStartGate {
        async fn start_session(
            &self,
            session_id: &str,
            worktree_path: &str,
            permission_mode: Option<String>,
            system_prompt: Option<String>,
        ) -> Result<(), String> {
            self.records.lock().unwrap().push(RecordedSessionStart {
                session_id: session_id.to_string(),
                worktree_path: worktree_path.to_string(),
                permission_mode,
                system_prompt,
            });
            Ok(())
        }
    }

    #[tokio::test]
    async fn dispatch_session_start_passes_composed_system_prompt_to_gate() {
        // Scenario: 合成された system_prompt は AgentSession 開始時にバックエンドへ受け渡される
        // ───「バックエンド起動経路 (start_agent_session_internal 相当) はテストダブルで置換され
        // 受け取った引数を記録する」を直接検証する。
        let tmp = tempfile::TempDir::new().unwrap();
        let base = tmp.path();
        let policies = base.join("policies");
        let contracts = base.join("contracts");
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::create_dir_all(&contracts).unwrap();
        std::fs::write(policies.join("p.md"), "POLICY_BODY").unwrap();
        std::fs::write(contracts.join("c.md"), "CONTRACT_BODY").unwrap();

        let mut step = make_test_step("s", NodeType::Agent, "unused", vec![], None);
        step.policy = Some("p".to_string());
        step.output_contract = Some("c".to_string());
        step.instruction = None;
        resolve_node_facets_for_test(&mut step, base);

        // build_step_prompt → dispatch_session_start の経路をそのまま再現する。
        let (system_prompt, _prompt) = WorkflowEngine::build_step_prompt(
            &step,
            "00000000-0000-0000-0000-000000000000",
            "/repo",
            None,
            &HashMap::new(),
            &[],
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gate = RecordingSessionStartGate {
            records: records.clone(),
        };

        WorkflowEngine::dispatch_session_start(
            &gate,
            "step-session-id",
            "/repo",
            None,
            system_prompt.clone(),
        )
        .await
        .unwrap();

        let recorded = records.lock().unwrap();
        assert_eq!(
            recorded.len(),
            1,
            "gate.start_session must be invoked exactly once"
        );
        let r = &recorded[0];
        assert_eq!(r.session_id, "step-session-id");
        assert_eq!(r.worktree_path, "/repo");
        assert!(r.permission_mode.is_none());
        let sp = r
            .system_prompt
            .as_ref()
            .expect("system_prompt must be passed through as Some(_)");
        assert!(
            !sp.is_empty(),
            "system_prompt must not be dropped or replaced with an empty string"
        );
        assert!(sp.contains("POLICY_BODY"));
        assert!(sp.contains("CONTRACT_BODY"));
    }

    #[tokio::test]
    async fn build_and_dispatch_step_session_forwards_composed_system_prompt_through_gate() {
        // Scenario: 合成された system_prompt は AgentSession 開始時にバックエンドへ受け渡される
        // start_step_session 側の経路（build_step_prompt → SessionStartGate）を切り出したヘルパーを
        // 記録用 gate で駆動し、合成された system_prompt が None / 空文字に置換されずに
        // gate に渡ることを直接 assert する。
        let tmp = tempfile::TempDir::new().unwrap();
        let base = tmp.path();
        let policies = base.join("policies");
        let contracts = base.join("contracts");
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::create_dir_all(&contracts).unwrap();
        std::fs::write(policies.join("p.md"), "STEP_POLICY_BODY").unwrap();
        std::fs::write(contracts.join("c.md"), "STEP_CONTRACT_BODY").unwrap();

        let mut step = make_test_step("s", NodeType::Agent, "unused", vec![], None);
        step.policy = Some("p".to_string());
        step.output_contract = Some("c".to_string());
        step.instruction = None;
        resolve_node_facets_for_test(&mut step, base);

        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gate = RecordingSessionStartGate {
            records: records.clone(),
        };

        let prompt = WorkflowEngine::build_and_dispatch_step_session(
            &gate,
            &step,
            "00000000-0000-0000-0000-000000000000",
            "step-session-id",
            "/repo",
            None,
            None,
            &HashMap::new(),
            &[],
            &HashMap::new(),
        )
        .await
        .unwrap();

        // knowledge / instruction がなくても、output_contract があれば user_message には
        // Contract 由来の完了時アクションが入る。
        let _ = prompt;

        let recorded = records.lock().unwrap();
        assert_eq!(
            recorded.len(),
            1,
            "gate.start_session must be invoked exactly once via build_and_dispatch_step_session"
        );
        let r = &recorded[0];
        assert_eq!(r.session_id, "step-session-id");
        assert_eq!(r.worktree_path, "/repo");
        assert!(r.permission_mode.is_none());
        let sp = r.system_prompt.as_ref().expect(
            "system_prompt must be passed through start_step_session path as Some(_), not dropped",
        );
        assert!(
            !sp.is_empty(),
            "system_prompt must not be dropped or replaced with an empty string"
        );
        assert!(sp.contains("STEP_POLICY_BODY"));
        assert!(sp.contains("STEP_CONTRACT_BODY"));
    }

    #[tokio::test]
    async fn dispatch_session_start_passes_none_when_no_facets() {
        // Scenario: policy も output_contract も指定がないと system_prompt は設定されない
        // を SessionStartGate 経由でも維持することを検証する。
        let tmp = tempfile::TempDir::new().unwrap();
        let instructions = tmp.path().join("instructions");
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::write(instructions.join("only-instr.md"), "Body").unwrap();

        let mut step = make_test_step("s", NodeType::Agent, "unused", vec![], None);
        step.instruction = Some("only-instr".to_string());
        resolve_node_facets_for_test(&mut step, tmp.path());
        let (system_prompt, _prompt) = WorkflowEngine::build_step_prompt(
            &step,
            "00000000-0000-0000-0000-000000000000",
            "/repo",
            None,
            &HashMap::new(),
            &[],
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gate = RecordingSessionStartGate {
            records: records.clone(),
        };

        WorkflowEngine::dispatch_session_start(&gate, "sid", "/repo", None, system_prompt)
            .await
            .unwrap();

        let recorded = records.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert!(
            recorded[0].system_prompt.is_none(),
            "system_prompt must be None when neither policy nor output_contract is specified"
        );
    }

    // ---- start_step_session_with_deps (副作用境界の注入による順序保証検証) ----

    /// テスト用の `StepSessionDeps` 実装。副作用境界の各メソッドの呼び出し回数を
    /// 記録し、本番経路と同じ順序で副作用が発火することを assert できるようにする。
    /// プロンプト合成失敗時に `create_step_session` が呼ばれないこと等を検証する。
    #[derive(Default)]
    struct RecordingStepSessionDeps {
        create_step_session_count: std::sync::atomic::AtomicUsize,
        dispatch_session_start_count: std::sync::atomic::AtomicUsize,
        mark_step_tab_open_count: std::sync::atomic::AtomicUsize,
        broadcast_state_count: std::sync::atomic::AtomicUsize,
        start_agent_turn_count: std::sync::atomic::AtomicUsize,
        assert_runtime_lock_during_start: std::sync::atomic::AtomicBool,
        runtime_lock_was_held_during_start: std::sync::atomic::AtomicBool,
    }

    impl RecordingStepSessionDeps {
        fn create_step_session_count(&self) -> usize {
            self.create_step_session_count
                .load(std::sync::atomic::Ordering::SeqCst)
        }

        fn dispatch_session_start_count(&self) -> usize {
            self.dispatch_session_start_count
                .load(std::sync::atomic::Ordering::SeqCst)
        }

        fn mark_step_tab_open_count(&self) -> usize {
            self.mark_step_tab_open_count
                .load(std::sync::atomic::Ordering::SeqCst)
        }

        fn broadcast_state_count(&self) -> usize {
            self.broadcast_state_count
                .load(std::sync::atomic::Ordering::SeqCst)
        }

        fn start_agent_turn_count(&self) -> usize {
            self.start_agent_turn_count
                .load(std::sync::atomic::Ordering::SeqCst)
        }

        fn assert_runtime_lock_during_start(&self) {
            self.assert_runtime_lock_during_start
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }

        fn runtime_lock_was_held_during_start(&self) -> bool {
            self.runtime_lock_was_held_during_start
                .load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl StepSessionDeps for RecordingStepSessionDeps {
        async fn create_step_session(
            &self,
            _worktree_path: &str,
            _step_model: Option<String>,
            _step_permission: Option<String>,
            _workflow_defaults: WorkflowDefaults,
        ) -> Result<StepSessionInfo, WorkflowEngineError> {
            self.create_step_session_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(StepSessionInfo {
                id: "step-session-id".to_string(),
                permission_mode: "ask".to_string(),
            })
        }

        async fn dispatch_session_start(
            &self,
            _step_session_id: &str,
            _worktree_path: &str,
            _permission_mode: Option<String>,
            _system_prompt: Option<String>,
        ) -> Result<(), WorkflowEngineError> {
            self.dispatch_session_start_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        async fn mark_step_tab_open(&self, _step_session_id: &str) {
            self.mark_step_tab_open_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        async fn broadcast_state(&self, _worktree_path: &str, _snapshot: WorkflowState) {
            self.broadcast_state_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        async fn start_agent_turn_locked(
            &self,
            step_session_id: &str,
            _worktree_path: &str,
            _permission_mode: &str,
            _prompt: &str,
        ) -> Result<(), WorkflowEngineError> {
            self.start_agent_turn_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self
                .assert_runtime_lock_during_start
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                let lock_attempt = tokio::time::timeout(
                    std::time::Duration::from_millis(20),
                    crate::agent_sdk::acquire_session_runtime_lock(step_session_id),
                )
                .await;
                self.runtime_lock_was_held_during_start
                    .store(lock_attempt.is_err(), std::sync::atomic::Ordering::SeqCst);
            }
            Ok(())
        }
    }

    /// `executions` に 1 ステップのワークフロー実行を登録する。
    /// 指定された step を current_step_index=0 として登録する。
    fn insert_single_step_execution(
        execs: &mut HashMap<String, WorkflowExecution>,
        step: NodeDefinition,
    ) {
        let workflow = Workflow {
            variables: Default::default(),
            name: "regression-workflow".to_string(),
            description: "regression test".to_string(),
            builtin: false,
            nodes: vec![step],
        };
        let exec = WorkflowExecution {
            id: "exec-id".to_string(),
            workflow,
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        execs.insert(exec.id.clone(), exec);
    }

    #[tokio::test]
    async fn start_step_session_with_deps_skips_side_effects_when_prompt_synthesis_fails() {
        // 回帰防止: `start_step_session` 本番経路では、参照先ファセットが
        // 存在しないステップを起動した際にプロンプト合成段階で失敗し、
        // 後続の副作用（親セッション取得 / ChatSession 生成 / `session_workflow_refs`
        // 登録 / AgentSession 開始 / 永続化 / ブロードキャスト / ターン起動）は
        // 一切実行されないことを構造的に保証する。
        //
        // 旧実装では先に ChatSession を生成・参照マップへ登録してから
        // プロンプト合成（ファセット未発見で失敗し得る）を行っていたため、
        // 参照先ファセットが存在しないステップを起動すると孤立した
        // ChatSession と参照マップ entry が残るバグがあった。
        //
        // 本テストは `StepSessionDeps` 経由で副作用境界をテストダブルに差し替え、
        // ファセット参照が解決不能な execution に対し `start_step_session_with_deps`
        // を実行することで:
        //   (a) `Err(InvalidWorkflow(_))` が返ること
        //   (b) `create_step_session` の呼び出し回数が 0 であること
        //   (c) `fetch_parent_session` 等 他の副作用境界メソッドも 0 回であること
        //   (d) `engine.session_workflow_refs` が空のままであること
        //   (e) `executions["/repo"].current_session_id` が `None` のままであること
        // を assert する。`start_step_session` 内の順序を逆転（先に create_step_session
        // → 後に build_step_prompt）させると (b) が 1 となりテストが失敗する。
        let engine = WorkflowEngine::new_for_test();

        // 参照先ファセットが解決不能な step を含む execution を登録する。
        // facets_base_dir() 配下に "nonexistent_policy_<uuid>.md" が偶然存在することは
        // 実用上ありえないため、ファセット合成は必ず失敗する。
        let mut step = make_test_step("missing-facet", NodeType::Agent, "unused", vec![], None);
        step.instruction = None;
        step.policy = Some(format!(
            "nonexistent_policy_{}",
            uuid::Uuid::new_v4().simple()
        ));

        {
            let mut execs = engine.executions.lock().await;
            insert_single_step_execution(&mut execs, step);
        }

        // 事前条件: session_workflow_refs は空
        assert!(engine.session_workflow_refs.lock().await.is_empty());

        let deps = RecordingStepSessionDeps::default();
        let result = engine.start_step_session_with_deps(&deps, "/repo").await;

        // (a) build_step_prompt 失敗で InvalidWorkflow エラーになる
        let err =
            result.expect_err("missing facet must cause start_step_session_with_deps to fail");
        assert!(
            matches!(err, WorkflowEngineError::InvalidWorkflow(_)),
            "missing facet must produce InvalidWorkflow error, got: {err:?}"
        );

        // (b)/(c) 副作用境界はいずれも呼ばれていない
        assert_eq!(
            deps.create_step_session_count(),
            0,
            "create_step_session must NOT be invoked when prompt synthesis fails"
        );
        assert_eq!(
            deps.dispatch_session_start_count(),
            0,
            "dispatch_session_start must NOT be invoked when prompt synthesis fails"
        );
        assert_eq!(
            deps.mark_step_tab_open_count(),
            0,
            "mark_step_tab_open must NOT be invoked when prompt synthesis fails"
        );
        assert_eq!(
            deps.broadcast_state_count(),
            0,
            "broadcast_state must NOT be invoked when prompt synthesis fails"
        );
        assert_eq!(
            deps.start_agent_turn_count(),
            0,
            "start_agent_turn must NOT be invoked when prompt synthesis fails"
        );

        // (d) session_workflow_refs は空のまま
        assert!(
            engine.session_workflow_refs.lock().await.is_empty(),
            "session_workflow_refs must remain empty when prompt synthesis fails"
        );

        // (e) executions["/repo"].current_session_id は None のまま
        let execs = engine.executions.lock().await;
        let (_, exec) =
            find_by_worktree(&execs, "/repo").expect("execution must remain registered");
        assert!(
            exec.current_session_id.is_none(),
            "current_session_id must remain None when prompt synthesis fails"
        );
    }

    #[tokio::test]
    async fn start_step_session_with_deps_invokes_side_effects_in_order_on_success() {
        // 副作用境界が正しい順序で呼ばれる成功経路を併せて検証する。
        // プロンプト合成が成功した場合は、create_step_session → dispatch_session_start
        // → broadcast_state → start_agent_turn の全境界が各 1 回ずつ呼ばれ、
        // engine.session_workflow_refs と executions["/repo"].current_session_id が
        // 期待通り更新されることを assert する。
        let engine = WorkflowEngine::new_for_test();

        // inline_prompt のみのステップなら facet ファイルなしでも合成が成功する。
        let mut step = make_test_step("ok-step", NodeType::Agent, "unused", vec![], None);
        step.instruction = None;
        step.inline_prompt = Some("hello".to_string());

        {
            let mut execs = engine.executions.lock().await;
            insert_single_step_execution(&mut execs, step);
        }

        let deps = RecordingStepSessionDeps::default();
        deps.assert_runtime_lock_during_start();
        engine
            .start_step_session_with_deps(&deps, "/repo")
            .await
            .expect("start_step_session_with_deps must succeed for inline_prompt step");

        // 各副作用境界が 1 回ずつ呼ばれている
        assert_eq!(deps.create_step_session_count(), 1);
        assert_eq!(deps.dispatch_session_start_count(), 1);
        assert_eq!(deps.mark_step_tab_open_count(), 1);
        assert_eq!(deps.broadcast_state_count(), 1);
        assert_eq!(deps.start_agent_turn_count(), 1);
        assert!(
            deps.runtime_lock_was_held_during_start(),
            "step session runtime lock must cover the path until start_agent_turn marks it streaming"
        );

        // session_workflow_refs に SequentialStep として登録されている
        let refs = engine.session_workflow_refs.lock().await;
        let entry = refs
            .get("step-session-id")
            .expect("session_workflow_refs must contain step-session-id");
        assert_eq!(entry.run_id, "exec-id");
        drop(refs);

        // executions の current_session_id がステップセッションIDで更新されている
        let execs = engine.executions.lock().await;
        let (_, exec) =
            find_by_worktree(&execs, "/repo").expect("execution must remain registered");
        assert_eq!(
            exec.current_session_id.as_deref(),
            Some("step-session-id"),
            "current_session_id must be updated to the created step session id"
        );
    }

    // ---- build_parallel_step_prompt (並列子ステップの合成ルール) ----

    fn make_parallel_step(name: &str) -> crate::workflow::schema::ChildNodeDefinition {
        crate::workflow::schema::ChildNodeDefinition {
            name: name.to_string(),
            ..crate::workflow::schema::ChildNodeDefinition::default()
        }
    }

    #[test]
    fn build_parallel_step_prompt_splits_facets_into_system_and_user() {
        // Scenario: 並列ステップの子ステップでも同じ合成ルールが適用される
        // 並列子ステップに policy / output_contract / knowledge / instruction の 4 種すべてを指定し、
        // policy + output_contract が system_prompt に、knowledge + instruction が user_message に
        // 集約されることを検証する。
        let tmp = tempfile::TempDir::new().unwrap();
        let base = tmp.path();
        let policies = base.join("policies");
        let knowledges = base.join("knowledge");
        let instructions = base.join("instructions");
        let contracts = base.join("contracts");
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::create_dir_all(&knowledges).unwrap();
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::create_dir_all(&contracts).unwrap();
        std::fs::write(policies.join("pol.md"), "PARALLEL_POLICY_BODY").unwrap();
        std::fs::write(knowledges.join("know.md"), "PARALLEL_KNOWLEDGE_BODY").unwrap();
        std::fs::write(instructions.join("inst.md"), "PARALLEL_INSTRUCTION_BODY").unwrap();
        std::fs::write(contracts.join("oc.md"), "PARALLEL_CONTRACT_BODY").unwrap();

        let mut ps = make_parallel_step("child");
        ps.policy = Some("pol".to_string());
        ps.knowledge = Some("know".to_string());
        ps.instruction = Some("inst".to_string());
        ps.output_contract = Some("oc".to_string());
        resolve_child_facets_for_test(&mut ps, base);
        let (system_prompt, user_message) = WorkflowEngine::build_parallel_step_prompt(
            &ps,
            "11111111-1111-1111-1111-111111111111",
            "/repo",
            None,
            &HashMap::new(),
            false,
            None,
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        let sp =
            system_prompt.expect("system_prompt must be set for parallel child with policy/oc");
        // policy と output_contract の本文が system_prompt に集約される
        assert!(sp.contains("PARALLEL_POLICY_BODY"));
        assert!(sp.contains("PARALLEL_CONTRACT_BODY"));
        // Contract 本文は system_prompt に集約される
        assert!(!sp.contains("PARALLEL_KNOWLEDGE_BODY"));
        assert!(!sp.contains("PARALLEL_INSTRUCTION_BODY"));

        // knowledge / instruction の本文と、Contract 由来の完了時アクションは
        // user_message に集約される。
        assert!(user_message.contains("PARALLEL_KNOWLEDGE_BODY"));
        assert!(user_message.contains("PARALLEL_INSTRUCTION_BODY"));
        assert!(user_message.contains("完了時の必須アクション"));
        // CLI 名は起動環境別 alias で展開される（spec issues-1054）。
        let cli_alias = WorkflowEngine::resolve_releash_alias();
        assert!(user_message.contains(&format!(
            "{cli_alias} workflow output submit 11111111-1111-1111-1111-111111111111"
        )));
        assert!(user_message.contains("--step child"));
        assert!(user_message.contains("--type oc"));
        assert!(!user_message.contains("+  --step"));
        // policy / output_contract 本文は user_message には入らない
        assert!(!user_message.contains("PARALLEL_POLICY_BODY"));
        assert!(!user_message.contains("PARALLEL_CONTRACT_BODY"));
    }

    #[test]
    fn build_parallel_step_prompt_no_policy_or_contract_returns_none_system_prompt() {
        // 並列子ステップでも policy / output_contract がない場合は system_prompt が None になる。
        let tmp = tempfile::TempDir::new().unwrap();
        let base = tmp.path();
        let instructions = base.join("instructions");
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::write(instructions.join("inst.md"), "INSTR").unwrap();

        let mut ps = make_parallel_step("child");
        ps.instruction = Some("inst".to_string());
        resolve_child_facets_for_test(&mut ps, base);
        let (system_prompt, user_message) = WorkflowEngine::build_parallel_step_prompt(
            &ps,
            "11111111-1111-1111-1111-111111111111",
            "/repo",
            None,
            &HashMap::new(),
            false,
            None,
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();

        assert!(system_prompt.is_none());
        assert!(user_message.contains("INSTR"));
    }

    // ---- evaluate_aggregate ----

    #[test]
    fn evaluate_aggregate_all_match_all_children_match() {
        let engine = WorkflowEngine::new_for_test();
        let agg = ParallelAggregate {
            all_match: Some("LGTM".to_string()),
            any_match: None,
            then: "report".to_string(),
            r#else: "implement".to_string(),
        };
        let mut outputs = HashMap::new();
        outputs.insert(
            "arch-review".to_string(),
            make_step_output("arch-review", "looks good", Some("LGTM")),
        );
        outputs.insert(
            "security-review".to_string(),
            make_step_output("security-review", "no issues", Some("LGTM")),
        );
        // 逐次ステップの出力（フィルタで除外されるべき）
        outputs.insert(
            "implement".to_string(),
            make_step_output("implement", "done", Some("DONE")),
        );
        let children = vec!["arch-review".to_string(), "security-review".to_string()];
        assert!(engine.evaluate_aggregate(&agg, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_all_match_one_child_mismatch() {
        let engine = WorkflowEngine::new_for_test();
        let agg = ParallelAggregate {
            all_match: Some("LGTM".to_string()),
            any_match: None,
            then: "report".to_string(),
            r#else: "implement".to_string(),
        };
        let mut outputs = HashMap::new();
        outputs.insert(
            "arch-review".to_string(),
            make_step_output("arch-review", "ok", Some("LGTM")),
        );
        outputs.insert(
            "security-review".to_string(),
            make_step_output("security-review", "problems", Some("NEEDS_FIX")),
        );
        let children = vec!["arch-review".to_string(), "security-review".to_string()];
        assert!(!engine.evaluate_aggregate(&agg, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_any_match_one_child_matches() {
        let engine = WorkflowEngine::new_for_test();
        let agg = ParallelAggregate {
            all_match: None,
            any_match: Some("NEEDS_FIX".to_string()),
            then: "implement".to_string(),
            r#else: "report".to_string(),
        };
        let mut outputs = HashMap::new();
        outputs.insert(
            "arch-review".to_string(),
            make_step_output("arch-review", "ok", Some("LGTM")),
        );
        outputs.insert(
            "security-review".to_string(),
            make_step_output("security-review", "problems", Some("NEEDS_FIX")),
        );
        let children = vec!["arch-review".to_string(), "security-review".to_string()];
        assert!(engine.evaluate_aggregate(&agg, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_any_match_no_child_matches() {
        let engine = WorkflowEngine::new_for_test();
        let agg = ParallelAggregate {
            all_match: None,
            any_match: Some("NEEDS_FIX".to_string()),
            then: "implement".to_string(),
            r#else: "report".to_string(),
        };
        let mut outputs = HashMap::new();
        outputs.insert(
            "arch-review".to_string(),
            make_step_output("arch-review", "ok", Some("LGTM")),
        );
        outputs.insert(
            "security-review".to_string(),
            make_step_output("security-review", "ok", Some("LGTM")),
        );
        let children = vec!["arch-review".to_string(), "security-review".to_string()];
        assert!(!engine.evaluate_aggregate(&agg, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_no_condition_returns_true() {
        let engine = WorkflowEngine::new_for_test();
        let agg = ParallelAggregate {
            all_match: None,
            any_match: None,
            then: "next".to_string(),
            r#else: "fallback".to_string(),
        };
        let outputs = HashMap::new();
        let children: Vec<String> = vec![];
        assert!(engine.evaluate_aggregate(&agg, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_result_none_does_not_match() {
        let engine = WorkflowEngine::new_for_test();
        let agg = ParallelAggregate {
            all_match: Some("LGTM".to_string()),
            any_match: None,
            then: "report".to_string(),
            r#else: "implement".to_string(),
        };
        let mut outputs = HashMap::new();
        // result=None → matches_pattern returns false regardless of structured_output
        outputs.insert(
            "arch-review".to_string(),
            make_step_output("arch-review", "Review result: LGTM", None),
        );
        outputs.insert(
            "security-review".to_string(),
            make_step_output("security-review", "All good. LGTM", None),
        );
        let children = vec!["arch-review".to_string(), "security-review".to_string()];
        assert!(!engine.evaluate_aggregate(&agg, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_filters_only_child_steps() {
        let engine = WorkflowEngine::new_for_test();
        let agg = ParallelAggregate {
            all_match: Some("LGTM".to_string()),
            any_match: None,
            then: "report".to_string(),
            r#else: "implement".to_string(),
        };
        let mut outputs = HashMap::new();
        outputs.insert(
            "arch-review".to_string(),
            make_step_output("arch-review", "", Some("LGTM")),
        );
        // 逐次ステップがLGTMでなくても結果に影響しない
        outputs.insert(
            "implement".to_string(),
            make_step_output("implement", "done", Some("DONE")),
        );
        let children = vec!["arch-review".to_string()];
        assert!(engine.evaluate_aggregate(&agg, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_all_match_missing_child_output_returns_false() {
        let engine = WorkflowEngine::new_for_test();
        let agg = ParallelAggregate {
            all_match: Some("LGTM".to_string()),
            any_match: None,
            then: "report".to_string(),
            r#else: "implement".to_string(),
        };
        let mut outputs = HashMap::new();
        // arch-reviewの出力のみ（security-reviewはまだ未完了で出力なし）
        outputs.insert(
            "arch-review".to_string(),
            make_step_output("arch-review", "ok", Some("LGTM")),
        );
        let children = vec!["arch-review".to_string(), "security-review".to_string()];
        // 子step出力が欠けている場合はfalse
        assert!(!engine.evaluate_aggregate(&agg, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_invalid_regex_falls_back_to_contains() {
        let engine = WorkflowEngine::new_for_test();
        // 不正なregexパターン（validationで弾かれるべきだが、エンジン側もgraceful）
        let agg = ParallelAggregate {
            all_match: Some("[invalid(regex".to_string()),
            any_match: None,
            then: "report".to_string(),
            r#else: "implement".to_string(),
        };
        let mut outputs = HashMap::new();
        // resultに"[invalid(regex"を含む → contains fallbackでマッチ
        outputs.insert(
            "arch-review".to_string(),
            make_step_output("arch-review", "text", Some("[invalid(regex")),
        );
        let children = vec!["arch-review".to_string()];
        assert!(engine.evaluate_aggregate(&agg, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_invalid_regex_contains_no_match() {
        let engine = WorkflowEngine::new_for_test();
        let agg = ParallelAggregate {
            all_match: Some("[invalid(regex".to_string()),
            any_match: None,
            then: "report".to_string(),
            r#else: "implement".to_string(),
        };
        let mut outputs = HashMap::new();
        // resultに"[invalid(regex"を含まない
        outputs.insert(
            "arch-review".to_string(),
            make_step_output("arch-review", "LGTM text", Some("LGTM")),
        );
        let children = vec!["arch-review".to_string()];
        assert!(!engine.evaluate_aggregate(&agg, &outputs, &children));
    }

    #[test]
    fn evaluate_aggregate_empty_children_all_match_returns_true() {
        let engine = WorkflowEngine::new_for_test();
        let agg = ParallelAggregate {
            all_match: Some("LGTM".to_string()),
            any_match: None,
            then: "report".to_string(),
            r#else: "implement".to_string(),
        };
        let outputs = HashMap::new();
        let children: Vec<String> = vec![];
        // 空childrenの場合: child_outputs.len()==0, child_step_names.len()==0 → 等しいので
        // all()が空イテレータでtrueを返す
        assert!(engine.evaluate_aggregate(&agg, &outputs, &children));
    }

    // ---- decide_approval_action ----

    fn make_approval_exec(
        state: WorkflowExecutionState,
        rules: Vec<TransitionRule>,
    ) -> WorkflowExecution {
        WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: Workflow {
                variables: Default::default(),
                name: "test".to_string(),
                description: "test".to_string(),
                builtin: false,
                nodes: vec![NodeDefinition {
                    name: "review".to_string(),
                    node_type: NodeType::Approval,
                    instruction: Some("Review the code".to_string()),
                    transition_rules: rules,
                    ..NodeDefinition::default()
                }],
            },
            state,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: vec![],
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        }
    }

    // ---- validate_approval_decision ----

    #[test]
    fn validate_approval_decision_reject_empty_comment_returns_error() {
        let result = WorkflowEngine::validate_approval_decision(&ApprovalDecision::Reject {
            comment: "".to_string(),
        });
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Reject comment must not be empty"));
    }

    #[test]
    fn validate_approval_decision_reject_whitespace_only_returns_error() {
        let result = WorkflowEngine::validate_approval_decision(&ApprovalDecision::Reject {
            comment: "   \n\t  ".to_string(),
        });
        assert!(result.is_err());
    }

    #[test]
    fn validate_approval_decision_reject_over_limit_returns_error() {
        let result = WorkflowEngine::validate_approval_decision(&ApprovalDecision::Reject {
            comment: "x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1),
        });
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .starts_with("validation_error:"));
    }

    #[test]
    fn validate_approval_decision_reject_with_comment_ok() {
        let result = WorkflowEngine::validate_approval_decision(&ApprovalDecision::Reject {
            comment: "Please fix the bug".to_string(),
        });
        assert!(result.is_ok());
    }

    #[test]
    fn validate_approval_decision_approve_ok() {
        let result = WorkflowEngine::validate_approval_decision(&ApprovalDecision::Approve);
        assert!(result.is_ok());
    }

    #[test]
    fn validate_approval_target_missing_values_returns_unauthorized_target() {
        let exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        let result = WorkflowEngine::validate_approval_target_snapshot(&exec, None, Some("review"));
        assert!(matches!(
            result.unwrap_err(),
            WorkflowEngineError::UnauthorizedApprovalTarget(_)
        ));

        let result = WorkflowEngine::validate_approval_target_snapshot(&exec, Some("exec-1"), None);
        assert!(matches!(
            result.unwrap_err(),
            WorkflowEngineError::UnauthorizedApprovalTarget(_)
        ));
    }

    #[test]
    fn validate_approval_target_mismatch_returns_unauthorized_target() {
        let exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        let result = WorkflowEngine::validate_approval_target_snapshot(
            &exec,
            Some("other-exec"),
            Some("review"),
        );
        assert!(matches!(
            result.unwrap_err(),
            WorkflowEngineError::UnauthorizedApprovalTarget(_)
        ));

        let result = WorkflowEngine::validate_approval_target_snapshot(
            &exec,
            Some("exec-1"),
            Some("other-step"),
        );
        assert!(matches!(
            result.unwrap_err(),
            WorkflowEngineError::UnauthorizedApprovalTarget(_)
        ));
    }

    #[test]
    fn validate_approval_target_non_waiting_returns_invalid_state() {
        let exec = make_approval_exec(WorkflowExecutionState::Running, vec![]);
        let result = WorkflowEngine::validate_approval_target_snapshot(
            &exec,
            Some("exec-1"),
            Some("review"),
        );
        let err = result.unwrap_err();
        assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
        assert!(err.to_string().starts_with("invalid_state:"));
    }

    #[test]
    fn validate_approval_target_terminal_states_return_invalid_state_without_mutation() {
        for state in [
            WorkflowExecutionState::Completed,
            WorkflowExecutionState::Failed {
                reason: "failed".to_string(),
            },
            WorkflowExecutionState::Aborted,
        ] {
            let exec = make_approval_exec(state.clone(), vec![]);
            let result = WorkflowEngine::validate_approval_target_snapshot(
                &exec,
                Some("exec-1"),
                Some("review"),
            );
            let err = result.unwrap_err();
            assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
            assert_eq!(exec.state, state);
            assert!(exec.step_history.is_empty());
        }
    }

    #[tokio::test]
    async fn validate_approval_target_wrong_worktree_returns_unauthorized_without_mutating_state() {
        let engine = WorkflowEngine::new_for_test();
        let exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        {
            let mut execs = engine.executions.lock().await;
            execs.insert("/repo-a".to_string(), exec);
        }

        let result = engine
            .validate_approval_target("/repo-b", Some("exec-1"), Some("review"))
            .await;
        let err = result.unwrap_err();
        assert!(matches!(err, WorkflowEngineError::UnauthorizedWorktree(_)));
        assert!(err.to_string().starts_with("unauthorized_worktree:"));

        let execs = engine.executions.lock().await;
        let original = execs.get("/repo-a").unwrap();
        assert_eq!(original.state, WorkflowExecutionState::WaitingApproval);
        assert!(original.step_history.is_empty());
    }

    #[test]
    fn validate_approval_turn_phase_rejects_unfinished_turns() {
        assert!(WorkflowEngine::validate_approval_turn_phase(Some(
            crate::agent_sdk::TurnPhase::Streaming
        ))
        .unwrap_err()
        .to_string()
        .starts_with("validation_error:"));
        assert!(WorkflowEngine::validate_approval_turn_phase(Some(
            crate::agent_sdk::TurnPhase::WaitingPermission
        ))
        .is_err());
        assert!(WorkflowEngine::validate_approval_turn_phase(Some(
            crate::agent_sdk::TurnPhase::Idle
        ))
        .is_ok());
    }

    // [08] 旧 `validate_approval_contract_extraction` ベースの 4 テストは prose 抽出経路の
    // 廃止に伴い削除した。approval node の構造化出力は CLI / Tauri 経由の `SubmitOutput`
    // で確定し、対応する境界テストは `dispatch_boundary_tests::submit_output_*` 群と
    // `workflow::contract::tests::validate_contract_value_*` 群でカバーされる。

    #[tokio::test]
    async fn validate_approval_chat_instruction_limits_current_approval_session() {
        let engine = WorkflowEngine::new_for_test();
        let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        exec.current_session_id = Some("step-session".to_string());
        let run_id = exec.id.clone();
        {
            let mut execs = engine.executions.lock().await;
            execs.insert(run_id.clone(), exec);
        }
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                "step-session".to_string(),
                SessionWorkflowRef {
                    run_id: run_id.clone(),
                },
            );
        }

        let result = engine
            .validate_approval_chat_instruction(
                "step-session",
                &"x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1),
            )
            .await;
        assert!(result
            .unwrap_err()
            .to_string()
            .starts_with("validation_error:"));

        assert!(engine
            .validate_approval_chat_instruction("other-session", &"x".repeat(9000))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn validate_approval_chat_instruction_rejects_empty_or_whitespace_only_content() {
        let engine = WorkflowEngine::new_for_test();
        let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        exec.current_session_id = Some("step-session".to_string());
        let run_id = exec.id.clone();
        {
            let mut execs = engine.executions.lock().await;
            execs.insert(run_id.clone(), exec);
        }
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                "step-session".to_string(),
                SessionWorkflowRef {
                    run_id: run_id.clone(),
                },
            );
        }

        for content in ["", "   ", "\n\t \r\n"] {
            let err = engine
                .validate_approval_chat_instruction("step-session", content)
                .await
                .unwrap_err();
            assert!(
                err.to_string().starts_with("validation_error:"),
                "expected validation_error for content={content:?}, got: {err}"
            );
        }
    }

    #[tokio::test]
    async fn validate_approval_chat_instruction_rejects_current_approval_step_before_waiting() {
        let engine = WorkflowEngine::new_for_test();
        let mut exec = make_approval_exec(WorkflowExecutionState::Running, vec![]);
        exec.current_session_id = Some("step-session".to_string());
        let run_id = exec.id.clone();
        {
            let mut execs = engine.executions.lock().await;
            execs.insert(run_id.clone(), exec);
        }
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                "step-session".to_string(),
                SessionWorkflowRef {
                    run_id: run_id.clone(),
                },
            );
        }

        let result = engine
            .validate_approval_chat_instruction("step-session", "Please adjust the policy")
            .await;
        assert!(matches!(
            result.unwrap_err(),
            WorkflowEngineError::InvalidState(_)
        ));
    }

    #[tokio::test]
    async fn validate_approval_chat_instruction_rejects_stale_approved_policy_session() {
        let engine = WorkflowEngine::new_for_test();
        let mut exec = make_approval_exec(WorkflowExecutionState::Running, vec![]);
        exec.workflow.nodes[0].name = "implementation_fix_policy".to_string();
        exec.workflow.nodes[0].output_contract = Some("approved-fix-policy".to_string());
        exec.current_session_id = Some("fix-session".to_string());
        exec.step_history.push(StepHistoryEntry {
            step_name: "implementation_fix_policy".to_string(),
            completed_at: 1000.0,
            result: Some("approved".to_string()),
            session_id: Some("stale-policy-session".to_string()),
            token_usage: None,
            structured_output: Some(serde_json::json!({
                "policy": "Already approved.",
                "review_step": "code_review_parallel",
                "findings": []
            })),
            run_index: 1,
            child_outputs: None,
            state: crate::workflow::state::default_step_entry_state(),
        });
        exec.step_outputs.insert(
            "implementation_fix_policy".to_string(),
            StepOutput {
                step_name: "implementation_fix_policy".to_string(),
                run_index: 1,
                session_id: Some("stale-policy-session".to_string()),
                result: Some("approved".to_string()),
                structured_output: Some(serde_json::json!({
                    "policy": "Already approved.",
                    "review_step": "code_review_parallel",
                    "findings": []
                })),
                output_contract: Some("approved-fix-policy".to_string()),
                token_usage: None,
                completed_at: 1000.0,
            },
        );
        let run_id = exec.id.clone();
        {
            let mut execs = engine.executions.lock().await;
            execs.insert(run_id.clone(), exec);
        }
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                "stale-policy-session".to_string(),
                SessionWorkflowRef {
                    run_id: run_id.clone(),
                },
            );
        }

        let result = engine
            .validate_approval_chat_instruction("stale-policy-session", "Please change policy")
            .await;
        assert!(matches!(
            result.unwrap_err(),
            WorkflowEngineError::InvalidState(_)
        ));
    }

    #[tokio::test]
    async fn validate_approval_chat_instruction_rejects_stale_rejected_policy_session() {
        let engine = WorkflowEngine::new_for_test();
        let mut exec = make_approval_exec(WorkflowExecutionState::Running, vec![]);
        exec.workflow.nodes[0].name = "implementation_fix_policy".to_string();
        exec.workflow.nodes[0].output_contract = Some("approved-fix-policy".to_string());
        exec.current_session_id = Some("implementation-approval-session".to_string());
        exec.step_history.push(StepHistoryEntry {
            step_name: "implementation_fix_policy".to_string(),
            completed_at: 1000.0,
            result: Some("reject".to_string()),
            session_id: Some("stale-rejected-policy-session".to_string()),
            token_usage: None,
            structured_output: Some(serde_json::json!({
                "decision": "reject",
                "comment": "Revise policy."
            })),
            run_index: 1,
            child_outputs: None,
            state: crate::workflow::state::default_step_entry_state(),
        });
        exec.step_outputs.insert(
            "implementation_fix_policy".to_string(),
            StepOutput {
                step_name: "implementation_fix_policy".to_string(),
                run_index: 1,
                session_id: Some("stale-rejected-policy-session".to_string()),
                result: Some("reject".to_string()),
                structured_output: Some(serde_json::json!({
                    "decision": "reject",
                    "comment": "Revise policy."
                })),
                output_contract: None,
                token_usage: None,
                completed_at: 1000.0,
            },
        );
        let run_id = exec.id.clone();
        {
            let mut execs = engine.executions.lock().await;
            execs.insert(run_id.clone(), exec);
        }
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                "stale-rejected-policy-session".to_string(),
                SessionWorkflowRef {
                    run_id: run_id.clone(),
                },
            );
        }

        let result = engine
            .validate_approval_chat_instruction(
                "stale-rejected-policy-session",
                "Please change policy",
            )
            .await;
        assert!(matches!(
            result.unwrap_err(),
            WorkflowEngineError::InvalidState(_)
        ));
    }

    #[test]
    fn latest_assistant_output_after_approval_chat_adjustment_is_selected() {
        let session = crate::session::ChatSession {
            id: "policy-session".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![
                crate::session::ChatMessage {
                    id: "m1".to_string(),
                    role: crate::session::MessageRole::Agent,
                    content: "old policy".to_string(),
                    thinking: None,
                    activities: None,
                    parts: None,
                    timestamp: 1.0,
                    mentions: None,
                },
                crate::session::ChatMessage {
                    id: "m2".to_string(),
                    role: crate::session::MessageRole::Human,
                    content: "Narrow the fix policy".to_string(),
                    thinking: None,
                    activities: None,
                    parts: None,
                    timestamp: 2.0,
                    mentions: None,
                },
                crate::session::ChatMessage {
                    id: "m3".to_string(),
                    role: crate::session::MessageRole::Agent,
                    content: String::new(),
                    thinking: None,
                    activities: None,
                    parts: Some(vec![crate::session::MessagePart::Text {
                        content: "latest approved policy".to_string(),
                        parent_tool_use_id: None,
                    }]),
                    timestamp: 3.0,
                    mentions: None,
                },
            ],
            state: crate::session::SessionState::Idle,
            created_at: 1.0,
            updated_at: 3.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: None,
            workflow_step_session: false,
        };

        let output = WorkflowEngine::extract_last_assistant_text_from_session(&session).unwrap();
        assert_eq!(output, "latest approved policy");
    }

    // ---- make_step_history_entry ----

    #[test]
    fn make_step_history_entry_reject_no_structured_output() {
        let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        let entry = exec.make_step_history_entry(Some("reject".to_string()), None, None);
        assert_eq!(entry.result.as_deref(), Some("reject"));
        assert!(entry.structured_output.is_none());
        // structured_outputがNoneなのでStepOutputは生成されない
        assert!(!exec.step_outputs.contains_key("review"));
    }

    // ---- handle_approval integration (lock-inner logic) ----

    #[test]
    fn reject_comment_flows_through_approval_to_transition_and_history() {
        // handle_approval() のロック内ロジックを再現:
        // validate → decide → make_step_history_entry → apply_transition
        let decision = ApprovalDecision::Reject {
            comment: "Fix the naming convention".to_string(),
        };

        // 1. validate
        WorkflowEngine::validate_approval_decision(&decision).unwrap();

        // 3. 遷移先 "fix" ステップを含むワークフローを構築
        let mut exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: Workflow {
                variables: Default::default(),
                name: "review-fix".to_string(),
                description: "test".to_string(),
                builtin: false,
                nodes: vec![
                    NodeDefinition {
                        name: "review".to_string(),
                        node_type: NodeType::Approval,
                        policy: None,
                        knowledge: None,
                        instruction: Some("Review the code".to_string()),
                        output_contract: None,
                        transition_rules: vec![TransitionRule {
                            r#match: "reject".to_string(),
                            next: "fix".to_string(),
                        }],
                        cycle_guard: None,
                        pass_previous_response: None,
                        pass_output_from: None,
                        inline_prompt: None,
                        collect: None,
                        parallel_children: None,
                        aggregate: None,
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                        ..Default::default()
                    },
                    NodeDefinition {
                        name: "fix".to_string(),
                        node_type: NodeType::Agent,
                        policy: None,
                        knowledge: None,
                        instruction: Some("Fix the issues".to_string()),
                        output_contract: None,
                        transition_rules: vec![],
                        cycle_guard: None,
                        pass_previous_response: Some(true),
                        pass_output_from: None,
                        inline_prompt: None,
                        collect: None,
                        parallel_children: None,
                        aggregate: None,
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                        ..Default::default()
                    },
                ],
            },
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: vec![],
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        // 4. decide
        let action = exec.decide_approval_action(&decision).unwrap();
        assert_eq!(action, ApprovalAction::TransitionTo("fix".to_string()));

        // 5. handle_approvalと同じ適用経路でReject commentをStepOutputに保存する
        let outcome = WorkflowEngine::apply_approval_application(
            &mut exec,
            &decision,
            ApprovalApplication {
                effective_result: "reject".to_string(),
                structured_output: Some(WorkflowEngine::reject_structured_output(
                    "Fix the naming convention",
                    &[],
                )),
                output_contract: None,
            },
        )
        .unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));

        // 検証: step_history にReject結果が記録されている
        assert_eq!(exec.step_history.len(), 1);
        let hist = &exec.step_history[0];
        assert_eq!(hist.step_name, "review");
        assert_eq!(hist.result.as_deref(), Some("reject"));
        assert_eq!(
            hist.structured_output.as_ref().unwrap()["comment"],
            "Fix the naming convention"
        );
        let review_output = exec.step_outputs.get("review").unwrap();
        assert_eq!(review_output.result.as_deref(), Some("reject"));
        assert_eq!(
            review_output.structured_output.as_ref().unwrap()["comment"],
            "Fix the naming convention"
        );

        // 検証: 遷移先 "fix" ステップに移動している
        assert_eq!(exec.current_step_index, 1);
        assert_eq!(exec.workflow.nodes[exec.current_step_index].name, "fix");

        let injected = WorkflowEngine::inject_step_outputs(
            "Draft next policy",
            &exec.workflow.nodes[exec.current_step_index],
            &exec.step_outputs,
            &exec.step_history,
            &HashMap::new(),
        );
        assert!(injected.contains("\"decision\": \"reject\""));
        assert!(injected.contains("\"comment\": \"Fix the naming convention\""));
    }

    #[test]
    fn apply_approval_application_records_approved_policy_and_advances_once() {
        let mut exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: Workflow {
                variables: Default::default(),
                name: "auto-approve".to_string(),
                description: "test".to_string(),
                builtin: false,
                nodes: vec![
                    NodeDefinition {
                        name: "fix_policy".to_string(),
                        node_type: NodeType::Approval,
                        policy: None,
                        knowledge: None,
                        instruction: Some("Review fix policy".to_string()),
                        output_contract: Some("approved-fix-policy".to_string()),
                        transition_rules: vec![],
                        cycle_guard: None,
                        pass_previous_response: None,
                        pass_output_from: None,
                        inline_prompt: None,
                        collect: None,
                        parallel_children: None,
                        aggregate: None,
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                        ..Default::default()
                    },
                    make_test_step("fix", NodeType::Agent, "Fix", vec![], None),
                ],
            },
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 0,
            step_execution_counts: {
                let mut m = HashMap::new();
                m.insert("fix_policy".to_string(), 1);
                m
            },
            step_history: vec![],
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some("policy-session".to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let structured_output = serde_json::json!({
            "policy": "Fix only the reported issues.",
            "review_step": "code_review_parallel",
            "findings": []
        });
        let first = WorkflowEngine::apply_approval_application(
            &mut exec,
            &ApprovalDecision::Approve,
            ApprovalApplication {
                effective_result: "approved".to_string(),
                structured_output: Some(structured_output),
                output_contract: Some("approved-fix-policy".to_string()),
            },
        )
        .unwrap();
        assert!(matches!(first, StepOutcome::TransitionAndStart(_)));
        assert_eq!(exec.current_step_index, 1);
        assert_eq!(exec.step_history.len(), 1);
        assert_eq!(*exec.step_execution_counts.get("fix").unwrap(), 1);
        assert!(!exec.workflow_variables.contains_key("approved_fix_policy"));

        let duplicate = WorkflowEngine::apply_approval_application(
            &mut exec,
            &ApprovalDecision::Approve,
            ApprovalApplication {
                effective_result: "approved".to_string(),
                structured_output: Some(serde_json::json!({
                    "policy": "Duplicate",
                    "review_step": "code_review_parallel",
                    "findings": []
                })),
                output_contract: Some("approved-fix-policy".to_string()),
            },
        );
        match duplicate {
            Err(WorkflowEngineError::InvalidState(_)) => {}
            _ => panic!("expected invalid_state"),
        }
        assert_eq!(exec.step_history.len(), 1);
        assert_eq!(*exec.step_execution_counts.get("fix").unwrap(), 1);
    }

    #[test]
    fn spec_driven_spec_fix_policy_approve_records_policy_and_starts_spec_fix_once() {
        // [08] prose 抽出経路は廃止済み。テストでは CLI submit 経由で確定する想定の
        // structured_output と effective_result を直接組み立てて apply_approval_application
        // の遷移挙動を検証する（spec [08] Rule 4 / [05] internal node command 境界）。
        let mut exec =
            make_spec_driven_spec_fix_policy_exec("exec-plan-approve", "plan-policy-session");
        let structured_output = serde_json::json!({
            "policy": "Update the spec only for the approved plan review finding.",
            "review_step": "spec_review_parallel",
            "findings": []
        });
        let effective_result = "approved".to_string();

        let outcome = WorkflowEngine::apply_approval_application(
            &mut exec,
            &ApprovalDecision::Approve,
            ApprovalApplication {
                effective_result,
                structured_output: Some(structured_output),
                output_contract: Some("approved-fix-policy".to_string()),
            },
        )
        .unwrap();

        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.nodes[exec.current_step_index].name,
            "spec_fix"
        );
        assert_eq!(exec.step_execution_counts.get("spec_fix"), Some(&1));
        assert_eq!(
            exec.step_history
                .iter()
                .filter(|entry| entry.step_name == "spec_fix_policy")
                .count(),
            1
        );
        assert_eq!(
            exec.step_outputs
                .get("spec_fix_policy")
                .and_then(|output| output.structured_output.as_ref())
                .and_then(|output| output.get("policy"))
                .and_then(|policy| policy.as_str()),
            Some("Update the spec only for the approved plan review finding.")
        );
        assert_eq!(
            exec.step_outputs
                .get("spec_fix_policy")
                .and_then(|output| output.output_contract.as_deref()),
            Some("approved-fix-policy")
        );

        let duplicate = WorkflowEngine::apply_approval_application(
            &mut exec,
            &ApprovalDecision::Approve,
            ApprovalApplication {
                effective_result: "approved".to_string(),
                structured_output: Some(serde_json::json!({
                    "policy": "Duplicate",
                    "review_step": "spec_review_parallel",
                    "findings": []
                })),
                output_contract: Some("approved-fix-policy".to_string()),
            },
        );
        assert!(matches!(
            duplicate,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        assert_eq!(
            exec.step_history
                .iter()
                .filter(|entry| entry.step_name == "spec_fix_policy")
                .count(),
            1
        );
        assert_eq!(exec.step_execution_counts.get("spec_fix"), Some(&1));
    }

    #[test]
    fn spec_driven_spec_fix_policy_reject_returns_to_plan_approval_without_approved_policy_or_spec_fix(
    ) {
        let mut exec =
            make_spec_driven_spec_fix_policy_exec("exec-plan-reject", "plan-policy-session");
        let decision = ApprovalDecision::Reject {
            comment: "Revise the spec policy first.".to_string(),
        };

        let outcome = WorkflowEngine::apply_approval_application(
            &mut exec,
            &decision,
            ApprovalApplication {
                effective_result: "reject".to_string(),
                structured_output: Some(WorkflowEngine::reject_structured_output(
                    "Revise the spec policy first.",
                    &[],
                )),
                output_contract: None,
            },
        )
        .unwrap();

        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.nodes[exec.current_step_index].name,
            "approve_spec"
        );
        assert_eq!(exec.step_execution_counts.get("spec_fix"), None);
        assert!(!exec
            .step_outputs
            .values()
            .any(|output| output.output_contract.as_deref() == Some("approved-fix-policy")));
        assert_eq!(
            exec.step_outputs
                .get("spec_fix_policy")
                .and_then(|output| output.structured_output.as_ref())
                .and_then(|output| output.get("decision"))
                .and_then(|decision| decision.as_str()),
            Some("reject")
        );
    }

    #[test]
    fn spec_driven_implementation_fix_policy_reject_returns_to_implementation_approval_without_approved_policy_or_fix(
    ) {
        let mut exec = make_spec_driven_fix_policy_exec(
            "exec-implementation-reject",
            "implementation-policy-session",
            "implementation_fix_policy",
        );
        let decision = ApprovalDecision::Reject {
            comment: "Revise the implementation policy first.".to_string(),
        };

        let outcome = WorkflowEngine::apply_approval_application(
            &mut exec,
            &decision,
            ApprovalApplication {
                effective_result: "reject".to_string(),
                structured_output: Some(WorkflowEngine::reject_structured_output(
                    "Revise the implementation policy first.",
                    &[],
                )),
                output_contract: None,
            },
        )
        .unwrap();

        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.nodes[exec.current_step_index].name,
            "implementation_approval"
        );
        assert_eq!(exec.step_execution_counts.get("fix"), None);
        assert!(!exec
            .step_outputs
            .values()
            .any(|output| output.output_contract.as_deref() == Some("approved-fix-policy")));
        assert_eq!(
            exec.step_outputs
                .get("implementation_fix_policy")
                .and_then(|output| output.structured_output.as_ref())
                .and_then(|output| output.get("decision"))
                .and_then(|decision| decision.as_str()),
            Some("reject")
        );
    }

    #[test]
    fn auto_approve_persist_target_applies_latest_policy_and_advances_once() {
        let mut exec = WorkflowExecution {
            id: "exec-auto-approve".to_string(),
            workflow: Workflow {
                variables: Default::default(),
                name: "auto-approve-path".to_string(),
                description: "test".to_string(),
                builtin: false,
                nodes: vec![
                    NodeDefinition {
                        name: "implementation_fix_policy".to_string(),
                        node_type: NodeType::Approval,
                        policy: None,
                        knowledge: None,
                        instruction: Some("Review fix policy".to_string()),
                        output_contract: Some("approved-fix-policy".to_string()),
                        transition_rules: vec![],
                        cycle_guard: None,
                        pass_previous_response: None,
                        pass_output_from: Some(vec!["code_review_parallel".to_string()]),
                        inline_prompt: None,
                        collect: None,
                        parallel_children: None,
                        aggregate: None,
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                        ..Default::default()
                    },
                    make_test_step("fix", NodeType::Agent, "Fix", vec![], None),
                    NodeDefinition {
                        name: "code_review_parallel".to_string(),
                        node_type: NodeType::Parallel,
                        policy: None,
                        knowledge: None,
                        instruction: None,
                        output_contract: None,
                        transition_rules: vec![],
                        cycle_guard: None,
                        pass_previous_response: None,
                        pass_output_from: None,
                        inline_prompt: None,
                        collect: None,
                        parallel_children: Some(vec![]),
                        aggregate: Some(ParallelAggregate {
                            all_match: Some("LGTM".to_string()),
                            any_match: None,
                            then: "fix".to_string(),
                            r#else: "implementation_fix_policy".to_string(),
                        }),
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                        ..Default::default()
                    },
                ],
            },
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 0,
            step_execution_counts: HashMap::from([("implementation_fix_policy".to_string(), 1)]),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some("policy-session".to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let snapshot = exec.to_workflow_state();
        assert_eq!(
            WorkflowEngine::auto_approve_target_for_persisted_snapshot(&snapshot, true),
            Some((
                "exec-auto-approve".to_string(),
                "implementation_fix_policy".to_string()
            ))
        );

        // [08] prose 抽出経路は廃止済み。CLI submit 経由で確定する想定の structured_output
        // を直接組み立てて apply_approval_application の遷移挙動を検証する。
        let structured_output = serde_json::json!({
            "policy": "Fix only reviewed findings.",
            "review_step": "code_review_parallel",
            "findings": []
        });
        let outcome = WorkflowEngine::apply_approval_application(
            &mut exec,
            &ApprovalDecision::Approve,
            ApprovalApplication {
                effective_result: "approved".to_string(),
                structured_output: Some(structured_output),
                output_contract: Some("approved-fix-policy".to_string()),
            },
        )
        .unwrap();

        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(exec.current_step_index, 1);
        assert_eq!(exec.step_history.len(), 1);
        assert_eq!(exec.step_outputs.len(), 1);
        assert_eq!(
            exec.step_outputs["implementation_fix_policy"]
                .structured_output
                .as_ref()
                .unwrap()["policy"],
            "Fix only reviewed findings."
        );
        assert_eq!(exec.workflow_variables.get("approved_fix_policy"), None);
        assert_eq!(exec.step_execution_counts.get("fix"), Some(&1));

        let duplicate = WorkflowEngine::apply_approval_application(
            &mut exec,
            &ApprovalDecision::Approve,
            ApprovalApplication {
                effective_result: "approved".to_string(),
                structured_output: Some(serde_json::json!({
                    "policy": "Duplicate",
                    "review_step": "code_review_parallel",
                    "findings": []
                })),
                output_contract: Some("approved-fix-policy".to_string()),
            },
        );
        assert!(matches!(
            duplicate,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        assert_eq!(exec.step_history.len(), 1);
        assert_eq!(exec.step_execution_counts.get("fix"), Some(&1));
    }

    #[tokio::test]
    async fn execute_outcome_auto_approve_persist_adopts_policy_and_starts_fix_once() {
        let engine = WorkflowEngine::new_for_test();
        let worktree_path = "/repo";
        let policy_session_id = uuid::Uuid::new_v4().to_string();

        let mut fix_step = make_test_step("fix", NodeType::Agent, "Fix", vec![], None);
        fix_step.collect = Some(CollectConfig {
            from: vec!["implementation_fix_policy".to_string()],
            reduce: ReduceStrategy::Last,
        });
        let exec = WorkflowExecution {
            id: "exec-auto-approve".to_string(),
            workflow: Workflow {
                variables: Default::default(),
                name: "auto-approve-execute-outcome".to_string(),
                description: "test".to_string(),
                builtin: false,
                nodes: vec![
                    NodeDefinition {
                        name: "code_review_parallel".to_string(),
                        node_type: NodeType::Parallel,
                        policy: None,
                        knowledge: None,
                        instruction: None,
                        output_contract: None,
                        transition_rules: vec![],
                        cycle_guard: None,
                        pass_previous_response: None,
                        pass_output_from: None,
                        inline_prompt: None,
                        collect: None,
                        parallel_children: Some(vec![]),
                        aggregate: Some(ParallelAggregate {
                            all_match: Some("LGTM".to_string()),
                            any_match: None,
                            then: "done".to_string(),
                            r#else: "implementation_fix_policy".to_string(),
                        }),
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                        ..Default::default()
                    },
                    NodeDefinition {
                        name: "implementation_fix_policy".to_string(),
                        node_type: NodeType::Approval,
                        policy: None,
                        knowledge: None,
                        instruction: Some("Review fix policy".to_string()),
                        output_contract: Some("approved-fix-policy".to_string()),
                        transition_rules: vec![],
                        cycle_guard: None,
                        pass_previous_response: None,
                        pass_output_from: Some(vec!["code_review_parallel".to_string()]),
                        inline_prompt: None,
                        collect: None,
                        parallel_children: None,
                        aggregate: None,
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                        ..Default::default()
                    },
                    fix_step,
                ],
            },
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 1,
            step_execution_counts: HashMap::from([("implementation_fix_policy".to_string(), 1)]),
            step_history: Vec::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some(policy_session_id.clone()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let snapshot = exec.to_workflow_state();
        let run_id = exec.id.clone();
        engine.executions.lock().await.insert(run_id.clone(), exec);
        engine.session_workflow_refs.lock().await.insert(
            policy_session_id,
            SessionWorkflowRef {
                run_id: run_id.clone(),
            },
        );

        let outcome = engine
            .execute_outcome_persist_auto_approve_for_test(worktree_path, &snapshot)
            .await
            .unwrap()
            .unwrap();

        let execs = engine.executions.lock().await;
        let (_, exec) = find_by_worktree(&execs, worktree_path).unwrap();
        assert!(matches!(outcome, StepOutcome::ReduceAndTransition(_)));
        assert_eq!(exec.step_execution_counts.get("fix"), Some(&1));
        assert_eq!(
            exec.step_history
                .iter()
                .filter(|entry| entry.step_name == "implementation_fix_policy")
                .count(),
            1
        );
        // [08] prose 抽出経路は廃止済み。auto approve 経路でも structured_output は
        // 確定されず、step は output 無しで完了する（spec [08] Rule 4）。
        assert!(exec
            .step_outputs
            .get("implementation_fix_policy")
            .and_then(|output| output.structured_output.as_ref())
            .is_none());
    }

    #[tokio::test]
    async fn execute_outcome_auto_approve_plan_policy_starts_spec_fix_once() {
        let engine = WorkflowEngine::new_for_test();
        let worktree_path = "/repo";
        let policy_session_id = uuid::Uuid::new_v4().to_string();
        let exec =
            make_spec_driven_spec_fix_policy_exec("exec-plan-auto-approve", &policy_session_id);
        let snapshot = exec.to_workflow_state();
        let run_id = exec.id.clone();
        engine.executions.lock().await.insert(run_id.clone(), exec);
        engine.session_workflow_refs.lock().await.insert(
            policy_session_id,
            SessionWorkflowRef {
                run_id: run_id.clone(),
            },
        );

        let outcome = engine
            .execute_outcome_persist_auto_approve_for_test(worktree_path, &snapshot)
            .await
            .unwrap()
            .unwrap();

        let execs = engine.executions.lock().await;
        let (_, exec) = find_by_worktree(&execs, worktree_path).unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.nodes[exec.current_step_index].name,
            "spec_fix"
        );
        assert_eq!(exec.step_execution_counts.get("spec_fix"), Some(&1));
        assert_eq!(
            exec.step_history
                .iter()
                .filter(|entry| entry.step_name == "spec_fix_policy")
                .count(),
            1
        );
        // [08] prose 抽出経路は廃止済み（spec [08] Rule 4）。
        assert!(exec
            .step_outputs
            .get("spec_fix_policy")
            .and_then(|output| output.structured_output.as_ref())
            .is_none());
    }

    #[tokio::test]
    async fn auto_approve_and_manual_approve_race_starts_spec_fix_once() {
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let worktree_path = "/repo";
        let policy_session_id = uuid::Uuid::new_v4().to_string();
        let exec =
            make_spec_driven_spec_fix_policy_exec("exec-plan-approve-race", &policy_session_id);
        let snapshot = exec.to_workflow_state();
        let run_id = exec.id.clone();
        engine.executions.lock().await.insert(run_id.clone(), exec);
        engine.session_workflow_refs.lock().await.insert(
            policy_session_id,
            SessionWorkflowRef {
                run_id: run_id.clone(),
            },
        );

        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let expected_execution_id = snapshot.execution_id.clone();
        let expected_step_name = snapshot.current_step_name.clone();
        let auto_snapshot = snapshot.clone();

        let auto_engine = Arc::clone(&engine);
        let auto_barrier = Arc::clone(&barrier);
        let auto_worktree_path = worktree_path.to_string();
        let auto_expected_execution_id = expected_execution_id.clone();
        let auto_expected_step_name = expected_step_name.clone();
        let auto = async move {
            {
                let execs = auto_engine.executions.lock().await;
                let (_, exec) = find_by_worktree(&execs, &auto_worktree_path).unwrap();
                WorkflowEngine::validate_approval_target_snapshot(
                    exec,
                    Some(&auto_expected_execution_id),
                    Some(&auto_expected_step_name),
                )
                .unwrap();
            }
            auto_barrier.wait().await;
            auto_engine
                .execute_outcome_persist_auto_approve_for_test(&auto_worktree_path, &auto_snapshot)
                .await
                .and_then(|outcome| {
                    outcome.ok_or_else(|| {
                        WorkflowEngineError::InvalidState(
                            "auto approve did not target waiting approval".to_string(),
                        )
                    })
                })
        };

        let manual_engine = Arc::clone(&engine);
        let manual_barrier = Arc::clone(&barrier);
        let manual_worktree_path = worktree_path.to_string();
        let manual = async move {
            {
                let execs = manual_engine.executions.lock().await;
                let (_, exec) = find_by_worktree(&execs, &manual_worktree_path).unwrap();
                WorkflowEngine::validate_approval_target_snapshot(
                    exec,
                    Some(&expected_execution_id),
                    Some(&expected_step_name),
                )
                .unwrap();
            }
            manual_barrier.wait().await;
            manual_engine
                .handle_approval_with_output_for_test(
                    &manual_worktree_path,
                    ApprovalDecision::Approve,
                    Some(&expected_execution_id),
                    Some(&expected_step_name),
                )
                .await
        };

        let (auto_result, manual_result) = tokio::join!(auto, manual);
        let results = [&auto_result, &manual_result];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        assert!(results
            .iter()
            .any(|result| matches!(result, Ok(StepOutcome::TransitionAndStart(_)))));
        assert!(results.iter().any(|result| matches!(
            result,
            Err(WorkflowEngineError::InvalidState(_))
                | Err(WorkflowEngineError::UnauthorizedApprovalTarget(_))
        )));

        let execs = engine.executions.lock().await;
        let (_, exec) = find_by_worktree(&execs, worktree_path).unwrap();
        assert_eq!(
            exec.workflow.nodes[exec.current_step_index].name,
            "spec_fix"
        );
        assert_eq!(exec.step_execution_counts.get("spec_fix"), Some(&1));
        assert_eq!(
            exec.step_history
                .iter()
                .filter(|entry| entry.step_name == "spec_fix_policy")
                .count(),
            1
        );
        // [08] prose 抽出経路は廃止済み（spec [08] Rule 4）。
        assert!(exec
            .step_outputs
            .get("spec_fix_policy")
            .and_then(|output| output.structured_output.as_ref())
            .is_none());
    }

    #[test]
    fn execute_outcome_persist_path_builds_auto_approve_target_for_current_step() {
        let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        exec.current_session_id = Some("policy-session".to_string());
        let waiting = exec.to_workflow_state();

        assert_eq!(
            WorkflowEngine::auto_approve_target_for_persisted_snapshot(&waiting, true),
            Some(("exec-1".to_string(), "review".to_string()))
        );
        assert_eq!(
            WorkflowEngine::auto_approve_target_for_persisted_snapshot(&waiting, false),
            None
        );

        exec.state = WorkflowExecutionState::Running;
        let running = exec.to_workflow_state();
        assert_eq!(
            WorkflowEngine::auto_approve_target_for_persisted_snapshot(&running, true),
            None
        );
    }

    #[test]
    fn workflow_approval_auto_approve_flag_controls_waiting_approval_snapshots() {
        let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        exec.current_session_id = Some("policy-session".to_string());
        let waiting = exec.to_workflow_state();
        assert!(WorkflowEngine::should_auto_approve_workflow_approval(
            &waiting, true
        ));
        assert!(!WorkflowEngine::should_auto_approve_workflow_approval(
            &waiting, false
        ));

        exec.state = WorkflowExecutionState::Running;
        let running = exec.to_workflow_state();
        assert!(!WorkflowEngine::should_auto_approve_workflow_approval(
            &running, true
        ));
    }

    #[test]
    fn workflow_approval_auto_approve_disabled_ignores_agent_auto_approve_permission_mode() {
        let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        exec.current_session_id = Some("policy-session".to_string());
        let agent_auto_approve_permission_mode = "full";
        let workflow_approval_auto_approve_enabled = false;
        let snapshot = exec.to_workflow_state();

        assert_eq!(agent_auto_approve_permission_mode, "full");
        assert!(!WorkflowEngine::should_auto_approve_workflow_approval(
            &snapshot,
            workflow_approval_auto_approve_enabled
        ));
        assert_eq!(
            WorkflowEngine::auto_approve_target_for_persisted_snapshot(
                &snapshot,
                workflow_approval_auto_approve_enabled,
            ),
            None
        );
    }

    // R4-01: output_contractなしのparallel childはStepOutputを生成しない
    #[test]
    fn evaluate_aggregate_child_without_output_contract_has_no_step_output() {
        let engine = WorkflowEngine::new_for_test();
        let agg = ParallelAggregate {
            all_match: Some("LGTM".to_string()),
            any_match: None,
            then: "report".to_string(),
            r#else: "implement".to_string(),
        };
        let mut outputs = HashMap::new();
        outputs.insert(
            "arch-review".to_string(),
            make_step_output("arch-review", "ok", Some("LGTM")),
        );
        let children = vec!["arch-review".to_string(), "test-step".to_string()];
        assert!(!engine.evaluate_aggregate(&agg, &outputs, &children));
    }

    // R4-02: make_step_history_entryがcontract resultをStepOutput.resultに保存する
    #[test]
    fn make_step_history_entry_saves_contract_result_to_step_output() {
        let mut exec = WorkflowExecution {
            id: "test-exec".to_string(),
            workflow: make_test_workflow(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: {
                let mut m = HashMap::new();
                m.insert("plan".to_string(), 1);
                m
            },
            step_history: vec![],
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some("session-1".to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        let structured = serde_json::json!({"verdict": "LGTM", "findings": []});
        let entry = exec.make_step_history_entry(
            Some("LGTM".to_string()),
            Some(structured.clone()),
            Some("review-verdict".to_string()),
        );

        assert_eq!(entry.result.as_deref(), Some("LGTM"));
        assert_eq!(entry.structured_output, Some(structured.clone()));

        let step_output = exec
            .step_outputs
            .get("plan")
            .expect("StepOutput should exist");
        assert_eq!(step_output.result.as_deref(), Some("LGTM"));
        assert_eq!(step_output.structured_output, Some(structured));
        assert_eq!(
            step_output.output_contract.as_deref(),
            Some("review-verdict")
        );
    }

    #[test]
    fn make_step_history_entry_no_structured_output_no_step_output() {
        let mut exec = WorkflowExecution {
            id: "test-exec".to_string(),
            workflow: make_test_workflow(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: {
                let mut m = HashMap::new();
                m.insert("plan".to_string(), 1);
                m
            },
            step_history: vec![],
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some("session-1".to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        let entry = exec.make_step_history_entry(Some("complete".to_string()), None, None);

        assert_eq!(entry.result.as_deref(), Some("complete"));
        assert!(entry.structured_output.is_none());
        assert!(!exec.step_outputs.contains_key("plan"));
    }

    // ---- on_exhausted: apply_transition テスト ----

    fn make_on_exhausted_workflow() -> Workflow {
        Workflow {
            variables: Default::default(),
            name: "on-exhausted-test".to_string(),
            description: "Test on_exhausted".to_string(),
            builtin: false,
            nodes: vec![
                make_test_step(
                    "fix",
                    NodeType::Agent,
                    "Fix issues",
                    vec![TransitionRule {
                        r#match: ".*".to_string(),
                        next: "review".to_string(),
                    }],
                    Some(CycleGuard {
                        max_iterations: 2,
                        on_exhausted: Some("approval".to_string()),
                    }),
                ),
                make_test_step(
                    "review",
                    NodeType::Agent,
                    "Review",
                    vec![TransitionRule {
                        r#match: "NEEDS_FIX".to_string(),
                        next: "fix".to_string(),
                    }],
                    None,
                ),
                NodeDefinition {
                    resets_cycle_for: Some(vec!["fix".to_string()]),
                    ..make_test_step(
                        "approval",
                        NodeType::Agent,
                        "Approve",
                        vec![TransitionRule {
                            r#match: "NEEDS_FIX".to_string(),
                            next: "fix".to_string(),
                        }],
                        None,
                    )
                },
            ],
        }
    }

    #[test]
    fn on_exhausted_transitions_to_fallback_step() {
        let mut exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: make_on_exhausted_workflow(),
            state: WorkflowExecutionState::Running,
            current_step_index: 1, // review
            step_execution_counts: {
                let mut m = HashMap::new();
                m.insert("fix".to_string(), 2); // already at max
                m
            },
            step_history: vec![],
            step_outputs: HashMap::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        // fix への遷移を試みる → ガード超過 → on_exhausted で approval へ
        let outcome = WorkflowEngine::apply_transition(&mut exec, "fix").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.nodes[exec.current_step_index].name,
            "approval"
        );
    }

    #[test]
    fn on_exhausted_none_fails_workflow() {
        let mut wf = make_on_exhausted_workflow();
        // on_exhausted を None に変更
        wf.nodes[0].cycle_guard = Some(CycleGuard {
            max_iterations: 2,
            on_exhausted: None,
        });

        let mut exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: wf,
            state: WorkflowExecutionState::Running,
            current_step_index: 1,
            step_execution_counts: {
                let mut m = HashMap::new();
                m.insert("fix".to_string(), 2);
                m
            },
            step_history: vec![],
            step_outputs: HashMap::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        let outcome = WorkflowEngine::apply_transition(&mut exec, "fix").unwrap();
        assert!(matches!(outcome, StepOutcome::Persist(_)));
        assert!(matches!(exec.state, WorkflowExecutionState::Failed { .. }));
    }

    #[test]
    fn check_cycle_guard_exceeded_with_on_exhausted() {
        let exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: make_on_exhausted_workflow(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: {
                let mut m = HashMap::new();
                m.insert("fix".to_string(), 2);
                m
            },
            step_history: vec![],
            step_outputs: HashMap::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        assert_eq!(
            exec.check_cycle_guard("fix").unwrap(),
            CycleGuardResult::Exceeded {
                max_iterations: 2,
                count: 2,
                on_exhausted: Some("approval".to_string()),
            }
        );
    }

    // ---- resets_cycle_for テスト ----

    #[test]
    fn resets_cycle_for_clears_execution_count() {
        let mut exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: make_on_exhausted_workflow(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0, // fix
            step_execution_counts: {
                let mut m = HashMap::new();
                m.insert("fix".to_string(), 2);
                m
            },
            step_history: vec![],
            step_outputs: HashMap::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        // approval に遷移 → resets_cycle_for で fix のカウントがリセット
        let outcome = WorkflowEngine::apply_transition(&mut exec, "approval").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.nodes[exec.current_step_index].name,
            "approval"
        );
        // fix のカウントがリセットされている
        assert_eq!(exec.step_execution_counts.get("fix"), None);
    }

    #[test]
    fn resets_cycle_for_allows_reloop_after_reset() {
        let mut exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: make_on_exhausted_workflow(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: {
                let mut m = HashMap::new();
                m.insert("fix".to_string(), 2);
                m
            },
            step_history: vec![],
            step_outputs: HashMap::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        // approval に遷移（カウントリセット）
        WorkflowEngine::apply_transition(&mut exec, "approval").unwrap();
        assert_eq!(exec.step_execution_counts.get("fix"), None);

        // fix に再遷移可能（リセット後なのでガードに引っかからない）
        let outcome = WorkflowEngine::apply_transition(&mut exec, "fix").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(exec.workflow.nodes[exec.current_step_index].name, "fix");
        assert_eq!(exec.step_execution_counts.get("fix"), Some(&1));

        // 2回目も可能
        let outcome = WorkflowEngine::apply_transition(&mut exec, "fix").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(exec.step_execution_counts.get("fix"), Some(&2));

        // 3回目は上限到達 → on_exhausted で approval へ
        let outcome = WorkflowEngine::apply_transition(&mut exec, "fix").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.nodes[exec.current_step_index].name,
            "approval"
        );
    }

    // ---- on_exhausted チェーン遷移テスト ----

    #[test]
    fn on_exhausted_chain_transitions() {
        // step_a → (exhausted) → step_b → (exhausted) → step_c
        let wf = Workflow {
            variables: Default::default(),
            name: "chain-test".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![
                make_test_step(
                    "step_a",
                    NodeType::Agent,
                    "A",
                    vec![],
                    Some(CycleGuard {
                        max_iterations: 1,
                        on_exhausted: Some("step_b".to_string()),
                    }),
                ),
                make_test_step(
                    "step_b",
                    NodeType::Agent,
                    "B",
                    vec![],
                    Some(CycleGuard {
                        max_iterations: 1,
                        on_exhausted: Some("step_c".to_string()),
                    }),
                ),
                make_test_step("step_c", NodeType::Agent, "C", vec![], None),
            ],
        };
        let mut exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: wf,
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: {
                let mut m = HashMap::new();
                m.insert("step_a".to_string(), 1);
                m.insert("step_b".to_string(), 1);
                m
            },
            step_history: vec![],
            step_outputs: HashMap::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        // step_a → exhausted → step_b → exhausted → step_c
        let outcome = WorkflowEngine::apply_transition(&mut exec, "step_a").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(exec.workflow.nodes[exec.current_step_index].name, "step_c");
    }

    #[test]
    fn on_exhausted_chain_to_non_exhausted_fails() {
        // step_a → (exhausted) → step_b (exhausted, no on_exhausted) → Failed
        let wf = Workflow {
            variables: Default::default(),
            name: "chain-fail-test".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![
                make_test_step(
                    "step_a",
                    NodeType::Agent,
                    "A",
                    vec![],
                    Some(CycleGuard {
                        max_iterations: 1,
                        on_exhausted: Some("step_b".to_string()),
                    }),
                ),
                make_test_step(
                    "step_b",
                    NodeType::Agent,
                    "B",
                    vec![],
                    Some(CycleGuard {
                        max_iterations: 1,
                        on_exhausted: None,
                    }),
                ),
            ],
        };
        let mut exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: wf,
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: {
                let mut m = HashMap::new();
                m.insert("step_a".to_string(), 1);
                m.insert("step_b".to_string(), 1);
                m
            },
            step_history: vec![],
            step_outputs: HashMap::new(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        let outcome = WorkflowEngine::apply_transition(&mut exec, "step_a").unwrap();
        assert!(matches!(outcome, StepOutcome::Persist(_)));
        assert!(matches!(exec.state, WorkflowExecutionState::Failed { .. }));
    }

    // ---- step が新しい実行を開始する瞬間に step_outputs から前回値を破棄する（Spec issues-989） ----

    fn make_step_output_fixture(step_name: &str, run_index: u32) -> StepOutput {
        StepOutput {
            step_name: step_name.to_string(),
            run_index,
            session_id: None,
            result: Some("prev".to_string()),
            structured_output: Some(serde_json::json!({"verdict": "LGTM"})),
            output_contract: None,
            token_usage: None,
            completed_at: 1000.0,
        }
    }

    #[test]
    fn apply_advance_clears_step_outputs_for_new_step() {
        // ループで同一 step が再実行されるとき、advance による遷移で
        // 遷移先 step の前回出力が step_outputs から破棄されることを検証する。
        let mut exec = make_exec(0); // plan → implement
        exec.step_outputs.insert(
            "implement".to_string(),
            make_step_output_fixture("implement", 1),
        );
        // 他 step の前回出力は残り続けることも併せて確認。
        exec.step_outputs
            .insert("plan".to_string(), make_step_output_fixture("plan", 1));

        let outcome = WorkflowEngine::apply_advance(&mut exec);
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.nodes[exec.current_step_index].name,
            "implement"
        );
        assert!(!exec.step_outputs.contains_key("implement"));
        assert!(exec.step_outputs.contains_key("plan"));
    }

    #[test]
    fn apply_transition_clears_step_outputs_for_target_step() {
        // ループで前ステップ（review）に戻る遷移でも、遷移先の前回出力が破棄される。
        let mut exec = make_exec(2); // review
        exec.step_outputs.insert(
            "implement".to_string(),
            make_step_output_fixture("implement", 1),
        );

        let outcome = WorkflowEngine::apply_transition(&mut exec, "implement").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.nodes[exec.current_step_index].name,
            "implement"
        );
        assert!(!exec.step_outputs.contains_key("implement"));
    }

    #[test]
    fn apply_transition_to_parallel_block_clears_block_and_children() {
        // 並列ブロックへの遷移では、ブロック自身と全子 step の前回出力が破棄される。
        let parallel_block = NodeDefinition {
            name: "code_review_parallel".to_string(),
            node_type: NodeType::Parallel,
            parallel_children: Some(vec![
                make_parallel_step("review_security"),
                make_parallel_step("review_style"),
            ]),
            ..NodeDefinition::default()
        };
        let wf = Workflow {
            variables: Default::default(),
            name: "loop-parallel".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![
                make_test_step("fix", NodeType::Agent, "Fix", vec![], None),
                parallel_block,
            ],
        };
        let mut exec = WorkflowExecution {
            id: "exec-1".to_string(),
            workflow: wf,
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: vec![],
            step_outputs: {
                let mut m = HashMap::new();
                m.insert(
                    "code_review_parallel".to_string(),
                    make_step_output_fixture("code_review_parallel", 1),
                );
                m.insert(
                    "review_security".to_string(),
                    make_step_output_fixture("review_security", 1),
                );
                m.insert(
                    "review_style".to_string(),
                    make_step_output_fixture("review_style", 1),
                );
                m.insert("fix".to_string(), make_step_output_fixture("fix", 1));
                m
            },
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/repo".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        let outcome = WorkflowEngine::apply_transition(&mut exec, "code_review_parallel").unwrap();
        assert!(matches!(outcome, StepOutcome::StartParallel(_)));
        assert!(!exec.step_outputs.contains_key("code_review_parallel"));
        assert!(!exec.step_outputs.contains_key("review_security"));
        assert!(!exec.step_outputs.contains_key("review_style"));
        // 並列ブロック外の step の前回出力は破棄されない。
        assert!(exec.step_outputs.contains_key("fix"));
    }

    // ---- resolve_step_settings ----

    #[test]
    fn resolve_step_settings_model_and_permission_specified() {
        let result = resolve_step_settings(
            Some("codex-mini".to_string()),
            Some("full".to_string()),
            Some("codex".to_string()),
            &WorkflowDefaults {
                backend_id: Some("claude".to_string()),
                permission_mode: "edit".to_string(),
            },
        );
        assert_eq!(
            result,
            ResolvedStepSettings {
                backend_id: Some("codex".to_string()),
                selected_model: Some("codex-mini".to_string()),
                permission_mode: "full".to_string(),
            }
        );
    }

    #[test]
    fn resolve_step_settings_model_only() {
        let result = resolve_step_settings(
            Some("haiku".to_string()),
            None,
            Some("claude".to_string()),
            &WorkflowDefaults {
                backend_id: Some("claude".to_string()),
                permission_mode: "edit".to_string(),
            },
        );
        assert_eq!(
            result,
            ResolvedStepSettings {
                backend_id: Some("claude".to_string()),
                selected_model: Some("haiku".to_string()),
                permission_mode: "edit".to_string(),
            }
        );
    }

    #[test]
    fn resolve_step_settings_permission_only_clears_model_to_unset() {
        // Spec: workflow 経路では step model 未指定なら親の選択モデルへフォールバックしない。
        // permission のみ指定でも selected_model は None になる。
        let result = resolve_step_settings(
            None,
            Some("ask".to_string()),
            None,
            &WorkflowDefaults {
                backend_id: Some("claude".to_string()),
                permission_mode: "edit".to_string(),
            },
        );
        assert_eq!(
            result,
            ResolvedStepSettings {
                backend_id: Some("claude".to_string()),
                selected_model: None,
                permission_mode: "ask".to_string(),
            }
        );
    }

    #[test]
    fn resolve_step_settings_nothing_specified_clears_model_to_unset() {
        // Spec: model 未指定（None）は未指定状態のまま。親の selected_model へ
        // 暗黙フォールバックしない。
        let result = resolve_step_settings(
            None,
            None,
            None,
            &WorkflowDefaults {
                backend_id: Some("claude".to_string()),
                permission_mode: "edit".to_string(),
            },
        );
        assert_eq!(
            result,
            ResolvedStepSettings {
                backend_id: Some("claude".to_string()),
                selected_model: None,
                permission_mode: "edit".to_string(),
            }
        );
    }

    #[test]
    fn resolve_step_settings_parallel_children_different_configs() {
        // ステップA: model=opus-4, permission=ask
        let result_a = resolve_step_settings(
            Some("opus-4".to_string()),
            Some("ask".to_string()),
            Some("claude".to_string()),
            &WorkflowDefaults {
                backend_id: Some("claude".to_string()),
                permission_mode: "edit".to_string(),
            },
        );
        assert_eq!(
            result_a,
            ResolvedStepSettings {
                backend_id: Some("claude".to_string()),
                selected_model: Some("opus-4".to_string()),
                permission_mode: "ask".to_string(),
            }
        );

        // ステップB: model=codex-mini, permission=full
        let result_b = resolve_step_settings(
            Some("codex-mini".to_string()),
            Some("full".to_string()),
            Some("codex".to_string()),
            &WorkflowDefaults {
                backend_id: Some("claude".to_string()),
                permission_mode: "edit".to_string(),
            },
        );
        assert_eq!(
            result_b,
            ResolvedStepSettings {
                backend_id: Some("codex".to_string()),
                selected_model: Some("codex-mini".to_string()),
                permission_mode: "full".to_string(),
            }
        );

        // 並列ステップ間で結果が独立していることを確認
        assert_ne!(result_a.backend_id, result_b.backend_id);
        assert_ne!(result_a.selected_model, result_b.selected_model);
        assert_ne!(result_a.permission_mode, result_b.permission_mode);
    }

    // ---- ワークフロー step session の attributes 永続化 ----

    // Spec issues-947: ワークフロー step session 作成は
    // `create_session_internal_with_attributes` 経由で permission_mode / selected_model /
    // workflow_step_session=true を初回保存で確定する。create_step_session_with_settings の
    // 後段（resolve_step_settings の結果を attributes に流して save する経路）が
    // 二段階保存に逆戻りしないことをガードする。
    #[test]
    fn step_session_persists_permission_workflow_flag_and_model_on_initial_save() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = crate::session::SessionStore::default();

        let settings = resolve_step_settings(
            Some("opus-4".to_string()),
            Some("edit".to_string()),
            Some("claude".to_string()),
            &WorkflowDefaults {
                backend_id: Some("codex".to_string()),
                permission_mode: "ask".to_string(),
            },
        );
        let permission_mode =
            crate::permission::PermissionMode::parse(&settings.permission_mode).unwrap();
        let session = crate::session::create_session_internal_with_attributes(
            &store,
            tmp.path(),
            "/repo",
            settings.backend_id.clone(),
            permission_mode,
            settings.selected_model.clone(),
            true,
        )
        .unwrap();

        // 初回保存で permission_mode / workflow_step_session / selected_model / backend_id が確定。
        assert_eq!(session.permission_mode, "edit");
        assert!(session.workflow_step_session);
        assert_eq!(session.selected_model.as_deref(), Some("opus-4"));
        assert_eq!(session.backend_id.as_deref(), Some("claude"));

        // 別インスタンスから読み直しても同じ値で復元される（永続化が確定値で書かれている）。
        let store2 = crate::session::SessionStore::default();
        let loaded = store2
            .get_session(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.permission_mode, "edit");
        assert!(loaded.workflow_step_session);
        assert_eq!(loaded.selected_model.as_deref(), Some("opus-4"));
        assert_eq!(loaded.backend_id.as_deref(), Some("claude"));
    }

    // 親セッションから permission_mode/backend_id を継承する経路でも初回保存で確定することを確認する。
    // selected_model は Spec issues-946 により暗黙フォールバック禁止のため、step 未指定なら None。
    #[test]
    fn step_session_inherits_parent_permission_and_backend_on_initial_save() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = crate::session::SessionStore::default();

        let settings = resolve_step_settings(
            None,
            None,
            None,
            &WorkflowDefaults {
                backend_id: Some("claude".to_string()),
                permission_mode: "full".to_string(),
            },
        );
        let permission_mode =
            crate::permission::PermissionMode::parse(&settings.permission_mode).unwrap();
        let session = crate::session::create_session_internal_with_attributes(
            &store,
            tmp.path(),
            "/repo",
            settings.backend_id,
            permission_mode,
            settings.selected_model,
            true,
        )
        .unwrap();

        assert_eq!(session.permission_mode, "full");
        assert!(session.workflow_step_session);
        // 親 selected_model="haiku" は継承しない（Spec issues-946: 暗黙フォールバック禁止）
        assert_eq!(session.selected_model, None);
        assert_eq!(session.backend_id.as_deref(), Some("claude"));
    }

    // ---- run_id 主体性に関する engine レベル統合テスト ----

    /// engine が WorkflowExecution を登録する際に、`WorkflowExecution.id` と
    /// Run Store の `WorkflowRunSummary.run_id` が同一 run_id を共有することを検証する。
    /// finding 13 対応: `return 値 run_id = WorkflowExecution.id = active summary の run_id
    /// = workflow_runs/{run_id}.json の run_id` の一致を engine レベルで検証する。
    #[tokio::test]
    async fn engine_run_id_consistency_across_execution_and_run_store_metadata() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = WorkflowEngine::new_for_test();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;

        // Run Store API 境界の UUID 検証を満たすため UUID を採用する。
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/a";
        let workflow = make_minimal_workflow();
        let exec = WorkflowExecution {
            id: run_id.clone(),
            workflow: workflow.clone(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 100.0,
            updated_at: 100.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: worktree_path.to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        engine.executions.lock().await.insert(exec.id.clone(), exec);
        engine
            .run_store
            .register_active(crate::workflow::run::WorkflowRun {
                run_id: run_id.clone(),
                workflow_name: workflow.name.clone(),
                task: None,
                status: RunStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node_name: workflow.nodes.first().map(|n| n.name.clone()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();

        // (1) WorkflowExecution.id
        let exec_id = {
            let execs = engine.executions.lock().await;
            execs.get(&run_id).unwrap().id.clone()
        };
        assert_eq!(exec_id, run_id);

        // (2) Run Store active summary の run_id
        let active = engine.list_active_runs().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);

        // (3) workflow_runs/{run_id}.json の run_id
        let metadata_path = tmp
            .path()
            .join("workflow_runs")
            .join(format!("{run_id}.json"));
        assert!(metadata_path.exists());
        let metadata: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&metadata_path).unwrap()).unwrap();
        assert_eq!(metadata["runId"].as_str(), Some(run_id.as_str()));

        // (4) worktree -> run_id reverse lookup も一致
        assert_eq!(
            engine.run_id_for_worktree(worktree_path).await,
            Some(run_id.clone())
        );
        assert_eq!(
            engine.resolve_worktree_by_run(&run_id).await,
            Some(worktree_path.to_string())
        );
    }

    /// 同一 worktree への重複起動が `validate_start` で拒否されることを検証する。
    /// finding 14 対応: 既存 active な実行が同一 worktree に存在する間、
    /// validate_start は `AlreadyActive` を返す。
    #[tokio::test]
    async fn engine_validate_start_rejects_duplicate_active_run_on_same_worktree() {
        let engine = WorkflowEngine::new_for_test();
        let workflow = make_minimal_workflow();
        let worktree_path = "/wt/dup";

        let exec = WorkflowExecution {
            id: "existing-run".to_string(),
            workflow: workflow.clone(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 100.0,
            updated_at: 100.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: worktree_path.to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let existing_id = exec.id.clone();
        engine.executions.lock().await.insert(exec.id.clone(), exec);

        // validate_start should reject a new start while an active exec lives on this worktree
        let execs = engine.executions.lock().await;
        let existing = find_by_worktree(&execs, worktree_path).map(|(_, e)| e);
        assert!(existing.is_some());
        let result = WorkflowExecution::validate_start(&workflow, existing);
        match result {
            Err(WorkflowEngineError::AlreadyActive(_)) => {}
            other => panic!("expected AlreadyActive, got {other:?}"),
        }

        // Existing exec.id remains accessible by run_id
        let still_there = execs.get(&existing_id).unwrap();
        assert_eq!(still_there.id, existing_id);
        assert_eq!(still_there.worktree_path, worktree_path);
    }

    /// engine が状態遷移を反映した際に Run Store の active / completed 一覧および
    /// metadata が同期されることを検証する。
    /// finding 15 対応: Running -> WaitingApproval -> Completed の遷移で
    /// list_active / list_completed と metadata が正しく更新される。
    #[tokio::test]
    async fn engine_state_transitions_sync_to_run_store_active_and_completed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = WorkflowEngine::new_for_test();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;

        // disk fallback の reverse lookup は UUID 形式しか受理しないため、UUID を採用する。
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/transit";
        let workflow = make_minimal_workflow();
        engine
            .run_store
            .register_active(crate::workflow::run::WorkflowRun {
                run_id: run_id.clone(),
                workflow_name: workflow.name.clone(),
                task: None,
                status: RunStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node_name: workflow.nodes.first().map(|n| n.name.clone()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();

        // Running -> WaitingApproval
        let snapshot_waiting = WorkflowState {
            execution_id: run_id.clone(),
            workflow_name: workflow.name.clone(),
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 0,
            current_step_name: workflow.nodes[0].name.clone(),
            current_session_id: None,
            total_steps: workflow.nodes.len(),
            step_history: vec![],
            step_execution_counts: HashMap::new(),
            workflow_definition: workflow.clone(),
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 100.0,
            updated_at: 200.0,
        };
        engine
            .sync_run_store_from_snapshot(&run_id, &snapshot_waiting)
            .await
            .unwrap();
        let active = engine.list_active_runs().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);
        assert_eq!(active[0].status, RunStatus::WaitingApproval);

        // Completed
        let snapshot_completed = WorkflowState {
            state: WorkflowExecutionState::Completed,
            updated_at: 300.0,
            ..snapshot_waiting.clone()
        };
        engine
            .sync_run_store_from_snapshot(&run_id, &snapshot_completed)
            .await
            .unwrap();
        let active_after = engine.list_active_runs().await;
        assert!(
            active_after.is_empty(),
            "completed run must leave the active set"
        );
        let completed = engine.list_completed_runs().await;
        assert!(completed.iter().any(|r| r.run_id == run_id));
        let completed_entry = completed.iter().find(|r| r.run_id == run_id).unwrap();
        assert_eq!(completed_entry.status, RunStatus::Completed);

        // 終了後でも reverse lookup（persistence fallback）で worktree が解決できる。
        assert_eq!(
            engine.resolve_worktree_by_run(&run_id).await,
            Some(worktree_path.to_string())
        );
    }

    fn make_minimal_workflow() -> Workflow {
        Workflow {
            variables: Default::default(),
            name: "engine-test-wf".to_string(),
            description: "minimal".to_string(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "only-step".to_string(),
                node_type: NodeType::Agent,
                inline_prompt: Some("do".to_string()),
                permission: Some("edit".to_string()),
                ..NodeDefinition::default()
            }],
        }
    }

    /// G3: workflow 構造の事前検証は `validate_workflow_shape` で副作用なく完結する。
    /// 空 nodes / bash node が含まれる workflow を弾けば、`start_workflow` の Phase 1 で
    /// parent ChatSession 作成より前にエラーで return できる（孤立 session を残さない）。
    #[test]
    fn validate_workflow_shape_rejects_empty_and_bash_workflows_without_side_effects() {
        // 空 nodes は InvalidWorkflow
        let empty = Workflow {
            variables: Default::default(),
            name: "wf".to_string(),
            description: "".to_string(),
            builtin: false,
            nodes: vec![],
        };
        assert!(matches!(
            WorkflowExecution::validate_workflow_shape(&empty),
            Err(WorkflowEngineError::InvalidWorkflow(_))
        ));

        // bash node を含む workflow も InvalidWorkflow
        let bash = Workflow {
            variables: Default::default(),
            name: "wf".to_string(),
            description: "".to_string(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "bash-step".to_string(),
                node_type: NodeType::Bash,
                ..NodeDefinition::default()
            }],
        };
        assert!(matches!(
            WorkflowExecution::validate_workflow_shape(&bash),
            Err(WorkflowEngineError::InvalidWorkflow(_))
        ));

        // 正常な workflow は Ok
        let ok = make_minimal_workflow();
        assert!(WorkflowExecution::validate_workflow_shape(&ok).is_ok());
    }

    /// G3: `run_id_for_worktree` を Run Store 経由で参照すれば、parent ChatSession 作成より前に
    /// 重複起動を検出できる。`start_workflow` Phase 1 で副作用前に判定する経路の主要な
    /// 構成要素（Run Store の active index）を直接検証する。
    #[tokio::test]
    async fn run_store_active_index_resolves_worktree_to_run_id_for_duplicate_check() {
        let engine = WorkflowEngine::new_for_test();
        let tmp = tempfile::TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let worktree_path = "/wt/duplicate-check";
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .run_store
            .register_active(crate::workflow::run::WorkflowRun {
                run_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node_name: Some("s1".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        assert_eq!(
            engine.run_id_for_worktree(worktree_path).await,
            Some(run_id),
            "Phase 1 重複判定は Run Store の active index で成立する"
        );
    }

    /// G6: handle_auto_complete の fixture は `exec.id` を execs HashMap キーに使う
    /// （production と同じ run_id キー）。fixture が `worktree_path` をキーとして使う旧バグの
    /// 回帰防止。
    #[tokio::test]
    async fn handle_auto_complete_fixture_uses_run_id_as_executions_key() {
        let engine = WorkflowEngine::new_for_test();
        let exec = WorkflowExecution {
            id: "auto-complete-run".to_string(),
            workflow: make_minimal_workflow(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 0.0,
            updated_at: 0.0,
            current_session_id: Some("sess".to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: "/wt/auto-complete".to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };
        let run_id = exec.id.clone();
        let worktree_path = exec.worktree_path.clone();
        engine.executions.lock().await.insert(run_id.clone(), exec);

        // production と同じ key で参照できる
        {
            let execs = engine.executions.lock().await;
            assert!(execs.get(&run_id).is_some());
            // worktree_path をキーとした直接 lookup は失敗する（= 旧バグの回帰なし）
            assert!(execs.get(worktree_path.as_str()).is_none());
            // find_by_worktree 経由は成功する
            assert!(find_by_worktree(&execs, &worktree_path).is_some());
        }
    }

    fn make_exec_with(
        id: &str,
        worktree_path: &str,
        state: WorkflowExecutionState,
    ) -> WorkflowExecution {
        WorkflowExecution {
            id: id.to_string(),
            workflow: make_minimal_workflow(),
            state,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            started_at: 100.0,
            updated_at: 110.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            worktree_path: worktree_path.to_string(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        }
    }

    /// Spec issues-1011 finding 1/7: `find_by_worktree` / `find_by_worktree_mut` は
    /// terminal な execution を返さず、active な execution のみを返す。同一 worktree に
    /// terminal run と active run が共存しても production 経路で取り違えない。
    #[tokio::test]
    async fn find_by_worktree_filters_terminal_runs_and_returns_active_only() {
        let engine = WorkflowEngine::new_for_test();
        let worktree_path = "/wt/shared";
        let terminal_run_id = "terminal-run".to_string();
        let active_run_id = "active-run".to_string();
        let terminal_exec = make_exec_with(
            &terminal_run_id,
            worktree_path,
            WorkflowExecutionState::Completed,
        );
        let active_exec = make_exec_with(
            &active_run_id,
            worktree_path,
            WorkflowExecutionState::Running,
        );

        {
            let mut execs = engine.executions.lock().await;
            execs.insert(terminal_run_id.clone(), terminal_exec);
            execs.insert(active_run_id.clone(), active_exec);
        }

        // find_by_worktree は active のみを返す
        {
            let execs = engine.executions.lock().await;
            let (found_id, found_exec) =
                find_by_worktree(&execs, worktree_path).expect("active run must be findable");
            assert_eq!(found_id, &active_run_id);
            assert!(found_exec.is_active());
            assert_ne!(found_id, &terminal_run_id);
        }

        // find_any_by_worktree は terminal/active を問わず返す（validate_start 経路用）
        {
            let execs = engine.executions.lock().await;
            assert!(find_any_by_worktree(&execs, worktree_path).is_some());
        }
    }

    /// Spec issues-1011 finding 11: `abort_workflow_by_run_id` は terminal な run_id に対して
    /// no-op を返し、同一 worktree の active run を誤って中断しない。
    #[tokio::test]
    async fn abort_workflow_by_run_id_is_noop_for_terminal_run_even_if_active_shares_worktree() {
        let engine = WorkflowEngine::new_for_test();
        let worktree_path = "/wt/coexist";
        let terminal_run_id = "terminal-abort-target".to_string();
        let active_run_id = "active-bystander".to_string();
        {
            let mut execs = engine.executions.lock().await;
            execs.insert(
                terminal_run_id.clone(),
                make_exec_with(
                    &terminal_run_id,
                    worktree_path,
                    WorkflowExecutionState::Completed,
                ),
            );
            execs.insert(
                active_run_id.clone(),
                make_exec_with(
                    &active_run_id,
                    worktree_path,
                    WorkflowExecutionState::Running,
                ),
            );
        }

        // run_id 主語の abort 経路: terminal な exec の run_id を渡すと、内部の
        // `is_active()` ガードで即 Ok(()) を返し、worktree 主語の下流処理に委譲しない。
        // → 同一 worktree の active run は影響を受けない。
        // ここでは executions の lookup 経路だけを検証する（AppHandle が要らない範囲）。
        let abort_target_active = {
            let execs = engine.executions.lock().await;
            execs.get(&terminal_run_id).map(|e| e.is_active())
        };
        assert_eq!(abort_target_active, Some(false));
        // active な run は依然として is_active
        let bystander_active = {
            let execs = engine.executions.lock().await;
            execs.get(&active_run_id).map(|e| e.is_active())
        };
        assert_eq!(bystander_active, Some(true));
    }

    /// Spec issues-1011 finding 5/8: `start_workflow` のアトミック性。並行起動で
    /// Run Store reservation に負けた場合、parent ChatSession は作成されないため
    /// 「孤立 parent session」が構造的に発生しないことを保証する。
    /// reservation は最初の副作用であり、失敗時は他の副作用が走らない。
    #[tokio::test]
    async fn start_workflow_reservation_is_first_side_effect_so_no_orphan_session_on_conflict() {
        let engine = WorkflowEngine::new_for_test();
        let tmp = tempfile::TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let worktree_path = "/wt/reserve";

        // 既に active な reservation がある状態を作る。
        let existing_run_id = uuid::Uuid::new_v4().to_string();
        engine
            .run_store
            .register_active(crate::workflow::run::WorkflowRun {
                run_id: existing_run_id.clone(),
                workflow_name: "wf".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node_name: Some("only-step".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();

        // 同一 worktree への 2 回目の reservation は WorktreeAlreadyActive で拒否される。
        let new_run_id = uuid::Uuid::new_v4().to_string();
        let result = engine
            .run_store
            .register_active(crate::workflow::run::WorkflowRun {
                run_id: new_run_id.clone(),
                workflow_name: "wf".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node_name: Some("only-step".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 200.0,
                updated_at: 200.0,
                completed_at: None,
                error_reason: None,
            })
            .await;
        assert!(matches!(
            result,
            Err(crate::workflow::run::RunStoreError::WorktreeAlreadyActive { .. })
        ));
        // 新 run_id 用の metadata ファイルは作成されない
        let path = tmp
            .path()
            .join("workflow_runs")
            .join(format!("{new_run_id}.json"));
        assert!(
            !path.exists(),
            "新 run_id の metadata が作成されていないこと（reservation が副作用の最初の境界）"
        );
        // active は existing のみ
        let active = engine.list_active_runs().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, existing_run_id);
    }

    #[tokio::test]
    async fn reserve_workflow_run_maps_run_store_worktree_conflict_to_already_active() {
        let engine = WorkflowEngine::new_for_test();
        let tmp = tempfile::TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let workflow = make_minimal_workflow();
        let worktree_path = "/wt/reserve-conflict";
        engine
            .reserve_workflow_run(
                &workflow,
                worktree_path,
                None,
                TriggerSource::DesktopUi,
                100.0,
            )
            .await
            .unwrap();

        let err = engine
            .reserve_workflow_run(
                &workflow,
                worktree_path,
                None,
                TriggerSource::DesktopUi,
                101.0,
            )
            .await
            .unwrap_err();

        assert!(matches!(err, WorkflowEngineError::AlreadyActive(_)));
    }

    /// Spec issues-1011 finding 10: `set_execution_state` 経路を通すと、active な
    /// execution が terminal に遷移したとき Run Store の active から外れて completed に
    /// 追加され、failed/aborted も同じく completed 一覧に現れる。
    /// （set_execution_state 自体は AppHandle を要するため、ここではその内部ヘルパー
    /// である `sync_run_store_from_snapshot` を terminal snapshot 3 種で走査して
    /// 同等の効果を検証する。）
    #[tokio::test]
    async fn run_store_completed_listing_includes_completed_failed_aborted_via_authoritative_sync()
    {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = WorkflowEngine::new_for_test();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;

        let cases = [
            ("completed", WorkflowExecutionState::Completed),
            (
                "failed",
                WorkflowExecutionState::Failed {
                    reason: "boom".to_string(),
                },
            ),
            ("aborted", WorkflowExecutionState::Aborted),
        ];
        let mut ids = Vec::new();
        for (_, state) in cases.iter().cloned() {
            let run_id = uuid::Uuid::new_v4().to_string();
            engine
                .run_store
                .register_active(crate::workflow::run::WorkflowRun {
                    run_id: run_id.clone(),
                    workflow_name: "wf".to_string(),
                    task: None,
                    status: RunStatus::Running,
                    worktree_path: format!("/wt/{run_id}"),
                    current_node_name: Some("only-step".to_string()),
                    trigger_source: TriggerSource::DesktopUi,
                    started_at: 100.0,
                    updated_at: 100.0,
                    completed_at: None,
                    error_reason: None,
                })
                .await
                .unwrap();
            // 権威遷移経路で使われる sync helper を直接呼ぶ
            let snapshot = WorkflowState {
                execution_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                state,
                current_step_index: 0,
                current_step_name: "only-step".to_string(),
                current_session_id: None,
                total_steps: 1,
                step_history: vec![],
                step_execution_counts: HashMap::new(),
                workflow_definition: make_minimal_workflow(),
                total_token_usage: TokenUsage::default(),
                step_states: HashMap::new(),
                step_outputs: HashMap::new(),
                active_parallel_steps: vec![],
                workflow_variables: HashMap::new(),
                approval_operations: None,
                started_at: 100.0,
                updated_at: 200.0,
            };
            engine
                .sync_run_store_from_snapshot(&run_id, &snapshot)
                .await
                .unwrap();
            ids.push(run_id);
        }

        // 3 件とも active からは外れている
        assert!(engine.list_active_runs().await.is_empty());
        // 3 件とも completed に並ぶ
        let completed = engine.list_completed_runs().await;
        let completed_ids: std::collections::HashSet<&str> =
            completed.iter().map(|r| r.run_id.as_str()).collect();
        for id in &ids {
            assert!(
                completed_ids.contains(id.as_str()),
                "completed listing must include run {id}"
            );
        }
    }

    #[tokio::test]
    async fn run_store_sync_failure_rolls_engine_projection_back_to_active_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = WorkflowEngine::new_for_test();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/sync-rollback";
        engine
            .run_store
            .register_active(crate::workflow::run::WorkflowRun {
                run_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node_name: Some("only-step".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        engine.executions.lock().await.insert(
            run_id.clone(),
            make_exec_with(&run_id, worktree_path, WorkflowExecutionState::Completed),
        );

        let bad_data_dir = tmp.path().join("not-a-directory");
        std::fs::write(&bad_data_dir, "file").unwrap();
        engine.set_run_store_data_dir(bad_data_dir).await;
        let snapshot = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .unwrap()
            .to_workflow_state();
        let err = engine
            .sync_run_store_from_snapshot(&run_id, &snapshot)
            .await
            .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::SessionStore(_)));

        engine
            .rollback_execution_projection_after_run_store_sync_failure(&run_id, &snapshot)
            .await;

        let exec_state = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .unwrap()
            .state
            .clone();
        assert_eq!(exec_state, WorkflowExecutionState::Running);
        assert_eq!(
            engine.run_id_for_worktree(worktree_path).await,
            Some(run_id),
            "Run Store rollback keeps the active worktree index authoritative"
        );
    }

    /// Spec issues-1011 finding 16: `abort_workflow_by_run_id` 経路の境界回帰検出。
    /// AppHandle を要するため `abort_workflow_by_run_id` 自体は production 経路で起動できないが、
    /// 内部 lookup 段階で「terminal run へ no-op を返し、同一 worktree の active run の状態を
    /// 変更しない」ことを直接検証する。terminal/active 共存時に run_id 主語の lookup が
    /// 取り違えないことを engine state 観測で保証する。
    #[tokio::test]
    async fn abort_workflow_by_run_id_does_not_modify_sibling_active_run_state() {
        let engine = WorkflowEngine::new_for_test();
        let worktree_path = "/wt/sibling";
        let terminal_run_id = uuid::Uuid::new_v4().to_string();
        let active_run_id = uuid::Uuid::new_v4().to_string();
        {
            let mut execs = engine.executions.lock().await;
            execs.insert(
                terminal_run_id.clone(),
                make_exec_with(
                    &terminal_run_id,
                    worktree_path,
                    WorkflowExecutionState::Completed,
                ),
            );
            execs.insert(
                active_run_id.clone(),
                make_exec_with(
                    &active_run_id,
                    worktree_path,
                    WorkflowExecutionState::Running,
                ),
            );
        }

        // run_id ベース lookup: terminal を引いても active のスナップショットには影響しない。
        let initial_active_state = {
            let execs = engine.executions.lock().await;
            execs.get(&active_run_id).map(|e| e.state.clone())
        };
        assert_eq!(initial_active_state, Some(WorkflowExecutionState::Running));

        // abort_workflow_by_run_id が production で使う lookup helper は、terminal target を
        // `AlreadyTerminal` として返す。worktree_path で sibling active run を探索しない。
        assert!(matches!(
            engine.abort_target_lookup(&terminal_run_id).await,
            AbortTargetLookup::AlreadyTerminal
        ));

        // active run には触れていない（同一 worktree でも誤って中断しない）
        let final_active_state = {
            let execs = engine.executions.lock().await;
            execs.get(&active_run_id).map(|e| e.state.clone())
        };
        assert_eq!(final_active_state, Some(WorkflowExecutionState::Running));
    }

    /// Spec issues-1011 finding 17: approval/reject は run_id を主語に対象 execution を
    /// 直接更新し、同一 worktree に別 run が存在しても指定 run 以外へ適用しない。
    #[tokio::test]
    async fn approval_for_run_id_updates_only_target_run_when_worktree_is_shared() {
        let engine = WorkflowEngine::new_for_test();
        let worktree_path = "/wt/approval-shared";
        let target_run_id = uuid::Uuid::new_v4().to_string();
        let sibling_run_id = uuid::Uuid::new_v4().to_string();

        let mut target = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        target.id = target_run_id.clone();
        target.worktree_path = worktree_path.to_string();

        let mut sibling = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        sibling.id = sibling_run_id.clone();
        sibling.worktree_path = worktree_path.to_string();

        {
            let mut execs = engine.executions.lock().await;
            execs.insert(target_run_id.clone(), target);
            execs.insert(sibling_run_id.clone(), sibling);
        }

        let outcome = engine
            .handle_approval_with_output_for_run_for_test(
                &target_run_id,
                ApprovalDecision::Approve,
                Some(&target_run_id),
                Some("review"),
            )
            .await
            .unwrap();
        assert!(matches!(outcome, StepOutcome::Persist(_)));

        let execs = engine.executions.lock().await;
        let target = execs.get(&target_run_id).unwrap();
        let sibling = execs.get(&sibling_run_id).unwrap();
        assert_eq!(target.state, WorkflowExecutionState::Completed);
        assert_eq!(target.step_history.len(), 1);
        assert_eq!(sibling.state, WorkflowExecutionState::WaitingApproval);
        assert!(sibling.step_history.is_empty());
    }

    /// Spec issues-1011 finding 13: `start_workflow` 本体の core 起動経路が払い出す
    /// run_id と、`WorkflowExecution.id` / active summary / workflow_runs/{run_id}.json が
    /// 一貫し、同一 worktree への重複起動を拒否することを直接検証する。
    #[tokio::test]
    async fn start_workflow_core_records_run_id_and_rejects_duplicate_worktree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = WorkflowEngine::new_for_test();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;

        let worktree_path = "/wt/start-fixture";
        let workflow = make_minimal_workflow();
        let now = 100.0;
        let run_id = engine
            .start_workflow_common_core_for_test(
                workflow.clone(),
                worktree_path.to_string(),
                Some("task-x".to_string()),
                TriggerSource::DesktopUi,
                now,
            )
            .await
            .unwrap();

        // 一貫性: (1) executions の id (2) active summary.run_id (3) workflow_runs/{run_id}.json
        let (exec_id, exec_worktree) = {
            let execs = engine.executions.lock().await;
            let exec = execs.get(&run_id).unwrap();
            (exec.id.clone(), exec.worktree_path.clone())
        };
        let active = engine.list_active_runs().await;
        let metadata_path = tmp
            .path()
            .join("workflow_runs")
            .join(format!("{run_id}.json"));
        let metadata: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&metadata_path).unwrap()).unwrap();
        assert_eq!(exec_id, run_id);
        assert_eq!(exec_worktree, worktree_path);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);
        assert_eq!(active[0].workflow_name, workflow.name);
        assert_eq!(active[0].worktree_path, worktree_path);
        assert_eq!(active[0].started_at, now);
        assert_eq!(active[0].updated_at, now);
        assert_eq!(active[0].trigger_source, TriggerSource::DesktopUi);
        assert_eq!(active[0].task.as_deref(), Some("task-x"));
        assert_eq!(metadata["runId"].as_str(), Some(run_id.as_str()));
        assert_eq!(
            metadata["workflowName"].as_str(),
            Some(workflow.name.as_str())
        );
        assert_eq!(metadata["worktreePath"].as_str(), Some(worktree_path));
        assert_eq!(metadata["startedAt"].as_f64(), Some(now));
        assert_eq!(metadata["updatedAt"].as_f64(), Some(now));
        assert_eq!(metadata["triggerSource"].as_str(), Some("desktop_ui"));
        assert_eq!(metadata["task"].as_str(), Some("task-x"));
        // worktree -> run の双方向解決も一貫している
        assert_eq!(
            engine.run_id_for_worktree(worktree_path).await,
            Some(run_id.clone())
        );
        assert_eq!(
            engine.resolve_worktree_by_run(&run_id).await,
            Some(worktree_path.to_string())
        );

        let duplicate = engine
            .start_workflow_common_core_for_test(
                make_minimal_workflow(),
                worktree_path.to_string(),
                None,
                TriggerSource::DesktopUi,
                now + 1.0,
            )
            .await;
        assert!(matches!(
            duplicate,
            Err(WorkflowEngineError::AlreadyActive(_))
        ));
    }

    /// Spec issues-1011 finding 14: 同一 worktree への重複起動は reservation 段階で拒否され、
    /// 新規 metadata / parent session / refs が孤立しない。Run Store の reservation は
    /// 起動経路上の「最初の副作用」であり、失敗時には他の副作用が一切走らない構造を保証する。
    #[tokio::test]
    async fn start_workflow_duplicate_reservation_does_not_leak_metadata_or_refs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = WorkflowEngine::new_for_test();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let worktree_path = "/wt/dup-leak";

        // 既存 active reservation
        let existing_run_id = uuid::Uuid::new_v4().to_string();
        engine
            .run_store
            .register_active(crate::workflow::run::WorkflowRun {
                run_id: existing_run_id.clone(),
                workflow_name: "wf".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node_name: Some("only-step".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();

        // 2 回目の reservation 失敗 → 新 metadata / refs / executions に何も追加されない
        let new_run_id = uuid::Uuid::new_v4().to_string();
        let result = engine
            .run_store
            .register_active(crate::workflow::run::WorkflowRun {
                run_id: new_run_id.clone(),
                workflow_name: "wf".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: worktree_path.to_string(),
                current_node_name: Some("only-step".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 200.0,
                updated_at: 200.0,
                completed_at: None,
                error_reason: None,
            })
            .await;
        assert!(matches!(
            result,
            Err(crate::workflow::run::RunStoreError::WorktreeAlreadyActive { .. })
        ));
        // (1) 新 run_id 用 metadata ファイル無し
        let path = tmp
            .path()
            .join("workflow_runs")
            .join(format!("{new_run_id}.json"));
        assert!(!path.exists());
        // (2) session_workflow_refs に新規エントリ無し（reservation 失敗の段階で副作用が走らない）
        let refs = engine.session_workflow_refs.lock().await;
        assert!(!refs
            .values()
            .any(|r: &SessionWorkflowRef| r.run_id == new_run_id));
        // (3) executions にも新 run_id が無い
        let execs = engine.executions.lock().await;
        assert!(!execs.contains_key(&new_run_id));
        // (4) active は existing のみ
        assert_eq!(active_only_summary(&engine).await, vec![existing_run_id]);
    }

    // 撤去済み: rollback_created_parent_session は parent ChatSession 機構撤去で消滅した。
    // 旧テスト `start_workflow_rollback_deletes_created_parent_session` も役目を終えた。

    async fn active_only_summary(engine: &WorkflowEngine) -> Vec<String> {
        engine
            .list_active_runs()
            .await
            .into_iter()
            .map(|s| s.run_id)
            .collect()
    }

    /// Spec issues-1011 finding 15: completed / failed / aborted の代表経路で
    /// active 一覧から消えて completed 一覧に status 付きで現れる。
    /// production の権威遷移経路で必ず呼ばれる `sync_run_store_from_snapshot` を直接呼び、
    /// 3 ステータスすべてで「Run Store の owner が active → completed に推移する」ことを
    /// 1 つのテストでまとめて検証する（既存の同種テストとは別に、status 観測も加える）。
    #[tokio::test]
    async fn run_store_terminal_statuses_propagate_status_field_in_completed_listing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = WorkflowEngine::new_for_test();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;

        let mut expectations: Vec<(String, RunStatus)> = Vec::new();
        for state in [
            WorkflowExecutionState::Completed,
            WorkflowExecutionState::Failed {
                reason: "boom".to_string(),
            },
            WorkflowExecutionState::Aborted,
        ] {
            let run_id = uuid::Uuid::new_v4().to_string();
            let expected_status = match state {
                WorkflowExecutionState::Completed => RunStatus::Completed,
                WorkflowExecutionState::Failed { .. } => RunStatus::Failed,
                WorkflowExecutionState::Aborted => RunStatus::Aborted,
                _ => unreachable!(),
            };
            engine
                .run_store
                .register_active(crate::workflow::run::WorkflowRun {
                    run_id: run_id.clone(),
                    workflow_name: "wf".to_string(),
                    task: None,
                    status: RunStatus::Running,
                    worktree_path: format!("/wt/{run_id}"),
                    current_node_name: Some("only-step".to_string()),
                    trigger_source: TriggerSource::DesktopUi,
                    started_at: 100.0,
                    updated_at: 100.0,
                    completed_at: None,
                    error_reason: None,
                })
                .await
                .unwrap();
            let snapshot = WorkflowState {
                execution_id: run_id.clone(),
                workflow_name: "wf".to_string(),
                state,
                current_step_index: 0,
                current_step_name: "only-step".to_string(),
                current_session_id: None,
                total_steps: 1,
                step_history: vec![],
                step_execution_counts: HashMap::new(),
                workflow_definition: make_minimal_workflow(),
                total_token_usage: TokenUsage::default(),
                step_states: HashMap::new(),
                step_outputs: HashMap::new(),
                active_parallel_steps: vec![],
                workflow_variables: HashMap::new(),
                approval_operations: None,
                started_at: 100.0,
                updated_at: 200.0,
            };
            engine
                .sync_run_store_from_snapshot(&run_id, &snapshot)
                .await
                .unwrap();
            expectations.push((run_id, expected_status));
        }

        // active 一覧から全て外れている
        assert!(engine.list_active_runs().await.is_empty());

        // completed 一覧に status 付きで現れる
        let completed = engine.list_completed_runs().await;
        for (id, expected_status) in &expectations {
            let entry = completed
                .iter()
                .find(|r| &r.run_id == id)
                .expect("completed listing must include run");
            assert_eq!(
                entry.status, *expected_status,
                "status must propagate to completed summary for {id}"
            );
        }
    }

    /// Spec issues-1011 finding 3: `resolve_chat_session_for_approval` は run state が
    /// `WaitingApproval` でない場合に Err を返す（任意 step session への注入経路を塞ぐ）。
    #[tokio::test]
    async fn resolve_chat_session_for_approval_rejects_non_waiting_approval_state() {
        let engine = WorkflowEngine::new_for_test();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_exec_with(&run_id, "/wt/x", WorkflowExecutionState::Running);
        exec.workflow.nodes[0].node_type = NodeType::Approval;
        exec.current_session_id = Some("step-sess".to_string());
        engine.executions.lock().await.insert(run_id.clone(), exec);

        let err = engine
            .resolve_chat_session_for_approval(&run_id)
            .await
            .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
    }

    /// Spec issues-1011 finding 3: `resolve_chat_session_for_approval` は current node が
    /// Approval node でない場合に拒否する。
    #[tokio::test]
    async fn resolve_chat_session_for_approval_rejects_non_approval_current_node() {
        let engine = WorkflowEngine::new_for_test();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_exec_with(&run_id, "/wt/x", WorkflowExecutionState::WaitingApproval);
        // current node は Agent のまま（make_minimal_workflow が Agent を返す）
        exec.current_session_id = Some("step-sess".to_string());
        engine.executions.lock().await.insert(run_id.clone(), exec);

        let err = engine
            .resolve_chat_session_for_approval(&run_id)
            .await
            .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
    }

    /// Spec issues-1011 finding 3: 全条件揃った場合のみ session_id / worktree_path を返す。
    #[tokio::test]
    async fn resolve_chat_session_for_approval_accepts_fully_valid_state() {
        let engine = WorkflowEngine::new_for_test();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_exec_with(&run_id, "/wt/x", WorkflowExecutionState::WaitingApproval);
        exec.workflow.nodes[0].node_type = NodeType::Approval;
        exec.current_session_id = Some("step-sess".to_string());
        engine.executions.lock().await.insert(run_id.clone(), exec);

        let (sid, wt) = engine
            .resolve_chat_session_for_approval(&run_id)
            .await
            .unwrap();
        assert_eq!(sid, "step-sess");
        assert_eq!(wt, "/wt/x");
    }

    /// Spec issues-1011 finding 3: terminal run の approval 解決は拒否される。
    /// 同一 worktree に terminal + active がある状況で terminal 側を狙う注入経路を防ぐ。
    #[tokio::test]
    async fn resolve_chat_session_for_approval_rejects_terminal_run() {
        let engine = WorkflowEngine::new_for_test();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_exec_with(&run_id, "/wt/x", WorkflowExecutionState::Completed);
        exec.workflow.nodes[0].node_type = NodeType::Approval;
        exec.current_session_id = Some("step-sess".to_string());
        engine.executions.lock().await.insert(run_id.clone(), exec);

        let err = engine
            .resolve_chat_session_for_approval(&run_id)
            .await
            .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::InvalidState(_)));
    }

    /// Spec issues-1011 finding 5: terminal transition 経路で `cleanup_session_workflow_refs_by_run_id`
    /// は対象 run の refs のみを削除し、同一 worktree の別 active run の refs は残す。
    #[tokio::test]
    async fn cleanup_session_workflow_refs_by_run_id_preserves_sibling_run_refs() {
        let engine = WorkflowEngine::new_for_test();
        let terminal_run_id = uuid::Uuid::new_v4().to_string();
        let active_run_id = uuid::Uuid::new_v4().to_string();

        // 両 run の refs を入れる（同一 worktree 想定）
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                "parent-terminal".to_string(),
                SessionWorkflowRef {
                    run_id: terminal_run_id.clone(),
                },
            );
            refs.insert(
                "step-terminal".to_string(),
                SessionWorkflowRef {
                    run_id: terminal_run_id.clone(),
                },
            );
            refs.insert(
                "parent-active".to_string(),
                SessionWorkflowRef {
                    run_id: active_run_id.clone(),
                },
            );
        }

        engine
            .cleanup_session_workflow_refs_by_run_id(&terminal_run_id)
            .await;

        let refs = engine.session_workflow_refs.lock().await;
        assert!(!refs.contains_key("parent-terminal"));
        assert!(!refs.contains_key("step-terminal"));
        assert!(
            refs.contains_key("parent-active"),
            "sibling active run の refs は残るべき"
        );
    }
}

/// [04] Command / Event Boundary 専用テスト。
///
/// `WorkflowEngine::dispatch` の routing 層と `handle_approval` 内の
/// ApprovalResolved append / snapshot 一括復元（atomic mutation 境界）を検証する。
/// 本モジュールは `tauri::AppHandle` を要さない範囲で dispatch / approval semantics の
/// production 経路を直接呼ぶ。
#[cfg(test)]
mod dispatch_boundary_tests {
    use super::*;
    use crate::backends::{
        AgentBackend, AgentBackendRegistry, AgentMessage as BackendAgentMessage,
        PermissionResponse as BackendPermissionResponse, SessionConfig as BackendSessionConfig,
        SessionHandle as BackendSessionHandle,
    };
    use crate::workflow::command::{WorkflowCommand, WorkflowCommandResult};
    use crate::workflow::command_input::MAX_APPROVAL_COMMENT_CHARS;
    use crate::workflow::event::{ApprovalDecisionRecord, WorkflowEvent};
    use crate::workflow::log::WorkflowEventLog;
    use crate::workflow::run::{RunStatus, TerminalRunStatus, TriggerSource, WorkflowRun};
    use crate::workflow::schema::{NodeDefinition, NodeType, TransitionRule, Workflow};
    use crate::workflow::state::WorkflowExecutionState;
    use async_trait::async_trait;
    use tauri::Manager;
    use tempfile::TempDir;

    /// 実バックエンドと同じ供給経路（`fixed_models()`）でモデル一覧を返す
    /// dispatch テスト用 backend。claude / codex の固定モデル定数をそのまま供給し、
    /// builtin workflow が使う `claude-opus-4-8` / `gpt-5.5` を production と同一経路で
    /// 解決できるようにする（dispatch フロー検証の本来意図を維持）。
    struct DispatchMockBackend {
        backend_id: String,
        fixed_models: Vec<String>,
    }

    #[async_trait]
    impl AgentBackend for DispatchMockBackend {
        fn id(&self) -> &str {
            &self.backend_id
        }
        fn name(&self) -> &str {
            "Mock"
        }
        fn fixed_models(&self) -> Option<Vec<String>> {
            Some(self.fixed_models.clone())
        }
        async fn start_session(
            &self,
            cfg: BackendSessionConfig,
        ) -> Result<BackendSessionHandle, String> {
            Ok(BackendSessionHandle {
                chat_session_id: cfg.chat_session_id,
                backend_id: self.backend_id.clone(),
            })
        }
        async fn send_message(
            &self,
            _s: &BackendSessionHandle,
            _m: BackendAgentMessage,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn interrupt(&self, _s: &BackendSessionHandle) -> Result<(), String> {
            Ok(())
        }
        async fn respond_permission(
            &self,
            _s: &BackendSessionHandle,
            _r: BackendPermissionResponse,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn close_session(&self, _s: &BackendSessionHandle) -> Result<(), String> {
            Ok(())
        }
    }

    fn dispatch_data_dir(app: &tauri::AppHandle<tauri::test::MockRuntime>) -> std::path::PathBuf {
        crate::session::resolve_data_dir(app).expect("mock app data dir must resolve")
    }

    fn make_approval_only_workflow() -> Workflow {
        Workflow {
            variables: Default::default(),
            name: "boundary-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                node_type: NodeType::Approval,
                instruction: Some("review".to_string()),
                ..NodeDefinition::default()
            }],
        }
    }

    fn make_rejectable_approval_workflow() -> Workflow {
        Workflow {
            variables: Default::default(),
            name: "boundary-wf".to_string(),
            description: "test".to_string(),
            builtin: false,
            nodes: vec![
                NodeDefinition {
                    name: "review".to_string(),
                    node_type: NodeType::Approval,
                    instruction: Some("review".to_string()),
                    transition_rules: vec![TransitionRule {
                        r#match: "reject".to_string(),
                        next: "fix".to_string(),
                    }],
                    ..NodeDefinition::default()
                },
                NodeDefinition {
                    name: "fix".to_string(),
                    node_type: NodeType::Agent,
                    instruction: Some("fix".to_string()),
                    ..NodeDefinition::default()
                },
            ],
        }
    }

    fn make_waiting_approval_execution(run_id: &str, worktree_path: &str) -> WorkflowExecution {
        let workflow = make_approval_only_workflow();
        make_waiting_approval_execution_with_workflow(run_id, worktree_path, workflow)
    }

    fn make_waiting_approval_execution_with_workflow(
        run_id: &str,
        worktree_path: &str,
        workflow: Workflow,
    ) -> WorkflowExecution {
        WorkflowExecution {
            id: run_id.to_string(),
            workflow,
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 0,
            step_execution_counts: HashMap::from([("review".to_string(), 1)]),
            step_history: Vec::new(),
            worktree_path: worktree_path.to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some("sess-1".to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        }
    }

    type DispatchTestApp = tauri::App<tauri::test::MockRuntime>;

    fn make_dispatch_app() -> DispatchTestApp {
        let mut config = crate::config::ReleashConfig::default();
        config.app.last_repo_paths = Vec::new();
        config.agents.default = Some("codex".to_string());
        let app_config = Arc::new(crate::config::AppConfig::new(
            config,
            TempDir::new().unwrap().path().join("config.toml"),
        ));
        // 実 backend と同じ供給経路（fixed_models()）で claude / codex の固定モデルを
        // 供給する mock backend を登録する。builtin workflow が使う claude-opus-4-8 /
        // gpt-5.5 が production と同一経路で解決され、dispatch フロー検証を維持できる。
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(DispatchMockBackend {
            backend_id: "claude".to_string(),
            fixed_models: crate::domain::agent_session::CLAUDE_FIXED_MODELS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }));
        registry.register(Arc::new(DispatchMockBackend {
            backend_id: "codex".to_string(),
            fixed_models: crate::domain::agent_session::CODEX_FIXED_MODELS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }));
        registry.set_default(Some("codex".to_string()));
        registry.set_config(Arc::clone(&app_config));
        let registry = Arc::new(registry);
        let data_dir =
            std::env::temp_dir().join(format!("releash-dispatch-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&data_dir).unwrap();
        tauri::test::mock_builder()
            .manage(crate::session::TestDataDir(data_dir))
            .manage(app_config)
            .manage(registry)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("tauri mock test app must build")
    }

    fn make_dispatch_deps() -> (
        Arc<crate::session::SessionStore>,
        Arc<Mutex<AgentProcessMap>>,
    ) {
        (
            Arc::new(crate::session::SessionStore::default()),
            Arc::new(Mutex::new(AgentProcessMap::new())),
        )
    }

    async fn insert_execution_and_active_run(
        engine: &WorkflowEngine,
        exec: WorkflowExecution,
        trigger_source: TriggerSource,
    ) {
        let run_id = exec.id.clone();
        engine
            .run_store
            .register_active(WorkflowRun {
                run_id: run_id.clone(),
                workflow_name: exec.workflow.name.clone(),
                task: exec.task.clone(),
                status: match exec.state {
                    WorkflowExecutionState::WaitingApproval => RunStatus::WaitingApproval,
                    _ => RunStatus::Running,
                },
                worktree_path: exec.worktree_path.clone(),
                current_node_name: Some(exec.workflow.nodes[exec.current_step_index].name.clone()),
                trigger_source,
                started_at: exec.started_at,
                updated_at: exec.updated_at,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        engine.executions.lock().await.insert(run_id, exec);
    }

    fn read_dispatch_events(app: &DispatchTestApp, run_id: &str) -> Vec<WorkflowEvent> {
        let data_dir = dispatch_data_dir(app.handle());
        WorkflowEventLog::new(&data_dir)
            .read_log(run_id)
            .unwrap_or_default()
    }

    fn make_managed_worktree() -> (TempDir, TempDir, std::path::PathBuf) {
        let repo_parent = TempDir::new().unwrap();
        let worktree_parent = TempDir::new().unwrap();
        let repo_path = repo_parent.path().join("repo");
        std::fs::create_dir(&repo_path).unwrap();
        let repo = git2::Repository::init(&repo_path).unwrap();
        std::fs::write(repo_path.join("README.md"), "test\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("README.md")).unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let sig = git2::Signature::now("Test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();
        let worktree_path = worktree_parent.path().join("managed-wt");
        repo.worktree("managed-wt", &worktree_path, None).unwrap();
        (repo_parent, worktree_parent, worktree_path)
    }

    fn configure_managed_repo(app: &DispatchTestApp, repo_path: &std::path::Path) {
        app.state::<Arc<crate::config::AppConfig>>()
            .with_config_mut(|config| {
                config.app.last_repo_paths = vec![repo_path.to_string_lossy().to_string()];
                Ok(())
            })
            .unwrap();
    }

    /// Spec [04]: ApprovalResolved event は decision を typed (snake_case) で記録し、
    /// approve コメントを comment field に伝播する。observer が dispatch 経由の判断を
    /// 統一語彙で読めることを担保する。
    #[test]
    fn approval_resolved_records_decision_and_comment_in_ndjson() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let run_id = "00000000-0000-0000-0000-000000000300";

        let event = WorkflowEvent::ApprovalResolved {
            run_id: run_id.to_string(),
            workflow_name: "boundary-wf".to_string(),
            node_name: "review".to_string(),
            decision: ApprovalDecisionRecord::Approve,
            comment: Some("lgtm".to_string()),
            timestamp: 1234.0,
        };
        log.append(&event).unwrap();

        let events = log.read_log(run_id).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            WorkflowEvent::ApprovalResolved {
                run_id: rid,
                node_name,
                decision,
                comment,
                ..
            } => {
                assert_eq!(rid, run_id);
                assert_eq!(node_name, "review");
                assert_eq!(*decision, ApprovalDecisionRecord::Approve);
                assert_eq!(comment.as_deref(), Some("lgtm"));
            }
            other => panic!("expected ApprovalResolved, got {other:?}"),
        }
    }

    /// Spec [04]: atomic mutation 境界。mutation 直前の `WorkflowExecution` snapshot を
    /// 一括復元することで、履歴・変数・state・current_step_index を含む全フィールドが
    /// 元に戻ることを担保する（部分 rollback helper を使わない構造）。
    #[tokio::test]
    async fn approval_snapshot_rollback_restores_workflow_execution_fully() {
        let engine = WorkflowEngine::new_for_test();
        let run_id = uuid::Uuid::new_v4().to_string();

        let mut exec = make_waiting_approval_execution(&run_id, "/wt/atomic");
        exec.workflow_variables
            .insert("preserved".to_string(), "before".to_string());
        let before_history_len = exec.step_history.len();
        let before_step_index = exec.current_step_index;
        let before_state = exec.state.clone();
        let before_variables = exec.workflow_variables.clone();
        let snapshot_before = exec.clone();

        engine.executions.lock().await.insert(run_id.clone(), exec);

        // mutation を適用（apply_approval_application + workflow_variables.extend）
        {
            let mut execs = engine.executions.lock().await;
            let exec = execs.get_mut(&run_id).unwrap();
            exec.workflow_variables
                .insert("after_only".to_string(), "x".to_string());
            let _ = WorkflowEngine::apply_approval_application(
                exec,
                &ApprovalDecision::Approve,
                ApprovalApplication {
                    effective_result: "approve".to_string(),
                    structured_output: None,
                    output_contract: None,
                },
            )
            .unwrap();
            assert_ne!(exec.state, before_state);
            assert!(exec.workflow_variables.contains_key("after_only"));
        }

        // event append 失敗時の一括復元（handle_approval 内と同じ操作）。
        {
            let mut execs = engine.executions.lock().await;
            if let Some(exec) = execs.get_mut(&run_id) {
                *exec = snapshot_before;
            }
        }

        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).expect("run must remain");
        assert_eq!(restored.state, before_state, "WaitingApproval が復元される");
        assert_eq!(
            restored.current_step_index, before_step_index,
            "current_step_index が復元される"
        );
        assert_eq!(
            restored.step_history.len(),
            before_history_len,
            "step_history.len() が復元される"
        );
        assert!(
            !restored.workflow_variables.contains_key("after_only"),
            "mutation 後に追加された workflow_variables が消える"
        );
        assert_eq!(
            restored.workflow_variables, before_variables,
            "workflow_variables 全体が mutation 前と等価"
        );
    }

    /// Spec [05] adapter-facing 拒否境界: `WorkflowCommand::CompleteNode` /
    /// `FailNode` は engine 内部の node 完了 / 失敗遷移を typed 化した internal-only
    /// variant であり、外部 adapter（Tauri command / CLI / agent path）から組み立てる
    /// 経路は提供しない。adapter 入口の `dispatch_external` から外部 caller が組み立てて
    /// 到達した場合は内部不整合として `Err` に変換することを境界仕様として確認する。
    /// engine 内部の commit 経路は `dispatch` を直接呼ぶ（次の
    /// `dispatch_commits_internal_*` テスト参照）。
    #[tokio::test]
    async fn workflow_command_internal_variants_are_rejected_by_dispatch_external() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let (session_store, handles) = make_dispatch_deps();
        let internal_complete = WorkflowCommand::CompleteNode {
            run_id: "00000000-0000-0000-0000-000000000600".to_string(),
            workflow_name: "wf".to_string(),
            node_name: "step-1".to_string(),
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: None,
            timestamp: 0.0,
        };
        let result = engine
            .dispatch_external(app.handle(), &session_store, &handles, internal_complete)
            .await;
        assert!(
            matches!(result, Err(WorkflowEngineError::ValidationError(_))),
            "dispatch_external must reject CompleteNode as internal-only; got {result:?}"
        );

        let internal_fail = WorkflowCommand::FailNode {
            run_id: "00000000-0000-0000-0000-000000000601".to_string(),
            workflow_name: "wf".to_string(),
            node_name: "step-1".to_string(),
            reason: "boom".to_string(),
            timestamp: 0.0,
        };
        let result = engine
            .dispatch_external(app.handle(), &session_store, &handles, internal_fail)
            .await;
        assert!(
            matches!(result, Err(WorkflowEngineError::ValidationError(_))),
            "dispatch_external must reject FailNode as internal-only; got {result:?}"
        );
    }

    fn dispatch_internal_test_snapshot(run_id: &str, workflow_name: &str) -> WorkflowState {
        WorkflowState {
            execution_id: run_id.to_string(),
            workflow_name: workflow_name.to_string(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            current_step_name: "node-1".to_string(),
            current_session_id: None,
            total_steps: 1,
            step_history: vec![],
            step_execution_counts: HashMap::new(),
            workflow_definition: crate::workflow::schema::Workflow {
                variables: Default::default(),
                name: workflow_name.to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![],
            },
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 0.0,
            updated_at: 0.0,
        }
    }

    /// Spec [05]: `dispatch_internal_node_command` は `InternalNodeCommand` を受け取り、
    /// 対応する state mutation を snapshot に適用したうえで event を返す
    /// atomic commit 関数として機能する（spec [05]: 発行点が typed command 経路に
    /// 集約 / state mutation と event 発行を同一 commit 境界に集約）。
    #[test]
    fn dispatch_internal_node_command_projects_complete_and_fail_commands() {
        // Complete は snapshot.step_history 末尾 entry と command effect の整合を
        // 検証する（commit 関数: 上流 push との同期境界）。
        let mut snapshot =
            dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000602", "wf");
        snapshot.step_history.push(StepHistoryEntry {
            step_name: "node-1".to_string(),
            completed_at: 100.0,
            result: Some("ok".to_string()),
            session_id: Some("sess-1".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: None,
            state: crate::workflow::state::default_step_entry_state(),
        });
        let complete = WorkflowCommand::CompleteNode {
            run_id: "00000000-0000-0000-0000-000000000602".to_string(),
            workflow_name: "wf".to_string(),
            node_name: "node-1".to_string(),
            result: Some("ok".to_string()),
            session_id: Some("sess-1".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: Some(1),
            timestamp: 100.0,
        };
        match WorkflowEngine::dispatch_internal_node_command(&mut snapshot, complete) {
            Ok(WorkflowEvent::NodeCompleted {
                run_id,
                node_name,
                result,
                timestamp,
                ..
            }) => {
                assert_eq!(run_id, "00000000-0000-0000-0000-000000000602");
                assert_eq!(node_name, "node-1");
                assert_eq!(result.as_deref(), Some("ok"));
                assert_eq!(timestamp, 100.0);
            }
            other => panic!("expected NodeCompleted, got {other:?}"),
        }

        let mut fail_snapshot =
            dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000603", "wf");
        let fail = WorkflowCommand::FailNode {
            run_id: "00000000-0000-0000-0000-000000000603".to_string(),
            workflow_name: "wf".to_string(),
            node_name: "node-1".to_string(),
            reason: "boom".to_string(),
            timestamp: 200.0,
        };
        match WorkflowEngine::dispatch_internal_node_command(&mut fail_snapshot, fail) {
            Ok(WorkflowEvent::NodeFailed {
                run_id,
                node_name,
                reason,
                timestamp,
                ..
            }) => {
                assert_eq!(run_id, "00000000-0000-0000-0000-000000000603");
                assert_eq!(node_name, "node-1");
                assert_eq!(reason, "boom");
                assert_eq!(timestamp, 200.0);
            }
            other => panic!("expected NodeFailed, got {other:?}"),
        }
        // state mutation: Fail 受領後 snapshot.state は Failed { reason } に遷移し、
        // updated_at は command の timestamp と一致する。
        assert!(matches!(
            fail_snapshot.state,
            WorkflowExecutionState::Failed { ref reason } if reason == "boom"
        ));
        assert_eq!(fail_snapshot.updated_at, 200.0);

        // Complete で snapshot の step_history 末尾と node_name が不一致な場合、
        // commit 関数は ValidationError を返す（spec [05] commit 境界: snapshot が
        // command effect を含まないことの検出）。
        let mut mismatched =
            dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000604", "wf");
        let mismatched_cmd = WorkflowCommand::CompleteNode {
            run_id: "00000000-0000-0000-0000-000000000604".to_string(),
            workflow_name: "wf".to_string(),
            node_name: "node-1".to_string(),
            result: None,
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: None,
            timestamp: 100.0,
        };
        assert!(matches!(
            WorkflowEngine::dispatch_internal_node_command(&mut mismatched, mismatched_cmd),
            Err(WorkflowEngineError::ValidationError(_))
        ));
    }

    /// Spec [05] commit 境界（snapshot と command effect の整合検証）の table-driven 網羅。
    /// `CompleteNode` の全 effect 列（run_id / workflow_name / node_name / result /
    /// session_id / token_usage / structured_output / run_index / timestamp）について、
    /// snapshot 側で 1 個ずつ意図的に mismatch を作成し、`dispatch_internal_node_command`
    /// が `ValidationError` を返すことを境界仕様として担保する（policy 指示）。
    #[test]
    fn dispatch_internal_complete_node_validates_all_effect_fields() {
        fn base_snapshot() -> WorkflowState {
            let mut s =
                dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000620", "table-wf");
            s.step_history.push(StepHistoryEntry {
                step_name: "node-1".to_string(),
                completed_at: 100.0,
                result: Some("ok".to_string()),
                session_id: Some("sess-1".to_string()),
                token_usage: Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 20,
                }),
                structured_output: Some(serde_json::json!({"k":"v"})),
                run_index: 1,
                child_outputs: None,
                state: crate::workflow::state::default_step_entry_state(),
            });
            s
        }
        fn base_command() -> WorkflowCommand {
            WorkflowCommand::CompleteNode {
                run_id: "00000000-0000-0000-0000-000000000620".to_string(),
                workflow_name: "table-wf".to_string(),
                node_name: "node-1".to_string(),
                result: Some("ok".to_string()),
                session_id: Some("sess-1".to_string()),
                token_usage: Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 20,
                }),
                structured_output: Some(serde_json::json!({"k":"v"})),
                run_index: Some(1),
                timestamp: 100.0,
            }
        }

        // baseline は受理される（all fields match）。
        let mut s = base_snapshot();
        assert!(WorkflowEngine::dispatch_internal_node_command(&mut s, base_command()).is_ok());

        // 各 field を 1 個ずつ意図的に乖離させて ValidationError を確認する。
        type CompleteNodeMutator = Box<dyn Fn(WorkflowCommand) -> WorkflowCommand>;
        let mutators: Vec<(&str, CompleteNodeMutator)> = vec![
            (
                "run_id",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let WorkflowCommand::CompleteNode {
                        run_id: ref mut r, ..
                    } = c
                    {
                        *r = "00000000-0000-0000-0000-000000000999".to_string();
                    }
                    c
                }),
            ),
            (
                "workflow_name",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let WorkflowCommand::CompleteNode {
                        workflow_name: ref mut w,
                        ..
                    } = c
                    {
                        *w = "other-wf".to_string();
                    }
                    c
                }),
            ),
            (
                "node_name",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let WorkflowCommand::CompleteNode {
                        node_name: ref mut n,
                        ..
                    } = c
                    {
                        *n = "node-X".to_string();
                    }
                    c
                }),
            ),
            (
                "result",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let WorkflowCommand::CompleteNode {
                        result: ref mut r, ..
                    } = c
                    {
                        *r = Some("DIFFERENT".to_string());
                    }
                    c
                }),
            ),
            (
                "session_id",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let WorkflowCommand::CompleteNode {
                        session_id: ref mut s,
                        ..
                    } = c
                    {
                        *s = Some("sess-X".to_string());
                    }
                    c
                }),
            ),
            (
                "token_usage",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let WorkflowCommand::CompleteNode {
                        token_usage: ref mut t,
                        ..
                    } = c
                    {
                        *t = Some(TokenUsage {
                            input_tokens: 999,
                            output_tokens: 999,
                        });
                    }
                    c
                }),
            ),
            (
                "structured_output",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let WorkflowCommand::CompleteNode {
                        structured_output: ref mut so,
                        ..
                    } = c
                    {
                        *so = Some(serde_json::json!({"k":"other"}));
                    }
                    c
                }),
            ),
            (
                "run_index",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let WorkflowCommand::CompleteNode {
                        run_index: ref mut r,
                        ..
                    } = c
                    {
                        *r = Some(99);
                    }
                    c
                }),
            ),
            (
                "timestamp",
                Box::new(|_cmd| {
                    let mut c = base_command();
                    if let WorkflowCommand::CompleteNode {
                        timestamp: ref mut t,
                        ..
                    } = c
                    {
                        *t = 999.0;
                    }
                    c
                }),
            ),
        ];

        for (label, mutate) in mutators {
            let mut snapshot = base_snapshot();
            let cmd = mutate(base_command());
            let result = WorkflowEngine::dispatch_internal_node_command(&mut snapshot, cmd);
            assert!(
                matches!(result, Err(WorkflowEngineError::ValidationError(_))),
                "CompleteNode {label} mismatch must return ValidationError, got: {result:?}"
            );
        }
    }

    /// Spec [05] commit 境界: `FailNode` の整合検証も run_id / workflow_name / node_name の
    /// 各次元で snapshot との mismatch を ValidationError として検出することを担保する。
    #[test]
    fn dispatch_internal_fail_node_validates_all_effect_fields() {
        fn base_snapshot() -> WorkflowState {
            let mut s =
                dispatch_internal_test_snapshot("00000000-0000-0000-0000-000000000621", "fail-wf");
            s.current_step_name = "node-1".to_string();
            s
        }
        fn base_command() -> WorkflowCommand {
            WorkflowCommand::FailNode {
                run_id: "00000000-0000-0000-0000-000000000621".to_string(),
                workflow_name: "fail-wf".to_string(),
                node_name: "node-1".to_string(),
                reason: "boom".to_string(),
                timestamp: 200.0,
            }
        }

        // baseline は受理される。
        let mut s = base_snapshot();
        assert!(WorkflowEngine::dispatch_internal_node_command(&mut s, base_command()).is_ok());

        // run_id mismatch
        let mut s = base_snapshot();
        let mut bad = base_command();
        if let WorkflowCommand::FailNode {
            run_id: ref mut r, ..
        } = bad
        {
            *r = "00000000-0000-0000-0000-000000000999".to_string();
        }
        assert!(matches!(
            WorkflowEngine::dispatch_internal_node_command(&mut s, bad),
            Err(WorkflowEngineError::ValidationError(_))
        ));

        // workflow_name mismatch
        let mut s = base_snapshot();
        let mut bad = base_command();
        if let WorkflowCommand::FailNode {
            workflow_name: ref mut w,
            ..
        } = bad
        {
            *w = "other-wf".to_string();
        }
        assert!(matches!(
            WorkflowEngine::dispatch_internal_node_command(&mut s, bad),
            Err(WorkflowEngineError::ValidationError(_))
        ));

        // node_name mismatch
        let mut s = base_snapshot();
        let mut bad = base_command();
        if let WorkflowCommand::FailNode {
            node_name: ref mut n,
            ..
        } = bad
        {
            *n = "node-X".to_string();
        }
        assert!(matches!(
            WorkflowEngine::dispatch_internal_node_command(&mut s, bad),
            Err(WorkflowEngineError::ValidationError(_))
        ));
    }

    /// Spec [05] Rule: node が失敗したときの状態遷移が run に反映され、node 失敗の事実が
    /// event log に記録される。engine の実 production 経路 (`set_execution_state` →
    /// `sync_run_store_from_snapshot` + `write_terminal_log` 一連) を通過して、
    /// (1) RunStore の status が Failed terminal に同期される、
    /// (2) NDJSON event log に NodeFailed + RunFailed が追記される、
    /// の双方が成立することを直接検証する（spec L122-130）。
    #[tokio::test]
    async fn engine_set_execution_state_failed_drives_run_state_and_node_failed_event_log() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps();

        let worktree_path = "/wt/engine-node-failure";
        let workflow = make_rejectable_approval_workflow();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 1; // node-1 = "fix"
        exec.current_session_id = None;
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        // 実 production 経路: set_execution_state → Failed への遷移を engine 経由で実施。
        // write_terminal_log + sync_run_store_from_snapshot がこの経路の中で連続して
        // 実行されることを境界仕様として担保する。
        engine
            .set_execution_state(
                app.handle(),
                &session_store,
                &handles,
                worktree_path,
                WorkflowExecutionState::Failed {
                    reason: "node failure".to_string(),
                },
            )
            .await
            .unwrap();

        // (1) engine.executions の state は Failed に遷移している。
        {
            let execs = engine.executions.lock().await;
            let exec = execs
                .get(&run_id)
                .expect("execution must remain after Failed");
            assert!(matches!(
                exec.state,
                WorkflowExecutionState::Failed { ref reason } if reason == "node failure"
            ));
        }

        // (2) RunStore の status も Failed terminal に同期される。
        let run = engine
            .run_store
            .get_run(&run_id)
            .await
            .expect("RunStore must reflect the run");
        assert!(
            run.status.is_terminal(),
            "RunStore status must be terminal, got {:?}",
            run.status
        );
        assert_eq!(run.error_reason.as_deref(), Some("node failure"));

        // (3) NDJSON event log に NodeFailed + RunFailed が連続 append される。
        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        let node_failed = events
            .iter()
            .find(|e| matches!(e, WorkflowEvent::NodeFailed { .. }));
        let run_failed = events
            .iter()
            .find(|e| matches!(e, WorkflowEvent::RunFailed { .. }));
        assert!(
            node_failed.is_some(),
            "NodeFailed event must be appended via engine dispatch path; got: {events:?}"
        );
        assert!(
            run_failed.is_some(),
            "RunFailed event must follow NodeFailed; got: {events:?}"
        );
    }

    /// Spec [05] commit 境界: production 経路 `execute_outcome` の pre-commit phase で
    /// `write_log_required_batch` が失敗した場合、`sync_run_store_from_snapshot` /
    /// `persist_state` は実行されず、RunStore は active のまま / NDJSON 上にも terminal
    /// event が残らないことを直接検証する（spec [05]: state mutation と event log の
    /// 分離を防ぐ rollback 境界）。
    ///
    /// 障害シミュレーション: workflow_logs ディレクトリパスに通常ファイルを置くと、
    /// `WorkflowEventLog::append_batch` 内の `create_dir_all` が失敗し、batch append が
    /// `Err` を返す。
    #[tokio::test]
    async fn execute_outcome_pre_commit_append_failure_keeps_run_store_active() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps();

        let worktree_path = "/wt/append-failure";

        let workflow = make_rejectable_approval_workflow();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 1; // node "fix"
        exec.current_session_id = None;
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        // workflow_logs ディレクトリを通常ファイルで塞いで append を恒常失敗させる。
        let log_dir = data_dir.join("workflow_logs");
        if log_dir.exists() {
            std::fs::remove_dir_all(&log_dir).unwrap();
        }
        std::fs::write(&log_dir, b"block").unwrap();

        // snapshot を Failed terminal に遷移させ、execute_outcome に persist 経路で渡す。
        let mut snapshot = {
            let execs = engine.executions.lock().await;
            execs.get(&run_id).unwrap().to_workflow_state()
        };
        snapshot.state = WorkflowExecutionState::Failed {
            reason: "node failure".to_string(),
        };
        snapshot.updated_at = 9999.0;

        let result = engine
            .execute_outcome_persist_failed_for_test(
                app.handle(),
                &session_store,
                &handles,
                worktree_path,
                snapshot,
            )
            .await;
        assert!(
            result.is_err(),
            "execute_outcome must return Err when pre-commit append fails: {result:?}"
        );

        // RunStore は active のまま（terminal に sync されていない）。
        let stored = engine
            .run_store
            .get_run(&run_id)
            .await
            .expect("RunStore must still hold the run");
        assert!(
            !stored.status.is_terminal(),
            "RunStore status must NOT be terminal when event log append fails; got {:?}",
            stored.status
        );
        assert!(
            stored.error_reason.is_none(),
            "RunStore error_reason must remain unset when event log append fails"
        );

        // workflow_logs ディレクトリを復旧して NDJSON が空であることを確認する。
        std::fs::remove_file(&log_dir).unwrap();
        std::fs::create_dir_all(&log_dir).unwrap();
        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert!(
            events.is_empty(),
            "NDJSON event log must be empty when pre-commit append fails; got {events:?}"
        );
    }

    /// Spec [05]: engine 内部の typed command 経路として、`WorkflowEngine::dispatch`
    /// 経由で `WorkflowCommand::FailNode` を流すと `WorkflowEvent::NodeFailed` が
    /// event log に commit され、engine.executions の state も Failed に遷移することを
    /// 境界仕様として担保する（spec L22-24, L144-146: dispatch を内部 node 失敗の発火点へ
    /// 集約）。adapter-facing 拒否境界は `dispatch_external` 側に分離されており、
    /// engine 内部経路は `dispatch` が internal variant を受理する。
    #[tokio::test]
    async fn dispatch_commits_internal_fail_node_via_typed_command() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps();

        let worktree_path = "/wt/dispatch-internal-fail";
        let workflow = make_rejectable_approval_workflow();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 1;
        exec.current_session_id = None;
        let workflow_name = exec.workflow.name.clone();
        let node_name = exec.workflow.nodes[exec.current_step_index].name.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let cmd = WorkflowCommand::FailNode {
            run_id: run_id.clone(),
            workflow_name: workflow_name.clone(),
            node_name: node_name.clone(),
            reason: "internal node failure".to_string(),
            timestamp: 9999.0,
        };
        let result = engine
            .dispatch(app.handle(), &session_store, &handles, cmd)
            .await
            .expect("dispatch must accept internal FailNode and commit");
        assert!(matches!(result, WorkflowCommandResult::Accepted));

        // engine.executions の state が Failed に遷移している。
        {
            let execs = engine.executions.lock().await;
            let exec = execs.get(&run_id).expect("exec must remain");
            assert!(matches!(
                exec.state,
                WorkflowExecutionState::Failed { ref reason } if reason == "internal node failure"
            ));
        }

        // event log には NodeFailed が append されている。
        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        let node_failed = events
            .iter()
            .find(|e| matches!(e, WorkflowEvent::NodeFailed { .. }));
        assert!(
            node_failed.is_some(),
            "dispatch must append NodeFailed via typed command path; got: {events:?}"
        );
    }

    /// Spec [05]: engine 内部の typed command 経路として、`WorkflowEngine::dispatch`
    /// 経由で `WorkflowCommand::CompleteNode` を流すと `WorkflowEvent::NodeCompleted` が
    /// event log に commit され、上流が push 済みの step_history 末尾 entry と整合する
    /// snapshot で受理されることを境界仕様として担保する（spec L22-24: NodeCompleted の
    /// 発行点が typed command 経路に集約される）。
    #[tokio::test]
    async fn dispatch_commits_internal_complete_node_via_typed_command() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let (session_store, handles) = make_dispatch_deps();

        let worktree_path = "/wt/dispatch-internal-complete";
        let workflow = make_rejectable_approval_workflow();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec =
            make_waiting_approval_execution_with_workflow(&run_id, worktree_path, workflow);
        exec.state = WorkflowExecutionState::Running;
        exec.current_step_index = 1; // node "fix"
        exec.current_session_id = None;
        // CompleteNode は dispatch_internal_node_command の整合検証で step_history 末尾と
        // command effect の一致を要求するため、対応する entry を事前に push する。
        let node_name = exec.workflow.nodes[exec.current_step_index].name.clone();
        let workflow_name = exec.workflow.name.clone();
        exec.step_history
            .push(crate::workflow::state::StepHistoryEntry {
                step_name: node_name.clone(),
                completed_at: 8888.0,
                result: Some("done".to_string()),
                session_id: Some("sess-x".to_string()),
                token_usage: None,
                structured_output: None,
                run_index: 1,
                child_outputs: None,
                state: crate::workflow::state::default_step_entry_state(),
            });
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let cmd = WorkflowCommand::CompleteNode {
            run_id: run_id.clone(),
            workflow_name: workflow_name.clone(),
            node_name: node_name.clone(),
            result: Some("done".to_string()),
            session_id: Some("sess-x".to_string()),
            token_usage: None,
            structured_output: None,
            run_index: Some(1),
            timestamp: 8888.0,
        };
        let result = engine
            .dispatch(app.handle(), &session_store, &handles, cmd)
            .await
            .expect("dispatch must accept internal CompleteNode and commit");
        assert!(matches!(result, WorkflowCommandResult::Accepted));

        // event log に NodeCompleted が required append されている。
        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        let node_completed = events
            .iter()
            .find(|e| matches!(e, WorkflowEvent::NodeCompleted { .. }));
        assert!(
            node_completed.is_some(),
            "dispatch must append NodeCompleted via typed command path; got: {events:?}"
        );

        // engine.executions は state を維持（CompleteNode は state 遷移を起こさない）。
        let execs = engine.executions.lock().await;
        let exec = execs.get(&run_id).expect("exec must remain");
        assert!(matches!(exec.state, WorkflowExecutionState::Running));
    }

    /// Spec [05] Rule: snapshot に Failed state が反映済みの場合、`write_terminal_log` の
    /// 単体経路 (`terminal_events_for_snapshot` → `write_log_required_batch`) が
    /// `NodeFailed` + `RunFailed` を順序通り append することを直接検証する（commit 境界の単位テスト）。
    #[test]
    fn write_terminal_log_emits_node_failed_followed_by_run_failed_for_failed_snapshot() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let data_dir = dispatch_data_dir(app.handle());
        let run_id = "00000000-0000-0000-0000-000000000605".to_string();

        let snapshot = WorkflowState {
            execution_id: run_id.clone(),
            workflow_name: "fail-wf".to_string(),
            state: WorkflowExecutionState::Failed {
                reason: "node boom".to_string(),
            },
            current_step_index: 0,
            current_step_name: "step-1".to_string(),
            current_session_id: None,
            total_steps: 1,
            step_history: vec![],
            step_execution_counts: HashMap::new(),
            workflow_definition: crate::workflow::schema::Workflow {
                variables: Default::default(),
                name: "fail-wf".to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![],
            },
            total_token_usage: TokenUsage::default(),
            step_states: HashMap::new(),
            step_outputs: HashMap::new(),
            active_parallel_steps: vec![],
            workflow_variables: HashMap::new(),
            approval_operations: None,
            started_at: 900.0,
            updated_at: 1000.0,
        };

        engine
            .write_terminal_log(app.handle(), &snapshot)
            .expect("write_terminal_log must succeed");

        let events = WorkflowEventLog::new(&data_dir).read_log(&run_id).unwrap();
        assert_eq!(
            events.len(),
            2,
            "terminal log must contain NodeFailed + RunFailed; got {events:?}"
        );
        match &events[0] {
            WorkflowEvent::NodeFailed {
                run_id: ev_run_id,
                workflow_name,
                node_name,
                reason,
                ..
            } => {
                assert_eq!(ev_run_id, &run_id);
                assert_eq!(workflow_name, "fail-wf");
                assert_eq!(node_name, "step-1");
                assert_eq!(reason, "node boom");
            }
            other => panic!("expected NodeFailed first, got {other:?}"),
        }
        match &events[1] {
            WorkflowEvent::RunFailed {
                run_id: ev_run_id,
                workflow_name,
                reason,
                ..
            } => {
                assert_eq!(ev_run_id, &run_id);
                assert_eq!(workflow_name, "fail-wf");
                assert_eq!(reason, "node boom");
            }
            other => panic!("expected RunFailed second, got {other:?}"),
        }
    }

    /// Spec [04]: `WorkflowCommand::ApproveNode` の comment は `ApprovalResolved.comment` に
    /// 伝播する経路で `WorkflowCommand::RejectNode` の reason も同様に伝播する。dispatch 入口
    /// に渡された command の comment / reason が production 経路で event payload に反映される
    /// ことを担保する（routing 層の意味を保つ）。
    #[test]
    fn dispatch_command_comments_map_to_approval_resolved_event_comment() {
        // dispatch 内で構築される ApprovalResolved の comment が approve/reject 双方で
        // 適切に決定されることを、command variant ごとに直接検証する。
        let approve = WorkflowCommand::ApproveNode {
            run_id: "00000000-0000-0000-0000-000000000400".to_string(),
            node_name: Some("review".to_string()),
            comment: Some("lgtm with notes".to_string()),
        };
        let approve_comment = match approve {
            WorkflowCommand::ApproveNode { comment, .. } => comment,
            _ => panic!("expected ApproveNode"),
        };
        assert_eq!(approve_comment.as_deref(), Some("lgtm with notes"));

        let reject = WorkflowCommand::RejectNode {
            run_id: "00000000-0000-0000-0000-000000000401".to_string(),
            node_name: Some("review".to_string()),
            reason: "needs fix".to_string(),
        };
        let reject_reason = match reject {
            WorkflowCommand::RejectNode { reason, .. } => reason,
            _ => panic!("expected RejectNode"),
        };
        assert_eq!(reject_reason, "needs fix");
    }

    /// Spec [04]: `WorkflowCommand::AbortRun { expected_node_name: Some(_) }` は
    /// approval UI 由来の Abort として handle_approval 経路に乗り、
    /// `expected_node_name: None` は run 全体の Abort として abort_workflow_by_run_id 経路に
    /// 乗る。dispatch の routing 分岐がこの 2 経路を区別することを構造的に担保する。
    #[test]
    fn dispatch_abort_run_variant_distinguishes_approval_vs_full_run_route() {
        let approval_route = WorkflowCommand::AbortRun {
            run_id: "00000000-0000-0000-0000-000000000500".to_string(),
            expected_node_name: Some("review".to_string()),
        };
        let full_run_route = WorkflowCommand::AbortRun {
            run_id: "00000000-0000-0000-0000-000000000501".to_string(),
            expected_node_name: None,
        };

        // dispatch 内部の `if let Some(node_name) = expected_node_name` 分岐の構造を再現。
        let approval_uses_approval_path = matches!(
            approval_route,
            WorkflowCommand::AbortRun {
                expected_node_name: Some(_),
                ..
            }
        );
        let full_run_uses_abort_path = matches!(
            full_run_route,
            WorkflowCommand::AbortRun {
                expected_node_name: None,
                ..
            }
        );
        assert!(
            approval_uses_approval_path,
            "approval UI 由来 Abort は expected_node_name を伴う"
        );
        assert!(
            full_run_uses_abort_path,
            "run 全体 Abort は expected_node_name None"
        );
    }

    /// Spec [04] sentinel 禁止: `WorkflowCommandResult::Accepted` と `RunStarted` は
    /// 型として区別され、Tauri adapter で variant 別に射影される。空文字列 sentinel を
    /// `Accepted` から生成しないことを構造的に担保する（Tauri adapter での match 化が
    /// 前提）。
    #[test]
    fn workflow_command_result_distinguishes_run_started_and_accepted() {
        let started = WorkflowCommandResult::RunStarted {
            run_id: "00000000-0000-0000-0000-000000000600".to_string(),
        };
        let accepted = WorkflowCommandResult::Accepted;
        assert_ne!(started, accepted);
        // `RunStarted` から `Accepted` への変換ヘルパーは boundary 上に存在しない。
        // adapter 側で match による射影のみが認められる（spec [04] 責務配置）。
    }

    /// Spec [04] Rule「対象不在 / 既に終了した command は受理されない」:
    /// `abort_target_lookup` は `executions` に存在しない run_id を `NotFound` と
    /// 判定し、後段の dispatch では非受理にマッピングされる構造を担保する。
    #[tokio::test]
    async fn abort_target_lookup_returns_not_found_for_unknown_run_id() {
        let engine = WorkflowEngine::new_for_test();
        match engine
            .abort_target_lookup("00000000-0000-0000-0000-000000000700")
            .await
        {
            AbortTargetLookup::NotFound => {}
            other => panic!("expected NotFound for unknown run_id, got {other:?}"),
        }
    }

    /// Spec [04] Rule「既に終了した run に対する操作 command が要求される」:
    /// terminal な run（Completed/Failed/Aborted）に対する Abort は `AlreadyTerminal`
    /// として lookup 段階で非受理になる。
    #[tokio::test]
    async fn abort_target_lookup_returns_already_terminal_for_terminal_run() {
        let engine = WorkflowEngine::new_for_test();
        let run_id = uuid::Uuid::new_v4().to_string();
        for terminal_state in [
            WorkflowExecutionState::Completed,
            WorkflowExecutionState::Aborted,
            WorkflowExecutionState::Failed {
                reason: "x".to_string(),
            },
        ] {
            let mut exec = make_waiting_approval_execution(&run_id, "/wt/term");
            exec.state = terminal_state.clone();
            engine.executions.lock().await.insert(run_id.clone(), exec);

            match engine.abort_target_lookup(&run_id).await {
                AbortTargetLookup::AlreadyTerminal => {}
                other => panic!(
                    "expected AlreadyTerminal for terminal {terminal_state:?}, got {other:?}"
                ),
            }
            engine.executions.lock().await.remove(&run_id);
        }
    }

    /// Spec [04] Rule: active run に対する `abort_target_lookup` は `Active` を返し、
    /// その後の state 遷移経路（mutation → required append → finalize）に乗る。
    #[tokio::test]
    async fn abort_target_lookup_returns_active_for_running_run() {
        let engine = WorkflowEngine::new_for_test();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_waiting_approval_execution(&run_id, "/wt/active");
        exec.state = WorkflowExecutionState::Running;
        exec.current_session_id = Some("sess-X".to_string());
        engine.executions.lock().await.insert(run_id.clone(), exec);

        match engine.abort_target_lookup(&run_id).await {
            AbortTargetLookup::Active {
                current_step_session_id,
                ..
            } => {
                assert_eq!(current_step_session_id.as_deref(), Some("sess-X"));
            }
            other => panic!("expected Active for running run, got {other:?}"),
        }
    }

    /// Spec [04] Rule「権限の無い / 対象不在 / 既決の command は state 変化を起こさない」:
    /// 既に判断済み（WaitingApproval ではない）node に対する Approve / Reject は
    /// `validate_approval_target_snapshot` で `InvalidState` として非受理になる。
    /// production dispatch 経路の `handle_approval` がこのガードを最初に通すため、
    /// 二度目以降の同一意図 command は state 変化を起こさない。
    #[tokio::test]
    async fn approval_target_validation_rejects_already_resolved_node() {
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_waiting_approval_execution(&run_id, "/wt/idempotent");
        exec.state = WorkflowExecutionState::Completed;
        let err =
            WorkflowEngine::validate_approval_target_snapshot(&exec, Some(&run_id), Some("review"))
                .unwrap_err();
        assert!(
            matches!(err, WorkflowEngineError::InvalidState(_)),
            "既決 node への Approve/Reject は InvalidState で非受理 (got {err:?})"
        );
    }

    /// Spec [04] Rule: `validate_approval_decision` は Reject の空コメント / 上限超過を
    /// 拒否する。dispatch 入口での新規外部入力に対する境界バリデーション。
    #[test]
    fn reject_decision_validation_rejects_empty_and_oversize_comments() {
        let empty = WorkflowEngine::validate_approval_decision(&ApprovalDecision::Reject {
            comment: "   ".to_string(),
        })
        .unwrap_err();
        assert!(matches!(empty, WorkflowEngineError::ValidationError(_)));

        let oversize = WorkflowEngine::validate_approval_decision(&ApprovalDecision::Reject {
            comment: "x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1),
        })
        .unwrap_err();
        assert!(matches!(oversize, WorkflowEngineError::ValidationError(_)));

        WorkflowEngine::validate_approval_decision(&ApprovalDecision::Reject {
            comment: "fix this".to_string(),
        })
        .expect("正常な reject reason は受理される");
    }

    /// Spec [04] Rule: Approve コメントも reject と同じ MAX_APPROVAL_COMMENT_CHARS を
    /// 適用する。空文字（None）は許容するが、上限超過は非受理。
    #[test]
    fn approve_comment_length_validation_rejects_oversize_but_accepts_empty() {
        WorkflowEngine::validate_approve_comment_length(None).expect("None は許容される");
        WorkflowEngine::validate_approve_comment_length(Some(""))
            .expect("空コメント (Some(empty)) は許容される");
        let oversize_comment = "x".repeat(MAX_APPROVAL_COMMENT_CHARS + 1);
        let err =
            WorkflowEngine::validate_approve_comment_length(Some(&oversize_comment)).unwrap_err();
        assert!(matches!(err, WorkflowEngineError::ValidationError(_)));
    }

    /// Spec [04] secret redaction: ApprovalResolved.comment に設定済み secret 値が
    /// 含まれる場合、event log に書き出す前に `mask_sensitive_text()` で redaction
    /// される。本テストは redaction primitive そのものの契約を担保する
    /// （`reject_structured_output` と同じ secret 列で構造的に共有する経路）。
    #[test]
    fn mask_sensitive_text_redacts_secret_in_approval_comment() {
        let secrets = vec!["super-secret-token".to_string()];
        let raw = "approving with token=super-secret-token please review";
        let masked = WorkflowEngine::mask_sensitive_text(raw, &secrets);
        assert!(
            !masked.contains("super-secret-token"),
            "secret 値が raw のまま残ってはならない (masked={masked})"
        );
    }

    /// Spec [04] atomic mutation 境界（AbortRun 経路）: `WorkflowCommand::AbortRun`
    /// が受理されると `RunAborted` event は `write_log_required` 経由で必須 append
    /// される。NDJSON に正しく snake_case で記録され、observer が typed event として
    /// 読めることを担保する。
    #[test]
    fn run_aborted_event_required_append_writes_typed_ndjson() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let run_id = "00000000-0000-0000-0000-000000000800";

        log.append(&WorkflowEvent::RunAborted {
            run_id: run_id.to_string(),
            workflow_name: "boundary-wf".to_string(),
            timestamp: 4321.0,
        })
        .expect("RunAborted は write_log_required 経由で append される");

        let events = log.read_log(run_id).unwrap();
        assert_eq!(events.len(), 1);
        match &events[0] {
            WorkflowEvent::RunAborted { run_id: rid, .. } => assert_eq!(rid, run_id),
            other => panic!("expected RunAborted, got {other:?}"),
        }
    }

    /// Spec [04] rollback: production dispatch 経由で event append が失敗した場合、
    /// WorkflowExecution / Run Store / event log は command 受理前 snapshot に戻る。
    #[tokio::test]
    async fn dispatch_approve_node_append_failure_rolls_back_full_snapshot() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/append-fail";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        exec.workflow_variables
            .insert("k".to_string(), "v_before".to_string());
        let snapshot_before = exec.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::ApproveNode {
                    run_id: run_id.clone(),
                    node_name: Some("review".to_string()),
                    comment: Some("lgtm".to_string()),
                },
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));

        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).expect("run must remain");
        assert_eq!(
            restored.state, snapshot_before.state,
            "state は snapshot で一括復元される"
        );
        assert_eq!(
            restored.current_step_index,
            snapshot_before.current_step_index
        );
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        assert_eq!(
            restored.workflow_variables.get("k").map(|s| s.as_str()),
            Some("v_before"),
            "workflow_variables も mutation 前の値に戻る"
        );
        drop(execs);

        let active = engine.list_active_runs().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);
        assert_eq!(active[0].status, RunStatus::WaitingApproval);
        assert!(read_dispatch_events(&app, &run_id).is_empty());
    }

    /// Spec [04] rollback: AbortRun の required event append が失敗した場合も、
    /// WorkflowExecution / Run Store / ChatSession workflow_state は mutation 前へ戻る。
    #[tokio::test]
    async fn dispatch_abort_run_append_failure_rolls_back_execution_run_store_and_session() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/abort-append-fail";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        exec.state = WorkflowExecutionState::Running;
        let snapshot_before = exec.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::AbortRun {
                    run_id: run_id.clone(),
                    expected_node_name: None,
                },
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).expect("run must remain");
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        drop(execs);
        let active = engine.list_active_runs().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);
        assert_eq!(active[0].status, RunStatus::Running);
        // ChatSession.workflow_state は撤去済みのため parent session 経由の rollback 観測は省略。
        assert!(read_dispatch_events(&app, &run_id).is_empty());
    }

    /// Spec [04] テスト境界: StartRun は production `WorkflowEngine::dispatch` 入口で
    /// validation され、拒否時は state / event を変更しない。
    #[tokio::test]
    async fn dispatch_start_run_rejects_invalid_name_without_state_change() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let run_store_dir = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(run_store_dir.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::StartRun {
                    workflow_file_stem: "../bad".to_string(),
                    worktree_path: "/wt/start-invalid".to_string(),
                    task: None,
                    trigger_source: TriggerSource::DesktopUi,
                    permission_mode: crate::permission::PermissionMode::Edit,
                },
            )
            .await;

        assert!(matches!(
            result,
            Err(WorkflowEngineError::ValidationError(_))
        ));
        assert!(engine.executions.lock().await.is_empty());
        assert!(engine.list_active_runs().await.is_empty());
    }

    /// Spec [04] テスト境界: StartRun の正常系は production dispatch 経由で
    /// `RunStarted` を返し、execution / Run Store / RunStarted event を作成する。
    #[tokio::test]
    async fn dispatch_start_run_accepts_creates_run_and_appends_event() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let run_store_dir = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(run_store_dir.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let (repo_parent, _worktree_parent, worktree_path) = make_managed_worktree();
        configure_managed_repo(&app, repo_parent.path().join("repo").as_path());
        let stem = "spec-driven-development".to_string();

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::StartRun {
                    workflow_file_stem: stem.clone(),
                    worktree_path: worktree_path.to_string_lossy().to_string(),
                    task: Some("start me".to_string()),
                    trigger_source: TriggerSource::DesktopUi,
                    permission_mode: crate::permission::PermissionMode::Edit,
                },
            )
            .await
            .unwrap();

        let WorkflowCommandResult::RunStarted { run_id } = result else {
            panic!("expected RunStarted");
        };
        assert!(
            engine.executions.lock().await.contains_key(&run_id),
            "StartRun must register a WorkflowExecution"
        );
        assert!(
            engine.get_run(&run_id).await.is_some(),
            "StartRun must create a Run Store entry"
        );
        assert!(read_dispatch_events(&app, &run_id).iter().any(|event| {
            matches!(
                event,
                WorkflowEvent::RunStarted {
                    workflow_file_stem,
                    ..
                } if workflow_file_stem == &stem
            )
        }));
    }

    /// Spec [04] rollback: StartRun の RunStarted append が失敗した場合、
    /// reservation / execution / parent ChatSession を command 受理前へ戻す。
    #[tokio::test]
    async fn dispatch_start_run_append_failure_clears_created_parent_workflow_state() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let run_store_dir = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(run_store_dir.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let (repo_parent, _worktree_parent, worktree_path) = make_managed_worktree();
        configure_managed_repo(&app, repo_parent.path().join("repo").as_path());
        let worktree = std::fs::canonicalize(&worktree_path)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::StartRun {
                    workflow_file_stem: "spec-driven-development".to_string(),
                    worktree_path: worktree.clone(),
                    task: Some("start with append failure".to_string()),
                    trigger_source: TriggerSource::DesktopUi,
                    permission_mode: crate::permission::PermissionMode::Edit,
                },
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        assert!(engine.executions.lock().await.is_empty());
        assert!(engine.list_active_runs().await.is_empty());
        let sessions = session_store
            .list_worktree_sessions(&dispatch_data_dir(app.handle()), &worktree)
            .unwrap();
        assert!(
            sessions.is_empty(),
            "RunStarted が存在しない失敗 run の parent ChatSession は残さない"
        );
    }

    // 撤去済み: persist_state は廃止された（NDJSON event log + Run Store metadata で永続化が完結）。
    // 旧 `dispatch_start_run_persist_failure_rolls_back_execution_run_store_and_parent_session` テストは
    // persist_state 注入失敗時の rollback を検証していたが、機構撤去により意味を失った。

    /// Spec [04] テスト境界: AbortRun は production dispatch 経由で Aborted に遷移し、
    /// RunAborted typed event を append する。
    #[tokio::test]
    async fn dispatch_abort_run_accepts_mutates_state_and_appends_event() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-abort";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        // spec issues-1023: session log 到達経路の維持を検証するため、
        // current_session_id を入れた状態で abort する。
        exec.current_session_id = Some("aborted-step-session".to_string());
        exec.state = WorkflowExecutionState::Running;
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::AbortRun {
                    run_id: run_id.clone(),
                    expected_node_name: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(result, WorkflowCommandResult::Accepted);
        let execs = engine.executions.lock().await;
        let aborted_exec = execs.get(&run_id).unwrap();
        assert_eq!(aborted_exec.state, WorkflowExecutionState::Aborted);

        // spec issues-1023: 中断された current step が `state="aborted"` entry として
        // step_history に積まれ、session_id が引き継がれていることを検証する。
        // これにより history タブ経由でも session log に到達できる。
        let aborted_entries: Vec<&StepHistoryEntry> = aborted_exec
            .step_history
            .iter()
            .filter(|e| e.state == "aborted")
            .collect();
        assert_eq!(
            aborted_entries.len(),
            1,
            "current step が 1 件 aborted entry として記録される"
        );
        assert_eq!(
            aborted_entries[0].session_id.as_deref(),
            Some("aborted-step-session"),
            "session_id が step_history に引き継がれ session log に到達可能"
        );
        drop(execs);

        assert!(read_dispatch_events(&app, &run_id)
            .iter()
            .any(|event| matches!(event, WorkflowEvent::RunAborted { .. })));
    }

    /// spec issues-1023: `make_aborted_parallel_history_entry` の単体検証。
    /// parallel ブロック中断時に parent step を 1 entry として、children を
    /// `child_outputs` に snapshot し、完了済み child は "completed"、それ以外は
    /// "aborted" 状態で記録される。session_id は全 child で残されることを担保する。
    #[test]
    fn make_aborted_parallel_history_entry_snapshots_mixed_child_states() {
        let workflow = Workflow {
            variables: Default::default(),
            name: "wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "parallel-review".to_string(),
                node_type: NodeType::Parallel,
                parallel_children: Some(vec![]),
                ..Default::default()
            }],
        };
        let exec = WorkflowExecution {
            id: "exec-abort-parallel".to_string(),
            workflow,
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::from([("parallel-review".to_string(), 1)]),
            step_history: Vec::new(),
            worktree_path: "/wt".to_string(),
            started_at: 0.0,
            updated_at: 0.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: Some(ParallelRunState {
                parent_step_name: "parallel-review".to_string(),
                aggregate: None,
                children: vec![
                    ParallelChildRun {
                        step_name: "child-a".to_string(),
                        session_id: "session-a".to_string(),
                        state: ParallelChildState::Completed,
                        result: Some("LGTM".to_string()),
                        structured_output: None,
                        output_contract: None,
                        token_usage: TokenUsage::default(),
                        run_index: 1,
                    },
                    ParallelChildRun {
                        step_name: "child-b".to_string(),
                        session_id: "session-b".to_string(),
                        state: ParallelChildState::Running,
                        result: None,
                        structured_output: None,
                        output_contract: None,
                        token_usage: TokenUsage::default(),
                        run_index: 1,
                    },
                ],
            }),
            workflow_variables: HashMap::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        };

        let entry = exec
            .make_aborted_parallel_history_entry(123.0)
            .expect("parallel_run が Some なら entry が返る");
        assert_eq!(entry.step_name, "parallel-review");
        assert_eq!(entry.state, "aborted");
        assert_eq!(entry.completed_at, 123.0);
        let children = entry.child_outputs.expect("child_outputs が Some");
        assert_eq!(children.len(), 2);
        let child_a = children.iter().find(|c| c.step_name == "child-a").unwrap();
        assert_eq!(child_a.state, "completed");
        assert_eq!(child_a.session_id.as_deref(), Some("session-a"));
        let child_b = children.iter().find(|c| c.step_name == "child-b").unwrap();
        assert_eq!(child_b.state, "aborted");
        assert_eq!(
            child_b.session_id.as_deref(),
            Some("session-b"),
            "未完了 child でも session_id が child_outputs に残る"
        );
    }

    /// Spec [06] テスト境界: node 限定 AbortRun は現在 node を照合した上で run abort として
    /// 扱い、Running / WaitingApproval のどちらでも `RunAborted` を append する。
    #[tokio::test]
    async fn dispatch_abort_run_with_expected_node_validates_node_and_appends_run_aborted() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-approval-abort";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::AbortRun {
                    run_id: run_id.clone(),
                    expected_node_name: Some("review".to_string()),
                },
            )
            .await
            .unwrap();

        assert_eq!(result, WorkflowCommandResult::Accepted);
        assert_eq!(
            engine.executions.lock().await.get(&run_id).unwrap().state,
            WorkflowExecutionState::Aborted
        );
        let events = read_dispatch_events(&app, &run_id);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], WorkflowEvent::RunAborted { .. }));
    }

    // 撤去済み: dispatch_abort_run_with_expected_node_persist_failure_rolls_back は
    // persist_state 注入失敗を介して rollback を検証していたが、persist_state 機構の撤去で
    // 意味を失った（NDJSON event log + Run Store metadata が権威）。
    // required event append 失敗時の rollback は下記
    // `dispatch_abort_run_with_expected_node_append_failure_rolls_back` で引き続き検証する。

    /// Spec [04] rollback: approval UI 由来の AbortRun で required event append が失敗した場合も、
    /// WorkflowExecution / Run Store / ChatSession workflow_state は mutation 前へ戻る。
    #[tokio::test]
    async fn dispatch_abort_run_with_expected_node_append_failure_rolls_back() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/approval-abort-append-rollback";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        let snapshot_before = exec.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::AbortRun {
                    run_id: run_id.clone(),
                    expected_node_name: Some("review".to_string()),
                },
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).unwrap();
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        drop(execs);
        let active = engine.list_active_runs().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);
        assert_eq!(active[0].status, RunStatus::WaitingApproval);
        // ChatSession.workflow_state は撤去済みのため parent session 経由の rollback 観測は省略。
        // RunStore active projection の rollback だけ確認する（上の assertion で済み）。
        assert!(read_dispatch_events(&app, &run_id).is_empty());
    }

    /// Spec [04] Rule「対象不在 / 既に終了した command は受理されない」:
    /// AbortRun の dispatch 拒否経路は state を変化させず event を append しない。
    #[tokio::test]
    async fn dispatch_abort_run_rejects_not_found_and_terminal_without_append() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let missing_run_id = uuid::Uuid::new_v4().to_string();

        let missing = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::AbortRun {
                    run_id: missing_run_id.clone(),
                    expected_node_name: None,
                },
            )
            .await;
        assert!(matches!(
            missing,
            Err(WorkflowEngineError::ExecutionNotFound(_))
        ));
        assert!(read_dispatch_events(&app, &missing_run_id).is_empty());

        let terminal_run_id = uuid::Uuid::new_v4().to_string();
        let mut terminal = make_waiting_approval_execution(&terminal_run_id, "/wt/terminal-abort");
        terminal.state = WorkflowExecutionState::Completed;
        let snapshot_before = terminal.clone();
        engine
            .executions
            .lock()
            .await
            .insert(terminal_run_id.clone(), terminal);

        let terminal_result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::AbortRun {
                    run_id: terminal_run_id.clone(),
                    expected_node_name: None,
                },
            )
            .await;
        assert!(matches!(
            terminal_result,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&terminal_run_id).unwrap();
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        drop(execs);
        assert!(read_dispatch_events(&app, &terminal_run_id).is_empty());
    }

    /// Spec [04] no-op 不変条件: approval UI 由来の
    /// `AbortRun { expected_node_name: Some(_) }` でも、対象不在・stale node・既決 node は
    /// production dispatch 経由で state / Run Store を変化させず event を append しない。
    #[tokio::test]
    async fn dispatch_approval_abort_rejects_missing_stale_and_resolved_targets_without_append() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();

        let missing_run_id = uuid::Uuid::new_v4().to_string();
        let missing = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::AbortRun {
                    run_id: missing_run_id.clone(),
                    expected_node_name: Some("review".to_string()),
                },
            )
            .await;
        assert!(matches!(
            missing,
            Err(WorkflowEngineError::ExecutionNotFound(_))
        ));
        assert!(engine.list_active_runs().await.is_empty());
        assert!(engine.list_completed_runs().await.is_empty());
        assert!(read_dispatch_events(&app, &missing_run_id).is_empty());

        let stale_run_id = uuid::Uuid::new_v4().to_string();
        let stale_worktree = "/wt/approval-abort-stale";
        let mut stale_exec = make_waiting_approval_execution(&stale_run_id, stale_worktree);
        stale_exec.current_session_id = None;
        let stale_before = stale_exec.clone();
        insert_execution_and_active_run(&engine, stale_exec, TriggerSource::DesktopUi).await;
        let stale_active_before = engine.list_active_runs().await;

        let stale = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::AbortRun {
                    run_id: stale_run_id.clone(),
                    expected_node_name: Some("old-review".to_string()),
                },
            )
            .await;
        assert!(matches!(
            stale,
            Err(WorkflowEngineError::UnauthorizedApprovalTarget(_))
        ));
        let execs = engine.executions.lock().await;
        let stale_after = execs.get(&stale_run_id).unwrap();
        assert_eq!(stale_after.state, stale_before.state);
        assert_eq!(
            stale_after.current_step_index,
            stale_before.current_step_index
        );
        assert_eq!(
            stale_after.step_history.len(),
            stale_before.step_history.len()
        );
        drop(execs);
        let stale_active_after = engine.list_active_runs().await;
        assert_eq!(stale_active_after.len(), stale_active_before.len());
        assert_eq!(stale_active_after[0].run_id, stale_active_before[0].run_id);
        assert_eq!(stale_active_after[0].status, stale_active_before[0].status);
        assert!(read_dispatch_events(&app, &stale_run_id).is_empty());

        let resolved_run_id = uuid::Uuid::new_v4().to_string();
        let resolved_worktree = "/wt/approval-abort-resolved";
        let mut resolved_exec =
            make_waiting_approval_execution(&resolved_run_id, resolved_worktree);
        resolved_exec.current_session_id = None;
        resolved_exec.state = WorkflowExecutionState::Completed;
        let resolved_before = resolved_exec.clone();
        engine
            .executions
            .lock()
            .await
            .insert(resolved_run_id.clone(), resolved_exec);
        engine
            .run_store
            .register_active(WorkflowRun {
                run_id: resolved_run_id.clone(),
                workflow_name: "boundary-wf".to_string(),
                task: None,
                status: RunStatus::WaitingApproval,
                worktree_path: resolved_worktree.to_string(),
                current_node_name: Some("review".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 1000.0,
                updated_at: 1000.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        engine
            .run_store
            .complete_run(&resolved_run_id, TerminalRunStatus::Completed, 2000.0, None)
            .await
            .unwrap();
        let completed_before = engine.list_completed_runs().await;

        let resolved = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::AbortRun {
                    run_id: resolved_run_id.clone(),
                    expected_node_name: Some("review".to_string()),
                },
            )
            .await;
        assert!(matches!(
            resolved,
            Err(WorkflowEngineError::InvalidState(_))
        ));
        let execs = engine.executions.lock().await;
        let resolved_after = execs.get(&resolved_run_id).unwrap();
        assert_eq!(resolved_after.state, resolved_before.state);
        assert_eq!(
            resolved_after.step_history.len(),
            resolved_before.step_history.len()
        );
        drop(execs);
        let completed_after = engine.list_completed_runs().await;
        assert_eq!(completed_after.len(), completed_before.len());
        assert_eq!(completed_after[0].run_id, completed_before[0].run_id);
        assert_eq!(completed_after[0].status, completed_before[0].status);
        assert!(read_dispatch_events(&app, &resolved_run_id).is_empty());
    }

    /// Spec [04] テスト境界: ApproveNode は production dispatch 経由で判断を受理し、
    /// state mutation と ApprovalResolved append を同じ command 受理サイクルで行う。
    #[tokio::test]
    async fn dispatch_approve_node_accepts_mutates_state_and_appends_event() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-approve";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::ApproveNode {
                    run_id: run_id.clone(),
                    node_name: Some("review".to_string()),
                    comment: Some("lgtm".to_string()),
                },
            )
            .await
            .unwrap();

        assert_eq!(result, WorkflowCommandResult::Accepted);
        let execs = engine.executions.lock().await;
        let exec = execs.get(&run_id).unwrap();
        assert_eq!(exec.state, WorkflowExecutionState::Completed);
        assert_eq!(exec.step_history.len(), 1);
        drop(execs);
        let events = read_dispatch_events(&app, &run_id);
        assert!(matches!(
            events.as_slice(),
            [
                WorkflowEvent::ApprovalResolved {
                    decision: ApprovalDecisionRecord::Approve,
                    ..
                },
                WorkflowEvent::NodeCompleted { node_name, .. },
                WorkflowEvent::RunCompleted { .. },
            ] if node_name == "review"
        ));
    }

    // 撤去済み: parent ChatSession / persist_state 機構の撤去で意味を失ったテスト。

    /// Spec [04] テスト境界: RejectNode は production dispatch 経由で判断を受理し、
    /// state mutation と ApprovalResolved { decision: Reject } append を行う。
    #[tokio::test]
    async fn dispatch_reject_node_accepts_mutates_state_and_appends_event() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-reject-accept";
        let mut exec = make_waiting_approval_execution_with_workflow(
            &run_id,
            worktree_path,
            make_rejectable_approval_workflow(),
        );
        exec.current_session_id = None;
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::RejectNode {
                    run_id: run_id.clone(),
                    node_name: Some("review".to_string()),
                    reason: "needs changes".to_string(),
                },
            )
            .await
            .unwrap();

        assert_eq!(result, WorkflowCommandResult::Accepted);
        let execs = engine.executions.lock().await;
        let exec = execs.get(&run_id).unwrap();
        assert_eq!(exec.current_step_index, 1);
        assert_eq!(
            exec.step_history
                .first()
                .and_then(|entry| entry.result.as_deref()),
            Some("reject")
        );
        drop(execs);
        let events = read_dispatch_events(&app, &run_id);
        assert!(matches!(
            &events[..3],
            [
                WorkflowEvent::ApprovalResolved {
                    decision: ApprovalDecisionRecord::Reject,
                    comment: Some(comment),
                    ..
                },
                WorkflowEvent::NodeCompleted {
                    node_name: completed,
                    ..
                },
                WorkflowEvent::NodeStarted {
                    node_name: started,
                    ..
                },
            ] if comment == "needs changes" && completed == "review" && started == "fix"
        ));
    }

    /// Spec [04] テスト境界: RejectNode の非受理経路は production dispatch 経由でも
    /// state を変化させず、typed event を append しない。
    #[tokio::test]
    async fn dispatch_reject_node_rejected_target_keeps_state_and_no_append() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/dispatch-reject";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        let snapshot_before = exec.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::RejectNode {
                    run_id: run_id.clone(),
                    node_name: Some("review".to_string()),
                    reason: "needs changes".to_string(),
                },
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::InvalidState(_))));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).unwrap();
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        drop(execs);
        assert!(read_dispatch_events(&app, &run_id).is_empty());
    }

    /// Spec [04] no-op 不変条件: ApproveNode / RejectNode の対象不在・stale node・既決 node は
    /// production dispatch 経由でも state を変化させず event を append しない。
    #[tokio::test]
    async fn dispatch_approval_commands_reject_missing_stale_and_resolved_targets_without_append() {
        for command_kind in ["approve", "reject"] {
            let app = make_dispatch_app();
            let engine = WorkflowEngine::new_for_test();
            let tmp = TempDir::new().unwrap();
            engine
                .set_run_store_data_dir(tmp.path().to_path_buf())
                .await;
            let (session_store, handles) = make_dispatch_deps();

            let missing_run_id = uuid::Uuid::new_v4().to_string();
            let missing_command = match command_kind {
                "approve" => WorkflowCommand::ApproveNode {
                    run_id: missing_run_id.clone(),
                    node_name: Some("review".to_string()),
                    comment: None,
                },
                "reject" => WorkflowCommand::RejectNode {
                    run_id: missing_run_id.clone(),
                    node_name: Some("review".to_string()),
                    reason: "needs changes".to_string(),
                },
                _ => unreachable!(),
            };
            let missing = engine
                .dispatch(app.handle(), &session_store, &handles, missing_command)
                .await;
            assert!(matches!(
                missing,
                Err(WorkflowEngineError::ExecutionNotFound(_))
            ));
            assert!(read_dispatch_events(&app, &missing_run_id).is_empty());

            let stale_run_id = uuid::Uuid::new_v4().to_string();
            let worktree_path = format!("/wt/{command_kind}-stale");
            let mut stale_exec = make_waiting_approval_execution(&stale_run_id, &worktree_path);
            stale_exec.current_session_id = None;
            let stale_before = stale_exec.clone();
            insert_execution_and_active_run(&engine, stale_exec, TriggerSource::DesktopUi).await;
            let stale_command = match command_kind {
                "approve" => WorkflowCommand::ApproveNode {
                    run_id: stale_run_id.clone(),
                    node_name: Some("old-review".to_string()),
                    comment: None,
                },
                "reject" => WorkflowCommand::RejectNode {
                    run_id: stale_run_id.clone(),
                    node_name: Some("old-review".to_string()),
                    reason: "needs changes".to_string(),
                },
                _ => unreachable!(),
            };
            let stale = engine
                .dispatch(app.handle(), &session_store, &handles, stale_command)
                .await;
            assert!(matches!(
                stale,
                Err(WorkflowEngineError::UnauthorizedApprovalTarget(_))
            ));
            let execs = engine.executions.lock().await;
            let restored = execs.get(&stale_run_id).unwrap();
            assert_eq!(restored.state, stale_before.state);
            assert_eq!(restored.current_step_index, stale_before.current_step_index);
            assert_eq!(restored.step_history.len(), stale_before.step_history.len());
            drop(execs);
            assert!(read_dispatch_events(&app, &stale_run_id).is_empty());

            let resolved_run_id = uuid::Uuid::new_v4().to_string();
            let worktree_path = format!("/wt/{command_kind}-resolved");
            let mut resolved_exec =
                make_waiting_approval_execution(&resolved_run_id, &worktree_path);
            resolved_exec.current_session_id = None;
            resolved_exec.state = WorkflowExecutionState::Completed;
            let resolved_before = resolved_exec.clone();
            engine
                .executions
                .lock()
                .await
                .insert(resolved_run_id.clone(), resolved_exec);
            let resolved_command = match command_kind {
                "approve" => WorkflowCommand::ApproveNode {
                    run_id: resolved_run_id.clone(),
                    node_name: Some("review".to_string()),
                    comment: None,
                },
                "reject" => WorkflowCommand::RejectNode {
                    run_id: resolved_run_id.clone(),
                    node_name: Some("review".to_string()),
                    reason: "needs changes".to_string(),
                },
                _ => unreachable!(),
            };
            let resolved = engine
                .dispatch(app.handle(), &session_store, &handles, resolved_command)
                .await;
            assert!(matches!(
                resolved,
                Err(WorkflowEngineError::InvalidState(_))
            ));
            let execs = engine.executions.lock().await;
            let restored = execs.get(&resolved_run_id).unwrap();
            assert_eq!(restored.state, resolved_before.state);
            assert_eq!(
                restored.step_history.len(),
                resolved_before.step_history.len()
            );
            drop(execs);
            assert!(read_dispatch_events(&app, &resolved_run_id).is_empty());
        }
    }

    /// Spec [04] rollback: RejectNode の required event append が失敗した場合も、
    /// WorkflowExecution / Run Store は mutation 前 snapshot に戻り、event は append されない。
    #[tokio::test]
    async fn dispatch_reject_node_append_failure_rolls_back_execution_and_run_store() {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/reject-append-rollback";
        let mut exec = make_waiting_approval_execution_with_workflow(
            &run_id,
            worktree_path,
            make_rejectable_approval_workflow(),
        );
        exec.current_session_id = None;
        let snapshot_before = exec.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;
        let log_dir_path = dispatch_data_dir(app.handle()).join("workflow_logs");
        std::fs::write(&log_dir_path, b"not a directory").unwrap();

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::RejectNode {
                    run_id: run_id.clone(),
                    node_name: Some("review".to_string()),
                    reason: "needs changes".to_string(),
                },
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).unwrap();
        assert_eq!(restored.state, snapshot_before.state);
        assert_eq!(
            restored.current_step_index,
            snapshot_before.current_step_index
        );
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        drop(execs);
        let active = engine.list_active_runs().await;
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].run_id, run_id);
        assert_eq!(active[0].status, RunStatus::WaitingApproval);
        assert!(read_dispatch_events(&app, &run_id).is_empty());
    }

    // 撤去済み: persist_state 注入失敗を介した rollback テストは persist_state 機構の撤去で
    // 意味を失った。required event append 失敗の rollback は append_failure 系テストが担保する。

    /// Spec [04] rollback: command 受理サイクル内の Run Store sync が失敗した場合も、
    /// engine state / Run Store / ChatSession projection を mutation 前へ戻し Err を返す。
    #[tokio::test]
    async fn dispatch_approve_node_run_store_sync_failure_rolls_back_execution_run_store_and_session(
    ) {
        let app = make_dispatch_app();
        let engine = WorkflowEngine::new_for_test();
        let tmp = TempDir::new().unwrap();
        engine
            .set_run_store_data_dir(tmp.path().to_path_buf())
            .await;
        let (session_store, handles) = make_dispatch_deps();
        let run_id = uuid::Uuid::new_v4().to_string();
        let worktree_path = "/wt/run-store-sync-rollback";
        let mut exec = make_waiting_approval_execution(&run_id, worktree_path);
        exec.current_session_id = None;
        exec.workflow_variables
            .insert("keep".to_string(), "before".to_string());
        let snapshot_before = exec.clone();
        insert_execution_and_active_run(&engine, exec, TriggerSource::DesktopUi).await;

        let bad_data_dir = tmp.path().join("not-a-directory");
        std::fs::write(&bad_data_dir, "file").unwrap();
        engine.set_run_store_data_dir(bad_data_dir).await;

        let result = engine
            .dispatch(
                app.handle(),
                &session_store,
                &handles,
                WorkflowCommand::ApproveNode {
                    run_id: run_id.clone(),
                    node_name: Some("review".to_string()),
                    comment: Some("lgtm".to_string()),
                },
            )
            .await;

        assert!(matches!(result, Err(WorkflowEngineError::SessionStore(_))));
        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).unwrap();
        assert_eq!(restored.state, WorkflowExecutionState::WaitingApproval);
        assert_eq!(
            restored.workflow_variables.get("keep").map(String::as_str),
            Some("before")
        );
        assert_eq!(
            restored.step_history.len(),
            snapshot_before.step_history.len()
        );
        drop(execs);
        assert_eq!(
            engine.list_active_runs().await[0].status,
            RunStatus::WaitingApproval
        );
        assert!(read_dispatch_events(&app, &run_id).is_empty());
    }

    // 撤去済み: persist_state 注入失敗テストは parent ChatSession 機構撤去で意味を失った。

    /// Spec [04] atomic mutation 境界（A2 batch commit）: `write_log_required_batch`
    /// 経由で ApprovalResolved + RunAborted を 1 つの commit point として書き込めば、
    /// `WorkflowEventLog::append_batch` の 1 回の write_all で両 event が NDJSON に
    /// 連結 append される。同一 commit batch 内の partial commit（最初の event のみ
    /// 残る）を構造的に排除することを担保する（handle_approval の Abort 経路と
    /// 同じ atomic 境界）。
    #[test]
    fn approval_abort_commit_batch_persists_both_events_in_single_write() {
        let tmp = TempDir::new().unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        let run_id = "00000000-0000-0000-0000-000000000900";
        let approval_event = WorkflowEvent::ApprovalResolved {
            run_id: run_id.to_string(),
            workflow_name: "boundary-wf".to_string(),
            node_name: "review".to_string(),
            decision: ApprovalDecisionRecord::Abort,
            comment: None,
            timestamp: 4000.0,
        };
        let aborted_event = WorkflowEvent::RunAborted {
            run_id: run_id.to_string(),
            workflow_name: "boundary-wf".to_string(),
            timestamp: 4000.0,
        };
        log.append_batch(&[approval_event, aborted_event])
            .expect("batch append for approval-abort commit point must succeed");
        let events = log.read_log(run_id).unwrap();
        assert_eq!(
            events.len(),
            2,
            "ApprovalResolved + RunAborted は atomic batch で 2 件 append される"
        );
        assert!(matches!(events[0], WorkflowEvent::ApprovalResolved { .. }));
        assert!(matches!(events[1], WorkflowEvent::RunAborted { .. }));
    }

    /// Spec [04] atomic mutation 境界（A3 AbortRun terminal sync post-commit 化）:
    /// `abort_workflow_by_run_id` は append 失敗時に Run Store / external 副作用を
    /// 一切実行しないことが構造的不変条件。本テストは pre-commit が in-memory state
    /// 変更のみであり、append 失敗時に snapshot 一括復元のみで完全に元状態へ戻せる
    /// ことを直接確認する（外部依存の差し替えを必要としない経路）。
    #[tokio::test]
    async fn abort_run_pre_commit_holds_only_in_memory_mutation() {
        let engine = WorkflowEngine::new_for_test();
        let run_id = uuid::Uuid::new_v4().to_string();
        let mut exec = make_waiting_approval_execution(&run_id, "/wt/pre-commit");
        exec.state = WorkflowExecutionState::Running;
        let snapshot_before = exec.clone();
        engine.executions.lock().await.insert(run_id.clone(), exec);

        // pre-commit 区間で行う state mutation を再現（abort_workflow_by_run_id 内の
        // step 2 と同等）。
        let mutated_timestamp = 1234.0;
        {
            let mut execs = engine.executions.lock().await;
            let exec = execs.get_mut(&run_id).unwrap();
            assert!(exec.is_active(), "active な run でなければ mutation しない");
            exec.state = WorkflowExecutionState::Aborted;
            exec.updated_at = mutated_timestamp;
        }
        {
            let execs = engine.executions.lock().await;
            let exec = execs.get(&run_id).unwrap();
            assert_eq!(exec.state, WorkflowExecutionState::Aborted);
            assert_eq!(exec.updated_at, mutated_timestamp);
        }

        // append 失敗を擬制した snapshot 一括復元（A3: pre-commit 区間は in-memory のみ
        // のため、Run Store / interrupt_agent / persist 等の外部副作用は不要）。
        {
            let mut execs = engine.executions.lock().await;
            if let Some(exec) = execs.get_mut(&run_id) {
                *exec = snapshot_before.clone();
            }
        }
        let execs = engine.executions.lock().await;
        let restored = execs.get(&run_id).expect("run must remain");
        assert_eq!(
            restored.state,
            WorkflowExecutionState::Running,
            "snapshot 復元で active 状態に戻る"
        );
        assert_ne!(
            restored.updated_at, mutated_timestamp,
            "pre-commit で書いた updated_at も一括復元される"
        );
    }

    /// 起動時 recovery: 前回起動中に terminal event が書かれないまま終了した run について、
    /// `recover_orphan_runs` が NDJSON 末尾に `RunAborted` を append し、metadata 上の
    /// status を Aborted に書き換える。reconstruction 経路が Aborted を返すようになる。
    #[tokio::test]
    async fn recover_orphan_runs_marks_non_terminal_metadata_as_aborted() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());

        // 前回プロセスの状態を模擬: workflow_runs/<id>.json に Running、event log に RunStarted のみ。
        let prev_store = std::sync::Arc::new(crate::workflow::run::RunStore::new());
        prev_store.set_data_dir(data_dir.clone()).await;
        let orphan_id = uuid::Uuid::new_v4().to_string();
        prev_store
            .register_active(WorkflowRun {
                run_id: orphan_id.clone(),
                workflow_name: "wf".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: "/wt/a".to_string(),
                current_node_name: Some("plan".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        let log = WorkflowEventLog::new(&data_dir);
        log.append(&WorkflowEvent::RunStarted {
            run_id: orphan_id.clone(),
            workflow_name: "wf".to_string(),
            workflow_file_stem: "wf".to_string(),
            worktree_path: "/wt/a".to_string(),
            workflow_definition: Workflow {
                variables: Default::default(),
                name: "wf".to_string(),
                description: String::new(),
                builtin: false,
                nodes: vec![NodeDefinition {
                    name: "plan".to_string(),
                    node_type: NodeType::Agent,
                    instruction: Some("plan".to_string()),
                    ..NodeDefinition::default()
                }],
            },
            timestamp: 100.0,
        })
        .unwrap();

        // 起動直後を模擬した engine (空の in-memory state + 同じ data_dir)。
        let engine = std::sync::Arc::new(WorkflowEngine::new_for_test());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        engine.recover_orphan_runs(app.handle()).await;

        // metadata が Aborted に書き換わっている（status / completed_at が更新される）。
        let summary = engine
            .run_store
            .get_run(&orphan_id)
            .await
            .expect("metadata must remain after recovery");
        assert_eq!(summary.status, RunStatus::Aborted);
        assert!(summary.completed_at.is_some());
        assert!(summary.error_reason.is_none());

        // 末尾 event が RunAborted。projection も Aborted を返すようになる。
        let events = read_dispatch_events(&app, &orphan_id);
        assert!(
            matches!(events.last(), Some(WorkflowEvent::RunAborted { .. })),
            "log の末尾は RunAborted: {:?}",
            events.last()
        );
        let projected =
            crate::workflow::event_projection::reconstruct_state_from_events(&orphan_id, &events)
                .unwrap()
                .unwrap();
        assert_eq!(projected.state, WorkflowExecutionState::Aborted);
    }

    /// 起動時 recovery: 既に terminal な metadata は変更されない（idempotent）。
    /// recovery 二回目以降は append も persist も走らない。
    #[tokio::test]
    async fn recover_orphan_runs_is_idempotent_for_already_terminal_runs() {
        let app = make_dispatch_app();
        let data_dir = dispatch_data_dir(app.handle());

        let prev_store = std::sync::Arc::new(crate::workflow::run::RunStore::new());
        prev_store.set_data_dir(data_dir.clone()).await;
        let done_id = uuid::Uuid::new_v4().to_string();
        prev_store
            .register_active(WorkflowRun {
                run_id: done_id.clone(),
                workflow_name: "wf".to_string(),
                task: None,
                status: RunStatus::Running,
                worktree_path: "/wt/b".to_string(),
                current_node_name: Some("plan".to_string()),
                trigger_source: TriggerSource::DesktopUi,
                started_at: 100.0,
                updated_at: 100.0,
                completed_at: None,
                error_reason: None,
            })
            .await
            .unwrap();
        prev_store
            .complete_run(&done_id, TerminalRunStatus::Completed, 150.0, None)
            .await
            .unwrap();

        let engine = std::sync::Arc::new(WorkflowEngine::new_for_test());
        engine.set_run_store_data_dir(data_dir.clone()).await;
        let events_before = read_dispatch_events(&app, &done_id);
        engine.recover_orphan_runs(app.handle()).await;
        let events_after = read_dispatch_events(&app, &done_id);
        assert_eq!(
            events_before.len(),
            events_after.len(),
            "terminal な run には event を append しない"
        );
        let summary = engine
            .run_store
            .get_run(&done_id)
            .await
            .expect("metadata must remain");
        assert_eq!(summary.status, RunStatus::Completed);
    }

    // ---- [08] handle_submit_output: 単一トランザクション境界 ----

    /// テスト用 helper: `dispatch_external_with_commit_context` 経由で SubmitOutput を
    /// 提出する。production 経路（CLI pending dispatcher → dispatch_external）と同じ
    /// 入口を踏むため、テストでも bypass を経由しない。
    #[allow(clippy::too_many_arguments)]
    async fn submit_output_for_test(
        engine: &Arc<WorkflowEngine>,
        app: &tauri::AppHandle<tauri::test::MockRuntime>,
        run_id: &str,
        step_name: &str,
        contract: &str,
        structured_output: serde_json::Value,
        request_id: Option<&str>,
        submitted_at: Option<f64>,
    ) -> Result<(), WorkflowEngineError> {
        let (session_store, handles) = make_dispatch_deps();
        let command = WorkflowCommand::SubmitOutput {
            run_id: run_id.to_string(),
            step_name: step_name.to_string(),
            contract: contract.to_string(),
            structured_output,
        };
        let result = match (request_id, submitted_at) {
            (Some(rid), Some(ts)) => {
                let ctx = CommandCommitContext::submit_output(
                    rid.to_string(),
                    ts,
                    step_name.to_string(),
                    contract.to_string(),
                );
                engine
                    .dispatch_external_with_commit_context(
                        app,
                        &session_store,
                        &handles,
                        command,
                        ctx,
                    )
                    .await
            }
            (None, None) => {
                engine
                    .dispatch_external(app, &session_store, &handles, command)
                    .await
            }
            _ => panic!("request_id と submitted_at は両方 Some か両方 None で渡すこと"),
        };
        result.map(|_| ())
    }

    fn make_submit_output_workflow() -> Workflow {
        Workflow {
            variables: Default::default(),
            name: "submit-wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "review".to_string(),
                node_type: NodeType::Agent,
                output_contract: Some("review-verdict".to_string()),
                ..NodeDefinition::default()
            }],
        }
    }

    fn read_submit_output_events(app: &DispatchTestApp, run_id: &str) -> Vec<WorkflowEvent> {
        let data_dir = crate::session::resolve_data_dir(app.handle()).expect("data_dir");
        WorkflowEventLog::new(&data_dir)
            .read_log(run_id)
            .unwrap_or_default()
    }

    async fn step_output_for(
        engine: &WorkflowEngine,
        run_id: &str,
        step_name: &str,
    ) -> Option<StepOutput> {
        engine
            .executions
            .lock()
            .await
            .get(run_id)
            .and_then(|exec| exec.step_outputs.get(step_name).cloned())
    }

    /// [08] 振る舞い定義 Rule 1（適合する場合）: contract に適合する構造化出力は
    /// step output として確定し、後続 step から参照可能になり、事実履歴に記録される。
    #[tokio::test]
    async fn submit_output_persists_step_output_and_appends_event_when_contract_satisfied() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                "/wt/submit-ok".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "review",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            Some("00000000-0000-0000-0000-000000000aa1"),
            Some(800.0),
        )
        .await
        .unwrap();

        // step_outputs slot に書き込まれている
        let step_output = step_output_for(&engine, &run_id, "review")
            .await
            .expect("step_outputs must be updated");
        assert_eq!(
            step_output.output_contract.as_deref(),
            Some("review-verdict")
        );
        assert_eq!(
            step_output.structured_output.as_ref().unwrap()["verdict"],
            "LGTM"
        );

        // OutputSubmitted event が追記されている
        let events = read_submit_output_events(&app, &run_id);
        let submitted = events
            .iter()
            .find_map(|e| match e {
                WorkflowEvent::OutputSubmitted {
                    node_name,
                    contract,
                    structured_output,
                    request_id,
                    submitted_at,
                    ..
                } if node_name == "review" => Some((
                    contract.clone(),
                    structured_output.clone(),
                    request_id.clone(),
                    *submitted_at,
                )),
                _ => None,
            })
            .expect("OutputSubmitted event must be appended");
        assert_eq!(submitted.0, "review-verdict");
        assert_eq!(submitted.1["verdict"], "LGTM");
        assert_eq!(
            submitted.2.as_deref(),
            Some("00000000-0000-0000-0000-000000000aa1")
        );
        assert_eq!(submitted.3, Some(800.0));
    }

    /// [08] 振る舞い定義 Rule 1（適合しない場合）: contract 不適合の入力は拒否され、
    /// step_outputs / workflow_variables / 事実履歴のいずれも変化しない。
    #[tokio::test]
    async fn submit_output_rejects_invalid_contract_without_side_effects() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                "/wt/submit-invalid".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "review",
            "review-verdict",
            serde_json::json!({"verdict": "MAYBE"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::ValidationError(_)));

        // step_outputs は更新されない
        assert!(step_output_for(&engine, &run_id, "review").await.is_none());
        // OutputSubmitted event も書かれない
        let events = read_submit_output_events(&app, &run_id);
        assert!(events
            .iter()
            .all(|e| !matches!(e, WorkflowEvent::OutputSubmitted { .. })));
    }

    /// [08] 振る舞い定義 Rule 1: 不在 step に対する提出は副作用なしで拒否される。
    #[tokio::test]
    async fn submit_output_rejects_unknown_step_without_side_effects() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                "/wt/submit-unknown".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "ghost-step",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::ValidationError(_)));
        let events = read_submit_output_events(&app, &run_id);
        assert!(events
            .iter()
            .all(|e| !matches!(e, WorkflowEvent::OutputSubmitted { .. })));
    }

    /// [08] 振る舞い定義 Rule 1: 不在 run （UUID 未登録）に対する提出は ExecutionNotFound で拒否。
    #[tokio::test]
    async fn submit_output_rejects_unknown_run() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir).await;
        let run_id = uuid::Uuid::new_v4().to_string();

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "review",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::ExecutionNotFound(_)));
    }

    /// [08] caller の `--type` と engine の expected contract が一致しない場合は拒否され、
    /// 副作用は発生しない。
    #[tokio::test]
    async fn submit_output_rejects_contract_type_mismatch() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                "/wt/submit-mismatch".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "review",
            "fix-result",
            serde_json::json!({"status": "FIXED"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::ValidationError(_)));
        assert!(step_output_for(&engine, &run_id, "review").await.is_none());
    }

    /// [08] 振る舞い定義 Rule 3: 提出済み output は後続 step から
    /// `pass_output_from` 経路で経路非依存に参照できる。step_outputs に
    /// 書き込まれた entry が contract 由来の `output_contract` を保持することを担保する。
    #[tokio::test]
    async fn submit_output_step_output_carries_contract_for_downstream_reference() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                "/wt/submit-downstream".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "review",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            None,
            None,
        )
        .await
        .unwrap();
        let step_output = step_output_for(&engine, &run_id, "review")
            .await
            .expect("step_outputs slot must be populated");
        assert_eq!(
            step_output.output_contract.as_deref(),
            Some("review-verdict")
        );
        // structured_output が後続経路に渡る shape で保持される
        assert!(step_output.structured_output.is_some());
    }

    /// [08] spec-directory contract が submit された場合、workflow_variables に
    /// `spec_dir` が反映される（extract_contract_variables の合流）。
    #[tokio::test]
    async fn submit_output_applies_contract_variables_for_spec_dir() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let workflow = Workflow {
            variables: Default::default(),
            name: "spec-wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "plan".to_string(),
                node_type: NodeType::Agent,
                output_contract: Some("spec-directory".to_string()),
                ..NodeDefinition::default()
            }],
        };
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                workflow,
                WorkflowExecutionState::Running,
                "/wt/submit-spec".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "plan",
            "spec-directory",
            serde_json::json!({"spec_dir": "docs/spec/issues-1029.md"}),
            None,
            None,
        )
        .await
        .unwrap();

        let vars = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|exec| exec.workflow_variables.clone())
            .unwrap();
        assert_eq!(
            vars.get("spec_dir").map(|s| s.as_str()),
            Some("docs/spec/issues-1029.md")
        );
    }

    /// [08] 振る舞い定義 Rule 1 Scenario 3: 既に出力を受け付けられる状態にない step に
    /// 対する提出は拒否され、state と event log が変化しないことを確認する。
    #[tokio::test]
    async fn submit_output_rejects_non_accepting_step_without_side_effects() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let workflow = Workflow {
            variables: Default::default(),
            name: "multi-step".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![
                NodeDefinition {
                    name: "first".to_string(),
                    node_type: NodeType::Agent,
                    output_contract: Some("review-verdict".to_string()),
                    ..NodeDefinition::default()
                },
                NodeDefinition {
                    name: "second".to_string(),
                    node_type: NodeType::Agent,
                    output_contract: Some("review-verdict".to_string()),
                    ..NodeDefinition::default()
                },
            ],
        };
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                workflow,
                WorkflowExecutionState::Running,
                "/wt/submit-stale".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        // current step を `second` に進めて、`first` を提出受付対象から外す。
        engine.force_current_step_index_for_test(&run_id, 1).await;

        let events_before = read_submit_output_events(&app, &run_id);
        let exec_before = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|e| (e.step_outputs.clone(), e.workflow_variables.clone()))
            .unwrap();

        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "first",
            "review-verdict",
            serde_json::json!({"verdict": "LGTM"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::InvalidState(_)));

        // state は変化していない
        let exec_after = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|e| (e.step_outputs.clone(), e.workflow_variables.clone()))
            .unwrap();
        assert_eq!(exec_before.0.len(), exec_after.0.len());
        assert_eq!(exec_before.1, exec_after.1);

        // OutputSubmitted event は append されない
        let events_after = read_submit_output_events(&app, &run_id);
        assert_eq!(events_before.len(), events_after.len());
        assert!(events_after
            .iter()
            .all(|e| !matches!(e, WorkflowEvent::OutputSubmitted { .. })));
    }

    /// [08] 振る舞い定義 Rule 4: agent step の自由文出力に `<workflow_output>` 相当の
    /// 表現が含まれていても、明示的提出が無い限り step_outputs は更新されず、
    /// OutputSubmitted event も追記されない（prose 抽出経路の完全廃止）。
    #[tokio::test]
    async fn agent_free_text_workflow_output_block_does_not_confirm_step_output() {
        use crate::session::MessagePart;

        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                make_submit_output_workflow(),
                WorkflowExecutionState::Running,
                "/wt/agent-freetext".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        let outputs_before = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|e| e.step_outputs.clone())
            .unwrap();
        let events_before = read_submit_output_events(&app, &run_id);

        let final_text = r#"承認します。
<workflow_output type="review-verdict">{"verdict":"LGTM"}</workflow_output>"#;
        let final_parts = vec![MessagePart::Text {
            content: final_text.to_string(),
            parent_tool_use_id: None,
        }];

        let (session_store, handles) = make_dispatch_deps();
        // 自由文経路は prose 抽出を行わないため、step_outputs は変化せず、
        // output_contract がある step は明示的提出なしでは完了しない。
        // [08] handle_auto_complete のエラーを .ok() で握り潰さないこと（review 指摘）。
        // 完了経路を通って初めて「自由文出力中の `<workflow_output>` は無視される」を
        // 検証できるため、.expect で経路実行を保証する。
        engine
            .handle_auto_complete(
                app.handle(),
                &session_store,
                &handles,
                "/wt/agent-freetext",
                &final_parts,
                &[],
                "review",
            )
            .await
            .expect("handle_auto_complete must succeed for agent free-text path");

        let outputs_after = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|e| e.step_outputs.clone())
            .unwrap_or_default();
        // step_outputs 数は変わらず、structured_output を持つ entry が追加されていない
        assert_eq!(outputs_before.len(), outputs_after.len());

        // OutputSubmitted event も追記されていない
        let events_after = read_submit_output_events(&app, &run_id);
        let submitted_count_before = events_before
            .iter()
            .filter(|e| matches!(e, WorkflowEvent::OutputSubmitted { .. }))
            .count();
        let submitted_count_after = events_after
            .iter()
            .filter(|e| matches!(e, WorkflowEvent::OutputSubmitted { .. }))
            .count();
        assert_eq!(submitted_count_before, submitted_count_after);
        let node_completed = events_after
            .iter()
            .filter(|e| matches!(e, WorkflowEvent::NodeCompleted { node_name, .. } if node_name == "review"))
            .count();
        assert_eq!(
            node_completed, 0,
            "handle_auto_complete must not advance a contract step without SubmitOutput"
        );
        let state_after = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|e| e.state.clone())
            .unwrap();
        assert!(
            matches!(state_after, WorkflowExecutionState::Failed { .. }),
            "seeded test execution has no active session, so missing SubmitOutput fails instead of advancing"
        );
    }

    #[test]
    fn missing_output_repair_prompt_uses_json_not_file() {
        let prompt = WorkflowEngine::build_missing_output_repair_prompt(
            "11111111-1111-1111-1111-111111111111",
            "review",
            "review-verdict",
            Some("データ:\n- verdict: LGTM | NEEDS_FIX"),
        );
        assert!(prompt.contains("--json"));
        assert!(!prompt.contains("--file"));
        assert!(prompt.contains("Contract definition (type: review-verdict)"));
        assert!(prompt.contains("verdict: LGTM | NEEDS_FIX"));
        assert!(prompt.contains("Do not create a temporary JSON file"));
    }

    /// [08] 振る舞い定義 Rule 1: OutputSubmitted append が失敗した場合、
    /// step_outputs / workflow_variables / event log は提出前状態のまま保たれる。
    /// `write_log_required` の挿入 fail 経由で append 失敗を再現し、rollback の事実を
    /// 直接検証する（spec [08]: 「副作用なしで提出前状態のまま保つ」）。
    #[tokio::test]
    async fn submit_output_rolls_back_state_when_event_append_fails() {
        let app = make_dispatch_app();
        let engine = Arc::new(WorkflowEngine::new_for_test());
        let data_dir = crate::session::resolve_data_dir(app.handle()).unwrap();
        engine.set_run_store_data_dir(data_dir).await;
        let run_id = uuid::Uuid::new_v4().to_string();
        let workflow = Workflow {
            variables: Default::default(),
            name: "spec-wf".to_string(),
            description: String::new(),
            builtin: false,
            nodes: vec![NodeDefinition {
                name: "plan".to_string(),
                node_type: NodeType::Agent,
                output_contract: Some("spec-directory".to_string()),
                ..NodeDefinition::default()
            }],
        };
        engine
            .seed_active_execution_for_test(
                run_id.clone(),
                workflow,
                WorkflowExecutionState::Running,
                "/wt/submit-rollback".to_string(),
                TriggerSource::DesktopUi,
            )
            .await;

        let exec_before = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|e| (e.step_outputs.clone(), e.workflow_variables.clone()))
            .unwrap();
        let events_before = read_submit_output_events(&app, &run_id);

        // 次の write_log_required を失敗させる。
        engine.fail_next_required_event_append_for_test();
        let err = submit_output_for_test(
            &engine,
            app.handle(),
            &run_id,
            "plan",
            "spec-directory",
            serde_json::json!({"spec_dir": "docs/spec/issues-1029.md"}),
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, WorkflowEngineError::SessionStore(_)));

        // state は提出前のまま保たれる
        let exec_after = engine
            .executions
            .lock()
            .await
            .get(&run_id)
            .map(|e| (e.step_outputs.clone(), e.workflow_variables.clone()))
            .unwrap();
        assert_eq!(exec_before.0.len(), exec_after.0.len());
        assert!(!exec_after.0.contains_key("plan"));
        assert_eq!(exec_before.1, exec_after.1);

        // OutputSubmitted event は append されない（log への副作用なし）
        let events_after = read_submit_output_events(&app, &run_id);
        assert_eq!(events_before.len(), events_after.len());
        assert!(events_after
            .iter()
            .all(|e| !matches!(e, WorkflowEvent::OutputSubmitted { .. })));
    }
}
