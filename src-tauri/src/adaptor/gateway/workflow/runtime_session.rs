use std::sync::Arc;
use std::{collections::HashMap, path::Path};

use tokio::sync::Mutex;

use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::execution_registry::{
    find_by_worktree, find_by_worktree_mut,
};
use crate::adaptor::gateway::workflow::facet::WorkflowFacetContents;
use crate::adaptor::gateway::workflow::failure_policy_config::workflow_runtime_timeout_policy;
use crate::adaptor::gateway::workflow::parallel_runtime::{
    self as workflow_parallel_runtime, ParallelChildSessionSetup, ParallelPromptInputs,
    ParallelStartContext,
};
use crate::adaptor::gateway::workflow::prompt_rendering as workflow_prompt;
use crate::adaptor::gateway::workflow::runtime_state::{SessionWorkflowRef, WorkflowExecution};
use crate::adaptor::gateway::workflow::state::{WorkflowExecutionState, WorkflowState};
use crate::adaptor::gateway::workflow::step_settings::{
    resolve_step_settings, ResolvedStepSettings, WorkflowDefaults,
};
use crate::domain::agent_session::PermissionMode;
use crate::domain::workflow::services::history::{
    self as workflow_history, RuntimeStartFailureKind,
};
use crate::domain::workflow::{
    NodeKindName, RetryPolicy, TimeoutContext, WorkflowStepContext, WorkflowStepFailureKind,
};
use crate::usecase::agent_session::backend_registry::AgentBackendRegistry;
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{ChatSession, OpenTabRegistry, SessionStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StepRuntimeKindContext {
    node_kind: NodeKindName,
    approval_gate: bool,
}

impl StepRuntimeKindContext {
    pub(crate) fn new(node_kind: NodeKindName, approval_gate: bool) -> Self {
        Self {
            node_kind,
            approval_gate,
        }
    }

    pub(crate) fn session() -> Self {
        Self::new(NodeKindName::Session, false)
    }
}

fn runtime_start_failure_reason(
    failure: RuntimeStartFailureKind,
    error: &WorkflowEngineError,
) -> String {
    workflow_history::runtime_start_failure_reason(failure, error.to_string())
}

pub(crate) fn runtime_start_failed_state(
    failure: RuntimeStartFailureKind,
    error: &WorkflowEngineError,
) -> WorkflowExecutionState {
    WorkflowExecutionState::Failed {
        reason: runtime_start_failure_reason(failure, error),
        kind: error.workflow_failure_kind(),
        retry_count: error.retry_count(),
    }
}

pub(crate) async fn record_step_session_start_failed_by_run_id(
    executions: &Mutex<HashMap<String, WorkflowExecution>>,
    run_id: &str,
    error: &WorkflowEngineError,
) {
    let mut execs = executions.lock().await;
    if let Some(exec) = execs.get_mut(run_id) {
        exec.record_step_session_start_failed(error.to_string());
    }
}

pub(crate) async fn record_step_session_start_failed_by_worktree(
    executions: &Mutex<HashMap<String, WorkflowExecution>>,
    worktree_path: &str,
    error: &WorkflowEngineError,
) {
    let mut execs = executions.lock().await;
    if let Some(exec) = find_by_worktree_mut(&mut execs, worktree_path) {
        exec.record_step_session_start_failed(error.to_string());
    }
}

pub(crate) async fn record_post_commit_runtime_start_failure(
    executions: &Mutex<HashMap<String, WorkflowExecution>>,
    worktree_path: &str,
    failure: RuntimeStartFailureKind,
    error: &WorkflowEngineError,
) -> WorkflowExecutionState {
    if matches!(failure, RuntimeStartFailureKind::StepSession) {
        record_step_session_start_failed_by_worktree(executions, worktree_path, error).await;
    }
    runtime_start_failed_state(failure, error)
}

pub(crate) struct ParallelStartRuntimeInputs {
    pub(crate) parallel_start: ParallelStartContext,
    pub(crate) prompt_inputs: ParallelPromptInputs,
}

pub(crate) async fn load_parallel_start_runtime_inputs(
    executions: &Mutex<HashMap<String, WorkflowExecution>>,
    worktree_path: &str,
) -> Result<ParallelStartRuntimeInputs, WorkflowEngineError> {
    let execs = executions.lock().await;
    let (_, exec) = find_by_worktree(&execs, worktree_path)
        .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
    Ok(ParallelStartRuntimeInputs {
        parallel_start: workflow_parallel_runtime::prepare_parallel_start_context(exec)?,
        prompt_inputs: workflow_parallel_runtime::parallel_prompt_inputs(exec),
    })
}

/// ステップの model 値から対応するバックエンドIDを解決する。
/// 形式検証（`ModelId`）と登録判定（`resolve_backend_for_model`）を
/// 一括で行い、`set_agent_model_internal` と同一の受け入れ基準を適用する。
pub(crate) fn resolve_backend_for_step_model(
    registry: &AgentBackendRegistry,
    model: &str,
) -> Result<Option<String>, WorkflowEngineError> {
    resolve_step_model_with_registry(registry, model).map(Some)
}

/// 形式検証＋登録判定をレジストリ単体で行う、ワークフロー経路用の解決関数。
/// `resolve_backend_for_step_model` の実体ロジックで、テストではこちらを直接呼ぶ。
pub(crate) fn resolve_step_model_with_registry(
    registry: &AgentBackendRegistry,
    model: &str,
) -> Result<String, WorkflowEngineError> {
    crate::domain::agent_session::ModelId::parse(model).map_err(|e| {
        WorkflowEngineError::InvalidWorkflow(format!("invalid model '{model}': {e}"))
    })?;
    let entry = registry.resolve_model_entry(model).map_err(|e| {
        WorkflowEngineError::InvalidWorkflow(format!("model '{model}' could not be resolved: {e}"))
    })?;
    Ok(entry.backend)
}

#[derive(Debug, Clone)]
struct StepSessionCreationSettings {
    backend_id: Option<String>,
    selected_model: Option<String>,
    permission_mode: PermissionMode,
}

fn resolve_step_session_creation_settings(
    registry: &AgentBackendRegistry,
    step_model: Option<String>,
    step_permission: Option<String>,
    workflow_defaults: &WorkflowDefaults,
) -> Result<StepSessionCreationSettings, WorkflowEngineError> {
    let (step_model, resolved_backend_id) = match step_model {
        Some(model) => {
            let backend_id = resolve_backend_for_step_model(registry, &model)?;
            (Some(model), backend_id)
        }
        None => {
            let backend_id = workflow_defaults
                .backend_id
                .clone()
                .map(Ok)
                .unwrap_or_else(|| {
                    registry.resolve_default_id().map_err(|e| {
                        WorkflowEngineError::InvalidWorkflow(format!(
                            "default backend could not be resolved: {e}"
                        ))
                    })
                })?;
            let selected_model = registry.default_model_for(&backend_id).map_err(|e| {
                WorkflowEngineError::InvalidWorkflow(format!(
                    "default model for backend '{backend_id}' could not be resolved: {e}"
                ))
            })?;
            (Some(selected_model), Some(backend_id))
        }
    };
    let settings = resolve_step_settings(
        step_model,
        step_permission,
        resolved_backend_id,
        workflow_defaults,
    );
    step_session_creation_settings_from_resolved(settings)
}

fn step_session_creation_settings_from_resolved(
    settings: ResolvedStepSettings,
) -> Result<StepSessionCreationSettings, WorkflowEngineError> {
    let permission_mode = PermissionMode::parse(&settings.permission_mode)
        .map_err(|e| WorkflowEngineError::InvalidWorkflow(e.to_string()))?;
    Ok(StepSessionCreationSettings {
        backend_id: settings.backend_id,
        selected_model: settings.selected_model,
        permission_mode,
    })
}

fn create_step_session_from_resolved_settings(
    session_store: &SessionStore,
    data_dir: &Path,
    worktree_path: &str,
    settings: StepSessionCreationSettings,
    workflow_step_context: WorkflowStepContext,
    kind_context: StepRuntimeKindContext,
) -> Result<ChatSession, WorkflowEngineError> {
    let workflow_step_context =
        workflow_step_context_with_runtime_timeouts(&settings, workflow_step_context, kind_context);
    crate::usecase::agent_session::session::create_session_internal_with_attributes(
        session_store,
        data_dir,
        worktree_path,
        settings.backend_id,
        settings.permission_mode,
        crate::usecase::agent_session::session::SessionCreationAttributes {
            selected_model: settings.selected_model,
            workflow_step_session: true,
            workflow_step_context: Some(workflow_step_context),
            ..Default::default()
        },
    )
    .map_err(|e| WorkflowEngineError::SessionStore(format!("create step session: {e}")))
}

fn workflow_step_context_with_runtime_timeouts(
    settings: &StepSessionCreationSettings,
    mut workflow_step_context: WorkflowStepContext,
    kind_context: StepRuntimeKindContext,
) -> WorkflowStepContext {
    let timeout_context = TimeoutContext::new(
        settings.selected_model.clone(),
        kind_context.node_kind,
        Some(workflow_step_context.workflow_name.clone()),
    )
    .with_approval_gate(kind_context.approval_gate);
    let policy = workflow_runtime_timeout_policy();
    workflow_step_context.startup_timeout_secs =
        Some(policy.startup_timeout(&timeout_context).as_secs());
    workflow_step_context.startup_max_retries =
        Some(RetryPolicy::default().max_retries(WorkflowStepFailureKind::StartupTimeout));
    workflow_step_context.stale_timeout_secs =
        Some(policy.stale_timeout(&timeout_context).as_secs());
    workflow_step_context
}

/// ステップ設定の解決 → セッション生成 → 解決済み設定の反映 → 保存を一括で行う。
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_step_session_with_settings(
    registry: &AgentBackendRegistry,
    session_store: &SessionStore,
    data_dir: &Path,
    worktree_path: &str,
    step_model: Option<String>,
    step_permission: Option<String>,
    workflow_defaults: &WorkflowDefaults,
    workflow_step_context: WorkflowStepContext,
    kind_context: StepRuntimeKindContext,
) -> Result<ChatSession, WorkflowEngineError> {
    let settings = resolve_step_session_creation_settings(
        registry,
        step_model,
        step_permission,
        workflow_defaults,
    )?;
    create_step_session_from_resolved_settings(
        session_store,
        data_dir,
        worktree_path,
        settings,
        workflow_step_context,
        kind_context,
    )
}

/// ワークフロー状態をブロードキャストする。
/// スナップショットは呼び出し元がロック内で確定したものを受け取る。
pub(crate) async fn broadcast_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    worktree_path: &str,
    workflow_state: WorkflowState,
) {
    crate::adaptor::gateway::workflow::emit_workflow_state_snapshot(
        app,
        worktree_path,
        crate::adaptor::gateway::workflow::state::workflow_state_to_domain_snapshot(workflow_state),
    )
    .await;
}

