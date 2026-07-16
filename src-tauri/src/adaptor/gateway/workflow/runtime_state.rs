use std::collections::HashMap;

use crate::adaptor::gateway::workflow::domain_mapping::{
    artifacts_to_domain, node_history_entries_to_domain, node_history_entry_from_domain,
    runtime_artifact_from_domain, runtime_execution_state_to_domain, token_usage_from_domain,
    token_usage_to_domain, workflow_definition_to_domain,
};
use crate::adaptor::gateway::workflow::engine_error::{
    workflow_error_to_engine_error, WorkflowEngineError,
};
use crate::adaptor::gateway::workflow::engine_start_guard;
use crate::adaptor::gateway::workflow::event::FanoutParentRef;
use crate::adaptor::gateway::workflow::node_settings::WorkflowDefaults;
use crate::adaptor::gateway::workflow::runtime_commit::NodeOutcome;
use crate::adaptor::gateway::workflow::schema::{NodeKindName, WorkflowDefinitionYaml};
use crate::adaptor::gateway::workflow::state::{
    NodeExecution, NodeExecutionFailure, NodeExecutionStatus, NodeHistoryEntry,
    NodeStallObservation, RuntimeArtifact, RuntimeCommitSnapshot, RuntimeExecutionState,
    TokenUsage,
};
use crate::domain::workflow as workflow_domain;
use crate::domain::workflow::services::history as workflow_history;
use crate::domain::workflow::services::projection as workflow_projection;
use crate::domain::workflow::services::routing as workflow_routing;
use crate::domain::workflow::services::submission as workflow_submission;
use crate::domain::workflow::services::transition as workflow_transition;
use crate::domain::workflow::{FailureDisposition, NodeExecutionFailureKind};
use crate::usecase::agent_session::status::current_timestamp;

/// ワークフロー実行の内部状態。
#[derive(Clone)]
pub(crate) struct WorkflowExecution {
    /// `RuntimeCommitSnapshot.execution_id` を `execution_id` として昇格させた識別子。
    /// `WorkflowRuntimeService.executions` の HashMap キーと一致する。
    pub(crate) id: String,
    pub(crate) workflow: WorkflowDefinitionYaml,
    pub(crate) state: RuntimeExecutionState,
    pub(crate) current_node_index: usize,
    pub(crate) node_execution_counts: HashMap<String, u32>,
    pub(crate) node_history: Vec<NodeHistoryEntry>,
    /// node / 並列子 node 起動時の継承デフォルト（permission_mode / backend_id / selected_model）。
    /// `start_workflow` 時に capture し、以降は session_store を読み直さない（in-memory のみ）。
    pub(crate) workflow_defaults: WorkflowDefaults,
    /// execution が紐づく worktree。HashMap キーではなく属性として保持する。
    /// `find_by_worktree` / `find_by_worktree_mut` が worktree 起点の lookup で参照する。
    pub(crate) worktree_path: String,
    pub(crate) created_from: workflow_domain::ExecutionOrigin,
    pub(crate) error_reason: Option<String>,
    pub(crate) started_at: f64,
    pub(crate) updated_at: f64,
    /// 現在のステップに対応するAgentSessionのセッションID。
    pub(crate) current_session_id: Option<String>,
    /// 現在のステップで累計したトークン使用量。
    pub(crate) current_node_token_usage: TokenUsage,
    /// node_name → 最新RuntimeArtifact のマップ。
    pub(crate) artifacts: HashMap<String, RuntimeArtifact>,
    /// event log / UI の第一級 read model となる node 実行列。
    /// fanout child も同じ列へ格納し、`fanout_parent` から親子関係を導出する。
    pub(crate) node_executions: Vec<NodeExecution>,
    /// ワークフロー実行時のタスク内容（テンプレート変数 {{ request }} の展開に使用）。
    pub(crate) request: Option<String>,
    /// 並列実行中の場合の状態。
    pub(crate) fanout_runtime: Option<FanoutRuntimeState>,
    /// 現在実行中の node session で観測した非終端 stall signal。
    pub(crate) current_stall_observations: Vec<NodeStallObservation>,
}

/// 並列実行中の内部状態。
#[derive(Clone)]
pub(crate) struct FanoutRuntimeState {
    pub(crate) parent_node_name: String,
    pub(crate) parent_node_execution_id: String,
    pub(crate) children: Vec<FanoutChildRuntime>,
}

