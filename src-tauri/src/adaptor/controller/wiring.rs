//! repository / usecase builder 群の composition root（DI 配線）。
//!
//! gateway 実装を repository / usecase へ合成する組み立ては controller の責務であり、
//! gateway 層や各エントリポイントへ漏らさない（依存方向の遵守）。AppState を持つ
//! Tauri コマンドだけでなく、MCP・watcher・workflow など非 AppState
//! エントリも、ここで構築した usecase を各 State へ注入する形で受け取る。
//!
//! repository / code / agent_session / workflow などの usecase builder を一元的に束ね、
//! query service や gateway 協力者は対応する usecase の構築時に注入する。

use std::sync::Arc;

use crate::adaptor::gateway::app_config::{read_config_if_exists, AppConfig, ReleashConfig};
use crate::adaptor::gateway::code::branch_base::BranchBaseResolverGateway;
use crate::adaptor::gateway::code::branch_diff::BranchDiffGateway;
use crate::adaptor::gateway::code::diff_compute::DiffComputerGateway;
use crate::adaptor::gateway::code::file_content::FileContentGateway;
use crate::adaptor::gateway::code::staging::StagingGateway;
use crate::adaptor::gateway::comment::{
    FileReviewEventStore, SystemReviewClock, UuidReviewIdGenerator,
};
use crate::adaptor::gateway::git_host::{GitHubGitHostGateway, InMemoryTtlCache, LatestPrStatuses};
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
#[cfg(any(test, feature = "test-support"))]
use crate::adaptor::gateway::local_event_store::LocalEventStoreConfig;
use crate::adaptor::gateway::repository::branch::BranchGateway;
use crate::adaptor::gateway::repository::git_config::GitConfigGateway;
use crate::adaptor::gateway::repository::status::StatusGateway;
use crate::adaptor::gateway::repository::util::RepoLocatorGateway;
use crate::adaptor::gateway::repository::worktree::WorktreeGateway;
use crate::adaptor::gateway::repository::worktree_terminal::NoopWorktreeTerminalGateway;
#[cfg(any(test, feature = "test-support"))]
use crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGateway;
#[cfg(any(test, feature = "test-support"))]
use crate::adaptor::gateway::workflow::{
    EmptySecretSourceGateway, NoopWorkflowExternalEditorGateway, PassthroughManagedWorktreeGateway,
};
use crate::adaptor::gateway::workflow::{
    ExecutionTreeArchiveFactRepository, RepoPathsManagedWorktreeGateway,
    RepositoryManagedWorktreeGateway, WorkflowDefinitionFileRepository,
    WorkflowDefinitionFileSourceGateway, WorkflowDiagnosticsFileGateway,
    WorkflowEventLogRepository, WorkflowExecutionProjectionLogRepository,
    WorkflowExternalEditorGateway, WorkflowFacetFileRepository, WorkflowRuntimeCommandGateway,
    WorkflowRuntimeCommandGatewayDeps, WorkflowSecretSourceConfigGateway,
};
use crate::domain::app_config::{ConfigRepository, ConfigSecretRepository};
use crate::domain::git_host::{CacheTtl, IssueInfo};
use crate::domain::repository::WorktreeTerminalGateway;
use crate::domain::workflow::{ManagedWorktreeGateway, SecretSourceGateway};
use crate::usecase::code_query_service::CodeQueryService;
use crate::usecase::code_usecase::CodeUsecase;
use crate::usecase::comment::{
    ReviewClock, ReviewCommentUsecase, ReviewEventStore, ReviewIdGenerator,
};
use crate::usecase::git_host::GitHostUsecase;
use crate::usecase::repository_usecase::RepositoryUsecase;
#[cfg(any(test, feature = "test-support"))]
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
use crate::usecase::workflow::ports::ExternalEditorGateway;
use crate::usecase::workflow::query_service::WorkflowQueryService;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use crate::usecase::workflow::{
    WorkflowReadUsecase, WorkflowRuntimeUsecase, WorkflowUsecase, WorkspaceNodeActionResolver,
    WorkspaceNodeCommandUsecase,
};
use crate::usecase::workspace_tree::WorkspaceQueryService;

/// git ベースの repository usecase を既定の gateway 実装で構築する。
/// terminal runtime を持たない composition（standalone read-only・テスト）向けに、
/// worktree terminal 停止は no-op とする。
#[cfg(any(test, feature = "test-support"))]
pub fn build_repository_usecase() -> RepositoryUsecase {
    build_repository_usecase_with_worktree_terminals(
        Arc::new(NoopWorktreeTerminalGateway),
        Default::default(),
    )
}

