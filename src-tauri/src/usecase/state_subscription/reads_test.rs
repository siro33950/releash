use super::*;
use crate::adaptor::controller::wiring;
use crate::adaptor::gateway::{
    app_config::{AppConfig, ReleashConfig},
    git_host::InMemoryTtlCache,
    repository::{repo_paths::RepoPathsGateway, scanner::DefaultRepositoryScanner, state::*},
};
use crate::domain::git_host::{CacheTtl, GitHostError, GitHostProvider, IssueInfo, PrStatus};
use crate::usecase::agent_session::*;
use crate::usecase::state_subscription::{StateSubscriptionEvent, StateSubscriptionUsecase};
use futures_util::StreamExt;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Sessions {
    item: Mutex<Option<AgentSessionItemDto>>,
    calls: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl AgentSessionQueryService for Sessions {
    async fn get(&self, id: &str) -> Result<Option<AgentSessionItemDto>, AgentSessionQueryError> {
        self.calls.lock().push(id.into());
        Ok(self
            .item
            .lock()
            .as_ref()
            .filter(|item| item.id == id)
            .cloned())
    }
}
#[async_trait::async_trait]
impl AgentSessionGarbageCollectionPort for Sessions {
    async fn reconcile_garbage_collection(
        &self,
        _: &str,
        _: &str,
    ) -> Result<AgentSessionGarbageCollectionOutcome, AgentSessionLifecycleUsecaseError> {
        Ok(AgentSessionGarbageCollectionOutcome::Retained)
    }
}
#[async_trait::async_trait]
impl AgentSessionHistoryQueryService for Sessions {
    async fn list(
        &self,
        request: AgentSessionHistoryRequest,
    ) -> Result<AgentSessionHistoryPageDto, AgentSessionHistoryQueryError> {
        self.calls.lock().push(format!(
            "{}:{}",
            request.worktree_path, request.visible_count
        ));
        Ok(AgentSessionHistoryPageDto {
            items: vec![AgentSessionHistoryCandidateDto {
                provider: AgentSessionProviderDto::Codex,
                provider_session_id: "history-session".into(),
                label: "history".into(),
                updated_at_ms: 10,
            }],
            has_more: true,
        })
    }
}

#[derive(Default)]
struct Issues {
    calls: AtomicUsize,
    values: Mutex<Vec<IssueInfo>>,
}
impl GitHostProvider for Issues {
    fn fetch_pr_status(&self, _: &str) -> Result<PrStatus, GitHostError> {
        Ok(PrStatus::default())
    }
    fn list_issues(&self, _: &str) -> Result<Vec<IssueInfo>, GitHostError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.values.lock().clone())
    }
}
fn issue(number: u64) -> IssueInfo {
    IssueInfo {
        number,
        title: "issue".into(),
        state: "OPEN".into(),
        url: "https://example.test/issue".into(),
        author: crate::domain::git_host::value_objects::issue::PrAuthor {
            login: "author".into(),
        },
        created_at: String::new(),
        updated_at: String::new(),
        labels: vec![],
        assignees: vec![],
        body: String::new(),
        milestone: None,
    }
}

