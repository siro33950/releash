use std::sync::{Arc, Mutex as StdMutex};
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

use tokio::sync::{oneshot, Mutex};

use crate::adaptor::gateway::workflow::engine_error::WorkflowEngineError;
use crate::adaptor::gateway::workflow::execution_registry::find_by_worktree;
use crate::adaptor::gateway::workflow::facet::WorkflowFacetContents;
use crate::adaptor::gateway::workflow::failure_policy_config::workflow_runtime_timeout_policy;
use crate::adaptor::gateway::workflow::fanout_runtime::{
    self as workflow_fanout_runtime, FanoutChildSessionSetup, FanoutPromptInputs,
    FanoutStartContext,
};
use crate::adaptor::gateway::workflow::node_settings::{
    resolve_node_settings, ResolvedNodeSettings, WorkflowDefaults,
};
use crate::adaptor::gateway::workflow::prompt_rendering as workflow_prompt;
use crate::adaptor::gateway::workflow::runtime_state::{SessionWorkflowRef, WorkflowExecution};
use crate::adaptor::gateway::workflow::schema::SchemaDef;
use crate::adaptor::gateway::workflow::state::RuntimeCommitSnapshot;
use crate::domain::agent_session::PermissionMode;
use crate::domain::workflow::{
    NodeExecutionFailureKind, NodeKindName, RetryPolicy, TimeoutContext, WorkflowNodeContext,
};
use crate::usecase::agent_session::backend_registry::AgentBackendRegistry;
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::runtime::usecase::SessionRuntimeLockGuard;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{ChatSession, OpenTabRegistry, SessionStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NodeRuntimeKindContext {
    node_kind: NodeKindName,
    approval_gate: bool,
}

impl NodeRuntimeKindContext {
    pub(crate) fn new(node_kind: NodeKindName, approval_gate: bool) -> Self {
        Self {
            node_kind,
            approval_gate,
        }
    }

    #[cfg(test)]
    pub(crate) fn session() -> Self {
        Self::new(NodeKindName::Session, false)
    }
}

pub(crate) struct FanoutStartRuntimeInputs {
    pub(crate) fanout_start: FanoutStartContext,
    pub(crate) prompt_inputs: FanoutPromptInputs,
}

pub(crate) async fn load_fanout_start_runtime_inputs(
    executions: &Mutex<HashMap<String, WorkflowExecution>>,
    worktree_path: &str,
) -> Result<FanoutStartRuntimeInputs, WorkflowEngineError> {
    let execs = executions.lock().await;
    let (_, exec) = find_by_worktree(&execs, worktree_path)
        .ok_or_else(|| WorkflowEngineError::ExecutionNotFound(worktree_path.to_string()))?;
    Ok(FanoutStartRuntimeInputs {
        fanout_start: workflow_fanout_runtime::prepare_fanout_start_context(exec)?,
        prompt_inputs: workflow_fanout_runtime::fanout_prompt_inputs(exec),
    })
}

/// ステップの model 値から対応するバックエンドIDを解決する。
/// 形式検証（`ModelId`）と登録判定（`resolve_backend_for_model`）を
/// 一括で行い、`set_agent_model_internal` と同一の受け入れ基準を適用する。
pub(crate) fn resolve_backend_for_node_model(
    registry: &AgentBackendRegistry,
    model: &str,
) -> Result<Option<String>, WorkflowEngineError> {
    resolve_node_model_with_registry(registry, model).map(Some)
}

/// 形式検証＋登録判定をレジストリ単体で行う、ワークフロー経路用の解決関数。
/// `resolve_backend_for_node_model` の実体ロジックで、テストではこちらを直接呼ぶ。
pub(crate) fn resolve_node_model_with_registry(
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
struct NodeSessionCreationSettings {
    backend_id: Option<String>,
    selected_model: Option<String>,
    permission_mode: PermissionMode,
}

fn resolve_node_session_creation_settings(
    registry: &AgentBackendRegistry,
    node_model: Option<String>,
    node_permission: Option<String>,
    workflow_defaults: &WorkflowDefaults,
) -> Result<NodeSessionCreationSettings, WorkflowEngineError> {
    let (node_model, resolved_backend_id) = match node_model {
        Some(model) => {
            let backend_id = resolve_backend_for_node_model(registry, &model)?;
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
    let settings = resolve_node_settings(
        node_model,
        node_permission,
        resolved_backend_id,
        workflow_defaults,
    );
    node_session_creation_settings_from_resolved(settings)
}

fn node_session_creation_settings_from_resolved(
    settings: ResolvedNodeSettings,
) -> Result<NodeSessionCreationSettings, WorkflowEngineError> {
    let permission_mode = PermissionMode::parse_canonical(&settings.permission_mode)
        .map_err(|e| WorkflowEngineError::InvalidWorkflow(e.to_string()))?;
    Ok(NodeSessionCreationSettings {
        backend_id: settings.backend_id,
        selected_model: settings.selected_model,
        permission_mode,
    })
}

fn create_node_session_from_resolved_settings(
    session_store: &SessionStore,
    data_dir: &Path,
    worktree_path: &str,
    settings: NodeSessionCreationSettings,
    workflow_node_context: WorkflowNodeContext,
    kind_context: NodeRuntimeKindContext,
) -> Result<ChatSession, WorkflowEngineError> {
    let workflow_node_context =
        workflow_node_context_with_runtime_timeouts(&settings, workflow_node_context, kind_context);
    crate::usecase::agent_session::session::create_session_internal_with_attributes(
        session_store,
        data_dir,
        worktree_path,
        settings.backend_id,
        settings.permission_mode,
        crate::usecase::agent_session::session::SessionCreationAttributes {
            selected_model: settings.selected_model,
            workflow_node_session: true,
            workflow_node_context: Some(workflow_node_context),
            ..Default::default()
        },
    )
    .map_err(|e| WorkflowEngineError::SessionStore(format!("create node session: {e}")))
}

fn workflow_node_context_with_runtime_timeouts(
    settings: &NodeSessionCreationSettings,
    mut workflow_node_context: WorkflowNodeContext,
    kind_context: NodeRuntimeKindContext,
) -> WorkflowNodeContext {
    let timeout_context = TimeoutContext::new(
        settings.selected_model.clone(),
        kind_context.node_kind,
        Some(workflow_node_context.workflow_name.clone()),
    )
    .with_approval_gate(kind_context.approval_gate);
    let policy = workflow_runtime_timeout_policy();
    workflow_node_context.startup_timeout_secs =
        Some(policy.startup_timeout(&timeout_context).as_secs());
    workflow_node_context.startup_max_retries =
        Some(RetryPolicy::default().max_retries(NodeExecutionFailureKind::StartupTimeout));
    workflow_node_context.stale_timeout_secs =
        Some(policy.stale_timeout(&timeout_context).as_secs());
    workflow_node_context
}

/// ステップ設定の解決 → セッション生成 → 解決済み設定の反映 → 保存を一括で行う。
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_node_session_with_settings(
    registry: &AgentBackendRegistry,
    session_store: &SessionStore,
    data_dir: &Path,
    worktree_path: &str,
    node_model: Option<String>,
    node_permission: Option<String>,
    workflow_defaults: &WorkflowDefaults,
    workflow_node_context: WorkflowNodeContext,
    kind_context: NodeRuntimeKindContext,
) -> Result<ChatSession, WorkflowEngineError> {
    let settings = resolve_node_session_creation_settings(
        registry,
        node_model,
        node_permission,
        workflow_defaults,
    )?;
    create_node_session_from_resolved_settings(
        session_store,
        data_dir,
        worktree_path,
        settings,
        workflow_node_context,
        kind_context,
    )
}

/// ワークフロー状態をブロードキャストする。
/// スナップショットは呼び出し元がロック内で確定したものを受け取る。
pub(crate) async fn broadcast_state<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    worktree_path: &str,
    commit_snapshot: RuntimeCommitSnapshot,
) {
    crate::adaptor::gateway::workflow::emit_workflow_execution_from_snapshot(
        app,
        worktree_path,
        crate::adaptor::gateway::workflow::state::runtime_commit_snapshot_to_domain_snapshot(
            commit_snapshot,
        ),
    )
    .await;
}

/// AgentSessionを中断する。
pub(crate) async fn interrupt_agent(runtime: &Arc<AgentSessionRuntimeUsecase>, session_id: &str) {
    if let Err(e) = runtime.interrupt(session_id).await {
        log::warn!("Failed to interrupt agent session '{session_id}': {e}");
    }
}

pub(crate) async fn release_completed_node_session(
    runtime: &Arc<AgentSessionRuntimeUsecase>,
    session_id: &str,
) {
    crate::adaptor::gateway::workflow::release_node_runtime_on_done(runtime, session_id).await;
}

pub(crate) async fn release_completed_node_sessions(
    runtime: &Arc<AgentSessionRuntimeUsecase>,
    session_ids: &[String],
) {
    for session_id in session_ids {
        release_completed_node_session(runtime, session_id).await;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn prepare_fanout_child_session_setups<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    registry: &AgentBackendRegistry,
    session_store: &Arc<SessionStore>,
    session_workflow_refs: &Mutex<HashMap<String, SessionWorkflowRef>>,
    worktree_path: &str,
    fanout_start: &FanoutStartContext,
    prompt_inputs: &FanoutPromptInputs,
    facet_contents: &WorkflowFacetContents,
    schemas: &BTreeMap<String, SchemaDef>,
) -> Result<Vec<FanoutChildSessionSetup>, WorkflowEngineError> {
    let prompt_plans =
        prepare_fanout_child_prompt_plans(fanout_start, prompt_inputs, facet_contents, schemas)?;
    let creation_plans = prepare_fanout_child_creation_plans(registry, fanout_start, prompt_plans)?;
    let data_dir = crate::infrastructure::platform::app_data_dir::resolve_data_dir(app)
        .map_err(|e| WorkflowEngineError::SessionStore(format!("resolve_data_dir: {e}")))?;
    let mut child_setups = Vec::new();
    let mut created_session_ids = Vec::new();

    for creation_plan in creation_plans {
        let child = &fanout_start.children[creation_plan.expansion_index];
        let node_session = match create_node_session_from_resolved_settings(
            session_store,
            &data_dir,
            worktree_path,
            creation_plan.settings,
            creation_plan.workflow_node_context,
            creation_plan.kind_context,
        ) {
            Ok(session) => session,
            Err(err) => {
                return Err(rollback_created_fanout_child_sessions(
                    session_store,
                    &data_dir,
                    session_workflow_refs,
                    &created_session_ids,
                    err,
                )
                .await);
            }
        };
        let child_permission_mode = node_session.permission_mode.clone();
        let node_session_id = node_session.id.clone();

        {
            let mut map = session_workflow_refs.lock().await;
            map.insert(
                node_session_id.clone(),
                SessionWorkflowRef {
                    execution_id: fanout_start.execution_id.clone(),
                },
            );
        }
        created_session_ids.push(node_session_id.clone());

        child_setups.push(FanoutChildSessionSetup {
            node_execution_id: child.node_execution_id.clone(),
            node_name: child.node.name.clone(),
            session_id: node_session_id,
            system_prompt: creation_plan.system_prompt,
            workflow_instruction: creation_plan.workflow_instruction,
            user_message: creation_plan.user_message,
            permission_mode: child_permission_mode,
        });
    }

    Ok(child_setups)
}

struct FanoutChildPromptPlan {
    expansion_index: usize,
    attempt: u32,
    system_prompt: Option<String>,
    user_message: String,
    workflow_instruction: Option<String>,
}

struct FanoutChildCreationPlan {
    expansion_index: usize,
    system_prompt: Option<String>,
    user_message: String,
    workflow_instruction: Option<String>,
    settings: NodeSessionCreationSettings,
    workflow_node_context: WorkflowNodeContext,
    kind_context: NodeRuntimeKindContext,
}

fn prepare_fanout_child_prompt_plans(
    fanout_start: &FanoutStartContext,
    prompt_inputs: &FanoutPromptInputs,
    facet_contents: &WorkflowFacetContents,
    schemas: &BTreeMap<String, SchemaDef>,
) -> Result<Vec<FanoutChildPromptPlan>, WorkflowEngineError> {
    fanout_start
        .children
        .iter()
        .enumerate()
        .filter(|(_, child)| child.reused.is_none() && child.node.session().is_some())
        .map(|(expansion_index, child)| {
            let (system_prompt, user_message) = workflow_prompt::build_fanout_child_prompt(
                &child.node,
                facet_contents.for_node(&child.node.name),
                &fanout_start.execution_id,
                fanout_start.request.as_deref(),
                &prompt_inputs.artifacts,
                workflow_prompt::FanoutChildPromptContext::new(
                    child.item.as_ref(),
                    &child.node_execution_id,
                ),
                schemas,
            )?;
            Ok(FanoutChildPromptPlan {
                expansion_index,
                attempt: child.attempt,
                system_prompt,
                user_message,
                workflow_instruction: workflow_prompt::render_fanout_child_workflow_instruction(
                    &child.node,
                    facet_contents.for_node(&child.node.name),
                    fanout_start.request.as_deref(),
                    &prompt_inputs.artifacts,
                    child.item.as_ref(),
                ),
            })
        })
        .collect()
}

fn prepare_fanout_child_creation_plans(
    registry: &AgentBackendRegistry,
    fanout_start: &FanoutStartContext,
    prompt_plans: Vec<FanoutChildPromptPlan>,
) -> Result<Vec<FanoutChildCreationPlan>, WorkflowEngineError> {
    let mut creation_plans = Vec::with_capacity(prompt_plans.len());
    for prompt_plan in prompt_plans {
        let child = &fanout_start.children[prompt_plan.expansion_index];
        let session = child.node.session().ok_or_else(|| {
            WorkflowEngineError::InvalidState(format!(
                "fanout child '{}' is not a session node",
                child.node.name
            ))
        })?;
        let settings = resolve_node_session_creation_settings(
            registry,
            session.model.clone(),
            session.permission.clone(),
            &fanout_start.workflow_defaults,
        )?;
        creation_plans.push(FanoutChildCreationPlan {
            expansion_index: prompt_plan.expansion_index,
            system_prompt: prompt_plan.system_prompt,
            user_message: prompt_plan.user_message,
            settings,
            workflow_node_context: WorkflowNodeContext {
                execution_id: fanout_start.execution_id.clone(),
                node_execution_id: child.node_execution_id.clone(),
                workflow_name: fanout_start.workflow_name.clone(),
                node_name: child.node.name.clone(),
                attempt: prompt_plan.attempt,
                parent_node_name: Some(fanout_start.parent_node_name.clone()),
                parent_attempt: Some(fanout_start.parent_attempt),
                order: fanout_start.order,
                startup_timeout_secs: None,
                startup_max_retries: None,
                stale_timeout_secs: None,
            },
            kind_context: NodeRuntimeKindContext::new(
                NodeKindName::Session,
                child.node.is_approval_session(),
            ),
            workflow_instruction: prompt_plan.workflow_instruction,
        });
    }
    Ok(creation_plans)
}

async fn rollback_created_fanout_child_sessions(
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
            "fanout child setup failed: {original_error}; rollback failed for created child sessions: {}",
            rollback_errors.join("; ")
        ))
    }
}

pub(crate) async fn rollback_prepared_fanout_child_sessions<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    session_store: &SessionStore,
    session_workflow_refs: &Mutex<HashMap<String, SessionWorkflowRef>>,
    child_setups: &[FanoutChildSessionSetup],
    original_error: WorkflowEngineError,
) -> WorkflowEngineError {
    let data_dir = match crate::infrastructure::platform::app_data_dir::resolve_data_dir(app) {
        Ok(data_dir) => data_dir,
        Err(error) => {
            return WorkflowEngineError::SessionStore(format!(
                "fanout child event commit failed: {original_error}; failed to resolve rollback data dir: {error}"
            ));
        }
    };
    let session_ids = child_setups
        .iter()
        .map(|setup| setup.session_id.clone())
        .collect::<Vec<_>>();
    rollback_created_fanout_child_sessions(
        session_store,
        &data_dir,
        session_workflow_refs,
        &session_ids,
        original_error,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn activate_fanout_child_sessions<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    _branch_diff_context: Option<Arc<dyn BranchDiffContextPort>>,
    _session_store: &Arc<SessionStore>,
    runtime: &Arc<AgentSessionRuntimeUsecase>,
    open_tabs: &Arc<OpenTabRegistry>,
    worktree_path: &str,
    child_setups: &[FanoutChildSessionSetup],
    snapshot: RuntimeCommitSnapshot,
    activation_tasks: Arc<FanoutActivationTaskTracker>,
) -> Result<(), WorkflowEngineError> {
    let activations =
        reserve_fanout_child_sessions(runtime, child_setups, &activation_tasks).await?;
    broadcast_state(app, worktree_path, snapshot).await;
    start_fanout_child_sessions(runtime, open_tabs, activations).await
}

#[derive(Default)]
pub(crate) struct FanoutActivationTaskTracker {
    tasks: StdMutex<Vec<FanoutActivationTaskCancellation>>,
}

struct FanoutActivationTaskCancellation {
    abort: tokio::task::AbortHandle,
    completed: oneshot::Receiver<()>,
}

impl FanoutActivationTaskTracker {
    fn register(&self, abort: tokio::task::AbortHandle, completed: oneshot::Receiver<()>) {
        self.tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(FanoutActivationTaskCancellation { abort, completed });
    }

    pub(crate) async fn abort_and_wait(&self) {
        let tasks = {
            let mut tasks = self
                .tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *tasks)
        };
        for task in &tasks {
            task.abort.abort();
        }
        for task in tasks {
            let _ = task.completed.await;
        }
    }
}

