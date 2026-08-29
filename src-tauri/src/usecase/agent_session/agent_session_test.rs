use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, Once};
use std::time::Duration;

use super::{
    AgentSessionHistoryResumeOutcome, AgentSessionHistoryResumeRequest, AgentSessionLaunchRequest,
    AgentSessionLaunchUsecase, AgentSessionLaunchUsecaseError, AgentSessionLifecycleUsecase,
    AgentSessionUsecase, AgentSessionUsecaseError, ExecutionTreeCacheReleaseError,
    ProviderAgentRuntime, StartedExecutionTreeRegistrar, StartedExecutionTreeRegistrationError,
    WorkflowAgentSessionLaunchRequest,
};
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionTreeLocation};
use crate::domain::agent_session::aggregates::{
    AgentSessionArchiveOutcome, AgentSessionLifecycle, AgentSessionProcessExitOutcome,
    AgentSessionRecoveryResult, ManagedPtyPresence, ResolvedProviderExecutable,
};
use crate::domain::agent_session::repository::{
    AgentSessionRepository, AgentSessionRepositoryError, VersionedAgentSession,
};
use crate::domain::agent_session::{
    AgentSessionHistoryGateway, AgentSessionHistoryGatewayError, AgentSessionHistoryMetadata,
    PreparedProviderLaunch, ProviderAgentLaunchGateway, ProviderAgentLaunchGatewayError,
    ProviderAgentTerminalGateway, ProviderAgentTerminalGatewayError,
    ProviderAgentTerminalSpawnError, ProviderAvailabilityReader, ProviderSessionLaunch,
};
use crate::domain::provider_lifecycle::{
    ArmedProviderLifecycle, ProviderHookHealth, ProviderHookHealthRepository,
    ProviderHookHealthRepositoryError, ProviderKind, ProviderLifecycleEventRepository,
    ProviderLifecycleRepositoryError, ProviderLifecycleScope, ProviderLifecycleUnavailableReason,
    ScopedProviderLifecycleEvent, VersionedProviderHookHealth,
};
use crate::domain::terminal_surface::{TerminalProcessLaunch, TerminalSurfaceOwner};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::provider_lifecycle::{ProviderHookHealthUsecase, ProviderLifecycleUsecase};

fn session_location(id: &str) -> AgentSessionTreeLocation {
    AgentSessionTreeLocation::session_tree_root(id).unwrap()
}

fn provider_runtime(
    availability: Arc<dyn ProviderAvailabilityReader>,
    launch_gateway: Arc<dyn ProviderAgentLaunchGateway>,
    terminal: Arc<dyn ProviderAgentTerminalGateway>,
) -> ProviderAgentRuntime {
    ProviderAgentRuntime::new(availability, launch_gateway, terminal)
}

fn workflow_location(tree_id: &str, node_execution_id: &str) -> AgentSessionTreeLocation {
    AgentSessionTreeLocation::workflow_node(tree_id, node_execution_id).unwrap()
}

#[derive(Default)]
struct RecordingStartedExecutionTrees {
    reservations: Mutex<Vec<String>>,
    tree_ids: Mutex<Vec<String>>,
    failure: Option<StartedExecutionTreeRegistrationError>,
    reservation_releases: Mutex<Vec<String>>,
    releases: Mutex<Vec<String>>,
    release_failure: Mutex<Option<ExecutionTreeCacheReleaseError>>,
}

