use std::collections::HashMap;

#[cfg(test)]
use crate::adaptor::gateway::workflow::domain_mapping::transition_rule_from_domain;
use crate::adaptor::gateway::workflow::domain_mapping::{
    node_definition_to_domain, parallel_aggregate_to_domain, parallel_step_state_from_domain,
    step_history_entries_to_domain, step_history_entry_from_domain, step_output_from_domain,
    step_outputs_to_domain, token_usage_from_domain, token_usage_to_domain,
    workflow_definition_to_domain, workflow_execution_state_to_domain,
};
use crate::adaptor::gateway::workflow::engine_error::{
    workflow_error_to_engine_error, WorkflowEngineError,
};
use crate::adaptor::gateway::workflow::engine_start_guard;
use crate::adaptor::gateway::workflow::output_submission as workflow_output_submission;
use crate::adaptor::gateway::workflow::runtime_commit::StepOutcome;
#[cfg(test)]
use crate::adaptor::gateway::workflow::schema::TransitionRule;
use crate::adaptor::gateway::workflow::schema::{ParallelAggregate, Workflow};
use crate::adaptor::gateway::workflow::state::{
    ApprovalOperations, StepHistoryEntry, StepOutput, TokenUsage, WorkflowExecutionState,
    WorkflowStallObservation, WorkflowState,
};
use crate::adaptor::gateway::workflow::step_settings::WorkflowDefaults;
use crate::domain::workflow as workflow_domain;
use crate::domain::workflow::services::history as workflow_history;
use crate::domain::workflow::services::projection as workflow_projection;
use crate::domain::workflow::services::submission as workflow_submission;
use crate::domain::workflow::services::transition as workflow_transition;
use crate::domain::workflow::ApprovalDecision as DomainApprovalDecision;
use crate::domain::workflow::{FailureDisposition, WorkflowStepFailureKind};
use crate::usecase::agent_session::status::current_timestamp;

/// ワークフロー実行の内部状態。
#[derive(Clone)]
pub(crate) struct WorkflowExecution {
    /// `WorkflowState.execution_id` を `run_id` として昇格させた識別子。
    /// `WorkflowRuntimeService.executions` の HashMap キーと一致する。
    pub(crate) id: String,
    pub(crate) workflow: Workflow,
    pub(crate) state: WorkflowExecutionState,
    pub(crate) current_step_index: usize,
    pub(crate) step_execution_counts: HashMap<String, u32>,
    pub(crate) step_history: Vec<StepHistoryEntry>,
    /// step / 並列子 step 起動時の継承デフォルト（permission_mode / backend_id / selected_model）。
    /// `start_workflow` 時に capture し、以降は session_store を読み直さない（in-memory のみ）。
    pub(crate) workflow_defaults: WorkflowDefaults,
    /// run が紐づく worktree。HashMap キーではなく属性として保持する。
    /// `find_by_worktree` / `find_by_worktree_mut` が worktree 起点の lookup で参照する。
    pub(crate) worktree_path: String,
    pub(crate) started_at: f64,
    pub(crate) updated_at: f64,
    /// 現在のステップに対応するAgentSessionのセッションID。
    pub(crate) current_session_id: Option<String>,
    /// 現在のステップで累計したトークン使用量。
    pub(crate) current_step_token_usage: TokenUsage,
    /// step_name → 最新StepOutput のマップ。
    pub(crate) step_outputs: HashMap<String, StepOutput>,
    /// ワークフロー実行時のタスク内容（テンプレート変数 {{task}} の展開に使用）。
    pub(crate) task: Option<String>,
    /// 並列実行中の場合の状態。
    pub(crate) parallel_run: Option<ParallelRunState>,
    /// ワークフローレベルの変数（spec-directory等のcontract結果から設定）。
    pub(crate) workflow_variables: HashMap<String, String>,
    /// 現在実行中の step session で観測した非終端 stall signal。
    pub(crate) current_stall_observations: Vec<WorkflowStallObservation>,
}

/// 並列実行中の内部状態。
#[derive(Clone)]
pub(crate) struct ParallelRunState {
    pub(crate) parent_step_name: String,
    pub(crate) aggregate: Option<ParallelAggregate>,
    pub(crate) children: Vec<ParallelChildRun>,
}