/// 並列子ステップの実行状態。
#[derive(Clone)]
pub(crate) struct FanoutChildRuntime {
    pub(crate) node_execution_id: String,
    pub(crate) node_name: String,
    pub(crate) session_id: String,
    pub(crate) state: FanoutChildRuntimeState,
    pub(crate) result: Option<String>,
    pub(crate) artifact: Option<serde_json::Value>,
    pub(crate) contract: Option<String>,
    pub(crate) failure_kind: Option<NodeExecutionFailureKind>,
    pub(crate) failure_disposition: Option<FailureDisposition>,
    pub(crate) token_usage: TokenUsage,
    pub(crate) attempt: u32,
    pub(crate) completed_at: Option<f64>,
}

/// 並列子ステップの状態。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FanoutChildRuntimeState {
    Running,
    Completed,
    Failed,
    Interrupted,
}

/// session_workflow_refsの値型。session_id → execution_id の逆引き索引。
///
/// parent ChatSession 機構撤去後は node session のみが登録されるため種別区別は不要
/// （Spec issues-929: 「逐次 node と並列子 node は単一経路で扱う」/ Spec issues-1011:
/// engine 内部キーは execution_id に統一）。worktree_path は `WorkflowExecution.worktree_path`
/// 属性として exec から取得する。
#[derive(Clone)]
pub(crate) struct SessionWorkflowRef {
    /// engine.executions の HashMap キー（= `WorkflowExecution.id` = `execution_id`）。
    pub(crate) execution_id: String,
}

impl WorkflowExecution {
    /// ワークフローが実行中（Running または WaitingApproval）かどうかを返す。
    pub(crate) fn is_active(&self) -> bool {
        matches!(
            self.state,
            RuntimeExecutionState::Running | RuntimeExecutionState::WaitingApproval
        )
    }