#[async_trait::async_trait]
impl StartedExecutionTreeRegistrar for RecordingStartedExecutionTrees {
    async fn reserve_started_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), StartedExecutionTreeRegistrationError> {
        self.reservations.lock().unwrap().push(tree_id.to_string());
        Ok(())
    }

    async fn register_started_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), StartedExecutionTreeRegistrationError> {
        self.tree_ids.lock().unwrap().push(tree_id.to_string());
        if let Some(error) = self.failure {
            return Err(error);
        }
        Ok(())
    }

    async fn release_started_execution_tree_reservation(
        &self,
        tree_id: &str,
    ) -> Result<(), StartedExecutionTreeRegistrationError> {
        self.reservation_releases
            .lock()
            .unwrap()
            .push(tree_id.to_string());
        Ok(())
    }

    async fn release_deleted_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), ExecutionTreeCacheReleaseError> {
        self.releases.lock().unwrap().push(tree_id.to_string());
        match *self.release_failure.lock().unwrap() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

fn started_execution_trees() -> Arc<RecordingStartedExecutionTrees> {
    Arc::new(RecordingStartedExecutionTrees::default())
}

struct FailingSaveRepository {
    stored: Mutex<Option<VersionedAgentSession>>,
    create_calls: AtomicUsize,
    atomic_create_calls: AtomicUsize,
    atomic_create_failure: Mutex<Option<AgentSessionRepositoryError>>,
    launch_lifecycle_events: Mutex<Vec<ScopedProviderLifecycleEvent>>,
    remove_failure: Mutex<Option<AgentSessionRepositoryError>>,
}

struct CapturingLogger {
    messages: Mutex<Vec<String>>,
}

impl log::Log for CapturingLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Error
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.messages
                .lock()
                .unwrap()
                .push(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

static CAPTURING_LOGGER: CapturingLogger = CapturingLogger {
    messages: Mutex::new(Vec::new()),
};
static CAPTURING_LOGGER_INIT: Once = Once::new();

fn install_capturing_logger() {
    CAPTURING_LOGGER_INIT.call_once(|| {
        log::set_logger(&CAPTURING_LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Trace);
    });
}

fn captured_terminal_spawn_failure(agent_session_id: &str) -> Option<String> {
    CAPTURING_LOGGER
        .messages
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|message| {
            message.contains("AgentSession terminal spawn failed")
                && message.contains(agent_session_id)
        })
        .cloned()
}

#[test]
fn test_agent_session_terminal_spawn_error_記録用kindとpayloadを表示する() {
    let cases = [
        (
            ProviderAgentTerminalSpawnError::OwnerConflict,
            "kind=owner_conflict",
        ),
        (
            ProviderAgentTerminalSpawnError::PtySpawn {
                error: "openpty failed".to_string(),
            },
            "kind=pty_spawn error=openpty failed",
        ),
        (
            ProviderAgentTerminalSpawnError::OtherSpawnFailure {
                error: "checkpoint failed".to_string(),
            },
            "kind=other_spawn_failure error=checkpoint failed",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

impl FailingSaveRepository {
    fn new(mut session: AgentSession) -> Self {
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

    fn snapshot(&self) -> VersionedAgentSession {
        self.stored.lock().unwrap().clone().unwrap()
    }
}

#[async_trait::async_trait]
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
        _authorization: crate::domain::agent_session::aggregates::AgentSessionRemovalAuthorization,
        _caller_request_id: &str,
    ) -> Result<(), AgentSessionRepositoryError> {
        if let Some(error) = self.remove_failure.lock().unwrap().clone() {
            return Err(error);
        }
        *self.stored.lock().unwrap() = None;
        Ok(())
    }
}

#[tokio::test]
async fn test_agent_session_usecase選択されたproviderでstandalone_sessionを作成する() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let usecase = AgentSessionUsecase::new(repository);

    let created = usecase
        .create(
            "agent-session-1",
            WorkspaceIdentity::new("/repo"),
            "/repo/.worktrees/feature",
            ProviderKind::Codex,
            session_location("agent-session-1"),
            "create-request-1",
        )
        .await
        .unwrap();

    assert_eq!(created.session().provider(), ProviderKind::Codex);
    assert_eq!(
        created.session().tree_location(),
        &session_location("agent-session-1")
    );
    assert_eq!(created.revision(), 1);
}

#[tokio::test]
async fn test_agent_session_usecase永続化失敗で共有状態を進めない() {
    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        session_location("agent-session-1"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(session));
    let usecase = AgentSessionUsecase::new(repository.clone());

    let result = usecase
        .associate_provider_session(
            "agent-session-1",
            "provider-session-1",
            None,
            "associate-request-1",
        )
        .await;

    assert_eq!(result.unwrap_err(), AgentSessionUsecaseError::Unavailable);
    let unchanged = repository.snapshot();
    assert_eq!(unchanged.revision(), 1);
    assert_eq!(unchanged.session().provider_session_id(), None);
}

struct FixedAvailability {
    available: bool,
    checks: Mutex<Vec<ProviderKind>>,
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

struct PanicOnFirstCheckAvailability {
    checks: AtomicUsize,
}

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
struct RecordingLifecycleEvents {
    events: Mutex<Vec<ScopedProviderLifecycleEvent>>,
}

#[derive(Default)]
struct FailingFirstLifecycleEvents {
    attempts: AtomicUsize,
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
struct RecordingLaunchGateway {
    armed: Mutex<Vec<ArmedProviderLifecycle>>,
    launches: Mutex<Vec<ProviderSessionLaunch>>,
    executables: Mutex<Vec<ResolvedProviderExecutable>>,
    cleanups: Mutex<Vec<String>>,
    fail_prepare: Mutex<bool>,
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
            return Err(ProviderAgentLaunchGatewayError::Unavailable);
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
struct MemoryHookHealthRepository {
    stored: Mutex<std::collections::HashMap<ProviderKind, VersionedProviderHookHealth>>,
    save_count: AtomicUsize,
    block_first_save: Option<Arc<HookHealthSaveBarrier>>,
}

#[derive(Default)]
struct HookHealthSaveBarrier {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
    completed: tokio::sync::Notify,
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

fn hook_health_usecase() -> Arc<ProviderHookHealthUsecase> {
    Arc::new(ProviderHookHealthUsecase::new(Arc::new(
        MemoryHookHealthRepository::default(),
    )))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedTerminalSpawn {
    owner: TerminalSurfaceOwner,
    worktree_path: String,
    process: TerminalProcessLaunch,
    rows: u16,
    cols: u16,
}

#[derive(Default)]
struct RecordingTerminal {
    spawns: Mutex<Vec<RecordedTerminalSpawn>>,
    spawn_error: Mutex<Option<ProviderAgentTerminalSpawnError>>,
    fail_delete: Mutex<bool>,
    deletes: Mutex<usize>,
}

struct BlockingLaunchTerminal {
    presence: Mutex<ManagedPtyPresence>,
    spawn_entered: Mutex<Option<mpsc::Sender<()>>>,
    spawn_release: Mutex<Option<mpsc::Receiver<()>>>,
    spawns: AtomicUsize,
    deletes: Mutex<usize>,
}

impl ProviderAgentTerminalGateway for BlockingLaunchTerminal {
    fn spawn(
        &self,
        _owner: TerminalSurfaceOwner,
        _worktree_path: &str,
        _process: TerminalProcessLaunch,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), ProviderAgentTerminalSpawnError> {
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
    ) -> Result<(), ProviderAgentTerminalSpawnError> {
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
            return Err(ProviderAgentTerminalGatewayError::Unavailable);
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

fn launch_usecase(
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

fn launch_usecase_with_hook_health(
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

fn launch_usecase_with_tree_registrar(
    repository: Arc<FailingSaveRepository>,
    availability: Arc<FixedAvailability>,
    launch_gateway: Arc<RecordingLaunchGateway>,
    terminal: Arc<RecordingTerminal>,
    hook_health: Arc<ProviderHookHealthUsecase>,
    execution_trees: Arc<dyn StartedExecutionTreeRegistrar>,
) -> AgentSessionLaunchUsecase {
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(
            crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
        ),
        Arc::new(RecordingLifecycleEvents::default()),
    ));
    AgentSessionLaunchUsecase::new(
        Arc::new(AgentSessionUsecase::new(repository)),
        lifecycle,
        provider_runtime(availability, launch_gateway, terminal),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health,
        execution_trees,
    )
}

struct FixedHistory {
    entries: Vec<AgentSessionHistoryMetadata>,
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
}

#[tokio::test]
async fn test_agent_session_launch_利用可能な選択providerをterminal_root_processへ接続する() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let availability = Arc::new(FixedAvailability {
        available: true,
        checks: Mutex::new(Vec::new()),
    });
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let execution_trees = started_execution_trees();
    let usecase = launch_usecase_with_tree_registrar(
        repository,
        availability.clone(),
        launch_gateway.clone(),
        terminal.clone(),
        hook_health_usecase(),
        execution_trees.clone(),
    );

    let launched = usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/.worktrees/feature".to_string(),
            provider: ProviderKind::Codex,
            rows: 30,
            cols: 120,
            caller_request_id: "launch-request-1".to_string(),
        })
        .await
        .unwrap();
    let expected_id = launched.session().id();

    assert_eq!(launched.session().provider(), ProviderKind::Codex);
    assert_eq!(
        availability.checks.lock().unwrap().as_slice(),
        &[ProviderKind::Codex]
    );
    let armed = launch_gateway.armed.lock().unwrap();
    assert_eq!(armed.len(), 1);
    assert_eq!(armed[0].scope().agent_session_id(), expected_id);
    let spawns = terminal.spawns.lock().unwrap();
    assert_eq!(spawns.len(), 1);
    assert_eq!(
        spawns[0].owner,
        TerminalSurfaceOwner::session(WorkspaceIdentity::new("/repo"), expected_id).unwrap()
    );
    assert_eq!(spawns[0].worktree_path, "/repo/.worktrees/feature");
    assert_eq!(spawns[0].process.executable(), "/opt/bin/provider");
    assert_eq!((spawns[0].rows, spawns[0].cols), (30, 120));
    assert_eq!(
        execution_trees.reservations.lock().unwrap().as_slice(),
        &[expected_id.to_string()]
    );
    assert_eq!(
        execution_trees.tree_ids.lock().unwrap().as_slice(),
        &[expected_id.to_string()]
    );
    assert_eq!(
        execution_trees
            .reservation_releases
            .lock()
            .unwrap()
            .as_slice(),
        &[expected_id.to_string()]
    );
}

#[tokio::test]
async fn test_agent_session_launch_create_commit失敗でも実行木予約を解放する() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    *repository.atomic_create_failure.lock().unwrap() =
        Some(AgentSessionRepositoryError::Unavailable);
    let execution_trees = started_execution_trees();
    let usecase = launch_usecase_with_tree_registrar(
        repository,
        Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        }),
        Arc::new(RecordingLaunchGateway::default()),
        Arc::new(RecordingTerminal::default()),
        hook_health_usecase(),
        execution_trees.clone(),
    );

    let error = usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            rows: 24,
            cols: 80,
            caller_request_id: "create-commit-failure".to_string(),
        })
        .await
        .unwrap_err();

    assert_eq!(error, AgentSessionLaunchUsecaseError::StorageUnavailable);
    let expected_id = execution_trees.reservations.lock().unwrap()[0].clone();
    assert!(execution_trees.tree_ids.lock().unwrap().is_empty());
    assert_eq!(
        execution_trees
            .reservation_releases
            .lock()
            .unwrap()
            .as_slice(),
        std::slice::from_ref(&expected_id)
    );
}

#[tokio::test]
async fn test_agent_session_launch_実行木登録失敗ではcreateと起動資源をrollbackする() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let lifecycle_events = Arc::new(RecordingLifecycleEvents::default());
    let execution_trees = Arc::new(RecordingStartedExecutionTrees {
        failure: Some(StartedExecutionTreeRegistrationError::Unavailable),
        ..RecordingStartedExecutionTrees::default()
    });
    let usecase = AgentSessionLaunchUsecase::new(
        Arc::new(AgentSessionUsecase::new(repository.clone())),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            lifecycle_events.clone(),
        )),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal.clone(),
        ),
        Arc::new(FixedHistory { entries: Vec::new() }),
        hook_health_usecase(),
        execution_trees.clone(),
    );

    let error = usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            rows: 24,
            cols: 80,
            caller_request_id: "registration-failure".to_string(),
        })
        .await
        .unwrap_err();

    assert_eq!(error, AgentSessionLaunchUsecaseError::StorageUnavailable);
    assert!(repository.stored.lock().unwrap().is_none());
    assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 1);
    let expected_id = execution_trees.tree_ids.lock().unwrap()[0].clone();
    assert_eq!(
        execution_trees.reservations.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees
            .reservation_releases
            .lock()
            .unwrap()
            .as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees.releases.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(*terminal.deletes.lock().unwrap(), 1);
    assert!(launch_gateway.launches.lock().unwrap().is_empty());
    assert!(terminal.spawns.lock().unwrap().is_empty());
    assert!(lifecycle_events.events.lock().unwrap().iter().any(|event| {
        matches!(
            event.clone().into_parts().1,
            crate::domain::provider_lifecycle::ProviderLifecycleEvent::BindingExpired { .. }
        )
    }));
}

