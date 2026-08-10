use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use super::{
    ProviderAgentSessionHistoryResumeOutcome, ProviderAgentSessionHistoryResumeRequest,
    ProviderAgentSessionLaunchRequest, ProviderAgentSessionLaunchUsecase,
    ProviderAgentSessionLaunchUsecaseError, ProviderAgentSessionLifecycleUsecase,
    ProviderAgentSessionUsecase, ProviderAgentSessionUsecaseError,
    ProviderAgentWorkflowSessionLaunchRequest,
};
use crate::domain::agent_session::aggregates::{AgentSession, AgentSessionOrigin};
use crate::domain::agent_session::aggregates::{
    AgentSessionArchiveOutcome, AgentSessionLifecycle, AgentSessionProcessExitOutcome,
    AgentSessionRecoveryResult, ManagedPtyPresence,
};
use crate::domain::agent_session::repository::{
    ProviderAgentSessionRepository, ProviderAgentSessionRepositoryError,
    VersionedProviderAgentSession,
};
use crate::domain::agent_session::{
    PreparedProviderLaunch, ProviderAgentLaunchGateway, ProviderAgentLaunchGatewayError,
    ProviderAgentSessionHistoryGateway, ProviderAgentSessionHistoryGatewayError,
    ProviderAgentSessionHistoryMetadata, ProviderAgentTerminalGateway,
    ProviderAgentTerminalGatewayError, ProviderAvailabilityGateway, ProviderSessionLaunch,
};
use crate::domain::provider_lifecycle::{
    ArmedProviderLifecycle, ProviderHookHealth, ProviderHookHealthRepository,
    ProviderHookHealthRepositoryError, ProviderKind, ProviderLifecycleEventRepository,
    ProviderLifecycleRepositoryError, ProviderLifecycleUnavailableReason,
    ScopedProviderLifecycleEvent, VersionedProviderHookHealth,
};
use crate::domain::terminal_surface::{TerminalProcessLaunch, TerminalSurfaceOwner};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::provider_lifecycle::{ProviderHookHealthUsecase, ProviderLifecycleUsecase};

struct FailingSaveRepository {
    stored: Mutex<Option<VersionedProviderAgentSession>>,
    create_calls: AtomicUsize,
    atomic_create_calls: AtomicUsize,
    launch_lifecycle_events: Mutex<Vec<ScopedProviderLifecycleEvent>>,
}

impl FailingSaveRepository {
    fn new(mut session: AgentSession) -> Self {
        session.take_uncommitted_events();
        Self {
            stored: Mutex::new(Some(VersionedProviderAgentSession::restored(session, 1))),
            create_calls: AtomicUsize::new(0),
            atomic_create_calls: AtomicUsize::new(0),
            launch_lifecycle_events: Mutex::new(Vec::new()),
        }
    }

    fn snapshot(&self) -> VersionedProviderAgentSession {
        self.stored.lock().unwrap().clone().unwrap()
    }
}

#[async_trait::async_trait]
impl ProviderAgentSessionRepository for FailingSaveRepository {
    async fn create(
        &self,
        mut session: AgentSession,
        _caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        self.create_calls.fetch_add(1, Ordering::SeqCst);
        session.take_uncommitted_events();
        let saved = VersionedProviderAgentSession::restored(session, 1);
        *self.stored.lock().unwrap() = Some(saved.clone());
        Ok(saved)
    }

    async fn create_with_lifecycle_events(
        &self,
        mut session: AgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        _caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        self.atomic_create_calls.fetch_add(1, Ordering::SeqCst);
        self.launch_lifecycle_events
            .lock()
            .unwrap()
            .extend(lifecycle_events);
        session.take_uncommitted_events();
        let saved = VersionedProviderAgentSession::restored(session, 1);
        *self.stored.lock().unwrap() = Some(saved.clone());
        Ok(saved)
    }

    async fn find(
        &self,
        _session_id: &str,
    ) -> Result<Option<VersionedProviderAgentSession>, ProviderAgentSessionRepositoryError> {
        Ok(self.stored.lock().unwrap().clone())
    }

