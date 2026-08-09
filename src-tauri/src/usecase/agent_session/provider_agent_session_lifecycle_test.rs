use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use super::{
    ProviderAgentSessionLifecycleUsecase, ProviderAgentSessionOpenOutcome,
    ProviderAgentSessionUsecase,
};
use crate::adaptor::gateway::agent_session::LocalProviderAgentSessionRepository;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway;
use crate::domain::agent_session::aggregates::{
    AgentSessionArchiveOutcome, AgentSessionLifecycle, AgentSessionOrigin, ManagedPtyPresence,
};
use crate::domain::agent_session::repository::{
    ProviderAgentSessionRepository, ProviderAgentSessionRepositoryError,
    VersionedProviderAgentSession,
};
use crate::domain::agent_session::{
    PreparedProviderLaunch, ProviderAgentLaunchGateway, ProviderAgentLaunchGatewayError,
    ProviderAgentTerminalGateway, ProviderAgentTerminalGatewayError, ProviderSessionLaunch,
};
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::provider_lifecycle::{
    ArmedProviderLifecycle, ProviderHookHealth, ProviderHookHealthRepository,
    ProviderHookHealthRepositoryError, ProviderKind, ProviderLifecycleEventRepository,
    ProviderLifecycleRepositoryError, ProviderLifecycleScope, ProviderLifecycleSlotId,
    ScopedProviderLifecycleEvent, VersionedProviderHookHealth,
};
use crate::domain::terminal_surface::{TerminalProcessLaunch, TerminalSurfaceOwner};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::provider_lifecycle::{ProviderHookHealthUsecase, ProviderLifecycleUsecase};

#[derive(Default)]
struct RecordingChangeNotifier {
    notified: Mutex<Vec<String>>,
}

impl crate::usecase::agent_session::ProviderAgentSessionChangeNotifier for RecordingChangeNotifier {
    fn provider_agent_session_changed(&self, worktree_path: &str) {
        self.notified
            .lock()
            .unwrap()
            .push(worktree_path.to_string());
    }
}

#[derive(Default)]
struct NoopLifecycleEvents;