#[tokio::test]
async fn test_agent_session_launch_session作成とlifecycle_armを一回のrepository操作で永続化する() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let lifecycle_events = Arc::new(RecordingLifecycleEvents::default());
    let usecase = AgentSessionLaunchUsecase::new(
        Arc::new(AgentSessionUsecase::new(repository.clone())),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            lifecycle_events.clone(),
        )),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            Arc::new(RecordingLaunchGateway::default()),
            Arc::new(RecordingTerminal::default()),
        ),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health_usecase(),
        started_execution_trees(),
    );

    usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Claude,
            rows: 24,
            cols: 80,
            caller_request_id: "atomic-launch-request".to_string(),
        })
        .await
        .unwrap();

    assert_eq!(repository.create_calls.load(Ordering::SeqCst), 0);
    assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 1);
    assert_eq!(repository.launch_lifecycle_events.lock().unwrap().len(), 1);
    assert!(lifecycle_events.events.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_agent_session_launch_hookwarning保存完了を待たずpty起動済みsessionを返す() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let barrier = Arc::new(HookHealthSaveBarrier::default());
    let hook_repository = Arc::new(MemoryHookHealthRepository {
        block_first_save: Some(barrier.clone()),
        ..Default::default()
    });
    let hook_health = Arc::new(ProviderHookHealthUsecase::new(hook_repository));
    let usecase = launch_usecase_with_hook_health(
        repository,
        Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        }),
        Arc::new(RecordingLaunchGateway::default()),
        Arc::new(RecordingTerminal::default()),
        hook_health.clone(),
    );
    let mut launch = tokio::spawn(async move {
        usecase
            .launch_standalone(AgentSessionLaunchRequest {
                workspace: WorkspaceIdentity::new("/repo"),
                worktree_path: "/repo/worktree".to_string(),
                provider: ProviderKind::Codex,
                rows: 24,
                cols: 80,
                caller_request_id: "launch-hook-warning".to_string(),
            })
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), barrier.started.notified())
        .await
        .unwrap();
    let returned_before_warning_save =
        tokio::time::timeout(Duration::from_millis(100), &mut launch)
            .await
            .is_ok();
    barrier.release.notify_one();
    if !returned_before_warning_save {
        launch.await.unwrap().unwrap();
    }
    tokio::time::timeout(Duration::from_secs(1), barrier.completed.notified())
        .await
        .unwrap();

    assert!(returned_before_warning_save);
    assert_eq!(hook_health.warnings().await.unwrap().len(), 1);
}