/// worktree 削除時に紐づく terminal surface を停止できる repository usecase を構築する
/// （Tauri アプリ本体の composition 用）。
pub(crate) fn build_repository_usecase_with_worktree_terminals(
    worktree_terminals: Arc<dyn WorktreeTerminalGateway>,
    operations: Arc<crate::usecase::worktree_operation::WorktreeOperations>,
) -> RepositoryUsecase {
    RepositoryUsecase::new(
        Arc::new(BranchGateway),
        Arc::new(StatusGateway),
        Arc::new(WorktreeGateway),
        Arc::new(GitConfigGateway),
        Arc::new(RepoLocatorGateway),
        worktree_terminals,
        operations,
    )
}

pub fn build_git_host_usecase() -> GitHostUsecase {
    let ttl = CacheTtl::EXTERNAL_INFORMATION;
    GitHostUsecase::new(
        Arc::new(GitHubGitHostGateway::default()),
        Arc::new(LatestPrStatuses::default()),
        Arc::new(InMemoryTtlCache::<Vec<IssueInfo>>::new(ttl)),
    )
}

/// code usecase を既定の gateway 実装で構築する。
/// staging（書き込み）は Command 側 Usecase が、ファイル内容参照・diff バッファ計算・
/// branch diff（読み取り）は `CodeQueryService` が各 gateway へ委譲する。
/// いずれの gateway もステートレスのため、起動時に 1 度だけ組み立てて Arc 共有する。
fn build_code_usecase_with_gateways() -> CodeUsecase {
    let query = CodeQueryService::new(
        Arc::new(FileContentGateway),
        Arc::new(DiffComputerGateway),
        Arc::new(BranchDiffGateway),
        Arc::new(BranchBaseResolverGateway::new(Arc::new(GitConfigGateway))),
    );
    CodeUsecase::new(Arc::new(StagingGateway), query)
}

pub fn build_code_usecase() -> CodeUsecase {
    build_code_usecase_with_gateways()
}

#[cfg(any(test, feature = "test-support"))]
pub fn build_terminal_surface_application_for_tests() -> TerminalSurfaceApplication {
    let hub =
        Arc::new(crate::adaptor::presenter::terminal_event_hub::TerminalSurfaceEventHub::new());
    TerminalSurfaceApplication::new(
        std::sync::Arc::new(crate::adaptor::gateway::telemetry::TelemetryGateway),
        Arc::new(TerminalSurfaceRuntimeGateway::default()),
        Arc::new(crate::adaptor::gateway::terminal_surface::event_source::TerminalSurfaceEventSourceGateway::new(hub.event_sender())),
        hub,
    )
}

pub(crate) fn build_canonical_agent_session_query(
    data_dir: impl Into<std::path::PathBuf>,
) -> Result<crate::adaptor::gateway::agent_session::LocalAgentSessionQueryService, String> {
    let data_dir = data_dir.into();
    let local_event_store = LocalEventReadStore::open(
        &data_dir,
        std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
    )?;
    Ok(
        crate::adaptor::gateway::agent_session::LocalAgentSessionQueryService::new_read_only(
            local_event_store,
        ),
    )
}

pub fn build_review_comment_usecase() -> ReviewCommentUsecase {
    let store: Arc<dyn ReviewEventStore> = Arc::new(FileReviewEventStore::default());
    let clock: Arc<dyn ReviewClock> = Arc::new(SystemReviewClock);
    let id_generator: Arc<dyn ReviewIdGenerator> = Arc::new(UuidReviewIdGenerator);
    ReviewCommentUsecase::new(store, clock, id_generator)
}

pub fn build_workspace_list_usecase(
    repositories: Arc<crate::usecase::repo_paths_usecase::RepoPathsUsecase>,
    repository: Arc<RepositoryUsecase>,
    repository_state: Arc<crate::usecase::repository_state::RepositoryStateService>,
    workflow: Arc<WorkflowUsecase>,
    git_host: Arc<GitHostUsecase>,
) -> crate::usecase::workspace_tree::WorkspaceListUsecase {
    crate::usecase::workspace_tree::WorkspaceListUsecase::new(
        repositories,
        repository,
        repository_state,
        workflow,
        git_host,
    )
}

pub(crate) fn build_workspace_node_command_usecase(
    resolver: Arc<dyn WorkspaceNodeActionResolver>,
    workflows: Arc<dyn crate::usecase::workflow::WorkspaceNodeWorkflowCommandExecutor>,
    session_renames: Arc<dyn crate::usecase::agent_session::AgentSessionRenameExecutor>,
) -> WorkspaceNodeCommandUsecase {
    WorkspaceNodeCommandUsecase::new(resolver, workflows, session_renames)
}

