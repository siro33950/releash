use std::collections::HashMap;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex;

use super::runtime_session as workflow_runtime_session;
#[cfg(test)]
use super::runtime_session::resolve_step_model_with_registry;
#[cfg(test)]
use super::step_session_boundary::StepSessionInfo;
#[cfg(test)]
use super::step_session_boundary::{dispatch_session_start, SessionStartGate};
use super::step_session_boundary::{RealStepSessionDeps, StepSessionDeps};
use crate::adaptor::gateway::workflow::approval_runtime as workflow_approval_runtime;
use crate::adaptor::gateway::workflow::domain_mapping::transition_rule_from_domain;
use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::event::{ApprovalDecisionRecord, WorkflowEvent};
use crate::adaptor::gateway::workflow::event_log_query::{
    request_event_already_recorded, RequestEventKind, RequestEventLookupError,
};
use crate::adaptor::gateway::workflow::event_log_writer as workflow_event_log_writer;
use crate::adaptor::gateway::workflow::execution_registry::{
    find_any_by_worktree, find_by_worktree, find_by_worktree_mut, ExecutionStateTarget,
};
use crate::adaptor::gateway::workflow::external_execution_restore as workflow_external_restore;
use crate::adaptor::gateway::workflow::log::WorkflowEventLog;
use crate::adaptor::gateway::workflow::orphan_recovery as workflow_orphan_recovery;
use crate::adaptor::gateway::workflow::output_submission as workflow_output_submission;
use crate::adaptor::gateway::workflow::parallel_runtime as workflow_parallel_runtime;
use crate::adaptor::gateway::workflow::prompt_rendering as workflow_prompt;
use crate::adaptor::gateway::workflow::resolver::{
    ManagedWorktreeResolver, WorkflowDefinitionResolver,
};
#[cfg(test)]
use crate::adaptor::gateway::workflow::resolver::{
    ManagedWorktreeResolverError, WorkflowDefinitionResolverError,
};
use crate::adaptor::gateway::workflow::route_context::CommandCommitContext;
use crate::adaptor::gateway::workflow::run::{
    RunStatus, RunStore, RunStoreError, TerminalRunStatus, TriggerSource, WorkflowRun,
};
use crate::adaptor::gateway::workflow::runtime_commit::{
    self as workflow_runtime_commit, AbortOutcome, AbortTargetLookup, CommandMutationRollback,
    RequiredEventCommit, StepOutcome,
};
use crate::adaptor::gateway::workflow::runtime_events as workflow_runtime_events;
use crate::adaptor::gateway::workflow::runtime_state::{
    ApprovalDecision, ParallelChildState, SessionWorkflowRef, WorkflowExecution,
};
#[cfg(test)]
use crate::adaptor::gateway::workflow::runtime_state::{CycleGuardResult, NextStepDecision};
#[cfg(test)]
use crate::adaptor::gateway::workflow::runtime_state::{ParallelChildRun, ParallelRunState};
#[cfg(test)]
use crate::adaptor::gateway::workflow::schema::NodeDefinition;
#[cfg(test)]
use crate::adaptor::gateway::workflow::schema::NodeType;
use crate::adaptor::gateway::workflow::schema::{TransitionRule, Workflow};
use crate::adaptor::gateway::workflow::secret_source;
#[cfg(test)]
use crate::adaptor::gateway::workflow::state::ParallelStepState;
use crate::adaptor::gateway::workflow::state::{
    StepHistoryEntry, StepOutput, TokenUsage, WorkflowExecutionState, WorkflowState,
};
#[cfg(test)]
use crate::adaptor::gateway::workflow::step_settings::resolve_step_settings;
#[cfg(test)]
use crate::adaptor::gateway::workflow::step_settings::ResolvedStepSettings;
use crate::adaptor::gateway::workflow::step_settings::WorkflowDefaults;
use crate::adaptor::gateway::workflow::turn_completion;
use crate::domain::workflow::services::contract as workflow_contract;
use crate::domain::workflow::services::history::RuntimeStartFailureKind;
use crate::domain::workflow::services::secret_masker as workflow_secret_masker;
use crate::domain::workflow::services::transition as workflow_transition;
use crate::domain::workflow::OutcomeCommitMode;
use crate::domain::workflow::WorkflowStepContext;
use crate::domain::workflow::STEP_STATE_FAILED;
#[cfg(test)]
use crate::domain::workflow::STEP_STATE_RUNNING;
use crate::infrastructure::agent_session::runtime::AgentProcessMap;
use crate::permission::PermissionMode;
use crate::usecase::agent_session::session::SessionStore;
use crate::usecase::agent_session::status::current_timestamp;

use super::event_projection::reconstruct_state_from_events;

const MAX_CONTRACT_REPAIR_ATTEMPTS: u32 = 2;

fn request_event_lookup_error_to_engine_error(err: RequestEventLookupError) -> WorkflowEngineError {
    match err {
        RequestEventLookupError::InvalidRunId(message)
        | RequestEventLookupError::InvalidRequestId(message) => {
            WorkflowEngineError::ValidationError(message)
        }
        RequestEventLookupError::ReadLog(message) => WorkflowEngineError::SessionStore(message),
    }
}

#[cfg(test)]
struct TestWorkflowDefinitionResolver;

#[cfg(test)]
#[async_trait::async_trait]
impl WorkflowDefinitionResolver for TestWorkflowDefinitionResolver {
    async fn resolve(&self, file_stem: &str) -> Result<Workflow, WorkflowDefinitionResolverError> {
        let load_stem = file_stem.to_string();
        tokio::task::spawn_blocking(move || {
            let dir = crate::adaptor::gateway::workflow::storage::workflows_dir();
            let facets_base = crate::adaptor::gateway::workflow::facet::facets_base_dir();
            let file_path = dir.join(format!("{load_stem}.yml"));
            if file_path.exists() {
                match crate::adaptor::gateway::workflow::storage::load_workflow(
                    &file_path,
                    &facets_base,
                ) {
                    Ok(wf) => return Ok(wf),
                    Err(e)
                        if crate::adaptor::gateway::workflow::builtin::is_builtin_workflow(
                            &load_stem,
                        ) =>
                    {
                        log::warn!(
                            "user-side workflow '{load_stem}' failed to load ({e}); falling back to builtin"
                        );
                    }
                    Err(e) => {
                        return Err(WorkflowDefinitionResolverError::InvalidWorkflow(
                            e.to_string(),
                        ));
                    }
                }
            }
            crate::adaptor::gateway::workflow::builtin::load_builtin_workflow_resolved(&load_stem)
                .map_err(|e| WorkflowDefinitionResolverError::InvalidWorkflow(e.to_string()))?
                .ok_or_else(|| {
                    WorkflowDefinitionResolverError::InvalidWorkflow(format!(
                        "ワークフロー '{load_stem}' が見つかりません"
                    ))
                })
        })
        .await
        .map_err(|e| {
            WorkflowDefinitionResolverError::Infrastructure(format!("task join error: {e}"))
        })?
    }
}

#[cfg(test)]
struct PassthroughManagedWorktreeResolver;

