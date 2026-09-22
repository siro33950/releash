use super::*;
use crate::adaptor::gateway::local_event_store::{LocalEventStore, LocalEventStoreConfig};
use crate::adaptor::gateway::workflow::node_session_boundary::NodeSessionInfo;
use crate::domain::provider_lifecycle::ProviderKind;
use crate::domain::workflow::{IsolatedWorktree, IsolatedWorktreeGateway};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex as StdMutex;

#[derive(Default)]
pub(super) struct TestWorktrees {
    pub(super) calls: StdMutex<Vec<(String, IsolatedWorktree)>>,
    pub(super) failures: AtomicUsize,
}

impl IsolatedWorktreeGateway for TestWorktrees {
    fn repository_root(
        &self,
        _path: &str,
    ) -> Result<String, crate::domain::workflow::WorkflowError> {
        Ok("/repo".into())
    }
    fn is_created(
        &self,
        _parent: &str,
        _worktree: &IsolatedWorktree,
    ) -> Result<bool, crate::domain::workflow::WorkflowError> {
        Ok(false)
    }
    fn create(
        &self,
        parent: &str,
        worktree: &IsolatedWorktree,
    ) -> Result<(), crate::domain::workflow::WorkflowError> {
        self.calls
            .lock()
            .unwrap()
            .push((parent.into(), worktree.clone()));
        if self
            .failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_sub(1)
            })
            .is_ok()
        {
            return Err(crate::domain::workflow::WorkflowError::external(
                "creation failed",
            ));
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct TestSessions {
    pub(super) initial_instructions: StdMutex<Vec<String>>,
    pub(super) presence_unknown: AtomicBool,
    pub(crate) live_sessions: StdMutex<std::collections::HashSet<String>>,
    pub(super) conversation_missing: AtomicBool,
    pub(super) prepared: StdMutex<Vec<(String, String, String)>>,
    pub(super) activated: StdMutex<Vec<String>>,
    pub(super) recovered: StdMutex<Vec<String>>,
    pub(super) continuations: StdMutex<Vec<(String, String)>>,
    pub(super) admitted_continuations: StdMutex<std::collections::BTreeSet<(String, String)>>,
    pub(super) continuation_fails: AtomicBool,
    pub(super) block_continuation: AtomicBool,
    pub(super) continuation_entered: tokio::sync::Notify,
    pub(super) continuation_release: tokio::sync::Notify,
    pub(super) recovery_fails: AtomicBool,
    pub(super) stop_fails: AtomicBool,
    pub(super) preparation_fails: AtomicBool,
    pub(super) block_preparation: AtomicBool,
    pub(super) preparation_entered: tokio::sync::Notify,
    pub(super) preparation_release: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl WorkflowAgentSessionPort for TestSessions {
    async fn has_recoverable_conversation(&self, _id: &str) -> Result<bool, WorkflowRuntimeError> {
        Ok(!self.conversation_missing.load(Ordering::SeqCst))
    }

    fn is_provider_available(&self, _provider: ProviderKind) -> bool {
        true
    }
    async fn prepare_workflow_agent_session(
        &self,
        workspace: &str,
        cwd: &str,
        _config: WorkflowSessionLaunchConfig,
        _execution_id: &str,
        node_id: &str,
        instruction: &str,
    ) -> Result<NodeSessionInfo, WorkflowRuntimeError> {
        if self.block_preparation.swap(false, Ordering::SeqCst) {
            self.preparation_entered.notify_one();
            self.preparation_release.notified().await;
        }
        self.prepared
            .lock()
            .unwrap()
            .push((workspace.into(), cwd.into(), node_id.into()));
        if self.preparation_fails.load(Ordering::SeqCst) {
            return Err(WorkflowRuntimeError::AgentSession("prepare failed".into()));
        }
        self.initial_instructions
            .lock()
            .unwrap()
            .push(instruction.into());
        Ok(NodeSessionInfo {
            id: format!("agent-{node_id}"),
        })
    }
    async fn activate_workflow_agent_session(
        &self,
        session_id: &str,
        node_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.activated.lock().unwrap().push(node_id.into());
        self.live_sessions.lock().unwrap().insert(session_id.into());
        Ok(())
    }
    async fn confirm_workflow_agent_session_attachment(
        &self,
        _session_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        Ok(())
    }
    async fn dispatch_continuation(
        &self,
        session_id: &str,
        child_execution_id: &str,
        instruction: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        if self.block_continuation.load(Ordering::SeqCst) {
            self.continuation_entered.notify_one();
            self.continuation_release.notified().await;
        }
        if self.continuation_fails.load(Ordering::SeqCst) {
            return Err(WorkflowRuntimeError::AgentSession(
                "continuation failed".into(),
            ));
        }
        // 本物の port と同じく child ごとに一度だけ受理する。
        if !self
            .admitted_continuations
            .lock()
            .unwrap()
            .insert((session_id.to_string(), child_execution_id.to_string()))
        {
            return Ok(());
        }
        self.continuations
            .lock()
            .unwrap()
            .push((session_id.into(), instruction.into()));
        Ok(())
    }

    async fn recover_workflow_agent_session_provider(
        &self,
        session_id: &str,
        node_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.recovered.lock().unwrap().push(node_id.into());
        if self.recovery_fails.load(Ordering::SeqCst) {
            Err(WorkflowRuntimeError::AgentSession(
                "worktree is missing".into(),
            ))
        } else {
            self.live_sessions.lock().unwrap().insert(session_id.into());
            Ok(())
        }
    }
    async fn stop_agent_session_for_terminal_node_preserving_checkpoint(
        &self,
        session_id: &str,
        _node_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        if self.stop_fails.load(Ordering::SeqCst) {
            return Err(WorkflowRuntimeError::AgentSession("stop failed".into()));
        }
        self.live_sessions.lock().unwrap().remove(session_id);
        Ok(())
    }
    async fn rollback_workflow_agent_session(
        &self,
        session_id: &str,
        _node_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.live_sessions.lock().unwrap().remove(session_id);
        Ok(())
    }
}

impl crate::domain::agent_session::ProviderAgentTerminalGateway for TestSessions {
    fn spawn(
        &self,
        _owner: crate::domain::terminal_surface::TerminalSurfaceOwner,
        _path: &str,
        _process: crate::domain::terminal_surface::TerminalProcessLaunch,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), crate::domain::agent_session::ProviderAgentTerminalSpawnError> {
        panic!("test session launch uses WorkflowAgentSessionPort")
    }
    fn presence(
        &self,
        owner: &crate::domain::terminal_surface::TerminalSurfaceOwner,
    ) -> Result<
        crate::domain::agent_session::aggregates::ManagedPtyPresence,
        crate::domain::agent_session::ProviderAgentTerminalGatewayError,
    > {
        use crate::domain::agent_session::aggregates::ManagedPtyPresence;
        let crate::domain::terminal_surface::TerminalSurfaceOwner::Session { session_id, .. } =
            owner
        else {
            panic!("expected Session owner")
        };
        if self.presence_unknown.load(Ordering::SeqCst) {
            return Ok(crate::domain::agent_session::aggregates::ManagedPtyPresence::Unknown);
        }
        Ok(if self.live_sessions.lock().unwrap().contains(session_id) {
            ManagedPtyPresence::Live
        } else {
            ManagedPtyPresence::ConfirmedAbsent
        })
    }
    fn stop_preserving_checkpoint(
        &self,
        _owner: &crate::domain::terminal_surface::TerminalSurfaceOwner,
    ) -> Result<(), crate::domain::agent_session::ProviderAgentTerminalGatewayError> {
        panic!("unexpected terminal stop")
    }
    fn delete(
        &self,
        _owner: &crate::domain::terminal_surface::TerminalSurfaceOwner,
    ) -> Result<(), crate::domain::agent_session::ProviderAgentTerminalGatewayError> {
        panic!("unexpected terminal deletion")
    }
    fn is_current_runtime_generation(
        &self,
        _owner: &crate::domain::terminal_surface::TerminalSurfaceOwner,
        _generation: u64,
    ) -> Result<bool, crate::domain::agent_session::ProviderAgentTerminalGatewayError> {
        panic!("unexpected generation lookup")
    }
}

pub(crate) struct Fixture {
    pub(crate) _directory: tempfile::TempDir,
    pub(crate) app: WorkflowRuntimeDependencies,
    pub(super) store: Arc<LocalEventStore>,
    pub(crate) host: WorkflowRuntimeHost,
    pub(super) worktrees: Arc<TestWorktrees>,
    pub(super) sessions: Arc<TestSessions>,
}

impl Fixture {
    pub(crate) fn new(failures: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store =
            LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into()))
                .unwrap();
        let mut app = test_helpers::dependencies(Some(store.clone()));
        let worktrees = Arc::new(TestWorktrees {
            failures: AtomicUsize::new(failures),
            ..Default::default()
        });
        let sessions = Arc::new(TestSessions::default());
        let mut host = WorkflowRuntimeHost::with_execution_store(
            Arc::new(super::workflow_host_tests::UnusedWorkflowResolver),
            Arc::new(super::workflow_host_tests::AcceptingWorktreeResolver),
            Arc::new(ExecutionStore::new_in_memory_for_tests()),
            sessions.clone(),
            worktrees.clone(),
        );
        let processes = Arc::new(
            crate::adaptor::gateway::workflow::node_process::WorkflowNodeProcesses::new(
                sessions.clone(),
            ),
        );
        host.node_processes = processes.clone();
        app.processes = processes;
        let host =
            crate::adaptor::controller::wiring::wire_delegate_continuation(app.clone(), host);
        Self {
            _directory: directory,
            app,
            store,
            host,
            worktrees,
            sessions,
        }
    }

    pub(super) async fn start(&self, nodes: &str) -> String {
        self.start_at(nodes, "/repo-worktrees/development").await
    }

    pub(super) async fn wait_startup_retries(&self) {
        wait_startup_retries(&self.host).await;
    }

    pub(super) async fn start_at(&self, nodes: &str, root: &str) -> String {
        let definition = serde_saphyr::from_str(&format!(
            "name: isolated\ndescription: test\nnodes:\n{nodes}"
        ))
        .unwrap();
        self.host
            .start_resolved_workflow(
                &self.app,
                definition,
                root.into(),
                None,
                ExecutionOrigin::Cli,
            )
            .await
            .unwrap()
    }

    pub(super) fn with_repository() -> (Self, String) {
        let mut fixture = Self::new(0);
        let root = fixture._directory.path().join("repository");
        let repo = git2::Repository::init(&root).unwrap();
        crate::test_support::git::create_initial_commit(&repo);
        fixture.host.isolated_worktrees =
            Arc::new(crate::adaptor::gateway::workflow::RepositoryIsolatedWorktreeGateway);
        (
            fixture,
            root.canonicalize().unwrap().to_string_lossy().into_owned(),
        )
    }

    pub(super) async fn persist_started(&self, nodes: &str, root: &str) -> RuntimeCommitSnapshot {
        let workflow: WorkflowDefinition = serde_saphyr::from_str(&format!(
            "name: isolated\ndescription: test\nnodes:\n{nodes}"
        ))
        .unwrap();
        let now = current_timestamp();
        let (snapshot, applied) = self
            .host
            .insert_workflow_execution(WorkflowExecutionInsert {
                execution_id: uuid::Uuid::new_v4().to_string(),
                workflow: workflow.clone(),
                worktree_path: root.into(),
                request: None,
                created_from: ExecutionOrigin::Cli,
                workflow_defaults: WorkflowDefaults,
                now,
            })
            .await
            .unwrap();
        let mut events = vec![WorkflowEvent::ExecutionStarted {
            repository_root: snapshot.repository_root.clone(),
            execution_id: snapshot.execution_id.clone(),
            workflow_name: workflow.name.clone(),
            worktree_path: root.into(),
            created_from: ExecutionOrigin::Cli,
            request: String::new(),
            definition: workflow,
            timestamp: now,
        }];
        events.extend(applied.events);
        self.host
            .write_log_required_batch(&self.app, &events)
            .unwrap();
        snapshot
    }

    pub(super) fn restarted_host(&self) -> WorkflowRuntimeHost {
        let mut host = WorkflowRuntimeHost::with_execution_store(
            Arc::new(super::workflow_host_tests::UnusedWorkflowResolver),
            Arc::new(super::workflow_host_tests::AcceptingWorktreeResolver),
            Arc::new(ExecutionStore::new_in_memory_for_tests()),
            self.sessions.clone(),
            self.host.isolated_worktrees.clone(),
        );
        host.node_processes = self.host.node_processes.clone();
        crate::adaptor::controller::wiring::wire_delegate_continuation(self.app.clone(), host)
    }

    pub(super) async fn command_artifact(&self, execution_id: &str) -> serde_json::Value {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let folded = workflow_fact_log::fold_tree_from(
                    &workflow_fact_log::FactLogReadBackend::Live(self.store.clone()),
                    execution_id,
                )
                .unwrap()
                .unwrap();
                if let Some(node) = folded
                    .aggregate
                    .node_executions
                    .iter()
                    .find(|node| node.kind == NodeKindName::Command && !node.status.is_active())
                {
                    assert_eq!(node.status, NodeExecutionStatus::Succeeded, "{node:?}");
                    return node.artifact.clone().unwrap();
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("command must complete")
    }
}

pub(super) async fn wait_startup_retries(host: &WorkflowRuntimeHost) {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let pending = host
                .startup_retries
                .lock()
                .await
                .values()
                .map(|task| task.cancel.subscribe())
                .collect::<Vec<_>>();
            if pending.is_empty() {
                break;
            }
            for mut completion in pending {
                let _ = completion.changed().await;
            }
        }
    })
    .await
    .expect("startup retries must finish");
}