    async fn save(
        &self,
        _session: VersionedProviderAgentSession,
        _caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        Err(ProviderAgentSessionRepositoryError::Unavailable)
    }

    async fn remove(
        &self,
        _session: VersionedProviderAgentSession,
        _authorization: crate::domain::agent_session::aggregates::AgentSessionRemovalAuthorization,
        _caller_request_id: &str,
    ) -> Result<(), ProviderAgentSessionRepositoryError> {
        unreachable!()
    }
}

#[tokio::test]
async fn test_provider_agent_session_usecase選択されたproviderでstandalone_sessionを作成する() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let usecase = ProviderAgentSessionUsecase::new(repository);

    let created = usecase
        .create(
            "agent-session-1",
            WorkspaceIdentity::new("/repo"),
            "/repo/.worktrees/feature",
            ProviderKind::Codex,
            AgentSessionOrigin::Standalone,
            "create-request-1",
        )
        .await
        .unwrap();

    assert_eq!(created.session().provider(), ProviderKind::Codex);
    assert_eq!(created.session().origin(), &AgentSessionOrigin::Standalone);
    assert_eq!(created.revision(), 1);
}

#[tokio::test]
async fn test_provider_agent_session_usecase永続化失敗で共有状態を進めない() {
    let session = AgentSession::create(
        "agent-session-1",
        WorkspaceIdentity::new("/repo"),
        "/repo/.worktrees/feature",
        ProviderKind::Codex,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(session));
    let usecase = ProviderAgentSessionUsecase::new(repository.clone());

    let result = usecase
        .associate_provider_session(
            "agent-session-1",
            "provider-session-1",
            None,
            "associate-request-1",
        )
        .await;

    assert_eq!(
        result.unwrap_err(),
        ProviderAgentSessionUsecaseError::Unavailable
    );
    let unchanged = repository.snapshot();
    assert_eq!(unchanged.revision(), 1);
    assert_eq!(unchanged.session().provider_session_id(), None);
}

struct FixedAvailability {
    available: bool,
    checks: Mutex<Vec<ProviderKind>>,
}

impl ProviderAvailabilityGateway for FixedAvailability {
    fn is_available(&self, provider: ProviderKind) -> bool {
        self.checks.lock().unwrap().push(provider);
        self.available
    }
}

struct PanicOnFirstCheckAvailability {
    checks: AtomicUsize,
}

impl ProviderAvailabilityGateway for PanicOnFirstCheckAvailability {
    fn is_available(&self, _provider: ProviderKind) -> bool {
        if self.checks.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("availability check panicked for the launch panic test");
        }
        true
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
}

#[derive(Default)]
struct RecordingLaunchGateway {
    armed: Mutex<Vec<ArmedProviderLifecycle>>,
    launches: Mutex<Vec<ProviderSessionLaunch>>,
    cleanups: Mutex<Vec<String>>,
    fail_prepare: Mutex<bool>,
}

impl ProviderAgentLaunchGateway for RecordingLaunchGateway {
    fn prepare(
        &self,
        armed: &ArmedProviderLifecycle,
        launch: ProviderSessionLaunch,
    ) -> Result<PreparedProviderLaunch, ProviderAgentLaunchGatewayError> {
        if *self.fail_prepare.lock().unwrap() {
            return Err(ProviderAgentLaunchGatewayError::Unavailable);
        }
        self.launches.lock().unwrap().push(launch);
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
    fail_spawn: Mutex<bool>,
    fail_delete: Mutex<bool>,
    deletes: Mutex<usize>,
}

struct BlockingLaunchTerminal {
    presence: Mutex<ManagedPtyPresence>,
    spawn_entered: Mutex<Option<mpsc::Sender<()>>>,
    spawn_release: Mutex<Option<mpsc::Receiver<()>>>,
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
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        self.spawn_entered
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .send(())
            .unwrap();
        self.spawn_release
            .lock()
            .unwrap()
            .take()
            .unwrap()
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
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
        if *self.fail_spawn.lock().unwrap() {
            return Err(ProviderAgentTerminalGatewayError::Unavailable);
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
) -> ProviderAgentSessionLaunchUsecase {
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
) -> ProviderAgentSessionLaunchUsecase {
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(
            crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
        ),
        Arc::new(RecordingLifecycleEvents::default()),
    ));
    ProviderAgentSessionLaunchUsecase::new(
        Arc::new(ProviderAgentSessionUsecase::new(repository)),
        lifecycle,
        availability,
        launch_gateway,
        terminal,
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health,
    )
}