#[cfg(test)]
#[async_trait::async_trait]
impl ManagedWorktreeResolver for PassthroughManagedWorktreeResolver {
    async fn resolve(&self, worktree_path: String) -> Result<String, ManagedWorktreeResolverError> {
        Ok(worktree_path)
    }
}

/// ワークフローのステップを順次実行するステートマシンエンジン。
pub struct WorkflowRuntimeService {
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
    #[cfg(test)]
    abort_after_lookup_gate: Mutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>,
}

struct ParallelChildStartedLogObserver<'a, R: tauri::Runtime> {
    engine: &'a WorkflowRuntimeService,
    app: &'a tauri::AppHandle<R>,
    execution_id: &'a str,
    workflow_name: &'a str,
    parent_step_name: &'a str,
}

impl<R: tauri::Runtime> workflow_runtime_session::ParallelChildTurnObserver
    for ParallelChildStartedLogObserver<'_, R>
{
    fn child_turn_started(
        &self,
        started: workflow_runtime_session::ParallelChildStartedRuntime<'_>,
    ) {
        self.engine.write_log(
            self.app,
            WorkflowEvent::ParallelChildStarted {
                run_id: self.execution_id.to_string(),
                workflow_name: self.workflow_name.to_string(),
                parent_node_name: self.parent_step_name.to_string(),
                child_node_name: started.step_name.to_string(),
                session_id: started.session_id.to_string(),
                execution_count: started.execution_count,
                timestamp: current_timestamp(),
            },
        );
    }
}

// [08] `lookup_step_output_contract` は domain の contract service に移動済み。
// engine と CLI の双方が同じ domain service を参照するため、本モジュールではメモのみ残す。

