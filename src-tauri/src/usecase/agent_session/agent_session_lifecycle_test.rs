use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use super::{
    AgentSessionLifecycleUsecase, AgentSessionLifecycleUsecaseError, AgentSessionOpenOutcome,
    AgentSessionUsecase, ExecutionTreeCacheReleaseError, ProviderAgentRuntime,
    StartedExecutionTreeRegistrar, StartedExecutionTreeRegistrationError,
};
use crate::adaptor::gateway::agent_session::LocalAgentSessionRepository;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::provider_lifecycle::LocalProviderLifecycleCredentialGateway;
use crate::adaptor::gateway::workflow::test_support::{
    seed_workflow_session_facts, WorkflowSessionFactSeed,
};
use crate::domain::agent_session::aggregates::{
    AgentSessionArchiveOutcome, AgentSessionLifecycle, AgentSessionTreeLocation,
    ManagedPtyPresence, ResolvedProviderExecutable,
};
use crate::domain::agent_session::repository::{
    AgentSessionRepository, AgentSessionRepositoryError, VersionedAgentSession,
};
use crate::domain::agent_session::{
    PreparedProviderLaunch, ProviderAgentLaunchGateway, ProviderAgentLaunchGatewayError,
    ProviderAgentTerminalGateway, ProviderAgentTerminalGatewayError,
    ProviderAgentTerminalSpawnError, ProviderAvailabilityReader, ProviderSessionLaunch,
};
use crate::domain::provider_lifecycle::{
    ArmedProviderLifecycle, ProviderHookHealth, ProviderHookHealthRepository,
    ProviderHookHealthRepositoryError, ProviderKind, ProviderLifecycleEvent,
    ProviderLifecycleEventRepository, ProviderLifecycleIngressResult, ProviderLifecycleRejection,
    ProviderLifecycleRepositoryError, ProviderLifecycleScope, ProviderLifecycleSignal,
    ProviderLifecycleSlotId, ScopedProviderLifecycleEvent, VersionedProviderHookHealth,
};
use crate::domain::terminal_surface::{TerminalProcessLaunch, TerminalSurfaceOwner};
use crate::domain::workflow::{AgentSessionActivity, NodeFact};
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::provider_lifecycle::{
    ProviderExecutionTreeStopCommand, ProviderExecutionTreeStopTransaction,
    ProviderHookHealthUsecase, ProviderLifecycleIngressUsecase,
    ProviderLifecycleIngressUsecaseError, ProviderLifecycleUsecase,
};

fn session_location(id: &str) -> AgentSessionTreeLocation {
    AgentSessionTreeLocation::session_tree_root(id).unwrap()
}

fn workflow_location(tree_id: &str, node_execution_id: &str) -> AgentSessionTreeLocation {
    AgentSessionTreeLocation::workflow_node(tree_id, node_execution_id).unwrap()
}

#[derive(Default)]
struct RecordingChangeNotifier {
    notified: Mutex<Vec<String>>,
}

#[derive(Default)]
struct RecordingExecutionTrees {
    releases: Mutex<Vec<String>>,
    release_error: Mutex<Option<ExecutionTreeCacheReleaseError>>,
}

#[async_trait::async_trait]
impl StartedExecutionTreeRegistrar for RecordingExecutionTrees {
    async fn register_started_execution_tree(
        &self,
        _tree_id: &str,
    ) -> Result<(), StartedExecutionTreeRegistrationError> {
        Ok(())
    }

    async fn release_deleted_execution_tree(
        &self,
        tree_id: &str,
    ) -> Result<(), ExecutionTreeCacheReleaseError> {
        self.releases.lock().unwrap().push(tree_id.to_string());
        self.release_error
            .lock()
            .unwrap()
            .as_ref()
            .copied()
            .map_or(Ok(()), Err)
    }
}

