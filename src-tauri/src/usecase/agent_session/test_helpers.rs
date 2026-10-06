use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::domain::agent_session::aggregates::{
    ProviderAvailability, ProviderExecutable, ProviderUnavailableReason, ResolvedProviderExecutable,
};
use crate::domain::agent_session::{
    ProviderExecutableConfigRepository, ProviderExecutableConfigRepositoryError,
    ProviderExecutableProbeGateway, ProviderExecutableProbeGatewayError,
};
use crate::domain::provider_lifecycle::ProviderKind;

#[derive(Default)]
pub struct FakeProviderExecutableConfigRepository {
    overrides: Mutex<HashMap<ProviderKind, ProviderExecutable>>,
    fail_save: AtomicBool,
}

impl FakeProviderExecutableConfigRepository {
    #[cfg(test)]
    pub(crate) fn with_override(provider: ProviderKind, executable: &str) -> Self {
        Self {
            overrides: Mutex::new(HashMap::from([(
                provider,
                ProviderExecutable::new(executable).unwrap(),
            )])),
            fail_save: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_save(&self) {
        self.fail_save.store(true, Ordering::SeqCst);
    }
}

impl ProviderExecutableConfigRepository for FakeProviderExecutableConfigRepository {
    fn configured_executable(
        &self,
        provider: ProviderKind,
    ) -> Result<Option<ProviderExecutable>, ProviderExecutableConfigRepositoryError> {
        Ok(self.overrides.lock().unwrap().get(&provider).cloned())
    }

    fn save_configured_executable(
        &self,
        provider: ProviderKind,
        executable: Option<&ProviderExecutable>,
    ) -> Result<(), ProviderExecutableConfigRepositoryError> {
        if self.fail_save.load(Ordering::SeqCst) {
            return Err(ProviderExecutableConfigRepositoryError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Transient,
                    message: "unavailable".into(),
                },
            ));
        }
        let mut overrides = self.overrides.lock().unwrap();
        match executable {
            Some(executable) => {
                overrides.insert(provider, executable.clone());
            }
            None => {
                overrides.remove(&provider);
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct FakeProviderExecutableProbeGateway {
    force_missing: AtomicBool,
    pub(crate) refreshes: Mutex<usize>,
}

impl FakeProviderExecutableProbeGateway {
    #[cfg(test)]
    pub(crate) fn set_force_missing(&self, force_missing: bool) {
        self.force_missing.store(force_missing, Ordering::SeqCst);
    }
}

impl ProviderExecutableProbeGateway for FakeProviderExecutableProbeGateway {
    fn resolve(&self, executable: &ProviderExecutable) -> ProviderAvailability {
        if self.force_missing.load(Ordering::SeqCst) || executable.as_str().contains("missing") {
            ProviderAvailability::unavailable(ProviderUnavailableReason::NotFound)
        } else {
            let resolved = if executable.as_str().starts_with('/') {
                executable.as_str().into()
            } else {
                format!("/resolved/{}", executable.as_str()).into()
            };
            ProviderAvailability::available(ResolvedProviderExecutable::new(resolved).unwrap())
        }
    }

    fn refresh_search_path(&self) -> Result<(), ProviderExecutableProbeGatewayError> {
        *self.refreshes.lock().unwrap() += 1;
        Ok(())
    }
}

use crate::test_support::captured_error_messages;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::sync::Arc;

use std::time::Duration;

#[cfg(test)]
use crate::domain::agent_session::aggregates::agent_session::AgentSession;
use crate::domain::agent_session::aggregates::agent_session::ManagedPtyPresence;
use crate::domain::agent_session::provider_availability_gateway::ProviderAvailabilityReader;
use crate::domain::agent_session::provider_history_gateway::AgentSessionHistoryGateway;
use crate::domain::agent_session::provider_history_gateway::AgentSessionHistoryGatewayError;
use crate::domain::agent_session::provider_history_gateway::AgentSessionHistoryMetadata;
use crate::domain::agent_session::provider_history_gateway::ProviderSessionTitleEntry;
use crate::domain::agent_session::provider_launch::ProviderSessionLaunch;
use crate::domain::agent_session::provider_launch_gateway::PreparedProviderLaunch;
use crate::domain::agent_session::provider_launch_gateway::ProviderAgentLaunchGateway;
use crate::domain::agent_session::provider_launch_gateway::ProviderAgentLaunchGatewayError;
use crate::domain::agent_session::provider_terminal_gateway::ProviderAgentTerminalGateway;
use crate::domain::agent_session::provider_terminal_gateway::ProviderAgentTerminalGatewayError;
#[cfg(test)]
use crate::domain::agent_session::repository::AgentSessionRepository;
#[cfg(test)]
use crate::domain::agent_session::repository::AgentSessionRepositoryError;
#[cfg(test)]
use crate::domain::agent_session::repository::VersionedAgentSession;
use crate::domain::provider_lifecycle::entities::provider_hook_health::ProviderHookHealth;
use crate::domain::provider_lifecycle::repository::ProviderHookHealthRepository;
use crate::domain::provider_lifecycle::repository::ProviderHookHealthRepositoryError;
use crate::domain::provider_lifecycle::repository::ProviderLifecycleEventRepository;
use crate::domain::provider_lifecycle::repository::ProviderLifecycleRepositoryError;
use crate::domain::provider_lifecycle::repository::VersionedProviderHookHealth;
use crate::domain::provider_lifecycle::value_objects::armed_provider_lifecycle::ArmedProviderLifecycle;
use crate::domain::provider_lifecycle::value_objects::provider_lifecycle_scope::ProviderLifecycleScope;
use crate::domain::provider_lifecycle::value_objects::provider_lifecycle_unavailable::ProviderLifecycleUnavailableReason;
use crate::domain::provider_lifecycle::value_objects::scoped_provider_lifecycle_event::ScopedProviderLifecycleEvent;
use crate::domain::terminal_surface::value_objects::terminal_process_launch::TerminalProcessLaunch;
use crate::domain::terminal_surface::value_objects::terminal_surface_owner::TerminalSurfaceOwner;
#[cfg(test)]
use crate::domain::workspace_tree::value_objects::WorkspaceIdentity;
#[cfg(test)]
use crate::usecase::agent_session::agent_session_launch::AgentSessionLaunchRequest;
#[cfg(test)]
use crate::usecase::agent_session::agent_session_launch::AgentSessionLaunchUsecase;
use crate::usecase::agent_session::agent_session_launch::ExecutionTreeCacheReleaseError;
use crate::usecase::agent_session::agent_session_launch::ProviderAgentRuntime;
use crate::usecase::agent_session::agent_session_launch::StartedExecutionTreeRegistrationError;
#[cfg(test)]
use crate::usecase::agent_session::usecase::AgentSessionUsecase;
use crate::usecase::provider_lifecycle::hook_health::ProviderHookHealthUsecase;
#[cfg(test)]
use crate::usecase::provider_lifecycle::ProviderLifecycleUsecase;

pub fn provider_runtime(
    availability: Arc<dyn ProviderAvailabilityReader>,
    launch_gateway: Arc<dyn ProviderAgentLaunchGateway>,
    terminal: Arc<dyn ProviderAgentTerminalGateway>,
) -> ProviderAgentRuntime {
    ProviderAgentRuntime::new(availability, launch_gateway, terminal)
}

#[derive(Default)]
pub struct RecordingStartedExecutionTrees {
    pub operation_lock: Arc<tokio::sync::Mutex<()>>,
    pub tree_ids: Mutex<Vec<String>>,
    pub failure: Option<StartedExecutionTreeRegistrationError>,
    pub releases: Mutex<Vec<String>>,
    pub release_failure: Mutex<Option<ExecutionTreeCacheReleaseError>>,
}

impl crate::usecase::agent_session::agent_session_launch::WorktreeMutationAdmission
    for RecordingStartedExecutionTrees
{
    fn begin_worktree_mutation(
        &self,
        path: &str,
    ) -> Result<
        crate::usecase::worktree_operation::WorktreeMutationGuard,
        crate::domain::workflow::error::WorkflowError,
    > {
        crate::usecase::worktree_operation::WorktreeOperations::default()
            .mutate(path)
            .map_err(|error| {
                crate::domain::workflow::error::WorkflowError::invalid_state(error.to_string())
            })
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::agent_session_launch::ExecutionTreeCache
    for RecordingStartedExecutionTrees
{
    async fn release_deleted_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), ExecutionTreeCacheReleaseError> {
        self.releases.lock().unwrap().push(tree_id.to_string());
        match self.release_failure.lock().unwrap().clone() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::agent_session_launch::StartedExecutionTreeRegistrar
    for RecordingStartedExecutionTrees
{
    async fn register_started_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), StartedExecutionTreeRegistrationError> {
        self.tree_ids.lock().unwrap().push(tree_id.to_string());
        if let Some(error) = self.failure.clone() {
            return Err(error);
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl crate::usecase::agent_session::agent_session_launch::AgentSessionExecutionTreeLifecycle
    for RecordingStartedExecutionTrees
{
    async fn lock_execution_tree(
        &self,
        _: &str,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, crate::domain::workflow::error::WorkflowError>
    {
        Ok(self.operation_lock.clone().lock_owned().await)
    }

    async fn archive_execution_tree(
        &self,
        _: &str,
    ) -> Result<(), crate::domain::workflow::error::WorkflowError> {
        unreachable!()
    }

    async fn restore_execution_tree(
        &self,
        _: &str,
    ) -> Result<(), crate::domain::workflow::error::WorkflowError> {
        unreachable!()
    }
}

pub fn started_execution_trees() -> Arc<RecordingStartedExecutionTrees> {
    Arc::new(RecordingStartedExecutionTrees::default())
}

#[cfg(test)]
pub struct FailingSaveRepository {
    pub stored: Mutex<Option<VersionedAgentSession>>,
    pub create_calls: AtomicUsize,
    pub atomic_create_calls: AtomicUsize,
    pub atomic_create_failure: Mutex<Option<AgentSessionRepositoryError>>,
    pub launch_lifecycle_events: Mutex<Vec<ScopedProviderLifecycleEvent>>,
    pub remove_failure: Mutex<Option<AgentSessionRepositoryError>>,
}

pub fn captured_terminal_spawn_failure(agent_session_id: &str) -> Option<String> {
    captured_error_messages()
        .iter()
        .rev()
        .find(|message| {
            message.contains("AgentSession terminal spawn failed")
                && message.contains(agent_session_id)
        })
        .cloned()
}

#[cfg(test)]
impl FailingSaveRepository {
    pub fn new(mut session: AgentSession) -> Self {
        session.take_uncommitted_events();
        Self {
            stored: Mutex::new(Some(VersionedAgentSession::restored(session, 1))),
            create_calls: AtomicUsize::new(0),
            atomic_create_calls: AtomicUsize::new(0),
            atomic_create_failure: Mutex::new(None),
            launch_lifecycle_events: Mutex::new(Vec::new()),
            remove_failure: Mutex::new(None),
        }
    }

    pub fn snapshot(&self) -> VersionedAgentSession {
        self.stored.lock().unwrap().clone().unwrap()
    }
}

#[async_trait::async_trait]
#[cfg(test)]
impl AgentSessionRepository for FailingSaveRepository {
    async fn create(
        &self,
        mut session: AgentSession,
        _caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        self.create_calls.fetch_add(1, Ordering::SeqCst);
        session.take_uncommitted_events();
        let saved = VersionedAgentSession::restored(session, 1);
        *self.stored.lock().unwrap() = Some(saved.clone());
        Ok(saved)
    }

    async fn create_with_lifecycle_events(
        &self,
        mut session: AgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        _caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        self.atomic_create_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = self.atomic_create_failure.lock().unwrap().clone() {
            return Err(error);
        }
        self.launch_lifecycle_events
            .lock()
            .unwrap()
            .extend(lifecycle_events);
        session.take_uncommitted_events();
        let saved = VersionedAgentSession::restored(session, 1);
        *self.stored.lock().unwrap() = Some(saved.clone());
        Ok(saved)
    }

    async fn find(
        &self,
        _session_id: &str,
    ) -> Result<Option<VersionedAgentSession>, AgentSessionRepositoryError> {
        Ok(self.stored.lock().unwrap().clone())
    }

    async fn save(
        &self,
        _session: VersionedAgentSession,
        _caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        Err(AgentSessionRepositoryError::Unavailable)
    }

    async fn remove(
        &self,
        _session: VersionedAgentSession,
        _authorization: crate::domain::agent_session::aggregates::agent_session::AgentSessionRemovalAuthorization,
        _caller_request_id: &str,
    ) -> Result<(), AgentSessionRepositoryError> {
        if let Some(error) = self.remove_failure.lock().unwrap().clone() {
            return Err(error);
        }
        *self.stored.lock().unwrap() = None;
        Ok(())
    }
}

pub struct FixedAvailability {
    pub available: bool,
    pub checks: Mutex<Vec<ProviderKind>>,
}

impl ProviderAvailabilityReader for FixedAvailability {
    fn is_available(&self, provider: ProviderKind) -> bool {
        self.checks.lock().unwrap().push(provider);
        self.available
    }

    fn resolved_executable(&self, provider: ProviderKind) -> Option<ResolvedProviderExecutable> {
        self.is_available(provider)
            .then(|| ResolvedProviderExecutable::new("/provider-fixture".into()).unwrap())
    }
}

#[cfg(test)]
pub struct PanicOnFirstCheckAvailability {
    pub checks: AtomicUsize,
}

#[cfg(test)]
impl ProviderAvailabilityReader for PanicOnFirstCheckAvailability {
    fn is_available(&self, _provider: ProviderKind) -> bool {
        if self.checks.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("availability check panicked for the launch panic test");
        }
        true
    }

    fn resolved_executable(&self, provider: ProviderKind) -> Option<ResolvedProviderExecutable> {
        self.is_available(provider)
            .then(|| ResolvedProviderExecutable::new("/provider-fixture".into()).unwrap())
    }
}

#[derive(Default)]
pub struct RecordingLifecycleEvents {
    pub events: Mutex<Vec<ScopedProviderLifecycleEvent>>,
}

#[derive(Default)]
pub struct FailingFirstLifecycleEvents {
    pub attempts: AtomicUsize,
}

#[async_trait::async_trait]
impl ProviderLifecycleEventRepository for FailingFirstLifecycleEvents {
    async fn append(
        &self,
        _events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleRepositoryError> {
        if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
            Err(ProviderLifecycleRepositoryError::StorageUnavailable)
        } else {
            Ok(())
        }
    }

    async fn load_scope(
        &self,
        _scope: &ProviderLifecycleScope,
    ) -> Result<Vec<ScopedProviderLifecycleEvent>, ProviderLifecycleRepositoryError> {
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl ProviderLifecycleEventRepository for RecordingLifecycleEvents {
    async fn append(
        &self,
        events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleRepositoryError> {
        self.events.lock().unwrap().extend(events);
        Ok(())
    }

    async fn load_scope(
        &self,
        scope: &ProviderLifecycleScope,
    ) -> Result<Vec<ScopedProviderLifecycleEvent>, ProviderLifecycleRepositoryError> {
        Ok(self
            .events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| {
                let (event_scope, event) = event.clone().into_parts();
                (event_scope == *scope)
                    .then(|| ScopedProviderLifecycleEvent::new(event_scope, event))
            })
            .collect())
    }
}

#[derive(Default)]
pub struct RecordingLaunchGateway {
    pub armed: Mutex<Vec<ArmedProviderLifecycle>>,
    pub launches: Mutex<Vec<ProviderSessionLaunch>>,
    pub executables: Mutex<Vec<ResolvedProviderExecutable>>,
    pub cleanups: Mutex<Vec<String>>,
    pub fail_prepare: Mutex<bool>,
}

impl ProviderAgentLaunchGateway for RecordingLaunchGateway {
    fn prepare(
        &self,
        armed: &ArmedProviderLifecycle,
        executable: ResolvedProviderExecutable,
        launch: ProviderSessionLaunch,
        _worktree_path: &str,
    ) -> Result<PreparedProviderLaunch, ProviderAgentLaunchGatewayError> {
        if *self.fail_prepare.lock().unwrap() {
            return Err(ProviderAgentLaunchGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Transient,
                    message: "unavailable".into(),
                },
            ));
        }
        self.launches.lock().unwrap().push(launch);
        self.executables.lock().unwrap().push(executable);
        self.armed.lock().unwrap().push(armed.clone());
        Ok(PreparedProviderLaunch::new(
            TerminalProcessLaunch::new(
                "/opt/bin/provider",
                vec!["--hook-config".to_string()],
                vec![(
                    "RELEASH_BINDING".to_string(),
                    armed.binding_id().to_string(),
                )],
            )
            .unwrap(),
            None,
            (armed.provider() == ProviderKind::Codex)
                .then_some(ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed),
        ))
    }

    fn cleanup(&self, agent_session_id: &str) -> Result<(), ProviderAgentLaunchGatewayError> {
        self.cleanups
            .lock()
            .unwrap()
            .push(agent_session_id.to_string());
        Ok(())
    }
}

#[derive(Default)]
pub struct MemoryHookHealthRepository {
    pub stored: Mutex<std::collections::HashMap<ProviderKind, VersionedProviderHookHealth>>,
    pub save_count: AtomicUsize,
    pub block_first_save: Option<Arc<HookHealthSaveBarrier>>,
}

#[derive(Default)]
pub struct HookHealthSaveBarrier {
    pub started: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
    pub completed: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl ProviderHookHealthRepository for MemoryHookHealthRepository {
    async fn load(
        &self,
        provider: ProviderKind,
    ) -> Result<VersionedProviderHookHealth, ProviderHookHealthRepositoryError> {
        Ok(self
            .stored
            .lock()
            .unwrap()
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| {
                VersionedProviderHookHealth::restored(ProviderHookHealth::new(provider), 0)
            }))
    }

    async fn save(
        &self,
        mut health: VersionedProviderHookHealth,
        _caller_request_id: &str,
    ) -> Result<VersionedProviderHookHealth, ProviderHookHealthRepositoryError> {
        let save_number = self.save_count.fetch_add(1, Ordering::SeqCst) + 1;
        if save_number == 1 {
            if let Some(barrier) = &self.block_first_save {
                barrier.started.notify_one();
                barrier.release.notified().await;
            }
        }
        let revision = health.revision()
            + u64::try_from(health.health_mut().take_uncommitted_events().len()).unwrap();
        let saved = VersionedProviderHookHealth::restored(health.into_health(), revision);
        self.stored
            .lock()
            .unwrap()
            .insert(saved.health().provider(), saved.clone());
        if save_number == 1 {
            if let Some(barrier) = &self.block_first_save {
                barrier.completed.notify_one();
            }
        }
        Ok(saved)
    }
}

pub fn hook_health_usecase() -> Arc<ProviderHookHealthUsecase> {
    Arc::new(ProviderHookHealthUsecase::new(Arc::new(
        MemoryHookHealthRepository::default(),
    )))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedTerminalSpawn {
    pub owner: TerminalSurfaceOwner,
    pub worktree_path: String,
    pub process: TerminalProcessLaunch,
    pub rows: u16,
    pub cols: u16,
}

#[derive(Default)]
pub struct RecordingTerminal {
    pub spawns: Mutex<Vec<RecordedTerminalSpawn>>,
    pub spawn_error: Mutex<Option<ProviderAgentTerminalGatewayError>>,
    pub fail_delete: Mutex<bool>,
    pub deletes: Mutex<usize>,
}

pub struct BlockingLaunchTerminal {
    pub presence: Mutex<ManagedPtyPresence>,
    pub spawn_entered: Mutex<Option<mpsc::Sender<()>>>,
    pub spawn_release: Mutex<Option<mpsc::Receiver<()>>>,
    pub spawns: AtomicUsize,
    pub deletes: Mutex<usize>,
}

impl ProviderAgentTerminalGateway for BlockingLaunchTerminal {
    fn spawn(
        &self,
        _owner: TerminalSurfaceOwner,
        _worktree_path: &str,
        _process: TerminalProcessLaunch,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        let entered = self.spawn_entered.lock().unwrap().take();
        if let Some(entered) = entered {
            entered.send(()).unwrap();
            self.spawn_release
                .lock()
                .unwrap()
                .take()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        }
        *self.presence.lock().unwrap() = ManagedPtyPresence::Live;
        Ok(())
    }

    fn presence(
        &self,
        _owner: &TerminalSurfaceOwner,
    ) -> Result<ManagedPtyPresence, ProviderAgentTerminalGatewayError> {
        Ok(*self.presence.lock().unwrap())
    }

    fn stop_preserving_checkpoint(
        &self,
        _owner: &TerminalSurfaceOwner,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        *self.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
        Ok(())
    }

    fn delete(
        &self,
        _owner: &TerminalSurfaceOwner,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        *self.deletes.lock().unwrap() += 1;
        *self.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
        Ok(())
    }

    fn is_current_runtime_generation(
        &self,
        _owner: &TerminalSurfaceOwner,
        _runtime_generation: u64,
    ) -> Result<bool, ProviderAgentTerminalGatewayError> {
        Ok(true)
    }
}

impl ProviderAgentTerminalGateway for RecordingTerminal {
    fn spawn(
        &self,
        owner: TerminalSurfaceOwner,
        worktree_path: &str,
        process: TerminalProcessLaunch,
        rows: u16,
        cols: u16,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        if let Some(error) = self.spawn_error.lock().unwrap().clone() {
            return Err(error);
        }
        self.spawns.lock().unwrap().push(RecordedTerminalSpawn {
            owner,
            worktree_path: worktree_path.to_string(),
            process,
            rows,
            cols,
        });
        Ok(())
    }

    fn presence(
        &self,
        _owner: &TerminalSurfaceOwner,
    ) -> Result<ManagedPtyPresence, ProviderAgentTerminalGatewayError> {
        Ok(ManagedPtyPresence::Live)
    }

    fn stop_preserving_checkpoint(
        &self,
        _owner: &TerminalSurfaceOwner,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        Ok(())
    }

    fn delete(
        &self,
        _owner: &TerminalSurfaceOwner,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        *self.deletes.lock().unwrap() += 1;
        if *self.fail_delete.lock().unwrap() {
            return Err(ProviderAgentTerminalGatewayError::Technical(
                crate::domain::failure::TechnicalFailure {
                    nature: crate::domain::failure::TechnicalFailureNature::Transient,
                    message: "unavailable".into(),
                },
            ));
        }
        Ok(())
    }

    fn is_current_runtime_generation(
        &self,
        _owner: &TerminalSurfaceOwner,
        _runtime_generation: u64,
    ) -> Result<bool, ProviderAgentTerminalGatewayError> {
        Ok(true)
    }
}

#[cfg(test)]
pub fn launch_usecase(
    repository: Arc<FailingSaveRepository>,
    availability: Arc<FixedAvailability>,
    launch_gateway: Arc<RecordingLaunchGateway>,
    terminal: Arc<RecordingTerminal>,
) -> AgentSessionLaunchUsecase {
    launch_usecase_with_hook_health(
        repository,
        availability,
        launch_gateway,
        terminal,
        hook_health_usecase(),
    )
}

#[cfg(test)]
pub fn launch_usecase_with_hook_health(
    repository: Arc<FailingSaveRepository>,
    availability: Arc<FixedAvailability>,
    launch_gateway: Arc<RecordingLaunchGateway>,
    terminal: Arc<RecordingTerminal>,
    hook_health: Arc<ProviderHookHealthUsecase>,
) -> AgentSessionLaunchUsecase {
    launch_usecase_with_tree_registrar(
        repository,
        availability,
        launch_gateway,
        terminal,
        hook_health,
        started_execution_trees(),
    )
}

#[cfg(test)]
pub fn launch_usecase_with_tree_registrar(
    repository: Arc<FailingSaveRepository>,
    availability: Arc<FixedAvailability>,
    launch_gateway: Arc<RecordingLaunchGateway>,
    terminal: Arc<RecordingTerminal>,
    hook_health: Arc<ProviderHookHealthUsecase>,
    execution_trees: Arc<
        dyn crate::usecase::agent_session::agent_session_launch::AgentSessionLaunchExecutionTrees,
    >,
) -> AgentSessionLaunchUsecase {
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(crate::usecase::test_helpers::TestCredentials),
        Arc::new(RecordingLifecycleEvents::default()),
    ));
    AgentSessionLaunchUsecase::new(
        std::sync::Arc::new(crate::usecase::test_helpers::NoopPerformance),
        Arc::new(AgentSessionUsecase::new(repository)),
        lifecycle,
        provider_runtime(availability, launch_gateway, terminal),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health,
        execution_trees,
        tokio::sync::mpsc::unbounded_channel().0,
        crate::usecase::workspace_tree::test_support::TestWorkspaceTreeRepository::new(),
    )
}

pub struct FixedHistory {
    pub entries: Vec<AgentSessionHistoryMetadata>,
}

#[async_trait::async_trait]
impl AgentSessionHistoryGateway for FixedHistory {
    async fn list_metadata(
        &self,
        provider: ProviderKind,
        worktree_path: &str,
        limit: usize,
    ) -> Result<Vec<AgentSessionHistoryMetadata>, AgentSessionHistoryGatewayError> {
        Ok(self
            .entries
            .iter()
            .filter(|entry| entry.provider == provider && entry.worktree_path == worktree_path)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn list_session_titles(
        &self,
        _provider: ProviderKind,
        _worktree_path: &str,
        provider_session_ids: &[String],
    ) -> Result<Vec<ProviderSessionTitleEntry>, AgentSessionHistoryGatewayError> {
        Ok(provider_session_ids
            .iter()
            .map(|provider_session_id| ProviderSessionTitleEntry {
                provider_session_id: provider_session_id.clone(),
                session_title: None,
                first_user_prompt: None,
            })
            .collect())
    }
}

#[cfg(test)]
pub fn idempotent_launch_request(caller_request_id: &str) -> AgentSessionLaunchRequest {
    AgentSessionLaunchRequest {
        workspace: WorkspaceIdentity::new("/repo"),
        worktree_path: "/repo/.worktrees/feature".to_string(),
        provider: ProviderKind::Claude,
        rows: 24,
        cols: 80,
        caller_request_id: caller_request_id.to_string(),
    }
}

pub use crate::domain::agent_session::test_helpers::{session_location, workflow_location};