pub(crate) struct Fixture {
    pub(crate) reads: WorkspaceStateReads,
    pub(crate) subscriptions: StateSubscriptionUsecase,
    pub(crate) path: String,
    issues: Arc<Issues>,
    sessions: Arc<Sessions>,
    _directory: tempfile::TempDir,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().canonicalize().unwrap();
        let path = root.join("repo").to_str().unwrap().to_string();
        let git = git2::Repository::init(&path).unwrap();
        crate::test_support::git::create_initial_commit(&git);
        let commit = git.head().unwrap().peel_to_commit().unwrap();
        git.branch("feature", &commit, false).unwrap();
        let subscriptions = StateSubscriptionUsecase::new(
            vec![path.clone()],
            Arc::new(crate::adaptor::gateway::subscription_timer::TokioSubscriptionTimer),
        );
        let publisher = subscriptions.publisher();
        let repository =
            Arc::new(wiring::build_repository_usecase().with_state_publisher(publisher.clone()));
        let config = Arc::new(AppConfig::new(
            ReleashConfig::default(),
            root.join("config.toml"),
        ));
        let repositories = Arc::new(RepoPathsUsecase::new(
            Arc::new(RepoPathsGateway::new(
                Arc::new(parking_lot::RwLock::new(vec![path.clone()])),
                config,
            )),
            Arc::new(
                crate::adaptor::gateway::repository::notify::RepoPathsNotifyGateway::new(
                    publisher.clone(),
                ),
            ),
        ));
        let repository_state = Arc::new(RepositoryStateService::new(
            Arc::new(RepositoryStateRepositoryGateway::new(repository.clone())),
            Arc::new(DefaultRepositoryScanner::new(
                repository.clone(),
                Arc::new(wiring::build_code_usecase()),
            )),
            Arc::new(ClientRepositoryStateNotifier::new(
                Arc::new(crate::infrastructure::push::PushSink::new()),
                publisher.clone(),
            )),
            Arc::new(NotifyRepositoryStateWatcher::new(repository.clone())),
            Arc::new(TokioRepositoryStateWorkerRuntime),
            Arc::new(FsWorktreePathNormalizer),
        ));
        let workflow = Arc::new(wiring::build_workflow_usecase(root.join("data")));
        let issues = Arc::new(Issues::default());
        *issues.values.lock() = vec![issue(1)];
        let git_host = Arc::new(
            GitHostUsecase::new(
                issues.clone(),
                Arc::new(InMemoryTtlCache::<PrStatus>::new(
                    CacheTtl::EXTERNAL_INFORMATION,
                )),
                Arc::new(InMemoryTtlCache::<Vec<IssueInfo>>::new(
                    CacheTtl::EXTERNAL_INFORMATION,
                )),
            )
            .with_state_publisher(publisher.clone()),
        );
        let workspaces = Arc::new(WorkspaceListUsecase::new(Arc::new(
            crate::usecase::workspace_tree::WorkspaceListServices {
                repositories: repositories.clone(),
                repository_state: repository_state.clone(),
                workflow: workflow.clone(),
                git_host: git_host.clone(),
            },
        )));
        let sessions = Arc::new(Sessions::default());
        let providers = Arc::new(
            ProviderAvailabilityUsecase::initialize(
                Arc::new(
                    provider_availability_tests::FakeProviderExecutableConfigRepository::default(),
                ),
                Arc::new(
                    provider_availability_tests::FakeProviderExecutableProbeGateway::default(),
                ),
            )
            .unwrap()
            .with_state_publisher(publisher),
        );
        let reads = WorkspaceStateReads {
            repositories,
            repository,
            repository_state,
            workflow,
            workspaces,
            git_host,
            sessions: Arc::new(AgentSessionReadUsecase::new(
                sessions.clone(),
                sessions.clone(),
            )),
            history: Arc::new(AgentSessionHistoryReadUsecase::new(sessions.clone())),
            providers,
            workspace_state: Arc::new(
                crate::adaptor::gateway::workspace_state::WorkspaceStateStore::new(
                    root.join("workspace"),
                ),
            ),
        };
        Self {
            subscriptions: subscriptions.with_reads(Arc::new(reads.clone()), None, vec![]),
            reads,
            path,
            issues,
            sessions,
            _directory: directory,
        }
    }
}

