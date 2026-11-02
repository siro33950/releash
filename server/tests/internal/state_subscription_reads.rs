use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_fixture;
use releashd::test_support::integration::sessions::test_sessions;
use releashd::test_support::integration::sessions::MemoryHookHealthRepository as MemoryHookHealth;
use releashd::test_support::integration::subscriptions::SubscriptionTarget as T;

use releashd::test_support::integration::subscriptions::Event;

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
use releashd::test_support::integration::platform::StateSubscriptionReads;
use releashd::test_support::integration::sessions::AgentSessionAvailability;
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
    async fn terminal_availability(
        &self,
        _: &str,
    ) -> Result<
        (
            releashd::test_support::integration::sessions::ManagedPtyPresence,
            AgentSessionAvailability,
        ),
        AgentSessionLifecycleUsecaseError,
    > {
        let item = self.item.lock();
        let Some(item) = item.as_ref() else {
            return Err(AgentSessionLifecycleUsecaseError::NotFound);
        };
        let availability = if item.lifecycle == AgentSessionLifecycleDto::Archived {
            AgentSessionAvailability::Archived
        } else {
            AgentSessionAvailability::Open
        };
        Ok((
            releashd::test_support::integration::sessions::ManagedPtyPresence::Live,
            availability,
        ))
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
        let config_mutation = Arc::new(parking_lot::Mutex::new(()));
        let config = Arc::new(AppConfig::new(
            ReleashConfig::default(),
            root.join("config.toml"),
        ));
        let repo_paths_gateway = Arc::new(RepoPathsGateway::new(
            Arc::new(parking_lot::RwLock::new(vec![path.clone()])),
            config.clone(),
        ));
        let repositories = Arc::new(RepoPathsUsecase::new(
            repo_paths_gateway.clone(),
            repo_paths_gateway,
            publisher.clone(),
            config_mutation.clone(),
        ));
        let repository_config = Arc::new(
            releashd::test_support::integration::settings::RepositoryConfigUsecase::new(
                config.clone(),
                Arc::new(
                    releashd::test_support::integration::repository::GitGateway::new(
                        config.clone(),
                    ),
                ),
                config_mutation.clone(),
            )
            .with_state_publisher(publisher.clone()),
        );
        let repository_state = Arc::new(RepositoryStateService::new(
            Arc::new(RepositoryStateRepositoryGateway::new(repository.clone())),
            Arc::new(DefaultRepositoryScanner::new(
                repository.clone(),
                Arc::new(releashd::test_support::integration::platform::build_code_usecase()),
                repository_config.clone(),
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
                config_mutation.clone(),
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
            repository_config,
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
                releashd::test_support::integration::platform::workspace_state_store(
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
                releashd::test_support::integration::settings::AppConfigUsecase::new(config.clone(), config_mutation.clone())
                    .with_state_publisher(publisher.clone()),
            ),
            notion: Arc::new(
                releashd::test_support::integration::platform::NotionUsecase::new(
                    config.clone(),
                    config.clone(),
                    Arc::new(releashd::test_support::integration::platform::NotionApiGatewayImpl::new(
                        std::sync::Arc::new(releashd::test_support::integration::platform::RetryLimiter::new()),
                    )),
                    config_mutation.clone(),
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
            usage: {
                use releashd::test_support::integration::provider_usage::*;
                let model = Arc::new(ProviderUsageReadModel::default());
                Arc::new(ProviderUsageUsecase::new(Arc::new(LocalProviderUsageSource {
                    claude_account: root.join(".claude.json"), codex_home: root.join(".codex"),
                }), model.clone(), model, Arc::new(releashd::test_support::integration::provider_usage::CodexUsageObservationStore::default()), Arc::new(SystemProviderUsageClock), publisher.clone()))
            },
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
    todo!()
}

#[tokio::test]
pub async fn test_設定購読_初期値と保存後の値を既存のproto内容で配信する() {
    use releashd::test_support::integration::subscriptions::PublishedState;
    use releashd::test_support::integration::wire::{from_message, state_payload::Value as W};
    use serde_json::json;

    // Given
    let fixture = Fixture::new();
    for (target, before, after) in [
        (
            "desktop-settings",
            json!({"startMinimized":false,"crashReporting":true,"performanceTelemetry":true,"autoLaunch":false}),
            json!({"startMinimized":true,"crashReporting":true,"performanceTelemetry":true,"autoLaunch":false}),
        ),
        (
            "workflow-config",
            json!({"approval_auto_approve":false}),
            json!({"approval_auto_approve":true}),
        ),
        (
            "terminal-startup-command",
            json!(""),
            json!("printf '準備完了\\n'"),
        ),
    ] {
        let mut stream = Box::pin(fixture.subscriptions.open(target.into()).unwrap());
        assert!(matches!(
            stream.next().await,
            Some(StateSubscriptionEvent::Ready)
        ));
        start_read(&fixture.subscriptions, target, target, None)
            .await
            .unwrap();

        for (index, expected) in [before, after].into_iter().enumerate() {
            // When
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap();
            let value = match event {
                StateSubscriptionEvent::Item(_, Event::Snapshot(_, value)) if index == 0 => value,
                StateSubscriptionEvent::Item(_, Event::Change(_, _, value)) if index == 1 => value,
                _ => panic!("settings snapshot or change"),
            };
            let PublishedState::Value(value) = value.as_ref() else {
                panic!("settings payload");
            };
            let actual = match value.value.as_ref().unwrap() {
                W::DesktopSettings(value) => {
                    from_message("releash.client.v1.DesktopSettings", value).unwrap()
                }
                W::WorkflowConfig(value) => {
                    from_message("releash.client.v1.WorkflowSection", value).unwrap()
                }
                W::TerminalStartupCommand(value) => json!(value.value.as_ref().unwrap()),
                other => panic!("unexpected payload: {other:?}"),
            };

            // Then
            assert_eq!(actual, expected, "{target}");
            if index == 0 {
                assert!(matches!(
                    stream.next().await,
                    Some(StateSubscriptionEvent::Item(_, Event::Bookmark(_)))
                ));
                match target {
                    "desktop-settings" => {
                        fixture.reads.app_config.update_app_settings(true).unwrap()
                    }
                    "workflow-config" => fixture
                        .reads
                        .app_config
                        .update_workflow_config(
                            releashd::test_support::integration::settings::WorkflowConfigInput {
                                approval_auto_approve: true,
                            },
                        )
                        .unwrap(),
                    "terminal-startup-command" => fixture
                        .reads
                        .app_config
                        .update_terminal_startup_command("printf '準備完了\\n'".into())
                        .unwrap(),
                    _ => unreachable!(),
                }
            }
        }
        fixture.subscriptions.close_client(target);
    }
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
    todo!()
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
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, releashd::test_support::integration::subscriptions::Event::Snapshot(_, value))) if releashd::test_support::integration::subscriptions::same(&value, StateValue::Issues(releashd::test_support::integration::platform::Fetched::ready(releashd::test_support::integration::platform::IssueListing { options: releashd::test_support::integration::platform::IssueOptions::from_issues(&[issue(1)]), issues: vec![issue(1)] }))))
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
            releashd::test_support::integration::platform::Fetched::ready(
                releashd::test_support::integration::platform::IssueListing {
                    options:
                        releashd::test_support::integration::platform::IssueOptions::from_issues(&[
                            issue(2)
                        ]),
                    issues: vec![issue(2)]
                }
            )
        )
    ));
    assert!(before.elapsed() < CacheTtl::EXTERNAL_INFORMATION.duration());
    assert_eq!(fixture.issues.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
pub async fn test_終了済み実行木の選択_初期購読でツリーを配信する() {
    todo!()
}

#[tokio::test]
pub async fn test_終了済み実行木のarchive_取り直しなしで空のツリーを配信する() {
    todo!()
}

#[tokio::test]
pub async fn test_終了済み実行木のrestore_取り直しなしでツリーを配信する() {
    todo!()
}

#[tokio::test]
pub async fn test_sessionタブ購読_背景処理の注意状態の発生と解消を同じ購読へ配信する() {
    todo!()
}

#[tokio::test]
pub async fn test_sessionタブ購読_子の背景失敗の発生と解消を親の同じ購読へ配信し一覧と一致する() {
    todo!()
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
    let expected = (item.clone(), AgentSessionAvailability::Open);
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(id, Event::Snapshot(_, value))) if id == format!("client:{target}") && releashd::test_support::integration::subscriptions::same(&value, StateValue::AgentSession(Some(expected))))
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
        let expected = next.clone().map(|value| {
            let availability = if value.lifecycle == AgentSessionLifecycleDto::Archived {
                AgentSessionAvailability::Archived
            } else {
                AgentSessionAvailability::Open
            };
            (value, availability)
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
            matches!(event, Some(StateSubscriptionEvent::Item(id, Event::Change(_, releashd::test_support::integration::subscriptions::Delivery::Full, value))) if id == format!("client:{target}") && releashd::test_support::integration::subscriptions::same(&value, StateValue::AgentSession(expected)))
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
            value: Some(releashd::test_support::integration::platform::IssueListing { options: releashd::test_support::integration::platform::IssueOptions::from_issues(&[issue(1)]), issues: vec![issue(1)] }),
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
        matches!(event, StateSubscriptionEvent::Item(id, Event::Change(_, _, value)) if id == format!("client:{target}") && releashd::test_support::integration::subscriptions::same(&value, StateValue::Issues(releashd::test_support::integration::platform::Fetched::ready(releashd::test_support::integration::platform::IssueListing { options: releashd::test_support::integration::platform::IssueOptions::from_issues(&[issue(2)]), issues: vec![issue(2)] }))))
    );
}

#[tokio::test]
pub async fn test_workspace購読の値_reactが配置を省略して保存した後もswiftの配置を含む() {
    use releashd::test_support::integration::platform::{
        save_workspace_state, PaneLayout, PaneTab, PaneTabKind, WorkspaceLayoutState,
        WorkspaceState, WorkspaceTabsState,
    };
    // Given
    let fixture = Fixture::new();
    let reads = &fixture.reads;
    let key = &fixture.path;
    let panes = PaneLayout::pane(
        "pane".into(),
        vec![PaneTab {
            agent_session_id: None,
            node_id: None,
            id: "workflow".into(),
            kind: PaneTabKind::Command,
            file_path: None,
            file_mode: None,
            file_base: None,
        }],
        Some("workflow".into()),
    )
    .unwrap();
    let state = WorkspaceState {
        panes: Some(panes),
        version: 1,
        tabs: WorkspaceTabsState {
            editors: vec![],
            active_editor_path: None,
        },
        layout: WorkspaceLayoutState {
            center_tab: "agent".into(),
            active_view: "git".into(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
            right_bottom_active_tab: None,
            selected_diff_file: None,
        },
    };
    save_workspace_state(
        reads.workspace_state.as_ref(),
        Some(&fixture.subscriptions),
        key,
        state.clone(),
    )
    .await
    .unwrap();
    let mut react = state.clone();
    react.panes = None;
    react.layout.left_nav_collapsed = true;
    // When
    save_workspace_state(
        reads.workspace_state.as_ref(),
        Some(&fixture.subscriptions),
        key,
        react.clone(),
    )
    .await
    .unwrap();
    let actual = reads
        .read(&T::WorkspaceState(key.clone(), key.clone()))
        .await
        .unwrap();
    // Then
    react.panes = state.panes;
    assert_eq!(
        actual,
        StateValue::WorkspaceState(Some(react.into()), Vec::new(), None)
    );
}

#[tokio::test]
pub async fn test_sessionタブ購読_idだけでavailabilityと操作可否を配り不在も扱う() {
    // Given
    let fixture = Fixture::new();
    let reads = StateSubscriptionReads(fixture.reads.clone());
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
            can_archive: false,
            can_restore: false,
            can_delete: false,
        },
        last_exit_abnormal: false,
        terminal_presence: Some("live".into()),
    };
    *fixture.sessions.item.lock() = Some(item.clone());
    // When / Then
    let value = reads.read(&T::SessionTab("session".into())).await.unwrap();
    assert_eq!(
        value,
        StateValue::SessionTab {
            session: Some((item.clone(), AgentSessionAvailability::Open)),
            node: None
        }
    );
    let mut archived = item;
    archived.lifecycle = AgentSessionLifecycleDto::Archived;
    *fixture.sessions.item.lock() = Some(archived.clone());
    let value = reads.read(&T::SessionTab("session".into())).await.unwrap();
    assert_eq!(
        value,
        StateValue::SessionTab {
            session: Some((archived, AgentSessionAvailability::Archived)),
            node: None
        }
    );
    *fixture.sessions.item.lock() = None;
    assert_eq!(
        reads.read(&T::SessionTab("session".into())).await.unwrap(),
        StateValue::SessionTab {
            session: None,
            node: None
        }
    );
    assert_eq!(
        *fixture.sessions.calls.lock(),
        vec!["session", "session", "session"]
    );
}

#[tokio::test]
pub async fn test_保存済み会話タブの購読_旧参照を解決し一部の取得失敗でも配置と参照を保持する() {
    todo!()
}

#[tokio::test]
pub async fn test_状態読取_baseの保存と自動への復帰を購読値に反映する() {
    // Given
    let fixture = Fixture::new();
    let target = SubscriptionTarget::ReleashBase(fixture.path.clone());
    for base in [Some("selected-base"), None] {
        // When
        fixture
            .reads
            .repository_config
            .set_releash_base(&fixture.path, base)
            .unwrap();
        let value = fixture.reads.read(&target).await.unwrap();
        // Then
        assert!(matches!(value, StateValue::ReleashBase(actual) if actual.as_deref() == base));
    }
}