#[async_trait::async_trait]
impl ProviderLifecycleEventRepository for NoopLifecycleEvents {
    async fn append(
        &self,
        _events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleRepositoryError> {
        Ok(())
    }
}

#[derive(Default)]
struct RecordingResumeLaunches {
    launches: Mutex<Vec<ProviderSessionLaunch>>,
    cleanups: Mutex<Vec<String>>,
    armed: Mutex<Vec<ArmedProviderLifecycle>>,
    initial_hook_warning:
        Mutex<Option<crate::domain::provider_lifecycle::ProviderLifecycleUnavailableReason>>,
}

impl ProviderAgentLaunchGateway for RecordingResumeLaunches {
    fn prepare(
        &self,
        armed: &ArmedProviderLifecycle,
        launch: ProviderSessionLaunch,
    ) -> Result<PreparedProviderLaunch, ProviderAgentLaunchGatewayError> {
        self.armed.lock().unwrap().push(armed.clone());
        self.launches.lock().unwrap().push(launch);
        Ok(PreparedProviderLaunch::new(
            TerminalProcessLaunch::new("provider", Vec::new(), Vec::new()).unwrap(),
            None,
            *self.initial_hook_warning.lock().unwrap(),
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
        let revision =
            health.revision() + health.health_mut().take_uncommitted_events().len() as u64;
        let saved = VersionedProviderHookHealth::restored(health.into_health(), revision);
        self.stored
            .lock()
            .unwrap()
            .insert(saved.health().provider(), saved.clone());
        Ok(saved)
    }
}

struct LifecycleTerminal {
    presence: Mutex<ManagedPtyPresence>,
    runtime_generation: Mutex<u64>,
    fail_spawn: Mutex<bool>,
    spawn_count: Mutex<usize>,
    first_spawn_entered: Mutex<Option<mpsc::Sender<()>>>,
    first_spawn_release: Mutex<Option<mpsc::Receiver<()>>>,
    first_delete_entered: Mutex<Option<mpsc::Sender<()>>>,
    first_delete_release: Mutex<Option<mpsc::Receiver<()>>>,
    stops: Mutex<Vec<TerminalSurfaceOwner>>,
    deletes: Mutex<Vec<TerminalSurfaceOwner>>,
}

struct SaveFailingAgentSessionRepository {
    inner: Arc<LocalProviderAgentSessionRepository>,
}

#[async_trait::async_trait]
impl ProviderAgentSessionRepository for SaveFailingAgentSessionRepository {
    async fn create(
        &self,
        session: crate::domain::agent_session::aggregates::AgentSession,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        self.inner.create(session, caller_request_id).await
    }

    async fn create_with_lifecycle_events(
        &self,
        session: crate::domain::agent_session::aggregates::AgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedProviderAgentSession, ProviderAgentSessionRepositoryError> {
        self.inner
            .create_with_lifecycle_events(session, lifecycle_events, caller_request_id)
            .await
    }

    async fn find(
        &self,
        session_id: &str,
    ) -> Result<Option<VersionedProviderAgentSession>, ProviderAgentSessionRepositoryError> {
        self.inner.find(session_id).await
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
        session: VersionedProviderAgentSession,
        authorization: crate::domain::agent_session::aggregates::AgentSessionRemovalAuthorization,
        caller_request_id: &str,
    ) -> Result<(), ProviderAgentSessionRepositoryError> {
        self.inner
            .remove(session, authorization, caller_request_id)
            .await
    }
}

impl LifecycleTerminal {
    fn new(presence: ManagedPtyPresence) -> Self {
        Self {
            presence: Mutex::new(presence),
            runtime_generation: Mutex::new(1),
            fail_spawn: Mutex::new(false),
            spawn_count: Mutex::new(0),
            first_spawn_entered: Mutex::new(None),
            first_spawn_release: Mutex::new(None),
            first_delete_entered: Mutex::new(None),
            first_delete_release: Mutex::new(None),
            stops: Mutex::new(Vec::new()),
            deletes: Mutex::new(Vec::new()),
        }
    }
}

impl ProviderAgentTerminalGateway for LifecycleTerminal {
    fn spawn(
        &self,
        _owner: TerminalSurfaceOwner,
        _worktree_path: &str,
        _process: TerminalProcessLaunch,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        let should_block = {
            let mut spawn_count = self.spawn_count.lock().unwrap();
            *spawn_count += 1;
            *spawn_count == 1
        };
        if should_block {
            if let Some(sender) = self.first_spawn_entered.lock().unwrap().take() {
                sender.send(()).unwrap();
            }
            if let Some(receiver) = self.first_spawn_release.lock().unwrap().take() {
                receiver.recv_timeout(Duration::from_secs(1)).unwrap();
            }
        }
        if *self.fail_spawn.lock().unwrap() {
            return Err(ProviderAgentTerminalGatewayError::Unavailable);
        }
        *self.runtime_generation.lock().unwrap() += 1;
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
        owner: &TerminalSurfaceOwner,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        self.stops.lock().unwrap().push(owner.clone());
        *self.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
        Ok(())
    }

    fn delete(
        &self,
        owner: &TerminalSurfaceOwner,
    ) -> Result<(), ProviderAgentTerminalGatewayError> {
        if let Some(sender) = self.first_delete_entered.lock().unwrap().take() {
            sender.send(()).unwrap();
        }
        if let Some(receiver) = self.first_delete_release.lock().unwrap().take() {
            receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        }
        self.deletes.lock().unwrap().push(owner.clone());
        *self.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
        Ok(())
    }

    fn is_current_runtime_generation(
        &self,
        _owner: &TerminalSurfaceOwner,
        runtime_generation: u64,
    ) -> Result<bool, ProviderAgentTerminalGatewayError> {
        Ok(*self.runtime_generation.lock().unwrap() == runtime_generation)
    }
}

struct LifecycleTestContext {
    _directory: tempfile::TempDir,
    sessions: Arc<ProviderAgentSessionUsecase>,
    lifecycle: ProviderAgentSessionLifecycleUsecase,
    launches: Arc<RecordingResumeLaunches>,
    terminal: Arc<LifecycleTerminal>,
    provider_lifecycle: Arc<ProviderLifecycleUsecase>,
    hook_health: Arc<ProviderHookHealthUsecase>,
    change_notifier: Arc<RecordingChangeNotifier>,
}

fn setup() -> LifecycleTestContext {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(Arc::new(
        LocalProviderAgentSessionRepository::new(
            store.clone() as Arc<dyn LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
    )));
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(NoopLifecycleEvents),
    ));
    let launches = Arc::new(RecordingResumeLaunches::default());
    let terminal = Arc::new(LifecycleTerminal::new(ManagedPtyPresence::Live));
    let hook_health = Arc::new(ProviderHookHealthUsecase::new(Arc::new(
        MemoryHookHealthRepository::default(),
    )));
    let change_notifier = Arc::new(RecordingChangeNotifier::default());
    let usecase = ProviderAgentSessionLifecycleUsecase::new(
        sessions.clone(),
        lifecycle.clone(),
        launches.clone(),
        terminal.clone(),
        hook_health.clone(),
        change_notifier.clone(),
    );
    LifecycleTestContext {
        _directory: directory,
        sessions,
        lifecycle: usecase,
        launches,
        terminal,
        provider_lifecycle: lifecycle,
        hook_health,
        change_notifier,
    }
}

#[tokio::test]
async fn test_provider_agent_session_lifecycle_exit_resume_archive_restore_deleteを接続する() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        launches,
        terminal,
        provider_lifecycle,
        ..
    } = setup();
    sessions
        .create(
            "agent-1",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            AgentSessionOrigin::Standalone,
            "create-1",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session("agent-1", "provider-1", None, "associate-1")
        .await
        .unwrap();

    *terminal.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
    lifecycle
        .observe_process_exit("agent-1", 1, Some(0), "exit-1")
        .await
        .unwrap();
    assert_eq!(
        sessions
            .find("agent-1")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Paused
    );
    assert_eq!(
        lifecycle
            .resume("agent-1", 24, 80, "resume-1")
            .await
            .unwrap(),
        ProviderAgentSessionOpenOutcome::Resumed
    );
    assert_eq!(
        launches.launches.lock().unwrap().as_slice(),
        &[ProviderSessionLaunch::resume("provider-1").unwrap()]
    );

    assert_eq!(
        lifecycle.archive("agent-1", "archive-1").await.unwrap(),
        AgentSessionArchiveOutcome::Archived
    );
    assert_eq!(terminal.stops.lock().unwrap().len(), 1);
    assert_eq!(
        launches.cleanups.lock().unwrap().as_slice(),
        &["agent-1", "agent-1"]
    );
    let archived_binding = launches.armed.lock().unwrap()[0].clone();
    assert_eq!(
        provider_lifecycle
            .receive(
                archived_binding.slot_id(),
                archived_binding.capability(),
                crate::domain::provider_lifecycle::ProviderLifecycleSignal::session_started(
                    archived_binding.binding_id(),
                    archived_binding.provider(),
                    archived_binding.scope().clone(),
                    "provider-1",
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap(),
        crate::domain::provider_lifecycle::ProviderLifecycleIngressResult::Rejected(
            crate::domain::provider_lifecycle::ProviderLifecycleRejection::BindingNotActive,
        )
    );
    *terminal.fail_spawn.lock().unwrap() = true;
    assert!(lifecycle
        .restore("agent-1", 24, 80, "restore-fail")
        .await
        .is_err());
    assert_eq!(
        sessions
            .find("agent-1")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Archived
    );
    assert_eq!(
        launches.cleanups.lock().unwrap().as_slice(),
        &["agent-1", "agent-1", "agent-1"]
    );
    *terminal.fail_spawn.lock().unwrap() = false;
    assert_eq!(
        lifecycle
            .restore("agent-1", 24, 80, "restore-1")
            .await
            .unwrap(),
        ProviderAgentSessionOpenOutcome::Restored
    );
    lifecycle.archive("agent-1", "archive-2").await.unwrap();
    lifecycle.delete("agent-1", "delete-1").await.unwrap();
    assert!(sessions.find("agent-1").await.unwrap().is_none());
    assert_eq!(terminal.deletes.lock().unwrap().len(), 1);
    assert_eq!(
        launches.cleanups.lock().unwrap().as_slice(),
        &["agent-1", "agent-1", "agent-1", "agent-1", "agent-1"]
    );
}

#[tokio::test]
async fn test_provider_agent_session_lifecycle_unknown_idのprocess_exitをgcする() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        launches,
        terminal,
        provider_lifecycle,
        hook_health,
        ..
    } = setup();
    sessions
        .create(
            "agent-gc",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            AgentSessionOrigin::Standalone,
            "create-gc",
        )
        .await
        .unwrap();
    provider_lifecycle
        .arm(
            crate::domain::provider_lifecycle::ProviderLifecycleSlotId::new("launch-gc").unwrap(),
            ProviderKind::Codex,
            crate::domain::provider_lifecycle::ProviderLifecycleScope::new("agent-gc").unwrap(),
        )
        .await
        .unwrap();
    hook_health
        .record_launch(ProviderKind::Codex, "launch-gc", "launch-gc-request")
        .await
        .unwrap();
    *terminal.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;

    lifecycle
        .observe_process_exit("agent-gc", 1, Some(0), "exit-gc")
        .await
        .unwrap();

    assert!(sessions.find("agent-gc").await.unwrap().is_none());
    assert_eq!(terminal.deletes.lock().unwrap().len(), 1);
    assert_eq!(launches.cleanups.lock().unwrap().as_slice(), &["agent-gc"]);
    let warnings = hook_health.warnings().await.unwrap();
    assert_eq!(warnings.len(), 1);
    assert_eq!(
        warnings[0].reason,
        crate::domain::provider_lifecycle::ProviderLifecycleUnavailableReason::SessionStartDeadlineExceeded
    );
}

#[tokio::test]
async fn test_provider_agent_session_open_liveと生死不明では既存状態を破壊しない() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        launches,
        terminal,
        ..
    } = setup();
    sessions
        .create(
            "agent-open",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            AgentSessionOrigin::Standalone,
            "create-open",
        )
        .await
        .unwrap();

    assert_eq!(
        lifecycle
            .open("agent-open", 24, 80, "open-live")
            .await
            .unwrap(),
        ProviderAgentSessionOpenOutcome::Attached
    );
    *terminal.presence.lock().unwrap() = ManagedPtyPresence::Unknown;
    assert_eq!(
        lifecycle
            .open("agent-open", 24, 80, "open-unknown")
            .await
            .unwrap(),
        ProviderAgentSessionOpenOutcome::Indeterminate
    );
    assert!(sessions.find("agent-open").await.unwrap().is_some());
    assert!(launches.launches.lock().unwrap().is_empty());
    assert!(terminal.deletes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_provider_agent_session_open_known_idを自動resumeし失敗時はpausedにする() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        launches,
        terminal,
        ..
    } = setup();
    sessions
        .create(
            "agent-known",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            AgentSessionOrigin::Standalone,
            "create-known",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session("agent-known", "provider-known", None, "associate-known")
        .await
        .unwrap();
    *terminal.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
    *terminal.fail_spawn.lock().unwrap() = true;

    assert_eq!(
        lifecycle
            .open("agent-known", 24, 80, "open-known")
            .await
            .unwrap(),
        ProviderAgentSessionOpenOutcome::Paused
    );
    assert_eq!(
        sessions
            .find("agent-known")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Paused
    );
    assert_eq!(
        launches.launches.lock().unwrap().as_slice(),
        &[ProviderSessionLaunch::resume("provider-known").unwrap()]
    );
    assert!(terminal.deletes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_provider_agent_session_open_pausedは明示resumeを待ちunknown_idはgcする() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        launches,
        terminal,
        ..
    } = setup();
    sessions
        .create(
            "agent-paused",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            AgentSessionOrigin::Standalone,
            "create-paused",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session("agent-paused", "provider-paused", None, "associate-paused")
        .await
        .unwrap();
    *terminal.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
    lifecycle
        .observe_process_exit("agent-paused", 1, Some(0), "exit-paused")
        .await
        .unwrap();

    assert_eq!(
        lifecycle
            .open("agent-paused", 24, 80, "open-paused")
            .await
            .unwrap(),
        ProviderAgentSessionOpenOutcome::Paused
    );
    assert!(launches.launches.lock().unwrap().is_empty());

    sessions
        .create(
            "agent-orphan",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            AgentSessionOrigin::Standalone,
            "create-orphan",
        )
        .await
        .unwrap();
    assert_eq!(
        lifecycle
            .open("agent-orphan", 24, 80, "open-orphan")
            .await
            .unwrap(),
        ProviderAgentSessionOpenOutcome::GarbageCollected
    );
    assert!(sessions.find("agent-orphan").await.unwrap().is_none());
}

#[tokio::test]
async fn test_provider_agent_session_gc再照合は確定不在かつunknown_idだけを削除する() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        terminal,
        ..
    } = setup();
    sessions
        .create(
            "agent-reconcile",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            AgentSessionOrigin::Standalone,
            "create-reconcile",
        )
        .await
        .unwrap();

    *terminal.presence.lock().unwrap() = ManagedPtyPresence::Unknown;
    assert_eq!(
        lifecycle
            .reconcile_garbage_collection("agent-reconcile", "reconcile-unknown")
            .await
            .unwrap(),
        super::ProviderAgentSessionGarbageCollectionOutcome::Retained
    );
    assert!(sessions.find("agent-reconcile").await.unwrap().is_some());

    *terminal.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
    assert_eq!(
        lifecycle
            .reconcile_garbage_collection("agent-reconcile", "reconcile-absent")
            .await
            .unwrap(),
        super::ProviderAgentSessionGarbageCollectionOutcome::GarbageCollected
    );
    assert!(sessions.find("agent-reconcile").await.unwrap().is_none());
}

