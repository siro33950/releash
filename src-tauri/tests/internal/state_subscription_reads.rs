use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_fixture;
use releashd::test_support::integration::sessions::MemoryHookHealthRepository as MemoryHookHealth;
use releashd::test_support::integration::subscriptions::SubscriptionTarget as T;

use releashd::test_support::integration::subscriptions::Event;

use releashd::test_support::integration::workflow::NoopWorkflowExternalEditorGateway;
use releashd::test_support::integration::workflow::PassthroughManagedWorktreeGateway;

use releashd::test_support::integration::repository::FsWorktreePathNormalizer;

use releashd::test_support::integration::repository::NotifyRepositoryStateWatcher;

use releashd::test_support::integration::platform::InMemoryTtlCache;
use releashd::test_support::integration::repository::DefaultRepositoryScanner;
use releashd::test_support::integration::repository::RepoPathsGateway;
use releashd::test_support::integration::settings::AppConfig;
use releashd::test_support::integration::settings::ReleashConfig;

use releashd::test_support::integration::platform::CacheTtl;
use releashd::test_support::integration::platform::GitHostError;
use releashd::test_support::integration::platform::GitHostProvider;
use releashd::test_support::integration::platform::IssueInfo;
use releashd::test_support::integration::platform::PrStatus;
use releashd::test_support::integration::repository::RepositoryStateRepositoryGateway;
use releashd::test_support::integration::workflow::facet_FacetKind as FacetKind;
use std::sync::Arc;
async fn start_read(
    usecase: &releashd::test_support::integration::subscriptions::StateSubscriptionUsecase,
    client: &str,
    target: &str,
    cursor: Option<(&str, u64)>,
) -> Result<(), releashd::test_support::integration::subscriptions::StateReadError> {
    let target =
        releashd::test_support::integration::subscriptions::SubscriptionTarget::parse(target)
            .map_err(
                releashd::test_support::integration::subscriptions::StateReadError::from_error,
            )?;
    usecase
        .deps()
        .start_subscription(client, &target, &format!("{client}:{target}"), cursor)
        .await
}
use futures_util::StreamExt;
use parking_lot::Mutex;
use releashd::test_support::integration::platform::AgentSessionProviderDto;
use releashd::test_support::integration::platform::GitHostUsecase;
use releashd::test_support::integration::platform::RepoPathsUsecase;
use releashd::test_support::integration::platform::RepositoryStateService;
use releashd::test_support::integration::sessions::AgentSessionGarbageCollectionOutcome;
use releashd::test_support::integration::sessions::AgentSessionGarbageCollectionPort;
use releashd::test_support::integration::sessions::AgentSessionHistoryCandidateDto;
use releashd::test_support::integration::sessions::AgentSessionHistoryPageDto;
use releashd::test_support::integration::sessions::AgentSessionHistoryQueryError;
use releashd::test_support::integration::sessions::AgentSessionHistoryQueryService;
use releashd::test_support::integration::sessions::AgentSessionHistoryReadUsecase;
use releashd::test_support::integration::sessions::AgentSessionHistoryRequest;
use releashd::test_support::integration::sessions::AgentSessionItemDto;
use releashd::test_support::integration::sessions::AgentSessionLifecycleDto;
use releashd::test_support::integration::sessions::AgentSessionLifecycleUsecaseError;
use releashd::test_support::integration::sessions::AgentSessionOperationsDto;
use releashd::test_support::integration::sessions::AgentSessionQueryError;
use releashd::test_support::integration::sessions::AgentSessionQueryService;
use releashd::test_support::integration::sessions::AgentSessionReadUsecase;
use releashd::test_support::integration::sessions::AgentSessionTreeLocationDto;
use releashd::test_support::integration::sessions::ProviderAvailabilityUsecase;
use releashd::test_support::integration::subscriptions::StateChangeSource;
use releashd::test_support::integration::subscriptions::StateReadFailure;
use releashd::test_support::integration::subscriptions::StateSubscriptionEvent;
use releashd::test_support::integration::subscriptions::StateSubscriptionRead;
use releashd::test_support::integration::subscriptions::StateSubscriptionUsecase;
use releashd::test_support::integration::subscriptions::StateValue;
use releashd::test_support::integration::subscriptions::SubscriptionTarget;
use releashd::test_support::integration::subscriptions::WorkspaceStateReads;
use releashd::test_support::integration::workflow::WorkflowDiagnosticsTarget;
use releashd::test_support::integration::workspace::WorkspaceListUsecase;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

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
    async fn terminal_presence(
        &self,
        _: &str,
    ) -> Result<
        releashd::test_support::integration::sessions::ManagedPtyPresence,
        AgentSessionLifecycleUsecaseError,
    > {
        Ok(releashd::test_support::integration::sessions::ManagedPtyPresence::Live)
    }
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
#[async_trait::async_trait]
impl GitHostProvider for Issues {
    async fn fetch_pr_status(&self, _: &str) -> Result<PrStatus, GitHostError> {
        Ok(PrStatus::default())
    }
    async fn list_issues(&self, _: &str) -> Result<Vec<IssueInfo>, GitHostError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.own_runtime.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
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
        author: releashd::test_support::integration::platform::PrAuthor {
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

struct NoHookHealthFailures;
#[async_trait::async_trait]
impl releashd::test_support::integration::providers::ProviderHookHealthFailureQuery
    for NoHookHealthFailures
{
    async fn list(
        &self,
        _: usize,
    ) -> Result<
        Vec<
            Result<
                releashd::test_support::integration::providers::ProviderHookHealthFailureObservation,
                releashd::test_support::integration::providers::ProviderHookHealthFailureQueryError,
            >,
        >,
        releashd::test_support::integration::providers::ProviderHookHealthFailureQueryError,
    >{
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
        crate::test_support_git::create_initial_commit(&git);
        let commit = git.head().unwrap().peel_to_commit().unwrap();
        git.branch("feature", &commit, false).unwrap();
        let subscriptions = StateSubscriptionUsecase::new(
            vec![path.clone()],
            releashd::test_support::integration::subscriptions::read_driver(),
        );
        let publisher = subscriptions.clone();
        let repository = Arc::new(
            releashd::test_support::integration::platform::build_repository_usecase()
                .with_state_publisher(publisher.clone()),
        );
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
                Arc::new(releashd::test_support::integration::platform::build_code_usecase()),
            )),
            publisher.clone(),
            Arc::new(NotifyRepositoryStateWatcher::new(repository.clone())),
            Arc::new(
                releashd::test_support::integration::platform::RepositoryScanWorkerRuntime::new(),
            ),
            Arc::new(FsWorktreePathNormalizer),
            releashd::test_support::integration::platform::start(
                releashd::test_support::integration::platform::test_retrying(),
                Arc::new(
                    releashd::test_support::integration::platform::RepositoryScanWorkerRuntime::new(
                    ),
                ),
                releashd::test_support::integration::platform::delays(
                    releashd::test_support::integration::platform::DEBOUNCE,
                ),
            ),
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
        let (workflow, local_event_store) =
            releashd::test_support::integration::platform::build_workflow_usecase_and_store(
                root.join("data"),
                Some(workflows_dir.clone()),
            );
        let workflow = Arc::new(workflow);
        let issues = Arc::new(Issues::default());
        *issues.values.lock() = vec![issue(1)];
        let git_host = Arc::new(
            GitHostUsecase::new(
                issues.clone(),
                Arc::new(
                    releashd::test_support::integration::platform::LatestPrStatuses::default(),
                ),
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
                    releashd::test_support::integration::sessions::FakeProviderExecutableConfigRepository::default(),
                ),
                Arc::new(
                    releashd::test_support::integration::sessions::FakeProviderExecutableProbeGateway::default(),
                ),
            )
            .unwrap()
            .with_state_publisher(publisher.clone()),
        );
        let comments = Arc::new(
            releashd::test_support::integration::platform::build_review_comment_usecase()
                .with_subscriptions(subscriptions.clone()),
        );
        let session_comments = Arc::new(releashd::test_support::integration::platform::SessionReviewUsecase::new(
            releashd::test_support::integration::platform::ReviewContextUsecase::new(
                Arc::new(releashd::test_support::integration::sessions::LocalAgentSessionRepository::new(local_event_store.clone())),
                Arc::new(releashd::test_support::integration::workflow::StoredWorkspaceWorktreePathQuery::new(root.join("data"), Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()))),
            ), comments.clone(),
        ));
        let reads = WorkspaceStateReads {
            daemon: releashd::test_support::integration::daemon::DaemonUsecase::test_with_repository(
                releashd::test_support::integration::daemon::serving(),
            ),
            repositories,
            repository,
            workflow,
            workspaces,
            git_host,
            sessions: Arc::new(AgentSessionReadUsecase::new(
                std::sync::Arc::new(releashd::test_support::integration::platform::RandomIdentityIssuer),
                sessions.clone(),
                sessions.clone(),
            )),
            history: Arc::new(AgentSessionHistoryReadUsecase::new(sessions.clone())),
            providers,
            workspace_state: Arc::new(
                releashd::test_support::integration::platform::WorkspaceStateStore::new(
                    root.join("workspace"),
                ),
            ),
            review: Arc::new(releashd::test_support::integration::platform::ReviewUsecase::new(
                repository_state.clone(),
                Arc::new(releashd::test_support::integration::platform::build_code_usecase()),
            )),
            comments,
            session_comments,
            data_dir: root.to_path_buf(),
            review_comments_dir: releashd::test_support::integration::platform::state_dir(&root),
            workflows_dir: workflows_dir.clone(),
            app_config: Arc::new(
                releashd::test_support::integration::settings::AppConfigUsecase::new(config.clone(), config.clone())
                    .with_state_publisher(publisher.clone()),
            ),
            notion: Arc::new(
                releashd::test_support::integration::platform::NotionUsecase::new(
                    config.clone(),
                    config.clone(),
                    Arc::new(releashd::test_support::integration::platform::NotionApiGatewayImpl::new(
                        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
                    )),
                )
                .with_state_publisher(publisher.clone()),
            ),
            editor_settings: Arc::new(
                releashd::test_support::integration::platform::EditorSettingsConfigGateway::new(
                    config.clone(),
                ),
            ),
            editor_scanner: Arc::new(
                releashd::test_support::integration::platform::MacInstalledEditorGateway,
            ),
            hook_health: Arc::new(
                releashd::test_support::integration::providers::ProviderHookHealthReadUsecase::new(
                    Arc::new(
                        releashd::test_support::integration::providers::ProviderHookHealthUsecase::new(
                            Arc::new(MemoryHookHealth::default()),
                        )
                        .with_state_publisher(publisher.clone()),
                    ),
                    Arc::new(NoHookHealthFailures),
                ),
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
pub async fn test_状態読取_全対象を対応するサービスへ引数付きで振り分ける() {
    // Given
    use SubscriptionTarget as T;
    let fixture = Fixture::new();
    let r = &fixture.reads;
    let p = &fixture.path;
    let branch = r.repository.get_current_branch(p).unwrap();
    r.repository
        .set_branch_base_override(p, "feature", Some(&branch))
        .unwrap();
    let workspace: releashd::test_support::integration::platform::WorkspaceStateDto = serde_json::from_value(serde_json::json!({
        "version": 1, "tabs": {"editors": [], "activeEditorPath": null},
        "layout": {"centerTab": "agent", "activeView": "git", "leftNavCollapsed": true, "rightCollapsed": false, "rightBottomCollapsed": false}
    })).unwrap();
    releashd::test_support::integration::platform::save_workspace_state(
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
            StateValue::Branches(vec![
                releashd::test_support::integration::platform::BranchDto {
                    name: branch.clone(),
                    is_remote: false,
                },
            ]),
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
            StateValue::Issues(
                releashd::test_support::integration::platform::Fetched::ready(vec![issue(1)]),
            ),
        ),
        (
            T::Worktrees(p.clone()),
            StateValue::Worktrees(r.repository.list_worktrees(p).unwrap()),
        ),
        (
            T::StartupRepository,
            StateValue::StartupRepository(r.repository.startup_worktree().unwrap()),
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
                    .map(releashd::test_support::integration::workflow::facet_summary_to_dto)
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
    // When
    let mut actual = Vec::new();
    for (target, expected) in cases {
        let value = r
            .read(&target)
            .await
            .unwrap_or_else(|error| panic!("{target}: {error}"));
        actual.push((target, value, expected));
    }
    // Then
    for (target, value, expected) in actual {
        assert_eq!(value, expected, "{target}");
    }
    assert_eq!(
        *fixture.sessions.calls.lock(),
        ["missing-session".to_string(), format!("{p}:120")]
    );
    assert_eq!(fixture.issues.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
pub async fn test_状態読取_workflow一覧に置き場所の定義を含む() {
    // Given
    let fixture = Fixture::new();
    // When
    let value = fixture
        .reads
        .read(&SubscriptionTarget::Workflows)
        .await
        .unwrap();
    // Then
    let StateValue::Workflows(workflows) = value else {
        panic!("unexpected state")
    };
    assert!(workflows.iter().any(|workflow| workflow.name == "fixture"));
}

#[tokio::test]
pub async fn test_状態読取_workflowの本文を読む() {
    // Given
    let fixture = Fixture::new();
    // When
    let value = fixture
        .reads
        .read(&SubscriptionTarget::Workflow("fixture".into()))
        .await
        .unwrap();
    // Then
    let StateValue::Workflow(Some(workflow)) = value else {
        panic!("unexpected state")
    };
    assert_eq!(workflow.name, "fixture");
}

#[tokio::test]
pub async fn test_状態読取_診断に壊れた定義を含む() {
    // Given
    let fixture = Fixture::new();
    // When
    let value = fixture
        .reads
        .read(&SubscriptionTarget::Diagnostics)
        .await
        .unwrap();
    // Then
    let StateValue::Diagnostics(report) = value else {
        panic!("unexpected state")
    };
    assert!(report.workflow_summaries.contains_key("broken"));
}

#[tokio::test]
pub async fn test_状態読取_壊れた定義を失敗として返す() {
    // Given
    let fixture = Fixture::new();
    // When
    let error = fixture
        .reads
        .read(&SubscriptionTarget::Workflow("broken".into()))
        .await
        .unwrap_err();
    // Then
    assert!(matches!(error.source, StateReadFailure::Workflow(_)));
}

#[tokio::test]
pub async fn test_状態読取_facet不在を失敗として返す() {
    // Given
    let fixture = Fixture::new();
    // When
    let error = fixture
        .reads
        .read(&SubscriptionTarget::Facet(
            FacetKind::Instruction,
            "missing-facet".into(),
        ))
        .await
        .unwrap_err();
    // Then
    assert!(matches!(error.source, StateReadFailure::Workflow(_)));
}

#[tokio::test]
pub async fn test_状態読取_current_branchの失敗を返す() {
    // Given
    let fixture = Fixture::new();
    let expected = fixture
        .reads
        .repository
        .get_current_branch("/missing/repository")
        .unwrap_err();
    // When
    let error = fixture
        .reads
        .read(&SubscriptionTarget::CurrentBranch(
            "/missing/repository".into(),
        ))
        .await
        .unwrap_err();
    // Then
    assert!(
        matches!(error.source, StateReadFailure::Repository(actual) if format!("{actual:?}") == format!("{expected:?}"))
    );
}

#[tokio::test]
pub async fn test_issue手動更新_有効なcacheを無視し30秒前に同じ購読へ変更を配信する() {
    // Given
    let fixture = Fixture::new();
    let mut stream = Box::pin(fixture.subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::Issues(fixture.path.clone()).to_string();
    start_read(&fixture.subscriptions, "client", &target, None)
        .await
        .unwrap();
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, releashd::test_support::integration::subscriptions::Event::Snapshot(_, value))) if releashd::test_support::integration::subscriptions::same(&value, StateValue::Issues(releashd::test_support::integration::platform::Fetched::ready(vec![issue(1)]))))
    );
    stream.next().await;
    *fixture.issues.values.lock() = vec![issue(2)];
    // When
    let before = std::time::Instant::now();
    fixture
        .reads
        .git_host
        .fetch_issues(&fixture.path)
        .await
        .unwrap();
    let value = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Some(StateSubscriptionEvent::Item(
                id,
                releashd::test_support::integration::subscriptions::Event::Change(_, _, value),
            )) = stream.next().await
            {
                assert_eq!(id, format!("client:{target}"));
                break value;
            }
        }
    })
    .await
    .unwrap();
    // Then
    assert!(releashd::test_support::integration::subscriptions::same(
        &value,
        StateValue::Issues(
            releashd::test_support::integration::platform::Fetched::ready(vec![issue(2)])
        )
    ));
    assert!(before.elapsed() < CacheTtl::EXTERNAL_INFORMATION.duration());
    assert_eq!(fixture.issues.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
pub async fn test_終了済み実行木の選択_初期購読でツリーを配信する() {
    // Given
    use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_fixture;
    use releashd::test_support::integration::subscriptions::Event;

    use releashd::test_support::integration::workflow::NoopWorkflowExternalEditorGateway;
    use releashd::test_support::integration::workflow::PassthroughManagedWorktreeGateway;
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
            releashd::test_support::integration::workflow::ExecutionOrigin::Cli,
        )
        .await
        .unwrap();
    archive
        .runtime
        .abort_execution(
            releashd::test_support::integration::workflow::AbortExecutionCommand {
                execution_id: id.clone(),
                expected_node_name: None,
            },
        )
        .await
        .unwrap();
    let mut reads = fixture.reads.clone();
    reads.workflow = Arc::new(
        releashd::test_support::integration::platform::build_workflow_services_with_gateways(
            Arc::new(releashd::test_support::integration::platform::FailureRecordStore::default()),
            archive.directory.path(),
            Arc::new(PassthroughManagedWorktreeGateway),
            Arc::new(NoopWorkflowExternalEditorGateway),
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
    // When
    start_read(&subscriptions, "client", &target, None)
        .await
        .unwrap();
    let event = stream.next().await;
    // Then
    let Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) = event else {
        panic!("initial snapshot")
    };
    let Some(releashd::test_support::integration::wire::state_payload::Value::Selection(initial)) =
        &match value.as_ref() {
            releashd::test_support::integration::subscriptions::PublishedState::Value(value) => {
                value
            }
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
}

#[tokio::test]
pub async fn test_終了済み実行木のarchive_取り直しなしで空のツリーを配信する() {
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
            releashd::test_support::integration::workflow::ExecutionOrigin::Cli,
        )
        .await
        .unwrap();
    archive
        .runtime
        .abort_execution(
            releashd::test_support::integration::workflow::AbortExecutionCommand {
                execution_id: id.clone(),
                expected_node_name: None,
            },
        )
        .await
        .unwrap();
    let mut reads = fixture.reads.clone();
    reads.workflow = Arc::new(
        releashd::test_support::integration::platform::build_workflow_services_with_gateways(
            Arc::new(releashd::test_support::integration::platform::FailureRecordStore::default()),
            archive.directory.path(),
            Arc::new(PassthroughManagedWorktreeGateway),
            Arc::new(NoopWorkflowExternalEditorGateway),
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
        .await
        .unwrap();
    stream.next().await.unwrap();
    stream.next().await.unwrap();
    // When
    archive
        .runtime
        .archive_execution_tree(&id, "manual")
        .await
        .unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap();
    // Then
    let Some(StateSubscriptionEvent::Item(received, Event::Change(_, _, value))) = event else {
        panic!("changed tree")
    };
    assert_eq!(received, format!("client:{target}"));
    let Some(releashd::test_support::integration::wire::state_payload::Value::Selection(selection)) =
        &match value.as_ref() {
            releashd::test_support::integration::subscriptions::PublishedState::Value(value) => {
                value
            }
            _ => panic!("selection"),
        }
        .value
    else {
        panic!("selection")
    };
    assert!(selection
        .snapshot
        .as_ref()
        .unwrap()
        .nodes
        .as_ref()
        .unwrap()
        .items
        .is_empty());
}

#[tokio::test]
pub async fn test_終了済み実行木のrestore_取り直しなしでツリーを配信する() {
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
            releashd::test_support::integration::workflow::ExecutionOrigin::Cli,
        )
        .await
        .unwrap();
    archive
        .runtime
        .abort_execution(
            releashd::test_support::integration::workflow::AbortExecutionCommand {
                execution_id: id.clone(),
                expected_node_name: None,
            },
        )
        .await
        .unwrap();
    let mut reads = fixture.reads.clone();
    reads.workflow = Arc::new(
        releashd::test_support::integration::platform::build_workflow_services_with_gateways(
            Arc::new(releashd::test_support::integration::platform::FailureRecordStore::default()),
            archive.directory.path(),
            Arc::new(PassthroughManagedWorktreeGateway),
            Arc::new(NoopWorkflowExternalEditorGateway),
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
        .await
        .unwrap();
    stream.next().await.unwrap();
    stream.next().await.unwrap();
    archive
        .runtime
        .archive_execution_tree(&id, "manual")
        .await
        .unwrap();
    stream.next().await.unwrap();
    // When
    archive.runtime.restore_execution_tree(&id).await.unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap();
    // Then
    let Some(StateSubscriptionEvent::Item(received, Event::Change(_, _, value))) = event else {
        panic!("changed tree")
    };
    assert_eq!(received, format!("client:{target}"));
    let Some(releashd::test_support::integration::wire::state_payload::Value::Selection(selection)) =
        &match value.as_ref() {
            releashd::test_support::integration::subscriptions::PublishedState::Value(value) => {
                value
            }
            _ => panic!("selection"),
        }
        .value
    else {
        panic!("selection")
    };
    assert!(!selection
        .snapshot
        .as_ref()
        .unwrap()
        .nodes
        .as_ref()
        .unwrap()
        .items
        .is_empty());
}

#[tokio::test]
pub async fn test_agent_session購読_状態変更通知から再読取して同じ購読へ配信する() {
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
        terminal_presence: Some("live".into()),
    };
    *fixture.sessions.item.lock() = Some(item.clone());
    let mut stream = Box::pin(fixture.subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::AgentSession(item.id.clone()).to_string();
    start_read(&fixture.subscriptions, "client", &target, None)
        .await
        .unwrap();
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(id, Event::Snapshot(_, value))) if id == format!("client:{target}") && releashd::test_support::integration::subscriptions::same(&value, StateValue::AgentSession(Some(item.clone()))))
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
            matches!(event, Some(StateSubscriptionEvent::Item(id, Event::Change(_, releashd::test_support::integration::subscriptions::Delivery::Full, value))) if id == format!("client:{target}") && releashd::test_support::integration::subscriptions::same(&value, StateValue::AgentSession(next)))
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
pub async fn test_状態読取_review対象をworktreeとcomment置き場から読み取る() {
    use releashd::test_support::integration::platform::ReviewFileViewDto;
    use releashd::test_support::integration::review::ReviewBase;
    use releashd::test_support::integration::review::ReviewSection;

    // Given
    let fixture = Fixture::new();
    let r = &fixture.reads;
    let p = &fixture.path;
    let git = git2::Repository::open(p).unwrap();
    crate::test_support_git::add_and_commit(&git, "review.txt", "before\n", "tracked");
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
        releashd::test_support::integration::platform::state_dir(&r.data_dir)
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
pub async fn test_issue手動更新失敗_最後の一覧と失敗を購読へ届ける() {
    // Given

    let fixture = Fixture::new();
    let mut stream = Box::pin(fixture.subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::Issues(fixture.path.clone()).to_string();
    start_read(&fixture.subscriptions, "client", &target, None)
        .await
        .unwrap();
    stream.next().await.unwrap();
    stream.next().await.unwrap();
    // When
    *fixture.issues.failure.lock() = true;
    let result = fixture.reads.git_host.fetch_issues(&fixture.path).await;
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap();
    // Then
    assert!(result.is_err());
    assert!(
        matches!(event, StateSubscriptionEvent::Item(id, Event::Change(_, _, value)) if id == format!("client:{target}") && releashd::test_support::integration::subscriptions::same(&value, StateValue::Issues(releashd::test_support::integration::platform::Fetched {
            value: Some(vec![issue(1)]),
            error: Some(releashd::test_support::integration::platform::WorkFailure::from_error(&GitHostError::External("issues offline".into()))),
        })))
    );
}

#[tokio::test]
pub async fn test_issue手動更新失敗_回復時に新しい一覧を届ける() {
    // Given

    let fixture = Fixture::new();
    let mut stream = Box::pin(fixture.subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::Issues(fixture.path.clone()).to_string();
    start_read(&fixture.subscriptions, "client", &target, None)
        .await
        .unwrap();
    stream.next().await.unwrap();
    stream.next().await.unwrap();
    *fixture.issues.failure.lock() = true;
    let _ = fixture.reads.git_host.fetch_issues(&fixture.path).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap();
    // When
    *fixture.issues.failure.lock() = false;
    *fixture.issues.values.lock() = vec![issue(2)];
    fixture
        .reads
        .git_host
        .fetch_issues(&fixture.path)
        .await
        .unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap();
    // Then
    assert!(
        matches!(event, StateSubscriptionEvent::Item(id, Event::Change(_, _, value)) if id == format!("client:{target}") && releashd::test_support::integration::subscriptions::same(&value, StateValue::Issues(releashd::test_support::integration::platform::Fetched::ready(vec![issue(2)]))))
    );
}
