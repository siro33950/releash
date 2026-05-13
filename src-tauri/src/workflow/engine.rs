use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::{Arc, LazyLock};

use regex::RegexBuilder;
use tauri::Manager;
use tokio::sync::Mutex;

use crate::agent_sdk::AgentProcessMap;
use crate::agent_status::{current_timestamp, AgentStatusCenter};
use crate::session::{ChatSession, SessionStore};
use crate::workflow::contract::{
    build_repair_prompt, extract_workflow_output, validate_contract, ContractValidationResult,
    ExtractionResult,
};
use crate::workflow::log::{WorkflowEventLog, WorkflowLogEvent};
use crate::workflow::schema::{
    AggregateConfig, CollectConfig, ParallelStep, ReduceStrategy, StepMode, TransitionRule,
    Workflow,
};
use crate::workflow::state::{
    ApprovalOperations, ParallelStepState, StepHistoryEntry, StepOutput, TokenUsage,
    WorkflowExecutionState, WorkflowState,
};
use crate::workflow::storage;

#[allow(dead_code)]
const MAX_OUTPUT_SIZE: usize = 100 * 1024; // 100KB
const MAX_CONTRACT_RETRIES: u32 = 2;
const MAX_APPROVAL_COMMENT_CHARS: usize = 8192;

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
        chat_session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
    ) -> Result<(), String>;
}

/// production 用の `SessionStartGate` 実装。`start_agent_session_internal` をそのまま呼び出す。
struct RealSessionStartGate<'a> {
    app: &'a tauri::AppHandle,
    handles: &'a Arc<Mutex<AgentProcessMap>>,
    session_store: &'a Arc<SessionStore>,
}

#[async_trait::async_trait]
impl<'a> SessionStartGate for RealSessionStartGate<'a> {
    async fn start_session(
        &self,
        chat_session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
    ) -> Result<(), String> {
        crate::agent_sdk::start_agent_session_internal(
            self.app,
            self.handles,
            self.session_store,
            chat_session_id,
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
/// - ステップセッションへのターン起動 (`start_agent_turn_internal` 相当)
///
/// production では `AppHandle` / `SessionStore` / `AgentProcessMap` を握る
/// `RealStepSessionDeps` を渡し、テストでは記録用のテストダブルを差し替えることで、
/// 「`build_step_prompt` 失敗時に `create_step_session` 等が呼ばれない」という
/// 順序保証を実 production 経路と同じ構造で検証する。
#[async_trait::async_trait]
trait StepSessionDeps: Send + Sync {
    /// 親 ChatSession を取得し、ステップ用設定解決に必要なフィールドを返す。
    async fn fetch_parent_session(
        &self,
        chat_session_id: &str,
    ) -> Result<ParentSessionInfo, WorkflowEngineError>;

    /// ステップ用 ChatSession を生成し、IDと permission_mode を返す。
    async fn create_step_session(
        &self,
        worktree_path: &str,
        step_model: Option<String>,
        step_permission: Option<String>,
        parent: ParentSessionInfo,
    ) -> Result<StepSessionInfo, WorkflowEngineError>;

    /// 合成済み `system_prompt` を AgentSession 開始経路へ受け渡す。
    async fn dispatch_session_start(
        &self,
        step_session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
    ) -> Result<(), WorkflowEngineError>;

    /// ワークフロー状態を ChatSession に永続化する。
    async fn persist_workflow_state(
        &self,
        chat_session_id: &str,
        snapshot: WorkflowState,
    ) -> Result<(), WorkflowEngineError>;

    /// ワークフロー状態をブロードキャストする（best-effort）。
    fn broadcast_state(&self, worktree_path: &str, snapshot: WorkflowState);

    /// ステップ用 ChatSession に対しユーザーターンを起動する。
    async fn start_agent_turn(
        &self,
        step_session_id: &str,
        worktree_path: &str,
        permission_mode: &str,
        prompt: &str,
    ) -> Result<(), WorkflowEngineError>;
}

/// `StepSessionDeps::fetch_parent_session` の戻り値。
#[derive(Clone, Debug)]
struct ParentSessionInfo {
    backend_id: Option<String>,
    selected_model: Option<String>,
    permission_mode: String,
}

/// `StepSessionDeps::create_step_session` の戻り値。
#[derive(Clone, Debug)]
struct StepSessionInfo {
    id: String,
    permission_mode: String,
}

/// production 用の `StepSessionDeps` 実装。
struct RealStepSessionDeps<'a> {
    engine: &'a WorkflowEngine,
    app: &'a tauri::AppHandle,
    handles: &'a Arc<Mutex<AgentProcessMap>>,
    session_store: &'a Arc<SessionStore>,
}

#[async_trait::async_trait]
impl<'a> StepSessionDeps for RealStepSessionDeps<'a> {
    async fn fetch_parent_session(
        &self,
        chat_session_id: &str,
    ) -> Result<ParentSessionInfo, WorkflowEngineError> {
        let data_dir = crate::session::resolve_data_dir(self.app)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;
        let parent = self
            .session_store
            .get_session(&data_dir, chat_session_id)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("get_session: {e}")))?
            .ok_or_else(|| WorkflowEngineError::SessionNotFound(chat_session_id.to_string()))?;
        Ok(ParentSessionInfo {
            backend_id: parent.backend_id,
            selected_model: parent.selected_model,
            permission_mode: parent.permission_mode,
        })
    }