#[tokio::test]
async fn test_agent_session_launch_利用不可providerではsessionもptyも作らない() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let availability = Arc::new(FixedAvailability {
        available: false,
        checks: Mutex::new(Vec::new()),
    });
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let usecase = launch_usecase(
        repository.clone(),
        availability,
        launch_gateway.clone(),
        terminal.clone(),
    );

    let result = usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/.worktrees/feature".to_string(),
            provider: ProviderKind::Claude,
            rows: 24,
            cols: 80,
            caller_request_id: "launch-request-1".to_string(),
        })
        .await;

    assert_eq!(
        result.unwrap_err(),
        AgentSessionLaunchUsecaseError::ProviderUnavailable
    );
    assert!(repository.stored.lock().unwrap().is_none());
    assert!(launch_gateway.armed.lock().unwrap().is_empty());
    assert!(terminal.spawns.lock().unwrap().is_empty());
}

fn idempotent_launch_request(caller_request_id: &str) -> AgentSessionLaunchRequest {
    AgentSessionLaunchRequest {
        workspace: WorkspaceIdentity::new("/repo"),
        worktree_path: "/repo/.worktrees/feature".to_string(),
        provider: ProviderKind::Claude,
        rows: 24,
        cols: 80,
        caller_request_id: caller_request_id.to_string(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_agent_session_launch_同一request_idの並行呼び出しはsessionを一度だけ作成し同じ結果を返す(
) {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let terminal = Arc::new(BlockingLaunchTerminal {
        presence: Mutex::new(ManagedPtyPresence::ConfirmedAbsent),
        spawn_entered: Mutex::new(Some(entered_sender)),
        spawn_release: Mutex::new(Some(release_receiver)),
        spawns: AtomicUsize::new(0),
        deletes: Mutex::new(0),
    });
    let usecase = Arc::new(AgentSessionLaunchUsecase::new(
        Arc::new(AgentSessionUsecase::new(repository.clone())),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            Arc::new(RecordingLaunchGateway::default()),
            terminal,
        ),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health_usecase(),
        started_execution_trees(),
    ));

    let first = tokio::spawn(
        Arc::clone(&usecase).launch_standalone_idempotent(idempotent_launch_request("request-dup")),
    );
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let second = tokio::spawn(
        Arc::clone(&usecase).launch_standalone_idempotent(idempotent_launch_request("request-dup")),
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    release_sender.send(()).unwrap();

    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap().unwrap();
    assert_eq!(first, second);
    assert!(first.starts_with("agent-session-"));
    assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_agent_session_launch_完了済みrequest_id再送は再作成せず同じsession識別子を返す() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let availability = Arc::new(FixedAvailability {
        available: true,
        checks: Mutex::new(Vec::new()),
    });
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let usecase = Arc::new(launch_usecase(
        repository.clone(),
        availability,
        launch_gateway,
        terminal.clone(),
    ));

    let first = Arc::clone(&usecase)
        .launch_standalone_idempotent(idempotent_launch_request("request-1"))
        .await
        .unwrap();
    let replay = Arc::clone(&usecase)
        .launch_standalone_idempotent(idempotent_launch_request("request-1"))
        .await
        .unwrap();

    assert_eq!(first, replay);
    assert!(first.starts_with("agent-session-"));
    assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 1);
    assert_eq!(terminal.spawns.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_agent_session_launch_異なるrequest_idは別のsessionを作成する() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let availability = Arc::new(FixedAvailability {
        available: true,
        checks: Mutex::new(Vec::new()),
    });
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let usecase = Arc::new(launch_usecase(
        repository.clone(),
        availability,
        launch_gateway,
        terminal.clone(),
    ));

    let first = Arc::clone(&usecase)
        .launch_standalone_idempotent(idempotent_launch_request("request-1"))
        .await
        .unwrap();
    let second = Arc::clone(&usecase)
        .launch_standalone_idempotent(idempotent_launch_request("request-2"))
        .await
        .unwrap();

    assert_ne!(first, second);
    assert!(first.starts_with("agent-session-"));
    assert!(second.starts_with("agent-session-"));
    assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 2);
    assert_eq!(terminal.spawns.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn test_agent_session_launch_失敗結果も記録し同一request_id再送へ同じ失敗を返す() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let availability = Arc::new(FixedAvailability {
        available: false,
        checks: Mutex::new(Vec::new()),
    });
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let usecase = Arc::new(launch_usecase(
        repository.clone(),
        availability.clone(),
        launch_gateway,
        terminal.clone(),
    ));

    let first = Arc::clone(&usecase)
        .launch_standalone_idempotent(idempotent_launch_request("request-fail"))
        .await;
    let replay = Arc::clone(&usecase)
        .launch_standalone_idempotent(idempotent_launch_request("request-fail"))
        .await;

    assert_eq!(
        first.unwrap_err(),
        AgentSessionLaunchUsecaseError::ProviderUnavailable
    );
    assert_eq!(
        replay.unwrap_err(),
        AgentSessionLaunchUsecaseError::ProviderUnavailable
    );
    assert_eq!(availability.checks.lock().unwrap().len(), 1);
    assert!(repository.stored.lock().unwrap().is_none());
    assert!(terminal.spawns.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_agent_session_launch_起動panic後はin_flightに残さず同一request_id再送へ同じ失敗を返す(
) {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let availability = Arc::new(PanicOnFirstCheckAvailability {
        checks: AtomicUsize::new(0),
    });
    let usecase = Arc::new(AgentSessionLaunchUsecase::new(
        Arc::new(AgentSessionUsecase::new(repository.clone())),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        provider_runtime(
            availability.clone(),
            Arc::new(RecordingLaunchGateway::default()),
            Arc::new(RecordingTerminal::default()),
        ),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health_usecase(),
        started_execution_trees(),
    ));

    let first = Arc::clone(&usecase)
        .launch_standalone_idempotent(idempotent_launch_request("request-panic"))
        .await;
    let replay = Arc::clone(&usecase)
        .launch_standalone_idempotent(idempotent_launch_request("request-panic"))
        .await;

    assert_eq!(first.unwrap_err(), AgentSessionLaunchUsecaseError::Corrupt);
    assert_eq!(replay.unwrap_err(), AgentSessionLaunchUsecaseError::Corrupt);
    assert_eq!(usecase.standalone_in_flight_request_count().await, 0);
    assert_eq!(availability.checks.load(Ordering::SeqCst), 1);
    assert!(repository.stored.lock().unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_agent_session_launch_pty起動中のsessionをgcしない() {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store.clone()),
    )));
    let provider_lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(
            crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
        ),
        Arc::new(RecordingLifecycleEvents::default()),
    ));
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let terminal = Arc::new(BlockingLaunchTerminal {
        presence: Mutex::new(ManagedPtyPresence::ConfirmedAbsent),
        spawn_entered: Mutex::new(Some(entered_sender)),
        spawn_release: Mutex::new(Some(release_receiver)),
        spawns: AtomicUsize::new(0),
        deletes: Mutex::new(0),
    });
    let launches = Arc::new(RecordingLaunchGateway::default());
    let hook_health = hook_health_usecase();
    let launch = Arc::new(AgentSessionLaunchUsecase::new(
        sessions.clone(),
        provider_lifecycle.clone(),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launches.clone(),
            terminal.clone(),
        ),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health.clone(),
        started_execution_trees(),
    ));
    struct NoopChangeNotifier;
    impl crate::usecase::agent_session::AgentSessionChangeNotifier for NoopChangeNotifier {
        fn agent_session_changed(&self, _worktree_path: &str) {}
    }
    let lifecycle = Arc::new(AgentSessionLifecycleUsecase::new(
        sessions.clone(),
        provider_lifecycle,
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launches.clone(),
            terminal.clone(),
        ),
        hook_health,
        Arc::new(NoopChangeNotifier),
        started_execution_trees(),
    ));

    let launching = tokio::spawn(async move {
        launch
            .launch_standalone(AgentSessionLaunchRequest {
                workspace: WorkspaceIdentity::new("/repo"),
                worktree_path: "/repo/worktree".to_string(),
                provider: ProviderKind::Claude,
                rows: 24,
                cols: 80,
                caller_request_id: "launch-gc".to_string(),
            })
            .await
    });
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let expected_id = launches.armed.lock().unwrap()[0]
        .scope()
        .agent_session_id()
        .to_string();
    let gc_agent_session_id = expected_id.clone();
    let collecting = tokio::spawn(async move {
        lifecycle
            .reconcile_garbage_collection(&gc_agent_session_id, "gc-during-launch")
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    release_sender.send(()).unwrap();

    launching.await.unwrap().unwrap();
    assert_eq!(
        collecting.await.unwrap().unwrap(),
        super::AgentSessionGarbageCollectionOutcome::Retained
    );
    assert!(sessions.find(&expected_id).await.unwrap().is_some());
    assert_eq!(*terminal.deletes.lock().unwrap(), 0);
}

#[tokio::test]
async fn test_agent_session_history_resume_実行木登録失敗ではcreateをrollbackする() {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let execution_trees = Arc::new(RecordingStartedExecutionTrees {
        failure: Some(StartedExecutionTreeRegistrationError::Unavailable),
        ..RecordingStartedExecutionTrees::default()
    });
    let usecase = AgentSessionLaunchUsecase::new(
        sessions.clone(),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal.clone(),
        ),
        Arc::new(FixedHistory {
            entries: vec![AgentSessionHistoryMetadata {
                provider: ProviderKind::Codex,
                provider_session_id: "provider-registration-failure".to_string(),
                worktree_path: "/repo/worktree".to_string(),
                updated_at_ms: 10,
            }],
        }),
        hook_health_usecase(),
        execution_trees.clone(),
    );

    let error = usecase
        .resume_history(AgentSessionHistoryResumeRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            provider_session_id: "provider-registration-failure".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "history-registration-failure".to_string(),
        })
        .await
        .unwrap_err();

    assert_eq!(error, AgentSessionLaunchUsecaseError::StorageUnavailable);
    let expected_id = execution_trees.tree_ids.lock().unwrap()[0].clone();
    assert_eq!(
        execution_trees.reservations.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees
            .reservation_releases
            .lock()
            .unwrap()
            .as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert!(sessions.find(&expected_id).await.unwrap().is_none());
    assert_eq!(
        execution_trees.releases.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert!(launch_gateway.launches.lock().unwrap().is_empty());
    assert!(launch_gateway.cleanups.lock().unwrap().is_empty());
    assert!(terminal.spawns.lock().unwrap().is_empty());
    assert_eq!(*terminal.deletes.lock().unwrap(), 0);
}

#[tokio::test]
async fn test_agent_session_history_resume_同一要求の再送は既存sessionへ収束する() {
    // Given: provider history に再開対象が存在する
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store),
    )));
    let lifecycle_events = Arc::new(RecordingLifecycleEvents::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let usecase = AgentSessionLaunchUsecase::new(
        sessions,
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            lifecycle_events,
        )),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            Arc::new(RecordingLaunchGateway::default()),
            terminal.clone(),
        ),
        Arc::new(FixedHistory {
            entries: vec![AgentSessionHistoryMetadata {
                provider: ProviderKind::Claude,
                provider_session_id: "provider-history-idempotent".to_string(),
                worktree_path: "/repo/worktree".to_string(),
                updated_at_ms: 10,
            }],
        }),
        hook_health_usecase(),
        started_execution_trees(),
    );
    let request = AgentSessionHistoryResumeRequest {
        workspace: WorkspaceIdentity::new("/repo"),
        worktree_path: "/repo/worktree".to_string(),
        provider: ProviderKind::Claude,
        provider_session_id: "provider-history-idempotent".to_string(),
        rows: 24,
        cols: 80,
        caller_request_id: "history-resume-idempotent".to_string(),
    };

    // When: 同じ履歴 resume 要求を二度送る
    let first = usecase.resume_history(request.clone()).await.unwrap();
    let second = usecase.resume_history(request.clone()).await.unwrap();

    // Then: どちらも caller request から導出した同じ既存 Session を返す
    let AgentSessionHistoryResumeOutcome::Open(first) = first else {
        panic!("first history resume must keep the session open");
    };
    let AgentSessionHistoryResumeOutcome::Open(second) = second else {
        panic!("repeated history resume must keep the session open");
    };
    let expected_id = crate::domain::agent_session::launch_resource_id(
        "agent-session",
        &request.caller_request_id,
    )
    .unwrap();
    assert_eq!(first.session().id(), expected_id);
    assert_eq!(second.session().id(), expected_id);
    assert_eq!(
        second.session().provider_session_id(),
        Some(request.provider_session_id.as_str())
    );
    assert_eq!(terminal.spawns.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn test_agent_session_history_resumeは新しいsessionを作り失敗時もidを保持する() {
    install_capturing_logger();
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store.clone()),
    )));
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(
            crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
        ),
        Arc::new(RecordingLifecycleEvents::default()),
    ));
    let availability = Arc::new(FixedAvailability {
        available: true,
        checks: Mutex::new(Vec::new()),
    });
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    *terminal.spawn_error.lock().unwrap() =
        Some(ProviderAgentTerminalSpawnError::OtherSpawnFailure {
            error: "checkpoint restore failed".to_string(),
        });
    let hook_health = hook_health_usecase();
    let execution_trees = started_execution_trees();
    let usecase = AgentSessionLaunchUsecase::new(
        sessions.clone(),
        lifecycle,
        provider_runtime(availability, launch_gateway.clone(), terminal),
        Arc::new(FixedHistory {
            entries: vec![AgentSessionHistoryMetadata {
                provider: ProviderKind::Codex,
                provider_session_id: "provider-history-1".to_string(),
                worktree_path: "/repo/worktree".to_string(),
                updated_at_ms: 10,
            }],
        }),
        hook_health,
        execution_trees.clone(),
    );

    let outcome = usecase
        .resume_history(AgentSessionHistoryResumeRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            provider_session_id: "provider-history-1".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "history-resume-1".to_string(),
        })
        .await
        .unwrap();

    let expected_id = match outcome {
        AgentSessionHistoryResumeOutcome::Paused(session) => session.session().id().to_string(),
        AgentSessionHistoryResumeOutcome::Open(_) => panic!("expected paused session"),
    };
    let saved = sessions.find(&expected_id).await.unwrap().unwrap();
    assert_eq!(saved.session().lifecycle(), AgentSessionLifecycle::Paused);
    assert_eq!(
        saved.session().provider_session_id(),
        Some("provider-history-1")
    );
    assert_eq!(
        launch_gateway.launches.lock().unwrap().as_slice(),
        &[ProviderSessionLaunch::resume("provider-history-1").unwrap()]
    );
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees.tree_ids.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees.reservations.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees
            .reservation_releases
            .lock()
            .unwrap()
            .as_slice(),
        std::slice::from_ref(&expected_id)
    );
    let record = captured_terminal_spawn_failure(&expected_id).unwrap();
    assert!(record.contains("kind=other_spawn_failure"));
    assert!(record.contains("error=checkpoint restore failed"));
}