struct FixedHistory {
    entries: Vec<ProviderAgentSessionHistoryMetadata>,
}

#[async_trait::async_trait]
impl ProviderAgentSessionHistoryGateway for FixedHistory {
    async fn list_metadata(
        &self,
        provider: ProviderKind,
        worktree_path: &str,
        limit: usize,
    ) -> Result<Vec<ProviderAgentSessionHistoryMetadata>, ProviderAgentSessionHistoryGatewayError>
    {
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
async fn test_provider_agent_session_launch_利用可能な選択providerをterminal_root_processへ接続する(
) {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
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
    let usecase = launch_usecase(
        repository,
        availability.clone(),
        launch_gateway.clone(),
        terminal.clone(),
    );

    let launched = usecase
        .launch_standalone(ProviderAgentSessionLaunchRequest {
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
}

#[tokio::test]
async fn test_provider_agent_session_launch_session作成とlifecycle_armを一回のrepository操作で永続化する(
) {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let lifecycle_events = Arc::new(RecordingLifecycleEvents::default());
    let usecase = ProviderAgentSessionLaunchUsecase::new(
        Arc::new(ProviderAgentSessionUsecase::new(repository.clone())),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            lifecycle_events.clone(),
        )),
        Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        }),
        Arc::new(RecordingLaunchGateway::default()),
        Arc::new(RecordingTerminal::default()),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health_usecase(),
    );