pub(super) fn record_workflow_execution_broadcasts(
    app: &WorkflowRuntimeDependencies,
) -> tokio::sync::broadcast::Receiver<Arc<[u8]>> {
    app.push.subscribe()
}

pub(super) fn take_workflow_execution_broadcasts(
    receiver: &mut tokio::sync::broadcast::Receiver<Arc<[u8]>>,
) -> Vec<crate::adaptor::protocol::workflow::WorkflowExecutionChangedPayloadView> {
    use crate::adaptor::protocol::client as wire;
    use prost::Message;
    let mut broadcasts = Vec::new();
    loop {
        let frame = match receiver.try_recv() {
            Ok(frame) => frame,
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
            Err(error) => panic!("push reception failed: {error}"),
        };
        let push = wire::Push::decode(frame.as_ref()).unwrap();
        let (event, payload) = push.into_value().unwrap();
        if event == "workflow-execution-changed" {
            broadcasts.push(serde_json::from_value(payload).unwrap());
        }
    }
    broadcasts
}

pub(super) fn dependencies(store: Option<Arc<LocalEventStore>>) -> WorkflowRuntimeDependencies {
    WorkflowRuntimeDependencies {
        processes: Arc::new(
            crate::adaptor::gateway::workflow::node_process::WorkflowNodeProcesses::default(),
        ),
        store,
        config: None,
        secrets: None,
        push: Arc::new(crate::infrastructure::push::PushSink::new()),
    }
}