struct FanoutActivationTaskCompletion(Option<oneshot::Sender<()>>);

impl Drop for FanoutActivationTaskCompletion {
    fn drop(&mut self) {
        if let Some(completed) = self.0.take() {
            let _ = completed.send(());
        }
    }
}

struct FanoutChildSessionActivation {
    session_id: String,
    node_name: String,
    permission_mode: PermissionMode,
    user_message: String,
    system_prompt: Option<String>,
    workflow_instructions: Vec<String>,
    reserved: Option<oneshot::Receiver<()>>,
    start: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Option<SessionRuntimeLockGuard>>>,
}

impl Drop for FanoutChildSessionActivation {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn reserve_fanout_child_sessions(
    runtime: &Arc<AgentSessionRuntimeUsecase>,
    child_setups: &[FanoutChildSessionSetup],
    activation_tasks: &FanoutActivationTaskTracker,
) -> Result<Vec<FanoutChildSessionActivation>, WorkflowEngineError> {
    let mut activations = Vec::with_capacity(child_setups.len());

    for setup in child_setups {
        let permission_mode = match PermissionMode::parse_canonical(&setup.permission_mode) {
            Ok(permission_mode) => permission_mode,
            Err(error) => {
                let error = WorkflowEngineError::InvalidWorkflow(error.to_string());
                activation_tasks.abort_and_wait().await;
                return Err(error);
            }
        };
        let runtime = Arc::clone(runtime);
        let session_id = setup.session_id.clone();
        let session_id_for_task = session_id.clone();
        let node_name = setup.node_name.clone();
        let user_message = setup.user_message.clone();
        let system_prompt = setup.system_prompt.clone();
        let workflow_instructions = setup
            .workflow_instruction
            .clone()
            .into_iter()
            .collect::<Vec<_>>();
        let (reserved_tx, reserved_rx) = oneshot::channel();
        let (start_tx, start_rx) = oneshot::channel();
        let (completed_tx, completed_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _completion = FanoutActivationTaskCompletion(Some(completed_tx));
            let runtime_guard = runtime.acquire_session_lock(&session_id_for_task).await;
            if reserved_tx.send(()).is_err() || start_rx.await.is_err() {
                drop(runtime_guard);
                return None;
            }
            Some(runtime_guard)
        });
        activation_tasks.register(task.abort_handle(), completed_rx);
        activations.push(FanoutChildSessionActivation {
            session_id,
            node_name,
            permission_mode,
            user_message,
            system_prompt,
            workflow_instructions,
            reserved: Some(reserved_rx),
            start: Some(start_tx),
            task: Some(task),
        });
    }