    usecase
        .launch_standalone(ProviderAgentSessionLaunchRequest {
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
async fn test_provider_agent_session_launch_hookwarning保存完了を待たずpty起動済みsessionを返す() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
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
            .launch_standalone(ProviderAgentSessionLaunchRequest {
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
async fn test_provider_agent_session_launch_利用不可providerではsessionもptyも作らない() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
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
        .launch_standalone(ProviderAgentSessionLaunchRequest {
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
        ProviderAgentSessionLaunchUsecaseError::ProviderUnavailable
    );
    assert!(repository.stored.lock().unwrap().is_none());
    assert!(launch_gateway.armed.lock().unwrap().is_empty());
    assert!(terminal.spawns.lock().unwrap().is_empty());
}

fn idempotent_launch_request(caller_request_id: &str) -> ProviderAgentSessionLaunchRequest {
    ProviderAgentSessionLaunchRequest {
        workspace: WorkspaceIdentity::new("/repo"),
        worktree_path: "/repo/.worktrees/feature".to_string(),
        provider: ProviderKind::Claude,
        rows: 24,
        cols: 80,
        caller_request_id: caller_request_id.to_string(),
    }
}

#[test]
fn test_provider_agent_session_launch_id発行をcontrollerとworkflow_gatewayに分散しない() {
    let controller = include_str!("../../adaptor/controller/command/agent_session/provider_tui.rs");
    let workflow_gateway = include_str!("../../adaptor/gateway/workflow/node_session_boundary.rs");

    assert!(!controller.contains("Uuid::new_v4"));
    assert!(!workflow_gateway.contains("provider-agent-session-{nonce}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_provider_agent_session_launch_同一request_idの並行呼び出しはsessionを一度だけ作成し同じ結果を返す(
) {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
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
        deletes: Mutex::new(0),
    });
    let usecase = Arc::new(ProviderAgentSessionLaunchUsecase::new(
        Arc::new(ProviderAgentSessionUsecase::new(repository.clone())),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        }),
        Arc::new(RecordingLaunchGateway::default()),
        terminal,
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health_usecase(),
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
    assert!(first.starts_with("provider-agent-session-"));
    assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_provider_agent_session_launch_完了済みrequest_id再送は再作成せず同じsession識別子を返す(
) {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
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
    assert!(first.starts_with("provider-agent-session-"));
    assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 1);
    assert_eq!(terminal.spawns.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_provider_agent_session_launch_異なるrequest_idは別のsessionを作成する() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
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
    assert!(first.starts_with("provider-agent-session-"));
    assert!(second.starts_with("provider-agent-session-"));
    assert_eq!(repository.atomic_create_calls.load(Ordering::SeqCst), 2);
    assert_eq!(terminal.spawns.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn test_provider_agent_session_launch_失敗結果も記録し同一request_id再送へ同じ失敗を返す() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
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
        ProviderAgentSessionLaunchUsecaseError::ProviderUnavailable
    );
    assert_eq!(
        replay.unwrap_err(),
        ProviderAgentSessionLaunchUsecaseError::ProviderUnavailable
    );
    assert_eq!(availability.checks.lock().unwrap().len(), 1);
    assert!(repository.stored.lock().unwrap().is_none());
    assert!(terminal.spawns.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_provider_agent_session_launch_起動panic後はin_flightに残さず同一request_id再送へ同じ失敗を返す(
) {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
    )
    .unwrap();
    let repository = Arc::new(FailingSaveRepository::new(seed));
    *repository.stored.lock().unwrap() = None;
    let availability = Arc::new(PanicOnFirstCheckAvailability {
        checks: AtomicUsize::new(0),
    });
    let usecase = Arc::new(ProviderAgentSessionLaunchUsecase::new(
        Arc::new(ProviderAgentSessionUsecase::new(repository.clone())),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        availability.clone(),
        Arc::new(RecordingLaunchGateway::default()),
        Arc::new(RecordingTerminal::default()),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health_usecase(),
    ));

    let first = Arc::clone(&usecase)
        .launch_standalone_idempotent(idempotent_launch_request("request-panic"))
        .await;
    let replay = Arc::clone(&usecase)
        .launch_standalone_idempotent(idempotent_launch_request("request-panic"))
        .await;

    assert_eq!(
        first.unwrap_err(),
        ProviderAgentSessionLaunchUsecaseError::Corrupt
    );
    assert_eq!(
        replay.unwrap_err(),
        ProviderAgentSessionLaunchUsecaseError::Corrupt
    );
    assert_eq!(usecase.standalone_in_flight_request_count().await, 0);
    assert_eq!(availability.checks.load(Ordering::SeqCst), 1);
    assert!(repository.stored.lock().unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_provider_agent_session_launch_pty起動中のsessionをgcしない() {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalProviderAgentSessionRepository::new(
            store.clone() as Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
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
        deletes: Mutex::new(0),
    });
    let launches = Arc::new(RecordingLaunchGateway::default());
    let hook_health = hook_health_usecase();
    let launch = Arc::new(ProviderAgentSessionLaunchUsecase::new(
        sessions.clone(),
        provider_lifecycle.clone(),
        Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        }),
        launches.clone(),
        terminal.clone(),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health.clone(),
    ));
    struct NoopChangeNotifier;
    impl crate::usecase::agent_session::ProviderAgentSessionChangeNotifier for NoopChangeNotifier {
        fn provider_agent_session_changed(&self, _worktree_path: &str) {}
    }
    let lifecycle = Arc::new(ProviderAgentSessionLifecycleUsecase::new(
        sessions.clone(),
        provider_lifecycle,
        launches.clone(),
        terminal.clone(),
        hook_health,
        Arc::new(NoopChangeNotifier),
    ));

    let launching = tokio::spawn(async move {
        launch
            .launch_standalone(ProviderAgentSessionLaunchRequest {
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
        super::ProviderAgentSessionGarbageCollectionOutcome::Retained
    );
    assert!(sessions.find(&expected_id).await.unwrap().is_some());
    assert_eq!(*terminal.deletes.lock().unwrap(), 0);
}

#[tokio::test]
async fn test_provider_agent_session_history_resumeは新しいsessionを作り失敗時もidを保持する() {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalProviderAgentSessionRepository::new(
            store.clone() as Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
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
    *terminal.fail_spawn.lock().unwrap() = true;
    let hook_health = hook_health_usecase();
    let usecase = ProviderAgentSessionLaunchUsecase::new(
        sessions.clone(),
        lifecycle,
        availability,
        launch_gateway.clone(),
        terminal,
        Arc::new(FixedHistory {
            entries: vec![ProviderAgentSessionHistoryMetadata {
                provider: ProviderKind::Codex,
                provider_session_id: "provider-history-1".to_string(),
                worktree_path: "/repo/worktree".to_string(),
                updated_at_ms: 10,
            }],
        }),
        hook_health,
    );

    let outcome = usecase
        .resume_history(ProviderAgentSessionHistoryResumeRequest {
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
        ProviderAgentSessionHistoryResumeOutcome::Paused(session) => {
            session.session().id().to_string()
        }
        ProviderAgentSessionHistoryResumeOutcome::Open(_) => panic!("expected paused session"),
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
        &[expected_id]
    );
}

#[tokio::test]
async fn test_provider_agent_session_history_resume_lifecycle準備失敗でもpausedへ収束する() {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalProviderAgentSessionRepository::new(
            store.clone() as Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
    )));
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(
            crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
        ),
        Arc::new(FailingFirstLifecycleEvents::default()),
    ));
    let usecase = ProviderAgentSessionLaunchUsecase::new(
        sessions.clone(),
        lifecycle,
        Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        }),
        Arc::new(RecordingLaunchGateway::default()),
        Arc::new(RecordingTerminal::default()),
        Arc::new(FixedHistory {
            entries: vec![ProviderAgentSessionHistoryMetadata {
                provider: ProviderKind::Codex,
                provider_session_id: "provider-history-arm-failure".to_string(),
                worktree_path: "/repo/worktree".to_string(),
                updated_at_ms: 10,
            }],
        }),
        hook_health_usecase(),
    );

    let outcome = usecase
        .resume_history(ProviderAgentSessionHistoryResumeRequest {
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
        ProviderAgentSessionHistoryResumeOutcome::Paused(session) => {
            session.session().id().to_string()
        }
        ProviderAgentSessionHistoryResumeOutcome::Open(_) => panic!("expected paused session"),
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
    ProviderAgentSessionUsecase,
) {
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let repository = Arc::new(
        crate::adaptor::gateway::agent_session::LocalProviderAgentSessionRepository::new(
            store.clone() as Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
    );
    (store, ProviderAgentSessionUsecase::new(repository))
}

#[tokio::test]
async fn test_provider_agent_session_usecase_process_exit_resume_archive_deleteを永続化する() {
    let directory = tempfile::tempdir().unwrap();
    let (_store, usecase) = durable_usecase(&directory);
    usecase
        .create(
            "agent-session-1",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            AgentSessionOrigin::Standalone,
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
async fn test_provider_agent_session_usecase_id不明archiveは確認後deleteへ縮退する() {
    let directory = tempfile::tempdir().unwrap();
    let (_store, usecase) = durable_usecase(&directory);
    usecase
        .create(
            "agent-session-unknown",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            AgentSessionOrigin::Standalone,
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
async fn test_provider_agent_session_usecase_gcはpty不在確定時だけunknown_idを削除する() {
    let directory = tempfile::tempdir().unwrap();
    let (_store, usecase) = durable_usecase(&directory);
    usecase
        .create(
            "agent-session-gc",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            AgentSessionOrigin::Standalone,
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
        ProviderAgentSessionUsecaseError::InvalidOperation
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
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalProviderAgentSessionRepository::new(
            store.clone() as Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    let usecase = ProviderAgentSessionLaunchUsecase::new(
        sessions,
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        }),
        launch_gateway.clone(),
        terminal.clone(),
        Arc::new(FixedHistory { entries: Vec::new() }),
        hook_health_usecase(),
    );

    let launched = usecase
        .prepare_workflow_node(ProviderAgentWorkflowSessionLaunchRequest {
            workspace: WorkspaceIdentity::new("/repo"),
            worktree_path: "/repo/worktree".to_string(),
            provider: ProviderKind::Codex,
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
        launched.session().origin(),
        &AgentSessionOrigin::workflow_node("workflow-1", "node-1").unwrap()
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
    assert_eq!(terminal.spawns.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn test_provider_agent_session_launch_spawn失敗時はsessionとlaunch資源をrollbackする() {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalProviderAgentSessionRepository::new(
            store.clone() as Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    let terminal = Arc::new(RecordingTerminal::default());
    *terminal.fail_spawn.lock().unwrap() = true;
    let hook_health = hook_health_usecase();
    let usecase = ProviderAgentSessionLaunchUsecase::new(
        sessions.clone(),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        }),
        launch_gateway.clone(),
        terminal,
        Arc::new(FixedHistory { entries: Vec::new() }),
        hook_health,
    );

    let result = usecase
        .launch_standalone(ProviderAgentSessionLaunchRequest {
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
        ProviderAgentSessionLaunchUsecaseError::TerminalUnavailable
    );
    let expected_id = launch_gateway.cleanups.lock().unwrap()[0].clone();
    assert!(sessions.find(&expected_id).await.unwrap().is_none());
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        &[expected_id]
    );
}

#[tokio::test]
async fn test_provider_agent_session_launch_rollbackのterminal削除失敗でも後続cleanupが走り一次原因を返す(
) {
    let directory = tempfile::tempdir().unwrap();
    let store = crate::adaptor::gateway::local_event_store::LocalEventStore::open(
        crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
            directory.path().to_path_buf(),
        ),
    )
    .unwrap();
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(Arc::new(
        crate::adaptor::gateway::agent_session::LocalProviderAgentSessionRepository::new(
            store.clone() as Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
    )));
    let launch_gateway = Arc::new(RecordingLaunchGateway::default());
    *launch_gateway.fail_prepare.lock().unwrap() = true;
    let terminal = Arc::new(RecordingTerminal::default());
    *terminal.fail_delete.lock().unwrap() = true;
    let usecase = ProviderAgentSessionLaunchUsecase::new(
        sessions.clone(),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        Arc::new(FixedAvailability {
            available: true,
            checks: Mutex::new(Vec::new()),
        }),
        launch_gateway.clone(),
        terminal.clone(),
        Arc::new(FixedHistory {
            entries: Vec::new(),
        }),
        hook_health_usecase(),
    );

    let result = usecase
        .launch_standalone(ProviderAgentSessionLaunchRequest {
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
        ProviderAgentSessionLaunchUsecaseError::LaunchUnavailable
    );
    assert_eq!(*terminal.deletes.lock().unwrap(), 1);
    let expected_id = launch_gateway.cleanups.lock().unwrap()[0].clone();
    assert_eq!(
        launch_gateway.cleanups.lock().unwrap().as_slice(),
        std::slice::from_ref(&expected_id)
    );
    assert!(sessions.find(&expected_id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_provider_agent_session_launch_codexのhook_delivery未確認を警告しprocessを起動する() {
    let seed = AgentSession::create(
        "seed",
        WorkspaceIdentity::new("/seed"),
        "/seed",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
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
    let usecase = ProviderAgentSessionLaunchUsecase::new(
        Arc::new(ProviderAgentSessionUsecase::new(repository)),
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(
                crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway,
            ),
            Arc::new(RecordingLifecycleEvents::default()),
        )),
        availability,
        launch_gateway.clone(),
        terminal.clone(),
        Arc::new(FixedHistory { entries: Vec::new() }),
        hook_health.clone(),
    );

    let launched = usecase
        .launch_standalone(ProviderAgentSessionLaunchRequest {
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
