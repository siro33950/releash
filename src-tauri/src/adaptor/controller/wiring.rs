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
use crate::adaptor::gateway::code::review_blob_url::ReviewBlobUrlGateway;
use crate::adaptor::gateway::code::staging::StagingGateway;
use crate::adaptor::gateway::comment::{
    FileReviewEventStore, SystemReviewClock, UuidReviewIdGenerator,
};
use crate::adaptor::gateway::git_host::{GitHubGitHostGateway, InMemoryTtlCache};
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
#[cfg(test)]
use crate::adaptor::gateway::local_event_store::LocalEventStoreConfig;
use crate::adaptor::gateway::repository::branch::BranchGateway;
use crate::adaptor::gateway::repository::branch_card::BranchCardGateway;
use crate::adaptor::gateway::repository::git_config::GitConfigGateway;
use crate::adaptor::gateway::repository::log::LogGateway;
use crate::adaptor::gateway::repository::status::StatusGateway;
use crate::adaptor::gateway::repository::util::RepoLocatorGateway;
use crate::adaptor::gateway::repository::worktree::WorktreeGateway;
use crate::adaptor::gateway::repository::worktree_terminal::NoopWorktreeTerminalGateway;
#[cfg(test)]
use crate::adaptor::gateway::terminal_surface::runtime_gateway_impl::TerminalSurfaceRuntimeGateway;
#[cfg(test)]
use crate::adaptor::gateway::workflow::{
    EmptySecretSourceGateway, NoopWorkflowExternalEditorGateway, PassthroughManagedWorktreeGateway,
};
use crate::adaptor::gateway::workflow::{
    NodeEventIsolatedWorktreeLedgerRepository, RepoPathsManagedWorktreeGateway,
    RepositoryManagedWorktreeGateway, TauriWorkflowExternalEditorGateway,
    TauriWorkflowRuntimeCommandGateway, TauriWorkflowRuntimeCommandGatewayDeps,
    WorkflowConfigPathFileGateway, WorkflowDefinitionFileRepository,
    WorkflowDefinitionFileSourceGateway, WorkflowDiagnosticsFileGateway,
    WorkflowEventLogRepository, WorkflowExecutionArchiveFileRepository,
    WorkflowExecutionProjectionLogRepository, WorkflowFacetFileRepository,
    WorkflowSecretSourceConfigGateway,
};
use crate::domain::app_config::{ConfigRepository, ConfigSecretRepository};
use crate::domain::git_host::{CacheTtl, IssueInfo, PrStatus};
use crate::domain::repository::WorktreeTerminalGateway;
use crate::domain::workflow::{
    IsolatedWorktreeLedgerRepository, ManagedWorktreeGateway, SecretSourceGateway,
};
use crate::usecase::code_query_service::CodeQueryService;
use crate::usecase::code_usecase::CodeUsecase;
use crate::usecase::comment::{
    ReviewClock, ReviewCommentUsecase, ReviewEventStore, ReviewIdGenerator,
};
use crate::usecase::git_host::GitHostUsecase;
use crate::usecase::repository_query_service::{
    RepositoryQueryService, WorktreeClassificationQuery,
};
use crate::usecase::repository_usecase::RepositoryUsecase;
#[cfg(test)]
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
/// Entity の読み書きは Repository gateway へ、read model 生成は `WorktreeGateway` が実装する
/// `BranchCardQuery` を内包する `RepositoryQueryService` へ委譲する。
/// terminal runtime を持たない composition（standalone read-only・テスト）向けに、
/// worktree terminal 停止は no-op とする。
#[cfg(test)]
pub(crate) fn build_repository_usecase() -> RepositoryUsecase {
    build_repository_usecase_with_worktree_terminals(Arc::new(NoopWorktreeTerminalGateway))
}

/// worktree 削除時に紐づく terminal surface を停止できる repository usecase を構築する
/// （Tauri アプリ本体の composition 用）。
#[cfg(test)]
pub(crate) fn build_repository_usecase_with_worktree_terminals(
    worktree_terminals: Arc<dyn WorktreeTerminalGateway>,
) -> RepositoryUsecase {
    let query = RepositoryQueryService::new(
        Arc::new(BranchCardGateway),
        WorktreeClassificationQuery::empty(),
    );
    build_repository_usecase_inner(worktree_terminals, query)
}

