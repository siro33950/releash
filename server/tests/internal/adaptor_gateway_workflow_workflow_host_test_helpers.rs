use releashd::test_support::integration::persistence::LocalEventStore;
use releashd::test_support::integration::persistence::LocalEventStoreConfig;
use releashd::test_support::integration::providers::ProviderKind;
use releashd::test_support::integration::workflow::current_timestamp;
use releashd::test_support::integration::workflow::ExecutionOrigin;
use releashd::test_support::integration::workflow::IsolatedWorktree;
use releashd::test_support::integration::workflow::IsolatedWorktreeGateway;
use releashd::test_support::integration::workflow::ManagedWorktreeResolver;
use releashd::test_support::integration::workflow::NodeExecutionStatus;
use releashd::test_support::integration::workflow::NodeKindName;
use releashd::test_support::integration::workflow::NodeSessionInfo;
use releashd::test_support::integration::workflow::RuntimeCommitSnapshot;
use releashd::test_support::integration::workflow::WorkflowAgentSessionPort;
use releashd::test_support::integration::workflow::WorkflowDefaults;
use releashd::test_support::integration::workflow::WorkflowDefinition;
use releashd::test_support::integration::workflow::WorkflowEvent;
use releashd::test_support::integration::workflow::WorkflowExecutionInsert;
use releashd::test_support::integration::workflow::WorkflowRuntimeDependencies;
use releashd::test_support::integration::workflow::WorkflowRuntimeError;
use releashd::test_support::integration::workflow::WorkflowRuntimeHost;
use releashd::test_support::integration::workflow::WorkflowSessionLaunchConfig;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

#[derive(Default)]
pub(crate) struct TestWorktrees {
    pub(crate) calls: StdMutex<Vec<(String, IsolatedWorktree)>>,
    pub(crate) failures: AtomicUsize,
    pub(crate) creation_barrier: StdMutex<Option<Arc<std::sync::Barrier>>>,
}