impl WorkflowRuntimeService {
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
            #[cfg(test)]
            abort_after_lookup_gate: Mutex::new(None),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test() -> Self {
        Self::new(
            Arc::new(TestWorkflowDefinitionResolver),
            Arc::new(PassthroughManagedWorktreeResolver),
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
        if let Some(data_dir) = self.run_store.data_dir_for_test().await {
            WorkflowEventLog::new(&data_dir)
                .append(&WorkflowEvent::RunStarted {
                    run_id: run_id.clone(),
                    workflow_name: workflow.name.clone(),
                    workflow_file_stem: workflow.name.clone(),
                    worktree_path: worktree_path.clone(),
                    workflow_definition: workflow.clone(),
                    timestamp: now,
                })
                .unwrap();
        }
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

    #[cfg(test)]
    pub(crate) async fn pause_abort_after_lookup_for_test(
        &self,
        lookup_completed: Arc<tokio::sync::Notify>,
        continue_precommit: Arc<tokio::sync::Notify>,
    ) {
        *self.abort_after_lookup_gate.lock().await = Some((lookup_completed, continue_precommit));
    }

    #[cfg(test)]
    async fn wait_abort_after_lookup_for_test(&self) {
        let gate = self.abort_after_lookup_gate.lock().await.take();
        if let Some((lookup_completed, continue_precommit)) = gate {
            lookup_completed.notify_one();
            continue_precommit.notified().await;
        }
    }

    #[cfg(test)]
    pub(crate) async fn contains_execution_for_test(&self, run_id: &str) -> bool {
        self.executions.lock().await.contains_key(run_id)
    }

    #[cfg(test)]
    pub(crate) async fn executions_len_for_test(&self) -> usize {
        self.executions.lock().await.len()
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
    #[cfg(test)]
    pub async fn run_id_for_worktree(&self, worktree_path: &str) -> Option<String> {
        self.run_store.resolve_run_by_worktree(worktree_path).await
    }

    /// run_id から worktree_path を解決する。active な run のみならず、終了済み run も
    /// `workflow_runs/{run_id}.json` から metadata を読み込んで返す。
    /// Tauri command 経路で run_id 主語の操作を内部 worktree_path に解決する際に使用する。
    #[cfg(test)]
    pub async fn resolve_worktree_by_run(&self, run_id: &str) -> Option<String> {
        self.run_store.resolve_worktree_by_run(run_id).await
    }

    /// テスト専用 facade: active な run 一覧を取得する。
    /// production の read-only 経路は workflow QueryService を使う。
    #[cfg(test)]
    pub async fn list_active_runs(
        &self,
    ) -> Vec<crate::adaptor::gateway::workflow::run::WorkflowRunSummary> {
        self.run_store
            .list_runs(crate::adaptor::gateway::workflow::run::RunListFilter {
                status: Some(crate::adaptor::gateway::workflow::run::RunStatusFilter::Active),
                worktree_path: None,
            })
            .await
    }

    /// テスト専用 facade: terminal な run 一覧を取得する。
    #[cfg(test)]
    pub async fn list_completed_runs(
        &self,
    ) -> Vec<crate::adaptor::gateway::workflow::run::WorkflowRunSummary> {
        self.run_store
            .list_runs(crate::adaptor::gateway::workflow::run::RunListFilter {
                status: Some(crate::adaptor::gateway::workflow::run::RunStatusFilter::Terminal),
                worktree_path: None,
            })
            .await
    }

    /// テスト専用 facade: 単一 run の summary を取得する。
    /// active map → terminal metadata file の順で lookup する。
    #[cfg(test)]
    pub async fn get_run(
        &self,
        run_id: &str,
    ) -> Option<crate::adaptor::gateway::workflow::run::WorkflowRunSummary> {
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
        let recovery_items =
            workflow_orphan_recovery::orphan_run_recovery_items(orphans, current_timestamp());
        for item in recovery_items {
            if let Err(e) = self.write_log_required(app, item.event) {
                log::warn!(
                    "recover_orphan_runs: append RunAborted failed for {}: {e}",
                    item.run_id
                );
                // metadata 更新は次回起動で再試行するため、ここで skip する。
                continue;
            }
            if let Err(e) = self
                .run_store
                .force_complete_orphan_to_aborted(item.run, item.completed_at, None)
                .await
            {
                log::warn!(
                    "recover_orphan_runs: persist metadata failed for {}: {e}",
                    item.run_id
                );
            }
        }
    }
}

impl WorkflowRuntimeService {
    /// ワークフローを開始する。
    /// ChatSessionは既に作成済みの前提で、最初のステップのプロンプトを送信する。
    ///
    /// 戻り値は新しく払い出された `run_id`。
    /// `execution_id` を `run_id` として「昇格」させた値であり、ここ以外で採番されることはない。
    /// state 変化の入口は resolved StartRun port からこの private handler に合流する。
    /// 外部入口としては公開せず、usecase/gateway が解決済み workflow を渡す境界にする。
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
            .try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>()
            .ok_or_else(|| {
                WorkflowEngineError::InvalidWorkflow(
                    "AgentBackendRegistry is not registered".to_string(),
                )
            })?;
        let workflow_definition =
            crate::adaptor::gateway::workflow::domain_mapping::workflow_definition_to_domain(
                &workflow,
            );
        crate::domain::workflow::validation::validate_models(&workflow_definition, |model| {
            registry.resolve_backend_for_model(model)
        })
        .map_err(|e| WorkflowEngineError::InvalidWorkflow(e.to_string()))?;

        // ===== Phase 2: 副作用（Run Store reservation 先取り → 親 session 作成 → executions 登録） =====
        // Spec issues-1011 finding 5/8: 並行起動でも parent ChatSession を孤立させないために
        // Run Store reservation を「最初の副作用」にする。reservation が失敗（同一 worktree
        // への並行起動）した場合は AlreadyActive として返り、他の副作用は走らない。
        let data_dir = crate::app_data_dir::resolve_data_dir(app)
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
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot.clone()).await;

        // NDJSONログ: step_started 以降は補助ログとして best effort で書き込む。
        // 最初のステップが並列ブロックかどうかで分岐
        let first_step_is_parallel = workflow.nodes[0].is_parallel();

        // [04] post-commit: RunStarted append 済みのため start primitive は既に受理。
        //    初回 session / parallel children 起動失敗は Failed 状態遷移として観測し、
        //    start primitive は Ok(run_id) を返す（spec [04]『command 受理境界』Rule）。
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
                        workflow_runtime_session::runtime_start_failed_state(
                            RuntimeStartFailureKind::ParallelChildren,
                            &e,
                        ),
                    )
                    .await;
                log::warn!("workflow {run_id}: post-commit start_parallel_children failed: {e}");
            }
        } else {
            // 逐次ステップ → StepStartedログ + start_step_session
            self.write_log(
                app,
                workflow_runtime_events::node_started_event_for_snapshot(&snapshot),
            );

            if let Err(e) = self
                .start_step_session(app, handles, session_store, &worktree_path)
                .await
            {
                workflow_runtime_session::record_step_session_start_failed_by_run_id(
                    &self.executions,
                    &run_id,
                    &e,
                )
                .await;
                let _ = self
                    .set_execution_state(
                        app,
                        session_store,
                        handles,
                        &worktree_path,
                        workflow_runtime_session::runtime_start_failed_state(
                            RuntimeStartFailureKind::StepSession,
                            &e,
                        ),
                    )
                    .await;
                log::warn!("workflow {run_id}: post-commit start_step_session failed: {e}");
            }
        }
        Ok(run_id)
    }

    pub(crate) async fn resolve_start_run_worktree(
        &self,
        worktree_path: String,
    ) -> Result<String, WorkflowEngineError> {
        self.worktree_resolver
            .resolve(worktree_path)
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn resolve_start_run_workflow(
        &self,
        workflow_file_stem: &str,
    ) -> Result<Workflow, WorkflowEngineError> {
        crate::domain::workflow::validation::validate_name(workflow_file_stem)
            .map_err(|e| WorkflowEngineError::ValidationError(format!("validation_error: {e}")))?;
        self.workflow_resolver
            .resolve(workflow_file_stem)
            .await
            .map_err(Into::into)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn start_resolved_workflow<R: tauri::Runtime>(
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
        self.start_workflow(
            app,
            session_store,
            handles,
            workflow,
            worktree_path,
            file_stem,
            task,
            trigger_source,
            permission_mode,
        )
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
        let data_dir = crate::app_data_dir::resolve_data_dir(app)
            .map_err(WorkflowEngineError::SessionStore)?;
        request_event_already_recorded(
            &data_dir,
            RequestEventKind::OutputSubmitted,
            run_id,
            request_id,
        )
        .map_err(request_event_lookup_error_to_engine_error)
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
    pub(crate) async fn submit_workflow_output<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        step_name: String,
        contract: String,
        structured_output: serde_json::Value,
        request_id: Option<String>,
        submitted_at: Option<f64>,
    ) -> Result<(), WorkflowEngineError> {
        self.handle_submit_output(
            app,
            run_id,
            step_name,
            contract,
            structured_output,
            request_id,
            submitted_at,
        )
        .await
    }

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
        workflow_output_submission::validate_submit_output_request(run_id, &step_name, &contract)?;

        // 1. contract 適合判定（pure validator、副作用なし）。ロック取得前に行い、
        //    無効入力は writer lock を取らずに弾く。
        //    [08] 機密値 redaction: caller (CLI / Tauri API) 入力に approve コメントや
        //    secret token が混入していても event log / step_outputs に生で残らないよう、
        //    redaction 後の structured output を contract validation に通す。
        //    preflight (workflow_validate_output / CLI cmd_output_validate) と本 submit で
        //    同一の前処理 + validation を共有するため、`preprocess_and_validate_output`
        //    に集約する（spec [08] L169 / Rule 2）。
        let secrets = secret_source::collect_configured_secret_values(app);
        let workflow_output_submission::ValidatedSubmissionOutput {
            structured_output: validated_output,
            result: validated_result,
            workflow_variables: contract_vars,
        } = workflow_output_submission::validate_submission_output_with_secrets(
            &contract,
            structured_output,
            &secrets,
        )?;

        // 2. writer lock 取得後に state / contract / accepting target / run_index を
        //    再検証し、snapshot 採取と mutation を同一 lock スコープで行う
        //    （spec [08] 境界: OutputSubmitted の append は適合判定および state 更新と
        //    同一トランザクション境界内。並行 dispatch によって stale step の output が
        //    確定されないよう、validation と mutation のあいだに lock を手放さない）。
        let timestamp = current_timestamp();
        let mutation = {
            let mut execs = self.executions.lock().await;
            let exec = execs
                .get_mut(run_id)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
            workflow_output_submission::apply_validated_submission(
                exec,
                run_id,
                &step_name,
                &contract,
                &validated_output,
                validated_result,
                contract_vars,
                timestamp,
            )?
        };

        // 3. OutputSubmitted event を append。append 失敗時は state を snapshot から
        //    一括復元することで「validation・state 更新・event append」を原子的に揃える
        //    （spec [08] 振る舞い定義 Rule 1: 適合しない場合 / 適合する場合いずれも
        //    state と event log が一致する）。
        let event = workflow_output_submission::output_submitted_event(
            run_id,
            &mutation.workflow_name,
            &step_name,
            contract,
            validated_output,
            request_id,
            submitted_at,
            timestamp,
        );
        if let Err(append_err) = self.write_log_required(app, event) {
            let mut execs = self.executions.lock().await;
            if let Some(exec) = execs.get_mut(run_id) {
                workflow_output_submission::rollback_validated_submission(
                    exec, &step_name, mutation,
                );
            }
            return Err(WorkflowEngineError::SessionStore(append_err));
        }

        Ok(())
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
        let event = workflow_runtime_events::cli_mutation_requested_event(
            &workflow_name,
            context,
            current_timestamp(),
        )
        .expect("CliPending context must produce a CliMutationRequested event");
        self.write_log_required(app, event)
            .map_err(WorkflowEngineError::SessionStore)?;
        Ok(())
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
        let event = workflow_runtime_events::cli_mutation_rejected_event(
            workflow_name,
            context,
            error,
            current_timestamp(),
        )?;
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
        let workflow_name = self.workflow_name_for_external_run(run_id).await?;
        let event = workflow_runtime_events::submit_output_cli_mutation_rejected_event(
            workflow_name,
            run_id,
            context,
            error,
            current_timestamp(),
        )?;
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
        let data_dir = crate::app_data_dir::resolve_data_dir(app)
            .map_err(WorkflowEngineError::SessionStore)?;
        request_event_already_recorded(
            &data_dir,
            RequestEventKind::CliMutationRequested,
            run_id,
            request_id,
        )
        .map_err(request_event_lookup_error_to_engine_error)
    }

    /// 外部入口（CLI pending dispatcher / 将来追加される他経路）が dispatch
    /// する前に、in-memory execution を `workflow_runs/` から再構成する。
    ///
    /// CLI pending dispatcher / 外部 mutation primitive 入口の前段で呼ぶことで、
    /// 稼働アプリ再起動後でも `run_id` 主語の mutation が認可・冪等性判定の対象
    /// となる（spec [06] 経路非依存境界）。本関数は CLI 経路に限定されないため
    /// `_for_external` で命名統一する（review R2-02）。
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
        workflow_external_restore::validate_run_record_for_external_restore(run_id, &run)?;

        let data_dir = crate::app_data_dir::resolve_data_dir(app)
            .map_err(WorkflowEngineError::SessionStore)?;
        let events = WorkflowEventLog::new(&data_dir)
            .read_log(run_id)
            .map_err(WorkflowEngineError::SessionStore)?;
        let state = reconstruct_state_from_events(run_id, &events)
            .map_err(WorkflowEngineError::SessionStore)?
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;

        if self.run_store.active_run_snapshot(run_id).await.is_none() {
            self.run_store
                .register_active(run.clone())
                .await
                .map_err(|e| {
                    WorkflowEngineError::SessionStore(format!("RunStore restore failed: {e}"))
                })?;
        }

        let restored =
            workflow_external_restore::restore_execution_from_projected_state(run_id, run, state)?;
        let current_session_id = restored.current_session_id;
        let exec = restored.execution;

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
        final_parts: &[crate::usecase::agent_session::session::MessagePart],
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
            let plan = exec.plan_turn_complete_mutation(exit_code)?;

            match plan {
                workflow_transition::TurnCompleteMutationPlan::NotRunning => return Ok(()),
                workflow_transition::TurnCompleteMutationPlan::SessionError {
                    history_result,
                    failure_reason,
                    ..
                } => {
                    if exec.is_terminal() {
                        return Ok(());
                    }
                    let snapshot_before = exec.clone();
                    let entry = exec.make_step_history_entry(Some(history_result), None, None);
                    exec.step_history.push(entry);
                    exec.state = WorkflowExecutionState::Failed {
                        reason: failure_reason,
                    };
                    exec.updated_at = current_timestamp();
                    Ok(TurnCommit {
                        outcome: StepOutcome::Persist(exec.to_workflow_state()),
                        required_events: Vec::new(),
                        rollback_snapshot: (exec.id.clone(), snapshot_before),
                    })
                }
                workflow_transition::TurnCompleteMutationPlan::RequestApproval { node_name } => {
                    if exec.is_terminal() {
                        return Ok(());
                    }
                    let snapshot_before = exec.clone();
                    let workflow_name = exec.workflow.name.clone();
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
                workflow_transition::TurnCompleteMutationPlan::UnexpectedNodeType {
                    failure_reason,
                    ..
                } => {
                    if exec.is_terminal() {
                        return Ok(());
                    }
                    let snapshot_before = exec.clone();
                    let entry =
                        exec.make_step_history_entry(Some(failure_reason.clone()), None, None);
                    exec.step_history.push(entry);
                    exec.state = WorkflowExecutionState::Failed {
                        reason: failure_reason,
                    };
                    exec.updated_at = current_timestamp();
                    Ok(TurnCommit {
                        outcome: StepOutcome::Persist(exec.to_workflow_state()),
                        required_events: Vec::new(),
                        rollback_snapshot: (exec.id.clone(), snapshot_before),
                    })
                }
                workflow_transition::TurnCompleteMutationPlan::AutoEvaluate {
                    rules,
                    node_name,
                } => {
                    let rules: Vec<TransitionRule> =
                        rules.into_iter().map(transition_rule_from_domain).collect();
                    Err((rules, node_name))
                }
            }
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
        let completed_step_session_ids = outcome.completed_step_session_ids();
        let snapshot_for_commit = outcome.snapshot().clone();
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

        workflow_runtime_session::release_completed_step_sessions(
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

    fn apply_approval_application(
        exec: &mut WorkflowExecution,
        decision: &ApprovalDecision,
        application: workflow_transition::ApprovalApplication,
    ) -> Result<StepOutcome, WorkflowEngineError> {
        let plan = exec.plan_approval_application(decision, application)?;
        let outcome = match plan.transition {
            workflow_transition::ApprovalApplicationTransition::Advance => {
                let completion = plan.completion;
                let entry = exec.make_step_history_entry(
                    Some(completion.result),
                    completion.structured_output,
                    completion.output_contract,
                );
                exec.step_history.push(entry);
                exec.apply_advance()
            }
            workflow_transition::ApprovalApplicationTransition::TransitionTo(target) => {
                let completion = plan.completion;
                let entry = exec.make_step_history_entry(
                    Some(completion.result),
                    completion.structured_output,
                    completion.output_contract,
                );
                exec.step_history.push(entry);
                exec.apply_transition(&target)?
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
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn resolve_workflow_approval<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        run_id: &str,
        decision: ApprovalDecision,
        approve_comment: Option<String>,
        expected_step_name: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        self.resolve_workflow_approval_with_commit_context(
            app,
            session_store,
            handles,
            run_id,
            decision,
            approve_comment,
            expected_step_name,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn resolve_workflow_approval_with_commit_context<R: tauri::Runtime>(
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
        self.handle_approval(
            app,
            session_store,
            handles,
            run_id,
            decision,
            approve_comment,
            expected_step_name,
            commit_context,
        )
        .await
    }

    /// [04] 内部 typed boundary: approval mutation の handler 実体。production gateway と
    /// pending dispatcher は `resolve_workflow_approval*` からここに合流する。
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
            workflow_approval_runtime::resolve_approval_target_snapshot(
                exec,
                Some(run_id),
                expected_step_name,
            )?;
            let node = &exec.workflow.nodes[exec.current_step_index];
            let output_contract = node.output_contract.clone();
            let run_index = exec
                .step_execution_counts
                .get(&node.name)
                .copied()
                .unwrap_or(1);
            let submitted_output = output_contract.as_deref().and_then(|contract| {
                workflow_output_submission::submitted_step_output_for(
                    &exec.step_outputs,
                    &node.name,
                    run_index,
                    contract,
                )
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
        workflow_approval_runtime::validate_approval_input(&decision, approve_comment.as_deref())?;

        if matches!(decision, ApprovalDecision::Approve) {
            let turn_phase = if let Some(ref sid) = current_session_id {
                let map = handles.lock().await;
                map.get(sid).map(|p| p.turn_phase)
            } else {
                None
            };
            workflow_approval_runtime::validate_approval_turn_phase(turn_phase)?;
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
                    let secrets = secret_source::collect_configured_secret_values(app);
                    (
                        Some(workflow_approval_runtime::reject_structured_output(
                            comment, &secrets,
                        )),
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
        let contract_variables = workflow_contract::extract_workflow_variables_from_contract_output(
            application_output_contract.as_deref(),
            structured_output.as_ref(),
        );

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
            workflow_approval_runtime::resolve_approval_target_snapshot(
                exec,
                Some(run_id),
                expected_step_name,
            )?;
            let workflow_name = exec.workflow.name.clone();
            let node_name = exec.workflow.nodes[exec.current_step_index].name.clone();
            let snapshot_before = exec.clone();
            exec.workflow_variables.extend(contract_variables);
            let outcome = Self::apply_approval_application(
                exec,
                &decision,
                workflow_transition::ApprovalApplication {
                    effective_result,
                    structured_output,
                    output_contract: application_output_contract,
                },
            )?;
            (outcome, snapshot_before, workflow_name, node_name)
        };

        let snapshot_for_commit = outcome.snapshot().clone();
        let completed_step_session_ids = outcome.completed_step_session_ids();
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
            let secrets = secret_source::collect_configured_secret_values(app);
            Some(workflow_secret_masker::mask_sensitive_text(&raw, &secrets))
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
        let mut commit_events = match workflow_runtime_events::required_events_for_approval_commit(
            approval_event,
            &mut outcome,
        ) {
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
            if let Some(event) = workflow_runtime_events::cli_mutation_requested_event(
                &workflow_name_for_event,
                context,
                current_timestamp(),
            ) {
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
        // broadcast / terminal log / cleanup / 次 step 起動 / auto-approve primitive は
        // ここで実行する。
        workflow_runtime_session::release_completed_step_sessions(
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

    pub(crate) async fn abort_workflow_run<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        run_id: &str,
        expected_node_name: Option<&str>,
    ) -> Result<(), WorkflowEngineError> {
        self.abort_workflow_run_with_commit_context(
            app,
            session_store,
            handles,
            run_id,
            expected_node_name,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn abort_workflow_run_with_commit_context<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        session_store: &Arc<SessionStore>,
        handles: &Arc<Mutex<AgentProcessMap>>,
        run_id: &str,
        expected_node_name: Option<&str>,
        commit_context: Option<CommandCommitContext>,
    ) -> Result<(), WorkflowEngineError> {
        // run 全体の Abort: NotFound / AlreadyTerminal は非受理として typed error
        // に射影する（Spec [04] Rule「対象不在 / 既に終了した command は受理されない」）。
        match self
            .abort_workflow_by_run_id(
                app,
                session_store,
                handles,
                run_id,
                expected_node_name,
                commit_context,
            )
            .await?
        {
            AbortOutcome::Aborted => Ok(()),
            AbortOutcome::NotFound => {
                Err(WorkflowEngineError::ExecutionNotFound(run_id.to_string()))
            }
            AbortOutcome::AlreadyTerminal => Err(WorkflowEngineError::InvalidState(format!(
                "run {run_id} is already terminal"
            ))),
        }
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
    /// 外部から直接呼ばれることはなく、`abort_workflow_run*` runtime primitive 経路のみが
    /// 利用する（Spec [04]: 内部呼び出し元も engine の private method を直接叩かない）。
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
        #[cfg(test)]
        self.wait_abort_after_lookup_for_test().await;

        // 2. [04] pre-commit (rollback 可能): mutation 直前 snapshot を取得し、
        //    state を Aborted に遷移させる。競合で terminal 化していた場合は
        //    AlreadyTerminal で返す。
        let timestamp = current_timestamp();
        let run_store_snapshot_before = self.run_store.active_run_snapshot(run_id).await;
        let (snapshot_before, snapshot_state, workflow_name_for_event, aborted_step_for_event) = {
            let mut execs = self.executions.lock().await;
            let Some(exec) = execs.get_mut(run_id) else {
                drop(execs);
                return Ok(if self.has_terminal_run_record(run_id).await {
                    AbortOutcome::AlreadyTerminal
                } else {
                    AbortOutcome::NotFound
                });
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
            let mut aborted_step_for_event = None;

            // spec issues-1023: state を Aborted にする前に、中断時の current step /
            // parallel children を `step_history` に "aborted" entry として記録する。
            // これにより UI 側は既存 history 描画経路 + session_id を使って中断 step の
            // session log にアクセスできるようになる。`exec.parallel_run = None` を
            // 明示クリアして `to_workflow_state()` 経由の二重表示を防ぐ。
            if exec.parallel_run.is_some() {
                if let Some(entry) = exec.make_aborted_parallel_history_entry(timestamp) {
                    aborted_step_for_event = Some(
                        workflow_runtime_events::run_aborted_step_snapshot_from_history_entry(
                            &entry,
                        ),
                    );
                    exec.step_history.push(entry);
                }
                exec.parallel_run = None;
            } else {
                let current_step_name = exec.workflow.nodes[exec.current_step_index].name.clone();
                let current_run_index = exec
                    .step_execution_counts
                    .get(&current_step_name)
                    .copied()
                    .unwrap_or(1);
                let already_in_history = exec.step_history.last().is_some_and(|e| {
                    e.step_name == current_step_name && e.run_index == current_run_index
                });
                if !already_in_history {
                    let entry = exec.make_aborted_history_entry(timestamp);
                    aborted_step_for_event = Some(
                        workflow_runtime_events::run_aborted_step_snapshot_from_history_entry(
                            &entry,
                        ),
                    );
                    exec.step_history.push(entry);
                }
            }

            exec.state = WorkflowExecutionState::Aborted;
            exec.updated_at = timestamp;
            let snapshot_state = exec.to_workflow_state();
            (
                snapshot_before,
                snapshot_state,
                workflow_name,
                aborted_step_for_event,
            )
        };

        // 3. [04] commit point: RunAborted を必須 append。失敗時は
        //    WorkflowExecution / Run Store / ChatSession を snapshot で一括復元する。
        //    interrupt_agent はこの時点ではまだ実行していないため、append 失敗時には
        //    rollback 不能な外部副作用が残らない。
        let aborted_event = WorkflowEvent::RunAborted {
            run_id: run_id.to_string(),
            workflow_name: workflow_name_for_event.clone(),
            aborted_step: aborted_step_for_event,
            timestamp,
        };
        let mut required_events = vec![aborted_event];
        if let Some(context) = commit_context {
            if let Some(event) = workflow_runtime_events::cli_mutation_requested_event(
                &workflow_name_for_event,
                context,
                current_timestamp(),
            ) {
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
            workflow_runtime_session::interrupt_agent(handles, step_sid).await;
        }
        if let Some(ref session_ids) = parallel_session_ids {
            for sid in session_ids {
                workflow_runtime_session::interrupt_agent(handles, sid).await;
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

    /// `abort_workflow_by_run_id` の post-commit 区間。state は呼出し前に Aborted に
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
        let terminal_session_ids = workflow_runtime_commit::terminal_step_session_ids(&snapshot);
        workflow_runtime_session::release_completed_step_sessions(
            app,
            session_store,
            handles,
            &terminal_session_ids,
        )
        .await;
        self.cleanup_session_workflow_refs_by_run_id(run_id).await;
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot).await;
        self.release_terminal_execution(run_id).await;
    }

    async fn abort_target_lookup(&self, run_id: &str) -> AbortTargetLookup {
        {
            let execs = self.executions.lock().await;
            if let Some(exec) = execs.get(run_id) {
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
                return AbortTargetLookup::Active {
                    current_step_session_id,
                    parallel_session_ids,
                };
            }
        }
        if self.has_terminal_run_record(run_id).await {
            AbortTargetLookup::AlreadyTerminal
        } else {
            AbortTargetLookup::NotFound
        }
    }

    async fn has_terminal_run_record(&self, run_id: &str) -> bool {
        self.run_store
            .get_run_record(run_id)
            .await
            .is_some_and(|run| run.status.is_terminal())
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
        final_parts: &[crate::usecase::agent_session::session::MessagePart],
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
                let submitted = workflow_output_submission::submitted_step_output_for(
                    &exec.step_outputs,
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
        let (all_completed, outcome_opt, exec_snapshot_before, progress_events) = {
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

                if let Err(e) = workflow_runtime_commit::sync_run_store_from_snapshot(
                    &self.run_store,
                    run_id,
                    &snapshot,
                )
                .await
                {
                    workflow_runtime_commit::rollback_execution_projection_after_run_store_sync_failure(
                        &self.executions,
                        &self.run_store,
                        run_id,
                        &snapshot,
                    )
                    .await;
                    return Err(e);
                }
                // 他の子ステップをinterrupt
                for sid in &running_ids {
                    workflow_runtime_session::interrupt_agent(handles, sid).await;
                }
                let mut cleanup_ids = running_ids;
                cleanup_ids.push(session_id.to_string());
                cleanup_ids.sort();
                cleanup_ids.dedup();
                for sid in cleanup_ids {
                    workflow_runtime_session::release_completed_step_session(
                        app,
                        session_store,
                        handles,
                        &sid,
                    )
                    .await;
                }
                workflow_runtime_session::broadcast_state(app, worktree_path, snapshot.clone())
                    .await;
                self.cleanup_session_workflow_refs_by_run_id(&snapshot.execution_id)
                    .await;
                self.release_terminal_execution(run_id).await;
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
            let mut progress_events = vec![WorkflowEvent::ParallelChildCompleted {
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
            }];

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
                    progress_events,
                )
            } else {
                let aggregate = pr.aggregate.clone();
                let parent_step_name = pr.parent_step_name.clone();
                let parent_run_index = exec
                    .step_execution_counts
                    .get(&parent_step_name)
                    .copied()
                    .unwrap_or(1);
                let completed_at = current_timestamp();
                let completion_plan = workflow_parallel_runtime::plan_parallel_parent_completion(
                    &parent_step_name,
                    parent_run_index,
                    aggregate.as_ref(),
                    &pr.children,
                    &exec.step_outputs,
                    completed_at,
                );
                let parallel_completed_result = match &completion_plan.transition {
                    workflow_parallel_runtime::ParallelParentCompletionTransition::Advance => {
                        "advance".to_string()
                    }
                    workflow_parallel_runtime::ParallelParentCompletionTransition::TransitionTo {
                        aggregate_result,
                        ..
                    } => aggregate_result.clone(),
                };
                progress_events.push(WorkflowEvent::ParallelCompleted {
                    run_id: exec.id.clone(),
                    workflow_name: exec.workflow.name.clone(),
                    parent_node_name: parent_step_name.clone(),
                    aggregate_result: parallel_completed_result,
                    timestamp: current_timestamp(),
                });

                exec.parallel_run = None;
                exec.updated_at = completed_at;
                exec.step_outputs
                    .insert(parent_step_name.clone(), completion_plan.parent_step_output);
                exec.current_step_token_usage = TokenUsage::default();
                exec.current_session_id = None;
                exec.step_history.push(completion_plan.history_entry);

                let outcome = match completion_plan.transition {
                    workflow_parallel_runtime::ParallelParentCompletionTransition::Advance => {
                        exec.apply_advance()
                    }
                    workflow_parallel_runtime::ParallelParentCompletionTransition::TransitionTo {
                        target_node_name,
                        ..
                    } => exec.apply_transition(&target_node_name)?,
                };
                (true, Some(outcome), exec_snapshot_before, progress_events)
            }
        };

        for event in progress_events {
            self.write_log(app, event);
        }

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
        execs.get(run_id).map(|exec| exec.to_workflow_state())
    }

    async fn release_terminal_execution(&self, run_id: &str) {
        let mut execs = self.executions.lock().await;
        if execs.get(run_id).is_some_and(|exec| exec.is_terminal()) {
            execs.remove(run_id);
        }
    }

    /// 起動環境別の `releash` alias 名を返す（spec issues-1054）。
    ///
    /// 本関数は alias 名のみを必要とするため、data_dir 解決を経由しない pure helper
    /// (`alias_name_for_profile`) を直接呼び、`dirs::data_dir()` 失敗で alias 名解決が
    /// 巻き込まれないようにする。
    #[cfg(test)]
    fn resolve_releash_alias() -> String {
        crate::path_aliases::alias_name_for_profile(crate::path_aliases::BuildProfile::current())
            .to_string()
    }

    fn contract_repair_attempt_count<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        run_id: &str,
        node_name: &str,
    ) -> Result<u32, WorkflowEngineError> {
        let data_dir = crate::app_data_dir::resolve_data_dir(app)
            .map_err(WorkflowEngineError::SessionStore)?;
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

        let data_dir = crate::app_data_dir::resolve_data_dir(app)
            .map_err(WorkflowEngineError::SessionStore)?;
        let Some(session) = session_store
            .get_session_meta(&data_dir, session_id)
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
                workflow_output_submission::resolved_output_contract_definition_for(
                    &exec.workflow,
                    node_name,
                    contract,
                )
            })
        };
        let cli_alias = crate::path_aliases::alias_name_for_profile(
            crate::path_aliases::BuildProfile::current(),
        );
        let prompt = workflow_contract::build_missing_output_repair_prompt(
            cli_alias,
            run_id,
            node_name,
            contract,
            contract_definition.as_deref(),
        );
        let _runtime_guard =
            crate::infrastructure::agent_session::runtime::acquire_session_runtime_lock(session_id)
                .await;
        crate::infrastructure::agent_session::runtime::start_agent_turn_internal_locked(
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
            entry.state = STEP_STATE_FAILED.to_string();
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
        let session_id = workflow_approval_runtime::resolve_chat_session_for_approval(exec)?;
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
        workflow_approval_runtime::validate_approval_chat_instruction(exec, session_id, content)
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
        workflow_approval_runtime::validate_approval_target_snapshot(
            exec,
            expected_execution_id,
            expected_step_name,
        )
    }

    /// 現在実行中のワークフロー名の集合を返す（全worktreeを集約）。
    #[cfg(test)]
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
    // 実体は WorkflowRuntimeService impl の下に置く。

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
            let required_events =
                match workflow_runtime_events::terminal_required_events_for_snapshot(&snapshot) {
                    Ok(events) => events,
                    Err(e) => {
                        let mut execs = self.executions.lock().await;
                        if let Some(exec) = execs.get_mut(&run_id) {
                            *exec = snapshot_before;
                        }
                        return Err(e);
                    }
                };
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
            let terminal_session_ids =
                workflow_runtime_commit::terminal_step_session_ids(&snapshot);
            workflow_runtime_session::release_completed_step_sessions(
                app,
                session_store,
                handles,
                &terminal_session_ids,
            )
            .await;
            self.cleanup_session_workflow_refs_by_run_id(&run_id).await;
            workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot.clone()).await;
            self.release_terminal_execution(&run_id).await;
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

        if let Err(e) = workflow_runtime_commit::sync_run_store_from_snapshot(
            &self.run_store,
            &run_id,
            &snapshot,
        )
        .await
        {
            rollback_engine_state(run_id.clone(), snapshot_before).await;
            return Err(e);
        }

        if is_terminal {
            let terminal_session_ids =
                workflow_runtime_commit::terminal_step_session_ids(&snapshot);
            workflow_runtime_session::release_completed_step_sessions(
                app,
                session_store,
                handles,
                &terminal_session_ids,
            )
            .await;
            self.cleanup_session_workflow_refs_by_run_id(&run_id).await;
        }
        workflow_runtime_session::broadcast_state(app, &worktree_path, snapshot.clone()).await;
        if is_terminal {
            self.release_terminal_execution(&run_id).await;
        }
        Ok(())
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
        let run_store_result = workflow_runtime_commit::restore_run_store_active_snapshot(
            &self.run_store,
            run_store_snapshot_before,
        )
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
        final_parts: &[crate::usecase::agent_session::session::MessagePart],
        rules: &[TransitionRule],
        step_name: &str,
    ) -> Result<(), WorkflowEngineError> {
        // テキストパートを結合（ロック外で完了）
        let text = turn_completion::extract_text_from_parts(final_parts);

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
                workflow_output_submission::submitted_step_output_for(
                    &exec.step_outputs,
                    &node.name,
                    run_index,
                    contract,
                )
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
            Some(turn_completion::evaluate_auto_rules(result_str, rules))
        } else {
            Some(turn_completion::evaluate_auto_rules(&text, rules))
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
                    exec.apply_advance()
                }
                Some(Some((next_step, matched_rule))) => {
                    // ルールマッチ → 指定ステップへ遷移
                    let entry = exec.make_step_history_entry(
                        Some(matched_rule),
                        structured_output,
                        output_contract,
                    );
                    exec.step_history.push(entry);
                    exec.apply_transition(&next_step)?
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
    /// 5. `executions.current_session_id` 更新
    /// 6. `NodeSessionStarted` append とブロードキャスト
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
            run_id_for_ref,
            step_clone,
            step_outputs_clone,
            step_history_clone,
            task_clone,
            workflow_variables_clone,
            workflow_declared_variables_clone,
            workflow_defaults_clone,
            workflow_step_context,
        ) = {
            let execs = self.executions.lock().await;
            let (run_id, exec) = find_by_worktree(&execs, worktree_path)
                .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
            let step = &exec.workflow.nodes[exec.current_step_index];
            let step_run_index = exec
                .step_execution_counts
                .get(&step.name)
                .copied()
                .unwrap_or(1);
            (
                run_id.clone(),
                step.clone(),
                exec.step_outputs.clone(),
                exec.step_history.clone(),
                exec.task.clone(),
                exec.workflow_variables.clone(),
                exec.workflow.variables.clone(),
                exec.workflow_defaults.clone(),
                WorkflowStepContext {
                    run_id: run_id.clone(),
                    workflow_name: exec.workflow.name.clone(),
                    step_name: step.name.clone(),
                    run_index: step_run_index,
                    parent_step_name: None,
                    parent_run_index: None,
                    order: exec.step_history.len() as u32,
                },
            )
        };

        // プロンプト合成（純粋関数）を最初に行う。
        // ここで失敗（参照先ファセットが存在しない等）した場合、後続の
        // ChatSession 生成・`session_workflow_refs` 登録・AgentSession 開始は一切
        // 行われない。これにより、`start_step_session` がエラー経路で孤立した
        // ChatSession や参照マップ entry を残さないことを構造的に保証する。
        let (system_prompt, prompt) = workflow_prompt::build_step_prompt(
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
                workflow_step_context,
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

        let _runtime_guard =
            crate::infrastructure::agent_session::runtime::acquire_session_runtime_lock(
                &step_session_id,
            )
            .await;

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
            deps.append_node_session_started(&snapshot).await?;
            deps.broadcast_state(worktree_path, snapshot).await;
        }

        // プロンプト送信（ステップ用セッションIDを使用）
        deps.start_agent_turn_locked(&step_session_id, worktree_path, &permission_mode, &prompt)
            .await
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
        let (system_prompt, prompt) = workflow_prompt::build_step_prompt(
            step,
            run_id,
            worktree_path,
            task,
            step_outputs,
            step_history,
            workflow_variables,
            &HashMap::new(),
        )?;
        dispatch_session_start(
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
        let vars = workflow_contract::extract_workflow_variables_from_contract_output(
            output_contract.as_deref(),
            structured_output.as_ref(),
        );
        if !vars.is_empty() {
            let mut execs = self.executions.lock().await;
            if let Some(exec) = find_by_worktree_mut(&mut execs, worktree_path) {
                exec.workflow_variables.extend(vars);
            }
        }
    }

    /// [08] prose 抽出経路は engine から完全除去された（spec [08] Rule 4 構造化出力の
    /// 確定経路は明示的提出のみ）。本 helper は ChatSession 表示など event log と無関係な
    /// 経路で「最後の Agent メッセージ本文」を取り出すテスト用 fixture としてのみ残す。
    #[cfg(test)]
    fn extract_last_assistant_text_from_session(
        session: &crate::usecase::agent_session::session::ChatSession,
    ) -> Option<String> {
        let agent_msg = session
            .messages
            .iter()
            .rev()
            .find(|m| m.role == crate::usecase::agent_session::session::MessageRole::Agent)?;

        let text = if let Some(ref parts) = agent_msg.parts {
            turn_completion::extract_text_from_parts(parts)
        } else {
            agent_msg.content.clone()
        };

        if text.is_empty() {
            return None;
        }

        Some(text)
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
        workflow_runtime_commit::sync_run_store_from_snapshot(&self.run_store, &run_id, snapshot)
            .await
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
        if let Err(e) = workflow_runtime_commit::sync_run_store_from_snapshot(
            &self.run_store,
            &run_id,
            snapshot,
        )
        .await
        {
            workflow_runtime_commit::rollback_execution_projection_after_run_store_sync_failure(
                &self.executions,
                &self.run_store,
                &run_id,
                snapshot,
            )
            .await;
            return Err(e);
        }
        workflow_runtime_session::release_completed_step_sessions(
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
        workflow_runtime_session::broadcast_state(app, worktree_path, snapshot.clone()).await;
        if is_terminal {
            self.release_terminal_execution(&run_id).await;
        }
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
        let completed_step_session_ids = outcome.completed_step_session_ids();
        let snapshot_for_commit = outcome.snapshot().clone();
        let run_id = snapshot_for_commit.execution_id.clone();

        // [05] pre-commit phase: 必須 event の生成。`dispatch_internal_node_command` の
        // ValidationError は engine state を snapshot_before で復元して伝播する
        // （spec [05] silent error 禁止）。
        let pre_commit_events =
            match workflow_runtime_events::pre_commit_required_events_for_outcome(&outcome) {
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
            workflow_runtime_session::release_completed_step_sessions(
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

    /// [04] post-commit variant work（共通 side-effect helper）。
    ///
    /// snapshot は既に persist 済みである前提で、outcome variant に応じた残りの副作用
    /// （NodeStarted 書き込み・start_step_session・reduce + 派生 mutation の再帰・
    /// start_parallel_children・auto-approve approval primitive）のみを担当する。`execute_outcome`
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
                if let Some((run_id, step_name)) =
                    workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(
                        &snapshot,
                        workflow_approval_runtime::workflow_approval_auto_approve_enabled(app),
                    )
                {
                    return Box::pin(self.resolve_workflow_approval(
                        app,
                        session_store,
                        handles,
                        &run_id,
                        ApprovalDecision::Approve,
                        None,
                        Some(&step_name),
                    ))
                    .await;
                }
                Ok(())
            }
            StepOutcome::TransitionAndStart(snapshot) => {
                self.emit_post_commit_progress_events(
                    app,
                    commit_mode,
                    workflow_runtime_events::PostCommitProgressEventPlan::TransitionAndStart,
                    &snapshot,
                )?;
                if let Err(e) = self
                    .start_step_session(app, handles, session_store, worktree_path)
                    .await
                {
                    let failed_state =
                        workflow_runtime_session::record_post_commit_runtime_start_failure(
                            &self.executions,
                            worktree_path,
                            RuntimeStartFailureKind::StepSession,
                            &e,
                        )
                        .await;
                    let _ = self
                        .set_execution_state(
                            app,
                            session_store,
                            handles,
                            worktree_path,
                            failed_state,
                        )
                        .await;
                    return Err(e);
                }
                Ok(())
            }
            StepOutcome::ReduceAndTransition(snapshot) => {
                self.emit_post_commit_progress_events(
                    app,
                    commit_mode,
                    workflow_runtime_events::PostCommitProgressEventPlan::ReduceAndTransition,
                    &snapshot,
                )?;

                let reduce_transition =
                    workflow_parallel_runtime::apply_reduce_transition_by_worktree(
                        &self.executions,
                        worktree_path,
                        &snapshot,
                    )
                    .await?;
                self.write_log(app, reduce_transition.output_collected_event);

                // 次 outcome は新たな state mutation なので、再度 sync+persist が必要。
                // execute_outcome 経由でフル経路を回す（spec [04] post-commit 内で発生する
                // 派生 mutation も同じ atomic 境界に乗せる）。
                Box::pin(self.execute_outcome(
                    app,
                    session_store,
                    handles,
                    worktree_path,
                    reduce_transition.next_outcome,
                    reduce_transition.snapshot_before,
                ))
                .await
            }
            StepOutcome::StartParallel(snapshot) => {
                self.emit_post_commit_progress_events(
                    app,
                    commit_mode,
                    workflow_runtime_events::PostCommitProgressEventPlan::StartParallel,
                    &snapshot,
                )?;
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
                    let failed_state =
                        workflow_runtime_session::record_post_commit_runtime_start_failure(
                            &self.executions,
                            worktree_path,
                            RuntimeStartFailureKind::ParallelChildren,
                            &e,
                        )
                        .await;
                    let _ = self
                        .set_execution_state(
                            app,
                            session_store,
                            handles,
                            worktree_path,
                            failed_state,
                        )
                        .await;
                    return Err(e);
                }
                Ok(())
            }
        }
    }

    fn emit_post_commit_progress_events<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        commit_mode: OutcomeCommitMode,
        plan: workflow_runtime_events::PostCommitProgressEventPlan,
        snapshot: &WorkflowState,
    ) -> Result<(), WorkflowEngineError> {
        if !commit_mode.should_emit_progress_events() {
            return Ok(());
        }
        if let Err(e) = self.write_last_step_completed_log(app, snapshot) {
            return Err(plan.node_completed_append_error(e));
        }
        if let Some(event) = plan.followup_event(snapshot) {
            self.write_log(app, event);
        }
        Ok(())
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
        let workflow_runtime_session::ParallelStartRuntimeInputs {
            parallel_start,
            prompt_inputs,
        } = workflow_runtime_session::load_parallel_start_runtime_inputs(
            &self.executions,
            worktree_path,
        )
        .await?;

        // ParallelStarted ログ
        if emit_parallel_started {
            self.write_log(app, parallel_start.started_event(current_timestamp()));
        }

        // Phase 1: セッション生成 + ref登録 + プロンプト構築（AgentSessionはまだ起動しない）
        let child_setups = workflow_runtime_session::prepare_parallel_child_session_setups(
            app,
            session_store,
            &self.session_workflow_refs,
            worktree_path,
            &parallel_start,
            &prompt_inputs,
        )
        .await?;

        let observer = ParallelChildStartedLogObserver {
            engine: self,
            app,
            execution_id: &parallel_start.execution_id,
            workflow_name: &parallel_start.workflow_name,
            parent_step_name: &parallel_start.parent_step_name,
        };
        workflow_runtime_session::activate_parallel_child_sessions(
            app,
            session_store,
            handles,
            &self.executions,
            worktree_path,
            &parallel_start,
            &child_setups,
            &observer,
        )
        .await?;

        Ok(())
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
        let events = workflow_runtime_events::terminal_events_for_append(snapshot)?;
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
        match workflow_runtime_events::last_step_completed_event_for_append(snapshot)? {
            Some(event) => self.write_log_required(app, event),
            None => Ok(()),
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
        workflow_event_log_writer::append_required_events_for_app(app, events)
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
            workflow_approval_runtime::validate_approval_target_snapshot(
                exec,
                expected_execution_id,
                expected_step_name,
            )?;
        }

        workflow_approval_runtime::validate_approval_input(&decision, None)?;
        if matches!(decision, ApprovalDecision::Approve) {
            workflow_approval_runtime::validate_approval_turn_phase(None)?;
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
                ApprovalDecision::Reject { comment } => (
                    Some(workflow_approval_runtime::reject_structured_output(
                        comment,
                        &[],
                    )),
                    None,
                ),
            };

        let application_output_contract = if matches!(decision, ApprovalDecision::Approve) {
            output_contract.clone()
        } else {
            None
        };
        let contract_variables = workflow_contract::extract_workflow_variables_from_contract_output(
            application_output_contract.as_deref(),
            structured_output.as_ref(),
        );
        let effective_result = contract_result.unwrap_or_else(|| result_tag.to_string());

        let mut execs = self.executions.lock().await;
        let exec = execs
            .get_mut(run_id)
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(run_id.to_string()))?;
        workflow_approval_runtime::validate_approval_target_snapshot(
            exec,
            expected_execution_id,
            expected_step_name,
        )?;
        exec.workflow_variables.extend(contract_variables);
        Self::apply_approval_application(
            exec,
            &decision,
            workflow_transition::ApprovalApplication {
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
            workflow_approval_runtime::auto_approve_target_for_persisted_snapshot(snapshot, true)
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
}

#[cfg(test)]
mod tests;