#[tokio::test]
async fn test_agent_session_history_resume_lifecycle準備失敗でもpausedへ収束する() {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store.clone()),
    )));
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(
            crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
        ),
        Arc::new(FailingFirstLifecycleEvents::default()),
    ));
    let usecase = AgentSessionLaunchUsecase::new(
        sessions.clone(),
        lifecycle,
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            Arc::new(RecordingLaunchGateway::default()),
            Arc::new(RecordingTerminal::default()),
        ),
        Arc::new(FixedHistory {
            entries: vec![AgentSessionHistoryMetadata {
                provider: ProviderKind::Codex,
                provider_session_id: "provider-history-arm-failure".to_string(),
                worktree_path: "/repo/worktree".to_string(),
                updated_at_ms: 10,
            }],
        }),
        hook_health_usecase(),
        started_execution_trees(),
    );

    let outcome = usecase
        .resume_history(AgentSessionHistoryResumeRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            provider_session_id: "provider-history-arm-failure".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "history-arm-failure".to_string(),
        })
        .await
        .unwrap();

    let expected_id = match outcome {
        AgentSessionHistoryResumeOutcome::Paused(session) => session.session().id().to_string(),
        AgentSessionHistoryResumeOutcome::Open(_) => panic!("expected paused session"),
    };
    let saved = sessions.find(&expected_id).await.unwrap().unwrap();
    assert_eq!(saved.session().lifecycle(), AgentSessionLifecycle::Paused);
    assert_eq!(
        saved.session().provider_session_id(),
        Some("provider-history-arm-failure")
    );
}

