use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use regex::RegexBuilder;
use tauri::Manager;
use tokio::sync::Mutex;

use crate::agent_sdk::AgentProcessMap;
use crate::agent_status::{current_timestamp, AgentStatusCenter};
use crate::session::SessionStore;
use crate::workflow::log::{WorkflowEventLog, WorkflowLogEvent};
use crate::workflow::schema::{
    CollectConfig, ReduceStrategy, StepMode, StepPrompt, TransitionRule, Workflow,
};
use crate::workflow::state::{
    StepHistoryEntry, StepOutput, TokenUsage, WorkflowExecutionState, WorkflowState,
};
use crate::workflow::storage;

const MAX_OUTPUT_SIZE: usize = 100 * 1024; // 100KB

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
            Self::InvalidState(msg) => write!(f, "{msg}"),
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
    Exceeded { max_iterations: u32, count: u32 },
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
    /// interactiveモード → 何もしない
    Noop,
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
            started_at: self.started_at,
            updated_at: self.updated_at,
        }
    }

    /// 現在のステップの完了履歴エントリを生成し、トークン使用量をリセットする。
    /// output_textが100KBを超える場合はtruncateする。
    fn make_step_history_entry(
        &mut self,
        result: Option<String>,
        output_text: Option<String>,
    ) -> StepHistoryEntry {
        let output_text = output_text.map(truncate_output);

        let step_name = self.workflow.steps[self.current_step_index].name.clone();
        let run_index = self
            .step_execution_counts
            .get(&step_name)
            .copied()
            .unwrap_or(1);
        let completed_at = current_timestamp();
        let token_usage = Some(std::mem::take(&mut self.current_step_token_usage));

        // StepOutputを更新
        if let Some(ref text) = output_text {
            self.step_outputs.insert(
                step_name.clone(),
                StepOutput {
                    step_name: step_name.clone(),
                    run_index,
                    session_id: self.current_session_id.clone(),
                    result: result.clone(),
                    output_text: text.clone(),
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
            output_text,
            run_index,
        };
        self.current_session_id = None;
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

        match step.mode {
            StepMode::Auto => TurnCompleteAction::AutoEvaluate {
                rules: step.rules.clone(),
                step_name: step.name.clone(),
            },
            StepMode::Approval => TurnCompleteAction::WaitApproval,
            StepMode::Interactive => TurnCompleteAction::Noop,
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
            ApprovalDecision::Reject => match step.rules.iter().find(|r| r.r#match == "reject") {
                Some(r) => Ok(ApprovalAction::TransitionTo(r.next.clone())),
                None => Ok(ApprovalAction::FailedNoRejectRule(step.name.clone())),
            },
            ApprovalDecision::Abort => Ok(ApprovalAction::Abort),
        }
    }

    /// interactiveモードの判定ロジック（純粋関数）。
    fn decide_interactive_action(
        &self,
        abort: bool,
    ) -> Result<InteractiveAction, WorkflowEngineError> {
        if self.state != WorkflowExecutionState::Running {
            return Err(WorkflowEngineError::InvalidState(
                "Workflow is not running".to_string(),
            ));
        }
        let step = &self.workflow.steps[self.current_step_index];
        if step.mode != StepMode::Interactive {
            return Err(WorkflowEngineError::InvalidState(
                "Current step is not interactive mode".to_string(),
            ));
        }
        if abort {
            Ok(InteractiveAction::Abort)
        } else {
            Ok(InteractiveAction::Advance)
        }
    }
}

/// approvalモードのユーザー判定。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approve,
    Reject,
    Abort,
}

/// approvalモードの判定結果（純粋関数用）。
#[derive(Debug, Clone, PartialEq)]
enum ApprovalAction {
    Advance,
    TransitionTo(String),
    Abort,
    FailedNoRejectRule(String),
}

/// interactiveモードの判定結果（純粋関数用）。
#[derive(Debug, Clone, PartialEq)]
enum InteractiveAction {
    Advance,
    Abort,
}