    wait_for_fanout_child_session_reservations(&mut activations, activation_tasks).await?;

    Ok(activations)
}

async fn wait_for_fanout_child_session_reservations(
    activations: &mut [FanoutChildSessionActivation],
    activation_tasks: &FanoutActivationTaskTracker,
) -> Result<(), WorkflowEngineError> {
    for activation in activations.iter_mut() {
        let reserved = match activation.reserved.take() {
            Some(reserved) => reserved,
            None => {
                let error = WorkflowEngineError::InvalidState(format!(
                    "fanout child '{}' activation reservation was already consumed",
                    activation.session_id
                ));
                activation_tasks.abort_and_wait().await;
                return Err(error);
            }
        };
        if reserved.await.is_err() {
            let error = WorkflowEngineError::AgentSession(format!(
                "fanout child '{}' activation task ended before reserving its session",
                activation.session_id
            ));
            activation_tasks.abort_and_wait().await;
            return Err(error);
        }
    }

    Ok(())
}

async fn start_fanout_child_sessions(
    runtime: &Arc<AgentSessionRuntimeUsecase>,
    open_tabs: &Arc<OpenTabRegistry>,
    mut activations: Vec<FanoutChildSessionActivation>,
) -> Result<(), WorkflowEngineError> {
    let created_session_ids = activations
        .iter()
        .map(|activation| activation.session_id.clone())
        .collect::<Vec<_>>();

    for activation in &activations {
        open_tabs.add(&activation.session_id);
    }

    for activation in &mut activations {
        if let Err(error) = start_single_fanout_child(runtime, activation).await {
            for session_id in &created_session_ids {
                interrupt_agent(runtime, session_id).await;
            }
            return Err(error);
        }
    }

    Ok(())
}