fn durable_usecase(
    directory: &tempfile::TempDir,
) -> (
    Arc<crate::adaptor::gateway::local_event_store::LocalEventStore>,
    AgentSessionUsecase,
) {
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let repository = Arc::new(
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store.clone()),
    );
    (store, AgentSessionUsecase::new(repository))
}

#[tokio::test]
async fn test_agent_session_usecase_process_exit_resume_archive_deleteを永続化する() {
    let directory = tempfile::tempdir().unwrap();
    let (_store, usecase) = durable_usecase(&directory);
    usecase
        .create(
            "agent-session-1",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("agent-session-1"),
            "create-1",
        )
        .await
        .unwrap();
    usecase
        .associate_provider_session("agent-session-1", "claude-session-1", None, "associate-1")
        .await
        .unwrap();

    assert_eq!(
        usecase
            .observe_process_exit("agent-session-1", Some(0), "exit-1")
            .await
            .unwrap(),
        AgentSessionProcessExitOutcome::Paused
    );
    assert_eq!(
        usecase
            .find("agent-session-1")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Paused
    );
    usecase
        .complete_resume(
            "agent-session-1",
            AgentSessionRecoveryResult::Failed,
            "resume-failed-1",
        )
        .await
        .unwrap();
    assert_eq!(
        usecase
            .find("agent-session-1")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Paused
    );
    usecase
        .complete_resume(
            "agent-session-1",
            AgentSessionRecoveryResult::Succeeded,
            "resume-success-1",
        )
        .await
        .unwrap();
    assert_eq!(
        usecase
            .archive("agent-session-1", "archive-1")
            .await
            .unwrap(),
        AgentSessionArchiveOutcome::Archived
    );
    usecase.delete("agent-session-1", "delete-1").await.unwrap();
    assert!(usecase.find("agent-session-1").await.unwrap().is_none());
}