use super::workflow_host_tests::{AcceptingWorktreeResolver, UnusedWorkflowResolver};
use crate::adaptor::gateway::workflow::{
    ExecutionTreeArchiveFactRepository, WorkflowRuntimeCommandGateway,
};
use crate::adaptor::gateway::workspace_tree::{
    SqliteWorkspaceQueryService, SqliteWorkspaceTreeRepository,
};
pub(crate) struct ArchiveFixture {
    pub(crate) directory: tempfile::TempDir,
    pub(crate) store: Arc<LocalEventStore>,
    pub(crate) repository: Arc<ExecutionTreeArchiveFactRepository>,
    pub(crate) host: Arc<WorkflowRuntimeHost>,
    pub(crate) runtime: crate::usecase::workflow::WorkflowRuntimeUsecase,
    pub(crate) app: WorkflowRuntimeDependencies,
    pub(crate) sessions: Arc<TestSessions>,
    pub(crate) query: Arc<SqliteWorkspaceQueryService>,
}

pub(crate) fn archive_fixture() -> ArchiveFixture {
    archive_fixture_with_resolver(Arc::new(AcceptingWorktreeResolver))
}

pub(crate) fn archive_fixture_with_resolver(
    resolver: Arc<dyn ManagedWorktreeResolver>,
) -> ArchiveFixture {
    let directory = tempfile::tempdir().unwrap();
    let store =
        LocalEventStore::open(LocalEventStoreConfig::production(directory.path().into())).unwrap();
    let repository = Arc::new(ExecutionTreeArchiveFactRepository::new(
        store.clone(),
        directory.path(),
    ));
    let query = SqliteWorkspaceQueryService::with_repository(
        SqliteWorkspaceTreeRepository::new(store.clone()),
        repository.clone(),
    );
    let app = test_helpers::dependencies(Some(store.clone()));
    let sessions = Arc::new(TestSessions::default());
    let host = Arc::new(WorkflowRuntimeHost::with_execution_store(
        Arc::new(UnusedWorkflowResolver),
        resolver,
        Arc::new(ExecutionStore::new_canonical(query.clone())),
        sessions.clone(),
        Arc::new(TestWorktrees::default()),
    ));
    let runtime = crate::usecase::workflow::WorkflowRuntimeUsecase::new(
        Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(
            app.clone(),
            host.clone(),
        )),
        repository.clone(),
    );
    ArchiveFixture {
        directory,
        store,
        repository,
        host,
        runtime,
        app,
        sessions,
        query,
    }
}

pub(crate) async fn archive_workflow(fixture: &ArchiveFixture) -> String {
    let workflow = serde_saphyr::from_str("name: archive\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}").unwrap();
    fixture
        .host
        .start_resolved_workflow(
            &fixture.app,
            workflow,
            "/missing/worktree".into(),
            None,
            ExecutionOrigin::Cli,
        )
        .await
        .unwrap()
}
