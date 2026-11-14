use crate::adaptor_gateway_workflow_workflow_host_test_helpers::archive_fixture;
use releashd::test_support::integration::platform::FailureRecordStore;
use releashd::test_support::integration::sessions::test_sessions;
use releashd::test_support::integration::sessions::MemoryHookHealthRepository as MemoryHookHealth;
use releashd::test_support::integration::subscriptions::SubscriptionTarget as T;
use releashd::test_support::integration::workflow::WorkflowDiagnosticsTarget;

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
) -> Result<(), releashd::test_support::integration::subscriptions::StateReadError> {
    let target =
        releashd::test_support::integration::subscriptions::SubscriptionTarget::parse(target)
            .map_err(
                releashd::test_support::integration::subscriptions::StateReadError::from_error,
            )?;
    usecase
        .deps()
        .start_subscription(client, &target, &format!("{client}:{target}"))
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
use releashd::test_support::integration::subscriptions::StateSubscriptionEvent;
use releashd::test_support::integration::subscriptions::StateSubscriptionRead;
use releashd::test_support::integration::subscriptions::StateSubscriptionUsecase;
use releashd::test_support::integration::subscriptions::StateValue;
use releashd::test_support::integration::subscriptions::SubscriptionTarget;
use releashd::test_support::integration::subscriptions::WorkspaceStateReads;
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
    // Given
    use SubscriptionTarget as T;
    let fixture = Fixture::new();
    let r = &fixture.reads;
    let p = &fixture.path;
    let branch = releashd::test_support::integration::repository::get_current_branch(p).unwrap();
    let workspace = releashd::test_support::integration::platform::WorkspaceState {
        version: 1,
        panes: None,
        tabs: Some(
            releashd::test_support::integration::platform::WorkspaceTabsState { editors: vec![] },
        ),
        layout: Some(
            releashd::test_support::integration::platform::WorkspaceLayoutState {
                center_tab: "agent".into(),
                active_view: "git".into(),
                left_nav_collapsed: true,
                right_collapsed: false,
                right_bottom_collapsed: false,
            },
        ),
    };
    releashd::test_support::integration::platform::save_workspace_state(
        r.workspace_state.as_ref(),
        None,
        "repo",
        workspace.clone(),
    )
    .await
    .unwrap();
    r.workspaces.refresh().await;
    let cases = vec![
        (
            T::Workspaces,
            StateValue::Workspaces(r.workspaces.read().await.unwrap()),
        ),
        (
            T::NodeDetail(p.clone(), "missing".into()),
            StateValue::NodeDetail(None),
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
            T::Issues(p.clone()),
            StateValue::Issues(
                releashd::test_support::integration::platform::Fetched::ready(
                    releashd::test_support::integration::platform::IssueListing {
                        options:
                            releashd::test_support::integration::platform::IssueOptions::from_issues(
                                &[issue(1)],
                            ),
                        issues: vec![issue(1)],
                    },
                ),
            ),
        ),
        (
            T::WorkspaceState("repo".into(), p.clone()),
            StateValue::WorkspaceState(
                releashd::test_support::integration::platform::StreamVersion::new(1).unwrap(),
                Some(workspace.initialize_panes()),
                Vec::new(),
                None,
            ),
        ),
        (
            T::Workflows,
            StateValue::Workflows(r.workflow.list_workflow_summaries().await.unwrap()),
        ),
        (
            T::Workflow("fixture".into()),
            StateValue::Workflow(r.workflow.get_workflow("fixture").unwrap()),
        ),
        (
            T::Facets(FacetKind::Instruction),
            StateValue::Facets(
                r.workflow
                    .list_facet_summaries(FacetKind::Instruction)
                    .unwrap(),
            ),
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
    assert_eq!(*fixture.sessions.calls.lock(), [format!("{p}:120")]);
    assert_eq!(fixture.issues.calls.load(Ordering::SeqCst), 1);
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
        start_read(&fixture.subscriptions, target, target)
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
    // Given
    let fixture = Fixture::new();
    // When
    let value = fixture
        .reads
        .read(&SubscriptionTarget::Workflow("fixture".into()))
        .await
        .unwrap();
    // Then
    let StateValue::Workflow(workflow) = value else {
        panic!("unexpected state")
    };
    assert_eq!(workflow.unwrap().content().unwrap().name, "fixture");
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
pub async fn test_状態読取_壊れた定義の解釈失敗を空の詳細として配信する() {
    use releashd::test_support::integration::{subscriptions::payload, wire};
    // Given
    let fixture = Fixture::new();
    // When
    let value = fixture
        .reads
        .read(&SubscriptionTarget::Workflow("broken".into()))
        .await
        .unwrap();
    let message = payload(&value).unwrap();
    // Then
    assert!(matches!(
        message.value,
        Some(wire::state_payload::Value::Workflow(value)) if value.value.is_none()
    ));
}

#[tokio::test]
pub async fn test_状態読取_存在しない定義を空の詳細として配信する() {
    use releashd::test_support::integration::{subscriptions::payload, wire};
    // Given
    let fixture = Fixture::new();
    // When
    let value = fixture
        .reads
        .read(&SubscriptionTarget::Workflow("missing".into()))
        .await
        .unwrap();
    let message = payload(&value).unwrap();
    // Then
    assert!(
        matches!(message.value, Some(wire::state_payload::Value::Workflow(value)) if value.value.is_none())
    );
}

#[tokio::test]
pub async fn test_issue手動更新_有効なcacheを無視し30秒前に同じ購読へ変更を配信する() {
    // Given
    let fixture = Fixture::new();
    let mut stream = Box::pin(fixture.subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::Issues(fixture.path.clone()).to_string();
    start_read(&fixture.subscriptions, "client", &target)
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
pub async fn test_sessionタブ購読_背景処理の注意状態の発生と解消を同じ購読へ配信する() {
    use releashd::test_support::integration::platform::{
        Failure, FailureKey, FailureRecordStore, FailureRecordingUsecase, TechnicalFailureNature,
        WorkFailure,
    };
    use releashd::test_support::integration::providers::ProviderKind;
    use releashd::test_support::integration::sessions::{
        AgentSessionTreeLocation, LocalAgentSessionQueryService, LocalAgentSessionRepository,
    };
    use releashd::test_support::integration::workflow::AgentSessionActivity;
    use releashd::test_support::integration::workspace::WorkspaceIdentity;

    // Given
    let fixture = Fixture::new();
    let archive = archive_fixture();
    let sessions = test_sessions(
        Arc::new(LocalAgentSessionRepository::new(archive.store.clone())),
        Arc::default(),
        Arc::default(),
        Arc::default(),
    );
    sessions
        .create(
            "session-tab",
            WorkspaceIdentity::new(&fixture.path),
            &fixture.path,
            ProviderKind::Codex,
            AgentSessionTreeLocation::session_tree_root("session-tab").unwrap(),
            "create-session-tab",
        )
        .await
        .unwrap();
    sessions
        .observe_activity(
            "session-tab",
            AgentSessionActivity::AwaitingInstruction,
            "idle-session-tab",
        )
        .await
        .unwrap();
    let item = LocalAgentSessionQueryService::new(archive.store.clone())
        .get("session-tab")
        .await
        .unwrap()
        .unwrap();
    *fixture.sessions.item.lock() = Some(item.clone());
    let failures = Arc::new(FailureRecordStore::default());
    let mut reads = fixture.reads.clone();
    reads.workflow = Arc::new(
        releashd::test_support::integration::platform::build_workflow_services_with_gateways(
            failures.clone(),
            Arc::new(PassthroughManagedWorktreeGateway),
            Arc::new(releashd::test_support::integration::workflow::NoopEditorSettingsGateway),
            Arc::new(releashd::test_support::integration::workflow::NoopEditorLauncherGateway),
            releashd::test_support::integration::workspace::ExecutionReads::new(
                archive.store.clone(),
            ),
            None,
            None,
        )
        .0,
    );
    let subscriptions = fixture.subscriptions.with_reads(
        Arc::new(StateSubscriptionReads(reads.clone())),
        None,
        vec![],
        String::new(),
    );
    let recording = FailureRecordingUsecase::new(failures, Some(subscriptions.clone()));
    let target = SubscriptionTarget::SessionTab(item.id);
    let mut stream = Box::pin(subscriptions.open("client".into()).unwrap());
    stream.next().await;
    start_read(&subscriptions, "client", &target.to_string())
        .await
        .unwrap();
    let initial = reads.read(&target).await.unwrap();
    assert!(
        matches!(&initial, StateValue::SessionTab { node: Some(node), .. } if node.0.status_classification.as_public_str() == "idle")
    );
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if releashd::test_support::integration::subscriptions::same(&value, initial))
    );
    stream.next().await;

    // When / Then
    let key = FailureKey::new(
        "workflow_delegate_injection",
        &item.tree_location.node_execution_id,
    );
    for expected_classification in ["attention", "idle"] {
        if expected_classification == "attention" {
            recording.observed(
                &key,
                WorkFailure {
                    kind: Failure::Technical(TechnicalFailureNature::Other),
                    message: "delegate injection failed".into(),
                },
            );
        } else {
            recording.resolved(&key);
        }
        let expected = reads.read(&target).await.unwrap();
        assert!(
            matches!(&expected, StateValue::SessionTab { node: Some(node), .. } if node.0.status_classification.as_public_str() == expected_classification)
        );
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .expect("background attention change must reach the session tab");
        assert!(
            matches!(event, Some(StateSubscriptionEvent::Item(id, Event::Change(_, _, value))) if id == format!("client:{target}") && releashd::test_support::integration::subscriptions::same(&value, expected))
        );
    }
}

#[tokio::test]
pub async fn test_sessionタブ購読_子の背景失敗の発生と解消を親の同じ購読へ配信し一覧と一致する() {
    use releashd::test_support::integration::platform::{
        Failure, FailureKey, FailureRecordStore, FailureRecordingUsecase, TechnicalFailureNature,
        WorkFailure,
    };
    use releashd::test_support::integration::sessions::LocalAgentSessionQueryService;
    use releashd::test_support::integration::workflow::AgentSessionActivity;

    // Given
    let fixture = Fixture::new();
    let (_directory, store, parent, child) =
        crate::adaptor_gateway_workspace_tree_repository::delegate_fixture(
            &fixture.path,
            AgentSessionActivity::AwaitingInstruction,
        )
        .await;
    releashd::test_support::integration::workflow::append_single_fact(
        &store,
        &parent,
        &releashd::test_support::integration::workflow::NodeFact::ExecutionCompleted,
        9000,
    )
    .await
    .unwrap();
    let item = LocalAgentSessionQueryService::new(store.clone())
        .get("parent-session")
        .await
        .unwrap()
        .unwrap();
    *fixture.sessions.item.lock() = Some(item.clone());
    let failures = Arc::new(FailureRecordStore::default());
    let mut reads = fixture.reads.clone();
    reads.workflow = Arc::new(
        releashd::test_support::integration::platform::build_workflow_services_with_gateways(
            failures.clone(),
            Arc::new(PassthroughManagedWorktreeGateway),
            Arc::new(releashd::test_support::integration::workflow::NoopEditorSettingsGateway),
            Arc::new(releashd::test_support::integration::workflow::NoopEditorLauncherGateway),
            releashd::test_support::integration::workspace::ExecutionReads::new(store.clone()),
            None,
            None,
        )
        .0,
    );
    let subscriptions = fixture.subscriptions.with_reads(
        Arc::new(StateSubscriptionReads(reads.clone())),
        None,
        vec![],
        String::new(),
    );
    let recording = FailureRecordingUsecase::new(failures, Some(subscriptions.clone()));
    let target = SubscriptionTarget::SessionTab(item.id);
    let mut stream = Box::pin(subscriptions.open("client".into()).unwrap());
    stream.next().await;
    start_read(&subscriptions, "client", &target.to_string())
        .await
        .unwrap();
    let initial = reads.read(&target).await.unwrap();
    assert!(
        matches!(&initial, StateValue::SessionTab { node: Some(node), .. } if node.0.status_classification.as_public_str() == "idle")
    );
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if releashd::test_support::integration::subscriptions::same(&value, initial))
    );
    stream.next().await;

    // When / Then
    for failed_target in [
        &child.node_execution_id,
        &parent.node_execution_id,
        &parent.tree_id,
    ] {
        let key = FailureKey::new("workflow_delegate_injection", failed_target);
        for expected_classification in ["attention", "idle"] {
            if expected_classification == "attention" {
                recording.observed(
                    &key,
                    WorkFailure {
                        kind: Failure::Technical(TechnicalFailureNature::Other),
                        message: "delegate injection failed".into(),
                    },
                );
            } else {
                recording.resolved(&key);
            }
            let expected = reads.read(&target).await.unwrap();
            assert!(
                matches!(&expected, StateValue::SessionTab { node: Some(node), .. } if node.0.status_classification.as_public_str() == expected_classification)
            );
            let list = reads
                .workflow
                .retained_workspace_trees(&[reads
                    .workflow
                    .resolve_worktree_path(&fixture.path)
                    .unwrap()])
                .await
                .pop()
                .unwrap()
                .value
                .unwrap();
            let listed = list
                .nodes()
                .iter()
                .find(|node| node.node_execution_id.as_deref() == Some(&parent.node_execution_id))
                .unwrap();
            assert_eq!(
                listed.status_classification.as_public_str(),
                expected_classification
            );
            if let StateValue::SessionTab {
                node: Some(detail), ..
            } = &expected
            {
                assert_eq!(detail.0.error_reason, listed.failure_reason());
            }
            let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                .await
                .expect("background attention change must reach the session tab");
            assert!(
                matches!(event, Some(StateSubscriptionEvent::Item(id, Event::Change(_, _, value))) if id == format!("client:{target}") && releashd::test_support::integration::subscriptions::same(&value, expected))
            );
        }
    }
}

#[tokio::test]
pub async fn test_issue手動更新失敗_最後の一覧と失敗を購読へ届ける() {
    // Given

    let fixture = Fixture::new();
    let mut stream = Box::pin(fixture.subscriptions.open("client".into()).unwrap());
    stream.next().await;
    let target = SubscriptionTarget::Issues(fixture.path.clone()).to_string();
    start_read(&fixture.subscriptions, "client", &target)
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
    start_read(&fixture.subscriptions, "client", &target)
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
        }],
        Some("workflow".into()),
    )
    .unwrap();
    let state = WorkspaceState {
        panes: Some(panes),
        version: 1,
        tabs: Some(WorkspaceTabsState { editors: vec![] }),
        layout: Some(WorkspaceLayoutState {
            center_tab: "agent".into(),
            active_view: "git".into(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
        }),
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
    react.layout.as_mut().unwrap().left_nav_collapsed = true;
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
        StateValue::WorkspaceState(
            releashd::test_support::integration::platform::StreamVersion::new(2).unwrap(),
            Some(react),
            Vec::new(),
            None
        )
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
    use releashd::test_support::integration::platform::{
        save_workspace_state, PaneLayout, PaneTab, PaneTabKind, WorkspaceLayoutState,
        WorkspaceState, WorkspaceTabsState,
    };
    use releashd::test_support::integration::providers::ProviderKind;
    use releashd::test_support::integration::sessions::{
        AgentSessionTreeLocation, LocalAgentSessionRepository,
    };
    use releashd::test_support::integration::workspace::WorkspaceIdentity;
    // Given
    let fixture = Fixture::new();
    let archive = archive_fixture();
    let sessions = test_sessions(
        Arc::new(LocalAgentSessionRepository::new(archive.store.clone())),
        Arc::default(),
        Arc::default(),
        Arc::default(),
    );
    for id in ["ready", "broken", "broken-root", "broken-root-alias"] {
        sessions
            .create(
                id,
                WorkspaceIdentity::new(&fixture.path),
                &fixture.path,
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root(id).unwrap(),
                &format!("create-{id}"),
            )
            .await
            .unwrap();
    }
    use releashd::test_support::integration::workspace::{
        SqliteWorkspaceTreeRepository, WorkspaceTreeRepository,
    };
    let tree = SqliteWorkspaceTreeRepository::new(
        archive.store.clone(),
        std::sync::Arc::new(FailureRecordStore::default()),
    )
    .load_trees(&[WorkspaceIdentity::new(&fixture.path)])
    .await
    .remove(0)
    .unwrap();
    let ready_node = tree
        .nodes()
        .iter()
        .find(|node| node.session_id.as_deref() == Some("ready"))
        .unwrap()
        .id
        .clone();
    let broken_root_alias = tree
        .nodes()
        .iter()
        .find(|node| node.session_id.as_deref() == Some("broken-root-alias"))
        .unwrap()
        .id
        .clone();
    let broken_node = tree
        .nodes()
        .iter()
        .find(|node| node.session_id.as_deref() == Some("broken"))
        .unwrap()
        .id
        .clone();
    releashd::test_support::integration::workflow::seed_workflow_session_facts(
        &archive.store,
        releashd::test_support::integration::workflow::WorkflowSessionFactSeed {
            workflow_name: "non-session-root",
            request: "test",
            worktree_path: &fixture.path,
            provider: ProviderKind::Codex,
            workflow_execution_id: "00000000-0000-4000-8000-000000002063",
            node_execution_id: "workflow-session",
            session_id: "other-agent",
            initial_instruction_admitted: false,
        },
    )
    .await
    .unwrap();
    sessions
        .create(
            "unrelated",
            WorkspaceIdentity::new("/other"),
            "/other",
            ProviderKind::Codex,
            AgentSessionTreeLocation::session_tree_root("unrelated").unwrap(),
            "create-unrelated",
        )
        .await
        .unwrap();
    use releashd::test_support::integration::workflow::{
        append_single_fact, ExecutionTreeLaunch, NodeDefinition, NodeFact, NodeFactMeta,
        NodeKindName, SessionAttachedFact, StartedFact, TreeRootFact, WorkflowDefinitionContent,
    };
    let child_tree = SqliteWorkspaceTreeRepository::new(
        archive.store.clone(),
        std::sync::Arc::new(FailureRecordStore::default()),
    )
    .load_trees(&[WorkspaceIdentity::new(&fixture.path)])
    .await
    .remove(0)
    .unwrap();
    let broken_child_alias = child_tree
        .nodes()
        .iter()
        .find(|node| node.node_execution_id.as_deref() == Some("workflow-session"))
        .unwrap()
        .id
        .clone();
    let public = "00000000-0000-4000-8000-000000002090";
    let first = NodeFactMeta {
        tree_id: public.into(),
        node_execution_id: "00000000-0000-4000-8000-000000002091".into(),
        node_name: "main".into(),
        kind: NodeKindName::Session,
        parent_id: None,
        attempt: 1,
    };
    let next = NodeFactMeta {
        node_execution_id: "00000000-0000-4000-8000-000000002092".into(),
        attempt: 2,
        ..first.clone()
    };
    let attach = |id: &str| {
        NodeFact::SessionAttached(SessionAttachedFact {
            session_id: id.into(),
            provider_session_id: None,
            transcript_ref: None,
            initial_instruction_admitted: false,
        })
    };
    for (meta, fact) in [
        (
            &first,
            NodeFact::Started(StartedFact {
                root: Some(Box::new(TreeRootFact {
                    repository_root: None,
                    workspace_identity: fixture.path.clone(),
                    worktree_path: fixture.path.clone(),
                    request: String::new(),
                    workflow_name: "main".into(),
                    definition: Some(WorkflowDefinitionContent {
                        entry: "main".into(),
                        nodes: vec![NodeDefinition {
                            name: "main".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    launched_as: ExecutionTreeLaunch::Workflow,
                })),
                parent: None,
                worktree: None,
            }),
        ),
        (&first, attach("first-agent")),
        (&first, NodeFact::RetryRequested),
        (
            &next,
            NodeFact::Started(StartedFact {
                root: None,
                parent: None,
                worktree: None,
            }),
        ),
        (&next, attach("next-agent")),
    ] {
        append_single_fact(&archive.store, meta, &fact, 1000)
            .await
            .unwrap();
    }
    let connection =
        rusqlite::Connection::open(archive.directory.path().join("local-event-store.sqlite3"))
            .unwrap();
    let original_root: String = connection.query_row(
        "SELECT detail FROM node_events WHERE node_execution_id = 'broken-root-alias' AND event_type = 'started'", [], |row| row.get(0),
    ).unwrap();
    connection.execute("UPDATE node_events SET detail = 'broken' WHERE node_execution_id = 'broken' AND event_type = 'session_attached'", []).unwrap();
    connection.execute("UPDATE node_events SET detail = 'broken' WHERE node_execution_id = 'broken-root' AND event_type = 'started'", []).unwrap();
    connection.execute("UPDATE node_events SET detail = 'broken' WHERE node_execution_id = 'unrelated' AND event_type = 'started'", []).unwrap();
    connection.execute("UPDATE node_events SET detail = '{}' WHERE node_execution_id = 'broken-root-alias' AND event_type = 'started'", []).unwrap();
    connection.execute("UPDATE node_events SET detail = 'broken' WHERE node_execution_id = 'workflow-session' AND event_type = 'started'", []).unwrap();
    let panes = PaneLayout::pane(
        "pane".into(),
        [
            broken_child_alias.as_str(),
            public,
            first.node_execution_id.as_str(),
            ready_node.as_str(),
            broken_node.as_str(),
            "missing",
            "broken-root",
            broken_root_alias.as_str(),
            "00000000-0000-4000-8000-000000002063",
        ]
        .into_iter()
        .map(|id| PaneTab {
            id: id.into(),
            kind: PaneTabKind::Session,
            node_id: Some(id.into()),
            agent_session_id: None,
        })
        .collect(),
        Some(ready_node.clone()),
    )
    .unwrap();
    let state = WorkspaceState {
        panes: Some(panes),
        version: 1,
        tabs: Some(WorkspaceTabsState {
            editors: Vec::new(),
        }),
        layout: Some(WorkspaceLayoutState {
            center_tab: "agent".into(),
            active_view: "git".into(),
            left_nav_collapsed: false,
            right_collapsed: false,
            right_bottom_collapsed: false,
        }),
    };
    let mut reads = fixture.reads.clone();
    reads.workflow = Arc::new(
        releashd::test_support::integration::platform::build_workflow_services_with_gateways(
            Arc::new(releashd::test_support::integration::platform::FailureRecordStore::default()),
            Arc::new(PassthroughManagedWorktreeGateway),
            Arc::new(releashd::test_support::integration::workflow::NoopEditorSettingsGateway),
            Arc::new(releashd::test_support::integration::workflow::NoopEditorLauncherGateway),
            releashd::test_support::integration::workspace::ExecutionReads::new(
                archive.store.clone(),
            ),
            None,
            None,
        )
        .0,
    );
    save_workspace_state(
        reads.workspace_state.as_ref(),
        None,
        &fixture.path,
        state.clone(),
    )
    .await
    .unwrap();
    // When
    let actual = reads
        .read(&T::WorkspaceState(
            fixture.path.clone(),
            fixture.path.clone(),
        ))
        .await
        .unwrap();
    // Then
    let StateValue::WorkspaceState(_, Some(actual), failures, None) = actual else {
        panic!("Missing workspace state")
    };
    let mut expected = state;
    expected.panes.as_mut().unwrap().resolve_legacy_sessions(
        &[
            (ready_node.clone(), "ready".into()),
            (public.into(), "next-agent".into()),
            (first.node_execution_id.clone(), "first-agent".into()),
        ]
        .into(),
    );
    assert_eq!(actual, expected);
    let query = releashd::test_support::integration::workspace::SqliteWorkspaceQueryService::new(
        archive.store.clone(),
    );
    use releashd::test_support::integration::workspace::WorkspaceQueryService;
    let mut individual_failures = Vec::new();
    for (id, fails) in [
        (broken_child_alias.as_str(), true),
        (broken_node.as_str(), true),
        ("broken-root", true),
        (broken_root_alias.as_str(), true),
        ("00000000-0000-4000-8000-000000002063", false),
        ("missing", false),
    ] {
        let resolution = query
            .resolve_session_nodes(&WorkspaceIdentity::new(&fixture.path), &[id.into()])
            .await;
        assert_eq!(
            resolution.failures.len(),
            usize::from(fails),
            "reference {id}"
        );
        if fails {
            assert_eq!(resolution.failures[0].reference_id, id);
        }
        individual_failures.extend(
            resolution
                .failures
                .into_iter()
                .map(|failure| failure.failure.message),
        );
    }
    individual_failures.sort();
    let mut messages: Vec<_> = failures
        .iter()
        .map(|failure| failure.failure.message.clone())
        .collect();
    messages.sort();
    assert_eq!(failures.len(), 4, "{messages:?}");
    assert_eq!(messages, individual_failures);
    // When: a structurally valid header with an empty workspace is also corrupt.
    connection.execute(
        "UPDATE node_events SET detail = json_set(?1, '$.root.worktreePath', '') WHERE node_execution_id = 'broken-root-alias' AND event_type = 'started'", [&original_root],
    ).unwrap();
    let query = releashd::test_support::integration::workspace::SqliteWorkspaceQueryService::new(
        archive.store.clone(),
    );
    let resolution = query
        .resolve_session_nodes(&WorkspaceIdentity::new(&fixture.path), &[broken_root_alias])
        .await;
    // Then
    assert!(resolution.sessions.is_empty());
    assert_eq!(resolution.failures.len(), 1);
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

#[tokio::test]
async fn test_状態読取_builtinと同名の利用者ファイルをbuiltinではない一覧として配信する() {
    use releashd::test_support::integration::{subscriptions::payload, wire, workflow};
    // Given
    let fixture = Fixture::new();
    let dir = &fixture.reads.workflows_dir;
    let name = workflow::list_builtin_workflows()[0].name.clone();
    std::fs::write(
        dir.join(format!("{name}.yml")),
        format!("name: {name}\ndescription: user workflow\nnodes:\n  main: {{command: 'true'}}\n"),
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("policies")).unwrap();
    std::fs::write(dir.join("policies/coding.md"), "# User coding").unwrap();
    // When
    let workflows = payload(
        &fixture
            .reads
            .read(&SubscriptionTarget::Workflows)
            .await
            .unwrap(),
    )
    .unwrap();
    let facets = payload(
        &fixture
            .reads
            .read(&SubscriptionTarget::Facets(FacetKind::Policy))
            .await
            .unwrap(),
    )
    .unwrap();
    // Then
    let Some(wire::state_payload::Value::Workflows(workflows)) = workflows.value else {
        panic!("workflows payload");
    };
    let row = workflows
        .items
        .iter()
        .find(|row| row.name.as_deref() == Some(&name))
        .unwrap();
    assert_eq!(row.builtin, Some(false));
    assert_eq!(row.description.as_deref(), Some("user workflow"));
    let Some(wire::state_payload::Value::Facets(facets)) = facets.value else {
        panic!("facets payload");
    };
    let row = facets
        .items
        .iter()
        .find(|row| row.key.as_deref() == Some("coding"))
        .unwrap();
    assert_eq!(row.builtin, Some(false));
    assert_eq!(row.description.as_deref(), Some("User coding"));
}

#[tokio::test]
async fn test_workspace状態購読_sessionタブ追加を設定未保存のまま配信する() {
    use releashd::test_support::integration::{
        platform::{save_workspace_state, WorkspaceState},
        subscriptions::payload,
        wire,
    };
    // Given
    let fixture = Fixture::new();
    let target = T::WorkspaceState(fixture.path.clone(), fixture.path.clone());
    let mut stream = Box::pin(fixture.subscriptions.open("client".into()).unwrap());
    stream.next().await.unwrap();
    start_read(&fixture.subscriptions, "client", &target.to_string())
        .await
        .unwrap();
    stream.next().await.unwrap();
    stream.next().await.unwrap();
    let mut expected = WorkspaceState::default();
    expected.add_session_tab("session", "node");
    // When
    save_workspace_state(
        fixture.reads.workspace_state.as_ref(),
        Some(&fixture.subscriptions),
        &fixture.path,
        expected.clone(),
    )
    .await
    .unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap();
    let (version, _) =
        releashd::test_support::integration::repository::WorkspaceStateRepository::read(
            fixture.reads.workspace_state.as_ref(),
            &fixture.path,
        )
        .await
        .unwrap();
    // Then
    assert!(
        matches!(event, StateSubscriptionEvent::Item(id, Event::Change(_, _, value))
        if id == format!("client:{target}") && releashd::test_support::integration::subscriptions::same(&value, StateValue::WorkspaceState(version, Some(expected.clone()), vec![], None)))
    );
    let value = fixture.reads.read(&target).await.unwrap();
    let Some(wire::state_payload::Value::WorkspaceState(value)) = payload(&value).unwrap().value
    else {
        panic!("workspace state payload")
    };
    let actual = value.value.unwrap();
    assert!(actual.tabs.is_none());
    assert!(actual.layout.is_none());
    assert_eq!(WorkspaceState::try_from(actual).unwrap(), expected);
}

#[tokio::test]
async fn test_実行木概要購読_全worktreeの失敗と解消をworkspace_list通知で配信する() {
    use releashd::test_support::integration::platform::{
        Failure, FailureKey, FailureRecordStore, FailureRecordingUsecase, TechnicalFailureNature,
        WorkFailure,
    };
    use releashd::test_support::integration::providers::ProviderKind;
    use releashd::test_support::integration::sessions::{
        AgentSessionTreeLocation, LocalAgentSessionRepository,
    };
    use releashd::test_support::integration::workflow::AgentSessionActivity;
    use releashd::test_support::integration::workspace::WorkspaceIdentity;

    // Given
    let fixture = Fixture::new();
    let archive = archive_fixture();
    let sessions = test_sessions(
        Arc::new(LocalAgentSessionRepository::new(archive.store.clone())),
        Arc::default(),
        Arc::default(),
        Arc::default(),
    );
    for (id, workspace) in [
        ("session-tab", fixture.path.as_str()),
        ("other-session", "/other-worktree"),
    ] {
        sessions
            .create(
                id,
                WorkspaceIdentity::new(workspace),
                workspace,
                ProviderKind::Codex,
                AgentSessionTreeLocation::session_tree_root(id).unwrap(),
                &format!("create-{id}"),
            )
            .await
            .unwrap();
        sessions
            .observe_activity(
                id,
                AgentSessionActivity::AwaitingInstruction,
                &format!("idle-{id}"),
            )
            .await
            .unwrap();
    }
    let failures = Arc::new(FailureRecordStore::default());
    let mut reads = fixture.reads.clone();
    reads.workflow = Arc::new(
        releashd::test_support::integration::platform::build_workflow_services_with_gateways(
            failures.clone(),
            Arc::new(PassthroughManagedWorktreeGateway),
            Arc::new(releashd::test_support::integration::workflow::NoopEditorSettingsGateway),
            Arc::new(releashd::test_support::integration::workflow::NoopEditorLauncherGateway),
            releashd::test_support::integration::workspace::ExecutionReads::new(
                archive.store.clone(),
            ),
            None,
            None,
        )
        .0,
    );
    let subscriptions = fixture.subscriptions.with_reads(
        Arc::new(StateSubscriptionReads(reads.clone())),
        None,
        vec![],
        String::new(),
    );
    let recording = FailureRecordingUsecase::new(failures, Some(subscriptions.clone()));
    let target = SubscriptionTarget::ExecutionOverview;
    let mut stream = Box::pin(subscriptions.open("client".into()).unwrap());
    stream.next().await;
    start_read(&subscriptions, "client", &target.to_string())
        .await
        .unwrap();
    let initial = reads.read(&target).await.unwrap();
    assert!(
        matches!(&initial, StateValue::ExecutionOverview((trees, failures)) if failures.is_empty() && trees.len() == 2 && trees.iter().all(|tree| tree.visible().roots()[0].node().status_classification.as_public_str() == "idle"))
    );
    assert!(
        matches!(stream.next().await, Some(StateSubscriptionEvent::Item(_, Event::Snapshot(_, value))) if releashd::test_support::integration::subscriptions::same(&value, initial))
    );
    stream.next().await;

    // When / Then
    let key = FailureKey::new("workflow_delegate_injection", "session-tab");
    for expected_classification in ["attention", "idle"] {
        if expected_classification == "attention" {
            recording.observed(
                &key,
                WorkFailure {
                    kind: Failure::Technical(TechnicalFailureNature::Other),
                    message: "delegate injection failed".into(),
                },
            );
        } else {
            recording.resolved(&key);
        }
        let expected = reads.read(&target).await.unwrap();
        assert!(
            matches!(&expected, StateValue::ExecutionOverview((trees, failures)) if failures.is_empty() && trees.len() == 2 && trees.iter().find(|tree| tree.nodes().iter().any(|node| node.session_id.as_deref() == Some("session-tab"))).unwrap().visible().roots()[0].node().status_classification.as_public_str() == expected_classification)
        );
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .expect("WorkspaceList notification must update the execution overview");
        assert!(
            matches!(event, Some(StateSubscriptionEvent::Item(id, Event::Change(_, _, value))) if id == format!("client:{target}") && releashd::test_support::integration::subscriptions::same(&value, expected))
        );
    }
}