/// 並列子ステップの実行状態。
#[derive(Clone)]
pub(crate) struct ParallelChildRun {
    pub(crate) step_name: String,
    pub(crate) session_id: String,
    pub(crate) state: ParallelChildState,
    pub(crate) result: Option<String>,
    pub(crate) structured_output: Option<serde_json::Value>,
    pub(crate) output_contract: Option<String>,
    pub(crate) failure_kind: Option<WorkflowStepFailureKind>,
    pub(crate) failure_disposition: Option<FailureDisposition>,
    pub(crate) token_usage: TokenUsage,
    pub(crate) run_index: u32,
}

/// 並列子ステップの状態。
#[derive(Clone, PartialEq)]
pub(crate) enum ParallelChildState {
    Running,
    Completed,
    Failed,
    Interrupted,
}

impl From<&ParallelChildState> for workflow_output_submission::SubmissionParallelChildState {
    fn from(state: &ParallelChildState) -> Self {
        match state {
            ParallelChildState::Running => Self::Running,
            ParallelChildState::Completed => Self::Completed,
            ParallelChildState::Failed => Self::Failed,
            ParallelChildState::Interrupted => Self::Interrupted,
        }
    }
}

/// session_workflow_refsの値型。session_id → run_id の逆引き索引。
///
/// parent ChatSession 機構撤去後は step session のみが登録されるため種別区別は不要
/// （Spec issues-929: 「逐次 step と並列子 step は単一経路で扱う」/ Spec issues-1011:
/// engine 内部キーは run_id に統一）。worktree_path は `WorkflowExecution.worktree_path`
/// 属性として exec から取得する。
#[derive(Clone)]
pub(crate) struct SessionWorkflowRef {
    /// engine.executions の HashMap キー（= `WorkflowExecution.id` = `run_id`）。
    pub(crate) run_id: String,
}

impl WorkflowExecution {
    /// ワークフローが実行中（Running または WaitingApproval）かどうかを返す。
    pub(crate) fn is_active(&self) -> bool {
        matches!(
            self.state,
            WorkflowExecutionState::Running | WorkflowExecutionState::WaitingApproval
        )
    }