/// Test helper using the same mandatory canonical store wiring as production.
#[cfg(any(test, feature = "test-support"))]
pub fn build_workflow_usecase(
    data_dir: impl Into<std::path::PathBuf>,
    workflows_dir: Option<std::path::PathBuf>,
) -> WorkflowUsecase {
    build_workflow_usecase_and_store(data_dir, workflows_dir).0
}

/// Test composition hook that exposes the single writer owned by the workflow
/// services. Integration tests that exercise canonical runtime commits must
/// reuse this writer instead of opening a competing writer for the same DB.
#[cfg(any(test, feature = "test-support"))]
pub fn build_workflow_usecase_and_store(
    data_dir: impl Into<std::path::PathBuf>,
    workflows_dir: Option<std::path::PathBuf>,
) -> (WorkflowUsecase, Arc<LocalEventStore>) {
    let data_dir = data_dir.into();
    let local_event_store = LocalEventStore::open(LocalEventStoreConfig::production(
        data_dir.clone(),
        std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
    ))
    .expect("test workflow composition requires the canonical local event store");
    let workflow_usecase = build_workflow_services_with_gateways(
        Arc::new(crate::adaptor::gateway::failure_records::FailureRecordStore::default()),
        data_dir,
        Arc::new(PassthroughManagedWorktreeGateway),
        Arc::new(NoopWorkflowExternalEditorGateway),
        Arc::new(EmptySecretSourceGateway),
        local_event_store.clone(),
        None,
        workflows_dir,
    )
    .0;
    (workflow_usecase, local_event_store)
}

pub fn build_workflow_services_with_repository_worktrees(
    failures: Arc<crate::adaptor::gateway::failure_records::FailureRecordStore>,
    data_dir: impl Into<std::path::PathBuf>,
    repository_usecase: Arc<RepositoryUsecase>,
    app_config: Arc<dyn ConfigRepository>,
    config_secrets: Arc<dyn ConfigSecretRepository>,
    local_event_store: Arc<LocalEventStore>,
    processes: Arc<dyn crate::domain::workflow::NodeProcessReader>,
) -> (
    WorkflowUsecase,
    Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
) {
    let data_dir = data_dir.into();
    build_workflow_services_with_gateways(
        failures,
        data_dir,
        Arc::new(RepositoryManagedWorktreeGateway::new(
            repository_usecase,
            app_config.clone(),
        )),
        Arc::new(WorkflowExternalEditorGateway::new(app_config)),
        Arc::new(WorkflowSecretSourceConfigGateway::new(config_secrets)),
        local_event_store,
        Some(processes),
        None,
    )
}

pub fn build_workspace_worktree_path_usecase(
    data_dir: &std::path::Path,
) -> crate::usecase::workspace_tree::WorkspaceWorktreePathUsecase {
    crate::usecase::workspace_tree::WorkspaceWorktreePathUsecase::new(Arc::new(
        crate::adaptor::gateway::workflow::worktree_context::StoredWorkspaceWorktreePathQuery::new(
            data_dir.to_path_buf(),
            std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
        ),
    ))
}