impl crate::usecase::agent_session::AgentSessionChangeNotifier for RecordingChangeNotifier {
    fn agent_session_changed(&self, worktree_path: &str) {
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

    async fn load_scope(
        &self,
        _scope: &ProviderLifecycleScope,
    ) -> Result<Vec<ScopedProviderLifecycleEvent>, ProviderLifecycleRepositoryError> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct CoordinatedLifecycleEvents {
    stored: Mutex<Vec<ScopedProviderLifecycleEvent>>,
    transcript_associated: tokio::sync::Notify,
    binding_expired_entered: tokio::sync::Notify,
    binding_expired_release: tokio::sync::Notify,
    block_binding_expired: AtomicBool,
}

impl CoordinatedLifecycleEvents {
    fn block_next_binding_expired(&self) {
        self.block_binding_expired.store(true, Ordering::SeqCst);
    }

    fn release_binding_expired(&self) {
        self.binding_expired_release.notify_one();
    }
}

#[async_trait::async_trait]
impl ProviderLifecycleEventRepository for CoordinatedLifecycleEvents {
    async fn append(
        &self,
        events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleRepositoryError> {
        let has_transcript_associated = events.iter().any(|event| {
            matches!(
                event.clone().into_parts().1,
                ProviderLifecycleEvent::TranscriptAssociated { .. }
            )
        });
        let has_binding_expired = events.iter().any(|event| {
            matches!(
                event.clone().into_parts().1,
                ProviderLifecycleEvent::BindingExpired { .. }
            )
        });
        if has_transcript_associated {
            self.transcript_associated.notify_one();
        }
        if has_binding_expired && self.block_binding_expired.swap(false, Ordering::SeqCst) {
            self.binding_expired_entered.notify_one();
            self.binding_expired_release.notified().await;
        }
        self.stored.lock().unwrap().extend(events);
        Ok(())
    }

    async fn load_scope(
        &self,
        scope: &ProviderLifecycleScope,
    ) -> Result<Vec<ScopedProviderLifecycleEvent>, ProviderLifecycleRepositoryError> {
        Ok(self
            .stored
            .lock()
            .unwrap()
            .clone()
            .into_iter()
            .filter(|event| event.clone().into_parts().0 == *scope)
            .collect())
    }
}

#[derive(Default)]
struct NoopProviderExecutionTreeStops;

#[async_trait::async_trait]
impl ProviderExecutionTreeStopTransaction for NoopProviderExecutionTreeStops {
    async fn commit_provider_stop(
        &self,
        _command: ProviderExecutionTreeStopCommand,
        _lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
    ) -> Result<(), ProviderLifecycleIngressUsecaseError> {
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
        _executable: ResolvedProviderExecutable,
        launch: ProviderSessionLaunch,
        _worktree_path: &str,
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

struct AlwaysProviderAvailable;

impl ProviderAvailabilityReader for AlwaysProviderAvailable {
    fn is_available(&self, _provider: ProviderKind) -> bool {
        true
    }

    fn resolved_executable(&self, _provider: ProviderKind) -> Option<ResolvedProviderExecutable> {
        Some(ResolvedProviderExecutable::new("/provider".into()).unwrap())
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
    fail_stop: Mutex<bool>,
    spawn_count: Mutex<usize>,
    first_spawn_entered: Mutex<Option<mpsc::Sender<()>>>,
    first_spawn_release: Mutex<Option<mpsc::Receiver<()>>>,
    first_stop_entered: Mutex<Option<mpsc::Sender<()>>>,
    first_stop_release: Mutex<Option<mpsc::Receiver<()>>>,
    first_delete_entered: Mutex<Option<mpsc::Sender<()>>>,
    first_delete_release: Mutex<Option<mpsc::Receiver<()>>>,
    stops: Mutex<Vec<TerminalSurfaceOwner>>,
    deletes: Mutex<Vec<TerminalSurfaceOwner>>,
}

struct SaveFailingAgentSessionRepository {
    inner: Arc<LocalAgentSessionRepository>,
}

#[async_trait::async_trait]
impl AgentSessionRepository for SaveFailingAgentSessionRepository {
    async fn create(
        &self,
        session: crate::domain::agent_session::aggregates::AgentSession,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        self.inner.create(session, caller_request_id).await
    }

    async fn create_with_lifecycle_events(
        &self,
        session: crate::domain::agent_session::aggregates::AgentSession,
        lifecycle_events: Vec<ScopedProviderLifecycleEvent>,
        caller_request_id: &str,
    ) -> Result<VersionedAgentSession, AgentSessionRepositoryError> {
        self.inner
            .create_with_lifecycle_events(session, lifecycle_events, caller_request_id)
            .await
    }

    async fn find(
        &self,
        session_id: &str,
    ) -> Result<Option<VersionedAgentSession>, AgentSessionRepositoryError> {
        self.inner.find(session_id).await
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
        session: VersionedAgentSession,
        authorization: crate::domain::agent_session::aggregates::AgentSessionRemovalAuthorization,
        caller_request_id: &str,
    ) -> Result<(), AgentSessionRepositoryError> {
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
            fail_stop: Mutex::new(false),
            spawn_count: Mutex::new(0),
            first_spawn_entered: Mutex::new(None),
            first_spawn_release: Mutex::new(None),
            first_stop_entered: Mutex::new(None),
            first_stop_release: Mutex::new(None),
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
    ) -> Result<(), ProviderAgentTerminalSpawnError> {
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
            return Err(ProviderAgentTerminalSpawnError::OtherSpawnFailure {
                error: "test terminal spawn failure".to_string(),
            });
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
        if let Some(sender) = self.first_stop_entered.lock().unwrap().take() {
            sender.send(()).unwrap();
        }
        if let Some(receiver) = self.first_stop_release.lock().unwrap().take() {
            receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        }
        self.stops.lock().unwrap().push(owner.clone());
        if *self.fail_stop.lock().unwrap() {
            return Err(ProviderAgentTerminalGatewayError::Unavailable);
        }
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
    store: Arc<LocalEventStore>,
    sessions: Arc<AgentSessionUsecase>,
    lifecycle: AgentSessionLifecycleUsecase,
    launches: Arc<RecordingResumeLaunches>,
    terminal: Arc<LifecycleTerminal>,
    provider_lifecycle: Arc<ProviderLifecycleUsecase>,
    hook_health: Arc<ProviderHookHealthUsecase>,
    change_notifier: Arc<RecordingChangeNotifier>,
    execution_trees: Arc<RecordingExecutionTrees>,
}

fn setup() -> LifecycleTestContext {
    setup_with_lifecycle_events(Arc::new(NoopLifecycleEvents))
}

fn setup_with_lifecycle_events(
    lifecycle_events: Arc<dyn ProviderLifecycleEventRepository>,
) -> LifecycleTestContext {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        LocalAgentSessionRepository::new(store.clone()),
    )));
    let lifecycle = Arc::new(ProviderLifecycleUsecase::new(
        Arc::new(LocalProviderLifecycleCredentialGateway),
        lifecycle_events,
    ));
    let launches = Arc::new(RecordingResumeLaunches::default());
    let terminal = Arc::new(LifecycleTerminal::new(ManagedPtyPresence::Live));
    let hook_health = Arc::new(ProviderHookHealthUsecase::new(Arc::new(
        MemoryHookHealthRepository::default(),
    )));
    let change_notifier = Arc::new(RecordingChangeNotifier::default());
    let execution_trees = Arc::new(RecordingExecutionTrees::default());
    let usecase = AgentSessionLifecycleUsecase::new(
        sessions.clone(),
        lifecycle.clone(),
        ProviderAgentRuntime::new(
            Arc::new(AlwaysProviderAvailable),
            launches.clone(),
            terminal.clone(),
        ),
        hook_health.clone(),
        change_notifier.clone(),
        execution_trees.clone(),
    );
    LifecycleTestContext {
        _directory: directory,
        store,
        sessions,
        lifecycle: usecase,
        launches,
        terminal,
        provider_lifecycle: lifecycle,
        hook_health,
        change_notifier,
        execution_trees,
    }
}

struct ActivityStopExclusionFixture {
    context: LifecycleTestContext,
    ingress: ProviderLifecycleIngressUsecase,
    armed: ArmedProviderLifecycle,
    scope: ProviderLifecycleScope,
    agent_session_id: String,
    workflow_execution_id: String,
    node_execution_id: String,
    provider_session_id: String,
}

async fn setup_activity_stop_exclusion(case_name: &str) -> ActivityStopExclusionFixture {
    setup_activity_stop_exclusion_with_events(case_name, Arc::new(NoopLifecycleEvents)).await
}

async fn setup_activity_stop_exclusion_with_events(
    case_name: &str,
    lifecycle_events: Arc<dyn ProviderLifecycleEventRepository>,
) -> ActivityStopExclusionFixture {
    let context = setup_with_lifecycle_events(lifecycle_events);
    let agent_session_id = format!("agent-activity-stop-{case_name}");
    let workflow_execution_id = format!("workflow-activity-stop-{case_name}");
    let node_execution_id = format!("node-activity-stop-{case_name}");
    let provider_session_id = format!("provider-activity-stop-{case_name}");
    seed_workflow_session_facts(
        &context.store,
        WorkflowSessionFactSeed {
            workflow_name: "workflow",
            request: "test",
            worktree_path: "/repo/worktree",
            provider: ProviderKind::Claude,
            workflow_execution_id: &workflow_execution_id,
            node_execution_id: &node_execution_id,
            session_id: &agent_session_id,
            initial_instruction_admitted: true,
        },
    )
    .unwrap();
    context
        .sessions
        .create(
            &agent_session_id,
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            workflow_location(&workflow_execution_id, &node_execution_id),
            &format!("create-{case_name}"),
        )
        .await
        .unwrap();
    let ingress = ProviderLifecycleIngressUsecase::new(
        context.provider_lifecycle.clone(),
        context.sessions.clone(),
        context.hook_health.clone(),
        Arc::new(LocalAgentSessionRepository::new(context.store.clone())),
        Arc::new(NoopProviderExecutionTreeStops),
        context.change_notifier.clone(),
    );
    let slot_id = ProviderLifecycleSlotId::new(format!("slot-activity-stop-{case_name}")).unwrap();
    let scope = ProviderLifecycleScope::new(&agent_session_id).unwrap();
    let armed = context
        .provider_lifecycle
        .arm(slot_id.clone(), ProviderKind::Claude, scope.clone())
        .await
        .unwrap();
    assert_eq!(
        ingress
            .receive(
                &slot_id,
                armed.capability(),
                ProviderLifecycleSignal::session_started(
                    armed.binding_id(),
                    ProviderKind::Claude,
                    scope.clone(),
                    provider_session_id.clone(),
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap(),
        ProviderLifecycleIngressResult::Applied
    );

    ActivityStopExclusionFixture {
        context,
        ingress,
        armed,
        scope,
        agent_session_id,
        workflow_execution_id,
        node_execution_id,
        provider_session_id,
    }
}

fn activity_fact_count(store: &Arc<LocalEventStore>, tree_id: &str) -> usize {
    crate::adaptor::gateway::workflow::fact_log::read_tree_records(store, tree_id)
        .unwrap()
        .into_iter()
        .filter(|record| matches!(record.fact, NodeFact::AgentActivityObserved(_)))
        .count()
}

async fn stop_activity_fixture(fixture: &ActivityStopExclusionFixture, caller_suffix: &str) {
    fixture
        .context
        .lifecycle
        .stop_for_terminal_execution_tree_node_preserving_checkpoint(
            &fixture.agent_session_id,
            &fixture.node_execution_id,
            &format!("stop-{caller_suffix}"),
        )
        .await
        .unwrap();
    fixture
        .context
        .lifecycle
        .observe_process_exit(
            &fixture.agent_session_id,
            1,
            Some(0),
            &format!("exit-{caller_suffix}"),
        )
        .await
        .unwrap();
}

async fn observe_working(fixture: &ActivityStopExclusionFixture) -> ProviderLifecycleIngressResult {
    observe_working_with_transcript(fixture, None).await
}

async fn observe_working_with_transcript(
    fixture: &ActivityStopExclusionFixture,
    transcript_ref: Option<String>,
) -> ProviderLifecycleIngressResult {
    fixture
        .ingress
        .receive(
            fixture.armed.slot_id(),
            fixture.armed.capability(),
            ProviderLifecycleSignal::activity_observed(
                fixture.armed.binding_id(),
                ProviderKind::Claude,
                fixture.scope.clone(),
                fixture.provider_session_id.clone(),
                transcript_ref.as_deref(),
                AgentSessionActivity::Working,
            )
            .unwrap(),
        )
        .await
        .unwrap()
}

async fn assert_paused_awaiting_instruction(fixture: &ActivityStopExclusionFixture) {
    let session = fixture
        .context
        .sessions
        .find(&fixture.agent_session_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.session().lifecycle(), AgentSessionLifecycle::Paused);
    assert_eq!(
        session.session().activity(),
        AgentSessionActivity::AwaitingInstruction
    );
}

#[tokio::test]
async fn test_agent_session活動観測と停止は到着順に従い停止後のworkingを拒否する() {
    // Given: 停止経路が活動観測より先に確定した AgentSession
    let stop_first = setup_activity_stop_exclusion("stop-first").await;
    stop_activity_fixture(&stop_first, "stop-first").await;
    let facts_after_stop =
        activity_fact_count(&stop_first.context.store, &stop_first.workflow_execution_id);

    // When: binding 解放後に Working が到着する
    let rejected = observe_working(&stop_first).await;

    // Then: 後着の活動観測は拒否され、活動事実と停止状態を変えない
    assert_eq!(
        rejected,
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::BindingNotActive)
    );
    assert_eq!(
        activity_fact_count(&stop_first.context.store, &stop_first.workflow_execution_id,),
        facts_after_stop
    );
    assert_paused_awaiting_instruction(&stop_first).await;

    // Given: Working が停止経路より先に受理された AgentSession
    let activity_first = setup_activity_stop_exclusion("activity-first").await;
    assert_eq!(
        observe_working(&activity_first).await,
        ProviderLifecycleIngressResult::Applied
    );
    assert_eq!(
        activity_fact_count(
            &activity_first.context.store,
            &activity_first.workflow_execution_id,
        ),
        1
    );

    // When: 活動観測の後に停止経路を確定する
    stop_activity_fixture(&activity_first, "activity-first").await;

    // Then: 最終状態は停止側の値となり、停止は活動観測事実を追加しない
    assert_paused_awaiting_instruction(&activity_first).await;
    assert_eq!(
        activity_fact_count(
            &activity_first.context.store,
            &activity_first.workflow_execution_id,
        ),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_agent_session活動観測と停止_operation_lockが受理とbinding解放を直列化する() {
    // Given: 停止処理が operation lock を保持したまま terminal 停止へ到達している
    let receive_events = Arc::new(CoordinatedLifecycleEvents::default());
    let receive_fixture = Arc::new(
        setup_activity_stop_exclusion_with_events("receive-lock", receive_events.clone()).await,
    );
    let (stop_entered_sender, stop_entered_receiver) = mpsc::channel();
    let (stop_release_sender, stop_release_receiver) = mpsc::channel();
    *receive_fixture
        .context
        .terminal
        .first_stop_entered
        .lock()
        .unwrap() = Some(stop_entered_sender);
    *receive_fixture
        .context
        .terminal
        .first_stop_release
        .lock()
        .unwrap() = Some(stop_release_receiver);
    let stop = tokio::spawn({
        let fixture = receive_fixture.clone();
        async move {
            stop_activity_fixture(&fixture, "receive-lock").await;
        }
    });
    stop_entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let activity = tokio::spawn({
        let fixture = receive_fixture.clone();
        async move {
            observe_working_with_transcript(&fixture, Some("transcript-receive-lock".to_string()))
                .await
        }
    });

    // When: 後着の活動観測が停止側の operation lock を待つ
    let lifecycle_received_before_stop = tokio::time::timeout(
        Duration::from_millis(100),
        receive_events.transcript_associated.notified(),
    )
    .await
    .is_ok();
    stop_release_sender.send(()).unwrap();
    stop.await.unwrap();
    let activity_result = activity.await.unwrap();

    // Then: lifecycle 受理も停止確定後となり、解放済み binding として拒否される
    assert!(
        !lifecycle_received_before_stop,
        "活動 signal の lifecycle 受理を operation lock より先に行ってはならない"
    );
    assert_eq!(
        activity_result,
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::BindingNotActive)
    );
    assert_eq!(
        activity_fact_count(
            &receive_fixture.context.store,
            &receive_fixture.workflow_execution_id,
        ),
        0
    );
    assert_paused_awaiting_instruction(&receive_fixture).await;

    // Given: 停止状態を保存し、binding 解放 event の永続化へ到達した停止処理
    let release_events = Arc::new(CoordinatedLifecycleEvents::default());
    release_events.block_next_binding_expired();
    let release_fixture = Arc::new(
        setup_activity_stop_exclusion_with_events("release-lock", release_events.clone()).await,
    );
    let stop = tokio::spawn({
        let fixture = release_fixture.clone();
        async move {
            stop_activity_fixture(&fixture, "release-lock").await;
        }
    });
    tokio::time::timeout(
        Duration::from_secs(1),
        release_events.binding_expired_entered.notified(),
    )
    .await
    .unwrap();

    // When: binding 解放の完了前に同じ AgentSession の operation lock を取得する
    let operation = tokio::time::timeout(
        Duration::from_millis(100),
        release_fixture
            .context
            .sessions
            .lock_operation(&release_fixture.agent_session_id),
    )
    .await;
    let operation_was_available = operation.is_ok();
    drop(operation);
    let activity = tokio::spawn({
        let fixture = release_fixture.clone();
        async move { observe_working(&fixture).await }
    });
    tokio::task::yield_now().await;
    release_events.release_binding_expired();
    stop.await.unwrap();
    let activity_result = activity.await.unwrap();

    // Then: binding 解放まで lock は保持され、活動観測は停止後に拒否される
    assert!(
        !operation_was_available,
        "binding 解放が完了する前に operation lock を解放してはならない"
    );
    assert_eq!(
        activity_result,
        ProviderLifecycleIngressResult::Rejected(ProviderLifecycleRejection::BindingNotActive)
    );
    assert_eq!(
        activity_fact_count(
            &release_fixture.context.store,
            &release_fixture.workflow_execution_id,
        ),
        0
    );
    assert_paused_awaiting_instruction(&release_fixture).await;
}

#[tokio::test]
async fn test_workflow所有agent_session停止_checkpointとprovider参照を保持してresumeできる() {
    let LifecycleTestContext {
        _directory,
        store,
        sessions,
        lifecycle,
        launches,
        terminal,
        provider_lifecycle,
        ..
    } = setup();
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "workflow",
            request: "test",
            worktree_path: "/repo/worktree",
            provider: ProviderKind::Claude,
            workflow_execution_id: "workflow-1",
            node_execution_id: "node-1",
            session_id: "workflow-agent",
            initial_instruction_admitted: true,
        },
    )
    .unwrap();
    sessions
        .create(
            "workflow-agent",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            workflow_location("workflow-1", "node-1"),
            "create-workflow-agent",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session(
            "workflow-agent",
            "provider-1",
            Some("provider://transcript/1"),
            "associate-workflow-agent",
        )
        .await
        .unwrap();
    let scope = ProviderLifecycleScope::new("workflow-agent").unwrap();
    provider_lifecycle
        .arm(
            ProviderLifecycleSlotId::new("workflow-agent-slot").unwrap(),
            ProviderKind::Claude,
            scope.clone(),
        )
        .await
        .unwrap();

    assert_eq!(
        lifecycle
            .ensure_provider_running("workflow-agent", 24, 80, "ensure-open-workflow-agent")
            .await
            .unwrap(),
        AgentSessionOpenOutcome::Attached
    );
    assert!(launches.launches.lock().unwrap().is_empty());

    lifecycle
        .stop_for_terminal_execution_tree_node_preserving_checkpoint(
            "workflow-agent",
            "node-1",
            "stop-workflow-agent",
        )
        .await
        .unwrap();

    let stopped = sessions.find("workflow-agent").await.unwrap().unwrap();
    assert_eq!(stopped.session().lifecycle(), AgentSessionLifecycle::Paused);
    assert!(!stopped.session().last_exit_abnormal());
    assert_eq!(stopped.session().provider_session_id(), Some("provider-1"));
    assert_eq!(
        stopped.session().transcript_ref(),
        Some("provider://transcript/1")
    );
    assert_eq!(
        *terminal.presence.lock().unwrap(),
        ManagedPtyPresence::ConfirmedAbsent
    );
    assert_eq!(terminal.stops.lock().unwrap().len(), 1);
    assert!(terminal.deletes.lock().unwrap().is_empty());
    assert_eq!(
        launches.cleanups.lock().unwrap().as_slice(),
        &["workflow-agent"]
    );
    assert!(provider_lifecycle
        .active_launch_id(ProviderKind::Claude, &scope)
        .await
        .unwrap()
        .is_none());

    assert_eq!(
        lifecycle
            .ensure_provider_running("workflow-agent", 24, 80, "resume-workflow-agent")
            .await
            .unwrap(),
        AgentSessionOpenOutcome::Resumed
    );
    assert_eq!(
        launches.launches.lock().unwrap().as_slice(),
        &[ProviderSessionLaunch::resume("provider-1").unwrap()]
    );
    assert_eq!(
        sessions
            .find("workflow-agent")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Open
    );
}

#[tokio::test]
async fn test_archived_agent_sessionのensure_provider_runningは起動せず拒否する() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        launches,
        ..
    } = setup();
    sessions
        .create(
            "archived-agent",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("archived-agent"),
            "create-archived-agent",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session(
            "archived-agent",
            "provider-archived-agent",
            None,
            "associate-archived-agent",
        )
        .await
        .unwrap();
    assert_eq!(
        lifecycle
            .archive("archived-agent", "archive-agent")
            .await
            .unwrap(),
        AgentSessionArchiveOutcome::Archived
    );
    assert_eq!(
        sessions
            .find("archived-agent")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Archived
    );

    let error = lifecycle
        .ensure_provider_running("archived-agent", 24, 80, "ensure-archived-agent")
        .await
        .unwrap_err();

    assert_eq!(error, AgentSessionLifecycleUsecaseError::InvalidOperation);
    assert!(launches.launches.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_session起動木node終端停止_checkpoint保持とlaunch資源解放まで行う() {
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
            "standalone-agent",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("standalone-agent"),
            "create-standalone-agent",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session(
            "standalone-agent",
            "provider-standalone",
            Some("provider://transcript/standalone"),
            "associate-standalone-agent",
        )
        .await
        .unwrap();
    let scope = ProviderLifecycleScope::new("standalone-agent").unwrap();
    provider_lifecycle
        .arm(
            ProviderLifecycleSlotId::new("standalone-agent-slot").unwrap(),
            ProviderKind::Claude,
            scope.clone(),
        )
        .await
        .unwrap();

    lifecycle
        .stop_for_terminal_execution_tree_node_preserving_checkpoint(
            "standalone-agent",
            "standalone-agent",
            "stop-standalone-agent",
        )
        .await
        .unwrap();

    let stopped = sessions.find("standalone-agent").await.unwrap().unwrap();
    assert_eq!(stopped.session().lifecycle(), AgentSessionLifecycle::Paused);
    assert!(!stopped.session().last_exit_abnormal());
    assert_eq!(
        stopped.session().provider_session_id(),
        Some("provider-standalone")
    );
    assert_eq!(
        stopped.session().transcript_ref(),
        Some("provider://transcript/standalone")
    );
    assert_eq!(
        *terminal.presence.lock().unwrap(),
        ManagedPtyPresence::ConfirmedAbsent
    );
    assert_eq!(terminal.stops.lock().unwrap().len(), 1);
    assert_eq!(
        launches.cleanups.lock().unwrap().as_slice(),
        &["standalone-agent"]
    );
    assert!(provider_lifecycle
        .active_launch_id(ProviderKind::Claude, &scope)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_workflow所有agent_session停止_provider未確定でもgcせずpausedで保持する() {
    let LifecycleTestContext {
        _directory,
        store,
        sessions,
        lifecycle,
        terminal,
        ..
    } = setup();
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "workflow",
            request: "test",
            worktree_path: "/repo/worktree",
            provider: ProviderKind::Codex,
            workflow_execution_id: "workflow-1",
            node_execution_id: "node-1",
            session_id: "workflow-agent-unknown",
            initial_instruction_admitted: true,
        },
    )
    .unwrap();
    sessions
        .create(
            "workflow-agent-unknown",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            workflow_location("workflow-1", "node-1"),
            "create-workflow-agent-unknown",
        )
        .await
        .unwrap();

    lifecycle
        .stop_for_terminal_execution_tree_node_preserving_checkpoint(
            "workflow-agent-unknown",
            "node-1",
            "stop-workflow-agent-unknown",
        )
        .await
        .unwrap();

    assert_eq!(
        lifecycle
            .open(
                "workflow-agent-unknown",
                24,
                80,
                "open-workflow-agent-unknown"
            )
            .await
            .unwrap(),
        AgentSessionOpenOutcome::Paused
    );
    assert_eq!(
        lifecycle
            .reconcile_garbage_collection(
                "workflow-agent-unknown",
                "reconcile-workflow-agent-unknown",
            )
            .await
            .unwrap(),
        super::AgentSessionGarbageCollectionOutcome::Retained
    );
    let retained = sessions
        .find("workflow-agent-unknown")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        retained.session().lifecycle(),
        AgentSessionLifecycle::Paused
    );
    assert!(!retained.session().last_exit_abnormal());
    assert_eq!(retained.session().provider_session_id(), None);
    assert!(terminal.deletes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_実行木node終端停止_node不一致と停止失敗ではsettleしない() {
    let LifecycleTestContext {
        _directory,
        store,
        sessions,
        lifecycle,
        launches,
        terminal,
        ..
    } = setup();
    seed_workflow_session_facts(
        &store,
        WorkflowSessionFactSeed {
            workflow_name: "workflow",
            request: "test",
            worktree_path: "/repo/worktree",
            provider: ProviderKind::Claude,
            workflow_execution_id: "workflow-1",
            node_execution_id: "node-1",
            session_id: "workflow-agent",
            initial_instruction_admitted: true,
        },
    )
    .unwrap();
    sessions
        .create(
            "workflow-agent",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            workflow_location("workflow-1", "node-1"),
            "create-workflow-agent",
        )
        .await
        .unwrap();
    sessions
        .create(
            "manual-agent",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("manual-agent"),
            "create-manual-agent",
        )
        .await
        .unwrap();

    assert_eq!(
        lifecycle
            .stop_for_terminal_execution_tree_node_preserving_checkpoint(
                "workflow-agent",
                "different-node",
                "stop-wrong-node",
            )
            .await
            .unwrap_err(),
        super::AgentSessionLifecycleUsecaseError::InvalidOperation
    );
    assert_eq!(
        lifecycle
            .stop_for_terminal_execution_tree_node_preserving_checkpoint(
                "manual-agent",
                "node-1",
                "stop-manual-agent",
            )
            .await
            .unwrap_err(),
        super::AgentSessionLifecycleUsecaseError::InvalidOperation
    );
    assert!(terminal.stops.lock().unwrap().is_empty());

    *terminal.fail_stop.lock().unwrap() = true;
    assert_eq!(
        lifecycle
            .stop_for_terminal_execution_tree_node_preserving_checkpoint(
                "workflow-agent",
                "node-1",
                "stop-failure",
            )
            .await
            .unwrap_err(),
        super::AgentSessionLifecycleUsecaseError::TerminalUnavailable
    );
    assert_eq!(
        sessions
            .find("workflow-agent")
            .await
            .unwrap()
            .unwrap()
            .session()
            .lifecycle(),
        AgentSessionLifecycle::Open
    );
    assert!(launches.cleanups.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_agent_session_lifecycle_exit_resume_archive_restore_deleteを接続する() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        launches,
        terminal,
        provider_lifecycle,
        execution_trees,
        ..
    } = setup();
    sessions
        .create(
            "agent-1",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("agent-1"),
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
        AgentSessionOpenOutcome::Resumed
    );
    assert_eq!(
        launches.launches.lock().unwrap().as_slice(),
        &[ProviderSessionLaunch::resume("provider-1").unwrap()]
    );

    assert_eq!(
        lifecycle.archive("agent-1", "archive-1").await.unwrap(),
        AgentSessionArchiveOutcome::Archived
    );
    assert!(execution_trees.releases.lock().unwrap().is_empty());
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
        AgentSessionOpenOutcome::Restored
    );
    assert!(execution_trees.releases.lock().unwrap().is_empty());
    lifecycle.archive("agent-1", "archive-2").await.unwrap();
    lifecycle.delete("agent-1", "delete-1").await.unwrap();
    assert!(sessions.find("agent-1").await.unwrap().is_none());
    assert_eq!(terminal.deletes.lock().unwrap().len(), 1);
    assert_eq!(
        execution_trees.releases.lock().unwrap().as_slice(),
        &["agent-1"]
    );
    assert_eq!(
        launches.cleanups.lock().unwrap().as_slice(),
        &["agent-1", "agent-1", "agent-1", "agent-1", "agent-1"]
    );
}

#[tokio::test]
async fn test_agent_session_lifecycle_unknown_idのprocess_exitをgcする() {
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
            session_location("agent-gc"),
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
async fn test_agent_session_open_liveと生死不明では既存状態を破壊しない() {
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
            session_location("agent-open"),
            "create-open",
        )
        .await
        .unwrap();

    assert_eq!(
        lifecycle
            .open("agent-open", 24, 80, "open-live")
            .await
            .unwrap(),
        AgentSessionOpenOutcome::Attached
    );
    *terminal.presence.lock().unwrap() = ManagedPtyPresence::Unknown;
    assert_eq!(
        lifecycle
            .open("agent-open", 24, 80, "open-unknown")
            .await
            .unwrap(),
        AgentSessionOpenOutcome::Indeterminate
    );
    assert!(sessions.find("agent-open").await.unwrap().is_some());
    assert!(launches.launches.lock().unwrap().is_empty());
    assert!(terminal.deletes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn test_agent_session_open_known_idを自動resumeし失敗時はpausedにする() {
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
            session_location("agent-known"),
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
        AgentSessionOpenOutcome::Paused
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
async fn test_agent_session_open_pausedは明示resumeを待ちunknown_idはgcする() {
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
            session_location("agent-paused"),
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
        AgentSessionOpenOutcome::Paused
    );
    assert!(launches.launches.lock().unwrap().is_empty());

    sessions
        .create(
            "agent-orphan",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("agent-orphan"),
            "create-orphan",
        )
        .await
        .unwrap();
    assert_eq!(
        lifecycle
            .open("agent-orphan", 24, 80, "open-orphan")
            .await
            .unwrap(),
        AgentSessionOpenOutcome::GarbageCollected
    );
    assert!(sessions.find("agent-orphan").await.unwrap().is_none());
}

#[tokio::test]
async fn test_agent_session_gc再照合は確定不在かつunknown_idだけを削除する() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        terminal,
        execution_trees,
        ..
    } = setup();
    sessions
        .create(
            "agent-reconcile",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("agent-reconcile"),
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
        super::AgentSessionGarbageCollectionOutcome::Retained
    );
    assert!(sessions.find("agent-reconcile").await.unwrap().is_some());

    *terminal.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
    assert_eq!(
        lifecycle
            .reconcile_garbage_collection("agent-reconcile", "reconcile-absent")
            .await
            .unwrap(),
        super::AgentSessionGarbageCollectionOutcome::GarbageCollected
    );
    assert!(sessions.find("agent-reconcile").await.unwrap().is_none());
    assert_eq!(
        execution_trees.releases.lock().unwrap().as_slice(),
        &["agent-reconcile"]
    );
}

#[tokio::test]
async fn test_agent_session_delete_execution_cache解放失敗でも削除を完了する() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        execution_trees,
        ..
    } = setup();
    sessions
        .create(
            "agent-delete-release-failure",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("agent-delete-release-failure"),
            "create-delete-release-failure",
        )
        .await
        .unwrap();
    sessions
        .associate_provider_session(
            "agent-delete-release-failure",
            "provider-delete-release-failure",
            None,
            "associate-delete-release-failure",
        )
        .await
        .unwrap();
    lifecycle
        .archive(
            "agent-delete-release-failure",
            "archive-delete-release-failure",
        )
        .await
        .unwrap();
    *execution_trees.release_error.lock().unwrap() =
        Some(ExecutionTreeCacheReleaseError::Unavailable);

    lifecycle
        .delete("agent-delete-release-failure", "delete-release-failure")
        .await
        .unwrap();

    assert!(sessions
        .find("agent-delete-release-failure")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_agent_session_gc_execution_cache解放失敗でも削除を完了する() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        terminal,
        execution_trees,
        ..
    } = setup();
    sessions
        .create(
            "agent-gc-release-failure",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("agent-gc-release-failure"),
            "create-gc-release-failure",
        )
        .await
        .unwrap();
    *terminal.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
    *execution_trees.release_error.lock().unwrap() = Some(ExecutionTreeCacheReleaseError::Corrupt);

    assert_eq!(
        lifecycle
            .reconcile_garbage_collection("agent-gc-release-failure", "gc-release-failure",)
            .await
            .unwrap(),
        super::AgentSessionGarbageCollectionOutcome::GarbageCollected
    );
    assert!(sessions
        .find("agent-gc-release-failure")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_agent_session_resume_codexでも既知の配送失敗がなければ警告しない() {
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
            session_location("agent-hook"),
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
        AgentSessionOpenOutcome::Resumed
    );
    assert!(hook_health.warnings().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_agent_session_resume_spawn失敗時は未起動launchのhook警告を残さない() {
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
            session_location("agent-hook-spawn-failure"),
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
        super::AgentSessionLifecycleUsecaseError::TerminalUnavailable,
    );
    assert!(hook_health.warnings().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_agent_session_resume状態保存失敗時は起動済みprocessを停止する() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let repository = Arc::new(LocalAgentSessionRepository::new(store.clone()));
    let seed = AgentSessionUsecase::new(repository.clone());
    seed.create(
        "agent-save-failure",
        WorkspaceIdentity::new("/repo"),
        "/repo/worktree",
        ProviderKind::Claude,
        session_location("agent-save-failure"),
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
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
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
    let lifecycle = AgentSessionLifecycleUsecase::new(
        sessions,
        provider_lifecycle,
        ProviderAgentRuntime::new(
            Arc::new(AlwaysProviderAvailable),
            launches.clone(),
            terminal.clone(),
        ),
        hook_health,
        Arc::new(RecordingChangeNotifier::default()),
        Arc::new(RecordingExecutionTrees::default()),
    );

    assert_eq!(
        lifecycle
            .resume("agent-save-failure", 24, 80, "resume-save-failure")
            .await
            .unwrap_err(),
        super::AgentSessionLifecycleUsecaseError::StorageUnavailable
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
async fn test_agent_session_resume_残存bindingを解放して単一launchにする() {
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
            session_location("agent-stale-binding"),
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
        AgentSessionOpenOutcome::Resumed
    );
    let active = provider_lifecycle
        .active_launch_id(ProviderKind::Claude, &scope)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(active, old_slot);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_agent_session_resume_同一sessionへの並行要求はptyを一度だけ起動する() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        LocalAgentSessionRepository::new(store.clone()),
    )));
    sessions
        .create(
            "agent-concurrent-resume",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("agent-concurrent-resume"),
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
    let lifecycle = Arc::new(AgentSessionLifecycleUsecase::new(
        sessions,
        provider_lifecycle,
        ProviderAgentRuntime::new(
            Arc::new(AlwaysProviderAvailable),
            launches,
            terminal.clone(),
        ),
        Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealthRepository::default(),
        ))),
        Arc::new(RecordingChangeNotifier::default()),
        Arc::new(RecordingExecutionTrees::default()),
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
            .filter(|outcome| { **outcome == Ok(AgentSessionOpenOutcome::Resumed) })
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| {
                **outcome == Err(super::AgentSessionLifecycleUsecaseError::InvalidOperation)
            })
            .count(),
        1
    );
    assert_eq!(*terminal.spawn_count.lock().unwrap(), 1);
    assert!(terminal.stops.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_agent_session_resume中のarchiveは同一sessionの操作完了後に実行する() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        LocalAgentSessionRepository::new(store.clone()),
    )));
    sessions
        .create(
            "agent-resume-archive",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Claude,
            session_location("agent-resume-archive"),
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
    let lifecycle = Arc::new(AgentSessionLifecycleUsecase::new(
        sessions.clone(),
        provider_lifecycle,
        ProviderAgentRuntime::new(
            Arc::new(AlwaysProviderAvailable),
            launches,
            terminal.clone(),
        ),
        Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealthRepository::default(),
        ))),
        Arc::new(RecordingChangeNotifier::default()),
        Arc::new(RecordingExecutionTrees::default()),
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
        AgentSessionOpenOutcome::Resumed
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
async fn test_agent_session_open_同一sessionへの並行要求は一度だけ自動resumeする() {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().to_path_buf(),
    ))
    .unwrap();
    let sessions = Arc::new(AgentSessionUsecase::new(Arc::new(
        LocalAgentSessionRepository::new(store.clone()),
    )));
    sessions
        .create(
            "agent-concurrent-open",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            session_location("agent-concurrent-open"),
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
    let lifecycle = Arc::new(AgentSessionLifecycleUsecase::new(
        sessions,
        Arc::new(ProviderLifecycleUsecase::new(
            Arc::new(LocalProviderLifecycleCredentialGateway),
            Arc::new(NoopLifecycleEvents),
        )),
        ProviderAgentRuntime::new(
            Arc::new(AlwaysProviderAvailable),
            Arc::new(RecordingResumeLaunches::default()),
            terminal.clone(),
        ),
        Arc::new(ProviderHookHealthUsecase::new(Arc::new(
            MemoryHookHealthRepository::default(),
        ))),
        Arc::new(RecordingChangeNotifier::default()),
        Arc::new(RecordingExecutionTrees::default()),
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
    assert!(outcomes.contains(&AgentSessionOpenOutcome::Resumed));
    assert!(outcomes.contains(&AgentSessionOpenOutcome::Attached));
    assert_eq!(*terminal.spawn_count.lock().unwrap(), 1);
    assert!(terminal.stops.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_agent_session_restore中のdeleteは復帰完了後の状態で拒否する() {
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
            session_location("agent-restore-delete"),
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
        AgentSessionOpenOutcome::Restored
    );
    assert_eq!(
        delete.await.unwrap().unwrap_err(),
        super::AgentSessionLifecycleUsecaseError::InvalidOperation
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
async fn test_agent_session_exit_open待機中に旧世代になったexitを反映しない() {
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
            session_location("agent-open-stale-exit"),
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
        AgentSessionOpenOutcome::Resumed
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
async fn test_agent_session_archive縮退delete中のopenはdelete完了後に評価する() {
    let LifecycleTestContext {
        _directory,
        sessions,
        lifecycle,
        terminal,
        execution_trees,
        ..
    } = setup();
    sessions
        .create(
            "agent-fallback-delete-open",
            WorkspaceIdentity::new("/repo"),
            "/repo/worktree",
            ProviderKind::Codex,
            session_location("agent-fallback-delete-open"),
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
        execution_trees.releases.lock().unwrap().as_slice(),
        &["agent-fallback-delete-open"]
    );
    assert_eq!(
        open.await.unwrap().unwrap_err(),
        super::AgentSessionLifecycleUsecaseError::NotFound
    );
    assert!(sessions
        .find("agent-fallback-delete-open")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn test_agent_session_lifecycle_exit由来のpaused遷移で変更通知を発火する() {
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
            session_location("agent-notify"),
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

#[tokio::test]
async fn test_agent_session_open_未対応の親または自身の定義があってもattachとprovider再開を行える()
{
    // Given
    for unavailable in ["main", "session", "unused"] {
        let context = setup();
        crate::adaptor::gateway::workflow::test_support::seed_unavailable_definition(
            &context.store,
            "tree",
            "/repo",
            unavailable,
        );

        // When
        let attached = context
            .lifecycle
            .open("tree-session", 24, 80, "attach")
            .await
            .unwrap();
        assert_eq!(*context.terminal.spawn_count.lock().unwrap(), 0);
        *context.terminal.presence.lock().unwrap() = ManagedPtyPresence::ConfirmedAbsent;
        let resumed = context
            .lifecycle
            .open("tree-session", 24, 80, "resume")
            .await
            .unwrap();

        // Then
        assert_eq!(attached, AgentSessionOpenOutcome::Attached);
        assert_eq!(resumed, AgentSessionOpenOutcome::Resumed);
        assert_eq!(*context.terminal.spawn_count.lock().unwrap(), 1);
        assert_eq!(
            context.launches.launches.lock().unwrap().as_slice(),
            &[ProviderSessionLaunch::resume("provider-session").unwrap()]
        );
        assert_eq!(
            context
                .sessions
                .find("tree-session")
                .await
                .unwrap()
                .unwrap()
                .session()
                .lifecycle(),
            AgentSessionLifecycle::Open
        );
    }
}
