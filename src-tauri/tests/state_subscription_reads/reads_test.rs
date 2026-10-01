use crate::adaptor::controller::wiring;
use crate::adaptor::gateway::{
    app_config::{AppConfig, ReleashConfig},
    git_host::InMemoryTtlCache,
    repository::{repo_paths::RepoPathsGateway, scanner::DefaultRepositoryScanner, state::*},
};
use crate::domain::git_host::{CacheTtl, GitHostError, GitHostProvider, IssueInfo, PrStatus};
use crate::domain::workflow::FacetKind;
use crate::test_support::state_subscription::start_read;
use crate::test_support::state_subscription::StateSubscriptionEvent;
use crate::usecase::agent_session::*;
use crate::usecase::git_host::GitHostUsecase;
use crate::usecase::provider_dto::AgentSessionProviderDto;
use crate::usecase::repo_paths_usecase::RepoPathsUsecase;
use crate::usecase::repository_state::RepositoryStateService;
use crate::usecase::state_subscription::{
    StateChangeSource, StateReadFailure, StateSubscriptionRead, StateSubscriptionUsecase,
    StateValue, SubscriptionTarget, WorkspaceStateReads,
};
use crate::usecase::workflow::ports::WorkflowDiagnosticsTarget;
use crate::usecase::workspace_tree::WorkspaceListUsecase;
use futures_util::StreamExt;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

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
    own_runtime: AtomicBool,
    failure: Mutex<bool>,
}
impl GitHostProvider for Issues {
    fn fetch_pr_status(&self, _: &str) -> Result<PrStatus, GitHostError> {
        Ok(PrStatus::default())
    }
    fn list_issues(&self, _: &str) -> Result<Vec<IssueInfo>, GitHostError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.own_runtime.load(Ordering::SeqCst) {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(async {});
        }
        if *self.failure.lock() {
            return Err(GitHostError::External("issues offline".into()));
        }
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

#[derive(Default)]
struct MemoryHookHealth(
    Mutex<
        std::collections::HashMap<
            crate::domain::provider_lifecycle::ProviderKind,
            crate::domain::provider_lifecycle::VersionedProviderHookHealth,
        >,
    >,
);
#[async_trait::async_trait]
impl crate::domain::provider_lifecycle::ProviderHookHealthRepository for MemoryHookHealth {
    async fn load(
        &self,
        provider: crate::domain::provider_lifecycle::ProviderKind,
    ) -> Result<
        crate::domain::provider_lifecycle::VersionedProviderHookHealth,
        crate::domain::provider_lifecycle::ProviderHookHealthRepositoryError,
    > {
        Ok(self.0.lock().get(&provider).cloned().unwrap_or_else(|| {
            crate::domain::provider_lifecycle::VersionedProviderHookHealth::restored(
                crate::domain::provider_lifecycle::ProviderHookHealth::new(provider),
                0,
            )
        }))
    }
    async fn save(
        &self,
        mut health: crate::domain::provider_lifecycle::VersionedProviderHookHealth,
        _: &str,
    ) -> Result<
        crate::domain::provider_lifecycle::VersionedProviderHookHealth,
        crate::domain::provider_lifecycle::ProviderHookHealthRepositoryError,
    > {
        let revision = health.revision()
            + u64::try_from(health.health_mut().take_uncommitted_events().len()).unwrap();
        let saved = crate::domain::provider_lifecycle::VersionedProviderHookHealth::restored(
            health.into_health(),
            revision,
        );
        self.0
            .lock()
            .insert(saved.health().provider(), saved.clone());
        Ok(saved)
    }
}
struct NoHookHealthFailures;
#[async_trait::async_trait]
impl crate::usecase::provider_lifecycle::ProviderHookHealthFailureQuery for NoHookHealthFailures {
    async fn list(
        &self,
        _: usize,
    ) -> Result<
        Vec<
            Result<
                crate::usecase::provider_lifecycle::ProviderHookHealthFailureObservation,
                crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError,
            >,
        >,
        crate::usecase::provider_lifecycle::ProviderHookHealthFailureQueryError,
    > {
        Ok(vec![])
    }
}

pub(crate) struct Fixture {
    pub(crate) reads: WorkspaceStateReads,
    pub(crate) repository_state: Arc<RepositoryStateService>,
    pub(crate) subscriptions: StateSubscriptionUsecase,
    pub(crate) config: Arc<AppConfig>,
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
        let publisher = subscriptions.clone();
        let repository =
            Arc::new(wiring::build_repository_usecase().with_state_publisher(publisher.clone()));
        let config = Arc::new(AppConfig::new(
            ReleashConfig::default(),
            root.join("config.toml"),
        ));
        let repositories = Arc::new(RepoPathsUsecase::new(
            Arc::new(RepoPathsGateway::new(
                Arc::new(parking_lot::RwLock::new(vec![path.clone()])),
                config.clone(),
            )),
            publisher.clone(),
        ));
        let repository_state = Arc::new(RepositoryStateService::new(
            Arc::new(RepositoryStateRepositoryGateway::new(repository.clone())),
            Arc::new(DefaultRepositoryScanner::new(
                repository.clone(),
                Arc::new(wiring::build_code_usecase()),
            )),
            publisher.clone(),
            Arc::new(NotifyRepositoryStateWatcher::new(repository.clone())),
            Arc::new(
                crate::adaptor::controller::repository_scan::RepositoryScanWorkerRuntime::new(
                    crate::usecase::retry::test_retrying(),
                ),
            ),
            Arc::new(FsWorktreePathNormalizer),
        ));
        let workflows_dir = root.join("workflows");
        std::fs::create_dir_all(workflows_dir.join("instructions")).unwrap();
        std::fs::write(
            workflows_dir.join("fixture.yml"),
            "name: fixture\ndescription: fixture workflow\nnodes:\n  main:\n    session:\n      provider: claude\n      facets:\n        instruction: fixture-instruction\n",
        )
        .unwrap();
        std::fs::write(workflows_dir.join("broken.yml"), "name: [").unwrap();
        std::fs::write(
            workflows_dir
                .join("instructions")
                .join("fixture-instruction.md"),
            "# fixture instruction\n",
        )
        .unwrap();
        let workflow = Arc::new(wiring::build_workflow_usecase(
            root.join("data"),
            Some(workflows_dir.clone()),
        ));
        let issues = Arc::new(Issues::default());
        *issues.values.lock() = vec![issue(1)];
        let git_host = Arc::new(
            GitHostUsecase::new(
                issues.clone(),
                Arc::new(crate::adaptor::gateway::git_host::LatestPrStatuses::default()),
                Arc::new(InMemoryTtlCache::<Vec<IssueInfo>>::new(
                    CacheTtl::EXTERNAL_INFORMATION,
                )),
            )
            .with_state_publisher(publisher.clone()),
        );
        let workspaces = Arc::new(WorkspaceListUsecase::new(
            repositories.clone(),
            repository.clone(),
            repository_state.clone(),
            workflow.clone(),
            git_host.clone(),
        ));
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
            .with_state_publisher(publisher.clone()),
        );
        let reads = WorkspaceStateReads {
            repositories,
            repository,
            workflow,
            workspaces,
            git_host,
            sessions: Arc::new(AgentSessionReadUsecase::new(
                std::sync::Arc::new(crate::adaptor::gateway::identity::RandomIdentityIssuer),
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
            review: Arc::new(crate::usecase::review_usecase::ReviewUsecase::new(
                repository_state.clone(),
                Arc::new(wiring::build_code_usecase()),
            )),
            comments: Arc::new(
                wiring::build_review_comment_usecase().with_subscriptions(subscriptions.clone()),
            ),
            data_dir: root.to_path_buf(),
            review_comments_dir: crate::adaptor::gateway::comment::state_dir(&root),
            workflows_dir: workflows_dir.clone(),
            app_config: Arc::new(
                crate::usecase::app_config::AppConfigUsecase::new(config.clone(), config.clone())
                    .with_state_publisher(publisher.clone()),
            ),
            notion: Arc::new(
                crate::usecase::notion::usecase::NotionUsecase::new(
                    config.clone(),
                    config.clone(),
                    Arc::new(crate::adaptor::gateway::notion::NotionApiGatewayImpl::new()),
                )
                .with_state_publisher(publisher.clone()),
            ),
            editor_settings: Arc::new(
                crate::adaptor::gateway::external_editor::EditorSettingsConfigGateway::new(
                    config.clone(),
                ),
            ),
            editor_scanner: Arc::new(
                crate::adaptor::gateway::external_editor::MacInstalledEditorGateway,
            ),
            performance_switches: crate::usecase::telemetry::PerformanceSwitches {
                real_app_mode: false,
                terminal: Default::default(),
            },
            hook_health: Arc::new(
                crate::usecase::provider_lifecycle::ProviderHookHealthReadUsecase::new(
                    Arc::new(
                        crate::usecase::provider_lifecycle::ProviderHookHealthUsecase::new(
                            Arc::new(MemoryHookHealth::default()),
                        )
                        .with_state_publisher(publisher.clone()),
                    ),
                    Arc::new(NoHookHealthFailures),
                ),
            ),
            startup: Arc::new(
                crate::usecase::application_startup::ApplicationStartupAuthority::ready(),
            ),
        };
        Self {
            subscriptions: subscriptions.with_reads(
                Arc::new(reads.clone()),
                None,
                vec![],
                String::new(),
            ),
            reads,
            repository_state,
            config,
            path,
            issues,
            sessions,
            _directory: directory,
        }
    }

    pub(crate) fn list_issues_in_own_runtime(&self) {
        self.issues.own_runtime.store(true, Ordering::SeqCst);
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
    let (tree, selected) = r
        .workflow
        .workspace_tree_selection(p, "missing")
        .await
        .unwrap();
    let cases = vec![
        (
            T::RepositoryPaths,
            StateValue::RepositoryPaths(vec![p.clone()]),
        ),
        (
            T::Workspaces,
            StateValue::Workspaces(r.workspaces.read().await.unwrap()),
        ),
        (
            T::Selection(p.clone(), "missing".into()),
            StateValue::Selection(tree, selected),
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
            StateValue::BranchStatus(r.repository.list_branches_with_worktree(p).unwrap()),
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
            StateValue::RepositoryRoot(Some(p.clone())),
        ),
        (
            T::StartupRepository,
            StateValue::StartupRepository(
                r.repository
                    .find_main_repo_path(&r.repository.get_cwd().unwrap())
                    .unwrap(),
            ),
        ),
        (
            T::WorkspaceState("repo".into(), p.clone()),
            StateValue::WorkspaceState(Some(workspace)),
        ),
        (
            T::Workflows,
            StateValue::Workflows(
                r.workflow
                    .read_usecase()
                    .list_workflow_summaries()
                    .await
                    .unwrap(),
            ),
        ),
        (
            T::Workflow("fixture".into()),
            StateValue::Workflow(r.workflow.get_workflow_dto("fixture").unwrap()),
        ),
        (T::Workflow("missing".into()), StateValue::Workflow(None)),
        (
            T::WorkflowSource("fixture".into()),
            StateValue::WorkflowSource(r.workflow.get_workflow_source("fixture").unwrap()),
        ),
        (
            T::Facets(FacetKind::Instruction),
            StateValue::Facets(
                r.workflow
                    .list_facet_summaries(FacetKind::Instruction)
                    .unwrap()
                    .into_iter()
                    .map(crate::usecase::workflow::dto::facet_summary_to_dto)
                    .collect(),
            ),
        ),
        (
            T::Facet(FacetKind::Instruction, "fixture-instruction".into()),
            StateValue::Facet("# fixture instruction\n".into()),
        ),
        (
            T::Diagnostics,
            StateValue::Diagnostics(
                r.workflow
                    .diagnose_all(WorkflowDiagnosticsTarget::AppliedConfigDirectory)
                    .unwrap(),
            ),
        ),
    ];
    for (target, expected) in cases {
        let value = r
            .read(&target)
            .await
            .unwrap_or_else(|error| panic!("{target}: {error}"));
        assert_eq!(value, expected, "{target}");
    }
    // When
    let StateValue::Workflows(workflows) = r.read(&T::Workflows).await.unwrap() else {
        unreachable!()
    };
    // Then
    assert!(workflows.iter().any(|workflow| workflow.name == "fixture"));
    let StateValue::Workflow(Some(fixture_workflow)) =
        r.read(&T::Workflow("fixture".into())).await.unwrap()
    else {
        panic!("fixture workflow is loadable")
    };
    assert_eq!(fixture_workflow.name, "fixture");
    let StateValue::Diagnostics(report) = r.read(&T::Diagnostics).await.unwrap() else {
        unreachable!()
    };
    assert!(matches!(
        r.read(&T::Workflow("broken".into())).await,
        Err(crate::usecase::state_subscription::StateReadError {
            source: StateReadFailure::Workflow(_),
            ..
        })
    ));
    assert!(report.workflow_summaries.contains_key("broken"));
    assert!(matches!(
        r.read(&T::Facet(FacetKind::Instruction, "missing-facet".into()))
            .await
            .unwrap_err()
            .source,
        StateReadFailure::Workflow(_)
    ));
    assert_eq!(
        *fixture.sessions.calls.lock(),
        ["missing-session".to_string(), format!("{p}:120")]
    );
    assert_eq!(fixture.issues.calls.load(Ordering::SeqCst), 1);
    let error = r
        .read(&T::CurrentBranch("/missing/repository".into()))
        .await
        .unwrap_err();
    let expected = r
        .repository
        .get_current_branch("/missing/repository")
        .unwrap_err();
    assert!(
        matches!(error.source, StateReadFailure::Repository(actual) if format!("{actual:?}") == format!("{expected:?}"))
    );
}

#[tokio::test]
async fn test_issue手動更新_有効なcacheを無視し30秒前に同じ購読へ変更を配信する() {
    // Given
    let fixture = Fixture::new();
    let mut stream = Box::pin(fixture.subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::Issues(fixture.path.clone()).to_string();
    start_read(&fixture.subscriptions, "client", &target, None)
        .await
        .unwrap();
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, crate::test_support::state_subscription::Event::Snapshot(_, value))) if crate::test_support::state_subscription::same(&value, &StateValue::Issues(vec![issue(1).into()])))
    );
    stream.next().await;
    *fixture.issues.values.lock() = vec![issue(2)];
    // When
    let before = std::time::Instant::now();
    fixture.reads.git_host.fetch_issues(&fixture.path).unwrap();
    // Then
    let value = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Some(StateSubscriptionEvent::Item(
                id,
                crate::test_support::state_subscription::Event::Change(_, _, value),
            )) = stream.next().await
            {
                assert_eq!(id, target);
                break value;
            }
        }
    })
    .await
    .unwrap();
    assert!(crate::test_support::state_subscription::same(
        &value,
        &StateValue::Issues(vec![issue(2).into()])
    ));
    assert!(before.elapsed() < CacheTtl::EXTERNAL_INFORMATION.duration());
    assert_eq!(fixture.issues.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_終了済み実行木のarchiveとrestore_取り直しなしでツリーが配信される() {
    // Given
    use crate::adaptor::gateway::workflow::workflow_host::test_helpers::archive_fixture;
    use crate::adaptor::gateway::workflow::{
        EmptySecretSourceGateway, NoopWorkflowExternalEditorGateway,
        PassthroughManagedWorktreeGateway,
    };
    use crate::test_support::state_subscription::Event;
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
            Arc::new(crate::adaptor::gateway::failure_records::FailureRecordStore::default()),
            archive.directory.path(),
            Arc::new(PassthroughManagedWorktreeGateway),
            Arc::new(NoopWorkflowExternalEditorGateway),
            Arc::new(EmptySecretSourceGateway),
            archive.store.clone(),
            None,
            None,
        )
        .0,
    );
    let subscriptions =
        fixture
            .subscriptions
            .with_reads(Arc::new(reads), None, vec![], String::new());
    archive.runtime = archive.runtime.with_state_publisher(subscriptions.clone());
    let mut stream = Box::pin(subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::Selection(fixture.path.clone(), "selected".into()).to_string();
    start_read(&subscriptions, "client", &target, None)
        // When
        .await
        .unwrap();
    // Then
    let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) = stream.next().await
    else {
        panic!("initial snapshot")
    };
    let Some(crate::adaptor::presenter::client::state_payload::Value::Selection(initial)) =
        &match value.as_ref() {
            crate::adaptor::presenter::state_subscription::PublishedState::Value(value) => value,
            _ => panic!("selection"),
        }
        .value
    else {
        panic!("selection")
    };
    assert!(!initial
        .snapshot
        .as_ref()
        .unwrap()
        .nodes
        .as_ref()
        .unwrap()
        .items
        .is_empty());
    stream.next().await;
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
        let Some(crate::adaptor::presenter::client::state_payload::Value::Selection(selection)) =
            &match value.as_ref() {
                crate::adaptor::presenter::state_subscription::PublishedState::Value(value) => {
                    value
                }
                _ => panic!("selection"),
            }
            .value
        else {
            panic!("selection")
        };
        assert_eq!(
            selection
                .snapshot
                .as_ref()
                .unwrap()
                .nodes
                .as_ref()
                .unwrap()
                .items
                .is_empty(),
            archived
        );
    }
}