pub fn build_canonical_workflow_read_usecase(
    data_dir: impl Into<std::path::PathBuf>,
    workflows_dir: Option<std::path::PathBuf>,
) -> Result<WorkflowReadUsecase, String> {
    let data_dir = data_dir.into();
    let local_event_store = LocalEventReadStore::open(
        &data_dir,
        std::sync::Arc::new(crate::common::retry::RetryLimiter::new()),
    )?;
    let repository_usecase = Arc::new(build_repository_usecase_with_worktree_terminals(Arc::new(
        NoopWorktreeTerminalGateway,
    ), Arc::new(crate::usecase::worktree_operation::WorktreeOperations::new(Arc::new(
        crate::adaptor::gateway::repository::worktree_operation::FileWorktreeOperationLocks::new(&data_dir),
    )))));
    let workflows_dir =
        workflows_dir.unwrap_or_else(WorkflowDefinitionFileRepository::default_workflows_dir);
    let config_path = data_dir.join("releash.toml");
    let config = read_config_if_exists(&config_path)?.unwrap_or_else(ReleashConfig::default);
    let mut repo_paths = config.app.last_repo_paths.clone();
    if !config.app.last_root_path.is_empty() && !repo_paths.contains(&config.app.last_root_path) {
        repo_paths.push(config.app.last_root_path.clone());
    }
    let worktrees: Arc<dyn ManagedWorktreeGateway> = Arc::new(
        RepoPathsManagedWorktreeGateway::new(repository_usecase, repo_paths),
    );
    let config_secrets: Arc<dyn ConfigSecretRepository> =
        Arc::new(AppConfig::new(config, config_path));
    let secrets: Arc<dyn SecretSourceGateway> =
        Arc::new(WorkflowSecretSourceConfigGateway::new(config_secrets));

    let definitions = Arc::new(WorkflowDefinitionFileRepository::new(
        workflows_dir.clone(),
        workflows_dir.clone(),
    ));
    let definition_sources = Arc::new(WorkflowDefinitionFileSourceGateway::new(
        workflows_dir.clone(),
        workflows_dir.clone(),
    ));
    let diagnostics = Arc::new(WorkflowDiagnosticsFileGateway::new(
        workflows_dir.clone(),
        workflows_dir.clone(),
    ));
    let facets = Arc::new(WorkflowFacetFileRepository::new(workflows_dir));
    let events = Arc::new(WorkflowEventLogRepository::with_read_store(
        local_event_store.clone(),
    ));
    let execution_projection = Arc::new(WorkflowExecutionProjectionLogRepository::new_read_only(
        local_event_store.clone(),
    ));
    let query = WorkflowQueryService::new(
        definitions,
        definition_sources,
        facets,
        events,
        execution_projection,
    );
    let workspace_query: Arc<dyn WorkspaceQueryService> =
        crate::adaptor::gateway::workspace_tree::SqliteWorkspaceQueryService::new_read_only(
            local_event_store,
        );
    Ok(WorkflowReadUsecase::new(
        query,
        worktrees,
        secrets,
        workspace_query,
        diagnostics,
    ))
}

/// gateway を呼び出し側から差し替えられる workflow composition。production 配線と
/// acceptance harness の双方がこの一箇所を通る。
#[allow(clippy::too_many_arguments)]
pub fn build_workflow_services_with_gateways(
    failures: Arc<crate::adaptor::gateway::failure_records::FailureRecordStore>,
    data_dir: impl Into<std::path::PathBuf>,
    worktrees: Arc<dyn ManagedWorktreeGateway>,
    editors: Arc<dyn ExternalEditorGateway>,
    secrets: Arc<dyn SecretSourceGateway>,
    store: Arc<LocalEventStore>,
    processes: Option<Arc<dyn crate::domain::workflow::NodeProcessReader>>,
    workflows_dir: Option<std::path::PathBuf>,
) -> (WorkflowUsecase, Arc<dyn WorkspaceQueryService>) {
    let data_dir = data_dir.into();
    let workflows_dir =
        workflows_dir.unwrap_or_else(WorkflowDefinitionFileRepository::default_workflows_dir);
    let facets_base_dir = workflows_dir.clone();
    let execution_archives = Arc::new(ExecutionTreeArchiveFactRepository::new(
        store.clone(),
        data_dir.clone(),
    ));
    let mut workspace_nodes =
        crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository::new(store.clone());
    Arc::get_mut(&mut workspace_nodes)
        .expect("new workspace repository")
        .processes = processes.clone();
    let workspace_query: Arc<dyn WorkspaceQueryService> =
        crate::adaptor::gateway::workspace_tree::SqliteWorkspaceQueryService::with_repository(
            workspace_nodes.clone(),
        );
    let definitions = Arc::new(WorkflowDefinitionFileRepository::new(
        workflows_dir.clone(),
        facets_base_dir.clone(),
    ));
    let definition_sources = Arc::new(WorkflowDefinitionFileSourceGateway::new(
        workflows_dir.clone(),
        facets_base_dir.clone(),
    ));
    let facets = Arc::new(WorkflowFacetFileRepository::new(facets_base_dir.clone()));
    let events = Arc::new(WorkflowEventLogRepository::with_store(store.clone()));
    let mut projection = WorkflowExecutionProjectionLogRepository::new(store.clone());
    projection.processes = processes;
    let execution_projection = Arc::new(projection);
    let diagnostics = Arc::new(WorkflowDiagnosticsFileGateway::new(
        workflows_dir.clone(),
        facets_base_dir,
    ));
    let query = WorkflowQueryService::new(
        definitions.clone(),
        definition_sources.clone(),
        facets.clone(),
        events,
        execution_projection,
    );
    let workflow_usecase = WorkflowUsecase::new(
        query.clone(),
        definitions,
        definition_sources,
        facets,
        worktrees.clone(),
        editors,
        diagnostics,
        secrets,
        execution_archives.clone(),
        workspace_nodes,
        workspace_query.clone(),
        failures,
    );
    (workflow_usecase, workspace_query)
}

