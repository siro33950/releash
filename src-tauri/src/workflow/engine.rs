use std::collections::HashMap;
use std::sync::Arc;

use regex::RegexBuilder;
use serde::Serialize;
use tauri::Manager;
use tokio::sync::Mutex;

use crate::agent_sdk::AgentProcessMap;
use crate::agent_status::{current_timestamp, AgentStatusCenter};
use crate::session::SessionStore;
use crate::workflow::schema::{StepMode, TransitionRule, Workflow};
use crate::workflow::state::{StepHistoryEntry, WorkflowExecutionState, WorkflowState};

/// ワークフローエンジンが外部にブロードキャストするイベントペイロード。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStatePayload {
    pub chat_session_id: String,
    pub workflow_state: WorkflowState,
}

/// ワークフロー実行の内部状態。
struct WorkflowExecution {
    id: String,
    workflow: Workflow,
    state: WorkflowExecutionState,
    current_step_index: usize,
    step_execution_counts: HashMap<String, u32>,
    step_history: Vec<StepHistoryEntry>,
    worktree_path: String,
    started_at: f64,
    updated_at: f64,
}

impl WorkflowExecution {
    /// ワークフローが実行中（Running または WaitingApproval）かどうかを返す。
    fn is_active(&self) -> bool {
        matches!(
            self.state,
            WorkflowExecutionState::Running | WorkflowExecutionState::WaitingApproval
        )
    }

