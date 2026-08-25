use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use futures_util::future::{BoxFuture, FutureExt, Shared};
use tokio::sync::{watch, Mutex, OwnedMutexGuard};

use crate::domain::agent_session::aggregates::{
    AgentSessionTreeParent, ManagedPtyPresence, ResolvedProviderExecutable,
};
use crate::domain::agent_session::repository::VersionedAgentSession;
use crate::domain::agent_session::{
    AgentSessionHistoryGateway, AgentSessionHistoryGatewayError, ProviderAgentLaunchGateway,
    ProviderAgentLaunchGatewayError, ProviderAgentTerminalGateway, ProviderAgentTerminalSpawnError,
    ProviderAvailabilityReader, ProviderSessionLaunch,
};
use crate::domain::provider_lifecycle::{
    ArmedProviderLifecycle, ProviderKind, ProviderLifecycleScope, ProviderLifecycleSlotId,
};
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workflow::SessionPermission;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::provider_lifecycle::{
    ProviderHookHealthUsecase, ProviderLifecycleUsecase, ProviderLifecycleUsecaseError,
};

use super::{AgentSessionCreateRequest, AgentSessionUsecase, AgentSessionUsecaseError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSessionLaunchRequest {
    pub(crate) workspace: WorkspaceIdentity,
    pub(crate) worktree_path: String,
    pub(crate) provider: ProviderKind,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) caller_request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkflowAgentSessionLaunchRequest {
    pub(crate) workspace: WorkspaceIdentity,
    pub(crate) worktree_path: String,
    pub(crate) provider: ProviderKind,
    pub(crate) model: Option<String>,
    pub(crate) permission: Option<SessionPermission>,
    pub(crate) workflow_execution_id: String,
    pub(crate) node_execution_id: String,
    pub(crate) initial_instruction: String,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) caller_request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentSessionHistoryResumeRequest {
    pub(crate) workspace: WorkspaceIdentity,
    pub(crate) worktree_path: String,
    pub(crate) provider: ProviderKind,
    pub(crate) provider_session_id: String,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) caller_request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSessionHistoryResumeOutcome {
    Open(VersionedAgentSession),
    Paused(VersionedAgentSession),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentSessionLaunchUsecaseError {
    ProviderUnavailable,
    InvalidInput,
    Conflict,
    StorageUnavailable,
    LaunchUnavailable,
    TerminalUnavailable,
    TerminalSpawn(ProviderAgentTerminalSpawnError),
    Corrupt,
}

impl std::fmt::Display for AgentSessionLaunchUsecaseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TerminalSpawn(error) => error.fmt(formatter),
            _ => write!(formatter, "{self:?}"),
        }
    }
}

impl std::error::Error for AgentSessionLaunchUsecaseError {}

impl From<ProviderLifecycleUsecaseError> for AgentSessionLaunchUsecaseError {
    fn from(error: ProviderLifecycleUsecaseError) -> Self {
        map_lifecycle_error(error)
    }
}

type StandaloneLaunchOutcome = Result<String, AgentSessionLaunchUsecaseError>;
type SharedStandaloneLaunch = Shared<BoxFuture<'static, StandaloneLaunchOutcome>>;

const COMPLETED_STANDALONE_LAUNCH_CAPACITY: usize = 128;
const ACTIVATED_WORKFLOW_LAUNCH_RETENTION: std::time::Duration =
    std::time::Duration::from_secs(300);

fn issue_agent_session_id(
    caller_request_id: &str,
) -> Result<String, AgentSessionLaunchUsecaseError> {
    if caller_request_id.trim().is_empty() {
        return Err(AgentSessionLaunchUsecaseError::InvalidInput);
    }
    crate::domain::agent_session::launch_resource_id("agent-session", caller_request_id)
        .ok_or(AgentSessionLaunchUsecaseError::InvalidInput)
}