pub fn build_workflow_runtime_usecase(
    retrying: Arc<crate::usecase::retry::Retrying>,
    app: crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeDependencies,
    deps: WorkflowRuntimeCommandGatewayDeps,
    daemon: Arc<crate::adaptor::gateway::daemon::InMemoryDaemonRepository>,
) -> Result<
    (
        WorkflowRuntimeUsecase,
        Option<Arc<crate::usecase::workflow::startup::WorkflowStartupUsecase>>,
    ),
    WorkflowRuntimeError,
> {
    use crate::adaptor::gateway::workflow::{
        runtime_resolver::{AppConfigManagedWorktreeResolver, DefaultWorkflowDefinitionResolver},
        workflow_host::WorkflowRuntimeHost,
    };
    let operations = deps.repository_usecase.worktree_operations();
    let mut driver = WorkflowRuntimeHost::new_canonical(
        retrying.clone(),
        Arc::new(DefaultWorkflowDefinitionResolver),
        Arc::new(AppConfigManagedWorktreeResolver::new(
            deps.repository_usecase,
            deps.app_config,
        )),
        deps.workspace_query,
        deps.agent_session_launch,
        deps.agent_session_initial_instruction,
        deps.agent_session_lifecycle,
        deps.provider_availability,
        deps.isolated_worktrees,
        daemon,
    );
    driver.node_processes = deps.node_processes;
    let driver = wire_delegate_continuation(app.clone(), driver);
    let archives = Arc::new(ExecutionTreeArchiveFactRepository::from_backend(
        crate::adaptor::gateway::workflow::fact_log::FactLogReadBackend::Live(
            app.store.clone().ok_or_else(|| {
                WorkflowRuntimeError::SessionStore("fact store unavailable".into())
            })?,
        ),
    ));
    let driver = Arc::new(driver);
    let startup = wire_workflow_startup(app.clone(), driver.clone());
    let publisher = app.state_changes.clone();
    Ok((
        WorkflowRuntimeUsecase::new_with_worktree_operations(
            retrying,
            Arc::new(WorkflowRuntimeCommandGateway::new_with_driver(app, driver)),
            archives,
            operations,
        )
        .with_state_publisher(publisher),
        startup,
    ))
}

pub fn wire_delegate_continuation(
    app: crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeDependencies,
    mut host: crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeHost,
) -> crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeHost {
    use crate::adaptor::gateway::workflow::workflow_host::delegate::HostDelegateContinuation;
    use crate::usecase::workflow::delegate::DelegateContinuationUsecase;
    host.delegate_continuation = Some(Arc::new(DelegateContinuationUsecase {
        retrying: host.queue.clone(),
        gateway: Arc::new(HostDelegateContinuation {
            host: host.clone(),
            app,
        }),
    }));
    host
}

pub fn wire_workflow_startup(
    app: crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeDependencies,
    host: Arc<crate::adaptor::gateway::workflow::workflow_host::WorkflowRuntimeHost>,
) -> Option<Arc<crate::usecase::workflow::startup::WorkflowStartupUsecase>> {
    use crate::adaptor::gateway::workflow::startup_repository::{
        HostWorkflowStartup, StoredWorkflowStartupRepository,
    };
    use crate::usecase::workflow::startup::WorkflowStartupUsecase;

    let store = app.store.clone()?;
    Some(Arc::new(WorkflowStartupUsecase::new(
        Arc::new(StoredWorkflowStartupRepository(store)),
        Arc::new(HostWorkflowStartup { host, app }),
    )))
}

/// Runs issue #1372 maintenance only after the fixed SQLite authority is
/// admitted. Inventory collection and sweeping are both blocking filesystem
/// work, while canonical runtime-protection is read asynchronously from the
/// already-open SQLite repository.
///
/// The GC inventory intentionally has no Session/Workflow file-store input.
/// Active Session and running Workflow protection comes from the canonical
/// the bounded `CanonicalRuntimeOwnerSnapshot`, so workspace-state/review
/// retention remains functional without violating issue #1499 B-070 or
/// composing independently snapshotted pages.
pub(crate) fn spawn_startup_app_data_gc(
    composition: crate::adaptor::controller::app_data_composition::ProductionAppDataComposition,
    shared_repo_paths: crate::adaptor::gateway::repository::repo_paths::SharedRepoPaths,
    repository: Arc<dyn crate::domain::local_event::LocalEventTransactionRepository>,
    execution_trees: Arc<dyn crate::usecase::app_data_gc::ExecutionTreeGc>,
) {
    tokio::spawn(async move {
        if let Err(error) = composition
            .run_startup_gc_pass(shared_repo_paths, repository, execution_trees)
            .await
        {
            log::error!("{error}");
        }
    });
}