    /// ワークフロー開始の事前条件を検証する（純粋関数）。
    fn validate_start(
        workflow: &Workflow,
        existing: Option<&WorkflowExecution>,
    ) -> Result<(), String> {
        if workflow.steps.is_empty() {
            return Err("Workflow has no steps".to_string());
        }
        if let Some(existing) = existing {
            if existing.is_active() {
                return Err(format!(
                    "Workflow '{}' is already running for this session",
                    existing.workflow.name,
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
        WorkflowState {
            execution_id: self.id.clone(),
            workflow_name: self.workflow.name.clone(),
            state: self.state.clone(),
            current_step_index: self.current_step_index,
            current_step_name: self.workflow.steps[self.current_step_index].name.clone(),
            total_steps: self.workflow.steps.len(),
            step_history: self.step_history.clone(),
            started_at: self.started_at,
            updated_at: self.updated_at,
        }
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
    fn check_cycle_guard(&self, target_step_name: &str) -> Result<CycleGuardResult, String> {
        let idx = self
            .workflow
            .steps
            .iter()
            .position(|s| s.name == target_step_name)
            .ok_or_else(|| format!("Step '{}' not found in workflow", target_step_name))?;

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
    ) -> Result<ApprovalAction, String> {
        if self.state != WorkflowExecutionState::WaitingApproval {
            return Err("Workflow is not waiting for approval".to_string());
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
    fn decide_interactive_action(&self, abort: bool) -> Result<InteractiveAction, String> {
        if self.state != WorkflowExecutionState::Running {
            return Err("Workflow is not running".to_string());
        }
        let step = &self.workflow.steps[self.current_step_index];
        if step.mode != StepMode::Interactive {
            return Err("Current step is not interactive mode".to_string());
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

/// ワークフローのステップを順次実行するステートマシンエンジン。
pub struct WorkflowEngine {
    executions: Mutex<HashMap<String, WorkflowExecution>>,
}

impl WorkflowEngine {
    pub fn new() -> Self {
        Self {
            executions: Mutex::new(HashMap::new()),
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
        worktree_path: &str,
    ) -> Result<(), String> {
        {
            let execs = self.executions.lock().await;
            WorkflowExecution::validate_start(&workflow, execs.get(chat_session_id))?;
        }

        let now = current_timestamp();
        let mut execution = WorkflowExecution {
            id: uuid::Uuid::new_v4().to_string(),
            workflow: workflow.clone(),
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::new(),
            step_history: Vec::new(),
            worktree_path: worktree_path.to_string(),
            started_at: now,
            updated_at: now,
        };

        let step_name = workflow.steps[0].name.clone();
        {
            let mut execs = self.executions.lock().await;
            execution.step_execution_counts.insert(step_name.clone(), 1);
            execs.insert(chat_session_id.to_string(), execution);
        }

        // 永続化・ブロードキャスト
        self.persist_state(app, session_store, chat_session_id)
            .await?;
        self.broadcast_state(app, chat_session_id).await;

        // 最初のステップのAgentSession開始＋プロンプト送信
        self.start_step_session(app, handles, session_store, chat_session_id)
            .await
    }

    /// turn_complete後に呼ばれるフック。
    /// autoモード→タグ検出で遷移、approvalモード→WaitingApproval、interactiveモード→何もしない。
    pub async fn on_turn_complete(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        chat_session_id: &str,
        exit_code: i64,
        final_parts: &[crate::session::MessagePart],
    ) -> Result<(), String> {
        let action = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(chat_session_id)
                .ok_or("No workflow execution found")?;
            exec.decide_turn_complete_action(exit_code)
        };

        match action {
            TurnCompleteAction::NotRunning | TurnCompleteAction::Noop => Ok(()),
            TurnCompleteAction::SessionError {
                step_name,
                exit_code,
            } => {
                self.set_execution_state(
                    app,
                    session_store,
                    chat_session_id,
                    WorkflowExecutionState::Failed {
                        reason: format!(
                            "AgentSession error at step '{}' (exit_code: {})",
                            step_name, exit_code
                        ),
                    },
                )
                .await
            }
            TurnCompleteAction::AutoEvaluate { rules, step_name } => {
                self.handle_auto_complete(
                    app,
                    session_store,
                    handles,
                    chat_session_id,
                    final_parts,
                    &rules,
                    &step_name,
                )
                .await
            }
            TurnCompleteAction::WaitApproval => {
                self.set_execution_state(
                    app,
                    session_store,
                    chat_session_id,
                    WorkflowExecutionState::WaitingApproval,
                )
                .await
            }
        }
    }

    /// approvalモードでのユーザー判定を処理する。
    pub async fn handle_approval(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        chat_session_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), String> {
        let (action, step_name) = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(chat_session_id)
                .ok_or("No workflow execution found")?;
            let action = exec.decide_approval_action(&decision)?;
            let step_name = exec.workflow.steps[exec.current_step_index].name.clone();
            (action, step_name)
        };

        let result_tag = match &decision {
            ApprovalDecision::Approve => "approve",
            ApprovalDecision::Reject => "reject",
            ApprovalDecision::Abort => "abort",
        };

        match action {
            ApprovalAction::Advance => {
                self.record_step_completion(
                    chat_session_id,
                    &step_name,
                    Some(result_tag.to_string()),
                )
                .await;
                self.advance_to_next(app, session_store, handles, chat_session_id)
                    .await
            }
            ApprovalAction::TransitionTo(target) => {
                self.record_step_completion(
                    chat_session_id,
                    &step_name,
                    Some(result_tag.to_string()),
                )
                .await;
                self.transition_to_step(app, session_store, handles, chat_session_id, &target)
                    .await
            }
            ApprovalAction::FailedNoRejectRule(name) => {
                self.record_step_completion(
                    chat_session_id,
                    &step_name,
                    Some(result_tag.to_string()),
                )
                .await;
                self.set_execution_state(
                    app,
                    session_store,
                    chat_session_id,
                    WorkflowExecutionState::Failed {
                        reason: format!("No reject rule defined for step '{}'", name),
                    },
                )
                .await
            }
            ApprovalAction::Abort => {
                self.set_execution_state(
                    app,
                    session_store,
                    chat_session_id,
                    WorkflowExecutionState::Aborted,
                )
                .await
            }
        }
    }

    /// interactiveモードの完了/中止を処理する。
    pub async fn complete_interactive(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        chat_session_id: &str,
        abort: bool,
    ) -> Result<(), String> {
        let (action, step_name) = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(chat_session_id)
                .ok_or("No workflow execution found")?;
            let action = exec.decide_interactive_action(abort)?;
            let step_name = exec.workflow.steps[exec.current_step_index].name.clone();
            (action, step_name)
        };

        match action {
            InteractiveAction::Abort => {
                self.set_execution_state(
                    app,
                    session_store,
                    chat_session_id,
                    WorkflowExecutionState::Aborted,
                )
                .await
            }
            InteractiveAction::Advance => {
                self.record_step_completion(
                    chat_session_id,
                    &step_name,
                    Some("complete".to_string()),
                )
                .await;
                self.advance_to_next(app, session_store, handles, chat_session_id)
                    .await
            }
        }
    }

    /// ワークフローを中断する。
    pub async fn abort_workflow(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        chat_session_id: &str,
    ) -> Result<(), String> {
        {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(chat_session_id)
                .ok_or("No workflow execution found")?;

            // 既に終了状態なら何もしない
            if !exec.is_active() {
                return Ok(());
            }
        }

        // 実行中のAgentSessionを中断
        self.interrupt_agent(handles, chat_session_id).await;

        self.set_execution_state(
            app,
            session_store,
            chat_session_id,
            WorkflowExecutionState::Aborted,
        )
        .await
    }

    /// 状態取得。
    pub async fn get_state(&self, chat_session_id: &str) -> Option<WorkflowState> {
        let execs = self.executions.lock().await;
        execs.get(chat_session_id).map(|e| e.to_workflow_state())
    }

    /// chat_session_idがワークフロー実行中かどうか。
    pub async fn is_running(&self, chat_session_id: &str) -> bool {
        let execs = self.executions.lock().await;
        execs.get(chat_session_id).is_some_and(|e| e.is_active())
    }

    // ---- 内部メソッド ----

    /// 実行状態を更新し、永続化・ブロードキャストする。
    async fn set_execution_state(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        chat_session_id: &str,
        new_state: WorkflowExecutionState,
    ) -> Result<(), String> {
        {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(chat_session_id)
                .ok_or("No workflow execution found")?;
            exec.state = new_state;
            exec.updated_at = current_timestamp();
        }
        self.persist_state(app, session_store, chat_session_id)
            .await?;
        self.broadcast_state(app, chat_session_id).await;
        Ok(())
    }

    /// autoモードのタグ検出結果を処理する。
    #[allow(clippy::too_many_arguments)]
    async fn handle_auto_complete(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        chat_session_id: &str,
        final_parts: &[crate::session::MessagePart],
        rules: &[TransitionRule],
        step_name: &str,
    ) -> Result<(), String> {
        // テキストパートを結合
        let text = Self::extract_text_from_parts(final_parts);

        if rules.is_empty() {
            // ルールがない場合は定義順で次のステップに遷移
            self.record_step_completion(chat_session_id, step_name, None)
                .await;
            return self
                .advance_to_next(app, session_store, handles, chat_session_id)
                .await;
        }

        // タグ検出
        match Self::evaluate_auto_rules(&text, rules) {
            Some((next_step, matched_rule)) => {
                self.record_step_completion(chat_session_id, step_name, Some(matched_rule))
                    .await;
                self.transition_to_step(app, session_store, handles, chat_session_id, &next_step)
                    .await
            }
            None => {
                self.set_execution_state(
                    app,
                    session_store,
                    chat_session_id,
                    WorkflowExecutionState::Failed {
                        reason: format!("No matching rule found for step '{}' output", step_name),
                    },
                )
                .await
            }
        }
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

    /// 指定ステップに遷移する（サイクルガード検証含む）。
    async fn transition_to_step(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        chat_session_id: &str,
        target_step_name: &str,
    ) -> Result<(), String> {
        // サイクルガードチェック + 遷移実行を1回のロックで処理
        let exceeded_info = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(chat_session_id)
                .ok_or("No workflow execution found")?;

            let idx = exec
                .workflow
                .steps
                .iter()
                .position(|s| s.name == target_step_name)
                .ok_or_else(|| format!("Step '{}' not found in workflow", target_step_name))?;

            let guard_result = exec.check_cycle_guard(target_step_name)?;

            match guard_result {
                CycleGuardResult::Exceeded {
                    max_iterations,
                    count,
                } => Some((max_iterations, count)),
                CycleGuardResult::Allowed => {
                    exec.current_step_index = idx;
                    exec.state = WorkflowExecutionState::Running;
                    *exec
                        .step_execution_counts
                        .entry(target_step_name.to_string())
                        .or_insert(0) += 1;
                    exec.updated_at = current_timestamp();
                    None
                }
            }
        };

        if let Some((max_iterations, count)) = exceeded_info {
            return self
                .set_execution_state(
                    app,
                    session_store,
                    chat_session_id,
                    WorkflowExecutionState::Failed {
                        reason: format!(
                            "Cycle guard exceeded for step '{}': max_iterations={}, executed={}",
                            target_step_name, max_iterations, count
                        ),
                    },
                )
                .await;
        }

        self.persist_state(app, session_store, chat_session_id)
            .await?;
        self.broadcast_state(app, chat_session_id).await;

        // 次のステップのAgentSession開始＋プロンプト送信
        self.start_step_session(app, handles, session_store, chat_session_id)
            .await
    }

    /// 定義順で次のステップに遷移する。最後のステップならCompletedに。
    async fn advance_to_next(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        chat_session_id: &str,
    ) -> Result<(), String> {
        let decision = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(chat_session_id)
                .ok_or("No workflow execution found")?;
            exec.decide_next_step()
        };

        match decision {
            NextStepDecision::Completed => {
                self.set_execution_state(
                    app,
                    session_store,
                    chat_session_id,
                    WorkflowExecutionState::Completed,
                )
                .await
            }
            NextStepDecision::TransitionTo(name) => {
                self.transition_to_step(app, session_store, handles, chat_session_id, &name)
                    .await
            }
        }
    }

    /// 現在のステップのAgentSessionを開始し、プロンプトを送信する。
    async fn start_step_session(
        &self,
        app: &tauri::AppHandle,
        handles: &Arc<Mutex<AgentProcessMap>>,
        session_store: &Arc<SessionStore>,
        chat_session_id: &str,
    ) -> Result<(), String> {
        let (worktree_path, prompt) = {
            let execs = self.executions.lock().await;
            let exec = execs
                .get(chat_session_id)
                .ok_or("No workflow execution found")?;
            let step = &exec.workflow.steps[exec.current_step_index];
            (exec.worktree_path.clone(), step.prompt.clone())
        };

        let data_dir =
            crate::session::resolve_data_dir(app).map_err(|e| format!("resolve_data_dir: {e}"))?;
        let session = session_store
            .get_session(&data_dir, chat_session_id)
            .map_err(|e| format!("get_session: {e}"))?
            .ok_or_else(|| format!("ChatSession not found: {}", chat_session_id))?;
        let permission_mode = session.permission_mode;

        // AgentSession開始
        crate::agent_sdk::start_agent_session_internal(
            app,
            handles,
            session_store,
            chat_session_id,
            &worktree_path,
            None,
        )
        .await?;

        // プロンプト送信
        crate::agent_sdk::start_agent_turn_internal(
            app,
            handles,
            session_store,
            chat_session_id,
            &worktree_path,
            &permission_mode,
            &prompt,
        )
        .await
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

    /// ステップ完了を履歴に記録する。
    async fn record_step_completion(
        &self,
        chat_session_id: &str,
        step_name: &str,
        result: Option<String>,
    ) {
        let mut execs = self.executions.lock().await;
        if let Some(exec) = execs.get_mut(chat_session_id) {
            exec.step_history.push(StepHistoryEntry {
                step_name: step_name.to_string(),
                completed_at: current_timestamp(),
                result,
            });
        }
    }

    /// ワークフロー状態をChatSessionに永続化する。
    async fn persist_state(
        &self,
        app: &tauri::AppHandle,
        session_store: &Arc<SessionStore>,
        chat_session_id: &str,
    ) -> Result<(), String> {
        let workflow_state = {
            let execs = self.executions.lock().await;
            execs.get(chat_session_id).map(|e| e.to_workflow_state())
        };

        let workflow_state = workflow_state
            .ok_or_else(|| format!("No workflow execution for session '{}'", chat_session_id))?;

        let data_dir =
            crate::session::resolve_data_dir(app).map_err(|e| format!("resolve_data_dir: {e}"))?;
        let mut session = session_store
            .get_session(&data_dir, chat_session_id)
            .map_err(|e| format!("get_session: {e}"))?
            .ok_or_else(|| format!("ChatSession not found: {}", chat_session_id))?;
        session.workflow_state = Some(workflow_state);
        session.updated_at = crate::session::now_timestamp();
        session_store
            .save_session(&data_dir, &session)
            .map_err(|e| format!("save_session: {e}"))?;

        Ok(())
    }

    /// ワークフロー状態をブロードキャストする。
    async fn broadcast_state(&self, app: &tauri::AppHandle, chat_session_id: &str) {
        let payload = {
            let execs = self.executions.lock().await;
            execs.get(chat_session_id).map(|e| WorkflowStatePayload {
                chat_session_id: chat_session_id.to_string(),
                workflow_state: e.to_workflow_state(),
            })
        };

        if let Some(payload) = payload {
            let center: Option<tauri::State<'_, Arc<AgentStatusCenter>>> =
                app.try_state::<Arc<AgentStatusCenter>>();
            if let Some(center) = center {
                // ワークフロー状態変更イベントを emit
                center.emit_workflow_state_changed(&payload);

                // SessionStatusのworkflowフィールドも更新
                if let Some(mut status) = center.get_session(chat_session_id) {
                    status.workflow_step = Some(payload.workflow_state.current_step_name.clone());
                    status.workflow_execution_state =
                        Some(payload.workflow_state.state.as_str().to_string());
                    center.update_session(status);
                }
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

    // ---- WorkflowExecution ----

    fn make_test_workflow() -> Workflow {
        Workflow {
            name: "test-workflow".to_string(),
            description: "Test workflow".to_string(),
            builtin: false,
            steps: vec![
                Step {
                    name: "plan".to_string(),
                    mode: StepMode::Interactive,
                    prompt: "Plan the work".to_string(),
                    rules: vec![],
                    cycle_guard: None,
                },
                Step {
                    name: "implement".to_string(),
                    mode: StepMode::Auto,
                    prompt: "Implement the plan".to_string(),
                    rules: vec![],
                    cycle_guard: None,
                },
                Step {
                    name: "review".to_string(),
                    mode: StepMode::Auto,
                    prompt: "Review the implementation".to_string(),
                    rules: vec![
                        TransitionRule {
                            r#match: "NEEDS_FIX".to_string(),
                            next: "implement".to_string(),
                        },
                        TransitionRule {
                            r#match: "LGTM".to_string(),
                            next: "report".to_string(),
                        },
                    ],
                    cycle_guard: Some(CycleGuard { max_iterations: 3 }),
                },
                Step {
                    name: "report".to_string(),
                    mode: StepMode::Approval,
                    prompt: "Generate report".to_string(),
                    rules: vec![TransitionRule {
                        r#match: "reject".to_string(),
                        next: "implement".to_string(),
                    }],
                    cycle_guard: None,
                },
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

            worktree_path: "/repo".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
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
            worktree_path: "/repo".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
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
            worktree_path: "/repo".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
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
            worktree_path: "/repo".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
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
            worktree_path: "/repo".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
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
            worktree_path: "/repo".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
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
            worktree_path: "/repo".to_string(),
            started_at: 1000.0,
            updated_at: 1001.0,
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
            }],
            worktree_path: "/repo".to_string(),
            started_at: 1000.0,
            updated_at: 1001.0,
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
            worktree_path: "/repo".to_string(),
            started_at: 1000.0,
            updated_at: 1001.0,
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
            worktree_path: "/repo".to_string(),
            started_at: 1000.0,
            updated_at: 1002.0,
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
            worktree_path: "/repo".to_string(),
            started_at: 1000.0,
            updated_at: 1000.0,
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
        assert!(result.unwrap_err().contains("no steps"));
    }

    #[test]
    fn validate_start_active_workflow_returns_err() {
        let workflow = make_test_workflow();
        let existing = make_exec(0); // Running state
        let result = WorkflowExecution::validate_start(&workflow, Some(&existing));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already running"));
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
}