    /// ワークフローが終了状態（Completed / Failed / Aborted）かどうかを返す。
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            WorkflowExecutionState::Completed
                | WorkflowExecutionState::Failed { .. }
                | WorkflowExecutionState::Aborted
        )
    }

    /// ワークフロー開始の事前条件を検証する（純粋関数）。
    ///
    /// executions ロック内の defense-in-depth で呼ばれる。Run Store の active index と
    /// in-memory executions 表が一時的に不整合な場合に、最終的な atomic guard として機能する。
    pub(crate) fn validate_start(
        workflow: &Workflow,
        existing: Option<&WorkflowExecution>,
    ) -> Result<(), WorkflowEngineError> {
        let existing_active_workflow_name = existing
            .filter(|existing| existing.is_active())
            .map(|existing| existing.workflow.name.as_str());
        engine_start_guard::validate_start(workflow, existing_active_workflow_name)
    }

    /// 永続化用の `WorkflowState` に変換する。
    pub(crate) fn to_workflow_state(&self) -> WorkflowState {
        let domain_history = step_history_entries_to_domain(&self.step_history);
        let total_token_usage =
            token_usage_from_domain(&workflow_projection::total_token_usage(&domain_history));
        let domain_parallel_run = self.domain_parallel_run();
        let active_parallel_steps =
            workflow_projection::active_parallel_steps(domain_parallel_run.as_ref())
                .into_iter()
                .map(parallel_step_state_from_domain)
                .collect();

        let step_states = crate::adaptor::gateway::workflow::state::compute_step_states(
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
            active_parallel_steps,
            workflow_variables: self.workflow_variables.clone(),
            approval_operations: self.build_approval_operations(),
            stall_observations: self.current_stall_observations.clone(),
            started_at: self.started_at,
            updated_at: self.updated_at,
        }
    }

    fn domain_parallel_run(&self) -> Option<workflow_domain::ParallelRunState> {
        self.parallel_run
            .as_ref()
            .map(|parallel_run| workflow_domain::ParallelRunState {
                parent_step_name: parallel_run.parent_step_name.clone(),
                aggregate: parallel_run
                    .aggregate
                    .as_ref()
                    .map(parallel_aggregate_to_domain),
                children: parallel_run
                    .children
                    .iter()
                    .map(|child| workflow_domain::ParallelChildRun {
                        step_name: child.step_name.clone(),
                        session_id: child.session_id.clone(),
                        state: match child.state {
                            ParallelChildState::Running => {
                                workflow_domain::ParallelChildState::Running
                            }
                            ParallelChildState::Completed => {
                                workflow_domain::ParallelChildState::Completed
                            }
                            ParallelChildState::Failed => {
                                workflow_domain::ParallelChildState::Failed
                            }
                            ParallelChildState::Interrupted => {
                                workflow_domain::ParallelChildState::Interrupted
                            }
                        },
                        result: child.result.clone(),
                        structured_output: child.structured_output.clone(),
                        output_contract: child.output_contract.clone(),
                        failure_kind: child.failure_kind,
                        failure_disposition: child.failure_disposition,
                        token_usage: token_usage_to_domain(&child.token_usage),
                        run_index: child.run_index,
                    })
                    .collect(),
            })
    }

    fn build_approval_operations(&self) -> Option<ApprovalOperations> {
        let state = workflow_execution_state_to_domain(&self.state);
        let current_step = self
            .workflow
            .nodes
            .get(self.current_step_index)
            .map(node_definition_to_domain);
        workflow_projection::approval_operations(&state, current_step.as_ref()).map(|ops| {
            ApprovalOperations {
                can_reject: ops.can_reject,
            }
        })
    }

    /// spec issues-1023: 中断された通常 step の `step_history` entry を作る。
    ///
    /// 既存 `make_step_history_entry` の副作用（`current_session_id` reset /
    /// step_outputs の前段クリーンアップ等）を**起こさない**点が違い。`abort_workflow_by_run_id`
    /// は post-commit で interrupt_agent や cleanup を行うため、reset 系は
    /// `finalize_terminal_transition_after_required_append` 経路に任せる。
    ///
    /// session_id を entry にコピーすることで、`step_history` 由来の session log 到達経路
    /// （domain session_projection の `collect_step_session_ids`）を復活させる。
    pub(crate) fn make_aborted_history_entry(&mut self, timestamp: f64) -> StepHistoryEntry {
        let step_name = self.workflow.nodes[self.current_step_index].name.clone();
        let run_index = self
            .step_execution_counts
            .get(&step_name)
            .copied()
            .unwrap_or(1);
        // 中断時点までに累積した token_usage は entry に残す。
        // current_step_token_usage 自体は take してクリアする
        // （post-commit 経路で参照されないため）。
        let token_usage = std::mem::take(&mut self.current_step_token_usage);
        step_history_entry_from_domain(workflow_history::aborted_step_history_entry(
            step_name,
            run_index,
            self.current_session_id.clone(),
            token_usage_to_domain(&token_usage),
            timestamp,
        ))
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
    pub(crate) fn make_aborted_parallel_history_entry(
        &self,
        timestamp: f64,
    ) -> Option<StepHistoryEntry> {
        let pr = self.parallel_run.as_ref()?;
        let parent_run_index = self
            .step_execution_counts
            .get(&pr.parent_step_name)
            .copied()
            .unwrap_or(1);
        let domain_parallel_run = self.domain_parallel_run()?;
        let domain_step_outputs = step_outputs_to_domain(&self.step_outputs);
        Some(step_history_entry_from_domain(
            workflow_history::aborted_parallel_history_entry(
                &domain_parallel_run,
                &domain_step_outputs,
                parent_run_index,
                timestamp,
            ),
        ))
    }

    /// 現在のステップの完了履歴エントリを生成し、トークン使用量をリセットする。
    pub(crate) fn make_step_history_entry(
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
        let token_usage = std::mem::take(&mut self.current_step_token_usage);
        let entry = workflow_history::completed_step_history_entry(
            workflow_history::CompletedStepHistoryInput {
                step_name: step_name.clone(),
                completed_at,
                result,
                session_id: self.current_session_id.clone(),
                token_usage: Some(token_usage_to_domain(&token_usage)),
                structured_output,
                run_index,
            },
        );
        if let Some(output) =
            workflow_history::step_output_from_completed_history_entry(&entry, output_contract)
        {
            self.step_outputs
                .insert(step_name, step_output_from_domain(output));
        }

        self.current_session_id = None;
        self.current_stall_observations.clear();
        step_history_entry_from_domain(entry)
    }

    pub(crate) fn record_step_session_start_failed(&mut self, reason: impl Into<String>) {
        let entry = self.make_step_history_entry(
            Some(workflow_history::session_start_failed_result(reason.into())),
            None,
            None,
        );
        self.step_history.push(entry);
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
    pub(crate) fn clear_step_outputs_for_new_execution(&mut self, step_index: usize) {
        let workflow = workflow_definition_to_domain(&self.workflow);
        for key in
            workflow_submission::step_output_keys_to_clear_for_new_execution(&workflow, step_index)
        {
            self.step_outputs.remove(&key);
        }
    }

    /// ロック内で次ステップへの advance を適用する（純粋な状態変更）。
    pub(crate) fn apply_advance(&mut self) -> StepOutcome {
        let decision = self.decide_next_step();
        match decision {
            NextStepDecision::Completed => {
                self.state = WorkflowExecutionState::Completed;
                self.updated_at = current_timestamp();
                StepOutcome::Persist(self.to_workflow_state())
            }
            NextStepDecision::TransitionTo(name) => {
                let idx = self
                    .workflow
                    .nodes
                    .iter()
                    .position(|step| step.name == name)
                    .expect("decide_next_step returned unknown step");
                self.apply_transition_index(idx, &name);
                self.step_outcome_for_current_step()
            }
        }
    }

    pub(crate) fn retry_current_step(&mut self) -> StepOutcome {
        let step_index = self.current_step_index;
        let step_name = self.workflow.nodes[step_index].name.clone();
        let completed_session_id = self.current_session_id.clone();
        self.state = WorkflowExecutionState::Running;
        *self.step_execution_counts.entry(step_name).or_insert(0) += 1;
        self.current_session_id = None;
        self.current_step_token_usage = TokenUsage::default();
        self.current_stall_observations.clear();
        self.clear_step_outputs_for_new_execution(step_index);
        self.updated_at = current_timestamp();
        StepOutcome::RetryCurrentStep {
            snapshot: self.to_workflow_state(),
            completed_session_id,
        }
    }

    /// ロック内で指定ステップへの遷移を適用する（サイクルガード検証含む）。
    pub(crate) fn apply_transition(
        &mut self,
        target_step_name: &str,
    ) -> Result<StepOutcome, WorkflowEngineError> {
        if self.is_terminal() {
            return Ok(StepOutcome::Persist(self.to_workflow_state()));
        }

        self.apply_transition_inner(target_step_name, 0)
    }

    fn apply_transition_inner(
        &mut self,
        target_step_name: &str,
        depth: usize,
    ) -> Result<StepOutcome, WorkflowEngineError> {
        let max_depth = self.workflow.nodes.len();
        if depth >= max_depth {
            self.state = WorkflowExecutionState::Failed {
                reason: format!("on_exhausted chain depth exceeded (max={max_depth})"),
                kind: WorkflowStepFailureKind::ValidationFailure,
                retry_count: None,
            };
            self.updated_at = current_timestamp();
            return Ok(StepOutcome::Persist(self.to_workflow_state()));
        }

        let idx = self
            .workflow
            .nodes
            .iter()
            .position(|step| step.name == target_step_name)
            .ok_or_else(|| {
                WorkflowEngineError::InvalidWorkflow(format!(
                    "Step '{target_step_name}' not found in workflow"
                ))
            })?;

        let guard_result = self.check_cycle_guard(target_step_name)?;
        match guard_result {
            CycleGuardResult::Exceeded {
                max_iterations,
                count,
                on_exhausted,
            } => {
                if let Some(fallback_target) = on_exhausted {
                    self.apply_transition_inner(&fallback_target, depth + 1)
                } else {
                    self.state = WorkflowExecutionState::Failed {
                        reason: format!(
                            "Cycle guard exceeded for step '{target_step_name}': max_iterations={max_iterations}, executed={count}"
                        ),
                        kind: WorkflowStepFailureKind::ValidationFailure,
                        retry_count: None,
                    };
                    self.updated_at = current_timestamp();
                    Ok(StepOutcome::Persist(self.to_workflow_state()))
                }
            }
            CycleGuardResult::Allowed => {
                self.apply_transition_index(idx, target_step_name);
                Ok(self.step_outcome_for_current_step())
            }
        }
    }

    fn apply_transition_index(&mut self, step_index: usize, step_name: &str) {
        self.current_step_index = step_index;
        self.state = WorkflowExecutionState::Running;
        *self
            .step_execution_counts
            .entry(step_name.to_string())
            .or_insert(0) += 1;
        self.current_session_id = None;
        self.current_stall_observations.clear();
        self.clear_step_outputs_for_new_execution(step_index);
        self.updated_at = current_timestamp();

        // resets_cycle_for: 遷移先ステップの設定に従い指定ステップのカウントをリセット
        if let Some(targets) = self.workflow.nodes[step_index].resets_cycle_for.clone() {
            for target in &targets {
                self.step_execution_counts.remove(target);
            }
        }
    }

    fn step_outcome_for_current_step(&self) -> StepOutcome {
        let step = &self.workflow.nodes[self.current_step_index];
        if step.is_fanout() {
            StepOutcome::StartParallel(self.to_workflow_state())
        } else if step.collect.is_some() {
            StepOutcome::ReduceAndTransition(self.to_workflow_state())
        } else {
            StepOutcome::TransitionAndStart(self.to_workflow_state())
        }
    }

    /// 次のステップ遷移先を判定する（純粋関数）。
    pub(crate) fn decide_next_step(&self) -> NextStepDecision {
        let workflow = workflow_definition_to_domain(&self.workflow);
        match workflow_transition::decide_next_node(&workflow, self.current_step_index) {
            workflow_transition::NextNodeDecision::Completed => NextStepDecision::Completed,
            workflow_transition::NextNodeDecision::TransitionTo(name) => {
                NextStepDecision::TransitionTo(name)
            }
        }
    }

    /// 指定ステップへの遷移時にサイクルガードを検証する（純粋関数）。
    pub(crate) fn check_cycle_guard(
        &self,
        target_step_name: &str,
    ) -> Result<CycleGuardResult, WorkflowEngineError> {
        let workflow = workflow_definition_to_domain(&self.workflow);
        match workflow_transition::check_cycle_guard(
            &workflow,
            &self.step_execution_counts,
            target_step_name,
        )
        .map_err(workflow_error_to_engine_error)?
        {
            workflow_transition::CycleGuardDecision::Allowed => Ok(CycleGuardResult::Allowed),
            workflow_transition::CycleGuardDecision::Exceeded {
                max_iterations,
                count,
                on_exhausted,
            } => Ok(CycleGuardResult::Exceeded {
                max_iterations,
                count,
                on_exhausted,
            }),
        }
    }

    /// turn_complete後のアクションを判定する（純粋関数）。
    #[cfg(test)]
    pub(crate) fn decide_turn_complete_action(&self, exit_code: i64) -> TurnCompleteAction {
        let workflow = workflow_definition_to_domain(&self.workflow);
        let state = workflow_execution_state_to_domain(&self.state);
        let action = workflow_transition::decide_turn_complete_action(
            &workflow,
            self.current_step_index,
            &state,
            exit_code,
        )
        .expect("current step index must reference workflow node");

        match action {
            workflow_transition::TurnCompleteDecision::NotRunning => TurnCompleteAction::NotRunning,
            workflow_transition::TurnCompleteDecision::SessionError {
                node_name,
                exit_code,
                kind,
            } => TurnCompleteAction::SessionError {
                step_name: node_name,
                exit_code,
                kind,
            },
            workflow_transition::TurnCompleteDecision::AutoEvaluate { rules, node_name } => {
                TurnCompleteAction::AutoEvaluate {
                    rules: rules.into_iter().map(transition_rule_from_domain).collect(),
                    step_name: node_name,
                }
            }
            workflow_transition::TurnCompleteDecision::WaitApproval => {
                TurnCompleteAction::WaitApproval
            }
            workflow_transition::TurnCompleteDecision::UnexpectedNodeKind { node_name, kind } => {
                TurnCompleteAction::UnexpectedNodeKind {
                    step_name: node_name,
                    kind,
                }
            }
        }
    }

    /// turn_complete後の状態変更 plan を domain service で組み立てる。
    pub(crate) fn plan_turn_complete_mutation(
        &self,
        exit_code: i64,
        failure_signal: Option<workflow_transition::SessionFailureSignal>,
    ) -> Result<workflow_transition::TurnCompleteMutationPlan, WorkflowEngineError> {
        let workflow = workflow_definition_to_domain(&self.workflow);
        let state = workflow_execution_state_to_domain(&self.state);
        workflow_transition::plan_turn_complete_mutation_with_signal(
            &workflow,
            self.current_step_index,
            &state,
            exit_code,
            failure_signal,
        )
        .map_err(workflow_error_to_engine_error)
    }

    /// approvalモードの判定ロジック（純粋関数）。
    #[cfg(test)]
    pub(crate) fn decide_approval_action(
        &self,
        decision: &ApprovalDecision,
    ) -> Result<ApprovalAction, WorkflowEngineError> {
        let workflow = workflow_definition_to_domain(&self.workflow);
        let state = workflow_execution_state_to_domain(&self.state);
        let decision = approval_decision_to_domain(decision);
        match workflow_transition::decide_approval_action(
            &workflow,
            self.current_step_index,
            &state,
            &decision,
        )
        .map_err(workflow_error_to_engine_error)?
        {
            workflow_transition::ApprovalTransitionDecision::Advance => Ok(ApprovalAction::Advance),
            workflow_transition::ApprovalTransitionDecision::TransitionTo(target) => {
                Ok(ApprovalAction::TransitionTo(target))
            }
        }
    }

    pub(crate) fn plan_approval_application(
        &self,
        decision: &ApprovalDecision,
        application: workflow_transition::ApprovalApplication,
    ) -> Result<workflow_transition::ApprovalApplicationPlan, WorkflowEngineError> {
        let workflow = workflow_definition_to_domain(&self.workflow);
        let state = workflow_execution_state_to_domain(&self.state);
        let decision = approval_decision_to_domain(decision);
        workflow_transition::plan_approval_application(
            &workflow,
            self.current_step_index,
            &state,
            &decision,
            application,
        )
        .map_err(workflow_error_to_engine_error)
    }
}

fn approval_decision_to_domain(decision: &ApprovalDecision) -> DomainApprovalDecision {
    match decision {
        ApprovalDecision::Approve => DomainApprovalDecision::Approve { comment: None },
        ApprovalDecision::Reject { comment } => DomainApprovalDecision::Reject {
            reason: comment.clone(),
        },
    }
}

/// 次のステップ遷移の判定結果。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NextStepDecision {
    /// ワークフロー完了（最後のステップを超えた）
    Completed,
    /// 指定ステップへ遷移
    TransitionTo(String),
}

/// サイクルガード検証結果。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CycleGuardResult {
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
#[cfg(test)]
pub(crate) enum TurnCompleteAction {
    /// AgentSessionがエラー終了 → Failed
    SessionError {
        step_name: String,
        exit_code: i64,
        kind: WorkflowStepFailureKind,
    },
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
    UnexpectedNodeKind {
        step_name: String,
        kind: workflow_domain::NodeKindName,
    },
    /// ワークフローが実行中でない → 何もしない
    NotRunning,
}

/// approvalモードのユーザー判定。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ApprovalDecision {
    Approve,
    Reject { comment: String },
}

/// approvalモードの判定結果（純粋関数用）。
#[derive(Debug, Clone, PartialEq)]
#[cfg(test)]
pub(crate) enum ApprovalAction {
    Advance,
    TransitionTo(String),
}