fn issue_lifecycle_slot_id(
    caller_request_id: &str,
) -> Result<ProviderLifecycleSlotId, AgentSessionLaunchUsecaseError> {
    let id = crate::domain::agent_session::launch_resource_id("provider-slot", caller_request_id)
        .ok_or(AgentSessionLaunchUsecaseError::InvalidInput)?;
    ProviderLifecycleSlotId::new(id).map_err(|_| AgentSessionLaunchUsecaseError::Corrupt)
}

#[derive(Default)]
struct StandaloneLaunchRequestRegistry {
    in_flight: HashMap<String, SharedStandaloneLaunch>,
    completed: HashMap<String, StandaloneLaunchOutcome>,
    completion_order: VecDeque<String>,
}

impl StandaloneLaunchRequestRegistry {
    fn recall_completed(&mut self, caller_request_id: &str) -> Option<StandaloneLaunchOutcome> {
        let outcome = self.completed.get(caller_request_id).cloned()?;
        if let Some(position) = self
            .completion_order
            .iter()
            .position(|id| id == caller_request_id)
        {
            if let Some(id) = self.completion_order.remove(position) {
                self.completion_order.push_back(id);
            }
        }
        Some(outcome)
    }

    fn record_completed(&mut self, caller_request_id: String, outcome: StandaloneLaunchOutcome) {
        self.in_flight.remove(&caller_request_id);
        if self
            .completed
            .insert(caller_request_id.clone(), outcome)
            .is_some()
        {
            return;
        }
        self.completion_order.push_back(caller_request_id);
        while self.completion_order.len() > COMPLETED_STANDALONE_LAUNCH_CAPACITY {
            if let Some(evicted) = self.completion_order.pop_front() {
                self.completed.remove(&evicted);
            }
        }
    }
}