#[tokio::test]
async fn test_agent_session_usecase_id不明archiveは確認後deleteへ縮退する() {
    let directory = tempfile::tempdir().unwrap();
    let (_store, usecase) = durable_usecase(&directory);
    usecase
        .create(
            "agent-session-unknown",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            session_location("agent-session-unknown"),
            "create-unknown",
        )
        .await
        .unwrap();

    assert_eq!(
        usecase
            .archive("agent-session-unknown", "archive-unknown")
            .await
            .unwrap(),
        AgentSessionArchiveOutcome::DeleteConfirmationRequired
    );
    assert!(usecase
        .find("agent-session-unknown")
        .await
        .unwrap()
        .is_some());
    usecase
        .confirm_archive_fallback_delete("agent-session-unknown", "fallback-delete-1")
        .await
        .unwrap();
    assert!(usecase
        .find("agent-session-unknown")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_agent_session_usecase_gcはpty不在確定時だけunknown_idを削除する() {
    let directory = tempfile::tempdir().unwrap();
    let (_store, usecase) = durable_usecase(&directory);
    usecase
        .create(
            "agent-session-gc",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            session_location("agent-session-gc"),
            "create-gc",
        )
        .await
        .unwrap();

    assert_eq!(
        usecase
            .garbage_collect(
                "agent-session-gc",
                ManagedPtyPresence::Unknown,
                "gc-unknown"
            )
            .await
            .unwrap_err(),
        AgentSessionUsecaseError::InvalidOperation
    );
    usecase
        .garbage_collect(
            "agent-session-gc",
            ManagedPtyPresence::ConfirmedAbsent,
            "gc-confirmed",
        )
        .await
        .unwrap();
    assert!(usecase.find("agent-session-gc").await.unwrap().is_none());
}

#[tokio::test]
async fn test_provider_agent_workflow_session_launch_workflow関連付け後に初回指示付きprovider_tuiを起動する(
) {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store.clone()),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let usecase = AgentSessionLaunchUsecase::new(
        sessions,
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal.clone(),
        ),
        Arc::new(FixedHistory { entries: Vec::new() }),
        hook_health_usecase(),
        started_execution_trees(),
    );

    let launched = usecase
        .prepare_workflow_node(WorkflowAgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            model: None,
            permission: None,
            workflow_execution_id: "workflow-1".to_string(),
            node_execution_id: "node-1".to_string(),
            initial_instruction: "Implement the workflow node.".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "workflow-launch-1".to_string(),
        })
        .await
        .unwrap();

    assert_eq!(
        launched.session().tree_location(),
        &workflow_location("workflow-1", "node-1")
    );
    assert_eq!(
        launch_gateway.launches.lock().unwrap().as_slice(),
        &[
            ProviderSessionLaunch::new_with_initial_instruction("Implement the workflow node.")
                .unwrap()
        ]
    );
    assert!(launched.session().initial_instruction_admitted());
    assert!(terminal.spawns.lock().unwrap().is_empty());

    usecase
        .activate_workflow_node(launched.session().id())
        .await
        .unwrap();
    {
        let spawns = terminal.spawns.lock().unwrap();
        assert_eq!(spawns.len(), 1);
        assert_eq!((spawns[0].rows, spawns[0].cols), (24, 80));
    }
    usecase
        .confirm_workflow_node_attachment(launched.session().id())
        .await
        .unwrap();
    assert_eq!(
        usecase
            .confirm_workflow_node_attachment(launched.session().id())
            .await,
        Err(AgentSessionLaunchUsecaseError::InvalidInput)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_provider_agent_workflow_session_launch_別sessionのactivateを起動待ちで直列化しない() {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store),
    )));
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let terminal = Arc::new(BlockingLaunchTerminal {
        presence: Mutex::new(ManagedPtyPresence::ConfirmedAbsent),
        spawn_entered: Mutex::new(Some(entered_sender)),
        spawn_release: Mutex::new(Some(release_receiver)),
        spawns: AtomicUsize::new(0),
        deletes: Mutex::new(0),
    });
    let usecase = Arc::new(AgentSessionLaunchUsecase::new(
        sessions,
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            Arc::new(RecordingLaunchGateway::default()),
            terminal.clone(),
        ),
        Arc::new(FixedHistory { entries: Vec::new() }),
        hook_health_usecase(),
        started_execution_trees(),
    ));
    let first = usecase
        .prepare_workflow_node(WorkflowAgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree-first".to_string(),
            provider: ProviderKind::Claude,
            model: None,
            permission: None,
            workflow_execution_id: "workflow-parallel".to_string(),
            node_execution_id: "node-first".to_string(),
            initial_instruction: "Implement first.".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "workflow-parallel-first".to_string(),
        })
        .await
        .unwrap();
    let second = usecase
        .prepare_workflow_node(WorkflowAgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree-second".to_string(),
            provider: ProviderKind::Claude,
            model: None,
            permission: None,
            workflow_execution_id: "workflow-parallel".to_string(),
            node_execution_id: "node-second".to_string(),
            initial_instruction: "Implement second.".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "workflow-parallel-second".to_string(),
        })
        .await
        .unwrap();

    let first_session_id = first.session().id().to_string();
    let first_usecase = Arc::clone(&usecase);
    let first_activation = tokio::spawn(async move {
        first_usecase
            .activate_workflow_node(&first_session_id)
            .await
    });
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let second_session_id = second.session().id().to_string();
    let second_usecase = Arc::clone(&usecase);
    let mut second_activation = tokio::spawn(async move {
        second_usecase
            .activate_workflow_node(&second_session_id)
            .await
    });
    let second_result = tokio::time::timeout(Duration::from_secs(5), &mut second_activation).await;
    release_sender.send(()).unwrap();
    first_activation.await.unwrap().unwrap();

    match second_result {
        Ok(result) => {
            result.unwrap().unwrap();
        }
        Err(_) => {
            second_activation.await.unwrap().unwrap();
            panic!("unrelated workflow activation waited for another terminal spawn");
        }
    }
    assert_eq!(terminal.spawns.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_provider_agent_workflow_session_launch_activate後のrollbackで起動資源を解放する() {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store),
    )));
    let lifecycle_events = Arc::new(RecordingLifecycleEvents::default());
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let usecase = AgentSessionLaunchUsecase::new(
        sessions,
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            lifecycle_events.clone(),
        )),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal.clone(),
        ),
        Arc::new(FixedHistory { entries: Vec::new() }),
        hook_health_usecase(),
        started_execution_trees(),
    );
    let launched = usecase
        .prepare_workflow_node(WorkflowAgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Claude,
            model: None,
            permission: None,
            workflow_execution_id: "workflow-rollback".to_string(),
            node_execution_id: "node-rollback".to_string(),
            initial_instruction: "Implement rollback.".to_string(),
            rows: 24,
            cols: 80,
            caller_request_id: "workflow-launch-rollback".to_string(),
        })
        .await
        .unwrap();
    usecase
        .activate_workflow_node(launched.session().id())
        .await
        .unwrap();

    usecase
        .rollback_workflow_node(launched.session().id(), "rollback-request")
        .await
        .unwrap();

    assert_eq!(*terminal.deletes.lock().unwrap(), 1);
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        &[launched.session().id().to_string()]
    );
    assert!(lifecycle_events.events.lock().unwrap().iter().any(|event| {
        matches!(
            event.clone().into_parts().1,
            crate::domain::provider_lifecycle::ProviderLifecycleEvent::BindingExpired { .. }
        )
    }));
}