impl IsolatedWorktreeGateway for TestWorktrees {
    fn repository_root(
        &self,
        _path: &str,
    ) -> Result<String, releashd::test_support::integration::workflow::WorkflowError> {
        Ok("/repo".into())
    }
    fn is_created(
        &self,
        _parent: &str,
        _worktree: &IsolatedWorktree,
    ) -> Result<bool, releashd::test_support::integration::workflow::WorkflowError> {
        Ok(false)
    }
    fn create(
        &self,
        parent: &str,
        worktree: &IsolatedWorktree,
    ) -> Result<(), releashd::test_support::integration::workflow::WorkflowError> {
        let barrier = self.creation_barrier.lock().unwrap().clone();
        if let Some(barrier) = barrier {
            barrier.wait();
        }
        self.calls
            .lock()
            .unwrap()
            .push((parent.into(), worktree.clone()));
        if self
            .failures
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                count.checked_sub(1)
            })
            .is_ok()
        {
            return Err(releashd::test_support::integration::workflow::WorkflowError::Store(
                releashd::test_support::integration::platform::StorageFailure::from(
                    releashd::test_support::integration::platform::CommitBatchError::TreeHeadConflict,
                )
                .with_message("creation failed"),
            ));
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct TestSessions {
    pub(crate) initial_instructions: StdMutex<Vec<String>>,
    pub(crate) presence_unknown: AtomicBool,
    pub(crate) presence_error_session: StdMutex<Option<String>>,
    pub(crate) live_sessions: StdMutex<std::collections::HashSet<String>>,
    pub(crate) conversation_missing: AtomicBool,
    pub(crate) prepared: StdMutex<Vec<(String, String, String)>>,
    pub(crate) activated: StdMutex<Vec<String>>,
    pub(crate) rolled_back: StdMutex<Vec<String>>,
    pub(crate) recovered: StdMutex<Vec<String>>,
    pub(crate) continuations: StdMutex<Vec<(String, String)>>,
    pub(crate) admitted_continuations: StdMutex<std::collections::BTreeSet<(String, String)>>,
    pub(crate) continuation_fails: AtomicBool,
    pub(crate) block_continuation: AtomicBool,
    pub(crate) continuation_entered: tokio::sync::Notify,
    pub(crate) continuation_release: tokio::sync::Notify,
    pub(crate) recovery_fails: AtomicBool,
    pub(crate) stop_fails: AtomicBool,
    pub(crate) preparation_fails: AtomicBool,
    pub(crate) preparation_conflicts: AtomicBool,
    pub(crate) block_preparation: AtomicBool,
    pub(crate) preparation_entered: tokio::sync::Notify,
    pub(crate) preparation_release: tokio::sync::Notify,
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
        if self.preparation_conflicts.load(Ordering::SeqCst) {
            return Err(WorkflowRuntimeError::Conflict("prepare conflicted".into()));
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
        node_id: &str,
    ) -> Result<(), WorkflowRuntimeError> {
        self.rolled_back.lock().unwrap().push(node_id.into());
        self.live_sessions.lock().unwrap().remove(session_id);
        Ok(())
    }
}

impl releashd::test_support::integration::sessions::ProviderAgentTerminalGateway for TestSessions {
    fn spawn(
        &self,
        _owner: releashd::test_support::integration::terminal::TerminalSurfaceOwner,
        _path: &str,
        _process: releashd::test_support::integration::terminal::TerminalProcessLaunch,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), releashd::test_support::integration::sessions::ProviderAgentTerminalGatewayError>
    {
        panic!("test session launch uses WorkflowAgentSessionPort")
    }
    fn presence(
        &self,
        owner: &releashd::test_support::integration::terminal::TerminalSurfaceOwner,
    ) -> Result<
        releashd::test_support::integration::sessions::ManagedPtyPresence,
        releashd::test_support::integration::sessions::ProviderAgentTerminalGatewayError,
    > {
        use releashd::test_support::integration::sessions::ManagedPtyPresence;
        let releashd::test_support::integration::terminal::TerminalSurfaceOwner::Session {
            session_id,
            ..
        } = owner
        else {
            panic!("expected Session owner")
        };
        if self.presence_error_session.lock().unwrap().as_deref() == Some(session_id.as_str()) {
            return Err(
                releashd::test_support::integration::sessions::ProviderAgentTerminalGatewayError::Technical(
                    releashd::test_support::integration::platform::TechnicalFailure {
                        nature: releashd::test_support::integration::platform::TechnicalFailureNature::Transient,
                        message: "unavailable".into(),
                    },
                ),
            );
        }
        if self.presence_unknown.load(Ordering::SeqCst) {
            return Ok(releashd::test_support::integration::sessions::ManagedPtyPresence::Unknown);
        }
        Ok(if self.live_sessions.lock().unwrap().contains(session_id) {
            ManagedPtyPresence::Live
        } else {
            ManagedPtyPresence::ConfirmedAbsent
        })
    }
    fn stop_preserving_checkpoint(
        &self,
        _owner: &releashd::test_support::integration::terminal::TerminalSurfaceOwner,
    ) -> Result<(), releashd::test_support::integration::sessions::ProviderAgentTerminalGatewayError>
    {
        panic!("unexpected terminal stop")
    }
    fn delete(
        &self,
        _owner: &releashd::test_support::integration::terminal::TerminalSurfaceOwner,
    ) -> Result<(), releashd::test_support::integration::sessions::ProviderAgentTerminalGatewayError>
    {
        panic!("unexpected terminal deletion")
    }
    fn is_current_runtime_generation(
        &self,
        _owner: &releashd::test_support::integration::terminal::TerminalSurfaceOwner,
        _generation: u64,
    ) -> Result<
        bool,
        releashd::test_support::integration::sessions::ProviderAgentTerminalGatewayError,
    > {
        panic!("unexpected generation lookup")
    }
}

pub(crate) struct Fixture {
    pub(crate) _directory: tempfile::TempDir,
    pub(crate) app: WorkflowRuntimeDependencies,
    pub(crate) store: Arc<LocalEventStore>,
    pub(crate) host: WorkflowRuntimeHost,
    pub(crate) worktrees: Arc<TestWorktrees>,
    pub(crate) sessions: Arc<TestSessions>,
}

impl Fixture {
    pub(crate) fn daemon_repository(
        &self,
    ) -> Arc<releashd::test_support::integration::daemon::InMemoryDaemonRepository> {
        self.host.test_daemon().clone()
    }
    pub(crate) fn new(failures: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(LocalEventStoreConfig::production(
            directory.path().into(),
            std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
        ))
        .unwrap();
        let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(Some(
            store.clone(),
        ));
        let worktrees = Arc::new(TestWorktrees {
            failures: AtomicUsize::new(failures),
            ..Default::default()
        });
        let sessions = Arc::new(TestSessions::default());
        let mut host = WorkflowRuntimeHost::with_runtime_ports(
            releashd::test_support::integration::platform::shared().clone(),
            Arc::new(crate::adaptor_gateway_workflow_workflow_host::workflow_host_tests::UnusedWorkflowResolver),
            Arc::new(crate::adaptor_gateway_workflow_workflow_host::workflow_host_tests::AcceptingWorktreeResolver),
            workspace_query(store.clone()),
            sessions.clone(),
            worktrees.clone(),
            releashd::test_support::integration::daemon::serving(),
        );
        let processes = Arc::new(
            releashd::test_support::integration::workflow::WorkflowNodeProcesses::new(
                sessions.clone(),
            ),
        );
        host.node_processes = processes;
        let host = releashd::test_support::integration::platform::wire_delegate_continuation(
            app.clone(),
            host,
        );
        Self {
            _directory: directory,
            app,
            store,
            host,
            worktrees,
            sessions,
        }
    }

    pub(crate) async fn start(&self, nodes: &str) -> String {
        self.start_at(nodes, "/repo-worktrees/development").await
    }

    pub(crate) async fn wait_startup_retries(&self) {
        wait_startup_retries(&self.host).await;
    }

    pub(crate) async fn start_at(&self, nodes: &str, root: &str) -> String {
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

    pub(crate) fn with_repository() -> (Self, String) {
        let mut fixture = Self::new(0);
        let root = fixture._directory.path().join("repository");
        let repo = git2::Repository::init(&root).unwrap();
        crate::test_support_git::create_initial_commit(&repo);
        *fixture.host.test_isolated_worktrees_mut() = Arc::new(
            releashd::test_support::integration::workflow::RepositoryIsolatedWorktreeGateway,
        );
        (
            fixture,
            root.canonicalize().unwrap().to_string_lossy().into_owned(),
        )
    }

    pub(crate) async fn persist_started(&self, nodes: &str, root: &str) -> RuntimeCommitSnapshot {
        let workflow: WorkflowDefinition = serde_saphyr::from_str(&format!(
            "name: isolated\ndescription: test\nnodes:\n{nodes}"
        ))
        .unwrap();
        let now = current_timestamp();
        let (snapshot, applied) = self
            .host
            .insert_workflow_execution(WorkflowExecutionInsert::test_new(
                uuid::Uuid::new_v4().to_string(),
                workflow.clone(),
                root.into(),
                None,
                ExecutionOrigin::Cli,
                WorkflowDefaults,
                now,
            ))
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
            .await
            .unwrap();
        snapshot
    }

    pub(crate) fn restarted_host(&self) -> WorkflowRuntimeHost {
        let mut host = WorkflowRuntimeHost::with_runtime_ports(
            releashd::test_support::integration::platform::shared().clone(),
            Arc::new(crate::adaptor_gateway_workflow_workflow_host::workflow_host_tests::UnusedWorkflowResolver),
            Arc::new(crate::adaptor_gateway_workflow_workflow_host::workflow_host_tests::AcceptingWorktreeResolver),
            self.host.test_workspace_query().clone(),
            self.sessions.clone(),
            self.host.test_isolated_worktrees().clone(),
            releashd::test_support::integration::daemon::serving(),
        );
        host.node_processes = self.host.node_processes.clone();
        releashd::test_support::integration::platform::wire_delegate_continuation(
            self.app.clone(),
            host,
        )
    }

    pub(crate) async fn command_artifact(&self, execution_id: &str) -> serde_json::Value {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let folded = releashd::test_support::integration::workflow::fold_tree_from(
                    &releashd::test_support::integration::workflow::FactLogReadBackend::Live(
                        self.store.clone(),
                    ),
                    execution_id,
                )
                .await
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

pub(crate) async fn wait_startup_retries(host: &WorkflowRuntimeHost) {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let pending = host
                .test_startup_retries()
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

pub(crate) fn record_workflow_execution_broadcasts(
    app: &WorkflowRuntimeDependencies,
) -> tokio::sync::broadcast::Receiver<
    releashd::test_support::integration::subscriptions::StateChangeSource,
> {
    releashd::test_support::integration::subscriptions::changes(&app.state_changes)
}

pub(crate) fn take_workflow_execution_broadcasts(
    receiver: &mut tokio::sync::broadcast::Receiver<
        releashd::test_support::integration::subscriptions::StateChangeSource,
    >,
) -> Vec<releashd::test_support::integration::subscriptions::StateChangeSource> {
    releashd::test_support::integration::subscriptions::take_changes(receiver)
}

pub(crate) fn workspace_query(store: Arc<LocalEventStore>) -> Arc<SqliteWorkspaceQueryService> {
    SqliteWorkspaceQueryService::with_repository(SqliteWorkspaceTreeRepository::new(store))
}

pub(crate) fn dependencies(store: Option<Arc<LocalEventStore>>) -> WorkflowRuntimeDependencies {
    WorkflowRuntimeDependencies {
        store,
        config: None,
        secrets: None,
        state_changes: releashd::test_support::integration::subscriptions::test_subscriptions(),
    }
}

use crate::adaptor_gateway_workflow_workflow_host::workflow_host_tests::AcceptingWorktreeResolver;
use crate::adaptor_gateway_workflow_workflow_host::workflow_host_tests::UnusedWorkflowResolver;
use releashd::test_support::integration::workflow::ExecutionTreeArchiveFactRepository;
use releashd::test_support::integration::workflow::WorkflowRuntimeCommandGateway;
use releashd::test_support::integration::workspace::SqliteWorkspaceQueryService;
use releashd::test_support::integration::workspace::SqliteWorkspaceTreeRepository;
pub(crate) struct ArchiveFixture {
    pub(crate) directory: tempfile::TempDir,
    pub(crate) store: Arc<LocalEventStore>,
    pub(crate) repository: Arc<ExecutionTreeArchiveFactRepository>,
    pub(crate) host: Arc<WorkflowRuntimeHost>,
    pub(crate) runtime: releashd::test_support::integration::workflow::WorkflowRuntimeUsecase,
    pub(crate) app: WorkflowRuntimeDependencies,
    pub(crate) sessions: Arc<TestSessions>,
    pub(crate) trees: Arc<SqliteWorkspaceTreeRepository>,
}

impl ArchiveFixture {
    /// worktree の画面に出る実行木の根の数。
    pub(crate) async fn visible_root_count(&self, worktree_path: &str) -> usize {
        use releashd::test_support::integration::workspace::WorkspaceIdentity;
        use releashd::test_support::integration::workspace::WorkspaceTreeRepository;
        self.trees
            .load_trees(&[WorkspaceIdentity::new(worktree_path)])
            .await
            .pop()
            .unwrap()
            .unwrap()
            .visible()
            .roots()
            .len()
    }
}

pub(crate) fn archive_fixture() -> ArchiveFixture {
    archive_fixture_with_resolver(Arc::new(AcceptingWorktreeResolver))
}

pub(crate) fn archive_fixture_with_resolver(
    resolver: Arc<dyn ManagedWorktreeResolver>,
) -> ArchiveFixture {
    let directory = tempfile::tempdir().unwrap();
    let store = LocalEventStore::open(LocalEventStoreConfig::production(
        directory.path().into(),
        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
    ))
    .unwrap();
    let repository = Arc::new(ExecutionTreeArchiveFactRepository::new(
        store.clone(),
        directory.path(),
    ));
    let trees = SqliteWorkspaceTreeRepository::new(store.clone());
    let query = SqliteWorkspaceQueryService::with_repository(trees.clone());
    let app = crate::adaptor_gateway_workflow_workflow_host_test_helpers::dependencies(Some(
        store.clone(),
    ));
    let sessions = Arc::new(TestSessions::default());
    let host = Arc::new(WorkflowRuntimeHost::with_runtime_ports(
        releashd::test_support::integration::platform::shared().clone(),
        Arc::new(UnusedWorkflowResolver),
        resolver,
        query,
        sessions.clone(),
        Arc::new(TestWorktrees::default()),
        releashd::test_support::integration::daemon::serving(),
    ));
    let runtime = releashd::test_support::integration::workflow::WorkflowRuntimeUsecase::new(
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
        trees,
    }
}

pub(crate) async fn archive_workflow(fixture: &ArchiveFixture) -> String {
    let workflow = serde_saphyr::from_str("name: archive\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}").unwrap();
    Box::pin(fixture.host.start_resolved_workflow(
        &fixture.app,
        workflow,
        "/missing/worktree".into(),
        None,
        ExecutionOrigin::Cli,
    ))
    .await
    .unwrap()
}
pub(crate) async fn reconcile_startup(
    host: &WorkflowRuntimeHost,
    app: &WorkflowRuntimeDependencies,
) -> Result<(), WorkflowRuntimeError> {
    match releashd::test_support::integration::platform::wire_workflow_startup(
        app.clone(),
        Arc::new(host.clone()),
    ) {
        Some(startup) => releashd::test_support::integration::platform::recover(
            &releashd::test_support::integration::platform::test_retrying(),
            &startup,
        )
        .await
        .map_err(|error| WorkflowRuntimeError::SessionStore(error.to_string())),
        None => Ok(()),
    }
}

pub(crate) async fn poll_until_pending<F: std::future::Future>(
    mut operation: std::pin::Pin<&mut F>,
    ready: impl Fn() -> bool,
) {
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        std::future::poll_fn(|cx| {
            assert!(operation.as_mut().poll(cx).is_pending());
            if ready() {
                std::task::Poll::Ready(())
            } else {
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        }),
    )
    .await
    .expect("operation must reach the pending checkpoint");
}