#[tokio::test]
async fn test_状態読取_全対象を対応するサービスへ引数付きで振り分ける() {
    // Given
    use SubscriptionTarget as T;
    let fixture = Fixture::new();
    let r = &fixture.reads;
    let p = &fixture.path;
    let branch = r.repository.get_current_branch(p).unwrap();
    r.repository
        .set_branch_base_override(p, "feature", Some(&branch))
        .unwrap();
    let workspace: crate::usecase::workspace_state::dto::WorkspaceStateDto = serde_json::from_value(serde_json::json!({
        "version": 1, "tabs": {"editors": [], "activeEditorPath": null},
        "layout": {"centerTab": "agent", "activeView": "git", "leftNavCollapsed": true, "rightCollapsed": false, "rightBottomCollapsed": false}
    })).unwrap();
    crate::usecase::workspace_state::usecase::save_workspace_state(
        r.workspace_state.as_ref(),
        None,
        "repo",
        workspace.clone().into(),
    )
    .unwrap();
    r.workspaces.refresh().await;
    let selection = r
        .workflow
        .get_workspace_tree_selection_reconciliation(p, "missing")
        .await
        .unwrap();
    // When / Then
    let cases = vec![
        (
            T::RepositoryPaths,
            StateValue::RepositoryPaths(vec![p.clone()]),
        ),
        (
            T::Workspaces,
            StateValue::Workspaces(r.workspaces.snapshot()),
        ),
        (
            T::Selection(p.clone(), "missing".into()),
            StateValue::Selection(selection),
        ),
        (
            T::NodeDetail(p.clone(), "missing".into()),
            StateValue::NodeDetail(None),
        ),
        (
            T::SessionNode(p.clone(), "missing".into()),
            StateValue::SessionNode(None),
        ),
        (
            T::AgentSession("missing-session".into()),
            StateValue::AgentSession(None),
        ),
        (
            T::SessionHistory(p.clone(), 120),
            StateValue::SessionHistory(AgentSessionHistoryPageDto {
                items: vec![AgentSessionHistoryCandidateDto {
                    provider: AgentSessionProviderDto::Codex,
                    provider_session_id: "history-session".into(),
                    label: "history".into(),
                    updated_at_ms: 10,
                }],
                has_more: true,
            }),
        ),
        (
            T::Providers,
            StateValue::Providers(vec![
                AgentSessionProviderDto::Claude,
                AgentSessionProviderDto::Codex,
            ]),
        ),
        (
            T::Branches(p.clone(), Some("feature".into())),
            StateValue::Branches(vec![crate::usecase::repository_dto::BranchDto {
                name: branch.clone(),
                is_remote: false,
            }]),
        ),
        (
            T::BranchBase(p.clone(), "feature".into()),
            StateValue::BranchBase(Some(branch.clone())),
        ),
        (
            T::BranchStatus(p.clone()),
            StateValue::BranchStatus(
                r.repository_state
                    .list_branches_with_status_snapshot(p)
                    .unwrap(),
            ),
        ),
        (
            T::CurrentBranch(p.clone()),
            StateValue::CurrentBranch(branch),
        ),
        (
            T::Issues(p.clone()),
            StateValue::Issues(vec![issue(1).into()]),
        ),
        (
            T::Worktrees(p.clone()),
            StateValue::Worktrees(r.repository.list_worktrees(p).unwrap()),
        ),
        (
            T::RepositoryRoot(format!("{p}/.git")),
            StateValue::RepositoryRoot(p.clone()),
        ),
        (
            T::StartupRepository,
            StateValue::StartupRepository(
                r.repository
                    .get_main_repo_path(&r.repository.get_cwd().unwrap())
                    .unwrap(),
            ),
        ),
        (
            T::WorkspaceState("repo".into(), p.clone()),
            StateValue::WorkspaceState(Some(workspace)),
        ),
    ];
    for (target, expected) in cases {
        assert_eq!(r.read(&target).await.unwrap(), expected, "{target}");
    }
    assert_eq!(
        *fixture.sessions.calls.lock(),
        ["missing-session".to_string(), format!("{p}:120")]
    );
    assert_eq!(fixture.issues.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        r.read(&T::CurrentBranch("/missing/repository".into()))
            .await
            .unwrap_err()
            .kind,
        r.repository
            .get_current_branch("/missing/repository")
            .unwrap_err()
            .failure_kind()
    );
}