pub(crate) fn build_repository_usecase_with_worktree_terminals_and_ledger(
    worktree_terminals: Arc<dyn WorktreeTerminalGateway>,
    worktree_ledger: Arc<dyn IsolatedWorktreeLedgerRepository>,
    workflow_executions: Arc<
        dyn crate::usecase::workflow::ports::WorkflowExecutionProjectionRepository,
    >,
) -> RepositoryUsecase {
    let query = RepositoryQueryService::new(
        Arc::new(BranchCardGateway),
        WorktreeClassificationQuery::new(worktree_ledger, workflow_executions),
    );
    build_repository_usecase_inner(worktree_terminals, query)
}

fn build_repository_usecase_inner(
    worktree_terminals: Arc<dyn WorktreeTerminalGateway>,
    query: RepositoryQueryService,
) -> RepositoryUsecase {
    RepositoryUsecase::new(
        Arc::new(BranchGateway),
        Arc::new(LogGateway),
        Arc::new(StatusGateway),
        Arc::new(WorktreeGateway),
        Arc::new(GitConfigGateway),
        Arc::new(RepoLocatorGateway),
        worktree_terminals,
        query,
    )
}

pub(crate) fn build_git_host_usecase() -> GitHostUsecase {
    let ttl = CacheTtl::from_secs(30);
    GitHostUsecase::new(
        Arc::new(GitHubGitHostGateway::default()),
        Arc::new(InMemoryTtlCache::<PrStatus>::new(ttl)),
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
    CodeUsecase::new(
        Arc::new(StagingGateway),
        query,
        Arc::new(ReviewBlobUrlGateway),
    )
}

#[cfg(test)]
pub(crate) fn build_code_usecase() -> CodeUsecase {
    build_code_usecase_with_gateways()
}

pub(crate) fn build_code_usecase_with_app<R: tauri::Runtime + 'static>(
    _app: tauri::AppHandle<R>,
) -> CodeUsecase {
    build_code_usecase_with_gateways()
}

#[cfg(test)]
pub(crate) fn build_terminal_surface_application_for_tests() -> TerminalSurfaceApplication {
    TerminalSurfaceApplication::new(
        Arc::new(TerminalSurfaceRuntimeGateway::default()),
        Arc::new(
            crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub::new(),
        ),
    )
}

pub(crate) fn build_canonical_agent_session_query(
    data_dir: impl Into<std::path::PathBuf>,
) -> Result<crate::adaptor::gateway::agent_session::LocalAgentSessionQueryService, String> {
    let data_dir = data_dir.into();
    let local_event_store = LocalEventReadStore::open(&data_dir)?;
    Ok(
        crate::adaptor::gateway::agent_session::LocalAgentSessionQueryService::new_read_only(
            local_event_store,
        ),
    )
}

pub(crate) fn build_review_comment_usecase() -> ReviewCommentUsecase {
    let store: Arc<dyn ReviewEventStore> = Arc::new(FileReviewEventStore::default());
    let clock: Arc<dyn ReviewClock> = Arc::new(SystemReviewClock);
    let id_generator: Arc<dyn ReviewIdGenerator> = Arc::new(UuidReviewIdGenerator);
    ReviewCommentUsecase::new(store, clock, id_generator)
}

pub(crate) fn build_workspace_node_command_usecase(
    resolver: Arc<dyn WorkspaceNodeActionResolver>,
    workflows: Arc<dyn crate::usecase::workflow::WorkspaceNodeWorkflowCommandExecutor>,
    session_renames: Arc<dyn crate::usecase::agent_session::AgentSessionRenameExecutor>,
) -> WorkspaceNodeCommandUsecase {
    WorkspaceNodeCommandUsecase::new(resolver, workflows, session_renames)
}

/// Test helper using the same mandatory canonical store wiring as production.
#[cfg(test)]
pub(crate) fn build_workflow_usecase(data_dir: impl Into<std::path::PathBuf>) -> WorkflowUsecase {
    build_workflow_usecase_and_store(data_dir).0
}