#[tokio::test]
async fn test_agent_session購読_状態変更通知から再読取して同じ購読へ配信する() {
    use crate::test_support::state_subscription::Event;
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
    start_read(&fixture.subscriptions, "client", &target, None)
        .await
        .unwrap();
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(id, Event::Snapshot(_, value))) if id == target && crate::test_support::state_subscription::same(&value, &StateValue::AgentSession(Some(item.clone()))))
    );
    stream.next().await;
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
        fixture
            .subscriptions
            .notify(StateChangeSource::Worktree(fixture.path.clone()));
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap();
        assert!(
            matches!(event, Some(StateSubscriptionEvent::Item(id, Event::Change(_, crate::test_support::state_subscription::Delivery::Full, value))) if id == target && crate::test_support::state_subscription::same(&value, &StateValue::AgentSession(next)))
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

#[tokio::test]
async fn test_状態読取_review対象をworktreeとcomment置き場から読み取る() {
    use crate::domain::code::{ReviewBase, ReviewSection};
    use crate::usecase::code_dto::ReviewFileViewDto;
    use SubscriptionTarget as T;
    // Given
    let fixture = Fixture::new();
    let r = &fixture.reads;
    let p = &fixture.path;
    let git = git2::Repository::open(p).unwrap();
    crate::test_support::git::add_and_commit(&git, "review.txt", "before\n", "tracked");
    std::fs::write(std::path::Path::new(p).join("review.txt"), "after\n").unwrap();
    // When
    let snapshot = r
        .read(&T::ReviewSnapshot(p.clone(), ReviewBase::Head))
        .await
        .unwrap();
    let view = r
        .read(&T::ReviewFileView(
            p.clone(),
            "review.txt".into(),
            ReviewSection::Changes,
            ReviewBase::Head,
        ))
        .await
        .unwrap();
    let threads = r
        .read(&T::ReviewThreads("repository".into()))
        .await
        .unwrap();
    // Then
    assert!(
        matches!(&snapshot, StateValue::ReviewSnapshot(value) if value.changed_files.iter().any(|file| file.path == "review.txt")),
        "{snapshot:?}"
    );
    assert!(
        matches!(&view, StateValue::ReviewFileView(ReviewFileViewDto::TextDiff(text)) if text.original == "before\n" && text.modified == "after\n"),
        "{view:?}"
    );
    assert_eq!(threads, StateValue::ReviewThreads(vec![]));
    assert_eq!(
        r.review_comments_dir(),
        crate::adaptor::gateway::comment::state_dir(&r.data_dir)
            .to_string_lossy()
            .into_owned()
    );
    let error = r
        .read(&T::ReviewFileView(
            p.clone(),
            "missing.txt".into(),
            ReviewSection::Changes,
            ReviewBase::Head,
        ))
        .await
        .unwrap_err();
    assert!(matches!(error.source, StateReadFailure::Code(_)));
}

#[tokio::test]
async fn test_issue手動更新失敗_購読へ失敗を届け回復時に新しい一覧を届ける() {
    // Given
    use crate::test_support::state_subscription::Event;
    let fixture = Fixture::new();
    let mut stream = Box::pin(fixture.subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::Issues(fixture.path.clone()).to_string();
    start_read(&fixture.subscriptions, "client", &target, None)
        .await
        .unwrap();
    stream.next().await.unwrap();
    // When
    stream.next().await.unwrap();
    *fixture.issues.failure.lock() = true;
    // Then
    assert!(fixture.reads.git_host.fetch_issues(&fixture.path).is_err());
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event, StateSubscriptionEvent::Item(id, Event::Change(_, _, value)) if id == target && matches!(value.as_ref(), crate::adaptor::presenter::state_subscription::PublishedState::Failure(_)))
    );
    *fixture.issues.failure.lock() = false;
    *fixture.issues.values.lock() = vec![issue(2)];
    fixture.reads.git_host.fetch_issues(&fixture.path).unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(event, StateSubscriptionEvent::Item(id, Event::Change(_, _, value)) if id == target && crate::test_support::state_subscription::same(&value, &StateValue::Issues(vec![issue(2).into()])))
    );
}