async fn start_single_fanout_child(
    runtime: &Arc<AgentSessionRuntimeUsecase>,
    activation: &mut FanoutChildSessionActivation,
) -> Result<(), WorkflowEngineError> {
    let started = activation
        .start
        .take()
        .is_some_and(|start| start.send(()).is_ok());
    if !started {
        return Err(WorkflowEngineError::AgentSession(format!(
            "fanout child '{}' activation task ended before start",
            activation.session_id
        )));
    }

    let task = activation.task.take().ok_or_else(|| {
        WorkflowEngineError::InvalidState(format!(
            "fanout child '{}' activation task was already consumed",
            activation.session_id
        ))
    })?;
    let runtime_guard = task
        .await
        .map_err(|error| {
            WorkflowEngineError::AgentSession(format!(
                "fanout child '{}' activation task failed: {error}",
                activation.session_id
            ))
        })?
        .ok_or_else(|| {
            WorkflowEngineError::AgentSession(format!(
                "fanout child '{}' activation task ended before transferring its session reservation",
                activation.session_id
            ))
        })?;
    #[cfg(test)]
    let mut runtime_guard = runtime_guard;
    #[cfg(test)]
    runtime_guard.adopt_for_current_test_flow();
    let result = runtime
        .start_turn_locked(
            &activation.session_id,
            activation.permission_mode,
            std::mem::take(&mut activation.user_message),
            activation.system_prompt.take(),
            std::mem::take(&mut activation.workflow_instructions),
        )
        .await;
    drop(runtime_guard);
    result.map_err(|error| {
        WorkflowEngineError::with_agent_runtime_context(
            format!(
                "Failed to start turn for fanout child '{}'",
                activation.node_name
            ),
            error,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adaptor::gateway::workflow::schema::{
        FanoutSpec, NodeDefinition, NodeKind, WorkflowDefinitionYaml,
    };
    use crate::adaptor::gateway::workflow::state::{
        RuntimeArtifact, RuntimeExecutionState, TokenUsage,
    };
    use crate::domain::agent_session::gateway::{
        AgentBackend, AgentBackendError, AgentSessionRuntime, ForkSessionRequest, SessionSpec,
    };
    use crate::domain::agent_session::value_objects::{
        BackendCapabilities, ModelDescriptor, ModelId, SkillEntry,
    };
    use crate::domain::workflow::NodeKindName;
    use crate::test_support::TestRuntimeCallKind;
    use crate::usecase::agent_session::runtime::SendAgentMessageRequest;
    use crate::usecase::agent_session::session::SessionCreationAttributes;
    use async_trait::async_trait;
    use std::time::Duration;

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

    async fn register_held_session_lock(
        runtime: &Arc<AgentSessionRuntimeUsecase>,
        activation_tasks: &FanoutActivationTaskTracker,
        session_id: &str,
    ) {
        let runtime = Arc::clone(runtime);
        let session_id = session_id.to_string();
        let (acquired_tx, acquired_rx) = oneshot::channel();
        let (completed_tx, completed_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _completion = FanoutActivationTaskCompletion(Some(completed_tx));
            let _runtime_guard = runtime.acquire_session_lock(&session_id).await;
            let _ = acquired_tx.send(());
            std::future::pending::<()>().await;
        });
        activation_tasks.register(task.abort_handle(), completed_rx);
        tokio::time::timeout(Duration::from_secs(1), acquired_rx)
            .await
            .expect("test session lock must be acquired")
            .expect("test session lock task must remain active");
    }

    fn fanout_activation_with_reservation(
        session_id: &str,
        reserved: Option<oneshot::Receiver<()>>,
    ) -> FanoutChildSessionActivation {
        FanoutChildSessionActivation {
            session_id: session_id.to_string(),
            node_name: "child".to_string(),
            permission_mode: PermissionMode::Edit,
            user_message: "workflow-child".to_string(),
            system_prompt: None,
            workflow_instructions: Vec::new(),
            reserved,
            start: None,
            task: None,
        }
    }

    fn workflow_context_for_test() -> WorkflowNodeContext {
        WorkflowNodeContext {
            execution_id: "execution-1".to_string(),
            node_execution_id: "node-execution-1".to_string(),
            workflow_name: "workflow".to_string(),
            node_name: "node".to_string(),
            attempt: 0,
            parent_node_name: None,
            parent_attempt: None,
            order: 1,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        }
    }

    fn workflow_execution_fixture(execution_id: &str, worktree_path: &str) -> WorkflowExecution {
        let node_name = "plan".to_string();
        WorkflowExecution {
            id: execution_id.to_string(),
            workflow: WorkflowDefinitionYaml {
                name: "test-workflow".to_string(),
                description: String::new(),
                builtin: false,
                schemas: Default::default(),
                nodes: vec![NodeDefinition {
                    name: node_name.clone(),
                    ..Default::default()
                }],
            },
            state: RuntimeExecutionState::Running,
            current_node_index: 0,
            node_execution_counts: HashMap::from([(node_name, 1)]),
            loop_guard_reset_baselines: Default::default(),
            node_history: Vec::new(),
            workflow_defaults: WorkflowDefaults {
                backend_id: None,
                permission_mode: "edit".to_string(),
            },
            worktree_path: worktree_path.to_string(),
            created_from: crate::domain::workflow::ExecutionOrigin::Cli,
            error_reason: None,
            started_at: 1.0,
            updated_at: 1.0,
            current_session_id: Some("session-1".to_string()),
            current_node_token_usage: TokenUsage::default(),
            artifacts: HashMap::new(),
            node_executions: Vec::new(),
            request: None,
            fanout_runtime: None,
            current_stall_observations: Vec::new(),
        }
    }

    #[test]
    fn resolve_node_session_creation_settings_without_model_uses_default_backend_and_model() {
        let registry = registry_with_default_backend();

        let settings = resolve_node_session_creation_settings(
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
    fn resolve_node_session_creation_settings_without_default_backend_errors() {
        let registry = AgentBackendRegistry::new();

        let error = resolve_node_session_creation_settings(
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
    fn create_node_session_without_model_persists_non_empty_backend_id() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = registry_with_default_backend();
        let session_store = crate::test_support::build_session_store();

        let session = create_node_session_with_settings(
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
            NodeRuntimeKindContext::session(),
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
    fn workflow_node_context_with_runtime_timeouts_injects_gateway_policy_values() {
        let settings = NodeSessionCreationSettings {
            backend_id: Some("codex".to_string()),
            selected_model: Some("gpt-5.6-sol".to_string()),
            permission_mode: PermissionMode::Edit,
        };
        let context = WorkflowNodeContext {
            execution_id: "execution-1".to_string(),
            node_execution_id: "node-execution-1".to_string(),
            workflow_name: "05_review-fix_codex".to_string(),
            node_name: "review".to_string(),
            attempt: 1,
            parent_node_name: None,
            parent_attempt: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        };

        let context = workflow_node_context_with_runtime_timeouts(
            &settings,
            context,
            NodeRuntimeKindContext::session(),
        );

        assert_eq!(context.startup_timeout_secs, Some(30));
        assert_eq!(context.startup_max_retries, Some(2));
        assert_eq!(context.stale_timeout_secs, Some(600));
    }

    #[test]
    fn workflow_node_context_with_runtime_timeouts_injects_template_only_policy_values() {
        let settings = NodeSessionCreationSettings {
            backend_id: Some("codex".to_string()),
            selected_model: Some("unknown-fast".to_string()),
            permission_mode: PermissionMode::Edit,
        };
        let context = WorkflowNodeContext {
            execution_id: "execution-1".to_string(),
            node_execution_id: "node-execution-1".to_string(),
            workflow_name: "05_review-fix_codex".to_string(),
            node_name: "review".to_string(),
            attempt: 1,
            parent_node_name: None,
            parent_attempt: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        };

        let context = workflow_node_context_with_runtime_timeouts(
            &settings,
            context,
            NodeRuntimeKindContext::session(),
        );

        assert_eq!(context.startup_timeout_secs, Some(30));
        assert_eq!(context.startup_max_retries, Some(2));
        assert_eq!(context.stale_timeout_secs, Some(600));
    }

    #[test]
    fn workflow_node_context_with_runtime_timeouts_injects_approval_gate_policy_values() {
        let settings = NodeSessionCreationSettings {
            backend_id: Some("codex".to_string()),
            selected_model: Some("unknown-fast".to_string()),
            permission_mode: PermissionMode::Edit,
        };
        let context = WorkflowNodeContext {
            execution_id: "execution-1".to_string(),
            node_execution_id: "node-execution-1".to_string(),
            workflow_name: "unknown-template".to_string(),
            node_name: "approval".to_string(),
            attempt: 1,
            parent_node_name: None,
            parent_attempt: None,
            order: 0,
            startup_timeout_secs: None,
            startup_max_retries: None,
            stale_timeout_secs: None,
        };

        let context = workflow_node_context_with_runtime_timeouts(
            &settings,
            context,
            NodeRuntimeKindContext::new(NodeKindName::Session, true),
        );

        assert_eq!(context.startup_timeout_secs, Some(30));
        assert_eq!(context.startup_max_retries, Some(2));
        assert_eq!(context.stale_timeout_secs, Some(600));
    }

    #[tokio::test]
    async fn load_fanout_start_runtime_inputs_reads_context_and_prompt_inputs() {
        let mut exec = workflow_execution_fixture("execution-1", "/tmp/repo");
        exec.workflow.nodes[0] = NodeDefinition {
            name: "fanout-review".to_string(),
            kind: NodeKind::Fanout(FanoutSpec {
                child: vec!["review-a".to_string()],
                items: None,
            }),
            ..Default::default()
        };
        exec.workflow.nodes.push(NodeDefinition {
            name: "review-a".to_string(),
            ..Default::default()
        });
        exec.artifacts.insert(
            "plan".to_string(),
            RuntimeArtifact {
                node_name: "plan".to_string(),
                attempt: 1,
                session_id: Some("plan-session".to_string()),
                result: Some("DONE".to_string()),
                artifact: Some(serde_json::json!({ "status": "ok" })),
                contract: None,
                token_usage: None,
                completed_at: 2.0,
            },
        );
        let executions = Mutex::new(HashMap::from([("execution-1".to_string(), exec)]));

        let inputs = load_fanout_start_runtime_inputs(&executions, "/tmp/repo")
            .await
            .unwrap();

        assert_eq!(inputs.fanout_start.parent_node_name, "fanout-review");
        assert_eq!(
            inputs.fanout_start.child_node_names(),
            vec!["review-a".to_string()]
        );
        assert_eq!(
            inputs.prompt_inputs.artifacts["plan"].artifact,
            Some(serde_json::json!({ "status": "ok" }))
        );
    }

    #[test]
    fn fanout_resume_plans_prompts_and_session_creation_only_for_unconfirmed_children() {
        let mut exec = workflow_execution_fixture("execution-1", "/tmp/repo");
        exec.current_session_id = None;
        exec.workflow.nodes = vec![
            NodeDefinition {
                name: "fanout-review".to_string(),
                kind: NodeKind::Fanout(FanoutSpec {
                    child: vec!["review-reused".to_string(), "review-pending".to_string()],
                    items: None,
                }),
                ..Default::default()
            },
            NodeDefinition {
                name: "review-reused".to_string(),
                kind: NodeKind::Session(
                    crate::adaptor::gateway::workflow::schema::SessionSpec::default(),
                ),
                artifact: Some("review".to_string()),
                ..Default::default()
            },
            NodeDefinition {
                name: "review-pending".to_string(),
                kind: NodeKind::Session(
                    crate::adaptor::gateway::workflow::schema::SessionSpec::default(),
                ),
                artifact: Some("review".to_string()),
                ..Default::default()
            },
        ];

        let prompt_inputs = workflow_fanout_runtime::fanout_prompt_inputs(&exec);
        let mut fanout_start =
            workflow_fanout_runtime::prepare_fanout_start_context(&exec).unwrap();
        let reused_node_execution_id = fanout_start.children[0].node_execution_id.clone();
        let pending_node_execution_id = fanout_start.children[1].node_execution_id.clone();
        fanout_start.children[0].reused = Some(workflow_fanout_runtime::ReusableFanoutChild {
            result: Some("already confirmed".to_string()),
            display_command: None,
            artifact: Some(serde_json::json!({ "verdict": "pass" })),
            contract: Some("review".to_string()),
            token_usage: Some(TokenUsage {
                input_tokens: 3,
                output_tokens: 4,
            }),
            completed_at: 2.0,
        });

        let prompt_plans = prepare_fanout_child_prompt_plans(
            &fanout_start,
            &prompt_inputs,
            &WorkflowFacetContents::default(),
            &exec.workflow.schemas,
        )
        .unwrap();

        assert_eq!(prompt_plans.len(), 1);
        assert_eq!(prompt_plans[0].expansion_index, 1);
        assert!(prompt_plans[0]
            .user_message
            .contains(&pending_node_execution_id));
        assert!(!prompt_plans[0]
            .user_message
            .contains(&reused_node_execution_id));

        let creation_plans = prepare_fanout_child_creation_plans(
            &registry_with_default_backend(),
            &fanout_start,
            prompt_plans,
        )
        .unwrap();

        assert_eq!(creation_plans.len(), 1);
        assert_eq!(creation_plans[0].expansion_index, 1);
        assert_eq!(
            creation_plans[0].workflow_node_context.node_execution_id,
            pending_node_execution_id
        );
        assert_eq!(
            creation_plans[0].workflow_node_context.node_name,
            "review-pending"
        );
    }

    #[tokio::test]
    async fn invalid_fanout_permission_waits_for_activation_task_cleanup_before_returning() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let (runtime, _) = crate::test_support::build_agent_runtime_usecase_with_controller(
            session_store,
            tmp.path(),
        );
        let activation_tasks = FanoutActivationTaskTracker::default();
        let held_session_id = "held-before-invalid-permission";
        register_held_session_lock(&runtime, &activation_tasks, held_session_id).await;
        let invalid_permission_mode = "invalid";
        let setups = vec![FanoutChildSessionSetup {
            node_execution_id: "node-execution-child".to_string(),
            node_name: "child".to_string(),
            session_id: "child-session".to_string(),
            system_prompt: None,
            workflow_instruction: None,
            user_message: "workflow-child".to_string(),
            permission_mode: invalid_permission_mode.to_string(),
        }];

        let error = match reserve_fanout_child_sessions(&runtime, &setups, &activation_tasks).await
        {
            Ok(_) => panic!("invalid permission mode must fail fanout reservation"),
            Err(error) => error,
        };

        match error {
            WorkflowEngineError::InvalidWorkflow(message) => assert_eq!(
                message,
                PermissionMode::parse_canonical(invalid_permission_mode)
                    .unwrap_err()
                    .to_string()
            ),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!runtime.session_runtime_lock_is_held_for_test(held_session_id));
    }

    #[tokio::test]
    async fn consumed_fanout_reservation_waits_for_activation_task_cleanup_before_returning() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let (runtime, _) = crate::test_support::build_agent_runtime_usecase_with_controller(
            session_store,
            tmp.path(),
        );
        let activation_tasks = FanoutActivationTaskTracker::default();
        let held_session_id = "held-before-consumed-reservation";
        register_held_session_lock(&runtime, &activation_tasks, held_session_id).await;
        let mut activations = vec![fanout_activation_with_reservation("child-session", None)];

        let error = wait_for_fanout_child_session_reservations(&mut activations, &activation_tasks)
            .await
            .unwrap_err();

        match error {
            WorkflowEngineError::InvalidState(message) => assert_eq!(
                message,
                "fanout child 'child-session' activation reservation was already consumed"
            ),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!runtime.session_runtime_lock_is_held_for_test(held_session_id));
    }

    #[tokio::test]
    async fn ended_fanout_reservation_task_waits_for_activation_task_cleanup_before_returning() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let (runtime, _) = crate::test_support::build_agent_runtime_usecase_with_controller(
            session_store,
            tmp.path(),
        );
        let activation_tasks = FanoutActivationTaskTracker::default();
        let held_session_id = "held-before-ended-reservation";
        register_held_session_lock(&runtime, &activation_tasks, held_session_id).await;
        let (reserved_tx, reserved_rx) = oneshot::channel();
        drop(reserved_tx);
        let mut activations = vec![fanout_activation_with_reservation(
            "child-session",
            Some(reserved_rx),
        )];

        let error = wait_for_fanout_child_session_reservations(&mut activations, &activation_tasks)
            .await
            .unwrap_err();

        match error {
            WorkflowEngineError::AgentSession(message) => assert_eq!(
                message,
                "fanout child 'child-session' activation task ended before reserving its session"
            ),
            other => panic!("unexpected error: {other:?}"),
        }
        assert!(!runtime.session_runtime_lock_is_held_for_test(held_session_id));
    }

    #[tokio::test]
    async fn fanout_reserves_later_child_activation_before_publishing_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::test_support::build_session_store());
        let (runtime, controller) =
            crate::test_support::build_agent_runtime_usecase_with_controller(
                Arc::clone(&session_store),
                tmp.path(),
            );
        let worktree_path = tmp.path().to_string_lossy().to_string();
        let first_session =
            crate::usecase::agent_session::session::create_session_internal_with_attributes(
                &session_store,
                tmp.path(),
                &worktree_path,
                Some("codex".to_string()),
                PermissionMode::Edit,
                SessionCreationAttributes {
                    selected_model: Some("gpt-5".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        let second_session =
            crate::usecase::agent_session::session::create_session_internal_with_attributes(
                &session_store,
                tmp.path(),
                &worktree_path,
                Some("codex".to_string()),
                PermissionMode::Edit,
                SessionCreationAttributes {
                    selected_model: Some("gpt-5".to_string()),
                    ..Default::default()
                },
            )
            .unwrap();
        let setups = vec![
            FanoutChildSessionSetup {
                node_execution_id: "node-execution-first".to_string(),
                node_name: "first".to_string(),
                session_id: first_session.id.clone(),
                system_prompt: None,
                workflow_instruction: None,
                user_message: "workflow-first".to_string(),
                permission_mode: PermissionMode::Edit.as_str().to_string(),
            },
            FanoutChildSessionSetup {
                node_execution_id: "node-execution-second".to_string(),
                node_name: "second".to_string(),
                session_id: second_session.id.clone(),
                system_prompt: None,
                workflow_instruction: None,
                user_message: "workflow-second".to_string(),
                permission_mode: PermissionMode::Edit.as_str().to_string(),
            },
        ];
        controller.pause_start_turn();

        let activation_tasks = FanoutActivationTaskTracker::default();
        let activations = reserve_fanout_child_sessions(&runtime, &setups, &activation_tasks)
            .await
            .unwrap();
        assert!(runtime.session_runtime_lock_is_held_for_test(&first_session.id));
        assert!(runtime.session_runtime_lock_is_held_for_test(&second_session.id));

        let open_tabs = Arc::new(OpenTabRegistry::default());
        let start_runtime = Arc::clone(&runtime);
        let start_open_tabs = Arc::clone(&open_tabs);
        let start = tokio::spawn(async move {
            start_fanout_child_sessions(&start_runtime, &start_open_tabs, activations).await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if controller
                    .call_kinds_for(&first_session.id)
                    .iter()
                    .any(|call| {
                        matches!(
                            call,
                            TestRuntimeCallKind::StartTurnPrompt { prompt }
                                if prompt == "workflow-first"
                        )
                    })
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let send_runtime = Arc::clone(&runtime);
        let second_session_id = second_session.id.clone();
        let send_worktree_path = worktree_path.clone();
        let mut send = tokio::spawn(async move {
            send_runtime
                .send_message(SendAgentMessageRequest {
                    chat_session_id: Some(second_session_id),
                    worktree_path: send_worktree_path,
                    content: "user-second".to_string(),
                    permission_mode: PermissionMode::Edit,
                    plan_mode: false,
                    backend_id: Some("codex".to_string()),
                    model_id: Some("gpt-5".to_string()),
                    images: None,
                    mentions: None,
                    editor_context: None,
                })
                .await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut send)
                .await
                .is_err(),
            "the later child operation must wait behind its reserved workflow activation"
        );
        assert!(controller
            .call_kinds_for(&second_session.id)
            .iter()
            .all(|call| !matches!(call, TestRuntimeCallKind::StartTurnPrompt { .. })));

        controller.release_start_turn();
        start.await.unwrap().unwrap();
        let send_response = send.await.unwrap().unwrap();

        assert!(send_response.agent_message.is_none());
        assert!(send_response.queued_turn.is_some());
        assert_eq!(send_response.pending_queue_count, 1);
        assert_eq!(
            controller
                .call_kinds_for(&second_session.id)
                .into_iter()
                .filter(|call| matches!(call, TestRuntimeCallKind::StartTurnPrompt { .. }))
                .collect::<Vec<_>>(),
            vec![TestRuntimeCallKind::StartTurnPrompt {
                prompt: "workflow-second".to_string(),
            }]
        );
        assert_eq!(
            open_tabs.snapshot(),
            [first_session.id, second_session.id].into_iter().collect()
        );
    }
}
