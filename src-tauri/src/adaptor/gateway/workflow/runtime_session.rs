use std::sync::Arc;
use std::{collections::HashMap, path::Path};

use tauri::Manager;
use tokio::sync::Mutex;

use crate::adaptor::gateway::workflow::domain_mapping::node_type_to_domain;
use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::execution_registry::{
    find_by_worktree, find_by_worktree_mut,
};
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
use crate::domain::workflow::services::history::{
    self as workflow_history, RuntimeStartFailureKind,
};
use crate::domain::workflow::{
    NodeType, RetryPolicy, TimeoutContext, WorkflowStepContext, WorkflowStepFailureKind,
};
use crate::infrastructure::agent_session::runtime::AgentProcessMap;
use crate::permission::PermissionMode;
use crate::usecase::agent_session::session::{ChatSession, OpenTabRegistry, SessionStore};

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
pub(crate) async fn resolve_backend_for_step_model<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    model: &str,
) -> Result<Option<String>, WorkflowEngineError> {
    let registry = app
        .try_state::<Arc<crate::infrastructure::agent_session::runtime::AgentBackendRegistry>>()
        .ok_or_else(|| {
            WorkflowEngineError::InvalidWorkflow(format!(
                "cannot resolve model '{model}': backend registry is unavailable"
            ))
        })?;
    resolve_step_model_with_registry(&registry, model).map(Some)
}