#[tokio::test]
async fn test_provider_agent_session_resume_codexでも既知の配送失敗がなければ警告しない() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        terminal,
        hook_health,
        ..
    } = setup();
    sessions
        .create(
            "agent-hook",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            AgentSessionOrigin::Standalone,
            "create-hook",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session("agent-hook", "provider-hook", None, "associate-hook")
        .await
        .unwrap();
    *terminal.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
    lifecycle
        .observe_process_exit("agent-hook", 1, Some(0), "exit-hook")
        .await
        .unwrap();

    assert_eq!(
        lifecycle
            .resume("agent-hook", 24, 80, "resume-hook")
            .await
            .unwrap(),
        ProviderAgentSessionOpenOutcome::Resumed
    );
    assert!(hook_health.warnings().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_provider_agent_session_resume_spawn失敗時は未起動launchのhook警告を残さない() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        launches,
        terminal,
        hook_health,
        ..
    } = setup();
    sessions
        .create(
            "agent-hook-spawn-failure",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            AgentSessionOrigin::Standalone,
            "create-hook-spawn-failure",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session(
            "agent-hook-spawn-failure",
            "provider-hook-spawn-failure",
            None,
            "associate-hook-spawn-failure",
        )
        .await
        .unwrap();
    *terminal.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
    lifecycle
        .observe_process_exit(
            "agent-hook-spawn-failure",
            1,
            Some(0),
            "exit-hook-spawn-failure",
        )
        .await
        .unwrap();
    *launches.initial_hook_warning.lock().unwrap() = Some(
        crate::domain::provider_lifecycle::ProviderLifecycleUnavailableReason::CodexHookDeliveryUnconfirmed,
    );
    *terminal.fail_spawn.lock().unwrap() = true;

    assert_eq!(
        lifecycle
            .resume(
                "agent-hook-spawn-failure",
                24,
                80,
                "resume-hook-spawn-failure",
            )
            .await
            .unwrap_err(),
        super::ProviderAgentSessionLifecycleUsecaseError::TerminalUnavailable,
    );
    assert!(hook_health.warnings().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_provider_agent_session_resume状態保存失敗時は起動済みprocessを停止する() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let repository = Arc::new(LocalProviderAgentSessionRepository::new(
        store.clone() as Arc<dyn LocalEventTransactionRepository>,
        store.installation_id().to_string(),
    ));
    let seed = ProviderAgentSessionUsecase::new(repository.clone());
    seed.create(
        "agent-save-failure",
        WorkspaceIdentity::new("/repo"),
        "/repo/worktree",
        ProviderKind::Claude,
        AgentSessionOrigin::Standalone,
        "create-save-failure",
    )
    .await
    .unwrap();
    seed.associate_provider_session(
        "agent-save-failure",
        "provider-save-failure",
        None,
        "associate-save-failure",
    )
    .await
    .unwrap();
    seed.observe_process_exit("agent-save-failure", Some(0), "pause-save-failure")
        .await
        .unwrap();
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(Arc::new(
        SaveFailingAgentSessionRepository { inner: repository },
    )));
    let provider_lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(NoopLifecycleEvents),
    ));
    let launches = Arc::new(RecordingResumeLaunches::default());
    let terminal = Arc::new(LifecycleTerminal::new(ManagedPtyPresence::ConfirmedAbsent));
    let hook_health = Arc::new(ProviderHookHealthUsecase::new(Arc::new(
        MemoryHookHealthRepository::default(),
    )));
    let lifecycle = ProviderAgentSessionLifecycleUsecase::new(
        sessions,
        provider_lifecycle,
        launches.clone(),
        terminal.clone(),
        hook_health,
        Arc::new(RecordingChangeNotifier::default()),
    );

    assert_eq!(
        lifecycle
            .resume("agent-save-failure", 24, 80, "resume-save-failure")
            .await
            .unwrap_err(),
        super::ProviderAgentSessionLifecycleUsecaseError::StorageUnavailable
    );
    assert_eq!(
        *terminal.presence.lock().unwrap(),
        ManagedPtyPresence::ConfirmedAbsent
    );
    assert_eq!(terminal.stops.lock().unwrap().len(), 1);
    assert_eq!(
        launches.cleanups.lock().unwrap().as_slice(),
        &["agent-save-failure"]
    );
}