#[tokio::test]
async fn test_issue手動更新_有効なcacheを無視し30秒前に同じ購読へ変更を配信する() {
    // Given
    let fixture = Fixture::new();
    let mut stream = Box::pin(fixture.subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::Issues(fixture.path.clone()).to_string();
    fixture
        .subscriptions
        .start_read("client", &target, None)
        .await
        .unwrap();
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, crate::domain::state_subscription::Event::Snapshot(_, value))) if *value == StateValue::Issues(vec![issue(1).into()]))
    );
    stream.next().await;
    *fixture.issues.values.lock() = vec![issue(2)];
    // When
    let before = std::time::Instant::now();
    let mut dispatch = crate::adaptor::controller::client::ClientCommandDispatch::new(Arc::new(
        crate::usecase::application_startup::ApplicationStartupAuthority::ready(),
    ));
    let git_host = fixture.reads.git_host.clone();
    dispatch.register_domain(
        &["fetch_issues"],
        Box::new(move |command| {
            let git_host = git_host.clone();
            Box::pin(async move {
                use crate::adaptor::controller::api::protocol::client as wire;
                let wire::command_request::Command::FetchIssues(args) = command else {
                    panic!("unexpected command")
                };
                crate::adaptor::controller::client::git_host::issue::fetch_issues_shared(
                    &git_host,
                    args.repo_path.unwrap(),
                )
                .await
                .map_err(wire::CommandFailure::from)?;
                Ok(wire::command_result::Command::FetchIssues(wire::Unit {}))
            })
        }),
    );
    use crate::adaptor::controller::api::protocol::client as wire;
    assert!(matches!(
        dispatch
            .dispatch(wire::command_request::Command::FetchIssues(
                wire::FetchIssuesRequest {
                    repo_path: Some(fixture.path.clone())
                }
            ))
            .await
            .unwrap(),
        wire::command_result::Command::FetchIssues(_)
    ));
    // Then
    let value = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Some(StateSubscriptionEvent::Item(
                id,
                crate::domain::state_subscription::Event::Change(_, _, value),
            )) = stream.next().await
            {
                assert_eq!(id, target);
                break value;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(*value, StateValue::Issues(vec![issue(2).into()]));
    assert!(before.elapsed() < CacheTtl::EXTERNAL_INFORMATION.duration());
    assert_eq!(fixture.issues.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_終了済み実行木のarchiveとrestore_取り直しなしでツリーが配信される() {
    use crate::adaptor::gateway::workflow::workflow_host::test_helpers::archive_fixture;
    use crate::adaptor::gateway::workflow::{
        EmptySecretSourceGateway, NoopWorkflowExternalEditorGateway,
        PassthroughManagedWorktreeGateway,
    };
    use crate::domain::state_subscription::Event;
    // Given
    let fixture = Fixture::new();
    let mut archive = archive_fixture();
    let workflow = serde_saphyr::from_str("name: archive\ndescription: test\nnodes:\n  main: {session: {provider: codex, facets: {instruction: policy-confirmation}}}").unwrap();
    let id = archive
        .host
        .start_resolved_workflow(
            &archive.app,
            workflow,
            fixture.path.clone(),
            None,
            crate::domain::workflow::ExecutionOrigin::Cli,
        )
        .await
        .unwrap();
    archive
        .runtime
        .abort_execution(crate::usecase::workflow::command::AbortExecutionCommand {
            execution_id: id.clone(),
            expected_node_name: None,
        })
        .await
        .unwrap();
    let mut reads = fixture.reads.clone();
    reads.workflow = Arc::new(
        wiring::build_workflow_services_with_gateways(
            archive.directory.path(),
            Arc::new(PassthroughManagedWorktreeGateway),
            Arc::new(NoopWorkflowExternalEditorGateway),
            Arc::new(EmptySecretSourceGateway),
            archive.store.clone(),
            None,
        )
        .0,
    );
    let subscriptions = fixture
        .subscriptions
        .with_reads(Arc::new(reads), None, vec![]);
    archive.runtime = archive
        .runtime
        .with_state_publisher(subscriptions.publisher());
    let mut stream = Box::pin(subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::Selection(fixture.path.clone(), "selected".into()).to_string();
    subscriptions
        .start_read("client", &target, None)
        .await
        .unwrap();
    let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) = stream.next().await
    else {
        panic!("initial snapshot")
    };
    let StateValue::Selection(initial) = value.as_ref() else {
        panic!("selection")
    };
    assert!(!initial.snapshot.nodes.is_empty());
    stream.next().await;
    // When / Then
    for archived in [true, false] {
        if archived {
            archive
                .runtime
                .archive_execution_tree(&id, "manual")
                .await
                .unwrap();
        } else {
            archive.runtime.restore_execution_tree(&id).await.unwrap();
        }
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap();
        let Some(StateSubscriptionEvent::Item(received, Event::Change(_, _, value))) = event else {
            panic!("changed tree")
        };
        assert_eq!(received, target);
        let StateValue::Selection(selection) = value.as_ref() else {
            panic!("selection")
        };
        assert_eq!(selection.snapshot.nodes.is_empty(), archived);
    }
}

#[tokio::test]
async fn test_agent_session購読_状態変更通知から再読取して同じ購読へ配信する() {
    use crate::adaptor::gateway::push::ClientAgentSessionChangeNotifier;
    use crate::domain::state_subscription::Event;
    // Given
    let fixture = Fixture::new();
    let item = AgentSessionItemDto {
        id: "session".into(),
        workspace_identity: fixture.path.clone(),
        worktree_path: fixture.path.clone(),
        workspace_worktree_path: fixture.path.clone(),
        provider: AgentSessionProviderDto::Codex,
        tree_location: AgentSessionTreeLocationDto {
            tree_id: "tree".into(),
            node_execution_id: "node".into(),
        },
        lifecycle: AgentSessionLifecycleDto::Open,
        provider_session_id: None,
        transcript_ref: None,
        operations: AgentSessionOperationsDto {
            can_archive: true,
            can_restore: false,
            can_delete: false,
        },
        last_exit_abnormal: false,
    };
    *fixture.sessions.item.lock() = Some(item.clone());
    let mut stream = Box::pin(fixture.subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::AgentSession(item.id.clone()).to_string();
    fixture
        .subscriptions
        .start_read("client", &target, None)
        .await
        .unwrap();
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(id, Event::Snapshot(_, value))) if id == target && *value == StateValue::AgentSession(Some(item.clone())))
    );
    stream.next().await;
    let notifier = ClientAgentSessionChangeNotifier::new(fixture.subscriptions.publisher());
    // When / Then
    for lifecycle in [
        Some(AgentSessionLifecycleDto::Paused),
        Some(AgentSessionLifecycleDto::Archived),
        Some(AgentSessionLifecycleDto::Open),
        None,
    ] {
        let next = lifecycle.map(|lifecycle| AgentSessionItemDto {
            lifecycle,
            operations: AgentSessionOperationsDto {
                can_archive: lifecycle != AgentSessionLifecycleDto::Archived,
                can_restore: lifecycle == AgentSessionLifecycleDto::Archived,
                can_delete: lifecycle == AgentSessionLifecycleDto::Archived,
            },
            ..item.clone()
        });
        let before = fixture.sessions.calls.lock().len();
        *fixture.sessions.item.lock() = next.clone();
        notifier.agent_session_changed(&fixture.path);
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap();
        assert!(
            matches!(event, Some(StateSubscriptionEvent::Item(id, Event::Change(_, crate::domain::state_subscription::Delivery::Full, value))) if id == target && *value == StateValue::AgentSession(next))
        );
        assert_eq!(fixture.sessions.calls.lock().len(), before + 1);
        assert!(fixture
            .sessions
            .calls
            .lock()
            .iter()
            .all(|id| id == "session"));
    }
}