/// 形式検証＋登録判定をレジストリ単体で行う、ワークフロー経路用の解決関数。
/// `resolve_backend_for_step_model` の実体ロジックで、テストではこちらを直接呼ぶ。
pub(crate) fn resolve_step_model_with_registry(
    registry: &crate::infrastructure::agent_session::runtime::AgentBackendRegistry,
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

#[derive(Debug, Clone)]
struct StepSessionCreationSettings {
    backend_id: Option<String>,
    selected_model: Option<String>,
    permission_mode: PermissionMode,
}

async fn resolve_step_session_creation_settings<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    step_model: Option<String>,
    step_permission: Option<String>,
    workflow_defaults: &WorkflowDefaults,
) -> Result<StepSessionCreationSettings, WorkflowEngineError> {
    let resolved_backend_id = match step_model {
        Some(ref model) => resolve_backend_for_step_model(app, model).await?,
        None => None,
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
    node_kind: NodeType,
) -> Result<ChatSession, WorkflowEngineError> {
    let workflow_step_context =
        workflow_step_context_with_runtime_timeouts(&settings, workflow_step_context, node_kind);
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
    node_kind: NodeType,
) -> WorkflowStepContext {
    let timeout_context = TimeoutContext::new(
        settings.selected_model.clone(),
        node_kind,
        Some(workflow_step_context.workflow_name.clone()),
    );
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
pub(crate) async fn create_step_session_with_settings<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    data_dir: &Path,
    worktree_path: &str,
    step_model: Option<String>,
    step_permission: Option<String>,
    workflow_defaults: &WorkflowDefaults,
    workflow_step_context: WorkflowStepContext,
    node_kind: NodeType,
) -> Result<ChatSession, WorkflowEngineError> {
    let settings =
        resolve_step_session_creation_settings(app, step_model, step_permission, workflow_defaults)
            .await?;
    create_step_session_from_resolved_settings(
        session_store,
        data_dir,
        worktree_path,
        settings,
        workflow_step_context,
        node_kind,
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
    handles: &Arc<Mutex<AgentProcessMap>>,
    open_tabs: &OpenTabRegistry,
    worktree_path: &str,
    workflow_state: WorkflowState,
) {
    crate::adaptor::gateway::workflow::emit_workflow_state_from_snapshot(
        app,
        worktree_path,
        crate::adaptor::gateway::workflow::state::workflow_state_to_domain_snapshot(workflow_state),
        handles,
        open_tabs,
    )
    .await;
}

/// AgentSessionを中断する。
pub(crate) async fn interrupt_agent(handles: &Arc<Mutex<AgentProcessMap>>, session_id: &str) {
    use tokio::io::AsyncWriteExt;

    let stdin = {
        let map = handles.lock().await;
        map.get(session_id).map(|proc| Arc::clone(&proc.stdin))
    };
    if let Some(stdin) = stdin {
        let mut stdin = stdin.lock().await;
        if let Err(e) = stdin.write_all(b"{\"type\":\"interrupt\"}\n").await {
            log::warn!(
                "Failed to write interrupt for session '{}': {e}",
                session_id
            );
        }
        if let Err(e) = stdin.flush().await {
            log::warn!(
                "Failed to flush interrupt for session '{}': {e}",
                session_id
            );
        }
    }
}

pub(crate) async fn release_completed_step_session<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_id: &str,
) {
    crate::adaptor::gateway::workflow::release_step_runtime_on_done(
        app,
        session_store,
        handles,
        session_id,
    )
    .await;
}

pub(crate) async fn release_completed_step_sessions<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
    session_ids: &[String],
) {
    for session_id in session_ids {
        release_completed_step_session(app, session_store, handles, session_id).await;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare_parallel_child_session_setups<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &Arc<SessionStore>,
    session_workflow_refs: &Mutex<HashMap<String, SessionWorkflowRef>>,
    worktree_path: &str,
    parallel_start: &ParallelStartContext,
    prompt_inputs: &ParallelPromptInputs,
) -> Result<Vec<ParallelChildSessionSetup>, WorkflowEngineError> {
    let prompt_plans =
        prepare_parallel_child_prompt_plans(worktree_path, parallel_start, prompt_inputs)?;
    let creation_plans =
        prepare_parallel_child_creation_plans(app, parallel_start, prompt_plans).await?;
    let data_dir = crate::app_data_dir::resolve_data_dir(app)
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
            creation_plan.node_kind,
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
            user_message: creation_plan.user_message,
            permission_mode: child_permission_mode,
            output_contract: ps.output_contract.clone(),
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
}

struct ParallelChildCreationPlan {
    step_index: usize,
    run_index: u32,
    system_prompt: Option<String>,
    user_message: String,
    settings: StepSessionCreationSettings,
    workflow_step_context: WorkflowStepContext,
    node_kind: NodeType,
}

fn prepare_parallel_child_prompt_plans(
    worktree_path: &str,
    parallel_start: &ParallelStartContext,
    prompt_inputs: &ParallelPromptInputs,
) -> Result<Vec<ParallelChildPromptPlan>, WorkflowEngineError> {
    parallel_start
        .parallel_steps
        .iter()
        .enumerate()
        .zip(parallel_start.child_run_indices.iter().copied())
        .map(|((step_index, ps), run_index)| {
            let (system_prompt, user_message) = workflow_prompt::build_parallel_step_prompt(
                ps,
                &parallel_start.execution_id,
                worktree_path,
                parallel_start.task.as_deref(),
                &prompt_inputs.step_outputs,
                ps.pass_previous_response.unwrap_or(false),
                ps.pass_output_from.as_deref(),
                &prompt_inputs.workflow_variables,
                &prompt_inputs.workflow_declared_variables,
            )?;
            Ok(ParallelChildPromptPlan {
                step_index,
                run_index,
                system_prompt,
                user_message,
            })
        })
        .collect()
}

async fn prepare_parallel_child_creation_plans<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    parallel_start: &ParallelStartContext,
    prompt_plans: Vec<ParallelChildPromptPlan>,
) -> Result<Vec<ParallelChildCreationPlan>, WorkflowEngineError> {
    let mut creation_plans = Vec::with_capacity(prompt_plans.len());
    for prompt_plan in prompt_plans {
        let ps = &parallel_start.parallel_steps[prompt_plan.step_index];
        let settings = resolve_step_session_creation_settings(
            app,
            ps.model.clone(),
            ps.permission.clone(),
            &parallel_start.workflow_defaults,
        )
        .await?;
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
            node_kind: node_type_to_domain(ps.node_type),
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
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
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
        session_store,
        handles,
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
    session_store: &Arc<SessionStore>,
    handles: &Arc<Mutex<AgentProcessMap>>,
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
        let runtime_guard =
            crate::infrastructure::agent_session::runtime::acquire_session_runtime_lock(
                &setup.session_id,
            )
            .await;
        if let Err(e) = crate::infrastructure::agent_session::runtime::start_agent_session_internal(
            app,
            handles,
            session_store,
            &setup.session_id,
            worktree_path,
            None,
            false,
            setup.system_prompt.clone(),
        )
        .await
        {
            for session_id in &created_session_ids {
                interrupt_agent(handles, session_id).await;
            }
            return Err(WorkflowEngineError::with_agent_runtime_context(
                format!("Failed to start parallel child '{}'", setup.step_name),
                e,
            ));
        }
        runtime_guards.push(runtime_guard);
        if let Some(open_tabs) =
            app.try_state::<Arc<crate::usecase::agent_session::session::OpenTabRegistry>>()
        {
            open_tabs.add(&setup.session_id);
            if let Some(state) = workflow_state_for_projection.clone() {
                emit_workflow_runtime_projection(app, handles, &open_tabs, worktree_path, state)
                    .await;
            }
        }
        created_session_ids.push(setup.session_id.clone());
    }

    for (index, setup) in child_setups.iter().enumerate() {
        let runtime_guard = runtime_guards.remove(0);
        if let Err(e) =
            crate::infrastructure::agent_session::runtime::start_agent_turn_internal_locked(
                app,
                handles,
                session_store,
                &setup.session_id,
                worktree_path,
                &setup.permission_mode,
                &setup.user_message,
            )
            .await
        {
            for session_id in &created_session_ids {
                interrupt_agent(handles, session_id).await;
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
        ChildNodeDefinition, NodeDefinition, NodeType, Workflow,
    };
    use crate::adaptor::gateway::workflow::state::{StepOutput, TokenUsage};
    use crate::domain::workflow::WorkflowStepFailureKind;
    use crate::infrastructure::agent_session::runtime::AgentRuntimeError;

    fn workflow_execution_fixture(run_id: &str, worktree_path: &str) -> WorkflowExecution {
        let step_name = "plan".to_string();
        WorkflowExecution {
            id: run_id.to_string(),
            workflow: Workflow {
                name: "test-workflow".to_string(),
                description: String::new(),
                builtin: false,
                variables: HashMap::new(),
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
            workflow_variables: HashMap::new(),
        }
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
        let error = WorkflowEngineError::from(AgentRuntimeError::startup_timeout(2, 2));

        let state = runtime_start_failed_state(RuntimeStartFailureKind::StepSession, &error);

        assert!(matches!(
            state,
            WorkflowExecutionState::Failed {
                reason,
                kind: WorkflowStepFailureKind::StartupTimeout,
                retry_count: Some(2),
            } if reason.contains("Timed out waiting for Codex app-server thread")
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
            crate::domain::workflow::NodeType::Agent,
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
            crate::domain::workflow::NodeType::Agent,
        );

        assert_eq!(context.startup_timeout_secs, Some(30));
        assert_eq!(context.startup_max_retries, Some(2));
        assert_eq!(context.stale_timeout_secs, Some(600));
    }

    #[test]
    fn workflow_step_context_with_runtime_timeouts_injects_node_kind_policy_values() {
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
            crate::domain::workflow::NodeType::Approval,
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
            node_type: NodeType::Parallel,
            parallel_children: Some(vec![ChildNodeDefinition {
                name: "review-a".to_string(),
                ..Default::default()
            }]),
            ..Default::default()
        };
        exec.workflow
            .variables
            .insert("declared".to_string(), "yes".to_string());
        exec.workflow_variables
            .insert("runtime".to_string(), "ready".to_string());
        exec.step_outputs.insert(
            "plan".to_string(),
            StepOutput {
                step_name: "plan".to_string(),
                run_index: 1,
                session_id: Some("plan-session".to_string()),
                result: Some("DONE".to_string()),
                structured_output: Some(serde_json::json!({ "status": "ok" })),
                output_contract: None,
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
            inputs
                .prompt_inputs
                .workflow_variables
                .get("runtime")
                .map(String::as_str),
            Some("ready")
        );
        assert_eq!(
            inputs
                .prompt_inputs
                .workflow_declared_variables
                .get("declared")
                .map(String::as_str),
            Some("yes")
        );
        assert_eq!(
            inputs.prompt_inputs.step_outputs["plan"].structured_output,
            Some(serde_json::json!({ "status": "ok" }))
        );
    }
}