/// Test composition hook that exposes the single writer owned by the workflow
/// services. Integration tests that exercise canonical runtime commits must
/// reuse this writer instead of opening a competing writer for the same DB.
#[cfg(test)]
pub(crate) fn build_workflow_usecase_and_store(
    data_dir: impl Into<std::path::PathBuf>,
) -> (WorkflowUsecase, Arc<LocalEventStore>) {
    let data_dir = data_dir.into();
    let local_event_store =
        LocalEventStore::open(LocalEventStoreConfig::production(data_dir.clone()))
            .expect("test workflow composition requires the canonical local event store");
    let workflow_usecase = build_workflow_services_with_gateways(
        data_dir,
        Arc::new(PassthroughManagedWorktreeGateway),
        Arc::new(NoopWorkflowExternalEditorGateway),
        Arc::new(EmptySecretSourceGateway),
        local_event_store.clone(),
    )
    .0;
    (workflow_usecase, local_event_store)
}

pub(crate) fn build_workflow_services_with_repository_worktrees<R: tauri::Runtime + 'static>(
    data_dir: impl Into<std::path::PathBuf>,
    repository_usecase: Arc<RepositoryUsecase>,
    app_config: Arc<dyn ConfigRepository>,
    config_secrets: Arc<dyn ConfigSecretRepository>,
    app: tauri::AppHandle<R>,
    local_event_store: Arc<LocalEventStore>,
) -> (
    WorkflowUsecase,
    Arc<dyn crate::usecase::workspace_tree::WorkspaceQueryService>,
) {
    let data_dir = data_dir.into();
    build_workflow_services_with_gateways(
        data_dir,
        Arc::new(RepositoryManagedWorktreeGateway::new(
            repository_usecase,
            app_config.clone(),
        )),
        Arc::new(TauriWorkflowExternalEditorGateway::new(app, app_config)),
        Arc::new(WorkflowSecretSourceConfigGateway::new(config_secrets)),
        local_event_store,
    )
}