    /// ワークフローが終了状態（Completed / Failed / Aborted）かどうかを返す。
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            RuntimeExecutionState::Completed
                | RuntimeExecutionState::Failed { .. }
                | RuntimeExecutionState::Aborted
        )
    }

    pub(crate) fn start_node_execution(
        &mut self,
        node_name: String,
        kind: NodeKindName,
        attempt: u32,
        fanout_parent: Option<FanoutParentRef>,
        node_execution_id: Option<String>,
        timestamp: f64,
    ) -> String {
        let id = node_execution_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.node_executions.push(NodeExecution {
            id: id.clone(),
            execution_id: self.id.clone(),
            node_name,
            kind,
            attempt,
            status: NodeExecutionStatus::Running,
            session_id: None,
            display_command: None,
            artifact: None,
            token_usage: None,
            failure: None,
            fanout_parent,
            started_at: timestamp,
            completed_at: None,
        });
        id
    }

    pub(crate) fn start_current_node_execution(&mut self, timestamp: f64) -> String {
        let node_name = self.workflow.nodes[self.current_node_index].name.clone();
        let kind = self.workflow.nodes[self.current_node_index].kind_name();
        let attempt = self
            .node_execution_counts
            .get(&node_name)
            .copied()
            .unwrap_or(1);
        self.start_node_execution(node_name, kind, attempt, None, None, timestamp)
    }

    pub(crate) fn active_current_node_execution_id(&self) -> Option<&str> {
        let node = self.workflow.nodes.get(self.current_node_index)?;
        let attempt = self
            .node_execution_counts
            .get(&node.name)
            .copied()
            .unwrap_or(1);
        self.node_executions
            .iter()
            .rev()
            .find(|execution| {
                execution.node_name == node.name
                    && execution.attempt == attempt
                    && execution.fanout_parent.is_none()
                    && execution.status.is_active()
            })
            .map(|execution| execution.id.as_str())
    }

    pub(crate) fn complete_node_execution(
        &mut self,
        node_execution_id: &str,
        artifact: Option<serde_json::Value>,
        token_usage: Option<TokenUsage>,
        timestamp: f64,
    ) {
        if let Some(execution) = self
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == node_execution_id)
        {
            execution.status = NodeExecutionStatus::Succeeded;
            execution.artifact = artifact;
            execution.token_usage = token_usage;
            execution.failure = None;
            execution.completed_at = Some(timestamp);
        }
    }

    pub(crate) fn fail_node_execution(
        &mut self,
        node_execution_id: &str,
        reason: String,
        kind: NodeExecutionFailureKind,
        timestamp: f64,
    ) {
        if let Some(execution) = self
            .node_executions
            .iter_mut()
            .find(|execution| execution.id == node_execution_id)
        {
            execution.status = NodeExecutionStatus::Failed;
            execution.failure = Some(NodeExecutionFailure { reason, kind });
            execution.completed_at = Some(timestamp);
        }
    }

    /// ワークフロー開始の事前条件を検証する（純粋関数）。
    ///
    /// executions ロック内の defense-in-depth で呼ばれる。Execution Store の active index と
    /// in-memory executions 表が一時的に不整合な場合に、最終的な atomic guard として機能する。
    pub(crate) fn validate_start(
        workflow: &WorkflowDefinitionYaml,
        existing: Option<&WorkflowExecution>,
    ) -> Result<(), WorkflowEngineError> {
        let existing_active_workflow_name = existing
            .filter(|existing| existing.is_active())
            .map(|existing| existing.workflow.name.as_str());
        engine_start_guard::validate_start(workflow, existing_active_workflow_name)
    }

    /// 永続化用の `RuntimeCommitSnapshot` に変換する。
    pub(crate) fn to_commit_snapshot(&self) -> RuntimeCommitSnapshot {
        let domain_history = node_history_entries_to_domain(&self.node_history);
        let total_token_usage =
            token_usage_from_domain(&workflow_projection::total_token_usage(&domain_history));
        RuntimeCommitSnapshot {
            execution_id: self.id.clone(),
            workflow_name: self.workflow.name.clone(),
            worktree_path: self.worktree_path.clone(),
            created_from: self.created_from,
            request: self.request.clone().unwrap_or_default(),
            error_reason: match &self.state {
                RuntimeExecutionState::Failed { reason, .. } => Some(reason.clone()),
                RuntimeExecutionState::Interrupted => self.error_reason.clone(),
                _ => None,
            },
            state: self.state.clone(),
            current_node_index: self.current_node_index,
            current_node_name: self.workflow.nodes[self.current_node_index].name.clone(),
            current_session_id: self.current_session_id.clone(),
            node_history: self.node_history.clone(),
            node_execution_counts: self.node_execution_counts.clone(),
            workflow_definition: self.workflow.clone(),
            total_token_usage,
            artifacts: self.artifacts.clone(),
            node_executions: self.node_executions.clone(),
            started_at: self.started_at,
            updated_at: self.updated_at,
        }
    }

    fn domain_fanout_runtime(&self) -> Option<workflow_domain::FanoutRuntimeState> {
        self.fanout_runtime
            .as_ref()
            .map(|fanout_runtime| workflow_domain::FanoutRuntimeState {
                parent_node_name: fanout_runtime.parent_node_name.clone(),
                children: fanout_runtime
                    .children
                    .iter()
                    .map(|child| workflow_domain::FanoutChildRuntime {
                        node_name: child.node_name.clone(),
                        session_id: child.session_id.clone(),
                        state: match child.state {
                            FanoutChildRuntimeState::Running => {
                                workflow_domain::FanoutChildRuntimeState::Running
                            }
                            FanoutChildRuntimeState::Completed => {
                                workflow_domain::FanoutChildRuntimeState::Completed
                            }
                            FanoutChildRuntimeState::Failed => {
                                workflow_domain::FanoutChildRuntimeState::Failed
                            }
                            FanoutChildRuntimeState::Interrupted => {
                                workflow_domain::FanoutChildRuntimeState::Interrupted
                            }
                        },
                        result: child.result.clone(),
                        artifact: child.artifact.clone(),
                        contract: child.contract.clone(),
                        failure_kind: child.failure_kind,
                        failure_disposition: child.failure_disposition,
                        token_usage: token_usage_to_domain(&child.token_usage),
                        attempt: child.attempt,
                    })
                    .collect(),
            })
    }

    /// spec issues-1023: 中断された通常 node の `node_history` entry を作る。
    ///
    /// 既存 `make_node_history_entry` の副作用（`current_session_id` reset /
    /// artifacts の前段クリーンアップ等）を**起こさない**点が違い。`abort_workflow_by_execution_id`
    /// は post-commit で interrupt_agent や cleanup を行うため、reset 系は
    /// `finalize_terminal_transition_after_required_append` 経路に任せる。
    ///
    /// session_id を entry にコピーすることで、`node_history` 由来の session log 到達経路
    /// （domain node_session_projection の `collect_node_session_ids`）を復活させる。
    pub(crate) fn make_aborted_history_entry(&mut self, timestamp: f64) -> NodeHistoryEntry {
        let node_name = self.workflow.nodes[self.current_node_index].name.clone();
        let attempt = self
            .node_execution_counts
            .get(&node_name)
            .copied()
            .unwrap_or(1);
        // 中断時点までに累積した token_usage は entry に残す。
        // current_node_token_usage 自体は take してクリアする
        // （post-commit 経路で参照されないため）。
        let token_usage = std::mem::take(&mut self.current_node_token_usage);
        node_history_entry_from_domain(workflow_history::aborted_node_history_entry(
            node_name,
            attempt,
            self.current_session_id.clone(),
            token_usage_to_domain(&token_usage),
            timestamp,
        ))
    }

    /// spec issues-1023: 中断された fanout parent node の `node_history` entry を作る。
    ///
    /// `fanout_runtime.children` 全件を `fanout_children` に snapshot する。
    /// 完了済み child（`FanoutChildRuntimeState::Completed`）は `state="completed"`、
    /// それ以外（Running / Failed / Interrupted）は `state="aborted"` として記録し、
    /// session_id は child runtime / artifacts から維持する（session log 到達経路維持）。
    ///
    /// 呼出し側は本関数で entry を組み立てた後、`self.fanout_runtime = None;` を明示
    /// セットし、fanout の live child state を残さないこと。
    pub(crate) fn make_aborted_fanout_history_entry(
        &self,
        timestamp: f64,
    ) -> Option<NodeHistoryEntry> {
        let pr = self.fanout_runtime.as_ref()?;
        let parent_attempt = self
            .node_execution_counts
            .get(&pr.parent_node_name)
            .copied()
            .unwrap_or(1);
        let domain_fanout_runtime = self.domain_fanout_runtime()?;
        let domain_artifacts = artifacts_to_domain(&self.artifacts);
        Some(node_history_entry_from_domain(
            workflow_history::aborted_fanout_history_entry(
                &domain_fanout_runtime,
                &domain_artifacts,
                parent_attempt,
                timestamp,
            ),
        ))
    }

    /// 現在のステップの完了履歴エントリを生成し、トークン使用量をリセットする。
    pub(crate) fn make_node_history_entry(
        &mut self,
        result: Option<String>,
        artifact: Option<serde_json::Value>,
        contract: Option<String>,
    ) -> NodeHistoryEntry {
        let node_name = self.workflow.nodes[self.current_node_index].name.clone();
        let attempt = self
            .node_execution_counts
            .get(&node_name)
            .copied()
            .unwrap_or(1);
        let completed_at = current_timestamp();
        let token_usage = std::mem::take(&mut self.current_node_token_usage);
        let entry = workflow_history::completed_node_history_entry(
            workflow_history::CompletedNodeHistoryInput {
                node_name: node_name.clone(),
                completed_at,
                result,
                session_id: self.current_session_id.clone(),
                token_usage: Some(token_usage_to_domain(&token_usage)),
                artifact,
                attempt,
            },
        );
        if let Some(output) =
            workflow_history::artifact_from_completed_history_entry(&entry, contract)
        {
            self.artifacts
                .insert(node_name, runtime_artifact_from_domain(output));
        }

        self.current_session_id = None;
        self.current_stall_observations.clear();
        node_history_entry_from_domain(entry)
    }

    /// 指定インデックスの node が新しい実行を開始する瞬間に、当該 node の
    /// 前回出力を `artifacts` から破棄する。並列ブロックの場合は
    /// 親ブロック名と全子 node 名を一括で削除する。
    ///
    /// 同一 node がループで再実行される際、前回値が残ったままになると
    /// `input_reference` / `inject_artifacts` が前回値を引いてしまい、新しい実行で
    /// `artifact` が更新されないケースや LLM が前回ターンの
    /// `<workflow_output>` を引用してきたケースで Contract 違反が
    /// 「正常完了（Done）」扱いされる不具合の原因となる。
    pub(crate) fn clear_artifacts_for_new_execution(&mut self, node_index: usize) {
        let workflow = workflow_definition_to_domain(&self.workflow);
        for key in workflow_submission::artifact_keys_to_clear_for_new_node_execution(
            &workflow, node_index,
        ) {
            self.artifacts.remove(&key);
        }
    }

    /// ロック内で次ステップへの advance を適用する（純粋な状態変更）。
    pub(crate) fn apply_advance(&mut self) -> NodeOutcome {
        let decision = self.decide_next_node();
        match decision {
            NextNodeDecision::Completed => {
                self.state = RuntimeExecutionState::Completed;
                self.updated_at = current_timestamp();
                NodeOutcome::Persist(self.to_commit_snapshot())
            }
            NextNodeDecision::Failed { reason } => self.fail_validation(reason),
            NextNodeDecision::TransitionTo(name) => {
                let idx = self
                    .workflow
                    .nodes
                    .iter()
                    .position(|node| node.name == name)
                    .expect("decide_next_node returned unknown node");
                self.apply_transition_index(idx, &name);
                self.node_outcome_for_current_node()
            }
        }
    }

    pub(crate) fn retry_current_node(&mut self) -> NodeOutcome {
        let node_index = self.current_node_index;
        let node_name = self.workflow.nodes[node_index].name.clone();
        let completed_session_id = self.current_session_id.clone();
        self.state = RuntimeExecutionState::Running;
        *self.node_execution_counts.entry(node_name).or_insert(0) += 1;
        self.current_session_id = None;
        self.current_node_token_usage = TokenUsage::default();
        self.current_stall_observations.clear();
        self.clear_artifacts_for_new_execution(node_index);
        self.updated_at = current_timestamp();
        self.start_current_node_execution(self.updated_at);
        NodeOutcome::RetryCurrentNode {
            snapshot: self.to_commit_snapshot(),
            completed_session_id,
        }
    }

    fn fail_validation(&mut self, reason: impl Into<String>) -> NodeOutcome {
        self.state = RuntimeExecutionState::Failed {
            reason: reason.into(),
            kind: NodeExecutionFailureKind::ValidationFailure,
            retry_count: None,
        };
        self.updated_at = current_timestamp();
        NodeOutcome::Persist(self.to_commit_snapshot())
    }

    fn apply_transition_index(&mut self, node_index: usize, node_name: &str) {
        self.current_node_index = node_index;
        self.state = RuntimeExecutionState::Running;
        *self
            .node_execution_counts
            .entry(node_name.to_string())
            .or_insert(0) += 1;
        self.current_session_id = None;
        self.current_stall_observations.clear();
        self.clear_artifacts_for_new_execution(node_index);
        self.updated_at = current_timestamp();
        self.start_current_node_execution(self.updated_at);
    }

    fn node_outcome_for_current_node(&self) -> NodeOutcome {
        let node = &self.workflow.nodes[self.current_node_index];
        if node.is_fanout() {
            NodeOutcome::StartFanout(self.to_commit_snapshot())
        } else {
            NodeOutcome::TransitionAndStart(self.to_commit_snapshot())
        }
    }

    /// 次のステップ遷移先を判定する（純粋関数）。
    pub(crate) fn decide_next_node(&self) -> NextNodeDecision {
        let workflow = workflow_definition_to_domain(&self.workflow);
        match workflow_routing::route(
            &workflow,
            self.current_node_index,
            self.current_node_artifact(),
            &self.node_execution_counts,
        ) {
            Ok(workflow_routing::RouteDecision::Completed) => NextNodeDecision::Completed,
            Ok(workflow_routing::RouteDecision::TransitionTo(name)) => {
                NextNodeDecision::TransitionTo(name)
            }
            Err(err) => NextNodeDecision::Failed {
                reason: err.to_string(),
            },
        }
    }

    fn current_node_artifact(&self) -> Option<&serde_json::Value> {
        let node_name = &self.workflow.nodes.get(self.current_node_index)?.name;
        self.artifacts
            .get(node_name)
            .and_then(|output| output.artifact.as_ref())
    }

    /// 指定 node への遷移時に loop_guard を検証する（純粋関数）。
    #[cfg(test)]
    pub(crate) fn check_loop_guard(
        &self,
        target_node_name: &str,
    ) -> Result<LoopGuardResult, WorkflowEngineError> {
        let workflow = workflow_definition_to_domain(&self.workflow);
        let decision = workflow_routing::guarded_target(
            &workflow,
            target_node_name.to_string(),
            &self.node_execution_counts,
        )
        .map_err(workflow_error_to_engine_error)?;
        if matches!(
            decision,
            workflow_routing::RouteDecision::TransitionTo(ref name) if name == target_node_name
        ) {
            return Ok(LoopGuardResult::Allowed);
        }
        let node = workflow
            .nodes
            .iter()
            .find(|node| node.name == target_node_name)
            .ok_or_else(|| {
                WorkflowEngineError::InvalidWorkflow(format!(
                    "Node '{target_node_name}' not found in workflow"
                ))
            })?;
        let Some((max_iterations, on_exhausted)) = workflow_routing::loop_guard(node) else {
            return Ok(LoopGuardResult::Allowed);
        };
        let count = self
            .node_execution_counts
            .get(target_node_name)
            .copied()
            .unwrap_or(0);
        Ok(LoopGuardResult::Exceeded {
            max_iterations,
            count,
            on_exhausted: Some(on_exhausted.to_string()),
        })
    }

    /// turn_complete後のアクションを判定する（純粋関数）。
    #[cfg(test)]
    pub(crate) fn decide_turn_complete_action(&self, exit_code: i64) -> TurnCompleteAction {
        let workflow = workflow_definition_to_domain(&self.workflow);
        let state = runtime_execution_state_to_domain(&self.state);
        let action = workflow_transition::decide_turn_complete_action(
            &workflow,
            self.current_node_index,
            &state,
            exit_code,
        )
        .expect("current node index must reference workflow node");

        match action {
            workflow_transition::TurnCompleteDecision::NotRunning => TurnCompleteAction::NotRunning,
            workflow_transition::TurnCompleteDecision::SessionError {
                node_name,
                exit_code,
                kind,
            } => TurnCompleteAction::SessionError {
                node_name,
                exit_code,
                kind,
            },
            workflow_transition::TurnCompleteDecision::AutoEvaluate { node_name } => {
                TurnCompleteAction::AutoEvaluate { node_name }
            }
            workflow_transition::TurnCompleteDecision::WaitApproval => {
                TurnCompleteAction::WaitApproval
            }
            workflow_transition::TurnCompleteDecision::UnexpectedNodeKind { node_name, kind } => {
                TurnCompleteAction::UnexpectedNodeKind { node_name, kind }
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
        let state = runtime_execution_state_to_domain(&self.state);
        workflow_transition::plan_turn_complete_mutation_with_signal(
            &workflow,
            self.current_node_index,
            &state,
            exit_code,
            failure_signal,
        )
        .map_err(workflow_error_to_engine_error)
    }

    /// approvalモードの判定ロジック（純粋関数）。
    #[cfg(test)]
    pub(crate) fn decide_approve_action(&self) -> Result<(), WorkflowEngineError> {
        let workflow = workflow_definition_to_domain(&self.workflow);
        let state = runtime_execution_state_to_domain(&self.state);
        workflow_transition::decide_approve_action(&workflow, self.current_node_index, &state)
            .map_err(workflow_error_to_engine_error)
    }

    pub(crate) fn plan_approval_application(
        &self,
        application: workflow_transition::ApprovalApplication,
    ) -> Result<workflow_transition::ApprovalApplicationPlan, WorkflowEngineError> {
        let workflow = workflow_definition_to_domain(&self.workflow);
        let state = runtime_execution_state_to_domain(&self.state);
        workflow_transition::plan_approval_application(
            &workflow,
            self.current_node_index,
            &state,
            application,
        )
        .map_err(workflow_error_to_engine_error)
    }
}

/// 次のステップ遷移の判定結果。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NextNodeDecision {
    /// ワークフロー完了（最後のステップを超えた）
    Completed,
    /// 指定ステップへ遷移
    TransitionTo(String),
    /// routing 不変条件違反による失敗
    Failed { reason: String },
}

/// サイクルガード検証結果。
#[derive(Debug, Clone, PartialEq)]
#[cfg(test)]
pub(crate) enum LoopGuardResult {
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
        node_name: String,
        exit_code: i64,
        kind: NodeExecutionFailureKind,
    },
    /// agent ノード → タグ検出して遷移
    AutoEvaluate { node_name: String },
    /// approval ノード → WaitingApproval
    WaitApproval,
    /// 設計上 turn_complete に流入してはならない node 種別を検出した
    /// （`validate_start` などの上流ガードで弾くべきケース）。`Failed` に遷移させ、
    /// `SessionError { exit_code: 0 }` の「正常終了」セマンティクスと混同しないようにする。
    UnexpectedNodeKind {
        node_name: String,
        kind: workflow_domain::NodeKindName,
    },
    /// ワークフローが実行中でない → 何もしない
    NotRunning,
}