/// ロック内で確定した遷移結果。ロ��ク外で永続化・AgentSession起動を行うための情報を持つ。
enum StepOutcome {
    /// 状態を永続化・ブロードキャストするだけ（終了状態遷移���ど）
    Persist(WorkflowState),
    /// 次のステップに遷移し、AgentSession を起動する
    TransitionAndStart(WorkflowState),
    /// collect仮想stepに遷移し、reduce処理を実行する
    ReduceAndTransition(WorkflowState),
}

/// reduce処理の結果。
struct ReduceResult {
    result: Option<String>,
    text: String,
}

/// ワークフローのステップを順次実行するステートマシンエンジン。
pub struct WorkflowEngine {
    /// worktree_path → WorkflowExecution のマッピング
    executions: Mutex<HashMap<String, WorkflowExecution>>,
    /// session_id（親・ステップ両方） → worktree_path のマッピング
    session_worktree_map: Mutex<HashMap<String, String>>,
}

impl WorkflowEngine {
    pub fn new() -> Self {
        Self {
            executions: Mutex::new(HashMap::new()),
            session_worktree_map: Mutex::new(HashMap::new()),
        }
    }

    /// ワークフローを開始する。
    /// ChatSessionは既に作成済みの前提で、最初のステップのプロンプトを送信する。
    pub async fn start_workflow(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        workflow: Workflow,
        chat_session_id: &str,
        file_stem: &str,
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

        // session_worktree_map に親セッションIDを登録
        {
            let mut map = self.session_worktree_map.lock().await;
            map.insert(chat_session_id.to_string(), worktree_path.clone());
        }

        // 永続化・ブロードキャスト（ロック内で確定したスナップショットを使用）
        if let Err(e) = self
            .persist_state(app, session_store, chat_session_id, snapshot.clone())
            .await
        {
            let mut execs = self.executions.lock().await;
            execs.remove(&worktree_path);
            drop(execs);
            self.cleanup_session_worktree_map(&worktree_path).await;
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

        // 最初のステップのAgentSession開始＋プロンプト送信
        // 失敗時はFailed状態に遷移して永続化する
        if let Err(e) = self
            .start_step_session(app, handles, session_store, &worktree_path)
            .await
        {
            {
                let mut execs = self.executions.lock().await;
                if let Some(exec) = execs.get_mut(&worktree_path) {
                    let entry = exec
                        .make_step_history_entry(Some(format!("session_start_failed: {e}")), None);
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
        // session_id から worktree_path を解決（ワークフロー既終了なら何もしない）
        let Some(worktree_path) = self.resolve_worktree_path(session_id).await else {
            return Ok(());
        };

        // 判定 + 状態変更を原子的に実行（AutoEvaluate以外）
        let (chat_session_id, action_or_outcome) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(&worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.clone()))?;

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
                TurnCompleteAction::NotRunning | TurnCompleteAction::Noop => return Ok(()),
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

    /// approvalモードでのユーザー判定を処理する。
    /// 判定 + 状態変更 + 履歴記録を1回のロックで原子的に実行し、
    /// ロック外では永続化・ブロードキャスト・AgentSession起動のみ行う。
    pub async fn handle_approval(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        decision: ApprovalDecision,
    ) -> Result<(), WorkflowEngineError> {
        let result_tag = match &decision {
            ApprovalDecision::Approve => "approve",
            ApprovalDecision::Reject => "reject",
            ApprovalDecision::Abort => "abort",
        };

        // ロック外でoutput_textを事前取得（approvalはAgentSession完了後なので取得可能）
        let output_text = self
            .fetch_current_output(app, session_store, worktree_path)
            .await?;

        // 判定 + 状態変更 + 履歴記録を原子的に実行
        let (chat_session_id, outcome) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let chat_session_id = exec.chat_session_id.clone();
            let action = exec.decide_approval_action(&decision)?;

            let outcome = match action {
                ApprovalAction::Advance => {
                    let entry =
                        exec.make_step_history_entry(Some(result_tag.to_string()), output_text);
                    exec.step_history.push(entry);
                    Self::apply_advance(exec)
                }
                ApprovalAction::TransitionTo(target) => {
                    let entry =
                        exec.make_step_history_entry(Some(result_tag.to_string()), output_text);
                    exec.step_history.push(entry);
                    Self::apply_transition(exec, &target)?
                }
                ApprovalAction::FailedNoRejectRule(name) => {
                    let entry =
                        exec.make_step_history_entry(Some(result_tag.to_string()), output_text);
                    exec.step_history.push(entry);
                    exec.state = WorkflowExecutionState::Failed {
                        reason: format!("No reject rule defined for step '{}'", name),
                    };
                    exec.updated_at = current_timestamp();
                    StepOutcome::Persist(exec.to_workflow_state())
                }
                ApprovalAction::Abort => {
                    exec.state = WorkflowExecutionState::Aborted;
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

    /// interactiveモードの完了/中止を処理する。
    /// 判定 + 状態変更 + 履歴記録を1回のロックで原子的に実行する。
    pub async fn complete_interactive(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
        abort: bool,
    ) -> Result<(), WorkflowEngineError> {
        // ロック外でoutput_textを事前取得
        let output_text = if !abort {
            self.fetch_current_output(app, session_store, worktree_path)
                .await?
        } else {
            None
        };

        let (chat_session_id, outcome) = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let chat_session_id = exec.chat_session_id.clone();
            let action = exec.decide_interactive_action(abort)?;

            let outcome = match action {
                InteractiveAction::Abort => {
                    exec.state = WorkflowExecutionState::Aborted;
                    exec.updated_at = current_timestamp();
                    StepOutcome::Persist(exec.to_workflow_state())
                }
                InteractiveAction::Advance => {
                    let entry =
                        exec.make_step_history_entry(Some("complete".to_string()), output_text);
                    exec.step_history.push(entry);
                    Self::apply_advance(exec)
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

    /// ワークフローを中断する。
    pub async fn abort_workflow(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        worktree_path: &str,
    ) -> Result<(), WorkflowEngineError> {
        let current_step_session_id;
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
        }

        // 実行中のステップセッションを中断
        if let Some(ref step_sid) = current_step_session_id {
            self.interrupt_agent(handles, step_sid).await;
        }

        self.set_execution_state(
            app,
            session_store,
            worktree_path,
            WorkflowExecutionState::Aborted,
        )
        .await
    }

    /// 指定worktree_pathに関連する session_worktree_map エントリを削除する。
    async fn cleanup_session_worktree_map(&self, worktree_path: &str) {
        let mut map = self.session_worktree_map.lock().await;
        map.retain(|_, wt| wt != worktree_path);
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

    /// セッションIDからworktree_pathを解決する。
    /// session_worktree_mapに登録されていない場合はNoneを返す。
    async fn resolve_worktree_path(&self, session_id: &str) -> Option<String> {
        let map = self.session_worktree_map.lock().await;
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
            self.cleanup_session_worktree_map(worktree_path).await;
        }
        Ok(())
    }

    /// autoモードのタグ検出結果を処理する。
    /// 判定 + 状態変更 + 履歴記録を1回のロックで原子的に実行する。
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

        // タグ検出もロック外で完了（純粋関数）
        let rule_match = if rules.is_empty() {
            None // ルールなし → 定義順で次へ
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

            let output_text = Some(text.clone());
            let outcome = match rule_match {
                None => {
                    // ルールなし → 定義順で次へ
                    let entry = exec.make_step_history_entry(None, output_text);
                    exec.step_history.push(entry);
                    Self::apply_advance(exec)
                }
                Some(Some((next_step, matched_rule))) => {
                    // ルールマッチ → 指定ステップへ遷移
                    let entry = exec.make_step_history_entry(Some(matched_rule), output_text);
                    exec.step_history.push(entry);
                    Self::apply_transition(exec, &next_step)?
                }
                Some(None) => {
                    // マッチなし → Failed
                    let entry = exec
                        .make_step_history_entry(Some("no_matching_rule".to_string()), output_text);
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
    async fn start_step_session(
        &self,
        app: &tauri::AppHandle,
        handles: &Arc<Mutex<AgentProcessMap>>,
        session_store: &Arc<SessionStore>,
        worktree_path: &str,
    ) -> Result<(), WorkflowEngineError> {
        let (chat_session_id, prompt_ref, step_clone, step_outputs_clone, step_history_clone) = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let step = &exec.workflow.steps[exec.current_step_index];
            let prompt_ref = step.prompt.clone().ok_or_else(|| {
                WorkflowEngineError::InvalidWorkflow(format!(
                    "Step '{}' has no prompt (collect steps should not start sessions)",
                    step.name
                ))
            })?;
            (
                exec.chat_session_id.clone(),
                prompt_ref,
                step.clone(),
                exec.step_outputs.clone(),
                exec.step_history.clone(),
            )
        };

        let data_dir = crate::session::resolve_data_dir(app)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;
        let base_prompt = Self::resolve_step_prompt(&prompt_ref, worktree_path)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve prompt: {e}")))?;
        let prompt = Self::inject_step_outputs(
            &base_prompt,
            &step_clone,
            &step_outputs_clone,
            &step_history_clone,
        );
        let parent_session = session_store
            .get_session(&data_dir, &chat_session_id)
            .map_err(|e| WorkflowEngineError::SessionStore(format!("get_session: {e}")))?
            .ok_or_else(|| WorkflowEngineError::SessionNotFound(chat_session_id.clone()))?;
        let permission_mode = parent_session.permission_mode;

        // ステップ用の新しいChatSessionを生成
        let step_session =
            crate::session::create_session_internal(session_store, &data_dir, worktree_path)
                .map_err(|e| {
                    WorkflowEngineError::SessionStore(format!("create step session: {e}"))
                })?;
        let step_session_id = step_session.id.clone();

        // ステップセッションID → worktree_pathのマッピングを登録
        {
            let mut map = self.session_worktree_map.lock().await;
            map.insert(step_session_id.clone(), worktree_path.to_string());
        }

        // AgentSession開始（ステップ用セッションIDを使用）
        crate::agent_sdk::start_agent_session_internal(
            app,
            handles,
            session_store,
            &step_session_id,
            worktree_path,
            None,
        )
        .await
        .map_err(WorkflowEngineError::AgentSession)?;

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
            self.persist_state(app, session_store, &chat_session_id, snapshot.clone())
                .await?;
            self.broadcast_state(app, worktree_path, snapshot);
        }

        // プロンプト送信（ステップ用セッションIDを使用）
        crate::agent_sdk::start_agent_turn_internal(
            app,
            handles,
            session_store,
            &step_session_id,
            worktree_path,
            &permission_mode,
            &prompt,
        )
        .await
        .map_err(WorkflowEngineError::AgentSession)
    }

    /// step.prompt を実行用プロンプト本文へ展開する。
    fn resolve_step_prompt(prompt_ref: &StepPrompt, worktree_path: &str) -> Result<String, String> {
        match prompt_ref {
            StepPrompt::Template(t) => Self::load_prompt_template(&t.template, worktree_path),
            StepPrompt::InlineObject(o) => Ok(o.inline.clone()),
            StepPrompt::Inline(inline) => {
                // 旧builtin YAML互換: `prompt: fixer` のような1語参照は、
                // 同名テンプレートが存在する場合だけテンプレートとして扱う。
                let prompt_path = storage::prompts_dir().join(format!("{inline}.yml"));
                if prompt_path.exists() {
                    log::warn!(
                        "Deprecated workflow prompt syntax `prompt: {inline}` resolved as template. Use `prompt: {{ template: {inline} }}` instead."
                    );
                    Self::load_prompt_template(inline, worktree_path)
                } else {
                    Ok(inline.clone())
                }
            }
        }
    }

    fn load_prompt_template(template_name: &str, worktree_path: &str) -> Result<String, String> {
        let prompt_path = storage::prompts_dir().join(format!("{template_name}.yml"));
        if !prompt_path.exists() {
            return Err(format!("Prompt template not found: {template_name}"));
        }

        let template = storage::load_prompt(&prompt_path).map_err(|e| e.to_string())?;
        Ok(Self::render_prompt_template(&template, worktree_path))
    }

    fn render_prompt_template(
        template: &crate::workflow::prompt_schema::PromptTemplate,
        worktree_path: &str,
    ) -> String {
        let mut content = template.content.clone();
        for var in &template.variables {
            let value = if var.name == "project_name" {
                Self::project_name_from_worktree(worktree_path)
            } else {
                var.default.clone().unwrap_or_default()
            };
            content = content.replace(&format!("{{{{{}}}}}", var.name), &value);
        }
        content
    }

    fn project_name_from_worktree(worktree_path: &str) -> String {
        Path::new(worktree_path)
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(worktree_path)
            .to_string()
    }

    /// 現在のステップセッションからoutput_textを取得する。
    /// handle_approval / complete_interactive の共通パターン。
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

        Some(truncate_output(text))
    }

    /// ステップの出力をプロンプトにコンテキストブロックとして注入する。
    fn inject_step_outputs(
        prompt: &str,
        step: &crate::workflow::schema::Step,
        step_outputs: &HashMap<String, StepOutput>,
        step_history: &[StepHistoryEntry],
    ) -> String {
        let mut result = prompt.to_string();

        // pass_previous_response: true → step_historyの最後のエントリのstep_nameからstep_outputsを参照
        if step.pass_previous_response == Some(true) {
            if let Some(last_entry) = step_history.last() {
                let text = match step_outputs.get(&last_entry.step_name) {
                    Some(o) => o.output_text.as_str(),
                    None => "(not yet completed)",
                };
                Self::append_step_output_block(&mut result, &last_entry.step_name, text);
            }
        }

        // pass_output_from: ["step_a", "step_b"] → 指定step名のoutputをcontext block追加
        if let Some(ref refs) = step.pass_output_from {
            for step_name in refs {
                let text = match step_outputs.get(step_name.as_str()) {
                    Some(o) => o.output_text.as_str(),
                    None => "(not yet completed)",
                };
                Self::append_step_output_block(&mut result, step_name, text);
            }
        }

        result
    }

    fn append_step_output_block(result: &mut String, step_name: &str, text: &str) {
        result.push_str(&format!(
            "\n\n<step_output name=\"{}\">\n{}\n</step_output>",
            step_name, text
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
                        text: output.output_text.clone(),
                    },
                    None => ReduceResult {
                        result: None,
                        text: String::new(),
                    },
                }
            }
            ReduceStrategy::Concat => {
                let mut parts = Vec::new();
                for step_name in &collect.from {
                    if let Some(output) = step_outputs.get(step_name.as_str()) {
                        parts.push(format!("## {}\n{}", step_name, output.output_text));
                    }
                }
                ReduceResult {
                    result: None,
                    text: parts.join("\n\n"),
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
                let mut text = String::new();
                for (result_key, steps) in &groups {
                    text.push_str(&format!("## {}\n", result_key));
                    for step in steps {
                        text.push_str(&format!("- {}\n", step));
                    }
                    text.push('\n');
                }
                ReduceResult { result: None, text }
            }
            ReduceStrategy::AnyNeedsFix => {
                let mut any_needs_fix = false;
                let mut parts = Vec::new();
                for step_name in &collect.from {
                    if let Some(output) = step_outputs.get(step_name.as_str()) {
                        let step_result =
                            Self::resolve_step_result(output, &["NEEDS_FIX", "needs_fix"]);
                        if matches!(
                            step_result.as_deref(),
                            Some("NEEDS_FIX") | Some("needs_fix")
                        ) {
                            any_needs_fix = true;
                        }
                        parts.push(format!("## {}\n{}", step_name, output.output_text));
                    } else {
                        any_needs_fix = true;
                        parts.push(format!("## {}\n<MISSING OUTPUT>", step_name));
                    }
                }
                ReduceResult {
                    result: Some(if any_needs_fix { "NEEDS_FIX" } else { "LGTM" }.to_string()),
                    text: parts.join("\n\n"),
                }
            }
            ReduceStrategy::AllPassed => {
                let mut all_passed = true;
                let mut parts = Vec::new();
                for step_name in &collect.from {
                    if let Some(output) = step_outputs.get(step_name.as_str()) {
                        let step_result =
                            Self::resolve_step_result(output, &["PASSED", "passed", "LGTM"]);
                        if !matches!(
                            step_result.as_deref(),
                            Some("PASSED") | Some("passed") | Some("LGTM")
                        ) {
                            all_passed = false;
                        }
                        parts.push(format!("## {}\n{}", step_name, output.output_text));
                    } else {
                        all_passed = false;
                        parts.push(format!("## {}\n<MISSING OUTPUT>", step_name));
                    }
                }
                ReduceResult {
                    result: Some(if all_passed { "PASSED" } else { "FAILED" }.to_string()),
                    text: parts.join("\n\n"),
                }
            }
        }
    }

    /// StepOutputからresultを解決する。result直接値があればそれを優先、
    /// なければoutput_textからregexフォールバック。
    fn resolve_step_result(output: &StepOutput, patterns: &[&str]) -> Option<String> {
        if output.result.is_some() {
            return output.result.clone();
        }
        for pattern in patterns {
            if let Ok(re) = RegexBuilder::new(pattern).size_limit(1 << 20).build() {
                if re.is_match(&output.output_text) {
                    return Some(pattern.to_string());
                }
            }
        }
        None
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
                if exec.workflow.steps[idx].collect.is_some() {
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
            } => {
                exec.state = WorkflowExecutionState::Failed {
                    reason: format!(
                        "Cycle guard exceeded for step '{}': max_iterations={}, executed={}",
                        target_step_name, max_iterations, count
                    ),
                };
                exec.updated_at = current_timestamp();
                Ok(StepOutcome::Persist(exec.to_workflow_state()))
            }
            CycleGuardResult::Allowed => {
                exec.current_step_index = idx;
                exec.state = WorkflowExecutionState::Running;
                *exec
                    .step_execution_counts
                    .entry(target_step_name.to_string())
                    .or_insert(0) += 1;
                exec.updated_at = current_timestamp();
                if exec.workflow.steps[idx].collect.is_some() {
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
                    self.cleanup_session_worktree_map(worktree_path).await;
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
                        Some(reduce_result.text.clone()),
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
                        match Self::evaluate_auto_rules(&reduce_result.text, &step_rules) {
                            Some((next_step, _)) => Self::apply_transition(exec, &next_step)?,
                            None => Self::apply_advance(exec),
                        }
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
                                output_text_len: output.map_or(0, |o| o.output_text.len()),
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
                        reduce_text: reduce_result.text.clone(),
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
        }
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
                    output_text: last_entry.output_text.clone(),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::MessagePart;
    use crate::workflow::schema::{CycleGuard, Step, StepMode, TransitionRule, Workflow};

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

    // ---- prompt template rendering ----

    #[test]
    fn render_prompt_template_substitutes_project_name_from_worktree() {
        let template = crate::workflow::prompt_schema::PromptTemplate {
            name: "fixer".to_string(),
            description: "Fix prompt".to_string(),
            content: "{{project_name}} の問題を修正してください。".to_string(),
            variables: vec![crate::workflow::prompt_schema::PromptVariable {
                name: "project_name".to_string(),
                description: "Project name".to_string(),
                default: None,
            }],
            builtin: true,
        };

        let prompt = WorkflowEngine::render_prompt_template(&template, "/work/releash");

        assert_eq!(prompt, "releash の問題を修正してください。");
    }

    #[test]
    fn render_prompt_template_uses_variable_defaults() {
        let template = crate::workflow::prompt_schema::PromptTemplate {
            name: "custom".to_string(),
            description: "Custom prompt".to_string(),
            content: "対象: {{target}}".to_string(),
            variables: vec![crate::workflow::prompt_schema::PromptVariable {
                name: "target".to_string(),
                description: "Target".to_string(),
                default: Some("unit tests".to_string()),
            }],
            builtin: false,
        };

        let prompt = WorkflowEngine::render_prompt_template(&template, "/work/releash");

        assert_eq!(prompt, "対象: unit tests");
    }

    #[test]
    fn resolve_step_prompt_keeps_inline_prompt_when_template_file_is_missing() {
        let prompt_ref =
            StepPrompt::inline("この文字列はテンプレート名ではなく、そのまま送る").unwrap();
        let prompt = WorkflowEngine::resolve_step_prompt(&prompt_ref, "/work/releash").unwrap();

        assert_eq!(prompt, "この文字列はテンプレート名ではなく、そのまま送る");
    }

    // ---- WorkflowExecution ----

    fn make_test_step(
        name: &str,
        mode: StepMode,
        prompt: &str,
        rules: Vec<TransitionRule>,
        cycle_guard: Option<CycleGuard>,
    ) -> Step {
        Step {
            name: name.to_string(),
            mode,
            prompt: StepPrompt::inline(prompt),
            rules,
            cycle_guard,
            pass_previous_response: None,
            pass_output_from: None,
            collect: None,
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
                    Some(CycleGuard { max_iterations: 3 }),
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
        };
        let ws = exec.to_workflow_state();
        assert_eq!(ws.state, WorkflowExecutionState::WaitingApproval);
        assert_eq!(ws.current_step_name, "report");
        assert_eq!(ws.current_step_index, 3);
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
                output_text: None,
                run_index: 0,
            }],
            chat_session_id: "session-1".to_string(),
            started_at: 1000.0,
            updated_at: 1001.0,
            current_session_id: None,
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
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
                count: 3
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
    fn turn_complete_action_interactive_noop() {
        let exec = make_exec(0); // plan (interactive)
        assert_eq!(
            exec.decide_turn_complete_action(0),
            TurnCompleteAction::Noop
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
            exec.decide_approval_action(&ApprovalDecision::Reject)
                .unwrap(),
            ApprovalAction::TransitionTo("implement".to_string())
        );
    }

    #[test]
    fn decide_approval_action_reject_no_rule() {
        let mut exec = make_exec(0); // plan (interactive, no reject rule)
        exec.state = WorkflowExecutionState::WaitingApproval;
        assert_eq!(
            exec.decide_approval_action(&ApprovalDecision::Reject)
                .unwrap(),
            ApprovalAction::FailedNoRejectRule("plan".to_string())
        );
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

    // ---- decide_interactive_action ----

    #[test]
    fn decide_interactive_action_complete() {
        let exec = make_exec(0); // plan (interactive, state=Running)
        assert_eq!(
            exec.decide_interactive_action(false).unwrap(),
            InteractiveAction::Advance
        );
    }

    #[test]
    fn decide_interactive_action_abort() {
        let exec = make_exec(0); // plan (interactive, state=Running)
        assert_eq!(
            exec.decide_interactive_action(true).unwrap(),
            InteractiveAction::Abort
        );
    }

    #[test]
    fn decide_interactive_action_not_running() {
        let mut exec = make_exec(0);
        exec.state = WorkflowExecutionState::Completed;
        assert!(exec.decide_interactive_action(false).is_err());
    }

    #[test]
    fn decide_interactive_action_wrong_mode() {
        let exec = make_exec(1); // implement (auto mode, state=Running)
        assert!(exec.decide_interactive_action(false).is_err());
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
                output_text: None,
                run_index: 0,
            },
            StepHistoryEntry {
                step_name: "implement".to_string(),
                completed_at: 1001.0,
                result: None,
                session_id: None,
                token_usage: None,
                output_text: None,
                run_index: 0,
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
            output_text: None,
            run_index: 0,
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
            output_text: None,
            run_index: 0,
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
            output_text: output_text.to_string(),
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
            output_text: None,
            run_index: 0,
        }];
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &outputs, &history);
        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("output from A"));
    }

    #[test]
    fn inject_step_outputs_no_pass_previous_response() {
        let step = make_test_step("step_b", StepMode::Auto, "Do B", vec![], None);
        let outputs = HashMap::new();
        let history = vec![];
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &outputs, &history);
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
        let result = WorkflowEngine::inject_step_outputs("Do C", &step, &outputs, &[]);
        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("output A"));
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
        let result = WorkflowEngine::inject_step_outputs("Do C", &step, &outputs, &[]);
        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("output A"));
        assert!(result.contains("<step_output name=\"step_b\">"));
        assert!(result.contains("output B"));
    }

    #[test]
    fn inject_step_outputs_missing_step_shows_not_completed() {
        let mut step = make_test_step("step_b", StepMode::Auto, "Do B", vec![], None);
        step.pass_output_from = Some(vec!["step_a".to_string()]);

        let outputs = HashMap::new(); // step_a not present
        let result = WorkflowEngine::inject_step_outputs("Do B", &step, &outputs, &[]);
        assert!(result.contains("<step_output name=\"step_a\">"));
        assert!(result.contains("(not yet completed)"));
    }

    // ---- apply_reduce ----

    use crate::workflow::schema::{CollectConfig, ReduceStrategy};

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
        assert_eq!(r.text, "text_b");
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
        assert!(r.text.contains("## a\noutput from a"));
        assert!(r.text.contains("## b\noutput from b"));
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
        assert!(r.text.contains("## LGTM"));
        assert!(r.text.contains("- a"));
        assert!(r.text.contains("- c"));
        assert!(r.text.contains("## NEEDS_FIX"));
        assert!(r.text.contains("- b"));
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
    fn reduce_any_needs_fix_regex_fallback() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AnyNeedsFix);
        // result is None but output_text contains NEEDS_FIX
        let outputs = make_outputs(vec![
            ("a", "Everything looks good", None),
            ("b", "Found issues: NEEDS_FIX", None),
        ]);
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        assert_eq!(r.result, Some("NEEDS_FIX".to_string()));
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
    fn reduce_all_passed_regex_fallback() {
        let collect = make_collect(vec!["a", "b"], ReduceStrategy::AllPassed);
        let outputs = make_outputs(vec![
            ("a", "All tests PASSED", None),
            ("b", "Some tests failed", None),
        ]);
        let r = WorkflowEngine::apply_reduce(&collect, &outputs);
        assert_eq!(r.result, Some("FAILED".to_string()));
    }

    // ---- resolve_step_result ----

    #[test]
    fn resolve_step_result_prefers_direct_result() {
        let output = make_step_output("s", "output with NEEDS_FIX text", Some("LGTM"));
        let r = WorkflowEngine::resolve_step_result(&output, &["NEEDS_FIX"]);
        assert_eq!(r, Some("LGTM".to_string()));
    }

    #[test]
    fn resolve_step_result_regex_fallback() {
        let output = make_step_output("s", "found NEEDS_FIX issue", None);
        let r = WorkflowEngine::resolve_step_result(&output, &["NEEDS_FIX", "needs_fix"]);
        assert_eq!(r, Some("NEEDS_FIX".to_string()));
    }

    #[test]
    fn resolve_step_result_no_match_returns_none() {
        let output = make_step_output("s", "everything is fine", None);
        let r = WorkflowEngine::resolve_step_result(&output, &["NEEDS_FIX"]);
        assert!(r.is_none());
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
}