pub(crate) struct AgentSessionLaunchUsecase {
    sessions: Arc<AgentSessionUsecase>,
    lifecycle: Arc<ProviderLifecycleUsecase>,
    availability: Arc<dyn ProviderAvailabilityReader>,
    launch_gateway: Arc<dyn ProviderAgentLaunchGateway>,
    terminal: Arc<dyn ProviderAgentTerminalGateway>,
    history: Arc<dyn AgentSessionHistoryGateway>,
    hook_health: Arc<ProviderHookHealthUsecase>,
    standalone_requests: Mutex<StandaloneLaunchRequestRegistry>,
    pending_workflow_launches: Mutex<HashMap<String, PreparedAgentSessionLaunch>>,
    activated_workflow_launches: Arc<Mutex<HashMap<String, WorkflowLaunchActivation>>>,
    hook_health_tasks: std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

#[derive(Clone)]
enum WorkflowLaunchActivation {
    Activating(watch::Receiver<bool>),
    Activated(VersionedAgentSession),
}

struct PreparedAgentSessionLaunch {
    durable: DurableAgentSessionLaunch,
    prepared: crate::domain::agent_session::PreparedProviderLaunch,
    rows: u16,
    cols: u16,
    caller_request_id: String,
}

struct DurableAgentSessionLaunch {
    operation: OwnedMutexGuard<()>,
    created: VersionedAgentSession,
    armed: ArmedProviderLifecycle,
    executable: ResolvedProviderExecutable,
}

impl AgentSessionLaunchUsecase {
    pub(crate) fn new(
        sessions: Arc<AgentSessionUsecase>,
        lifecycle: Arc<ProviderLifecycleUsecase>,
        availability: Arc<dyn ProviderAvailabilityReader>,
        launch_gateway: Arc<dyn ProviderAgentLaunchGateway>,
        terminal: Arc<dyn ProviderAgentTerminalGateway>,
        history: Arc<dyn AgentSessionHistoryGateway>,
        hook_health: Arc<ProviderHookHealthUsecase>,
    ) -> Self {
        Self {
            sessions,
            lifecycle,
            availability,
            launch_gateway,
            terminal,
            history,
            hook_health,
            standalone_requests: Mutex::new(StandaloneLaunchRequestRegistry::default()),
            pending_workflow_launches: Mutex::new(HashMap::new()),
            activated_workflow_launches: Arc::new(Mutex::new(HashMap::new())),
            hook_health_tasks: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub(crate) async fn launch_standalone_idempotent(
        self: Arc<Self>,
        request: AgentSessionLaunchRequest,
    ) -> StandaloneLaunchOutcome {
        let caller_request_id = request.caller_request_id.clone();
        let shared = {
            let mut registry = self.standalone_requests.lock().await;
            if let Some(outcome) = registry.recall_completed(&caller_request_id) {
                return outcome;
            }
            if let Some(shared) = registry.in_flight.get(&caller_request_id) {
                shared.clone()
            } else {
                let usecase = Arc::clone(&self);
                let request_id = caller_request_id.clone();
                let task = tokio::spawn(async move {
                    let outcome =
                        match std::panic::AssertUnwindSafe(usecase.launch_standalone(request))
                            .catch_unwind()
                            .await
                        {
                            Ok(result) => result.map(|created| created.session().id().to_string()),
                            Err(_) => Err(AgentSessionLaunchUsecaseError::Corrupt),
                        };
                    usecase
                        .standalone_requests
                        .lock()
                        .await
                        .record_completed(request_id, outcome.clone());
                    outcome
                });
                let shared = async move {
                    task.await
                        .unwrap_or(Err(AgentSessionLaunchUsecaseError::Corrupt))
                }
                .boxed()
                .shared();
                registry.in_flight.insert(caller_request_id, shared.clone());
                shared
            }
        };
        shared.await
    }

    pub(crate) async fn launch_standalone(
        &self,
        request: AgentSessionLaunchRequest,
    ) -> Result<VersionedAgentSession, AgentSessionLaunchUsecaseError> {
        let agent_session_id = issue_agent_session_id(&request.caller_request_id)?;
        let pending = self
            .prepare_new_session(agent_session_id, request, None, ProviderSessionLaunch::New)
            .await?;
        self.spawn_prepared(pending).await
    }

    pub(crate) async fn confirm_workflow_node_attachment(
        &self,
        agent_session_id: &str,
    ) -> Result<(), AgentSessionLaunchUsecaseError> {
        loop {
            let activation = {
                let mut launches = self.activated_workflow_launches.lock().await;
                match launches.get(agent_session_id).cloned() {
                    Some(WorkflowLaunchActivation::Activated(_)) => {
                        launches.remove(agent_session_id);
                        return Ok(());
                    }
                    Some(WorkflowLaunchActivation::Activating(completion)) => completion,
                    None => return Err(AgentSessionLaunchUsecaseError::InvalidInput),
                }
            };
            if !wait_for_activation(activation).await {
                self.remove_abandoned_workflow_activation(agent_session_id)
                    .await;
                return Err(AgentSessionLaunchUsecaseError::Corrupt);
            }
        }
    }

    async fn remove_abandoned_workflow_activation(&self, agent_session_id: &str) {
        let mut launches = self.activated_workflow_launches.lock().await;
        if matches!(
            launches.get(agent_session_id),
            Some(WorkflowLaunchActivation::Activating(_))
        ) {
            launches.remove(agent_session_id);
        }
    }

    pub(crate) async fn prepare_workflow_node(
        &self,
        request: WorkflowAgentSessionLaunchRequest,
    ) -> Result<VersionedAgentSession, AgentSessionLaunchUsecaseError> {
        let tree_parent =
            AgentSessionTreeParent::new(&request.workflow_execution_id, &request.node_execution_id)
                .map(Some)
                .map_err(|_| AgentSessionLaunchUsecaseError::InvalidInput)?;
        let agent_session_id = issue_agent_session_id(&request.caller_request_id)?;
        let launch =
            ProviderSessionLaunch::new_with_initial_instruction(request.initial_instruction)
                .map_err(|_| AgentSessionLaunchUsecaseError::InvalidInput)?
                .with_options(crate::domain::agent_session::ProviderLaunchOptions::new(
                    request.model,
                    request.permission,
                ));
        let pending = self
            .prepare_new_session(
                agent_session_id.clone(),
                AgentSessionLaunchRequest {
                    workspace: request.workspace,
                    worktree_path: request.worktree_path,
                    provider: request.provider,
                    rows: request.rows,
                    cols: request.cols,
                    caller_request_id: request.caller_request_id,
                },
                tree_parent,
                launch,
            )
            .await?;
        let created = pending.durable.created.clone();
        if self
            .pending_workflow_launches
            .lock()
            .await
            .insert(agent_session_id, pending)
            .is_some()
        {
            return Err(AgentSessionLaunchUsecaseError::Corrupt);
        }
        Ok(created)
    }

    pub(crate) async fn activate_workflow_node(
        &self,
        agent_session_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionLaunchUsecaseError> {
        let (completion_tx, completion_rx) = watch::channel(false);
        {
            let mut launches = self.activated_workflow_launches.lock().await;
            if launches.contains_key(agent_session_id) {
                return Err(AgentSessionLaunchUsecaseError::Corrupt);
            }
            launches.insert(
                agent_session_id.to_string(),
                WorkflowLaunchActivation::Activating(completion_rx),
            );
        }
        let pending = match self
            .pending_workflow_launches
            .lock()
            .await
            .remove(agent_session_id)
        {
            Some(pending) => pending,
            None => {
                self.activated_workflow_launches
                    .lock()
                    .await
                    .remove(agent_session_id);
                let _ = completion_tx.send(true);
                return Err(AgentSessionLaunchUsecaseError::InvalidInput);
            }
        };
        let activated = match self.spawn_prepared(pending).await {
            Ok(activated) => activated,
            Err(error) => {
                self.activated_workflow_launches
                    .lock()
                    .await
                    .remove(agent_session_id);
                let _ = completion_tx.send(true);
                return Err(error);
            }
        };
        self.activated_workflow_launches.lock().await.insert(
            agent_session_id.to_string(),
            WorkflowLaunchActivation::Activated(activated.clone()),
        );
        let _ = completion_tx.send(true);
        let launches = Arc::clone(&self.activated_workflow_launches);
        let agent_session_id = agent_session_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(ACTIVATED_WORKFLOW_LAUNCH_RETENTION).await;
            launches.lock().await.remove(&agent_session_id);
        });
        Ok(activated)
    }

    async fn prepare_new_session(
        &self,
        agent_session_id: String,
        request: AgentSessionLaunchRequest,
        tree_parent: Option<AgentSessionTreeParent>,
        launch: ProviderSessionLaunch,
    ) -> Result<PreparedAgentSessionLaunch, AgentSessionLaunchUsecaseError> {
        let availability_and_lock = crate::other::telemetry::start_terminal_launch_phase(
            crate::other::telemetry::TerminalLaunch::AvailabilityAndLock,
        );
        let executable = self
            .availability
            .resolved_executable(request.provider)
            .ok_or(AgentSessionLaunchUsecaseError::ProviderUnavailable)?;
        let operation = self
            .sessions
            .lock_operation(&agent_session_id)
            .await
            .map_err(map_session_error)?;
        availability_and_lock.finish();
        let slot_id = issue_lifecycle_slot_id(&request.caller_request_id)?;
        let scope = ProviderLifecycleScope::new(&agent_session_id)
            .map_err(|_| AgentSessionLaunchUsecaseError::Corrupt)?;
        let durable_create = crate::other::telemetry::start_terminal_launch_phase(
            crate::other::telemetry::TerminalLaunch::DurableCreateCommit,
        );
        let (armed, created) = self
            .lifecycle
            .arm_with_commit(slot_id, request.provider, scope, |lifecycle_events| async {
                self.sessions
                    .create_with_lifecycle_events(
                        AgentSessionCreateRequest {
                            agent_session_id: agent_session_id.clone(),
                            workspace: request.workspace.clone(),
                            worktree_path: request.worktree_path.clone(),
                            provider: request.provider,
                            tree_parent,
                            admit_initial_instruction: launch.initial_instruction().is_some(),
                        },
                        lifecycle_events,
                        &request.caller_request_id,
                    )
                    .await
                    .map_err(map_session_error)
            })
            .await?;
        durable_create.finish();
        let durable = DurableAgentSessionLaunch {
            operation,
            created,
            armed,
            executable,
        };
        self.prepare_newly_created(
            durable,
            request.rows,
            request.cols,
            &request.caller_request_id,
            launch,
        )
        .await
    }

    pub(crate) async fn rollback_workflow_node(
        &self,
        agent_session_id: &str,
        _caller_request_id: &str,
    ) -> Result<(), AgentSessionLaunchUsecaseError> {
        let pending = self
            .pending_workflow_launches
            .lock()
            .await
            .remove(agent_session_id);
        let activated = loop {
            let activation = {
                let mut launches = self.activated_workflow_launches.lock().await;
                match launches.get(agent_session_id).cloned() {
                    Some(WorkflowLaunchActivation::Activated(session)) => {
                        launches.remove(agent_session_id);
                        break Some(session);
                    }
                    Some(WorkflowLaunchActivation::Activating(completion)) if pending.is_none() => {
                        Some(completion)
                    }
                    Some(WorkflowLaunchActivation::Activating(_)) | None => {
                        launches.remove(agent_session_id);
                        break None;
                    }
                }
            };
            if let Some(activation) = activation {
                if !wait_for_activation(activation).await {
                    self.remove_abandoned_workflow_activation(agent_session_id)
                        .await;
                    break None;
                }
            }
        };
        let session = match (pending, activated) {
            (Some(pending), _) => pending.durable.created.clone(),
            (None, Some(activated)) => activated,
            (None, None) => self
                .sessions
                .find(agent_session_id)
                .await
                .map_err(map_session_error)?
                .ok_or(AgentSessionLaunchUsecaseError::InvalidInput)?,
        };
        session
            .session()
            .authorize_workflow_launch_rollback()
            .map_err(|_| AgentSessionLaunchUsecaseError::InvalidInput)?;

        let terminal_result = self
            .terminal
            .delete(&session.session().terminal_surface_owner())
            .map_err(|_| AgentSessionLaunchUsecaseError::TerminalUnavailable);
        let lifecycle_result = ProviderLifecycleScope::new(agent_session_id)
            .map_err(|_| AgentSessionLaunchUsecaseError::Corrupt);
        let lifecycle_result = match lifecycle_result {
            Ok(scope) => self
                .lifecycle
                .release_scope(&scope)
                .await
                .map(|_| ())
                .map_err(map_lifecycle_error),
            Err(error) => Err(error),
        };
        let launch_result = self
            .launch_gateway
            .cleanup(agent_session_id)
            .map_err(map_launch_error);
        terminal_result?;
        lifecycle_result?;
        launch_result?;
        Ok(())
    }

    async fn prepare_newly_created(
        &self,
        durable: DurableAgentSessionLaunch,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
        launch: ProviderSessionLaunch,
    ) -> Result<PreparedAgentSessionLaunch, AgentSessionLaunchUsecaseError> {
        let launch_file_materialize = crate::other::telemetry::start_terminal_launch_phase(
            crate::other::telemetry::TerminalLaunch::LaunchFileMaterialize,
        );
        let prepared = match self
            .launch_gateway
            .prepare(
                &durable.armed,
                durable.executable.clone(),
                launch,
                durable.created.session().worktree_path(),
            )
            .map_err(map_launch_error)
        {
            Ok(prepared) => prepared,
            Err(error) => {
                self.rollback_failed_new_launch_preserving_cause(
                    &durable.created,
                    &durable.armed,
                    caller_request_id,
                )
                .await;
                return Err(error);
            }
        };
        launch_file_materialize.finish();
        Ok(PreparedAgentSessionLaunch {
            durable,
            prepared,
            rows,
            cols,
            caller_request_id: caller_request_id.to_string(),
        })
    }

    async fn spawn_prepared(
        &self,
        pending: PreparedAgentSessionLaunch,
    ) -> Result<VersionedAgentSession, AgentSessionLaunchUsecaseError> {
        let PreparedAgentSessionLaunch {
            durable,
            prepared,
            rows,
            cols,
            caller_request_id,
        } = pending;
        let DurableAgentSessionLaunch {
            operation: _operation,
            created,
            armed,
            executable: _,
        } = durable;
        let initial_hook_warning = prepared.initial_hook_warning();
        if let Err(error) = self.terminal.spawn(
            created.session().terminal_surface_owner(),
            created.session().worktree_path(),
            prepared.into_process(),
            rows,
            cols,
        ) {
            record_terminal_spawn_failure(created.session().id(), &error);
            self.rollback_failed_new_launch_preserving_cause(&created, &armed, &caller_request_id)
                .await;
            return Err(AgentSessionLaunchUsecaseError::TerminalSpawn(error));
        }
        self.record_hook_launch(
            created.session().provider(),
            armed.slot_id().as_str(),
            initial_hook_warning,
            &caller_request_id,
        );
        Ok(created)
    }

    pub(crate) async fn resume_history(
        &self,
        request: AgentSessionHistoryResumeRequest,
    ) -> Result<AgentSessionHistoryResumeOutcome, AgentSessionLaunchUsecaseError> {
        if request.provider_session_id.trim().is_empty() {
            return Err(AgentSessionLaunchUsecaseError::InvalidInput);
        }
        let agent_session_id = issue_agent_session_id(&request.caller_request_id)?;
        let candidates = self
            .history
            .list_metadata(request.provider, &request.worktree_path, 201)
            .await
            .map_err(|error| match error {
                AgentSessionHistoryGatewayError::InvalidRequest => {
                    AgentSessionLaunchUsecaseError::InvalidInput
                }
                AgentSessionHistoryGatewayError::Unavailable => {
                    AgentSessionLaunchUsecaseError::StorageUnavailable
                }
                AgentSessionHistoryGatewayError::Corrupt => AgentSessionLaunchUsecaseError::Corrupt,
            })?;
        if !candidates.iter().any(|candidate| {
            candidate.provider_session_id == request.provider_session_id
                && candidate.worktree_path == request.worktree_path
        }) {
            return Err(AgentSessionLaunchUsecaseError::InvalidInput);
        }
        let executable = self
            .availability
            .resolved_executable(request.provider)
            .ok_or(AgentSessionLaunchUsecaseError::ProviderUnavailable)?;
        let _operation = self
            .sessions
            .lock_operation(&agent_session_id)
            .await
            .map_err(map_session_error)?;
        self.sessions
            .create(
                &agent_session_id,
                request.workspace.clone(),
                &request.worktree_path,
                request.provider,
                None,
                &format!("{}.create", request.caller_request_id),
            )
            .await
            .map_err(map_session_error)?;
        let associated = self
            .sessions
            .associate_provider_session(
                &agent_session_id,
                &request.provider_session_id,
                None,
                &format!("{}.associate", request.caller_request_id),
            )
            .await
            .map_err(map_session_error)?;
        let slot_id = issue_lifecycle_slot_id(&request.caller_request_id)?;
        let scope = ProviderLifecycleScope::new(associated.session().id())
            .map_err(|_| AgentSessionLaunchUsecaseError::Corrupt)?;
        let armed = match self
            .lifecycle
            .arm(slot_id, associated.session().provider(), scope)
            .await
            .map_err(map_lifecycle_error)
        {
            Ok(armed) => armed,
            Err(_) => return self.pause_history_resume(&request, &agent_session_id).await,
        };
        let prepared = match self.launch_gateway.prepare(
            &armed,
            executable,
            ProviderSessionLaunch::resume(&request.provider_session_id)
                .map_err(|_| AgentSessionLaunchUsecaseError::Corrupt)?,
            associated.session().worktree_path(),
        ) {
            Ok(prepared) => prepared,
            Err(_) => {
                self.cleanup_failed_history_resume(&request, &agent_session_id, &armed)
                    .await?;
                return self.pause_history_resume(&request, &agent_session_id).await;
            }
        };
        let initial_hook_warning = prepared.initial_hook_warning();
        if let Err(error) = self.terminal.spawn(
            associated.session().terminal_surface_owner(),
            associated.session().worktree_path(),
            prepared.into_process(),
            request.rows,
            request.cols,
        ) {
            record_terminal_spawn_failure(&agent_session_id, &error);
            self.cleanup_failed_history_resume(&request, &agent_session_id, &armed)
                .await?;
            return self.pause_history_resume(&request, &agent_session_id).await;
        }
        self.record_hook_launch(
            associated.session().provider(),
            armed.slot_id().as_str(),
            initial_hook_warning,
            &request.caller_request_id,
        );
        Ok(AgentSessionHistoryResumeOutcome::Open(associated))
    }

    async fn cleanup_failed_history_resume(
        &self,
        request: &AgentSessionHistoryResumeRequest,
        agent_session_id: &str,
        armed: &ArmedProviderLifecycle,
    ) -> Result<(), AgentSessionLaunchUsecaseError> {
        let terminal_result =
            TerminalSurfaceOwner::session(request.workspace.clone(), agent_session_id)
                .map_err(|_| AgentSessionLaunchUsecaseError::TerminalUnavailable)
                .and_then(|terminal_owner| {
                    self.terminal
                        .stop_preserving_checkpoint(&terminal_owner)
                        .map_err(|_| AgentSessionLaunchUsecaseError::TerminalUnavailable)
                });
        let lifecycle_result = self
            .lifecycle
            .release(armed.slot_id(), armed.binding_id())
            .await
            .map(|_| ())
            .map_err(map_lifecycle_error);
        let launch_result = self
            .launch_gateway
            .cleanup(agent_session_id)
            .map_err(map_launch_error);

        terminal_result?;
        lifecycle_result?;
        launch_result?;
        Ok(())
    }

    async fn pause_history_resume(
        &self,
        request: &AgentSessionHistoryResumeRequest,
        agent_session_id: &str,
    ) -> Result<AgentSessionHistoryResumeOutcome, AgentSessionLaunchUsecaseError> {
        self.sessions
            .observe_process_exit(
                agent_session_id,
                None,
                &format!("{}.paused", request.caller_request_id),
            )
            .await
            .map_err(map_session_error)?;
        let session = self
            .sessions
            .find(agent_session_id)
            .await
            .map_err(map_session_error)?
            .ok_or(AgentSessionLaunchUsecaseError::Corrupt)?;
        Ok(AgentSessionHistoryResumeOutcome::Paused(session))
    }

    async fn rollback_failed_new_launch_preserving_cause(
        &self,
        created: &VersionedAgentSession,
        armed: &ArmedProviderLifecycle,
        caller_request_id: &str,
    ) {
        if let Err(rollback_error) = self
            .rollback_failed_new_launch(created, armed, caller_request_id)
            .await
        {
            log::warn!(
                "failed to roll back AgentSession launch resources without masking the launch failure: {rollback_error:?}"
            );
        }
    }

    async fn rollback_failed_new_launch(
        &self,
        created: &VersionedAgentSession,
        armed: &ArmedProviderLifecycle,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionLaunchUsecaseError> {
        let terminal_result = self
            .terminal
            .delete(&created.session().terminal_surface_owner())
            .map_err(|_| AgentSessionLaunchUsecaseError::TerminalUnavailable);
        let lifecycle_result = self
            .lifecycle
            .release(armed.slot_id(), armed.binding_id())
            .await
            .map(|_| ())
            .map_err(map_lifecycle_error);
        let launch_result = self
            .launch_gateway
            .cleanup(created.session().id())
            .map_err(map_launch_error);
        let session_result = self
            .sessions
            .garbage_collect(
                created.session().id(),
                ManagedPtyPresence::ConfirmedAbsent,
                &format!("{caller_request_id}.rollback"),
            )
            .await
            .map_err(map_session_error);
        terminal_result?;
        lifecycle_result?;
        launch_result?;
        session_result?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn standalone_in_flight_request_count(&self) -> usize {
        self.standalone_requests.lock().await.in_flight.len()
    }

    pub(crate) async fn wait_for_background_tasks(&self) -> Result<(), tokio::task::JoinError> {
        let tasks = {
            let mut tasks = self
                .hook_health_tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *tasks)
        };
        for task in tasks {
            task.await?;
        }
        Ok(())
    }

    fn record_hook_launch(
        &self,
        provider: ProviderKind,
        launch_id: &str,
        warning: Option<crate::domain::provider_lifecycle::ProviderLifecycleUnavailableReason>,
        caller_request_id: &str,
    ) {
        let hook_health = Arc::clone(&self.hook_health);
        let launch_id = launch_id.to_string();
        let caller_request_id = format!("{caller_request_id}.hook-launch");
        let task = tokio::spawn(async move {
            if let Err(error) = hook_health
                .record_launch_with_warning(provider, &launch_id, warning, &caller_request_id)
                .await
            {
                log::warn!(
                    "failed to persist Provider Hook launch health without blocking AgentSession launch: {error:?}"
                );
            }
        });
        let mut tasks = self
            .hook_health_tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
    }
}

fn record_terminal_spawn_failure(agent_session_id: &str, error: &ProviderAgentTerminalSpawnError) {
    log::error!("AgentSession terminal spawn failed agent_session_id={agent_session_id} {error}");
}

async fn wait_for_activation(mut completion: watch::Receiver<bool>) -> bool {
    if *completion.borrow() {
        return true;
    }
    completion.changed().await.is_ok() && *completion.borrow()
}

fn map_session_error(error: AgentSessionUsecaseError) -> AgentSessionLaunchUsecaseError {
    match error {
        AgentSessionUsecaseError::NotFound | AgentSessionUsecaseError::InvalidOperation => {
            AgentSessionLaunchUsecaseError::InvalidInput
        }
        AgentSessionUsecaseError::Conflict
        | AgentSessionUsecaseError::ProviderSessionAlreadyOwned { .. } => {
            AgentSessionLaunchUsecaseError::Conflict
        }
        AgentSessionUsecaseError::Unavailable => AgentSessionLaunchUsecaseError::StorageUnavailable,
        AgentSessionUsecaseError::Corrupt => AgentSessionLaunchUsecaseError::Corrupt,
    }
}

fn map_lifecycle_error(error: ProviderLifecycleUsecaseError) -> AgentSessionLaunchUsecaseError {
    match error {
        ProviderLifecycleUsecaseError::InvalidInput => AgentSessionLaunchUsecaseError::InvalidInput,
        ProviderLifecycleUsecaseError::StorageUnavailable => {
            AgentSessionLaunchUsecaseError::StorageUnavailable
        }
        ProviderLifecycleUsecaseError::Corrupt => AgentSessionLaunchUsecaseError::Corrupt,
    }
}

fn map_launch_error(error: ProviderAgentLaunchGatewayError) -> AgentSessionLaunchUsecaseError {
    match error {
        ProviderAgentLaunchGatewayError::InvalidInput => {
            AgentSessionLaunchUsecaseError::InvalidInput
        }
        ProviderAgentLaunchGatewayError::Unavailable => {
            AgentSessionLaunchUsecaseError::LaunchUnavailable
        }
    }
}

#[cfg(test)]
#[path = "agent_session_launch_test.rs"]
mod agent_session_launch_tests;