pub(crate) fn build_canonical_workflow_read_usecase(
    data_dir: impl Into<std::path::PathBuf>,
    workflows_dir: Option<std::path::PathBuf>,
) -> Result<WorkflowReadUsecase, String> {
    let data_dir = data_dir.into();
    let local_event_store = LocalEventReadStore::open(&data_dir)?;
    let worktree_ledger = Arc::new(NodeEventIsolatedWorktreeLedgerRepository::new_read_only(
        local_event_store.clone(),
    ));
    let workflow_executions = Arc::new(WorkflowExecutionProjectionLogRepository::new_read_only(
        local_event_store.clone(),
    ));
    let repository_usecase = Arc::new(build_repository_usecase_with_worktree_terminals_and_ledger(
        Arc::new(NoopWorktreeTerminalGateway),
        worktree_ledger,
        workflow_executions,
    ));
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
            Arc::new(WorkflowExecutionArchiveFileRepository::new(data_dir)),
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
pub(crate) fn build_workflow_services_with_gateways(
    data_dir: impl Into<std::path::PathBuf>,
    worktrees: Arc<dyn ManagedWorktreeGateway>,
    editors: Arc<dyn ExternalEditorGateway>,
    secrets: Arc<dyn SecretSourceGateway>,
    store: Arc<LocalEventStore>,
) -> (WorkflowUsecase, Arc<dyn WorkspaceQueryService>) {
    let data_dir = data_dir.into();
    let workflows_dir = WorkflowDefinitionFileRepository::default_workflows_dir();
    let facets_base_dir = workflows_dir.clone();
    let execution_archives = Arc::new(WorkflowExecutionArchiveFileRepository::new(
        data_dir.clone(),
    ));
    let workspace_nodes =
        crate::adaptor::gateway::workspace_tree::SqliteWorkspaceTreeRepository::new(store.clone());
    let workspace_query: Arc<dyn WorkspaceQueryService> =
        crate::adaptor::gateway::workspace_tree::SqliteWorkspaceQueryService::with_repository(
            workspace_nodes.clone(),
            execution_archives.clone(),
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
    let execution_projection =
        Arc::new(WorkflowExecutionProjectionLogRepository::new(store.clone()));
    let diagnostics = Arc::new(WorkflowDiagnosticsFileGateway::new(
        workflows_dir.clone(),
        facets_base_dir,
    ));
    let config_paths = Arc::new(WorkflowConfigPathFileGateway::new(workflows_dir));
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
        config_paths,
        secrets,
        execution_archives.clone(),
        workspace_nodes,
        workspace_query.clone(),
    );
    (workflow_usecase, workspace_query)
}

pub(crate) fn build_workflow_runtime_usecase(
    app: tauri::AppHandle,
    deps: TauriWorkflowRuntimeCommandGatewayDeps,
) -> Result<WorkflowRuntimeUsecase, WorkflowRuntimeError> {
    Ok(WorkflowRuntimeUsecase::new(Arc::new(
        TauriWorkflowRuntimeCommandGateway::new_with_default_driver(app, deps)?,
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
) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = composition
            .run_startup_gc_pass(shared_repo_paths, repository)
            .await
        {
            log::error!("{error}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_診断read_usecase_適用済みdirectoryを診断する() {
        // Given
        let data = tempfile::tempdir().unwrap();
        let workflows = tempfile::tempdir().unwrap();
        let _store =
            LocalEventStore::open(LocalEventStoreConfig::production(data.path().to_path_buf()))
                .unwrap();
        std::fs::write(workflows.path().join("configured.yml"), "name: [").unwrap();
        let read = build_canonical_workflow_read_usecase(
            data.path(),
            Some(workflows.path().to_path_buf()),
        )
        .unwrap();

        // When
        let report = read
            .diagnose_all(
                crate::usecase::workflow::ports::WorkflowDiagnosticsTarget::AppliedConfigDirectory,
            )
            .unwrap();

        // Then
        assert!(report["workflow_summaries"]["configured"].is_object());
    }

    async fn seed_b006_execution(store: &Arc<LocalEventStore>, workspace: &str) {
        use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus};

        let execution_id = "00000000-0000-4000-8000-000000001491";
        crate::adaptor::gateway::workflow::test_support::seed_canonical_execution(
            store,
            &crate::adaptor::gateway::workflow::execution_store::WorkflowExecutionMetadata {
                execution_id: execution_id.to_string(),
                workflow_name: "B006 workflow".to_string(),
                status: ExecutionStatus::Running,
                worktree_path: workspace.to_string(),
                current_node: None,
                created_from: ExecutionOrigin::DesktopUi,
                started_at: 1.0,
                updated_at: 2.0,
                completed_at: None,
                error_reason: None,
                interruption_reason: None,
                resume_from_node: None,
                total_token_usage: Default::default(),
            },
            &[],
        );
    }

    #[tokio::test]
    async fn b006_all_client_surfaces_use_the_production_workspace_query_contract() {
        // Given: the production composition root owns one live query object,
        // and the standalone loopback composition opens the same SQLite
        // authority through its read-only backend.
        let root = tempfile::tempdir().unwrap();
        let store = LocalEventStore::open(
            crate::adaptor::gateway::local_event_store::LocalEventStoreConfig::production(
                root.path().to_path_buf(),
            ),
        )
        .unwrap();
        let workspace = root
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        seed_b006_execution(&store, &workspace).await;
        let (workflow, query) = build_workflow_services_with_gateways(
            root.path(),
            Arc::new(PassthroughManagedWorktreeGateway),
            Arc::new(NoopWorkflowExternalEditorGateway),
            Arc::new(EmptySecretSourceGateway),
            store.clone(),
        );
        let standalone =
            build_canonical_workflow_read_usecase(root.path(), Some(root.path().join("workflows")))
                .unwrap();

        // When
        let page = crate::domain::workflow::WorkflowPageRequest::new(0, 10);
        let direct_executions = query
            .execution_summaries(None, None, Some(page))
            .unwrap()
            .into_iter()
            .map(crate::usecase::workflow::dto::workflow_execution_summary_to_dto)
            .collect::<Vec<_>>();
        let live_loopback_executions = workflow
            .read_usecase()
            .list_executions_filtered(None, None, page)
            .unwrap();
        let standalone_executions = standalone
            .list_executions_filtered(None, None, page)
            .unwrap();
        let workspace_identity = crate::domain::workspace_tree::WorkspaceIdentity::new(&workspace);
        let direct_tree = query.workspace_tree(&workspace_identity).unwrap();
        let tauri_tree = workflow.list_workspace_tree_nodes(&workspace).unwrap();
        // Then
        assert_eq!(direct_executions.len(), 1);
        assert!(!direct_tree.nodes.is_empty());
        assert_eq!(live_loopback_executions, direct_executions);
        assert_eq!(standalone_executions, direct_executions);
        assert_eq!(tauri_tree, direct_tree);
    }
}