#[tokio::test]
async fn test_provider_agent_session_resume_残存bindingを解放して単一launchにする() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        provider_lifecycle,
        ..
    } = setup();
    sessions
        .create(
            "agent-stale-binding",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            AgentSessionOrigin::Standalone,
            "create-stale-binding",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session(
            "agent-stale-binding",
            "provider-stale-binding",
            None,
            "associate-stale-binding",
        )
        .await
        .unwrap();
    sessions
        .observe_process_exit("agent-stale-binding", Some(0), "pause-stale-binding")
        .await
        .unwrap();
    let scope = ProviderLifecycleScope::new("agent-stale-binding").unwrap();
    let old_slot = ProviderLifecycleSlotId::new("old-slot").unwrap();
    provider_lifecycle
        .arm(old_slot.clone(), ProviderKind::Claude, scope.clone())
        .await
        .unwrap();

    assert_eq!(
        lifecycle
            .resume("agent-stale-binding", 24, 80, "resume-stale-binding")
            .await
            .unwrap(),
        ProviderAgentSessionOpenOutcome::Resumed
    );
    let active = provider_lifecycle
        .active_launch_id(ProviderKind::Claude, &scope)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(active, old_slot);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_provider_agent_session_resume_同一sessionへの並行要求はptyを一度だけ起動する() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(Arc::new(
        LocalProviderAgentSessionRepository::new(
            store.clone() as Arc<dyn LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
    )));
    sessions
        .create(
            "agent-concurrent-resume",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            AgentSessionOrigin::Standalone,
            "create-concurrent-resume",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session(
            "agent-concurrent-resume",
            "provider-concurrent-resume",
            None,
            "associate-concurrent-resume",
        )
        .await
        .unwrap();
    sessions
        .observe_process_exit(
            "agent-concurrent-resume",
            Some(0),
            "pause-concurrent-resume",
        )
        .await
        .unwrap();

    let provider_lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(NoopLifecycleEvents),
    ));
    let launches = Arc::new(RecordingResumeLaunches::default());
    let terminal = Arc::new(LifecycleTerminal::new(ManagedPtyPresence::ConfirmedAbsent));
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    *terminal.first_spawn_entered.lock().unwrap() = Some(entered_sender);
    *terminal.first_spawn_release.lock().unwrap() = Some(release_receiver);
    let lifecycle = Arc::new(ProviderAgentSessionLifecycleUsecase::new(
        sessions,
        provider_lifecycle,
        launches,
        terminal.clone(),
        Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealthRepository::default(),
        ))),
        Arc::new(RecordingChangeNotifier::default()),
    ));

    let first = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            lifecycle
                .resume("agent-concurrent-resume", 24, 80, "resume-concurrent-1")
                .await
        }
    });
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let second = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            lifecycle
                .resume("agent-concurrent-resume", 24, 80, "resume-concurrent-2")
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    release_sender.send(()).unwrap();

    let outcomes = [first.await.unwrap(), second.await.unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| { **outcome == Ok(ProviderAgentSessionOpenOutcome::Resumed) })
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| {
                **outcome == Err(super::ProviderAgentSessionLifecycleUsecaseError::InvalidOperation)
            })
            .count(),
        1
    );
    assert_eq!(*terminal.spawn_count.lock().unwrap(), 1);
    assert!(terminal.stops.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_provider_agent_session_resume中のarchiveは同一sessionの操作完了後に実行する() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(Arc::new(
        LocalProviderAgentSessionRepository::new(
            store.clone() as Arc<dyn LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
    )));
    sessions
        .create(
            "agent-resume-archive",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            AgentSessionOrigin::Standalone,
            "create-resume-archive",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session(
            "agent-resume-archive",
            "provider-resume-archive",
            None,
            "associate-resume-archive",
        )
        .await
        .unwrap();
    sessions
        .observe_process_exit("agent-resume-archive", Some(0), "pause-resume-archive")
        .await
        .unwrap();

    let provider_lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        Arc::new(NoopLifecycleEvents),
    ));
    let launches = Arc::new(RecordingResumeLaunches::default());
    let terminal = Arc::new(LifecycleTerminal::new(ManagedPtyPresence::ConfirmedAbsent));
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    *terminal.first_spawn_entered.lock().unwrap() = Some(entered_sender);
    *terminal.first_spawn_release.lock().unwrap() = Some(release_receiver);
    let lifecycle = Arc::new(ProviderAgentSessionLifecycleUsecase::new(
        sessions.clone(),
        provider_lifecycle,
        launches,
        terminal.clone(),
        Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealthRepository::default(),
        ))),
        Arc::new(RecordingChangeNotifier::default()),
    ));

    let resume = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            lifecycle
                .resume("agent-resume-archive", 24, 80, "resume-before-archive")
                .await
        }
    });
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let archive = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            lifecycle
                .archive("agent-resume-archive", "archive-after-resume")
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    release_sender.send(()).unwrap();

    assert_eq!(
        resume.await.unwrap().unwrap(),
        ProviderAgentSessionOpenOutcome::Resumed
    );
    assert_eq!(
        archive.await.unwrap().unwrap(),
        AgentSessionArchiveOutcome::Archived
    );
    assert_eq!(terminal.stops.lock().unwrap().len(), 1);
    assert_eq!(
        sessions
            .find("agent-resume-archive")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Archived
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_provider_agent_session_open_同一sessionへの並行要求は一度だけ自動resumeする() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let sessions = Arc::new(ProviderAgentSessionUsecase::new(Arc::new(
        LocalProviderAgentSessionRepository::new(
            store.clone() as Arc<dyn LocalEventTransactionRepository>,
            store.installation_id().to_string(),
        ),
    )));
    sessions
        .create(
            "agent-concurrent-open",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            AgentSessionOrigin::Standalone,
            "create-concurrent-open",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session(
            "agent-concurrent-open",
            "provider-concurrent-open",
            None,
            "associate-concurrent-open",
        )
        .await
        .unwrap();

    let terminal = Arc::new(LifecycleTerminal::new(ManagedPtyPresence::ConfirmedAbsent));
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    *terminal.first_spawn_entered.lock().unwrap() = Some(entered_sender);
    *terminal.first_spawn_release.lock().unwrap() = Some(release_receiver);
    let lifecycle = Arc::new(ProviderAgentSessionLifecycleUsecase::new(
        sessions,
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(NoopLifecycleEvents),
        )),
        Arc::new(RecordingResumeLaunches::default()),
        terminal.clone(),
        Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealthRepository::default(),
        ))),
        Arc::new(RecordingChangeNotifier::default()),
    ));

    let first = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            lifecycle
                .open("agent-concurrent-open", 24, 80, "open-concurrent-1")
                .await
        }
    });
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let second = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            lifecycle
                .open("agent-concurrent-open", 24, 80, "open-concurrent-2")
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    release_sender.send(()).unwrap();

    let outcomes = [
        first.await.unwrap().unwrap(),
        second.await.unwrap().unwrap(),
    ];
    assert!(outcomes.contains(&ProviderAgentSessionOpenOutcome::Resumed));
    assert!(outcomes.contains(&ProviderAgentSessionOpenOutcome::Attached));
    assert_eq!(*terminal.spawn_count.lock().unwrap(), 1);
    assert!(terminal.stops.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_provider_agent_session_restore中のdeleteは復帰完了後の状態で拒否する() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        terminal,
        ..
    } = setup();
    sessions
        .create(
            "agent-restore-delete",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            AgentSessionOrigin::Standalone,
            "create-restore-delete",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session(
            "agent-restore-delete",
            "provider-restore-delete",
            None,
            "associate-restore-delete",
        )
        .await
        .unwrap();
    lifecycle
        .archive("agent-restore-delete", "archive-before-restore-delete")
        .await
        .unwrap();
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    *terminal.first_spawn_entered.lock().unwrap() = Some(entered_sender);
    *terminal.first_spawn_release.lock().unwrap() = Some(release_receiver);
    let lifecycle = Arc::new(lifecycle);

    let restore = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            lifecycle
                .restore("agent-restore-delete", 24, 80, "restore-before-delete")
                .await
        }
    });
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let delete = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            lifecycle
                .delete("agent-restore-delete", "delete-during-restore")
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    release_sender.send(()).unwrap();

    assert_eq!(
        restore.await.unwrap().unwrap(),
        ProviderAgentSessionOpenOutcome::Restored
    );
    assert_eq!(
        delete.await.unwrap().unwrap_err(),
        super::ProviderAgentSessionLifecycleUsecaseError::InvalidOperation
    );
    assert!(terminal.deletes.lock().unwrap().is_empty());
    assert_eq!(
        sessions
            .find("agent-restore-delete")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Open
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_provider_agent_session_exit_open待機中に旧世代になったexitを反映しない() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        terminal,
        ..
    } = setup();
    sessions
        .create(
            "agent-open-stale-exit",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            AgentSessionOrigin::Standalone,
            "create-open-stale-exit",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session(
            "agent-open-stale-exit",
            "provider-open-stale-exit",
            None,
            "associate-open-stale-exit",
        )
        .await
        .unwrap();
    *terminal.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    *terminal.first_spawn_entered.lock().unwrap() = Some(entered_sender);
    *terminal.first_spawn_release.lock().unwrap() = Some(release_receiver);
    let lifecycle = Arc::new(lifecycle);

    let open = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            lifecycle
                .open("agent-open-stale-exit", 24, 80, "open-before-stale-exit")
                .await
        }
    });
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let exit = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            lifecycle
                .observe_process_exit(
                    "agent-open-stale-exit",
                    1,
                    Some(0),
                    "old-runtime-exit-during-open",
                )
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    release_sender.send(()).unwrap();

    assert_eq!(
        open.await.unwrap().unwrap(),
        ProviderAgentSessionOpenOutcome::Resumed
    );
    exit.await.unwrap().unwrap();
    assert_eq!(*terminal.runtime_generation.lock().unwrap(), 2);
    assert_eq!(
        sessions
            .find("agent-open-stale-exit")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Open
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_provider_agent_session_archive縮退delete中のopenはdelete完了後に評価する() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        terminal,
        ..
    } = setup();
    sessions
        .create(
            "agent-fallback-delete-open",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            AgentSessionOrigin::Standalone,
            "create-fallback-delete-open",
        )
        .await
        .unwrap();
    let (entered_sender, entered_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    *terminal.first_delete_entered.lock().unwrap() = Some(entered_sender);
    *terminal.first_delete_release.lock().unwrap() = Some(release_receiver);
    let lifecycle = Arc::new(lifecycle);

    let fallback_delete = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            lifecycle
                .confirm_archive_fallback_delete(
                    "agent-fallback-delete-open",
                    "fallback-delete-before-open",
                )
                .await
        }
    });
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let mut open = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            lifecycle
                .open(
                    "agent-fallback-delete-open",
                    24,
                    80,
                    "open-during-fallback-delete",
                )
                .await
        }
    });

    assert!(tokio::time::timeout(Duration::from_millis(50), &mut open)
        .await
        .is_err());
    release_sender.send(()).unwrap();
    fallback_delete.await.unwrap().unwrap();
    assert_eq!(
        open.await.unwrap().unwrap_err(),
        super::ProviderAgentSessionLifecycleUsecaseError::NotFound
    );
    assert!(sessions
        .find("agent-fallback-delete-open")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_provider_agent_session_lifecycle_exit由来のpaused遷移で変更通知を発火する() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        terminal,
        change_notifier,
        ..
    } = setup();
    sessions
        .create(
            "agent-notify",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            AgentSessionOrigin::Standalone,
            "create-notify",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session("agent-notify", "provider-notify", None, "associate-notify")
        .await
        .unwrap();

    *terminal.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
    lifecycle
        .observe_process_exit("agent-notify", 1, Some(137), "exit-notify")
        .await
        .unwrap();

    assert_eq!(
        change_notifier.notified.lock().unwrap().as_slice(),
        &["/repo/worktree"]
    );
    assert!(
        sessions
            .find("agent-notify")
            .await
            .unwrap()
            .unwrap()
            .session()
            .last_exit_abnormal(),
        "非0 exitはabnormalとして永続化される"
    );

    lifecycle
        .observe_process_exit("agent-notify", 1, Some(137), "exit-notify-again")
        .await
        .unwrap();
    assert_eq!(
        change_notifier.notified.lock().unwrap().len(),
        1,
        "既にpausedの遅延通知では再emitしない"
    );
}