    async fn create_step_session(
        &self,
        worktree_path: &str,
        step_model: Option<String>,
        step_permission: Option<String>,
        parent: ParentSessionInfo,
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
                parent.backend_id,
                parent.selected_model,
                parent.permission_mode,
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

    async fn persist_workflow_state(
        &self,
        chat_session_id: &str,
        snapshot: WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
        self.engine
            .persist_state(self.app, self.session_store, chat_session_id, snapshot)
            .await
    }

    fn broadcast_state(&self, worktree_path: &str, snapshot: WorkflowState) {
        self.engine
            .broadcast_state(self.app, worktree_path, snapshot);
    }

    async fn start_agent_turn(
        &self,
        step_session_id: &str,
        worktree_path: &str,
        permission_mode: &str,
        prompt: &str,
    ) -> Result<(), WorkflowEngineError> {
        crate::agent_sdk::start_agent_turn_internal(
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

/// ワークフロー実行の内部状態。
struct WorkflowExecution {
    id: String,
    workflow: Workflow,
    state: WorkflowExecutionState,
    current_step_index: usize,
    step_execution_counts: HashMap<String, u32>,
    step_history: Vec<StepHistoryEntry>,
    /// ワークフローを開始した親セッションのID（persist_state用）。
    chat_session_id: String,
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
    /// ワークフローレベルの変数（spec-file-path等のcontract結果から設定）。
    workflow_variables: HashMap<String, String>,
    /// 現在のステップでのcontractリトライ回数。
    contract_retry_count: u32,
}

/// 並列実行中の内部状態。
struct ParallelRunState {
    parent_step_name: String,
    aggregate: Option<AggregateConfig>,
    children: Vec<ParallelChildRun>,
}

/// 並列子ステップの実行状態。
struct ParallelChildRun {
    step_name: String,
    session_id: String,
    state: ParallelChildState,
    result: Option<String>,
    structured_output: Option<serde_json::Value>,
    output_contract: Option<String>,
    token_usage: TokenUsage,
    run_index: u32,
    contract_retry_count: u32,
}

/// 並列子ステップの状態。
#[derive(Clone, PartialEq)]
enum ParallelChildState {
    Running,
    Completed,
    Failed,
    Interrupted,
}

/// session_workflow_refsの値型。セッションの種別情報を保持する。
#[derive(Clone)]
struct SessionWorkflowRef {
    worktree_path: String,
    kind: SessionRefKind,
}

/// セッションの種別。
#[derive(Clone, PartialEq)]
enum SessionRefKind {
    /// 親セッション（ワークフロー開始元のChatSession）
    Parent,
    /// 逐次実行中のステップセッション
    SequentialStep,
    /// 並列実行中の子ステップセッション
    ParallelChild { parent_step_name: String },
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

    /// ワークフロー開始の事前条件を検証する（純粋関数）。
    fn validate_start(
        workflow: &Workflow,
        existing: Option<&WorkflowExecution>,
    ) -> Result<(), WorkflowEngineError> {
        if workflow.steps.is_empty() {
            return Err(WorkflowEngineError::InvalidWorkflow(
                "Workflow has no steps".to_string(),
            ));
        }
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
    /// autoモード → タグ検出して遷移
    AutoEvaluate {
        rules: Vec<TransitionRule>,
        step_name: String,
    },
    /// approvalモード → WaitingApproval
    WaitApproval,
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
            chat_session_id: Some(self.chat_session_id.clone()),
            state: self.state.clone(),
            current_step_index: self.current_step_index,
            current_step_name: self.workflow.steps[self.current_step_index].name.clone(),
            current_session_id: self.current_session_id.clone(),
            total_steps: self.workflow.steps.len(),
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
        let step = &self.workflow.steps[self.current_step_index];
        Some(ApprovalOperations {
            can_reject: step.rules.iter().any(|r| r.r#match == "reject"),
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

    /// 現在のステップの完了履歴エントリを生成し、トークン使用量をリセットする。
    fn make_step_history_entry(
        &mut self,
        result: Option<String>,
        structured_output: Option<serde_json::Value>,
        output_contract: Option<String>,
    ) -> StepHistoryEntry {
        let step_name = self.workflow.steps[self.current_step_index].name.clone();
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
        };
        self.current_session_id = None;
        self.contract_retry_count = 0;
        entry
    }

    /// 次のステップ遷移先を判定する（純粋関数）。
    fn decide_next_step(&self) -> NextStepDecision {
        let current_index = self.current_step_index;
        if current_index + 1 >= self.workflow.steps.len() {
            NextStepDecision::Completed
        } else {
            NextStepDecision::TransitionTo(self.workflow.steps[current_index + 1].name.clone())
        }
    }

    /// 指定ステップへの遷移時にサイクルガードを検証する（純粋関数）。
    fn check_cycle_guard(
        &self,
        target_step_name: &str,
    ) -> Result<CycleGuardResult, WorkflowEngineError> {
        let idx = self
            .workflow
            .steps
            .iter()
            .position(|s| s.name == target_step_name)
            .ok_or_else(|| {
                WorkflowEngineError::InvalidWorkflow(format!(
                    "Step '{}' not found in workflow",
                    target_step_name
                ))
            })?;

        let step = &self.workflow.steps[idx];
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

        let step = &self.workflow.steps[self.current_step_index];

        if exit_code != 0 {
            return TurnCompleteAction::SessionError {
                step_name: step.name.clone(),
                exit_code,
            };
        }

        match step.mode_unwrap() {
            StepMode::Auto => TurnCompleteAction::AutoEvaluate {
                rules: step.rules.clone(),
                step_name: step.name.clone(),
            },
            StepMode::Approval => TurnCompleteAction::WaitApproval,
            StepMode::Interactive => TurnCompleteAction::SessionError {
                step_name: step.name.clone(),
                exit_code: 0,
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
        let step = &self.workflow.steps[self.current_step_index];
        match decision {
            ApprovalDecision::Approve => Ok(ApprovalAction::Advance),
            ApprovalDecision::Reject { .. } => {
                match step.rules.iter().find(|r| r.r#match == "reject") {
                    Some(r) => Ok(ApprovalAction::TransitionTo(r.next.clone())),
                    None => Err(WorkflowEngineError::InvalidState(format!(
                        "Step '{}' does not allow reject",
                        step.name
                    ))),
                }
            }
            ApprovalDecision::Abort => Ok(ApprovalAction::Abort),
        }
    }
}

/// approvalモードのユーザー判定。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Reject { comment: String },
    Abort,
}

/// approvalモードの判定結果（純粋関数用）。
#[derive(Debug, Clone, PartialEq)]
enum ApprovalAction {
    Advance,
    TransitionTo(String),
    Abort,
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

/// Contract検証の結果（呼び出し元の次アクションを示す）
enum ContractCheckResult {
    /// contractなし → 通常フローで続行
    NoContract,
    /// 検証成功
    Valid {
        structured_output: serde_json::Value,
        result: Option<String>,
    },
    /// repair prompt送信済み → 呼び出し元はreturn Ok(())
    RetrySent,
    /// workflow Failed化済み → 呼び出し元はreturn Ok(())
    Failed,
}

/// ステップ設定解決の結果。
/// ステップのmodel/permission指定と親セッション設定のマージ結果を保持する。
#[derive(Debug, Clone, PartialEq)]
struct ResolvedStepSettings {
    backend_id: Option<String>,
    selected_model: Option<String>,
    permission_mode: String,
}

/// ステップの model/permission 設定を親セッション設定とマージして解決する。
///
/// - permission: ステップ指定があれば採用、なければ親セッションの値を継承
/// - backend_id: model指定があれば resolved_backend_id を採用、なければ親セッションの値を継承
/// - selected_model: ステップ指定があれば採用、なければ親セッションの値を継承
///
/// `resolved_backend_id` は、ステップにmodel指定がある場合に
/// `resolve_backend_for_step_model` で事前に解決されたbackend_id。
/// model未指定時は無視される。
fn resolve_step_settings(
    step_model: Option<String>,
    step_permission: Option<String>,
    resolved_backend_id: Option<String>,
    parent_backend_id: Option<String>,
    parent_selected_model: Option<String>,
    parent_permission_mode: String,
) -> ResolvedStepSettings {
    let permission_mode = step_permission.unwrap_or(parent_permission_mode);
    let backend_id = if step_model.is_some() {
        resolved_backend_id
    } else {
        parent_backend_id
    };
    let selected_model = step_model.or(parent_selected_model);
    ResolvedStepSettings {
        backend_id,
        selected_model,
        permission_mode,
    }
}

/// ワークフローのステップを順次実行するステートマシンエンジン。
pub struct WorkflowEngine {
    /// worktree_path → WorkflowExecution のマッピング
    executions: Mutex<HashMap<String, WorkflowExecution>>,
    /// session_id（親・ステップ・並列子） → SessionWorkflowRef のマッピング
    session_workflow_refs: Mutex<HashMap<String, SessionWorkflowRef>>,
}

impl WorkflowEngine {
    pub fn new() -> Self {
        Self {
            executions: Mutex::new(HashMap::new()),
            session_workflow_refs: Mutex::new(HashMap::new()),
        }
    }

    /// ステップの model 値から対応するバックエンドIDを解決する。
    async fn resolve_backend_for_step_model(
        &self,
        app: &tauri::AppHandle,
        model: &str,
    ) -> Result<Option<String>, WorkflowEngineError> {
        let registry = app
            .try_state::<Arc<crate::backends::AgentBackendRegistry>>()
            .ok_or_else(|| {
                WorkflowEngineError::InvalidWorkflow(format!(
                    "cannot resolve model '{model}': backend registry is unavailable"
                ))
            })?;
        let backend_id = registry
            .resolve_backend_for_model(model)
            .await
            .ok_or_else(|| {
                WorkflowEngineError::InvalidWorkflow(format!("unknown model: {model}"))
            })?;
        Ok(Some(backend_id))
    }

    /// ステップ設定の解決 → セッション生成 → 解決済み設定の反映 → 保存を一括で行う。
    ///
    /// `start_step_session` と `start_parallel_children` の共通パターンを抽出したヘルパー。
    #[allow(clippy::too_many_arguments)]
    async fn create_step_session_with_settings(
        &self,
        app: &tauri::AppHandle,
        session_store: &SessionStore,
        data_dir: &std::path::Path,
        worktree_path: &str,
        step_model: Option<String>,
        step_permission: Option<String>,
        parent_backend_id: Option<String>,
        parent_selected_model: Option<String>,
        parent_permission_mode: String,
    ) -> Result<ChatSession, WorkflowEngineError> {
        let resolved_backend_id = match step_model {
            Some(ref model) => self.resolve_backend_for_step_model(app, model).await?,
            None => None,
        };
        let settings = resolve_step_settings(
            step_model,
            step_permission,
            resolved_backend_id,
            parent_backend_id,
            parent_selected_model,
            parent_permission_mode,
        );

        let mut step_session = crate::session::create_session_internal(
            session_store,
            data_dir,
            worktree_path,
            settings.backend_id,
        )
        .map_err(|e| WorkflowEngineError::SessionStore(format!("create step session: {e}")))?;
        step_session.selected_model = settings.selected_model;
        step_session.permission_mode = settings.permission_mode;
        session_store
            .save_session(data_dir, &step_session)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("save step session: {e}")))?;

        Ok(step_session)
    }

    /// ワークフローを開始する。
    /// ChatSessionは既に作成済みの前提で、最初のステップのプロンプトを送信する。
    #[allow(clippy::too_many_arguments)]
    pub async fn start_workflow(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        workflow: Workflow,
        chat_session_id: &str,
        file_stem: &str,
        task: Option<String>,
    ) -> Result<(), WorkflowEngineError> {
        // worktree_pathはセッションから取得（唯一のソース）
        // ロック前に非同期I/Oを完了させる
        let data_dir = crate::session::resolve_data_dir(app)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;
        let session = session_store
            .get_session(&data_dir, chat_session_id)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("get_session: {e}")))?
            .ok_or_else(|| WorkflowEngineError::SessionNotFound(chat_session_id.to_string()))?;
        let worktree_path = session.worktree_path.clone();

        // ステップ設定のmodel検証: 全バックエンドの available_models から有効モデルを収集
        if let Some(registry) = app.try_state::<Arc<crate::backends::AgentBackendRegistry>>() {
            let valid_models = registry.collect_all_model_values().await;
            crate::workflow::validation::validate_models(&workflow, &valid_models)
                .map_err(|e| WorkflowEngineError::InvalidWorkflow(e.to_string()))?;
        }

        let now = current_timestamp();
        let mut execution = WorkflowExecution {
            id: uuid::Uuid::new_v4().to_string(),
            workflow: workflow.clone(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            chat_session_id: chat_session_id.to_string(),
            started_at: now,
            updated_at: now,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
        };

        // validate_start → insert → スナップショット確定を同一ロックで原子的に実行
        let step_name = workflow.steps[0].name.clone();
        let snapshot = {
            let mut execs = self.executions.lock().await;
            WorkflowExecution::validate_start(&workflow, execs.get(&worktree_path))?;
            execution.step_execution_counts.insert(step_name.clone(), 1);
            execs.insert(worktree_path.clone(), execution);
            execs.get(&worktree_path).unwrap().to_workflow_state()
        };

        // session_workflow_refs に親セッションIDを登録
        {
            let mut map = self.session_workflow_refs.lock().await;
            map.insert(
                chat_session_id.to_string(),
                SessionWorkflowRef {
                    worktree_path: worktree_path.clone(),
                    kind: SessionRefKind::Parent,
                },
            );
        }

        // 永続化・ブロードキャスト（ロック内で確定したスナップショットを使用）
        if let Err(e) = self
            .persist_state(app, session_store, chat_session_id, snapshot.clone())
            .await
        {
            let mut execs = self.executions.lock().await;
            execs.remove(&worktree_path);
            drop(execs);
            self.cleanup_session_workflow_refs(&worktree_path).await;
            return Err(e);
        }
        self.broadcast_state(app, &worktree_path, snapshot.clone());

        // NDJSONログ: workflow_started + step_started
        self.write_log(
            app,
            WorkflowLogEvent::WorkflowStarted {
                execution_id: snapshot.execution_id.clone(),
                workflow_name: snapshot.workflow_name.clone(),
                workflow_file_stem: file_stem.to_string(),
                worktree_path: worktree_path.clone(),
                workflow_definition: Some(workflow.clone()),
                timestamp: now,
            },
        );
        // 最初のステップが並列ブロックかどうかで分岐
        let first_step_is_parallel = workflow.steps[0].is_parallel_block();

        if first_step_is_parallel {
            // 並列ブロック → start_parallel_children を呼ぶ
            // (StepStartedログは書かず、start_parallel_children内でParallelStarted等を記録)
            if let Err(e) = self
                .start_parallel_children(app, session_store, handles, &worktree_path)
                .await
            {
                let _ = self
                    .set_execution_state(
                        app,
                        session_store,
                        &worktree_path,
                        WorkflowExecutionState::Failed {
                            reason: format!("Failed to start parallel children: {e}"),
                        },
                    )
                    .await;
                return Err(e);
            }
        } else {
            // 逐次ステップ → StepStartedログ + start_step_session
            self.write_log(
                app,
                WorkflowLogEvent::StepStarted {
                    execution_id: snapshot.execution_id,
                    workflow_name: snapshot.workflow_name,
                    step_name: step_name.clone(),
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
                    if let Some(exec) = execs.get_mut(&worktree_path) {
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
                        &worktree_path,
                        WorkflowExecutionState::Failed {
                            reason: format!("Failed to start step session: {e}"),
                        },
                    )
                    .await;
                return Err(e);
            }
        }
        Ok(())
    }

    /// turn_complete後に呼ばれるフック。
    /// autoモード→タグ検出で遷移、approvalモード→WaitingApproval、interactiveモード→何もしない。
    /// SessionError / WaitApproval は判定 + 状態変更を1回のロックで原子的に実行する。
    /// AutoEvaluate はタグ検出が必要なため handle_auto_complete に委譲する。
    #[allow(clippy::too_many_arguments)]
    pub async fn on_turn_complete(
        &self,
        app: &tauri::AppHandle,
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
        let worktree_path = session_ref.worktree_path.clone();

        // セッション種別に応じたディスパッチ
        match &session_ref.kind {
            SessionRefKind::ParallelChild { parent_step_name } => {
                return self
                    .handle_parallel_child_complete(
                        app,
                        session_store,
                        handles,
                        &worktree_path,
                        session_id,
                        parent_step_name,
                        exit_code,
                        final_parts,
                        token_usage,
                    )
                    .await;
            }
            SessionRefKind::Parent => return Ok(()),
            SessionRefKind::SequentialStep => {}
        }

        // 判定 + 状態変更を原子的に実行（AutoEvaluate以外）
        let (chat_session_id, action_or_outcome) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(&worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.clone()))?;

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

            let chat_session_id = exec.chat_session_id.clone();
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
                    Ok(StepOutcome::Persist(exec.to_workflow_state()))
                }
                TurnCompleteAction::WaitApproval => {
                    if exec.is_terminal() {
                        return Ok(());
                    }
                    exec.state = WorkflowExecutionState::WaitingApproval;
                    exec.updated_at = current_timestamp();
                    Ok(StepOutcome::Persist(exec.to_workflow_state()))
                }
                TurnCompleteAction::AutoEvaluate { rules, step_name } => Err((rules, step_name)),
            };
            (chat_session_id, result)
        };

        match action_or_outcome {
            Ok(outcome) => {
                self.execute_outcome(
                    app,
                    session_store,
                    handles,
                    &worktree_path,
                    &chat_session_id,
                    outcome,
                )
                .await
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

    /// ApprovalDecisionのバリデーション。Reject時に空コメントを拒否する。
    fn validate_approval_decision(decision: &ApprovalDecision) -> Result<(), WorkflowEngineError> {
        if let ApprovalDecision::Reject { ref comment } = decision {
            if comment.trim().is_empty() {
                return Err(WorkflowEngineError::ValidationError(
                    "Reject comment must not be empty".to_string(),
                ));
            }
            if comment.chars().count() > MAX_APPROVAL_COMMENT_CHARS {
                return Err(WorkflowEngineError::ValidationError(
                    "Reject comment exceeds 8192 characters".to_string(),
                ));
            }
        }
        Ok(())
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
            ApprovalAction::Abort => {
                exec.state = WorkflowExecutionState::Aborted;
                exec.updated_at = current_timestamp();
                StepOutcome::Persist(exec.to_workflow_state())
            }
        };
        Ok(outcome)
    }

    /// approvalモードでのユーザー判定を処理する。
    /// 判定 + 状態変更 + 履歴記録を1回のロックで原子的に実行し、
    /// ロック外では永続化・ブロードキャスト・AgentSession起動のみ行う。
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_approval(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        decision: ApprovalDecision,
        expected_execution_id: Option<&str>,
        expected_step_name: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        let result_tag = match &decision {
            ApprovalDecision::Approve => "approve",
            ApprovalDecision::Reject { .. } => "reject",
            ApprovalDecision::Abort => "abort",
        };

        // target検証 + session_id + output_contract + workflow を1回のロックで取得
        let (
            current_session_id,
            output_contract,
            workflow_for_contract,
            current_step_index_for_contract,
        ) = {
            let execs = self.executions.lock().await;
            let exec = execs.get(worktree_path).ok_or_else(|| {
                WorkflowEngineError::UnauthorizedWorktree(worktree_path.to_string())
            })?;
            Self::validate_approval_target_snapshot(
                exec,
                expected_execution_id,
                expected_step_name,
            )?;
            (
                exec.current_session_id.clone(),
                exec.workflow.steps[exec.current_step_index]
                    .output_contract
                    .clone(),
                exec.workflow.clone(),
                exec.current_step_index,
            )
        };

        // Reject時: 空コメントバリデーション（副作用の前に実施）
        Self::validate_approval_decision(&decision)?;

        if matches!(decision, ApprovalDecision::Approve) {
            let turn_phase = if let Some(ref sid) = current_session_id {
                let map = handles.lock().await;
                map.get(sid).map(|p| p.turn_phase)
            } else {
                None
            };
            Self::validate_approval_turn_phase(turn_phase)?;
        }

        // ロック外でoutput_textを事前取得（approvalはAgentSession完了後なので取得可能）
        // Reject時はコメントをoutput_textとして使用するため、fetch不要だがApprove時に必要
        let output_text = match &decision {
            ApprovalDecision::Reject { ref comment } => Some(comment.clone()),
            _ => {
                self.fetch_current_output(app, session_store, worktree_path)
                    .await?
            }
        };

        // contract検証（Approve時のみ）。approvalではrepair/failに進めず、状態を変えずに
        // validation_errorとして返す。
        let (structured_output, contract_result) = if matches!(decision, ApprovalDecision::Approve)
        {
            match Self::validate_approval_output_contract(
                app,
                &output_contract,
                output_text.as_deref(),
                &workflow_for_contract,
                current_step_index_for_contract,
            )? {
                ContractCheckResult::NoContract => (None, None),
                ContractCheckResult::Valid {
                    structured_output,
                    result,
                } => (Some(structured_output), result),
                ContractCheckResult::RetrySent | ContractCheckResult::Failed => unreachable!(),
            }
        } else if let ApprovalDecision::Reject { ref comment } = decision {
            let secrets = Self::collect_configured_secret_values(app);
            (
                Some(Self::reject_structured_output(comment, &secrets)),
                None,
            )
        } else {
            (None, None)
        };

        let application_output_contract = if matches!(decision, ApprovalDecision::Approve) {
            output_contract.clone()
        } else {
            None
        };
        let contract_variables =
            Self::extract_contract_variables(&application_output_contract, &structured_output);

        // contract resultがあればそちらを優先、なければresult_tag
        let effective_result = contract_result.unwrap_or_else(|| result_tag.to_string());

        // 判定 + 状態変更 + 履歴記録を原子的に実行
        let (chat_session_id, outcome) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            Self::validate_approval_target_snapshot(
                exec,
                expected_execution_id,
                expected_step_name,
            )?;
            let chat_session_id = exec.chat_session_id.clone();
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
            (chat_session_id, outcome)
        };

        self.execute_outcome(
            app,
            session_store,
            handles,
            worktree_path,
            &chat_session_id,
            outcome,
        )
        .await
    }

    /// ワークフローを中断する。
    pub async fn abort_workflow(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
    ) -> Result<(), WorkflowEngineError> {
        let (current_step_session_id, parallel_session_ids);
        {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;

            // 既に終了状態なら何もしない
            if !exec.is_active() {
                return Ok(());
            }
            current_step_session_id = exec.current_session_id.clone();
            parallel_session_ids = exec.parallel_run.as_ref().map(|pr| {
                pr.children
                    .iter()
                    .filter(|c| c.state == ParallelChildState::Running)
                    .map(|c| c.session_id.clone())
                    .collect::<Vec<_>>()
            });
        }

        // 実行中のステップセッションを中断
        if let Some(ref step_sid) = current_step_session_id {
            self.interrupt_agent(handles, step_sid).await;
        }

        // 並列子ステップのセッションも中断
        if let Some(session_ids) = parallel_session_ids {
            for sid in &session_ids {
                self.interrupt_agent(handles, sid).await;
            }
        }

        self.set_execution_state(
            app,
            session_store,
            worktree_path,
            WorkflowExecutionState::Aborted,
        )
        .await
    }

    /// 並列子ステップの完了を処理する。
    #[allow(clippy::too_many_arguments)]
    async fn handle_parallel_child_complete(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        session_id: &str,
        parent_step_name: &str,
        exit_code: i64,
        final_parts: &[crate::session::MessagePart],
        token_usage: Option<(u64, u64)>,
    ) -> Result<(), WorkflowEngineError> {
        // 子ステップのoutput_textを取得
        let output_text = {
            let text = Self::extract_text_from_parts(final_parts);
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        };

        // 子ステップのoutput_contractを取得し、contract検証を実行（ロック外）
        let child_output_contract = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let step = &exec.workflow.steps[exec.current_step_index];
            step.parallel.as_ref().and_then(|children| {
                let pr = exec.parallel_run.as_ref()?;
                let child_run = pr.children.iter().find(|c| c.session_id == session_id)?;
                children
                    .iter()
                    .find(|c| c.name == child_run.step_name)
                    .and_then(|c| c.output_contract.clone())
            })
        };

        // contract検証（exit_code == 0 かつ output_contractが設定されている場合のみ）
        let (child_result, child_structured_output) = if exit_code == 0 {
            if let Some(ref contract) = child_output_contract {
                // output_textがNoneの場合もNoBlockとして検証する
                let extraction = match &output_text {
                    Some(text) => extract_workflow_output(text),
                    None => ExtractionResult::NoBlock,
                };
                match validate_contract(contract, extraction) {
                    ContractValidationResult::Valid {
                        structured_output,
                        result,
                    } => (
                        result,
                        Some(Self::mask_sensitive_structured_output(
                            app,
                            contract,
                            structured_output,
                        )),
                    ),
                    ContractValidationResult::Invalid(violation) => {
                        // contract violation: child単位のリトライまたは失敗
                        let (should_retry, retry_count, exec_id, wf_name, child_step_name) = {
                            let mut execs = self.executions.lock().await;
                            let exec = execs.get_mut(worktree_path).ok_or_else(|| {
                                WorkflowEngineError::ExecutionNotFound(worktree_path.to_string())
                            })?;
                            let exec_id = exec.id.clone();
                            let wf_name = exec.workflow.name.clone();
                            let pr = exec.parallel_run.as_mut().ok_or_else(|| {
                                WorkflowEngineError::ExecutionNotFound(worktree_path.to_string())
                            })?;
                            let child = pr
                                .children
                                .iter_mut()
                                .find(|c| c.session_id == session_id)
                                .ok_or_else(|| {
                                    WorkflowEngineError::ExecutionNotFound(
                                        worktree_path.to_string(),
                                    )
                                })?;
                            let retry_count = child.contract_retry_count;
                            let child_step_name = child.step_name.clone();
                            if retry_count < MAX_CONTRACT_RETRIES {
                                child.contract_retry_count += 1;
                                (true, retry_count, exec_id, wf_name, child_step_name)
                            } else {
                                (false, retry_count, exec_id, wf_name, child_step_name)
                            }
                        };

                        if should_retry {
                            self.send_contract_repair(
                                app,
                                session_store,
                                handles,
                                worktree_path,
                                session_id,
                                contract,
                                &violation,
                                &exec_id,
                                &wf_name,
                                &child_step_name,
                                retry_count + 1,
                            )
                            .await?;
                            return Ok(());
                        } else {
                            // リトライ上限超過 → ワークフロー全体をFailed
                            let (chat_session_id, snapshot, running_ids) = {
                                let mut execs = self.executions.lock().await;
                                let exec = execs.get_mut(worktree_path).ok_or_else(|| {
                                    WorkflowEngineError::ExecutionNotFound(
                                        worktree_path.to_string(),
                                    )
                                })?;
                                let chat_session_id = exec.chat_session_id.clone();
                                let running_ids: Vec<String> = exec
                                    .parallel_run
                                    .as_mut()
                                    .map(|pr| {
                                        pr.children
                                            .iter_mut()
                                            .filter(|c| c.state == ParallelChildState::Running)
                                            .map(|c| {
                                                c.state = ParallelChildState::Interrupted;
                                                c.session_id.clone()
                                            })
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                exec.state = WorkflowExecutionState::Failed {
                                        reason: format!(
                                        "Contract violation at parallel child '{}' after {} retries: {}",
                                        child_step_name, MAX_CONTRACT_RETRIES, violation.details
                                    ),
                                    };
                                exec.parallel_run = None;
                                exec.updated_at = current_timestamp();
                                (chat_session_id, exec.to_workflow_state(), running_ids)
                            };
                            for sid in &running_ids {
                                self.interrupt_agent(handles, sid).await;
                            }
                            self.persist_state(
                                app,
                                session_store,
                                &chat_session_id,
                                snapshot.clone(),
                            )
                            .await?;
                            self.broadcast_state(app, worktree_path, snapshot.clone());
                            self.write_terminal_log(app, &snapshot);
                            self.cleanup_session_workflow_refs(worktree_path).await;
                            return Ok(());
                        }
                    }
                }
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        // contract検証成功時のworkflow_variables反映
        self.apply_contract_variables(
            worktree_path,
            &child_output_contract,
            &child_structured_output,
        )
        .await;

        // ロック内: 子ステップの状態更新 + 全完了チェック
        let (chat_session_id, all_completed, outcome_opt) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;

            if exec.is_terminal() {
                return Ok(());
            }

            let chat_session_id = exec.chat_session_id.clone();
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

                // 他の子ステップをinterrupt
                for sid in &running_ids {
                    self.interrupt_agent(handles, sid).await;
                }

                self.persist_state(app, session_store, &chat_session_id, snapshot.clone())
                    .await?;
                self.broadcast_state(app, worktree_path, snapshot.clone());
                self.write_terminal_log(app, &snapshot);
                self.cleanup_session_workflow_refs(worktree_path).await;
                return Ok(());
            }

            // 成功
            child.state = ParallelChildState::Completed;
            child.result = child_result.clone();
            child.structured_output = child_structured_output.clone();
            let child_name = child.step_name.clone();
            let child_token_usage = child.token_usage.clone();
            let child_run_index = child.run_index;

            // output_contractがあるstepのみStepOutputを生成する（Spec準拠）
            let log_result = child_result.clone();
            let log_structured_output = child_structured_output.clone();
            if child_structured_output.is_some() {
                exec.step_outputs.insert(
                    child_name.clone(),
                    StepOutput {
                        step_name: child_name.clone(),
                        run_index: child_run_index,
                        session_id: Some(session_id.to_string()),
                        result: child_result,
                        structured_output: child_structured_output,
                        output_contract: child_output_contract,
                        token_usage: Some(child_token_usage.clone()),
                        completed_at: current_timestamp(),
                    },
                );
            }

            // ParallelStepCompleted ログ
            self.write_log(
                app,
                WorkflowLogEvent::ParallelStepCompleted {
                    execution_id: exec.id.clone(),
                    workflow_name: exec.workflow.name.clone(),
                    parent_step_name: pr.parent_step_name.clone(),
                    child_step_name: child_name,
                    result: log_result,
                    session_id: session_id.to_string(),
                    token_usage: Some(child_token_usage),
                    structured_output: log_structured_output,
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
                (chat_session_id, false, Some(StepOutcome::Persist(snapshot)))
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
                        WorkflowLogEvent::ParallelCompleted {
                            execution_id: exec.id.clone(),
                            workflow_name: exec.workflow.name.clone(),
                            parent_step_name: parent_step_name.clone(),
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
                        WorkflowLogEvent::ParallelCompleted {
                            execution_id: exec.id.clone(),
                            workflow_name: exec.workflow.name.clone(),
                            parent_step_name: parent_step_name.clone(),
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
                    };
                    exec.current_step_token_usage = TokenUsage::default();
                    exec.current_session_id = None;
                    exec.step_history.push(entry);

                    Self::apply_advance(exec)
                };
                (chat_session_id, true, Some(outcome))
            }
        };

        if let Some(outcome) = outcome_opt {
            if all_completed {
                self.execute_outcome(
                    app,
                    session_store,
                    handles,
                    worktree_path,
                    &chat_session_id,
                    outcome,
                )
                .await?;
            } else {
                // まだ完了していない → Persistのみ
                if let StepOutcome::Persist(snapshot) = outcome {
                    self.persist_state(app, session_store, &chat_session_id, snapshot.clone())
                        .await?;
                    self.broadcast_state(app, worktree_path, snapshot);
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
        agg: &AggregateConfig,
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

    fn validate_approval_output_contract(
        app: &tauri::AppHandle,
        output_contract: &Option<String>,
        output_text: Option<&str>,
        workflow: &Workflow,
        current_step_index: usize,
    ) -> Result<ContractCheckResult, WorkflowEngineError> {
        let contract = match output_contract {
            Some(c) => c,
            None => return Ok(ContractCheckResult::NoContract),
        };
        let extraction = match output_text {
            Some(text) if !text.trim().is_empty() => extract_workflow_output(text),
            _ => ExtractionResult::NoBlock,
        };
        match Self::validate_approval_contract_extraction(
            contract,
            extraction,
            workflow,
            current_step_index,
        )? {
            ContractCheckResult::Valid {
                structured_output,
                result,
            } => Ok(ContractCheckResult::Valid {
                structured_output: Self::mask_sensitive_structured_output(
                    app,
                    contract,
                    structured_output,
                ),
                result,
            }),
            other => Ok(other),
        }
    }

    fn validate_approval_contract_extraction(
        contract: &str,
        extraction: ExtractionResult,
        workflow: &Workflow,
        current_step_index: usize,
    ) -> Result<ContractCheckResult, WorkflowEngineError> {
        let step_name = workflow
            .steps
            .get(current_step_index)
            .map(|s| s.name.as_str())
            .unwrap_or("<unknown>");
        match validate_contract(contract, extraction) {
            ContractValidationResult::Valid {
                structured_output,
                result,
            } => {
                Self::validate_approval_contract_semantics(
                    contract,
                    &structured_output,
                    workflow,
                    current_step_index,
                )?;
                Ok(ContractCheckResult::Valid {
                    structured_output,
                    result,
                })
            }
            ContractValidationResult::Invalid(violation) => {
                Err(WorkflowEngineError::ValidationError(format!(
                    "approval output contract violation at step '{}': {} ({})",
                    step_name, violation.details, violation.reason
                )))
            }
        }
    }

    fn validate_approval_contract_semantics(
        contract: &str,
        structured_output: &serde_json::Value,
        workflow: &Workflow,
        current_step_index: usize,
    ) -> Result<(), WorkflowEngineError> {
        if contract != "approved-fix-policy" {
            return Ok(());
        }
        let step = workflow.steps.get(current_step_index).ok_or_else(|| {
            WorkflowEngineError::InvalidWorkflow(format!(
                "Current step index {} is out of range",
                current_step_index
            ))
        })?;
        let review_step = structured_output
            .get("review_step")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                WorkflowEngineError::ValidationError(format!(
                    "approval output contract violation at step '{}': Missing required field \"review_step\". (missing_field)",
                    step.name
                ))
            })?;
        if !step
            .pass_output_from
            .as_deref()
            .is_some_and(|refs| refs.iter().any(|r| r == review_step))
        {
            return Err(WorkflowEngineError::ValidationError(format!(
                "approval output contract violation at step '{}': \"review_step\" must name a review source passed to this approval step. (unknown_review_step)",
                step.name
            )));
        }
        if !Self::workflow_step_is_review_source(workflow, review_step) {
            return Err(WorkflowEngineError::ValidationError(format!(
                "approval output contract violation at step '{}': \"review_step\" must name a review or aggregate step in the current workflow. (unknown_review_step)",
                step.name
            )));
        }
        Ok(())
    }

    fn workflow_step_is_review_source(workflow: &Workflow, step_name: &str) -> bool {
        workflow.steps.iter().any(|step| {
            (step.name == step_name
                && (step.aggregate.is_some()
                    || step.output_contract.as_deref() == Some("review-verdict")))
                || step.parallel.as_ref().is_some_and(|children| {
                    children.iter().any(|child| {
                        child.name == step_name
                            && child.output_contract.as_deref() == Some("review-verdict")
                    })
                })
        })
    }

    /// 非並列stepのcontract検証を共通処理する。
    /// auto および並列子ステップの2パスで同一のcontract検証・retry・failure処理を行う。
    #[allow(clippy::too_many_arguments)]
    async fn validate_and_handle_contract(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        output_contract: &Option<String>,
        output_text: Option<&str>,
        current_session_id: &Option<String>,
        step_name: &str,
    ) -> Result<ContractCheckResult, WorkflowEngineError> {
        let contract = match output_contract {
            Some(c) => c,
            None => return Ok(ContractCheckResult::NoContract),
        };

        // output_textがNoneの場合もNoBlockとして検証する（Spec: blockが存在しない場合はretry）
        let extraction = match output_text {
            Some(text) => extract_workflow_output(text),
            None => ExtractionResult::NoBlock,
        };
        match validate_contract(contract, extraction) {
            ContractValidationResult::Valid {
                structured_output,
                result,
            } => Ok(ContractCheckResult::Valid {
                structured_output: Self::mask_sensitive_structured_output(
                    app,
                    contract,
                    structured_output,
                ),
                result,
            }),
            ContractValidationResult::Invalid(violation) => {
                let (should_retry, retry_count, exec_id, wf_name) = {
                    let mut execs = self.executions.lock().await;
                    let exec = execs.get_mut(worktree_path).ok_or_else(|| {
                        WorkflowEngineError::ExecutionNotFound(worktree_path.to_string())
                    })?;
                    let retry_count = exec.contract_retry_count;
                    let exec_id = exec.id.clone();
                    let wf_name = exec.workflow.name.clone();
                    if retry_count < MAX_CONTRACT_RETRIES {
                        exec.contract_retry_count += 1;
                        (true, retry_count, exec_id, wf_name)
                    } else {
                        (false, retry_count, exec_id, wf_name)
                    }
                };

                if should_retry {
                    if let Some(ref sid) = current_session_id {
                        self.send_contract_repair(
                            app,
                            session_store,
                            handles,
                            worktree_path,
                            sid,
                            contract,
                            &violation,
                            &exec_id,
                            &wf_name,
                            step_name,
                            retry_count + 1,
                        )
                        .await?;
                        return Ok(ContractCheckResult::RetrySent);
                    }
                }
                // retry不可またはsession_idなし → Failed遷移
                {
                    let (chat_session_id, snapshot) = {
                        let mut execs = self.executions.lock().await;
                        let exec = execs.get_mut(worktree_path).ok_or_else(|| {
                            WorkflowEngineError::ExecutionNotFound(worktree_path.to_string())
                        })?;
                        let chat_session_id = exec.chat_session_id.clone();
                        let entry = exec.make_step_history_entry(
                            Some("contract_violation".to_string()),
                            None,
                            output_contract.clone(),
                        );
                        exec.step_history.push(entry);
                        let fail_reason = if should_retry {
                            format!(
                                "Contract violation at step '{}': no active session for repair: {}",
                                step_name, violation.details
                            )
                        } else {
                            format!(
                                "Contract violation at step '{}' after {} retries: {}",
                                step_name, MAX_CONTRACT_RETRIES, violation.details
                            )
                        };
                        exec.state = WorkflowExecutionState::Failed {
                            reason: fail_reason,
                        };
                        exec.updated_at = current_timestamp();
                        (chat_session_id, exec.to_workflow_state())
                    };
                    self.persist_state(app, session_store, &chat_session_id, snapshot.clone())
                        .await?;
                    self.broadcast_state(app, worktree_path, snapshot.clone());
                    self.write_terminal_log(app, &snapshot);
                    self.cleanup_session_workflow_refs(worktree_path).await;
                    Ok(ContractCheckResult::Failed)
                }
            }
        }
    }

    /// contract violation時のrepair prompt送信を行う共通ヘルパー。
    /// ログ書き込み・ファセット読み込み・repair prompt生成・agent turn送信を一箇所にまとめる。
    #[allow(clippy::too_many_arguments)]
    async fn send_contract_repair(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        session_id: &str,
        contract: &str,
        violation: &crate::workflow::contract::ContractViolation,
        exec_id: &str,
        wf_name: &str,
        step_name: &str,
        attempt: u32,
    ) -> Result<(), WorkflowEngineError> {
        self.write_log(
            app,
            WorkflowLogEvent::ContractRepairRequested {
                execution_id: exec_id.to_string(),
                workflow_name: wf_name.to_string(),
                step_name: step_name.to_string(),
                attempt,
                violation_reason: violation.reason.clone(),
                timestamp: current_timestamp(),
            },
        );

        let contract_definition = {
            let base_dir = storage::facets_base_dir();
            crate::workflow::facet::load_facet(
                crate::workflow::facet::FacetKind::OutputContract,
                contract,
                &base_dir,
            )
            .ok()
        };
        let repair_prompt =
            build_repair_prompt(contract, violation, contract_definition.as_deref());
        let permission_mode = {
            let data_dir = crate::session::resolve_data_dir(app)
                .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;
            let step_session = session_store
                .get_session(&data_dir, session_id)
                .map_err(|e| WorkflowEngineError::SessionStore(format!("get_session: {e}")))?
                .ok_or_else(|| WorkflowEngineError::SessionNotFound(session_id.to_string()))?;
            step_session.permission_mode
        };
        crate::agent_sdk::start_agent_turn_internal(
            app,
            handles,
            session_store,
            session_id,
            worktree_path,
            &permission_mode,
            &repair_prompt,
        )
        .await
        .map_err(WorkflowEngineError::AgentSession)?;
        Ok(())
    }

    /// 指定worktree_pathに関連する session_workflow_refs エントリを削除する。
    async fn cleanup_session_workflow_refs(&self, worktree_path: &str) {
        let mut map = self.session_workflow_refs.lock().await;
        map.retain(|_, r| r.worktree_path != worktree_path);
    }

    /// 状態取得。worktree_pathで直接検索する。
    pub async fn get_state(&self, worktree_path: &str) -> Option<WorkflowState> {
        let execs = self.executions.lock().await;
        execs.get(worktree_path).map(|e| e.to_workflow_state())
    }

    /// session_idがワークフロー実行中かどうか。
    pub async fn is_running(&self, session_id: &str) -> bool {
        let Some(worktree_path) = self.resolve_worktree_path(session_id).await else {
            return false;
        };
        let execs = self.executions.lock().await;
        execs.get(&worktree_path).is_some_and(|e| e.is_active())
    }

    pub async fn validate_approval_chat_instruction(
        &self,
        session_id: &str,
        content: &str,
    ) -> Result<(), WorkflowEngineError> {
        let Some(session_ref) = self.resolve_session_ref(session_id).await else {
            return Ok(());
        };
        if session_ref.kind != SessionRefKind::SequentialStep {
            return Ok(());
        }

        let execs = self.executions.lock().await;
        let Some(exec) = execs.get(&session_ref.worktree_path) else {
            return Ok(());
        };
        let step = &exec.workflow.steps[exec.current_step_index];
        let is_current_approval_session = *step.mode_unwrap() == StepMode::Approval
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
        if content.chars().count() > MAX_APPROVAL_COMMENT_CHARS {
            return Err(WorkflowEngineError::ValidationError(
                "approval chat instruction exceeds 8192 characters".to_string(),
            ));
        }
        Ok(())
    }

    fn is_approval_step_session(exec: &WorkflowExecution, session_id: &str) -> bool {
        let step_is_approval = |step_name: &str| {
            exec.workflow
                .steps
                .iter()
                .find(|step| step.name == step_name)
                .is_some_and(|step| *step.mode_unwrap() == StepMode::Approval)
        };

        if exec.current_session_id.as_deref() == Some(session_id)
            && step_is_approval(&exec.workflow.steps[exec.current_step_index].name)
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
        let exec = execs
            .get(worktree_path)
            .ok_or_else(|| WorkflowEngineError::UnauthorizedWorktree(worktree_path.to_string()))?;
        Self::validate_approval_target_snapshot(exec, expected_execution_id, expected_step_name)
    }

    fn validate_approval_target_snapshot(
        exec: &WorkflowExecution,
        expected_execution_id: Option<&str>,
        expected_step_name: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        if exec.state != WorkflowExecutionState::WaitingApproval {
            return Err(WorkflowEngineError::InvalidState(
                "Workflow is not waiting for approval".to_string(),
            ));
        }
        let expected_execution_id = expected_execution_id.ok_or_else(|| {
            WorkflowEngineError::UnauthorizedApprovalTarget("execution_id is required".to_string())
        })?;
        let expected_step_name = expected_step_name.ok_or_else(|| {
            WorkflowEngineError::UnauthorizedApprovalTarget("step_name is required".to_string())
        })?;
        if expected_execution_id != exec.id {
            return Err(WorkflowEngineError::UnauthorizedApprovalTarget(
                "execution_id does not match".to_string(),
            ));
        }
        let current_step = &exec.workflow.steps[exec.current_step_index].name;
        if expected_step_name != current_step {
            return Err(WorkflowEngineError::UnauthorizedApprovalTarget(
                "step does not match".to_string(),
            ));
        }
        Ok(())
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
    pub async fn resolve_worktree_path(&self, session_id: &str) -> Option<String> {
        let map = self.session_workflow_refs.lock().await;
        map.get(session_id).map(|r| r.worktree_path.clone())
    }

    /// セッションIDからSessionWorkflowRefを解決する。
    async fn resolve_session_ref(&self, session_id: &str) -> Option<SessionWorkflowRef> {
        let map = self.session_workflow_refs.lock().await;
        map.get(session_id).cloned()
    }

    // ---- 内部メソッド ----

    /// 実行状態を更新し、永続化・ブロードキャストする。
    async fn set_execution_state(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        worktree_path: &str,
        new_state: WorkflowExecutionState,
    ) -> Result<(), WorkflowEngineError> {
        let (chat_session_id, snapshot) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            // 終了状態（Completed/Failed/Aborted）からの上書きを防止
            if exec.is_terminal() {
                return Ok(());
            }
            exec.state = new_state;
            exec.updated_at = current_timestamp();
            (exec.chat_session_id.clone(), exec.to_workflow_state())
        };
        self.persist_state(app, session_store, &chat_session_id, snapshot.clone())
            .await?;
        self.broadcast_state(app, worktree_path, snapshot.clone());
        if matches!(
            snapshot.state,
            WorkflowExecutionState::Completed
                | WorkflowExecutionState::Failed { .. }
                | WorkflowExecutionState::Aborted
        ) {
            if matches!(snapshot.state, WorkflowExecutionState::Completed) {
                self.write_last_step_completed_log(app, &snapshot);
            }
            self.write_terminal_log(app, &snapshot);
            self.cleanup_session_workflow_refs(worktree_path).await;
        }
        Ok(())
    }

    /// autoモードのタグ検出結果を処理する。
    /// 判定 + 状態変更 + 履歴記録を1回のロックで原子的に実行する。
    /// output_contractが設定されたステップではcontract検証を実行し、
    /// 違反時はリトライプロンプトを送信する。
    #[allow(clippy::too_many_arguments)]
    async fn handle_auto_complete(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        final_parts: &[crate::session::MessagePart],
        rules: &[TransitionRule],
        step_name: &str,
    ) -> Result<(), WorkflowEngineError> {
        // テキストパートを結合（ロック外で完了）
        let text = Self::extract_text_from_parts(final_parts);

        // contract検証
        let (output_contract, current_session_id) = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let step = &exec.workflow.steps[exec.current_step_index];
            (
                step.output_contract.clone(),
                exec.current_session_id.clone(),
            )
        };

        let (structured_output, contract_result) = match self
            .validate_and_handle_contract(
                app,
                session_store,
                handles,
                worktree_path,
                &output_contract,
                Some(&text),
                &current_session_id,
                step_name,
            )
            .await?
        {
            ContractCheckResult::NoContract => (None, None),
            ContractCheckResult::Valid {
                structured_output,
                result,
            } => (Some(structured_output), result),
            ContractCheckResult::RetrySent | ContractCheckResult::Failed => return Ok(()),
        };

        // contract検証成功時のworkflow_variables反映
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
        let (chat_session_id, outcome) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let chat_session_id = exec.chat_session_id.clone();

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
            (chat_session_id, outcome)
        };

        self.execute_outcome(
            app,
            session_store,
            handles,
            worktree_path,
            &chat_session_id,
            outcome,
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
    async fn start_step_session(
        &self,
        app: &tauri::AppHandle,
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
    /// 2. `deps.fetch_parent_session`
    /// 3. `deps.create_step_session`
    /// 4. `session_workflow_refs` への登録
    /// 5. `deps.dispatch_session_start`（AgentSession 開始）
    /// 6. `executions.current_session_id` 更新と永続化・ブロードキャスト
    /// 7. `deps.start_agent_turn`（ターン起動）
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
            chat_session_id,
            step_clone,
            step_outputs_clone,
            step_history_clone,
            task_clone,
            workflow_variables_clone,
        ) = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let step = &exec.workflow.steps[exec.current_step_index];
            (
                exec.chat_session_id.clone(),
                step.clone(),
                exec.step_outputs.clone(),
                exec.step_history.clone(),
                exec.task.clone(),
                exec.workflow_variables.clone(),
            )
        };

        // プロンプト合成（純粋関数）を最初に行う。
        // ここで失敗（参照先ファセットが存在しない等）した場合、後続の
        // ChatSession 生成・`session_workflow_refs` 登録・AgentSession 開始は一切
        // 行われない。これにより、`start_step_session` がエラー経路で孤立した
        // ChatSession や参照マップ entry を残さないことを構造的に保証する。
        let base_dir = storage::facets_base_dir();
        let (system_prompt, prompt) = Self::build_step_prompt(
            &step_clone,
            &base_dir,
            worktree_path,
            task_clone.as_deref(),
            &step_outputs_clone,
            &step_history_clone,
            &workflow_variables_clone,
        )?;

        let parent = deps.fetch_parent_session(&chat_session_id).await?;

        // ステップ設定の解決 → セッション生成
        let step_session = deps
            .create_step_session(
                worktree_path,
                step_clone.model.clone(),
                step_clone.permission.clone(),
                parent,
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
                    worktree_path: worktree_path.to_string(),
                    kind: SessionRefKind::SequentialStep,
                },
            );
        }

        // 合成済み system_prompt を AgentSession 起動経路へ受け渡す。
        deps.dispatch_session_start(&step_session_id, worktree_path, None, system_prompt)
            .await?;

        // ステップセッションIDをワークフロー実行に紐付け
        let snapshot = {
            let mut execs = self.executions.lock().await;
            if let Some(exec) = execs.get_mut(worktree_path) {
                exec.current_session_id = Some(step_session_id.clone());
                Some(exec.to_workflow_state())
            } else {
                None
            }
        };

        if let Some(snapshot) = snapshot {
            deps.persist_workflow_state(&chat_session_id, snapshot.clone())
                .await?;
            deps.broadcast_state(worktree_path, snapshot);
        }

        // プロンプト送信（ステップ用セッションIDを使用）
        deps.start_agent_turn(&step_session_id, worktree_path, &permission_mode, &prompt)
            .await
    }

    /// ファセット合成パイプライン: compose → 変数展開 → step output注入
    /// start_step_session の中核ロジックを純粋関数として切り出し、テスト可能にする。
    pub(crate) fn build_step_prompt(
        step: &crate::workflow::schema::Step,
        facets_base_dir: &Path,
        worktree_path: &str,
        task: Option<&str>,
        step_outputs: &HashMap<String, StepOutput>,
        step_history: &[StepHistoryEntry],
        workflow_variables: &HashMap<String, String>,
    ) -> Result<(Option<String>, String), WorkflowEngineError> {
        // inline_prompt のみ（ファセット参照なし）のステップ: inline_prompt をそのまま使用
        if !step.has_facet_refs() {
            if let Some(ref inline) = step.inline_prompt {
                let rendered = Self::render_facet_variables(inline, worktree_path, task);
                let prompt = Self::inject_step_outputs(
                    &rendered,
                    step,
                    step_outputs,
                    step_history,
                    workflow_variables,
                );
                return Ok((None, prompt));
            }
            return Err(WorkflowEngineError::InvalidWorkflow(format!(
                "Step '{}' has no facet refs and no inline_prompt.",
                step.name
            )));
        }
        let composed = crate::workflow::facet::compose_facets(step, facets_base_dir)
            .map_err(|e| WorkflowEngineError::InvalidWorkflow(format!("facet composition: {e}")))?;
        let system_prompt = composed
            .system_prompt
            .map(|s| Self::render_facet_variables(&s, worktree_path, task));
        let rendered_user =
            Self::render_facet_variables(&composed.user_message, worktree_path, task);
        let prompt = Self::inject_step_outputs(
            &rendered_user,
            step,
            step_outputs,
            step_history,
            workflow_variables,
        );
        Ok((system_prompt, prompt))
    }

    /// `SessionStartGate` 経由で AgentSession を開始する。
    /// production からは `RealSessionStartGate` を、テストからは記録用テストダブルを渡す。
    /// この関数を経由することで、合成された `system_prompt` がドロップ・空文字置換されずに
    /// バックエンドへ受け渡されることをユニットテストで検証可能にする。
    async fn dispatch_session_start<G: SessionStartGate + ?Sized>(
        gate: &G,
        chat_session_id: &str,
        worktree_path: &str,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
    ) -> Result<(), WorkflowEngineError> {
        gate.start_session(
            chat_session_id,
            worktree_path,
            permission_mode,
            system_prompt,
        )
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
        step: &crate::workflow::schema::Step,
        facets_base_dir: &Path,
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
            facets_base_dir,
            worktree_path,
            task,
            step_outputs,
            step_history,
            workflow_variables,
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
    /// spec-file-path contractの場合、spec_file_pathをworkflow_variablesに設定する。
    async fn apply_contract_variables(
        &self,
        worktree_path: &str,
        output_contract: &Option<String>,
        structured_output: &Option<serde_json::Value>,
    ) {
        let vars = Self::extract_contract_variables(output_contract, structured_output);
        if !vars.is_empty() {
            let mut execs = self.executions.lock().await;
            if let Some(exec) = execs.get_mut(worktree_path) {
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
            if contract == "spec-file-path" {
                if let Some(path) = so.get("spec_file_path").and_then(|v| v.as_str()) {
                    vars.insert("spec_file_path".to_string(), path.to_string());
                }
            }
        }
        vars
    }

    fn mask_sensitive_structured_output(
        app: &tauri::AppHandle,
        contract: &str,
        value: serde_json::Value,
    ) -> serde_json::Value {
        let secrets = Self::collect_configured_secret_values(app);
        Self::mask_sensitive_structured_output_with_secrets(contract, value, &secrets)
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

    fn collect_configured_secret_values(app: &tauri::AppHandle) -> Vec<String> {
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

    /// 現在のステップセッションからoutput_textを取得する。
    /// handle_approval で現在ステップの出力を取得する。
    async fn fetch_current_output(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        worktree_path: &str,
    ) -> Result<Option<String>, WorkflowEngineError> {
        let current_session_id = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            exec.current_session_id.clone()
        };
        Ok(if let Some(ref sid) = current_session_id {
            Self::extract_last_assistant_output(app, session_store, sid).await
        } else {
            None
        })
    }

    /// セッションから最後のAgentメッセージのテキストを抽出する。
    async fn extract_last_assistant_output(
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        session_id: &str,
    ) -> Option<String> {
        let data_dir = crate::session::resolve_data_dir(app).ok()?;
        let session = session_store.get_session(&data_dir, session_id).ok()??;
        Self::extract_last_assistant_text_from_session(&session)
    }

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
        step: &crate::workflow::schema::Step,
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
    async fn interrupt_agent(&self, handles: &Arc<Mutex<AgentProcessMap>>, chat_session_id: &str) {
        use tokio::io::AsyncWriteExt;

        let mut map = handles.lock().await;
        if let Some(proc) = map.get_mut(chat_session_id) {
            if let Err(e) = proc.stdin.write_all(b"{\"type\":\"interrupt\"}\n").await {
                log::warn!(
                    "Failed to write interrupt for session '{}': {e}",
                    chat_session_id
                );
            }
            if let Err(e) = proc.stdin.flush().await {
                log::warn!(
                    "Failed to flush interrupt for session '{}': {e}",
                    chat_session_id
                );
            }
        }
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
                    .steps
                    .iter()
                    .position(|s| s.name == name)
                    .expect("decide_next_step returned unknown step");
                exec.current_step_index = idx;
                exec.state = WorkflowExecutionState::Running;
                *exec.step_execution_counts.entry(name).or_insert(0) += 1;
                exec.updated_at = current_timestamp();

                // resets_cycle_for: 遷移先ステップの設定に従い指定ステップのカウントをリセット
                let resets = exec.workflow.steps[idx].resets_cycle_for.clone();
                if let Some(targets) = resets {
                    for target in &targets {
                        exec.step_execution_counts.remove(target);
                    }
                }

                let step = &exec.workflow.steps[idx];
                if step.is_parallel_block() {
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
        let max_depth = exec.workflow.steps.len();
        if depth >= max_depth {
            exec.state = WorkflowExecutionState::Failed {
                reason: format!("on_exhausted chain depth exceeded (max={})", max_depth),
            };
            exec.updated_at = current_timestamp();
            return Ok(StepOutcome::Persist(exec.to_workflow_state()));
        }

        let idx = exec
            .workflow
            .steps
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
                exec.updated_at = current_timestamp();

                // resets_cycle_for: 遷移先ステップの設定に従い指定ステップのカウントをリセット
                let resets = exec.workflow.steps[idx].resets_cycle_for.clone();
                if let Some(targets) = resets {
                    for target in &targets {
                        exec.step_execution_counts.remove(target);
                    }
                }

                let step = &exec.workflow.steps[idx];
                if step.is_parallel_block() {
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
    async fn execute_outcome(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        chat_session_id: &str,
        outcome: StepOutcome,
    ) -> Result<(), WorkflowEngineError> {
        match outcome {
            StepOutcome::Persist(snapshot) => {
                self.persist_state(app, session_store, chat_session_id, snapshot.clone())
                    .await?;
                self.broadcast_state(app, worktree_path, snapshot.clone());
                if let Some((execution_id, step_name)) =
                    Self::auto_approve_target_for_persisted_snapshot(
                        &snapshot,
                        Self::workflow_approval_auto_approve_enabled(app),
                    )
                {
                    return Box::pin(self.handle_approval(
                        app,
                        session_store,
                        handles,
                        worktree_path,
                        ApprovalDecision::Approve,
                        Some(&execution_id),
                        Some(&step_name),
                    ))
                    .await;
                }
                if matches!(
                    snapshot.state,
                    WorkflowExecutionState::Completed
                        | WorkflowExecutionState::Failed { .. }
                        | WorkflowExecutionState::Aborted
                ) {
                    // Completedの場合のみ最後のステップの完了ログを書く
                    // (TransitionAndStart経由の場合は既にexecute_outcome内で記録済み)
                    if matches!(snapshot.state, WorkflowExecutionState::Completed) {
                        self.write_last_step_completed_log(app, &snapshot);
                    }
                    self.write_terminal_log(app, &snapshot);
                    self.cleanup_session_workflow_refs(worktree_path).await;
                }
                Ok(())
            }
            StepOutcome::TransitionAndStart(snapshot) => {
                self.persist_state(app, session_store, chat_session_id, snapshot.clone())
                    .await?;
                self.broadcast_state(app, worktree_path, snapshot.clone());

                // 直前のステップ完了ログ + 新ステップ開始ログ
                self.write_last_step_completed_log(app, &snapshot);
                let exec_count = snapshot
                    .step_execution_counts
                    .get(&snapshot.current_step_name)
                    .copied()
                    .unwrap_or(1);
                self.write_log(
                    app,
                    WorkflowLogEvent::StepStarted {
                        execution_id: snapshot.execution_id.clone(),
                        workflow_name: snapshot.workflow_name.clone(),
                        step_name: snapshot.current_step_name.clone(),
                        execution_count: exec_count,
                        timestamp: snapshot.updated_at,
                    },
                );

                // AgentSession起動。失敗時はFailed状態に遷移。
                if let Err(e) = self
                    .start_step_session(app, handles, session_store, worktree_path)
                    .await
                {
                    {
                        let mut execs = self.executions.lock().await;
                        if let Some(exec) = execs.get_mut(worktree_path) {
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
                self.persist_state(app, session_store, chat_session_id, snapshot.clone())
                    .await?;
                self.broadcast_state(app, worktree_path, snapshot.clone());

                // 直前ステップの完了ログ
                self.write_last_step_completed_log(app, &snapshot);

                // reduce実行
                let (collect_config_clone, reduce_result, step_rules) = {
                    let execs = self.executions.lock().await;
                    let exec = execs.get(worktree_path).ok_or_else(|| {
                        WorkflowEngineError::ExecutionNotFound(worktree_path.to_string())
                    })?;
                    let step = &exec.workflow.steps[exec.current_step_index];
                    let collect = step
                        .collect
                        .clone()
                        .expect("ReduceAndTransition requires collect config");
                    let result = Self::apply_reduce(&collect, &exec.step_outputs);
                    (collect, result, step.rules.clone())
                };

                // collect step自体のStepHistoryEntryを記録 + 遷移判定
                let (next_outcome, log_step_name, log_exec_id, log_wf_name) = {
                    let mut execs = self.executions.lock().await;
                    let exec = execs.get_mut(worktree_path).ok_or_else(|| {
                        WorkflowEngineError::ExecutionNotFound(worktree_path.to_string())
                    })?;

                    let entry = exec.make_step_history_entry(
                        reduce_result.result.clone(),
                        reduce_result.structured_output.clone(),
                        None,
                    );
                    exec.step_history.push(entry);

                    let step_name = exec.workflow.steps[exec.current_step_index].name.clone();
                    let exec_id = exec.id.clone();
                    let wf_name = exec.workflow.name.clone();

                    log::info!(
                        "OutputCollected: step='{}', strategy={:?}, result={:?}, from={:?}",
                        step_name,
                        collect_config_clone.reduce,
                        reduce_result.result,
                        collect_config_clone.from,
                    );

                    // reduce resultに基づく遷移判定
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
                    (outcome, step_name, exec_id, wf_name)
                };

                // OutputCollected NDJSONログ永続化（ロック外）
                let collected_entries: Vec<crate::workflow::log::CollectedOutputEntry> =
                    collect_config_clone
                        .from
                        .iter()
                        .map(|name| {
                            let output = snapshot.step_outputs.get(name);
                            crate::workflow::log::CollectedOutputEntry {
                                step_name: name.clone(),
                                result: output.and_then(|o| o.result.clone()),
                                structured_output: output.and_then(|o| o.structured_output.clone()),
                            }
                        })
                        .collect();
                self.write_log(
                    app,
                    WorkflowLogEvent::OutputCollected {
                        execution_id: log_exec_id,
                        workflow_name: log_wf_name,
                        step_name: log_step_name,
                        step_outputs: collected_entries,
                        reduce_strategy: format!("{:?}", collect_config_clone.reduce),
                        reduce_result: reduce_result.result.clone(),
                        reduce_structured_output: reduce_result.structured_output.clone(),
                        timestamp: crate::session::now_timestamp(),
                    },
                );

                // 再帰的にexecute_outcomeを呼ぶ（次ステップがcollectの可能性）
                Box::pin(self.execute_outcome(
                    app,
                    session_store,
                    handles,
                    worktree_path,
                    chat_session_id,
                    next_outcome,
                ))
                .await
            }
            StepOutcome::StartParallel(snapshot) => {
                self.persist_state(app, session_store, chat_session_id, snapshot.clone())
                    .await?;
                self.broadcast_state(app, worktree_path, snapshot.clone());

                // 直前ステップの完了ログ
                self.write_last_step_completed_log(app, &snapshot);

                // 並列子ステップの起動（失敗時はFailed状態に遷移）
                if let Err(e) = self
                    .start_parallel_children(app, session_store, handles, worktree_path)
                    .await
                {
                    let _ = self
                        .set_execution_state(
                            app,
                            session_store,
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
    async fn start_parallel_children(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
    ) -> Result<(), WorkflowEngineError> {
        // ロック内: 子ステップ定義取得 + ParallelRunState構築
        let (
            parallel_steps,
            parent_step_name,
            aggregate,
            execution_id,
            workflow_name,
            chat_session_id,
            task_clone,
        ) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;

            let step = &exec.workflow.steps[exec.current_step_index];
            let parallel = step
                .parallel
                .clone()
                .expect("StartParallel requires parallel field");
            let agg = step.aggregate.clone();
            let parent_name = step.name.clone();
            let exec_id = exec.id.clone();
            let wf_name = exec.workflow.name.clone();
            let chat_sid = exec.chat_session_id.clone();
            let task = exec.task.clone();

            (parallel, parent_name, agg, exec_id, wf_name, chat_sid, task)
        };

        // ParallelStarted ログ
        let child_step_names: Vec<String> =
            parallel_steps.iter().map(|ps| ps.name.clone()).collect();
        self.write_log(
            app,
            WorkflowLogEvent::ParallelStarted {
                execution_id: execution_id.clone(),
                workflow_name: workflow_name.clone(),
                parent_step_name: parent_step_name.clone(),
                child_step_names: child_step_names.clone(),
                timestamp: current_timestamp(),
            },
        );

        // 各子ステップのセッション生成 + AgentSession起動
        let data_dir = crate::session::resolve_data_dir(app)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;

        let parent_session = session_store
            .get_session(&data_dir, &chat_session_id)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("get_session: {e}")))?
            .ok_or_else(|| WorkflowEngineError::SessionNotFound(chat_session_id.clone()))?;
        let parent_permission_mode = parent_session.permission_mode;
        let parent_backend_id = parent_session.backend_id.clone();
        let parent_selected_model = parent_session.selected_model.clone();

        // step_outputsとworkflow_variablesのスナップショットをロック外で取得
        let (step_outputs_snapshot, wf_variables_snapshot) = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            (exec.step_outputs.clone(), exec.workflow_variables.clone())
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
            // 子ステップ設定の解決 → セッション生成
            let step_session = self
                .create_step_session_with_settings(
                    app,
                    session_store,
                    &data_dir,
                    worktree_path,
                    ps.model.clone(),
                    ps.permission.clone(),
                    parent_backend_id.clone(),
                    parent_selected_model.clone(),
                    parent_permission_mode.clone(),
                )
                .await?;
            let child_permission_mode = step_session.permission_mode.clone();
            let step_session_id = step_session.id.clone();

            // session_workflow_refs に ParallelChild として登録
            {
                let mut map = self.session_workflow_refs.lock().await;
                map.insert(
                    step_session_id.clone(),
                    SessionWorkflowRef {
                        worktree_path: worktree_path.to_string(),
                        kind: SessionRefKind::ParallelChild {
                            parent_step_name: parent_step_name.clone(),
                        },
                    },
                );
            }

            // ファセットからプロンプト構築
            let base_dir = storage::facets_base_dir();
            let (system_prompt, user_message) = Self::build_parallel_step_prompt(
                ps,
                &base_dir,
                worktree_path,
                task_clone.as_deref(),
                &step_outputs_snapshot,
                ps.pass_previous_response.unwrap_or(false),
                ps.pass_output_from.as_deref(),
                &wf_variables_snapshot,
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
            let exec = execs
                .get_mut(worktree_path)
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
                    contract_retry_count: 0,
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
        // ロック解放後にI/O操作を実行
        self.persist_state(app, session_store, &chat_session_id, snapshot.clone())
            .await?;
        self.broadcast_state(app, worktree_path, snapshot);

        // Phase 3a: 全子セッション作成（AgentSessionプロセス起動）
        // Note: AppHandleが!Sendのためtokio::spawnによる真の並列化は不可能。
        // セッション作成とターン開始を分離し、全セッション準備完了後に
        // 全ターンを開始することで「ほぼ同時起動」を実現する。
        let mut created_session_ids: Vec<String> = Vec::new();
        for cs in &child_setups {
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
            created_session_ids.push(cs.session_id.clone());
        }

        // Phase 3b: 全子ターン開始（ここが実際のAgent作業トリガー）
        for (i, cs) in child_setups.iter().enumerate() {
            if let Err(e) = crate::agent_sdk::start_agent_turn_internal(
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

            // ParallelStepStarted ログ
            self.write_log(
                app,
                WorkflowLogEvent::ParallelStepStarted {
                    execution_id: execution_id.clone(),
                    workflow_name: workflow_name.clone(),
                    parent_step_name: parent_step_name.clone(),
                    child_step_name: cs.step_name.clone(),
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
    fn build_parallel_step_prompt(
        ps: &ParallelStep,
        facets_base_dir: &Path,
        worktree_path: &str,
        task: Option<&str>,
        step_outputs: &HashMap<String, StepOutput>,
        pass_previous_response: bool,
        pass_output_from: Option<&[String]>,
        workflow_variables: &HashMap<String, String>,
    ) -> Result<(Option<String>, String), WorkflowEngineError> {
        let composed = crate::workflow::facet::compose_facets_from_refs(
            ps.policy.as_deref(),
            ps.knowledge.as_deref(),
            ps.instruction.as_deref(),
            ps.output_contract.as_deref(),
            facets_base_dir,
        )
        .map_err(|e| WorkflowEngineError::InvalidWorkflow(format!("Facet error: {e}")))?;

        let system_prompt = composed
            .system_prompt
            .map(|s| Self::render_facet_variables(&s, worktree_path, task));
        let mut user_message =
            Self::render_facet_variables(&composed.user_message, worktree_path, task);

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

        Ok((system_prompt, user_message))
    }

    /// ワークフロー状態をChatSessionに永続化する。
    /// スナップショットは呼び出し元がロック内で確定したものを受け取る。
    async fn persist_state(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        chat_session_id: &str,
        workflow_state: WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
        let data_dir = crate::session::resolve_data_dir(app)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;
        let mut session = session_store
            .get_session(&data_dir, chat_session_id)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("get_session: {e}")))?
            .ok_or_else(|| WorkflowEngineError::SessionNotFound(chat_session_id.to_string()))?;
        session.workflow_state = Some(workflow_state);
        session.updated_at = crate::session::now_timestamp();
        session_store
            .save_session(&data_dir, &session)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("save_session: {e}")))?;

        Ok(())
    }

    /// ワークフロー状態をブロードキャストする。
    /// スナップショットは呼び出し元がロック内で確定したものを受け取る。
    /// worktree_pathベースでイベントを発行するため、同一worktreeの全セッションが受信可能。
    fn broadcast_state(
        &self,
        app: &tauri::AppHandle,
        worktree_path: &str,
        workflow_state: WorkflowState,
    ) {
        let center: Option<tauri::State<'_, Arc<AgentStatusCenter>>> =
            app.try_state::<Arc<AgentStatusCenter>>();
        if let Some(center) = center {
            // worktree_pathでイベントを emit
            center.emit_workflow_state_changed(worktree_path, &workflow_state);

            // 同一worktreeの全セッションのworkflowフィールドを更新
            for status in center.list_sessions() {
                if status.worktree_path == worktree_path {
                    let mut updated = status;
                    updated.workflow_step = Some(workflow_state.current_step_name.clone());
                    updated.workflow_execution_state =
                        Some(workflow_state.state.as_str().to_string());
                    center.update_session(updated);
                }
            }
        }
    }

    /// 終了状態（Completed/Failed/Aborted）のログを書き込む。
    /// StepCompletedログは呼び出し元で書き込み済みのため、ここでは書かない。
    fn write_terminal_log(&self, app: &tauri::AppHandle, snapshot: &WorkflowState) {
        // ワークフロー終了ログ
        match &snapshot.state {
            WorkflowExecutionState::Completed => {
                self.write_log(
                    app,
                    WorkflowLogEvent::WorkflowCompleted {
                        execution_id: snapshot.execution_id.clone(),
                        workflow_name: snapshot.workflow_name.clone(),
                        total_token_usage: snapshot.total_token_usage.clone(),
                        timestamp: snapshot.updated_at,
                    },
                );
            }
            WorkflowExecutionState::Failed { reason } => {
                // 失敗ステップのログ
                self.write_log(
                    app,
                    WorkflowLogEvent::StepFailed {
                        execution_id: snapshot.execution_id.clone(),
                        workflow_name: snapshot.workflow_name.clone(),
                        step_name: snapshot.current_step_name.clone(),
                        reason: reason.clone(),
                        timestamp: snapshot.updated_at,
                    },
                );
                self.write_log(
                    app,
                    WorkflowLogEvent::WorkflowFailed {
                        execution_id: snapshot.execution_id.clone(),
                        workflow_name: snapshot.workflow_name.clone(),
                        reason: reason.clone(),
                        timestamp: snapshot.updated_at,
                    },
                );
            }
            WorkflowExecutionState::Aborted => {
                self.write_log(
                    app,
                    WorkflowLogEvent::WorkflowAborted {
                        execution_id: snapshot.execution_id.clone(),
                        workflow_name: snapshot.workflow_name.clone(),
                        timestamp: snapshot.updated_at,
                    },
                );
            }
            // Running/WaitingApproval は終了状態ではないのでログ不要
            _ => {}
        }
    }

    /// 最後のステップのStepCompletedログを書き込む。
    fn write_last_step_completed_log(&self, app: &tauri::AppHandle, snapshot: &WorkflowState) {
        if let Some(last_entry) = snapshot.step_history.last() {
            self.write_log(
                app,
                WorkflowLogEvent::StepCompleted {
                    execution_id: snapshot.execution_id.clone(),
                    workflow_name: snapshot.workflow_name.clone(),
                    step_name: last_entry.step_name.clone(),
                    result: last_entry.result.clone(),
                    session_id: last_entry.session_id.clone(),
                    token_usage: last_entry.token_usage.clone(),
                    structured_output: last_entry.structured_output.clone(),
                    run_index: Some(last_entry.run_index),
                    timestamp: last_entry.completed_at,
                },
            );
        }
    }

    /// NDJSONログにイベントを書き込む。失敗してもワークフロー実行には影響しない。
    fn write_log(&self, app: &tauri::AppHandle, event: WorkflowLogEvent) {
        if let Ok(data_dir) = crate::session::resolve_data_dir(app) {
            let log = WorkflowEventLog::new(&data_dir);
            if let Err(e) = log.append(&event) {
                log::warn!("Failed to write workflow log: {e}");
            }
        }
    }

    fn workflow_approval_auto_approve_enabled(app: &tauri::AppHandle) -> bool {
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
        output_text: Option<String>,
    ) -> Result<StepOutcome, WorkflowEngineError> {
        {
            let execs = self.executions.lock().await;
            let exec = execs.get(worktree_path).ok_or_else(|| {
                WorkflowEngineError::UnauthorizedWorktree(worktree_path.to_string())
            })?;
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

        let (output_contract, workflow_for_contract, current_step_index_for_contract) = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            (
                exec.workflow.steps[exec.current_step_index]
                    .output_contract
                    .clone(),
                exec.workflow.clone(),
                exec.current_step_index,
            )
        };

        let result_tag = match &decision {
            ApprovalDecision::Approve => "approve",
            ApprovalDecision::Reject { .. } => "reject",
            ApprovalDecision::Abort => "abort",
        };
        let (structured_output, contract_result) = if matches!(decision, ApprovalDecision::Approve)
        {
            match output_contract.as_deref() {
                Some(contract) => {
                    let extraction = match output_text.as_deref() {
                        Some(text) => extract_workflow_output(text),
                        None => ExtractionResult::NoBlock,
                    };
                    match Self::validate_approval_contract_extraction(
                        contract,
                        extraction,
                        &workflow_for_contract,
                        current_step_index_for_contract,
                    )? {
                        ContractCheckResult::Valid {
                            structured_output,
                            result,
                        } => (Some(structured_output), result),
                        ContractCheckResult::NoContract => (None, None),
                        ContractCheckResult::RetrySent | ContractCheckResult::Failed => {
                            unreachable!()
                        }
                    }
                }
                None => (None, None),
            }
        } else if let ApprovalDecision::Reject { ref comment } = decision {
            (Some(Self::reject_structured_output(comment, &[])), None)
        } else {
            (None, None)
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
            .get_mut(worktree_path)
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
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
        output_text: String,
    ) -> Result<Option<StepOutcome>, WorkflowEngineError> {
        if let Some((execution_id, step_name)) =
            Self::auto_approve_target_for_persisted_snapshot(snapshot, true)
        {
            self.handle_approval_with_output_for_test(
                worktree_path,
                ApprovalDecision::Approve,
                Some(&execution_id),
                Some(&step_name),
                Some(output_text),
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
        chat_session_id: &str,
        current_session_id: &str,
        state: WorkflowExecutionState,
    ) -> WorkflowState {
        let workflow = Workflow {
            name: "test-approval-workflow".to_string(),
            description: "test".to_string(),
            builtin: false,
            steps: vec![crate::workflow::schema::Step {
                name: "implementation_fix_policy".to_string(),
                mode: Some(StepMode::Approval),
                policy: None,
                knowledge: None,
                instruction: Some("Review fix policy".to_string()),
                output_contract: Some("approved-fix-policy".to_string()),
                rules: vec![],
                cycle_guard: None,
                pass_previous_response: None,
                pass_output_from: None,
                inline_prompt: None,
                collect: None,
                parallel: None,
                aggregate: None,
                resets_cycle_for: None,
                model: None,
                permission: None,
            }],
        };
        let exec = WorkflowExecution {
            id: "exec-approval-chat".to_string(),
            workflow,
            state,
            current_step_index: 0,
            step_execution_counts: HashMap::from([("implementation_fix_policy".to_string(), 1)]),
            step_history: Vec::new(),
            chat_session_id: chat_session_id.to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some(current_session_id.to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
        };
        let snapshot = exec.to_workflow_state();
        self.executions
            .lock()
            .await
            .insert(worktree_path.to_string(), exec);
        self.session_workflow_refs.lock().await.insert(
            current_session_id.to_string(),
            SessionWorkflowRef {
                worktree_path: worktree_path.to_string(),
                kind: SessionRefKind::SequentialStep,
            },
        );
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::MessagePart;
    use crate::workflow::schema::{
        AggregateConfig, CollectConfig, CycleGuard, ReduceStrategy, Step, StepMode, TransitionRule,
        Workflow,
    };

    fn approved_fix_policy_output(policy: &str, review_step: &str) -> String {
        format!(
            r#"<workflow_output type="approved-fix-policy">{{"policy":"{policy}","review_step":"{review_step}"}}</workflow_output>"#
        )
    }

    fn make_approved_fix_policy_workflow() -> Workflow {
        Workflow {
            name: "approved-fix-policy-test".to_string(),
            description: "test".to_string(),
            builtin: false,
            steps: vec![
                Step {
                    name: "code_review_parallel".to_string(),
                    mode: None,
                    policy: None,
                    knowledge: None,
                    instruction: None,
                    output_contract: None,
                    rules: vec![],
                    cycle_guard: None,
                    pass_previous_response: None,
                    pass_output_from: None,
                    inline_prompt: None,
                    collect: None,
                    parallel: Some(vec![]),
                    aggregate: Some(AggregateConfig {
                        all_match: Some("LGTM".to_string()),
                        any_match: None,
                        then: "done".to_string(),
                        r#else: "implementation_fix_policy".to_string(),
                    }),
                    resets_cycle_for: None,
                    model: None,
                    permission: None,
                },
                Step {
                    name: "implementation_fix_policy".to_string(),
                    mode: Some(StepMode::Approval),
                    policy: None,
                    knowledge: None,
                    instruction: Some("Review fix policy".to_string()),
                    output_contract: Some("approved-fix-policy".to_string()),
                    rules: vec![],
                    cycle_guard: None,
                    pass_previous_response: None,
                    pass_output_from: Some(vec!["code_review_parallel".to_string()]),
                    inline_prompt: None,
                    collect: None,
                    parallel: None,
                    aggregate: None,
                    resets_cycle_for: None,
                    model: None,
                    permission: None,
                },
            ],
        }
    }

    fn make_spec_driven_plan_fix_policy_exec(
        execution_id: &str,
        current_session_id: &str,
    ) -> WorkflowExecution {
        make_spec_driven_fix_policy_exec(execution_id, current_session_id, "plan_fix_policy")
    }

    fn make_spec_driven_fix_policy_exec(
        execution_id: &str,
        current_session_id: &str,
        step_name: &str,
    ) -> WorkflowExecution {
        let workflow = crate::workflow::builtin::get_builtin_workflow("spec-driven-development")
            .expect("builtin workflow exists");
        let current_step_index = workflow
            .steps
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
            chat_session_id: "parent-session".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some(current_session_id.to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
        mode: StepMode,
        instruction: &str,
        rules: Vec<TransitionRule>,
        cycle_guard: Option<CycleGuard>,
    ) -> Step {
        Step {
            name: name.to_string(),
            mode: Some(mode),
            policy: None,
            knowledge: None,
            instruction: Some(instruction.to_string()),
            output_contract: None,
            rules,
            cycle_guard,
            pass_previous_response: None,
            pass_output_from: None,
            inline_prompt: None,
            collect: None,
            parallel: None,
            aggregate: None,
            resets_cycle_for: None,
            model: None,
            permission: None,
        }
    }

    fn make_test_workflow() -> Workflow {
        Workflow {
            name: "test-workflow".to_string(),
            description: "Test workflow".to_string(),
            builtin: false,
            steps: vec![
                make_test_step("plan", StepMode::Interactive, "Plan the work", vec![], None),
                make_test_step(
                    "implement",
                    StepMode::Auto,
                    "Implement the plan",
                    vec![],
                    None,
                ),
                make_test_step(
                    "review",
                    StepMode::Auto,
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
                    StepMode::Approval,
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

            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1001.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1001.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            }],
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1001.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1001.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1002.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
        let step = &workflow.steps[0]; // plan (no cycle_guard)
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
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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

    #[test]
    fn turn_complete_action_interactive_fails_for_validation_only_legacy_definition() {
        let exec = make_exec(0); // plan (interactive)
        assert_eq!(
            exec.decide_turn_complete_action(0),
            TurnCompleteAction::SessionError {
                step_name: "plan".to_string(),
                exit_code: 0,
            }
        );
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
    fn decide_approval_action_abort() {
        let mut exec = make_exec(3); // report (approval)
        exec.state = WorkflowExecutionState::WaitingApproval;
        assert_eq!(
            exec.decide_approval_action(&ApprovalDecision::Abort)
                .unwrap(),
            ApprovalAction::Abort
        );
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
            name: "empty".to_string(),
            description: String::new(),
            builtin: false,
            steps: vec![],
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
        let mut step = make_test_step("step_b", StepMode::Auto, "Do B", vec![], None);
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
        }];
        let wv = HashMap::new();
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &outputs, &history, &wv);
        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("output from A"));
    }

    #[test]
    fn inject_step_outputs_no_pass_previous_response() {
        let step = make_test_step("step_b", StepMode::Auto, "Do B", vec![], None);
        let outputs = HashMap::new();
        let history = vec![];
        let wv = HashMap::new();
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &outputs, &history, &wv);
        assert_eq!(result, "Do B");
    }

    #[test]
    fn inject_step_outputs_pass_output_from_single() {
        let mut step = make_test_step("step_c", StepMode::Auto, "Do C", vec![], None);
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
        let mut step = make_test_step("fix", StepMode::Auto, "Fix issues", vec![], None);
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
        let mut step = make_test_step("step_c", StepMode::Auto, "Do C", vec![], None);
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
        let mut step = make_test_step("step_b", StepMode::Auto, "Do B", vec![], None);
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
        }];
        let wv = HashMap::new();
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &outputs, &history, &wv);
        assert_eq!(result, "Do B");
    }

    #[test]
    fn inject_step_outputs_missing_step_shows_not_completed() {
        let mut step = make_test_step("step_b", StepMode::Auto, "Do B", vec![], None);
        step.pass_output_from = Some(vec!["step_a".to_string()]);

        let outputs = HashMap::new(); // step_a not present
        let result =
            WorkflowEngine::inject_step_outputs("Do B", &step, &outputs, &[], &HashMap::new());
        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("(not yet completed)"));
    }

    #[test]
    fn inject_step_outputs_workflow_variables_injected() {
        let step = make_test_step("step_b", StepMode::Auto, "Do B", vec![], None);
        let mut wv = HashMap::new();
        wv.insert(
            "spec_file_path".to_string(),
            "docs/spec/issues-909.md".to_string(),
        );
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &HashMap::new(), &[], &wv);
        assert!(result.contains("<workflow_variables>"));
        assert!(result.contains("spec_file_path"));
        assert!(result.contains("docs/spec/issues-909.md"));
    }

    #[test]
    fn inject_step_outputs_empty_workflow_variables_not_injected() {
        let step = make_test_step("step_b", StepMode::Auto, "Do B", vec![], None);
        let wv = HashMap::new();
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &HashMap::new(), &[], &wv);
        assert!(!result.contains("<workflow_variables>"));
    }

    #[test]
    fn inject_step_outputs_parallel_parent_aggregated_children() {
        // 並列ブロック親名で集約された子出力がpass_output_fromで参照できること
        let mut step = make_test_step("plan_fix", StepMode::Auto, "Fix plan", vec![], None);
        step.pass_output_from = Some(vec![
            "plan_review_parallel".to_string(),
            "plan_draft".to_string(),
        ]);

        let mut outputs = HashMap::new();
        // 並列ブロック親の集約StepOutput（子出力をまとめたJSONオブジェクト）
        outputs.insert(
            "plan_review_parallel".to_string(),
            StepOutput {
                step_name: "plan_review_parallel".to_string(),
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
        assert!(result.contains("<step_output name=\"plan_review_parallel\">"));
        assert!(result.contains("NEEDS_FIX"));
        assert!(result.contains("Missing error handling"));
        assert!(result.contains("<step_output name=\"plan_draft\">"));
        assert!(result.contains("Draft spec content"));
    }

    #[test]
    fn inject_step_outputs_parallel_parent_via_pass_previous_response() {
        // pass_previous_response: trueで並列ブロック親の集約出力が参照できること
        let mut step = make_test_step("plan_fix", StepMode::Auto, "Fix plan", vec![], None);
        step.pass_previous_response = Some(true);

        let mut outputs = HashMap::new();
        outputs.insert(
            "plan_review_parallel".to_string(),
            StepOutput {
                step_name: "plan_review_parallel".to_string(),
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
            step_name: "plan_review_parallel".to_string(),
            completed_at: 1000.0,
            result: Some("else".to_string()),
            session_id: None,
            token_usage: None,
            structured_output: None,
            run_index: 1,
            child_outputs: None,
        }];

        let result = WorkflowEngine::inject_step_outputs(
            "Fix plan",
            &step,
            &outputs,
            &history,
            &HashMap::new(),
        );
        assert!(result.contains("<step_output name=\"plan_review_parallel\">"));
        assert!(result.contains("NEEDS_FIX"));
        assert!(result.contains("SQL injection risk"));
    }

    // ---- extract_contract_variables ----

    #[test]
    fn extract_contract_variables_spec_file_path() {
        let contract = Some("spec-file-path".to_string());
        let so = Some(serde_json::json!({
            "spec_file_path": "docs/spec/issues-909.md"
        }));
        let vars = WorkflowEngine::extract_contract_variables(&contract, &so);
        assert_eq!(
            vars.get("spec_file_path").unwrap(),
            "docs/spec/issues-909.md"
        );
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
            "review_step": "plan_review_parallel"
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
                "review_step": "code_review_parallel"
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
        let mut step = make_test_step("fix", StepMode::Auto, "Fix", vec![], None);
        step.pass_output_from = Some(vec!["implementation_fix_policy".to_string()]);

        let sanitized = serde_json::json!({
            "policy": "Use password=[REDACTED] only in examples.",
            "review_step": "code_review_parallel"
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
            "review_step": "code_review_parallel"
        });
        WorkflowEngine::mask_json_strings(&mut structured, &["MY_TOKEN_VALUE_123456".to_string()]);
        let raw = serde_json::to_string(&structured).unwrap();
        assert!(!raw.contains("secret123"));
        assert!(!raw.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
        assert!(!raw.contains("PRIVATE KEY-----abc"));
        assert!(!raw.contains("MY_TOKEN_VALUE_123456"));

        let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        exec.workflow.steps[0].output_contract = Some("approved-fix-policy".to_string());
        exec.workflow.steps.push(Step {
            name: "fix".to_string(),
            mode: Some(StepMode::Auto),
            policy: None,
            knowledge: None,
            instruction: None,
            output_contract: None,
            rules: vec![],
            cycle_guard: None,
            pass_previous_response: Some(true),
            pass_output_from: None,
            inline_prompt: Some("Fix".to_string()),
            collect: None,
            parallel: None,
            aggregate: None,
            resets_cycle_for: None,
            model: None,
            permission: None,
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
            &exec.workflow.steps[exec.current_step_index],
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
        let mut exec = make_spec_driven_plan_fix_policy_exec("exec-plan-log", "policy-session");
        let secret_env_value = "MY_TOKEN_VALUE_123456".to_string();
        let mut structured = serde_json::json!({
            "policy": "Use password=secret123 with ghp_abcdefghijklmnopqrstuvwxyz1234567890 -----BEGIN PRIVATE KEY-----abc-----END PRIVATE KEY----- MY_TOKEN_VALUE_123456",
            "review_step": "plan_review_parallel"
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
            .find(|entry| entry.step_name == "plan_fix_policy")
            .unwrap();
        let log = WorkflowEventLog::new(tmp.path());
        log.append(&WorkflowLogEvent::WorkflowStarted {
            execution_id: exec.id.clone(),
            workflow_name: exec.workflow.name.clone(),
            workflow_file_stem: "spec-driven-development".to_string(),
            worktree_path: "/repo".to_string(),
            workflow_definition: Some(exec.workflow.clone()),
            timestamp: 1000.0,
        })
        .unwrap();
        log.append(&WorkflowLogEvent::StepCompleted {
            execution_id: exec.id.clone(),
            workflow_name: exec.workflow.name.clone(),
            step_name: entry.step_name.clone(),
            result: entry.result.clone(),
            session_id: entry.session_id.clone(),
            token_usage: entry.token_usage.clone(),
            structured_output: entry.structured_output.clone(),
            run_index: Some(entry.run_index),
            timestamp: entry.completed_at,
        })
        .unwrap();

        let raw_ndjson =
            std::fs::read_to_string(tmp.path().join("workflow_logs/exec-plan-log.ndjson")).unwrap();
        assert!(raw_ndjson.contains("[REDACTED]"));
        assert!(!raw_ndjson.contains("secret123"));
        assert!(!raw_ndjson.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
        assert!(!raw_ndjson.contains("PRIVATE KEY-----abc"));
        assert!(!raw_ndjson.contains("MY_TOKEN_VALUE_123456"));

        let events = log.read_log("exec-plan-log").unwrap();
        let serialized = serde_json::to_string(&events).unwrap();
        assert!(serialized.contains("[REDACTED]"));
        assert!(!serialized.contains("secret123"));
        assert!(!serialized.contains("ghp_abcdefghijklmnopqrstuvwxyz1234567890"));
        assert!(!serialized.contains("PRIVATE KEY-----abc"));
        assert!(!serialized.contains("MY_TOKEN_VALUE_123456"));
        let completed = events
            .iter()
            .find(|event| matches!(event, WorkflowLogEvent::StepCompleted { .. }))
            .unwrap();
        match completed {
            WorkflowLogEvent::StepCompleted {
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
        let so = Some(serde_json::json!({"spec_file_path": "docs/spec.md"}));
        let vars = WorkflowEngine::extract_contract_variables(&None, &so);
        assert!(vars.is_empty());
    }

    #[test]
    fn extract_contract_variables_no_output_returns_empty() {
        let contract = Some("spec-file-path".to_string());
        let vars = WorkflowEngine::extract_contract_variables(&contract, &None);
        assert!(vars.is_empty());
    }

    #[test]
    fn extract_contract_variables_missing_field_returns_empty() {
        let contract = Some("spec-file-path".to_string());
        let so = Some(serde_json::json!({"other_field": "value"}));
        let vars = WorkflowEngine::extract_contract_variables(&contract, &so);
        assert!(vars.is_empty());
    }

    // ---- contract retry判定 ----

    #[test]
    fn contract_retry_within_limit() {
        let retry_count: u32 = 0;
        assert!(retry_count < MAX_CONTRACT_RETRIES);
        let retry_count: u32 = 1;
        assert!(retry_count < MAX_CONTRACT_RETRIES);
    }

    #[test]
    fn contract_retry_at_limit_should_fail() {
        let retry_count: u32 = MAX_CONTRACT_RETRIES;
        assert!(retry_count >= MAX_CONTRACT_RETRIES);
    }

    #[test]
    fn contract_retry_over_limit_should_fail() {
        let retry_count: u32 = MAX_CONTRACT_RETRIES + 1;
        assert!(retry_count >= MAX_CONTRACT_RETRIES);
    }

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
        let output_contracts = base.join("output_contracts");
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::create_dir_all(&output_contracts).unwrap();
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
        std::fs::write(output_contracts.join("plan-doc.md"), "Output as markdown.").unwrap();

        let mut step = make_test_step("build", StepMode::Auto, "unused", vec![], None);
        step.instruction = Some("impl".to_string());
        step.policy = Some("coding".to_string());
        step.output_contract = Some("plan-doc".to_string());
        step.pass_previous_response = Some(true);

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
        }];

        let (sys, prompt) = WorkflowEngine::build_step_prompt(
            &step,
            base,
            "/home/user/my-app",
            Some("Fix bug"),
            &outputs,
            &history,
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
        // inject_step_outputs: pass_previous_response includes plan output
        assert!(prompt.contains("<step_output name=\"plan\">"));
        assert!(prompt.contains("Plan output text"));
    }

    #[test]
    fn build_step_prompt_no_facet_refs_returns_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let step = Step {
            name: "empty".to_string(),
            mode: Some(StepMode::Auto),
            policy: None,
            knowledge: None,
            instruction: None,
            output_contract: None,
            rules: vec![],
            cycle_guard: None,
            pass_previous_response: None,
            pass_output_from: None,
            inline_prompt: None,
            collect: None,
            parallel: None,
            aggregate: None,
            resets_cycle_for: None,
            model: None,
            permission: None,
        };
        let result = WorkflowEngine::build_step_prompt(
            &step,
            tmp.path(),
            "/repo",
            None,
            &HashMap::new(),
            &[],
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

        let mut step = make_test_step("review", StepMode::Auto, "unused", vec![], None);
        step.policy = Some("review".to_string());
        step.instruction = None;

        let (sys, prompt) = WorkflowEngine::build_step_prompt(
            &step,
            tmp.path(),
            "/repo",
            None,
            &HashMap::new(),
            &[],
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
        let output_contracts = tmp.path().join("output_contracts");
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::create_dir_all(&output_contracts).unwrap();
        std::fs::write(policies.join("coding.md"), "POLICY_BODY").unwrap();
        std::fs::write(output_contracts.join("plan-doc.md"), "CONTRACT_BODY").unwrap();

        let mut step = make_test_step("s", StepMode::Auto, "unused", vec![], None);
        step.policy = Some("coding".to_string());
        step.output_contract = Some("plan-doc".to_string());
        step.instruction = None;

        let (sys, _prompt) = WorkflowEngine::build_step_prompt(
            &step,
            tmp.path(),
            "/repo",
            None,
            &HashMap::new(),
            &[],
            &HashMap::new(),
        )
        .unwrap();

        // 合成された system_prompt は Some(...) として渡される（None や空文字に置換されない）
        let sys = sys.expect("system_prompt must be passed through, not dropped");
        assert!(!sys.is_empty(), "system_prompt must not be empty string");
        assert!(sys.contains("POLICY_BODY"));
        assert!(sys.contains("CONTRACT_BODY"));
    }

    // ---- dispatch_session_start (SessionStartGate 経由のテストダブル検証) ----

    /// テスト用の `SessionStartGate` 実装。受け取った引数を共有 Vec に記録する。
    struct RecordingSessionStartGate {
        records: Arc<std::sync::Mutex<Vec<RecordedSessionStart>>>,
    }

    #[derive(Clone, Debug)]
    struct RecordedSessionStart {
        chat_session_id: String,
        worktree_path: String,
        permission_mode: Option<String>,
        system_prompt: Option<String>,
    }

    #[async_trait::async_trait]
    impl SessionStartGate for RecordingSessionStartGate {
        async fn start_session(
            &self,
            chat_session_id: &str,
            worktree_path: &str,
            permission_mode: Option<String>,
            system_prompt: Option<String>,
        ) -> Result<(), String> {
            self.records.lock().unwrap().push(RecordedSessionStart {
                chat_session_id: chat_session_id.to_string(),
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
        let output_contracts = base.join("output_contracts");
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::create_dir_all(&output_contracts).unwrap();
        std::fs::write(policies.join("p.md"), "POLICY_BODY").unwrap();
        std::fs::write(output_contracts.join("c.md"), "CONTRACT_BODY").unwrap();

        let mut step = make_test_step("s", StepMode::Auto, "unused", vec![], None);
        step.policy = Some("p".to_string());
        step.output_contract = Some("c".to_string());
        step.instruction = None;

        // build_step_prompt → dispatch_session_start の経路をそのまま再現する。
        let (system_prompt, _prompt) = WorkflowEngine::build_step_prompt(
            &step,
            base,
            "/repo",
            None,
            &HashMap::new(),
            &[],
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
        assert_eq!(r.chat_session_id, "step-session-id");
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
        let output_contracts = base.join("output_contracts");
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::create_dir_all(&output_contracts).unwrap();
        std::fs::write(policies.join("p.md"), "STEP_POLICY_BODY").unwrap();
        std::fs::write(output_contracts.join("c.md"), "STEP_CONTRACT_BODY").unwrap();

        let mut step = make_test_step("s", StepMode::Auto, "unused", vec![], None);
        step.policy = Some("p".to_string());
        step.output_contract = Some("c".to_string());
        step.instruction = None;

        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        let gate = RecordingSessionStartGate {
            records: records.clone(),
        };

        let prompt = WorkflowEngine::build_and_dispatch_step_session(
            &gate,
            &step,
            base,
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

        // user_message に knowledge / instruction はなく空のままだが、関数自体は成功する
        let _ = prompt;

        let recorded = records.lock().unwrap();
        assert_eq!(
            recorded.len(),
            1,
            "gate.start_session must be invoked exactly once via build_and_dispatch_step_session"
        );
        let r = &recorded[0];
        assert_eq!(r.chat_session_id, "step-session-id");
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

        let mut step = make_test_step("s", StepMode::Auto, "unused", vec![], None);
        step.instruction = Some("only-instr".to_string());

        let (system_prompt, _prompt) = WorkflowEngine::build_step_prompt(
            &step,
            tmp.path(),
            "/repo",
            None,
            &HashMap::new(),
            &[],
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
        fetch_parent_count: std::sync::atomic::AtomicUsize,
        create_step_session_count: std::sync::atomic::AtomicUsize,
        dispatch_session_start_count: std::sync::atomic::AtomicUsize,
        persist_workflow_state_count: std::sync::atomic::AtomicUsize,
        broadcast_state_count: std::sync::atomic::AtomicUsize,
        start_agent_turn_count: std::sync::atomic::AtomicUsize,
    }

    impl RecordingStepSessionDeps {
        fn create_step_session_count(&self) -> usize {
            self.create_step_session_count
                .load(std::sync::atomic::Ordering::SeqCst)
        }

        fn fetch_parent_count(&self) -> usize {
            self.fetch_parent_count
                .load(std::sync::atomic::Ordering::SeqCst)
        }

        fn dispatch_session_start_count(&self) -> usize {
            self.dispatch_session_start_count
                .load(std::sync::atomic::Ordering::SeqCst)
        }

        fn persist_workflow_state_count(&self) -> usize {
            self.persist_workflow_state_count
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
    }

    #[async_trait::async_trait]
    impl StepSessionDeps for RecordingStepSessionDeps {
        async fn fetch_parent_session(
            &self,
            _chat_session_id: &str,
        ) -> Result<ParentSessionInfo, WorkflowEngineError> {
            self.fetch_parent_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(ParentSessionInfo {
                backend_id: None,
                selected_model: None,
                permission_mode: "default".to_string(),
            })
        }

        async fn create_step_session(
            &self,
            _worktree_path: &str,
            _step_model: Option<String>,
            _step_permission: Option<String>,
            _parent: ParentSessionInfo,
        ) -> Result<StepSessionInfo, WorkflowEngineError> {
            self.create_step_session_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(StepSessionInfo {
                id: "step-session-id".to_string(),
                permission_mode: "default".to_string(),
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

        async fn persist_workflow_state(
            &self,
            _chat_session_id: &str,
            _snapshot: WorkflowState,
        ) -> Result<(), WorkflowEngineError> {
            self.persist_workflow_state_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        fn broadcast_state(&self, _worktree_path: &str, _snapshot: WorkflowState) {
            self.broadcast_state_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        async fn start_agent_turn(
            &self,
            _step_session_id: &str,
            _worktree_path: &str,
            _permission_mode: &str,
            _prompt: &str,
        ) -> Result<(), WorkflowEngineError> {
            self.start_agent_turn_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    /// `executions` に 1 ステップのワークフロー実行を登録する。
    /// 指定された step を current_step_index=0 として登録する。
    fn insert_single_step_execution(execs: &mut HashMap<String, WorkflowExecution>, step: Step) {
        let workflow = Workflow {
            name: "regression-workflow".to_string(),
            description: "regression test".to_string(),
            builtin: false,
            steps: vec![step],
        };
        let exec = WorkflowExecution {
            id: "exec-id".to_string(),
            workflow,
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            chat_session_id: "parent-session-id".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
        };
        execs.insert("/repo".to_string(), exec);
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
        let engine = WorkflowEngine::new();

        // 参照先ファセットが解決不能な step を含む execution を登録する。
        // facets_base_dir() 配下に "nonexistent_policy_<uuid>.md" が偶然存在することは
        // 実用上ありえないため、ファセット合成は必ず失敗する。
        let mut step = make_test_step("missing-facet", StepMode::Auto, "unused", vec![], None);
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
            deps.fetch_parent_count(),
            0,
            "fetch_parent_session must NOT be invoked when prompt synthesis fails"
        );
        assert_eq!(
            deps.dispatch_session_start_count(),
            0,
            "dispatch_session_start must NOT be invoked when prompt synthesis fails"
        );
        assert_eq!(
            deps.persist_workflow_state_count(),
            0,
            "persist_workflow_state must NOT be invoked when prompt synthesis fails"
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
        let exec = execs
            .get("/repo")
            .expect("execution must remain registered");
        assert!(
            exec.current_session_id.is_none(),
            "current_session_id must remain None when prompt synthesis fails"
        );
    }

    #[tokio::test]
    async fn start_step_session_with_deps_invokes_side_effects_in_order_on_success() {
        // 副作用境界が正しい順序で呼ばれる成功経路を併せて検証する。
        // プロンプト合成が成功した場合は、fetch_parent_session →
        // create_step_session → dispatch_session_start → persist_workflow_state →
        // broadcast_state → start_agent_turn の全境界が各 1 回ずつ呼ばれ、
        // engine.session_workflow_refs と executions["/repo"].current_session_id が
        // 期待通り更新されることを assert する。
        let engine = WorkflowEngine::new();

        // inline_prompt のみのステップなら facet ファイルなしでも合成が成功する。
        let mut step = make_test_step("ok-step", StepMode::Auto, "unused", vec![], None);
        step.instruction = None;
        step.inline_prompt = Some("hello".to_string());

        {
            let mut execs = engine.executions.lock().await;
            insert_single_step_execution(&mut execs, step);
        }

        let deps = RecordingStepSessionDeps::default();
        engine
            .start_step_session_with_deps(&deps, "/repo")
            .await
            .expect("start_step_session_with_deps must succeed for inline_prompt step");

        // 各副作用境界が 1 回ずつ呼ばれている
        assert_eq!(deps.fetch_parent_count(), 1);
        assert_eq!(deps.create_step_session_count(), 1);
        assert_eq!(deps.dispatch_session_start_count(), 1);
        assert_eq!(deps.persist_workflow_state_count(), 1);
        assert_eq!(deps.broadcast_state_count(), 1);
        assert_eq!(deps.start_agent_turn_count(), 1);

        // session_workflow_refs に SequentialStep として登録されている
        let refs = engine.session_workflow_refs.lock().await;
        let entry = refs
            .get("step-session-id")
            .expect("session_workflow_refs must contain step-session-id");
        assert_eq!(entry.worktree_path, "/repo");
        assert!(matches!(entry.kind, SessionRefKind::SequentialStep));
        drop(refs);

        // executions の current_session_id がステップセッションIDで更新されている
        let execs = engine.executions.lock().await;
        let exec = execs
            .get("/repo")
            .expect("execution must remain registered");
        assert_eq!(
            exec.current_session_id.as_deref(),
            Some("step-session-id"),
            "current_session_id must be updated to the created step session id"
        );
    }

    // ---- build_parallel_step_prompt (並列子ステップの合成ルール) ----

    fn make_parallel_step(name: &str) -> ParallelStep {
        ParallelStep {
            name: name.to_string(),
            mode: StepMode::Auto,
            policy: None,
            knowledge: None,
            instruction: None,
            output_contract: None,
            pass_previous_response: None,
            pass_output_from: None,
            model: None,
            permission: None,
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
        let output_contracts = base.join("output_contracts");
        std::fs::create_dir_all(&policies).unwrap();
        std::fs::create_dir_all(&knowledges).unwrap();
        std::fs::create_dir_all(&instructions).unwrap();
        std::fs::create_dir_all(&output_contracts).unwrap();
        std::fs::write(policies.join("pol.md"), "PARALLEL_POLICY_BODY").unwrap();
        std::fs::write(knowledges.join("know.md"), "PARALLEL_KNOWLEDGE_BODY").unwrap();
        std::fs::write(instructions.join("inst.md"), "PARALLEL_INSTRUCTION_BODY").unwrap();
        std::fs::write(output_contracts.join("oc.md"), "PARALLEL_CONTRACT_BODY").unwrap();

        let mut ps = make_parallel_step("child");
        ps.policy = Some("pol".to_string());
        ps.knowledge = Some("know".to_string());
        ps.instruction = Some("inst".to_string());
        ps.output_contract = Some("oc".to_string());

        let (system_prompt, user_message) = WorkflowEngine::build_parallel_step_prompt(
            &ps,
            base,
            "/repo",
            None,
            &HashMap::new(),
            false,
            None,
            &HashMap::new(),
        )
        .unwrap();

        let sp =
            system_prompt.expect("system_prompt must be set for parallel child with policy/oc");
        // policy と output_contract の本文が system_prompt に集約される
        assert!(sp.contains("PARALLEL_POLICY_BODY"));
        assert!(sp.contains("PARALLEL_CONTRACT_BODY"));
        // user_message には knowledge / instruction しか入らない
        assert!(!sp.contains("PARALLEL_KNOWLEDGE_BODY"));
        assert!(!sp.contains("PARALLEL_INSTRUCTION_BODY"));

        // knowledge / instruction の本文は user_message に集約される
        assert!(user_message.contains("PARALLEL_KNOWLEDGE_BODY"));
        assert!(user_message.contains("PARALLEL_INSTRUCTION_BODY"));
        // policy / output_contract は user_message には入らない
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

        let (system_prompt, user_message) = WorkflowEngine::build_parallel_step_prompt(
            &ps,
            base,
            "/repo",
            None,
            &HashMap::new(),
            false,
            None,
            &HashMap::new(),
        )
        .unwrap();

        assert!(system_prompt.is_none());
        assert!(user_message.contains("INSTR"));
    }

    // ---- evaluate_aggregate ----

    #[test]
    fn evaluate_aggregate_all_match_all_children_match() {
        let engine = WorkflowEngine::new();
        let agg = AggregateConfig {
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
        let engine = WorkflowEngine::new();
        let agg = AggregateConfig {
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
        let engine = WorkflowEngine::new();
        let agg = AggregateConfig {
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
        let engine = WorkflowEngine::new();
        let agg = AggregateConfig {
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
        let engine = WorkflowEngine::new();
        let agg = AggregateConfig {
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
        let engine = WorkflowEngine::new();
        let agg = AggregateConfig {
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
        let engine = WorkflowEngine::new();
        let agg = AggregateConfig {
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
        let engine = WorkflowEngine::new();
        let agg = AggregateConfig {
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
        let engine = WorkflowEngine::new();
        // 不正なregexパターン（validationで弾かれるべきだが、エンジン側もgraceful）
        let agg = AggregateConfig {
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
        let engine = WorkflowEngine::new();
        let agg = AggregateConfig {
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
        let engine = WorkflowEngine::new();
        let agg = AggregateConfig {
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
                name: "test".to_string(),
                description: "test".to_string(),
                builtin: false,
                steps: vec![Step {
                    name: "review".to_string(),
                    mode: Some(StepMode::Approval),
                    policy: None,
                    knowledge: None,
                    instruction: Some("Review the code".to_string()),
                    output_contract: None,
                    rules,
                    cycle_guard: None,
                    pass_previous_response: None,
                    pass_output_from: None,
                    inline_prompt: None,
                    collect: None,
                    parallel: None,
                    aggregate: None,
                    resets_cycle_for: None,
                    model: None,
                    permission: None,
                }],
            },
            state,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: vec![],
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
    fn validate_approval_decision_abort_ok() {
        let result = WorkflowEngine::validate_approval_decision(&ApprovalDecision::Abort);
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
        let engine = WorkflowEngine::new();
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

    #[test]
    fn validate_approval_contract_valid_returns_structured_output() {
        let workflow = make_approved_fix_policy_workflow();
        let result = WorkflowEngine::validate_approval_contract_extraction(
            "approved-fix-policy",
            ExtractionResult::Found {
                type_name: "approved-fix-policy".to_string(),
                json: serde_json::json!({
                    "policy": "Fix the reported issue.",
                    "review_step": "code_review_parallel"
                }),
            },
            &workflow,
            1,
        )
        .unwrap();
        match result {
            ContractCheckResult::Valid {
                structured_output,
                result,
            } => {
                assert_eq!(result.as_deref(), Some("approved"));
                assert_eq!(structured_output["review_step"], "code_review_parallel");
            }
            _ => panic!("expected valid approval contract"),
        }
    }

    #[test]
    fn validate_approval_contract_rejects_review_step_not_passed_to_approval_step() {
        let workflow = make_approved_fix_policy_workflow();
        let result = WorkflowEngine::validate_approval_contract_extraction(
            "approved-fix-policy",
            ExtractionResult::Found {
                type_name: "approved-fix-policy".to_string(),
                json: serde_json::json!({
                    "policy": "Fix the reported issue.",
                    "review_step": "other_review_parallel"
                }),
            },
            &workflow,
            1,
        );
        match result {
            Err(err) => {
                let err = err.to_string();
                assert!(err.starts_with("validation_error:"));
                assert!(err.contains("unknown_review_step"));
            }
            Ok(_) => panic!("expected validation_error"),
        }
    }

    #[test]
    fn approval_contract_without_completed_assistant_output_returns_validation_error() {
        let workflow = make_approved_fix_policy_workflow();
        let result = WorkflowEngine::validate_approval_contract_extraction(
            "approved-fix-policy",
            ExtractionResult::NoBlock,
            &workflow,
            1,
        );
        match result {
            Err(WorkflowEngineError::ValidationError(message)) => {
                assert!(message.contains("no_block"));
            }
            _ => panic!("expected validation_error"),
        }
    }

    #[test]
    fn approval_contract_ignores_valid_policy_from_other_session_when_current_has_no_output() {
        let workflow = make_approved_fix_policy_workflow();
        let other_session = crate::session::ChatSession {
            id: "other-policy-session".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![crate::session::ChatMessage {
                id: "m1".to_string(),
                role: crate::session::MessageRole::Agent,
                content: approved_fix_policy_output(
                    "Policy from a previous run.",
                    "code_review_parallel",
                ),
                thinking: None,
                activities: None,
                parts: None,
                timestamp: 1.0,
                mentions: None,
            }],
            state: crate::session::SessionState::Idle,
            created_at: 1.0,
            updated_at: 1.0,
            agent_session_id: None,
            permission_mode: "acceptEdits".to_string(),
            selected_model: None,
            workflow_state: None,
            backend_id: None,
        };
        let current_session = crate::session::ChatSession {
            id: "current-policy-session".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![],
            state: crate::session::SessionState::Idle,
            created_at: 2.0,
            updated_at: 2.0,
            agent_session_id: None,
            permission_mode: "acceptEdits".to_string(),
            selected_model: None,
            workflow_state: None,
            backend_id: None,
        };

        assert!(WorkflowEngine::extract_last_assistant_text_from_session(&other_session).is_some());
        assert!(
            WorkflowEngine::extract_last_assistant_text_from_session(&current_session).is_none()
        );
        let result = WorkflowEngine::validate_approval_contract_extraction(
            "approved-fix-policy",
            ExtractionResult::NoBlock,
            &workflow,
            1,
        );
        match result {
            Err(WorkflowEngineError::ValidationError(_)) => {}
            _ => panic!("expected validation_error"),
        }
    }

    #[tokio::test]
    async fn validate_approval_chat_instruction_limits_current_approval_session() {
        let engine = WorkflowEngine::new();
        let mut exec = make_approval_exec(WorkflowExecutionState::WaitingApproval, vec![]);
        exec.current_session_id = Some("step-session".to_string());
        {
            let mut execs = engine.executions.lock().await;
            execs.insert("/repo".to_string(), exec);
        }
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                "step-session".to_string(),
                SessionWorkflowRef {
                    worktree_path: "/repo".to_string(),
                    kind: SessionRefKind::SequentialStep,
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
    async fn validate_approval_chat_instruction_rejects_current_approval_step_before_waiting() {
        let engine = WorkflowEngine::new();
        let mut exec = make_approval_exec(WorkflowExecutionState::Running, vec![]);
        exec.current_session_id = Some("step-session".to_string());
        {
            let mut execs = engine.executions.lock().await;
            execs.insert("/repo".to_string(), exec);
        }
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                "step-session".to_string(),
                SessionWorkflowRef {
                    worktree_path: "/repo".to_string(),
                    kind: SessionRefKind::SequentialStep,
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
        let engine = WorkflowEngine::new();
        let mut exec = make_approval_exec(WorkflowExecutionState::Running, vec![]);
        exec.workflow.steps[0].name = "implementation_fix_policy".to_string();
        exec.workflow.steps[0].output_contract = Some("approved-fix-policy".to_string());
        exec.current_session_id = Some("fix-session".to_string());
        exec.step_history.push(StepHistoryEntry {
            step_name: "implementation_fix_policy".to_string(),
            completed_at: 1000.0,
            result: Some("approved".to_string()),
            session_id: Some("stale-policy-session".to_string()),
            token_usage: None,
            structured_output: Some(serde_json::json!({
                "policy": "Already approved.",
                "review_step": "code_review_parallel"
            })),
            run_index: 1,
            child_outputs: None,
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
                    "review_step": "code_review_parallel"
                })),
                output_contract: Some("approved-fix-policy".to_string()),
                token_usage: None,
                completed_at: 1000.0,
            },
        );
        {
            let mut execs = engine.executions.lock().await;
            execs.insert("/repo".to_string(), exec);
        }
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                "stale-policy-session".to_string(),
                SessionWorkflowRef {
                    worktree_path: "/repo".to_string(),
                    kind: SessionRefKind::SequentialStep,
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
        let engine = WorkflowEngine::new();
        let mut exec = make_approval_exec(WorkflowExecutionState::Running, vec![]);
        exec.workflow.steps[0].name = "implementation_fix_policy".to_string();
        exec.workflow.steps[0].output_contract = Some("approved-fix-policy".to_string());
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
        {
            let mut execs = engine.executions.lock().await;
            execs.insert("/repo".to_string(), exec);
        }
        {
            let mut refs = engine.session_workflow_refs.lock().await;
            refs.insert(
                "stale-rejected-policy-session".to_string(),
                SessionWorkflowRef {
                    worktree_path: "/repo".to_string(),
                    kind: SessionRefKind::SequentialStep,
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
            permission_mode: "acceptEdits".to_string(),
            selected_model: None,
            workflow_state: None,
            backend_id: None,
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
                name: "review-fix".to_string(),
                description: "test".to_string(),
                builtin: false,
                steps: vec![
                    Step {
                        name: "review".to_string(),
                        mode: Some(StepMode::Approval),
                        policy: None,
                        knowledge: None,
                        instruction: Some("Review the code".to_string()),
                        output_contract: None,
                        rules: vec![TransitionRule {
                            r#match: "reject".to_string(),
                            next: "fix".to_string(),
                        }],
                        cycle_guard: None,
                        pass_previous_response: None,
                        pass_output_from: None,
                        inline_prompt: None,
                        collect: None,
                        parallel: None,
                        aggregate: None,
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                    },
                    Step {
                        name: "fix".to_string(),
                        mode: Some(StepMode::Auto),
                        policy: None,
                        knowledge: None,
                        instruction: Some("Fix the issues".to_string()),
                        output_contract: None,
                        rules: vec![],
                        cycle_guard: None,
                        pass_previous_response: Some(true),
                        pass_output_from: None,
                        inline_prompt: None,
                        collect: None,
                        parallel: None,
                        aggregate: None,
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                    },
                ],
            },
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: vec![],
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
        assert_eq!(exec.workflow.steps[exec.current_step_index].name, "fix");

        let injected = WorkflowEngine::inject_step_outputs(
            "Draft next policy",
            &exec.workflow.steps[exec.current_step_index],
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
                name: "auto-approve".to_string(),
                description: "test".to_string(),
                builtin: false,
                steps: vec![
                    Step {
                        name: "fix_policy".to_string(),
                        mode: Some(StepMode::Approval),
                        policy: None,
                        knowledge: None,
                        instruction: Some("Review fix policy".to_string()),
                        output_contract: Some("approved-fix-policy".to_string()),
                        rules: vec![],
                        cycle_guard: None,
                        pass_previous_response: None,
                        pass_output_from: None,
                        inline_prompt: None,
                        collect: None,
                        parallel: None,
                        aggregate: None,
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                    },
                    make_test_step("fix", StepMode::Auto, "Fix", vec![], None),
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
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some("policy-session".to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
        };
        let structured_output = serde_json::json!({
            "policy": "Fix only the reported issues.",
            "review_step": "code_review_parallel"
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
                    "review_step": "code_review_parallel"
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
    fn spec_driven_plan_fix_policy_approve_records_policy_and_starts_plan_fix_once() {
        let mut exec =
            make_spec_driven_plan_fix_policy_exec("exec-plan-approve", "plan-policy-session");
        let policy_text = approved_fix_policy_output(
            "Update the spec only for the approved plan review finding.",
            "plan_review_parallel",
        );
        let contract_result = WorkflowEngine::validate_approval_contract_extraction(
            "approved-fix-policy",
            extract_workflow_output(&policy_text),
            &exec.workflow,
            exec.current_step_index,
        )
        .unwrap();
        let (structured_output, effective_result) = match contract_result {
            ContractCheckResult::Valid {
                structured_output,
                result,
            } => (structured_output, result.unwrap()),
            _ => panic!("expected valid approved-fix-policy output"),
        };

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
            exec.workflow.steps[exec.current_step_index].name,
            "plan_fix"
        );
        assert_eq!(exec.step_execution_counts.get("plan_fix"), Some(&1));
        assert_eq!(
            exec.step_history
                .iter()
                .filter(|entry| entry.step_name == "plan_fix_policy")
                .count(),
            1
        );
        assert_eq!(
            exec.step_outputs
                .get("plan_fix_policy")
                .and_then(|output| output.structured_output.as_ref())
                .and_then(|output| output.get("policy"))
                .and_then(|policy| policy.as_str()),
            Some("Update the spec only for the approved plan review finding.")
        );
        assert_eq!(
            exec.step_outputs
                .get("plan_fix_policy")
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
                    "review_step": "plan_review_parallel"
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
                .filter(|entry| entry.step_name == "plan_fix_policy")
                .count(),
            1
        );
        assert_eq!(exec.step_execution_counts.get("plan_fix"), Some(&1));
    }

    #[test]
    fn spec_driven_plan_fix_policy_reject_returns_to_plan_approval_without_approved_policy_or_plan_fix(
    ) {
        let mut exec =
            make_spec_driven_plan_fix_policy_exec("exec-plan-reject", "plan-policy-session");
        let decision = ApprovalDecision::Reject {
            comment: "Revise the plan policy first.".to_string(),
        };

        let outcome = WorkflowEngine::apply_approval_application(
            &mut exec,
            &decision,
            ApprovalApplication {
                effective_result: "reject".to_string(),
                structured_output: Some(WorkflowEngine::reject_structured_output(
                    "Revise the plan policy first.",
                    &[],
                )),
                output_contract: None,
            },
        )
        .unwrap();

        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.steps[exec.current_step_index].name,
            "plan_approval"
        );
        assert_eq!(exec.step_execution_counts.get("plan_fix"), None);
        assert!(!exec
            .step_outputs
            .values()
            .any(|output| output.output_contract.as_deref() == Some("approved-fix-policy")));
        assert_eq!(
            exec.step_outputs
                .get("plan_fix_policy")
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
            exec.workflow.steps[exec.current_step_index].name,
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
                name: "auto-approve-path".to_string(),
                description: "test".to_string(),
                builtin: false,
                steps: vec![
                    Step {
                        name: "implementation_fix_policy".to_string(),
                        mode: Some(StepMode::Approval),
                        policy: None,
                        knowledge: None,
                        instruction: Some("Review fix policy".to_string()),
                        output_contract: Some("approved-fix-policy".to_string()),
                        rules: vec![],
                        cycle_guard: None,
                        pass_previous_response: None,
                        pass_output_from: Some(vec!["code_review_parallel".to_string()]),
                        inline_prompt: None,
                        collect: None,
                        parallel: None,
                        aggregate: None,
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                    },
                    make_test_step("fix", StepMode::Auto, "Fix", vec![], None),
                    Step {
                        name: "code_review_parallel".to_string(),
                        mode: None,
                        policy: None,
                        knowledge: None,
                        instruction: None,
                        output_contract: None,
                        rules: vec![],
                        cycle_guard: None,
                        pass_previous_response: None,
                        pass_output_from: None,
                        inline_prompt: None,
                        collect: None,
                        parallel: Some(vec![]),
                        aggregate: Some(AggregateConfig {
                            all_match: Some("LGTM".to_string()),
                            any_match: None,
                            then: "fix".to_string(),
                            r#else: "implementation_fix_policy".to_string(),
                        }),
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                    },
                ],
            },
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 0,
            step_execution_counts: HashMap::from([("implementation_fix_policy".to_string(), 1)]),
            step_history: Vec::new(),
            chat_session_id: "parent-session".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some("policy-session".to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
        };
        let snapshot = exec.to_workflow_state();
        assert_eq!(
            WorkflowEngine::auto_approve_target_for_persisted_snapshot(&snapshot, true),
            Some((
                "exec-auto-approve".to_string(),
                "implementation_fix_policy".to_string()
            ))
        );

        let session = crate::session::ChatSession {
            id: "policy-session".to_string(),
            worktree_path: "/repo".to_string(),
            messages: vec![
                crate::session::ChatMessage {
                    id: "m1".to_string(),
                    role: crate::session::MessageRole::Agent,
                    content: approved_fix_policy_output("Old policy.", "code_review_parallel"),
                    thinking: None,
                    activities: None,
                    parts: None,
                    timestamp: 1.0,
                    mentions: None,
                },
                crate::session::ChatMessage {
                    id: "m2".to_string(),
                    role: crate::session::MessageRole::Agent,
                    content: approved_fix_policy_output(
                        "Fix only reviewed findings.",
                        "code_review_parallel",
                    ),
                    thinking: None,
                    activities: None,
                    parts: None,
                    timestamp: 2.0,
                    mentions: None,
                },
            ],
            state: crate::session::SessionState::Idle,
            created_at: 1.0,
            updated_at: 2.0,
            agent_session_id: None,
            permission_mode: "acceptEdits".to_string(),
            selected_model: None,
            workflow_state: None,
            backend_id: None,
        };
        let latest = WorkflowEngine::extract_last_assistant_text_from_session(&session).unwrap();
        let contract_result = WorkflowEngine::validate_approval_contract_extraction(
            "approved-fix-policy",
            extract_workflow_output(&latest),
            &exec.workflow,
            exec.current_step_index,
        )
        .unwrap();
        let (structured_output, contract_result) = match contract_result {
            ContractCheckResult::Valid {
                structured_output,
                result,
            } => (structured_output, result),
            _ => panic!("expected valid approval contract"),
        };
        let outcome = WorkflowEngine::apply_approval_application(
            &mut exec,
            &ApprovalDecision::Approve,
            ApprovalApplication {
                effective_result: contract_result.unwrap(),
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
                    "review_step": "code_review_parallel"
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
        let engine = WorkflowEngine::new();
        let worktree_path = "/repo";
        let policy_session_id = uuid::Uuid::new_v4().to_string();

        let mut fix_step = make_test_step("fix", StepMode::Auto, "Fix", vec![], None);
        fix_step.collect = Some(CollectConfig {
            from: vec!["implementation_fix_policy".to_string()],
            reduce: ReduceStrategy::Last,
        });
        let exec = WorkflowExecution {
            id: "exec-auto-approve".to_string(),
            workflow: Workflow {
                name: "auto-approve-execute-outcome".to_string(),
                description: "test".to_string(),
                builtin: false,
                steps: vec![
                    Step {
                        name: "code_review_parallel".to_string(),
                        mode: None,
                        policy: None,
                        knowledge: None,
                        instruction: None,
                        output_contract: None,
                        rules: vec![],
                        cycle_guard: None,
                        pass_previous_response: None,
                        pass_output_from: None,
                        inline_prompt: None,
                        collect: None,
                        parallel: Some(vec![]),
                        aggregate: Some(AggregateConfig {
                            all_match: Some("LGTM".to_string()),
                            any_match: None,
                            then: "done".to_string(),
                            r#else: "implementation_fix_policy".to_string(),
                        }),
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                    },
                    Step {
                        name: "implementation_fix_policy".to_string(),
                        mode: Some(StepMode::Approval),
                        policy: None,
                        knowledge: None,
                        instruction: Some("Review fix policy".to_string()),
                        output_contract: Some("approved-fix-policy".to_string()),
                        rules: vec![],
                        cycle_guard: None,
                        pass_previous_response: None,
                        pass_output_from: Some(vec!["code_review_parallel".to_string()]),
                        inline_prompt: None,
                        collect: None,
                        parallel: None,
                        aggregate: None,
                        resets_cycle_for: None,
                        model: None,
                        permission: None,
                    },
                    fix_step,
                ],
            },
            state: WorkflowExecutionState::WaitingApproval,
            current_step_index: 1,
            step_execution_counts: HashMap::from([("implementation_fix_policy".to_string(), 1)]),
            step_history: Vec::new(),
            chat_session_id: "parent-session".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some(policy_session_id.clone()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
        };
        let snapshot = exec.to_workflow_state();
        engine
            .executions
            .lock()
            .await
            .insert(worktree_path.to_string(), exec);
        engine.session_workflow_refs.lock().await.insert(
            policy_session_id,
            SessionWorkflowRef {
                worktree_path: worktree_path.to_string(),
                kind: SessionRefKind::SequentialStep,
            },
        );

        let outcome = engine
            .execute_outcome_persist_auto_approve_for_test(
                worktree_path,
                &snapshot,
                approved_fix_policy_output(
                    "Fix only the approved review finding.",
                    "code_review_parallel",
                ),
            )
            .await
            .unwrap()
            .unwrap();

        let execs = engine.executions.lock().await;
        let exec = execs.get(worktree_path).unwrap();
        assert!(matches!(outcome, StepOutcome::ReduceAndTransition(_)));
        assert_eq!(exec.step_execution_counts.get("fix"), Some(&1));
        assert_eq!(
            exec.step_history
                .iter()
                .filter(|entry| entry.step_name == "implementation_fix_policy")
                .count(),
            1
        );
        assert_eq!(
            exec.step_outputs
                .get("implementation_fix_policy")
                .and_then(|output| output.structured_output.as_ref())
                .and_then(|output| output.get("policy"))
                .and_then(|policy| policy.as_str()),
            Some("Fix only the approved review finding.")
        );
    }

    #[tokio::test]
    async fn execute_outcome_auto_approve_plan_policy_starts_plan_fix_once() {
        let engine = WorkflowEngine::new();
        let worktree_path = "/repo";
        let policy_session_id = uuid::Uuid::new_v4().to_string();
        let exec =
            make_spec_driven_plan_fix_policy_exec("exec-plan-auto-approve", &policy_session_id);
        let snapshot = exec.to_workflow_state();
        engine
            .executions
            .lock()
            .await
            .insert(worktree_path.to_string(), exec);
        engine.session_workflow_refs.lock().await.insert(
            policy_session_id,
            SessionWorkflowRef {
                worktree_path: worktree_path.to_string(),
                kind: SessionRefKind::SequentialStep,
            },
        );

        let outcome = engine
            .execute_outcome_persist_auto_approve_for_test(
                worktree_path,
                &snapshot,
                approved_fix_policy_output(
                    "Revise the spec using the approved plan policy.",
                    "plan_review_parallel",
                ),
            )
            .await
            .unwrap()
            .unwrap();

        let execs = engine.executions.lock().await;
        let exec = execs.get(worktree_path).unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.steps[exec.current_step_index].name,
            "plan_fix"
        );
        assert_eq!(exec.step_execution_counts.get("plan_fix"), Some(&1));
        assert_eq!(
            exec.step_history
                .iter()
                .filter(|entry| entry.step_name == "plan_fix_policy")
                .count(),
            1
        );
        assert_eq!(
            exec.step_outputs
                .get("plan_fix_policy")
                .and_then(|output| output.structured_output.as_ref())
                .and_then(|output| output.get("policy"))
                .and_then(|policy| policy.as_str()),
            Some("Revise the spec using the approved plan policy.")
        );
    }

    #[tokio::test]
    async fn auto_approve_and_manual_approve_race_starts_plan_fix_once() {
        let engine = Arc::new(WorkflowEngine::new());
        let worktree_path = "/repo";
        let policy_session_id = uuid::Uuid::new_v4().to_string();
        let exec =
            make_spec_driven_plan_fix_policy_exec("exec-plan-approve-race", &policy_session_id);
        let snapshot = exec.to_workflow_state();
        engine
            .executions
            .lock()
            .await
            .insert(worktree_path.to_string(), exec);
        engine.session_workflow_refs.lock().await.insert(
            policy_session_id,
            SessionWorkflowRef {
                worktree_path: worktree_path.to_string(),
                kind: SessionRefKind::SequentialStep,
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
                let exec = execs.get(&auto_worktree_path).unwrap();
                WorkflowEngine::validate_approval_target_snapshot(
                    exec,
                    Some(&auto_expected_execution_id),
                    Some(&auto_expected_step_name),
                )
                .unwrap();
            }
            auto_barrier.wait().await;
            auto_engine
                .execute_outcome_persist_auto_approve_for_test(
                    &auto_worktree_path,
                    &auto_snapshot,
                    approved_fix_policy_output(
                        "Revise the spec from the auto approval path.",
                        "plan_review_parallel",
                    ),
                )
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
                let exec = execs.get(&manual_worktree_path).unwrap();
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
                    Some(approved_fix_policy_output(
                        "Revise the spec from the manual approval path.",
                        "plan_review_parallel",
                    )),
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
        let exec = execs.get(worktree_path).unwrap();
        assert_eq!(
            exec.workflow.steps[exec.current_step_index].name,
            "plan_fix"
        );
        assert_eq!(exec.step_execution_counts.get("plan_fix"), Some(&1));
        assert_eq!(
            exec.step_history
                .iter()
                .filter(|entry| entry.step_name == "plan_fix_policy")
                .count(),
            1
        );
        assert_eq!(
            exec.step_outputs
                .get("plan_fix_policy")
                .and_then(|output| output.structured_output.as_ref())
                .and_then(|output| output.get("review_step"))
                .and_then(|review_step| review_step.as_str()),
            Some("plan_review_parallel")
        );
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
        let agent_auto_approve_permission_mode = "bypassPermissions";
        let workflow_approval_auto_approve_enabled = false;
        let snapshot = exec.to_workflow_state();

        assert_eq!(agent_auto_approve_permission_mode, "bypassPermissions");
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

    // ---- ApprovalDecision serde ----

    #[test]
    fn approval_decision_deserialize_approve() {
        let json = r#""approve""#;
        let decision: ApprovalDecision = serde_json::from_str(json).unwrap();
        assert_eq!(decision, ApprovalDecision::Approve);
    }

    #[test]
    fn approval_decision_deserialize_reject_with_comment() {
        let json = r#"{"reject":{"comment":"Please fix this"}}"#;
        let decision: ApprovalDecision = serde_json::from_str(json).unwrap();
        assert_eq!(
            decision,
            ApprovalDecision::Reject {
                comment: "Please fix this".to_string()
            }
        );
    }

    #[test]
    fn approval_decision_deserialize_abort() {
        let json = r#""abort""#;
        let decision: ApprovalDecision = serde_json::from_str(json).unwrap();
        assert_eq!(decision, ApprovalDecision::Abort);
    }

    // R4-01: output_contractなしのparallel childはStepOutputを生成しない
    #[test]
    fn evaluate_aggregate_child_without_output_contract_has_no_step_output() {
        let engine = WorkflowEngine::new();
        let agg = AggregateConfig {
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
            chat_session_id: "chat-1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some("session-1".to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            chat_session_id: "chat-1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: Some("session-1".to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
        };

        let entry = exec.make_step_history_entry(Some("complete".to_string()), None, None);

        assert_eq!(entry.result.as_deref(), Some("complete"));
        assert!(entry.structured_output.is_none());
        assert!(!exec.step_outputs.contains_key("plan"));
    }

    // ---- on_exhausted: apply_transition テスト ----

    fn make_on_exhausted_workflow() -> Workflow {
        Workflow {
            name: "on-exhausted-test".to_string(),
            description: "Test on_exhausted".to_string(),
            builtin: false,
            steps: vec![
                make_test_step(
                    "fix",
                    StepMode::Auto,
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
                    StepMode::Auto,
                    "Review",
                    vec![TransitionRule {
                        r#match: "NEEDS_FIX".to_string(),
                        next: "fix".to_string(),
                    }],
                    None,
                ),
                Step {
                    resets_cycle_for: Some(vec!["fix".to_string()]),
                    ..make_test_step(
                        "approval",
                        StepMode::Interactive,
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
            chat_session_id: "s1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
        };

        // fix への遷移を試みる → ガード超過 → on_exhausted で approval へ
        let outcome = WorkflowEngine::apply_transition(&mut exec, "fix").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.steps[exec.current_step_index].name,
            "approval"
        );
    }

    #[test]
    fn on_exhausted_none_fails_workflow() {
        let mut wf = make_on_exhausted_workflow();
        // on_exhausted を None に変更
        wf.steps[0].cycle_guard = Some(CycleGuard {
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
            chat_session_id: "s1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            chat_session_id: "s1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
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
            chat_session_id: "s1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
        };

        // approval に遷移 → resets_cycle_for で fix のカウントがリセット
        let outcome = WorkflowEngine::apply_transition(&mut exec, "approval").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.steps[exec.current_step_index].name,
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
            chat_session_id: "s1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
        };

        // approval に遷移（カウントリセット）
        WorkflowEngine::apply_transition(&mut exec, "approval").unwrap();
        assert_eq!(exec.step_execution_counts.get("fix"), None);

        // fix に再遷移可能（リセット後なのでガードに引っかからない）
        let outcome = WorkflowEngine::apply_transition(&mut exec, "fix").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(exec.workflow.steps[exec.current_step_index].name, "fix");
        assert_eq!(exec.step_execution_counts.get("fix"), Some(&1));

        // 2回目も可能
        let outcome = WorkflowEngine::apply_transition(&mut exec, "fix").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(exec.step_execution_counts.get("fix"), Some(&2));

        // 3回目は上限到達 → on_exhausted で approval へ
        let outcome = WorkflowEngine::apply_transition(&mut exec, "fix").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(
            exec.workflow.steps[exec.current_step_index].name,
            "approval"
        );
    }

    // ---- on_exhausted チェーン遷移テスト ----

    #[test]
    fn on_exhausted_chain_transitions() {
        // step_a → (exhausted) → step_b → (exhausted) → step_c
        let wf = Workflow {
            name: "chain-test".to_string(),
            description: "test".to_string(),
            builtin: false,
            steps: vec![
                make_test_step(
                    "step_a",
                    StepMode::Auto,
                    "A",
                    vec![],
                    Some(CycleGuard {
                        max_iterations: 1,
                        on_exhausted: Some("step_b".to_string()),
                    }),
                ),
                make_test_step(
                    "step_b",
                    StepMode::Auto,
                    "B",
                    vec![],
                    Some(CycleGuard {
                        max_iterations: 1,
                        on_exhausted: Some("step_c".to_string()),
                    }),
                ),
                make_test_step("step_c", StepMode::Interactive, "C", vec![], None),
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
            chat_session_id: "s1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
        };

        // step_a → exhausted → step_b → exhausted → step_c
        let outcome = WorkflowEngine::apply_transition(&mut exec, "step_a").unwrap();
        assert!(matches!(outcome, StepOutcome::TransitionAndStart(_)));
        assert_eq!(exec.workflow.steps[exec.current_step_index].name, "step_c");
    }

    #[test]
    fn on_exhausted_chain_to_non_exhausted_fails() {
        // step_a → (exhausted) → step_b (exhausted, no on_exhausted) → Failed
        let wf = Workflow {
            name: "chain-fail-test".to_string(),
            description: "test".to_string(),
            builtin: false,
            steps: vec![
                make_test_step(
                    "step_a",
                    StepMode::Auto,
                    "A",
                    vec![],
                    Some(CycleGuard {
                        max_iterations: 1,
                        on_exhausted: Some("step_b".to_string()),
                    }),
                ),
                make_test_step(
                    "step_b",
                    StepMode::Auto,
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
            chat_session_id: "s1".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            task: None,
            parallel_run: None,
            workflow_variables: HashMap::new(),
            contract_retry_count: 0,
        };

        let outcome = WorkflowEngine::apply_transition(&mut exec, "step_a").unwrap();
        assert!(matches!(outcome, StepOutcome::Persist(_)));
        assert!(matches!(exec.state, WorkflowExecutionState::Failed { .. }));
    }

    // ---- resolve_step_settings ----

    #[test]
    fn resolve_step_settings_model_and_permission_specified() {
        let result = resolve_step_settings(
            Some("codex-mini".to_string()),
            Some("bypassPermissions".to_string()),
            Some("codex".to_string()),
            Some("claude".to_string()),
            Some("opus-4".to_string()),
            "acceptEdits".to_string(),
        );
        assert_eq!(
            result,
            ResolvedStepSettings {
                backend_id: Some("codex".to_string()),
                selected_model: Some("codex-mini".to_string()),
                permission_mode: "bypassPermissions".to_string(),
            }
        );
    }

    #[test]
    fn resolve_step_settings_model_only() {
        let result = resolve_step_settings(
            Some("haiku".to_string()),
            None,
            Some("claude".to_string()),
            Some("claude".to_string()),
            Some("opus-4".to_string()),
            "acceptEdits".to_string(),
        );
        assert_eq!(
            result,
            ResolvedStepSettings {
                backend_id: Some("claude".to_string()),
                selected_model: Some("haiku".to_string()),
                permission_mode: "acceptEdits".to_string(),
            }
        );
    }

    #[test]
    fn resolve_step_settings_permission_only() {
        let result = resolve_step_settings(
            None,
            Some("plan".to_string()),
            None,
            Some("claude".to_string()),
            Some("opus-4".to_string()),
            "acceptEdits".to_string(),
        );
        assert_eq!(
            result,
            ResolvedStepSettings {
                backend_id: Some("claude".to_string()),
                selected_model: Some("opus-4".to_string()),
                permission_mode: "plan".to_string(),
            }
        );
    }

    #[test]
    fn resolve_step_settings_nothing_specified() {
        let result = resolve_step_settings(
            None,
            None,
            None,
            Some("claude".to_string()),
            Some("opus-4".to_string()),
            "acceptEdits".to_string(),
        );
        assert_eq!(
            result,
            ResolvedStepSettings {
                backend_id: Some("claude".to_string()),
                selected_model: Some("opus-4".to_string()),
                permission_mode: "acceptEdits".to_string(),
            }
        );
    }

    #[test]
    fn resolve_step_settings_parallel_children_different_configs() {
        // ステップA: model=opus-4, permission=plan
        let result_a = resolve_step_settings(
            Some("opus-4".to_string()),
            Some("plan".to_string()),
            Some("claude".to_string()),
            Some("claude".to_string()),
            Some("opus-4".to_string()),
            "acceptEdits".to_string(),
        );
        assert_eq!(
            result_a,
            ResolvedStepSettings {
                backend_id: Some("claude".to_string()),
                selected_model: Some("opus-4".to_string()),
                permission_mode: "plan".to_string(),
            }
        );

        // ステップB: model=codex-mini, permission=bypassPermissions
        let result_b = resolve_step_settings(
            Some("codex-mini".to_string()),
            Some("bypassPermissions".to_string()),
            Some("codex".to_string()),
            Some("claude".to_string()),
            Some("opus-4".to_string()),
            "acceptEdits".to_string(),
        );
        assert_eq!(
            result_b,
            ResolvedStepSettings {
                backend_id: Some("codex".to_string()),
                selected_model: Some("codex-mini".to_string()),
                permission_mode: "bypassPermissions".to_string(),
            }
        );

        // 並列ステップ間で結果が独立していることを確認
        assert_ne!(result_a.backend_id, result_b.backend_id);
        assert_ne!(result_a.selected_model, result_b.selected_model);
        assert_ne!(result_a.permission_mode, result_b.permission_mode);
    }
}