#[tokio::test]
async fn test_agent_session_launch_spawn失敗時はsessionとlaunch資源をrollbackする() {
    install_capturing_logger();
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store.clone()),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    *terminal.spawn_error.lock().unwrap() = Some(ProviderAgentTerminalSpawnError::PtySpawn {
        error: "openpty failed".to_string(),
    });
    let hook_health = hook_health_usecase();
    let execution_trees = started_execution_trees();
    let usecase = AgentSessionLaunchUsecase::new(
        sessions.clone(),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal,
        ),
        Arc::new(FixedHistory { entries: Vec::new() }),
        hook_health,
        execution_trees.clone(),
    );

    let result = usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            rows: 24,
            cols: 80,
            caller_request_id: "failed-launch-1".to_string(),
        })
        .await;

    assert_eq!(
        result.unwrap_err(),
        AgentSessionLaunchUsecaseError::TerminalSpawn(ProviderAgentTerminalSpawnError::PtySpawn {
            error: "openpty failed".to_string()
        })
    );
    let expected_id = launch_gateway.cleanups.lock().unwrap()[0].clone();
    assert!(sessions.find(&expected_id).await.unwrap().is_none());
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees.releases.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    let record = captured_terminal_spawn_failure(&expected_id).unwrap();
    assert!(record.contains("kind=pty_spawn"));
    assert!(record.contains("error=openpty failed"));
}

#[tokio::test]
async fn test_agent_session_launch_prepare失敗時のrollbackのterminal削除失敗でも後続cleanupが走り一次原因を返す(
) {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store.clone()),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    *launch_gateway.fail_prepare.lock().unwrap() = true;
    let terminal = Arc::new(RecordingTerminal::default());
    *terminal.fail_delete.lock().unwrap() = true;
    let execution_trees = started_execution_trees();
    *execution_trees.release_failure.lock().unwrap() =
        Some(ExecutionTreeCacheReleaseError::Unavailable);
    let usecase = AgentSessionLaunchUsecase::new(
        sessions.clone(),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal.clone(),
        ),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health_usecase(),
        execution_trees.clone(),
    );

    let result = usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            rows: 24,
            cols: 80,
            caller_request_id: "rollback-delete-failure-1".to_string(),
        })
        .await;

    assert_eq!(
        result.unwrap_err(),
        AgentSessionLaunchUsecaseError::LaunchUnavailable
    );
    assert_eq!(*terminal.deletes.lock().unwrap(), 1);
    let expected_id = launch_gateway.cleanups.lock().unwrap()[0].clone();
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees.releases.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert!(sessions.find(&expected_id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_agent_session_launch_spawn失敗時のrollbackのterminal削除失敗でも後続cleanupが走り一次原因を返す(
) {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalAgentSessionRepository::new(store.clone()),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    *terminal.spawn_error.lock().unwrap() = Some(ProviderAgentTerminalSpawnError::PtySpawn {
        error: "openpty failed".to_string(),
    });
    *terminal.fail_delete.lock().unwrap() = true;
    let execution_trees = started_execution_trees();
    *execution_trees.release_failure.lock().unwrap() =
        Some(ExecutionTreeCacheReleaseError::Corrupt);
    let usecase = AgentSessionLaunchUsecase::new(
        sessions.clone(),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        provider_runtime(
            Arc::new(FixedAvailability {
                available: true,
                checks: Mutex::new(Vec::new()),
            }),
            launch_gateway.clone(),
            terminal.clone(),
        ),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health_usecase(),
        execution_trees.clone(),
    );

    let result = usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            rows: 24,
            cols: 80,
            caller_request_id: "rollback-spawn-delete-failure-1".to_string(),
        })
        .await;

    assert_eq!(
        result.unwrap_err(),
        AgentSessionLaunchUsecaseError::TerminalSpawn(ProviderAgentTerminalSpawnError::PtySpawn {
            error: "openpty failed".to_string()
        })
    );
    assert_eq!(*terminal.deletes.lock().unwrap(), 1);
    let expected_id = launch_gateway.cleanups.lock().unwrap()[0].clone();
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert_eq!(
        execution_trees.releases.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert!(sessions.find(&expected_id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_agent_session_launch_prepare失敗時のrollbackでgc失敗なら実行木を解放しない() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    *repository.remove_failure.lock().unwrap() = Some(AgentSessionRepositoryError::Unavailable);
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    *launch_gateway.fail_prepare.lock().unwrap() = true;
    let execution_trees = started_execution_trees();
    let usecase = launch_usecase_with_tree_registrar(
        repository.clone(),
        Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        }),
        launch_gateway,
        Arc::new(RecordingTerminal::default()),
        hook_health_usecase(),
        execution_trees.clone(),
    );

    let error = usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            rows: 24,
            cols: 80,
            caller_request_id: "rollback-gc-failure".to_string(),
        })
        .await
        .unwrap_err();

    assert_eq!(error, AgentSessionLaunchUsecaseError::LaunchUnavailable);
    assert!(repository.stored.lock().unwrap().is_some());
    assert!(execution_trees.releases.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_agent_session_launch_codexのhook_delivery未確認を警告しprocessを起動する() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        session_location("seed"),
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let availability = Arc::new(FixedAvailability {
        available: true,
        checks: Mutex::new(Vec::new()),
    });
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let hook_health = hook_health_usecase();
    let usecase = AgentSessionLaunchUsecase::new(
        Arc::new(AgentSessionUsecase::new(repository)),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        provider_runtime(availability, launch_gateway.clone(), terminal.clone()),
        Arc::new(FixedHistory { entries: Vec::new() }),
        hook_health.clone(),
        started_execution_trees(),
    );

    let launched = usecase
        .launch_standalone(AgentSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
            rows: 24,
            cols: 80,
            caller_request_id: "codex-launch-1".to_string(),
        })
        .await
        .unwrap();

    assert_eq!(
        launched.session().id(),
        launch_gateway.armed.lock().unwrap()[0]
            .scope()
            .agent_session_id()
    );
    assert_eq!(terminal.spawns.lock().unwrap().len(), 1);
    let warnings = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let warnings = hook_health.warnings().await.unwrap();
            if !warnings.is_empty() {
                break warnings;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        warnings[0].reason,
        ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed
    );
}