pub(crate) async fn emit_workflow_runtime_projection<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    runtime: &Arc<AgentSessionRuntimeUsecase>,
    open_tabs: &OpenTabRegistry,
    worktree_path: &str,
    workflow_state: WorkflowState,
) {
    crate::adaptor::gateway::workflow::emit_workflow_state_from_snapshot(
        app,
        worktree_path,
        crate::adaptor::gateway::workflow::state::workflow_state_to_domain_snapshot(workflow_state),
        runtime.as_ref(),
        open_tabs,
    )
    .await;
}

/// AgentSessionを中断する。
pub(crate) async fn interrupt_agent(runtime: &Arc<AgentSessionRuntimeUsecase>, session_id: &str) {
    if let Err(e) = runtime.interrupt(session_id).await {
        log::warn!("Failed to interrupt agent session '{session_id}': {e}");
    }
}

pub(crate) async fn release_completed_step_session<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    runtime: &Arc<AgentSessionRuntimeUsecase>,
    session_id: &str,
) {
    crate::adaptor::gateway::workflow::release_step_runtime_on_done(
        app,
        session_store,
        runtime,
        session_id,
    )
    .await;
}

pub(crate) async fn release_completed_step_sessions<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    runtime: &Arc<AgentSessionRuntimeUsecase>,
    session_ids: &[String],
) {
    for session_id in session_ids {
        release_completed_step_session(app, session_store, runtime, session_id).await;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare_parallel_child_session_setups<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    registry: &AgentBackendRegistry,
    session_store: &Arc<SessionStore>,
    session_workflow_refs: &Mutex<HashMap<String, SessionWorkflowRef>>,
    worktree_path: &str,
    parallel_start: &ParallelStartContext,
    prompt_inputs: &ParallelPromptInputs,
    facet_contents: &WorkflowFacetContents,
) -> Result<Vec<ParallelChildSessionSetup>, WorkflowEngineError> {
    let prompt_plans =
        prepare_parallel_child_prompt_plans(parallel_start, prompt_inputs, facet_contents)?;
    let creation_plans =
        prepare_parallel_child_creation_plans(registry, parallel_start, prompt_plans)?;
    let data_dir = crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
        .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;
    let mut child_setups = Vec::new();
    let mut created_session_ids = Vec::new();

    for creation_plan in creation_plans {
        let ps = &parallel_start.parallel_steps[creation_plan.step_index];
        let step_session = match create_step_session_from_resolved_settings(
            session_store,
            &data_dir,
            worktree_path,
            creation_plan.settings,
            creation_plan.workflow_step_context,
            creation_plan.kind_context,
        ) {
            Ok(session) => session,
            Err(err) => {
                return Err(rollback_created_parallel_child_sessions(
                    session_store,
                    &data_dir,
                    session_workflow_refs,
                    &created_session_ids,
                    err,
                )
                .await);
            }
        };
        let child_permission_mode = step_session.permission_mode.clone();
        let step_session_id = step_session.id.clone();

        {
            let mut map = session_workflow_refs.lock().await;
            map.insert(
                step_session_id.clone(),
                SessionWorkflowRef {
                    run_id: parallel_start.execution_id.clone(),
                },
            );
        }
        created_session_ids.push(step_session_id.clone());

        child_setups.push(ParallelChildSessionSetup {
            step_name: ps.name.clone(),
            session_id: step_session_id,
            system_prompt: creation_plan.system_prompt,
            workflow_instruction: creation_plan.workflow_instruction,
            user_message: creation_plan.user_message,
            permission_mode: child_permission_mode,
            artifact_contract: ps.artifact.clone(),
            run_index: creation_plan.run_index,
        });
    }

    Ok(child_setups)
}

struct ParallelChildPromptPlan {
    step_index: usize,
    run_index: u32,
    system_prompt: Option<String>,
    user_message: String,
    workflow_instruction: Option<String>,
}

struct ParallelChildCreationPlan {
    step_index: usize,
    run_index: u32,
    system_prompt: Option<String>,
    user_message: String,
    workflow_instruction: Option<String>,
    settings: StepSessionCreationSettings,
    workflow_step_context: WorkflowStepContext,
    kind_context: StepRuntimeKindContext,
}

fn prepare_parallel_child_prompt_plans(
    parallel_start: &ParallelStartContext,
    prompt_inputs: &ParallelPromptInputs,
    facet_contents: &WorkflowFacetContents,
) -> Result<Vec<ParallelChildPromptPlan>, WorkflowEngineError> {
    parallel_start
        .parallel_steps
        .iter()
        .enumerate()
        .zip(parallel_start.child_run_indices.iter().copied())
        .map(|((step_index, ps), run_index)| {
            let (system_prompt, user_message) = workflow_prompt::build_parallel_step_prompt(
                ps,
                facet_contents.for_child(&parallel_start.parent_step_name, &ps.name),
                &parallel_start.execution_id,
                parallel_start.task.as_deref(),
                &prompt_inputs.step_outputs,
                None,
            )?;
            Ok(ParallelChildPromptPlan {
                step_index,
                run_index,
                system_prompt,
                user_message,
                workflow_instruction: workflow_prompt::render_child_workflow_instruction(
                    ps,
                    facet_contents.for_child(&parallel_start.parent_step_name, &ps.name),
                    parallel_start.task.as_deref(),
                    &prompt_inputs.step_outputs,
                    None,
                ),
            })
        })
        .collect()
}

fn prepare_parallel_child_creation_plans(
    registry: &AgentBackendRegistry,
    parallel_start: &ParallelStartContext,
    prompt_plans: Vec<ParallelChildPromptPlan>,
) -> Result<Vec<ParallelChildCreationPlan>, WorkflowEngineError> {
    let mut creation_plans = Vec::with_capacity(prompt_plans.len());
    for prompt_plan in prompt_plans {
        let ps = &parallel_start.parallel_steps[prompt_plan.step_index];
        let settings = resolve_step_session_creation_settings(
            registry,
            ps.model.clone(),
            ps.permission.clone(),
            &parallel_start.workflow_defaults,
        )?;
        creation_plans.push(ParallelChildCreationPlan {
            step_index: prompt_plan.step_index,
            run_index: prompt_plan.run_index,
            system_prompt: prompt_plan.system_prompt,
            user_message: prompt_plan.user_message,
            settings,
            workflow_step_context: WorkflowStepContext {
                run_id: parallel_start.execution_id.clone(),
                workflow_name: parallel_start.workflow_name.clone(),
                step_name: ps.name.clone(),
                run_index: prompt_plan.run_index,
                parent_step_name: Some(parallel_start.parent_step_name.clone()),
                parent_run_index: Some(parallel_start.parent_run_index),
                order: parallel_start.order,
                startup_timeout_secs: None,
                startup_max_retries: None,
                stale_timeout_secs: None,
            },
            kind_context: StepRuntimeKindContext::session(),
            workflow_instruction: prompt_plan.workflow_instruction,
        });
    }
    Ok(creation_plans)
}

async fn rollback_created_parallel_child_sessions(
    session_store: &SessionStore,
    data_dir: &Path,
    session_workflow_refs: &Mutex<HashMap<String, SessionWorkflowRef>>,
    created_session_ids: &[String],
    original_error: WorkflowEngineError,
) -> WorkflowEngineError {
    if created_session_ids.is_empty() {
        return original_error;
    }

    {
        let mut refs = session_workflow_refs.lock().await;
        for session_id in created_session_ids {
            refs.remove(session_id);
        }
    }

    let mut rollback_errors = Vec::new();
    for session_id in created_session_ids {
        if let Err(err) = session_store.remove_session_for_rollback(data_dir, session_id) {
            rollback_errors.push(format!("{session_id}: {err}"));
        }
    }

    if rollback_errors.is_empty() {
        original_error
    } else {
        WorkflowEngineError::SessionStore(format!(
            "parallel child setup failed: {original_error}; rollback failed for created child sessions: {}",
            rollback_errors.join("; ")
        ))
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn activate_parallel_child_sessions<R: tauri::Runtime, O>(
    app: &tauri::AppHandle<R>,
    _branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    session_store: &Arc<SessionStore>,
    runtime: &Arc<AgentSessionRuntimeUsecase>,
    open_tabs: &Arc<OpenTabRegistry>,
    executions: &Mutex<HashMap<String, WorkflowExecution>>,
    worktree_path: &str,
    parallel_start: &ParallelStartContext,
    child_setups: &[ParallelChildSessionSetup],
    observer: &O,
) -> Result<(), WorkflowEngineError>
where
    O: ParallelChildTurnObserver + ?Sized,
{
    let (child_run_indices, snapshot) = {
        let mut execs = executions.lock().await;
        let exec = find_by_worktree_mut(&mut execs, worktree_path)
            .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;

        workflow_parallel_runtime::apply_parallel_run_state(
            exec,
            parallel_start.parent_step_name.clone(),
            parallel_start.aggregate.clone(),
            child_setups,
        )
    };
    broadcast_state(app, worktree_path, snapshot.clone()).await;
    start_parallel_child_sessions(
        app,
        None,
        session_store,
        runtime,
        open_tabs,
        worktree_path,
        child_setups,
        &child_run_indices,
        Some(snapshot),
        observer,
    )
    .await
}

pub(crate) struct ParallelChildStartedRuntime<'a> {
    pub(crate) step_name: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) execution_count: u32,
}

pub(crate) trait ParallelChildTurnObserver {
    fn child_turn_started(&self, started: ParallelChildStartedRuntime<'_>);
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn start_parallel_child_sessions<R: tauri::Runtime, O>(
    app: &tauri::AppHandle<R>,
    _branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    _session_store: &Arc<SessionStore>,
    runtime: &Arc<AgentSessionRuntimeUsecase>,
    open_tabs: &Arc<OpenTabRegistry>,
    worktree_path: &str,
    child_setups: &[ParallelChildSessionSetup],
    child_run_indices: &[u32],
    workflow_state_for_projection: Option<WorkflowState>,
    observer: &O,
) -> Result<(), WorkflowEngineError>
where
    O: ParallelChildTurnObserver + ?Sized,
{
    let mut created_session_ids: Vec<String> = Vec::new();
    let mut runtime_guards = Vec::new();

    for setup in child_setups {
        let runtime_guard = runtime.acquire_session_lock(&setup.session_id).await;
        runtime_guards.push(runtime_guard);
        open_tabs.add(&setup.session_id);
        if let Some(state) = workflow_state_for_projection.clone() {
            emit_workflow_runtime_projection(app, runtime, open_tabs, worktree_path, state).await;
        }
        created_session_ids.push(setup.session_id.clone());
    }

    for (index, setup) in child_setups.iter().enumerate() {
        let runtime_guard = runtime_guards.remove(0);
        let permission_mode = PermissionMode::parse(&setup.permission_mode)
            .map_err(|e| WorkflowEngineError::InvalidWorkflow(e.to_string()))?;
        if let Err(e) = runtime
            .start_turn_locked(
                &setup.session_id,
                permission_mode,
                setup.user_message.clone(),
                setup.system_prompt.clone(),
                setup.workflow_instruction.clone().into_iter().collect(),
            )
            .await
        {
            for session_id in &created_session_ids {
                interrupt_agent(runtime, session_id).await;
            }
            return Err(WorkflowEngineError::with_agent_runtime_context(
                format!(
                    "Failed to start turn for parallel child '{}'",
                    setup.step_name
                ),
                e,
            ));
        }
        drop(runtime_guard);

        observer.child_turn_started(ParallelChildStartedRuntime {
            step_name: &setup.step_name,
            session_id: &setup.session_id,
            execution_count: child_run_indices[index],
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        FanoutSpec, InterimChild, NodeDefinition, NodeKind, Workflow,
    };
    use crate::adaptor::gateway::workflow::state::{StepOutput, TokenUsage};
    use crate::domain::agent_session::gateway::{
        AgentBackend, AgentBackendError, AgentSessionRuntime, ForkSessionRequest, SessionSpec,
    };
    use crate::domain::agent_session::value_objects::{
        BackendCapabilities, ModelDescriptor, ModelId, SkillEntry,
    };
    use crate::domain::workflow::{NodeKindName, WorkflowStepFailureKind};
    use crate::usecase::agent_session::runtime::usecase::AgentRuntimeError;
    use async_trait::async_trait;

    struct RuntimeSessionMockBackend {
        id: &'static str,
        models: Vec<&'static str>,
    }

    #[async_trait]
    impl AgentBackend for RuntimeSessionMockBackend {
        fn id(&self) -> &str {
            self.id
        }

        fn name(&self) -> &str {
            self.id
        }

        fn available_models(&self) -> Vec<ModelDescriptor> {
            self.models
                .iter()
                .map(|model| ModelDescriptor {
                    id: ModelId::parse(*model).unwrap(),
                    display_name: (*model).to_string(),
                })
                .collect()
        }

        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities { steering: false }
        }

        async fn open_session(
            &self,
            _spec: SessionSpec,
        ) -> Result<Box<dyn AgentSessionRuntime>, AgentBackendError> {
            Err(AgentBackendError::Unavailable("test".to_string()))
        }

        async fn archive_session(
            &self,
            _backend_session_id: &str,
            _cwd: &str,
        ) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn unarchive_session(
            &self,
            _backend_session_id: &str,
            _cwd: &str,
        ) -> Result<(), AgentBackendError> {
            Ok(())
        }

        async fn fork_session(
            &self,
            _req: ForkSessionRequest,
        ) -> Result<Option<String>, AgentBackendError> {
            Ok(None)
        }

        async fn skill_catalog(
            &self,
            _cwd: &std::path::Path,
            _query: Option<&str>,
            _limit: Option<usize>,
        ) -> Result<Vec<SkillEntry>, AgentBackendError> {
            Ok(Vec::new())
        }

        async fn fuzzy_file_search(
            &self,
            _root: &std::path::Path,
            _query: &str,
            _limit: usize,
        ) -> Result<Option<Vec<String>>, AgentBackendError> {
            Ok(None)
        }
    }

    fn registry_with_default_backend() -> AgentBackendRegistry {
        let mut registry = AgentBackendRegistry::new();
        registry.register(Arc::new(RuntimeSessionMockBackend {
            id: "codex",
            models: vec!["gpt-5"],
        }));
        registry.set_default(Some("codex".to_string()));
        registry
    }

    fn workflow_context_for_test() -> WorkflowStepContext {
        WorkflowStepContext {
            run_id: "run-1".to_string(),
            workflow_name: "workflow".to_string(),
            step_name: "step".to_string(),
            run_index: 0,
            parent_step_name: None,
            parent_run_index: None,
            order: 1,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        }
    }

    fn workflow_execution_fixture(run_id: &str, worktree_path: &str) -> WorkflowExecution {
        let step_name = "plan".to_string();
        WorkflowExecution {
            id: run_id.to_string(),
            workflow: Workflow {
                name: "test-workflow".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![NodeDefinition {
                    name: step_name.clone(),
                    ..Default::default()
                }],
            },
            state: WorkflowExecutionState::Running,
            current_step_index: 0,
            step_execution_counts: HashMap::from([(step_name, 1)]),
            step_history: Vec::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
            worktree_path: worktree_path.to_string(),
            started_at: 1.0,
            updated_at: 1.0,
            current_session_id: Some("session-1".to_string()),
            current_step_token_usage: TokenUsage::default(),
            step_outputs: HashMap::new(),
            task: None,
            parallel_run: None,
            current_stall_observations: Vec::new(),
        }
    }

    #[test]
    fn resolve_step_session_creation_settings_without_model_uses_default_backend_and_model() {
        let registry = registry_with_default_backend();

        let settings = resolve_step_session_creation_settings(
            &registry,
            None,
            None,
            &WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        )
        .unwrap();

        assert_eq!(settings.backend_id.as_deref(), Some("codex"));
        assert_eq!(settings.selected_model.as_deref(), Some("gpt-5"));
        assert_eq!(settings.permission_mode, PermissionMode::Edit);
    }

    #[test]
    fn resolve_step_session_creation_settings_without_default_backend_errors() {
        let registry = AgentBackendRegistry::new();

        let error = resolve_step_session_creation_settings(
            &registry,
            None,
            None,
            &WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
        )
        .unwrap_err();

        assert!(matches!(error, WorkflowEngineError::InvalidWorkflow(_)));
        assert!(error.to_string().contains("default backend"));
    }

    #[test]
    fn create_step_session_without_model_persists_non_empty_backend_id() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_with_default_backend();
        let session_store = crate::test_support::build_session_store();

        let session = create_step_session_with_settings(
            &registry,
            &session_store,
            tmp.path(),
            "/repo",
            None,
            None,
            &WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
            workflow_context_for_test(),
            StepRuntimeKindContext::session(),
        )
        .unwrap();

        assert_eq!(session.backend_id.as_deref(), Some("codex"));
        assert_eq!(session.selected_model.as_deref(), Some("gpt-5"));
        let saved = session_store
            .get_session_meta(tmp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.backend_id, "codex");
        assert_eq!(saved.selected_model.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn post_commit_runtime_start_failure_reasons_match_state_contract() {
        let error = WorkflowEngineError::AgentSession("backend unavailable".to_string());

        assert_eq!(
            runtime_start_failure_reason(RuntimeStartFailureKind::StepSession, &error),
            "Failed to start step session: backend unavailable"
        );
        assert_eq!(
            runtime_start_failure_reason(RuntimeStartFailureKind::ParallelChildren, &error),
            "Failed to start parallel children: backend unavailable"
        );
        assert!(matches!(
            runtime_start_failed_state(RuntimeStartFailureKind::StepSession, &error),
            WorkflowExecutionState::Failed { reason, .. }
                if reason == "Failed to start step session: backend unavailable"
        ));
    }

    #[test]
    fn runtime_start_failed_state_preserves_startup_timeout_metadata() {
        let error = WorkflowEngineError::from(AgentRuntimeError::StartupTimeout {
            retry_count: 2,
            max_retries: 2,
        });

        let state = runtime_start_failed_state(RuntimeStartFailureKind::StepSession, &error);

        assert!(matches!(
            state,
            WorkflowExecutionState::Failed {
                reason,
                kind: WorkflowStepFailureKind::StartupTimeout,
                retry_count: Some(2),
            } if reason.contains("Timed out waiting for agent session startup")
        ));
    }

    #[test]
    fn runtime_start_failed_state_maps_validation_errors_to_validation_failure() {
        let error = WorkflowEngineError::InvalidWorkflow("missing facet: review".to_string());

        let state = runtime_start_failed_state(RuntimeStartFailureKind::StepSession, &error);

        assert!(matches!(
            state,
            WorkflowExecutionState::Failed {
                reason,
                kind: WorkflowStepFailureKind::ValidationFailure,
                retry_count: None,
            } if reason == "Failed to start step session: missing facet: review"
        ));
    }

    #[test]
    fn workflow_step_context_with_runtime_timeouts_injects_gateway_policy_values() {
        let settings = StepSessionCreationSettings {
            backend_id: Some("codex".to_string()),
            selected_model: Some("gpt-5.5".to_string()),
            permission_mode: PermissionMode::Edit,
        };
        let context = WorkflowStepContext {
            run_id: "run-1".to_string(),
            workflow_name: "05_review-fix_gpt55".to_string(),
            step_name: "review".to_string(),
            run_index: 1,
            parent_step_name: None,
            parent_run_index: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        };

        let context = workflow_step_context_with_runtime_timeouts(
            &settings,
            context,
            StepRuntimeKindContext::session(),
        );

        assert_eq!(context.startup_timeout_secs, Some(30));
        assert_eq!(context.startup_max_retries, Some(2));
        assert_eq!(context.stale_timeout_secs, Some(600));
    }

    #[test]
    fn workflow_step_context_with_runtime_timeouts_injects_template_only_policy_values() {
        let settings = StepSessionCreationSettings {
            backend_id: Some("codex".to_string()),
            selected_model: Some("unknown-fast".to_string()),
            permission_mode: PermissionMode::Edit,
        };
        let context = WorkflowStepContext {
            run_id: "run-1".to_string(),
            workflow_name: "05_review-fix_gpt55".to_string(),
            step_name: "review".to_string(),
            run_index: 1,
            parent_step_name: None,
            parent_run_index: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        };

        let context = workflow_step_context_with_runtime_timeouts(
            &settings,
            context,
            StepRuntimeKindContext::session(),
        );

        assert_eq!(context.startup_timeout_secs, Some(30));
        assert_eq!(context.startup_max_retries, Some(2));
        assert_eq!(context.stale_timeout_secs, Some(600));
    }

    #[test]
    fn workflow_step_context_with_runtime_timeouts_injects_approval_gate_policy_values() {
        let settings = StepSessionCreationSettings {
            backend_id: Some("codex".to_string()),
            selected_model: Some("unknown-fast".to_string()),
            permission_mode: PermissionMode::Edit,
        };
        let context = WorkflowStepContext {
            run_id: "run-1".to_string(),
            workflow_name: "unknown-template".to_string(),
            step_name: "approval".to_string(),
            run_index: 1,
            parent_step_name: None,
            parent_run_index: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        };

        let context = workflow_step_context_with_runtime_timeouts(
            &settings,
            context,
            StepRuntimeKindContext::new(NodeKindName::Session, true),
        );

        assert_eq!(context.startup_timeout_secs, Some(30));
        assert_eq!(context.startup_max_retries, Some(2));
        assert_eq!(context.stale_timeout_secs, Some(600));
    }

    #[tokio::test]
    async fn record_step_session_start_failed_by_run_id_appends_history_entry() {
        let executions = Mutex::new(HashMap::from([(
            "run-1".to_string(),
            workflow_execution_fixture("run-1", "/tmp/repo"),
        )]));
        let error = WorkflowEngineError::AgentSession("turn failed".to_string());

        record_step_session_start_failed_by_run_id(&executions, "run-1", &error).await;

        let execs = executions.lock().await;
        let exec = execs.get("run-1").unwrap();
        let entry = exec.step_history.last().unwrap();
        assert_eq!(
            entry.result.as_deref(),
            Some("session_start_failed: turn failed")
        );
        assert_eq!(entry.session_id.as_deref(), Some("session-1"));
        assert!(exec.current_session_id.is_none());
    }

    #[tokio::test]
    async fn record_step_session_start_failed_by_worktree_uses_active_execution() {
        let executions = Mutex::new(HashMap::from([(
            "run-1".to_string(),
            workflow_execution_fixture("run-1", "/tmp/repo"),
        )]));
        let error = WorkflowEngineError::AgentSession("session failed".to_string());

        record_step_session_start_failed_by_worktree(&executions, "/tmp/repo", &error).await;

        let execs = executions.lock().await;
        let exec = execs.get("run-1").unwrap();
        assert_eq!(exec.step_history.len(), 1);
        assert_eq!(
            exec.step_history[0].result.as_deref(),
            Some("session_start_failed: session failed")
        );
    }

    #[tokio::test]
    async fn record_post_commit_runtime_start_failure_records_step_session_history() {
        let executions = Mutex::new(HashMap::from([(
            "run-1".to_string(),
            workflow_execution_fixture("run-1", "/tmp/repo"),
        )]));
        let error = WorkflowEngineError::AgentSession("start failed".to_string());

        let state = record_post_commit_runtime_start_failure(
            &executions,
            "/tmp/repo",
            RuntimeStartFailureKind::StepSession,
            &error,
        )
        .await;

        assert!(matches!(
            state,
            WorkflowExecutionState::Failed { reason, .. }
                if reason == "Failed to start step session: start failed"
        ));
        let execs = executions.lock().await;
        assert_eq!(execs["run-1"].step_history.len(), 1);
        assert_eq!(
            execs["run-1"].step_history[0].result.as_deref(),
            Some("session_start_failed: start failed")
        );
    }

    #[tokio::test]
    async fn record_post_commit_runtime_start_failure_leaves_parallel_history_unchanged() {
        let executions = Mutex::new(HashMap::from([(
            "run-1".to_string(),
            workflow_execution_fixture("run-1", "/tmp/repo"),
        )]));
        let error = WorkflowEngineError::AgentSession("parallel failed".to_string());

        let state = record_post_commit_runtime_start_failure(
            &executions,
            "/tmp/repo",
            RuntimeStartFailureKind::ParallelChildren,
            &error,
        )
        .await;

        assert!(matches!(
            state,
            WorkflowExecutionState::Failed { reason, .. }
                if reason == "Failed to start parallel children: parallel failed"
        ));
        let execs = executions.lock().await;
        assert!(execs["run-1"].step_history.is_empty());
    }

    #[tokio::test]
    async fn load_parallel_start_runtime_inputs_reads_context_and_prompt_inputs() {
        let mut exec = workflow_execution_fixture("run-1", "/tmp/repo");
        exec.workflow.nodes[0] = NodeDefinition {
            name: "parallel-review".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                parallel_children: vec![InterimChild {
                    name: "review-a".to_string(),
                    ..Default::default()
                }],
                aggregate: None,
            }),
            ..Default::default()
        };
        exec.step_outputs.insert(
            "plan".to_string(),
            StepOutput {
                step_name: "plan".to_string(),
                run_index: 1,
                session_id: Some("plan-session".to_string()),
                result: Some("DONE".to_string()),
                structured_output: Some(serde_json::json!({ "status": "ok" })),
                artifact_contract: None,
                token_usage: None,
                completed_at: 2.0,
            },
        );
        let executions = Mutex::new(HashMap::from([("run-1".to_string(), exec)]));

        let inputs = load_parallel_start_runtime_inputs(&executions, "/tmp/repo")
            .await
            .unwrap();

        assert_eq!(inputs.parallel_start.parent_step_name, "parallel-review");
        assert_eq!(
            inputs.parallel_start.child_step_names(),
            vec!["review-a".to_string()]
        );
        assert_eq!(
            inputs.prompt_inputs.step_outputs["plan"].structured_output,
            Some(serde_json::json!({ "status": "ok" }))
        );
    }
}
