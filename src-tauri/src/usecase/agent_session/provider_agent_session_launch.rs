use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use futures_util::future::{BoxFuture, FutureExt, Shared};
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::domain::agent_session::aggregates::{
    AgentSessionOrigin, ManagedPtyPresence, ResolvedProviderExecutable,
};
use crate::domain::agent_session::repository::VersionedProviderAgentSession;
use crate::domain::agent_session::{
    ProviderAgentLaunchGateway, ProviderAgentLaunchGatewayError,
    ProviderAgentSessionHistoryGateway, ProviderAgentSessionHistoryGatewayError,
    ProviderAgentTerminalGateway, ProviderAvailabilityReader, ProviderSessionLaunch,
};
use crate::domain::provider_lifecycle::{
    ArmedProviderLifecycle, ProviderKind, ProviderLifecycleScope, ProviderLifecycleSlotId,
};
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::provider_lifecycle::{
    ProviderHookHealthUsecase, ProviderLifecycleUsecase, ProviderLifecycleUsecaseError,
};

use super::{
    ProviderAgentSessionCreateRequest, ProviderAgentSessionUsecase,
    ProviderAgentSessionUsecaseError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderAgentSessionLaunchRequest {
    pub(crate) workspace: WorkspaceIdentity,
    pub(crate) worktree_path: String,
    pub(crate) provider: ProviderKind,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) caller_request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderAgentWorkflowSessionLaunchRequest {
    pub(crate) workspace: WorkspaceIdentity,
    pub(crate) worktree_path: String,
    pub(crate) provider: ProviderKind,
    pub(crate) workflow_execution_id: String,
    pub(crate) node_execution_id: String,
    pub(crate) initial_instruction: String,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) caller_request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderAgentSessionHistoryResumeRequest {
    pub(crate) workspace: WorkspaceIdentity,
    pub(crate) worktree_path: String,
    pub(crate) provider: ProviderKind,
    pub(crate) provider_session_id: String,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) caller_request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderAgentSessionHistoryResumeOutcome {
    Open(VersionedProviderAgentSession),
    Paused(VersionedProviderAgentSession),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderAgentSessionLaunchUsecaseError {
    ProviderUnavailable,
    InvalidInput,
    Conflict,
    StorageUnavailable,
    LaunchUnavailable,
    TerminalUnavailable,
    Corrupt,
}

impl From<ProviderLifecycleUsecaseError> for ProviderAgentSessionLaunchUsecaseError {
    fn from(error: ProviderLifecycleUsecaseError) -> Self {
        map_lifecycle_error(error)
    }
}

type StandaloneLaunchOutcome = Result<String, ProviderAgentSessionLaunchUsecaseError>;
type SharedStandaloneLaunch = Shared<BoxFuture<'static, StandaloneLaunchOutcome>>;

const COMPLETED_STANDALONE_LAUNCH_CAPACITY: usize = 128;

fn issue_agent_session_id(
    caller_request_id: &str,
) -> Result<String, ProviderAgentSessionLaunchUsecaseError> {
    if caller_request_id.trim().is_empty() {
        return Err(ProviderAgentSessionLaunchUsecaseError::InvalidInput);
    }
    Ok(format!(
        "provider-agent-session-{}",
        crate::other::id::unique_simple_id()
    ))
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

pub(crate) struct ProviderAgentSessionLaunchUsecase {
    sessions: Arc<ProviderAgentSessionUsecase>,
    lifecycle: Arc<ProviderLifecycleUsecase>,
    availability: Arc<dyn ProviderAvailabilityReader>,
    launch_gateway: Arc<dyn ProviderAgentLaunchGateway>,
    terminal: Arc<dyn ProviderAgentTerminalGateway>,
    history: Arc<dyn ProviderAgentSessionHistoryGateway>,
    hook_health: Arc<ProviderHookHealthUsecase>,
    standalone_requests: Mutex<StandaloneLaunchRequestRegistry>,
    pending_workflow_launches: Mutex<HashMap<String, PreparedProviderAgentLaunch>>,
}

struct PreparedProviderAgentLaunch {
    durable: DurableProviderAgentLaunch,
    prepared: crate::domain::agent_session::PreparedProviderLaunch,
    rows: u16,
    cols: u16,
    caller_request_id: String,
}

struct DurableProviderAgentLaunch {
    operation: OwnedMutexGuard<()>,
    created: VersionedProviderAgentSession,
    armed: ArmedProviderLifecycle,
    executable: ResolvedProviderExecutable,
}

impl ProviderAgentSessionLaunchUsecase {
    pub(crate) fn new(
        sessions: Arc<ProviderAgentSessionUsecase>,
        lifecycle: Arc<ProviderLifecycleUsecase>,
        availability: Arc<dyn ProviderAvailabilityReader>,
        launch_gateway: Arc<dyn ProviderAgentLaunchGateway>,
        terminal: Arc<dyn ProviderAgentTerminalGateway>,
        history: Arc<dyn ProviderAgentSessionHistoryGateway>,
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
        }
    }

    pub(crate) async fn launch_standalone_idempotent(
        self: Arc<Self>,
        request: ProviderAgentSessionLaunchRequest,
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
                            Err(_) => Err(ProviderAgentSessionLaunchUsecaseError::Corrupt),
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
                        .unwrap_or(Err(ProviderAgentSessionLaunchUsecaseError::Corrupt))
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
        request: ProviderAgentSessionLaunchRequest,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionLaunchUsecaseError> {
        let agent_session_id = issue_agent_session_id(&request.caller_request_id)?;
        let pending = self
            .prepare_new_session(
                agent_session_id,
                request,
                Ok(AgentSessionOrigin::Standalone),
                ProviderSessionLaunch::New,
            )
            .await?;
        self.spawn_prepared(pending).await
    }

    pub(crate) async fn prepare_workflow_node(
        &self,
        request: ProviderAgentWorkflowSessionLaunchRequest,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionLaunchUsecaseError> {
        let origin = AgentSessionOrigin::workflow_node(
            &request.workflow_execution_id,
            &request.node_execution_id,
        )
        .map_err(|_| ProviderAgentSessionLaunchUsecaseError::InvalidInput);
        let agent_session_id = issue_agent_session_id(&request.caller_request_id)?;
        let launch =
            ProviderSessionLaunch::new_with_initial_instruction(request.initial_instruction)
                .map_err(|_| ProviderAgentSessionLaunchUsecaseError::InvalidInput)?;
        let pending = self
            .prepare_new_session(
                agent_session_id.clone(),
                ProviderAgentSessionLaunchRequest {
                    workspace: request.workspace,
                    worktree_path: request.worktree_path,
                    provider: request.provider,
                    rows: request.rows,
                    cols: request.cols,
                    caller_request_id: request.caller_request_id,
                },
                origin,
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
            return Err(ProviderAgentSessionLaunchUsecaseError::Corrupt);
        }
        Ok(created)
    }

    pub(crate) async fn activate_workflow_node(
        &self,
        agent_session_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionLaunchUsecaseError> {
        let pending = self
            .pending_workflow_launches
            .lock()
            .await
            .remove(agent_session_id)
            .ok_or(ProviderAgentSessionLaunchUsecaseError::InvalidInput)?;
        self.spawn_prepared(pending).await
    }

    async fn prepare_new_session(
        &self,
        agent_session_id: String,
        request: ProviderAgentSessionLaunchRequest,
        origin: Result<AgentSessionOrigin, ProviderAgentSessionLaunchUsecaseError>,
        launch: ProviderSessionLaunch,
    ) -> Result<PreparedProviderAgentLaunch, ProviderAgentSessionLaunchUsecaseError> {
        let availability_and_lock = crate::other::telemetry::start_terminal_launch_phase(
            crate::other::telemetry::TerminalLaunch::AvailabilityAndLock,
        );
        let executable = self
            .availability
            .resolved_executable(request.provider)
            .ok_or(ProviderAgentSessionLaunchUsecaseError::ProviderUnavailable)?;
        let operation = self
            .sessions
            .lock_operation(&agent_session_id)
            .await
            .map_err(map_session_error)?;
        availability_and_lock.finish();
        let origin = origin?;
        let slot_id = ProviderLifecycleSlotId::new(crate::other::id::unique_simple_id())
            .map_err(|_| ProviderAgentSessionLaunchUsecaseError::Corrupt)?;
        let scope = ProviderLifecycleScope::new(&agent_session_id)
            .map_err(|_| ProviderAgentSessionLaunchUsecaseError::Corrupt)?;
        let durable_create = crate::other::telemetry::start_terminal_launch_phase(
            crate::other::telemetry::TerminalLaunch::DurableCreateCommit,
        );
        let (armed, created) = self
            .lifecycle
            .arm_with_commit(slot_id, request.provider, scope, |lifecycle_events| async {
                self.sessions
                    .create_with_lifecycle_events(
                        ProviderAgentSessionCreateRequest {
                            agent_session_id: agent_session_id.clone(),
                            workspace: request.workspace.clone(),
                            worktree_path: request.worktree_path.clone(),
                            provider: request.provider,
                            origin,
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
        let durable = DurableProviderAgentLaunch {
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
        caller_request_id: &str,
    ) -> Result<(), ProviderAgentSessionLaunchUsecaseError> {
        let _pending = self
            .pending_workflow_launches
            .lock()
            .await
            .remove(agent_session_id)
            .ok_or(ProviderAgentSessionLaunchUsecaseError::InvalidInput)?;
        let session = self
            .sessions
            .find(agent_session_id)
            .await
            .map_err(map_session_error)?
            .ok_or(ProviderAgentSessionLaunchUsecaseError::InvalidInput)?;
        session
            .session()
            .authorize_workflow_launch_rollback()
            .map_err(|_| ProviderAgentSessionLaunchUsecaseError::InvalidInput)?;

        let terminal_result = self
            .terminal
            .delete(&session.session().terminal_surface_owner())
            .map_err(|_| ProviderAgentSessionLaunchUsecaseError::TerminalUnavailable);
        let lifecycle_result = ProviderLifecycleScope::new(agent_session_id)
            .map_err(|_| ProviderAgentSessionLaunchUsecaseError::Corrupt);
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
        let session_result = self
            .sessions
            .rollback_workflow_launch(agent_session_id, caller_request_id)
            .await
            .map_err(map_session_error);

        terminal_result?;
        lifecycle_result?;
        launch_result?;
        session_result?;
        Ok(())
    }

    async fn prepare_newly_created(
        &self,
        durable: DurableProviderAgentLaunch,
        rows: u16,
        cols: u16,
        caller_request_id: &str,
        launch: ProviderSessionLaunch,
    ) -> Result<PreparedProviderAgentLaunch, ProviderAgentSessionLaunchUsecaseError> {
        let launch_file_materialize = crate::other::telemetry::start_terminal_launch_phase(
            crate::other::telemetry::TerminalLaunch::LaunchFileMaterialize,
        );
        let prepared = match self
            .launch_gateway
            .prepare(&durable.armed, durable.executable.clone(), launch)
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
        Ok(PreparedProviderAgentLaunch {
            durable,
            prepared,
            rows,
            cols,
            caller_request_id: caller_request_id.to_string(),
        })
    }

    async fn spawn_prepared(
        &self,
        pending: PreparedProviderAgentLaunch,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionLaunchUsecaseError> {
        let PreparedProviderAgentLaunch {
            durable,
            prepared,
            rows,
            cols,
            caller_request_id,
        } = pending;
        let DurableProviderAgentLaunch {
            operation: _operation,
            created,
            armed,
            executable: _,
        } = durable;
        let initial_hook_warning = prepared.initial_hook_warning();
        if self
            .terminal
            .spawn(
                created.session().terminal_surface_owner(),
                created.session().worktree_path(),
                prepared.into_process(),
                rows,
                cols,
            )
            .is_err()
        {
            self.rollback_failed_new_launch_preserving_cause(&created, &armed, &caller_request_id)
                .await;
            return Err(ProviderAgentSessionLaunchUsecaseError::TerminalUnavailable);
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
        request: ProviderAgentSessionHistoryResumeRequest,
    ) -> Result<ProviderAgentSessionHistoryResumeOutcome, ProviderAgentSessionLaunchUsecaseError>
    {
        if request.provider_session_id.trim().is_empty() {
            return Err(ProviderAgentSessionLaunchUsecaseError::InvalidInput);
        }
        let agent_session_id = issue_agent_session_id(&request.caller_request_id)?;
        let candidates = self
            .history
            .list_metadata(request.provider, &request.worktree_path, 201)
            .await
            .map_err(|error| match error {
                ProviderAgentSessionHistoryGatewayError::InvalidRequest => {
                    ProviderAgentSessionLaunchUsecaseError::InvalidInput
                }
                ProviderAgentSessionHistoryGatewayError::Unavailable => {
                    ProviderAgentSessionLaunchUsecaseError::StorageUnavailable
                }
                ProviderAgentSessionHistoryGatewayError::Corrupt => {
                    ProviderAgentSessionLaunchUsecaseError::Corrupt
                }
            })?;
        if !candidates.iter().any(|candidate| {
            candidate.provider_session_id == request.provider_session_id
                && candidate.worktree_path == request.worktree_path
        }) {
            return Err(ProviderAgentSessionLaunchUsecaseError::InvalidInput);
        }
        let executable = self
            .availability
            .resolved_executable(request.provider)
            .ok_or(ProviderAgentSessionLaunchUsecaseError::ProviderUnavailable)?;
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
                AgentSessionOrigin::Standalone,
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
        let slot_id = ProviderLifecycleSlotId::new(crate::other::id::unique_simple_id())
            .map_err(|_| ProviderAgentSessionLaunchUsecaseError::Corrupt)?;
        let scope = ProviderLifecycleScope::new(associated.session().id())
            .map_err(|_| ProviderAgentSessionLaunchUsecaseError::Corrupt)?;
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
                .map_err(|_| ProviderAgentSessionLaunchUsecaseError::Corrupt)?,
        ) {
            Ok(prepared) => prepared,
            Err(_) => {
                self.cleanup_failed_history_resume(&request, &agent_session_id, &armed)
                    .await?;
                return self.pause_history_resume(&request, &agent_session_id).await;
            }
        };
        let initial_hook_warning = prepared.initial_hook_warning();
        if self
            .terminal
            .spawn(
                associated.session().terminal_surface_owner(),
                associated.session().worktree_path(),
                prepared.into_process(),
                request.rows,
                request.cols,
            )
            .is_err()
        {
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
        Ok(ProviderAgentSessionHistoryResumeOutcome::Open(associated))
    }

    async fn cleanup_failed_history_resume(
        &self,
        request: &ProviderAgentSessionHistoryResumeRequest,
        agent_session_id: &str,
        armed: &ArmedProviderLifecycle,
    ) -> Result<(), ProviderAgentSessionLaunchUsecaseError> {
        let terminal_result =
            TerminalSurfaceOwner::session(request.workspace.clone(), agent_session_id)
                .map_err(|_| ProviderAgentSessionLaunchUsecaseError::TerminalUnavailable)
                .and_then(|terminal_owner| {
                    self.terminal
                        .stop_preserving_checkpoint(&terminal_owner)
                        .map_err(|_| ProviderAgentSessionLaunchUsecaseError::TerminalUnavailable)
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
        request: &ProviderAgentSessionHistoryResumeRequest,
        agent_session_id: &str,
    ) -> Result<ProviderAgentSessionHistoryResumeOutcome, ProviderAgentSessionLaunchUsecaseError>
    {
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
            .ok_or(ProviderAgentSessionLaunchUsecaseError::Corrupt)?;
        Ok(ProviderAgentSessionHistoryResumeOutcome::Paused(session))
    }

    async fn rollback_failed_new_launch_preserving_cause(
        &self,
        created: &VersionedProviderAgentSession,
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
        created: &VersionedProviderAgentSession,
        armed: &ArmedProviderLifecycle,
        caller_request_id: &str,
    ) -> Result<(), ProviderAgentSessionLaunchUsecaseError> {
        let terminal_result = self
            .terminal
            .delete(&created.session().terminal_surface_owner())
            .map_err(|_| ProviderAgentSessionLaunchUsecaseError::TerminalUnavailable);
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
        let _hook_health_task = tokio::spawn(async move {
            if let Err(error) = hook_health
                .record_launch_with_warning(provider, &launch_id, warning, &caller_request_id)
                .await
            {
                log::warn!(
                    "failed to persist Provider Hook launch health without blocking AgentSession launch: {error:?}"
                );
            }
        });
    }
}

fn map_session_error(
    error: ProviderAgentSessionUsecaseError,
) -> ProviderAgentSessionLaunchUsecaseError {
    match error {
        ProviderAgentSessionUsecaseError::NotFound
        | ProviderAgentSessionUsecaseError::InvalidOperation => {
            ProviderAgentSessionLaunchUsecaseError::InvalidInput
        }
        ProviderAgentSessionUsecaseError::Conflict
        | ProviderAgentSessionUsecaseError::ProviderSessionAlreadyOwned { .. } => {
            ProviderAgentSessionLaunchUsecaseError::Conflict
        }
        ProviderAgentSessionUsecaseError::Unavailable => {
            ProviderAgentSessionLaunchUsecaseError::StorageUnavailable
        }
        ProviderAgentSessionUsecaseError::Corrupt => {
            ProviderAgentSessionLaunchUsecaseError::Corrupt
        }
    }
}

fn map_lifecycle_error(
    error: ProviderLifecycleUsecaseError,
) -> ProviderAgentSessionLaunchUsecaseError {
    match error {
        ProviderLifecycleUsecaseError::InvalidInput => {
            ProviderAgentSessionLaunchUsecaseError::InvalidInput
        }
        ProviderLifecycleUsecaseError::StorageUnavailable => {
            ProviderAgentSessionLaunchUsecaseError::StorageUnavailable
        }
        ProviderLifecycleUsecaseError::Corrupt => ProviderAgentSessionLaunchUsecaseError::Corrupt,
    }
}

fn map_launch_error(
    error: ProviderAgentLaunchGatewayError,
) -> ProviderAgentSessionLaunchUsecaseError {
    match error {
        ProviderAgentLaunchGatewayError::InvalidInput => {
            ProviderAgentSessionLaunchUsecaseError::InvalidInput
        }
        ProviderAgentLaunchGatewayError::Unavailable => {
            ProviderAgentSessionLaunchUsecaseError::LaunchUnavailable
        }
    }
}

#[cfg(test)]
#[path = "provider_agent_session_launch_test.rs"]
mod provider_agent_session_launch_tests;
