//! repository / usecase builder 群の composition root（DI 配線）。
//!
//! gateway 実装を repository / usecase へ合成する組み立ては controller の責務であり、
//! gateway 層や各エントリポイントへ漏らさない（依存方向の遵守）。AppState を持つ
//! Tauri コマンドだけでなく、MCP・watcher・workflow など非 AppState
//! エントリも、ここで構築した usecase を各 State へ注入する形で受け取る。
//!
//! repository / code / agent_session / workflow などの usecase builder を一元的に束ね、
//! query service や gateway 協力者は対応する usecase の構築時に注入する。

use std::path::PathBuf;
use std::sync::Arc;

#[cfg(test)]
use crate::adaptor::gateway::agent_session::FileSessionStorage;
use crate::adaptor::gateway::agent_session::GitAgentPromptSuggestionGateway;
use crate::adaptor::gateway::app_config::{read_config_if_exists, AppConfig, ReleashConfig};
use crate::adaptor::gateway::code::branch_base::BranchBaseResolverGateway;
use crate::adaptor::gateway::code::branch_diff::BranchDiffGateway;
use crate::adaptor::gateway::code::diff_compute::DiffComputerGateway;
use crate::adaptor::gateway::code::file_content::FileContentGateway;
use crate::adaptor::gateway::code::mention::MentionGateway;
use crate::adaptor::gateway::code::review_blob_url::ReviewBlobUrlGateway;
use crate::adaptor::gateway::code::staging::StagingGateway;
use crate::adaptor::gateway::comment::{
    FileReviewEventStore, SystemReviewClock, UuidReviewIdGenerator,
};
use crate::adaptor::gateway::git_host::{GitHubGitHostGateway, InMemoryTtlCache};
#[cfg(not(test))]
use crate::adaptor::gateway::local_event_store::read_only::LocalEventReadStore;
use crate::adaptor::gateway::local_event_store::LocalEventStore;
#[cfg(test)]
use crate::adaptor::gateway::pty_session::backend_impl::PtySessionRuntimeGateway;
use crate::adaptor::gateway::repository::branch::BranchGateway;
use crate::adaptor::gateway::repository::branch_card::BranchCardGateway;
use crate::adaptor::gateway::repository::git_config::GitConfigGateway;
use crate::adaptor::gateway::repository::log::LogGateway;
use crate::adaptor::gateway::repository::status::StatusGateway;
use crate::adaptor::gateway::repository::util::RepoLocatorGateway;
use crate::adaptor::gateway::repository::worktree::WorktreeGateway;
use crate::adaptor::gateway::workflow::{
    DurableWorkspaceNodeSessionCloseGateway, RepoPathsManagedWorktreeGateway,
    RepositoryManagedWorktreeGateway, StoredWorkspaceSessionGateway,
    TauriNodeExecutionLifecycleGateway, TauriWorkflowExternalEditorGateway,
    TauriWorkflowRuntimeCommandGateway, TauriWorkflowRuntimeCommandGatewayDeps,
    WorkflowConfigPathFileGateway, WorkflowDefinitionFileRepository,
    WorkflowDefinitionFileSourceGateway, WorkflowDiagnosticsFileGateway,
    WorkflowEventLogRepository, WorkflowExecutionArchiveFileRepository,
    WorkflowExecutionFileRepository, WorkflowExecutionProjectionLogRepository,
    WorkflowFacetFileRepository, WorkflowSecretSourceConfigGateway,
};
#[cfg(test)]
use crate::adaptor::gateway::workflow::{
    EmptySecretSourceGateway, NoopWorkflowExternalEditorGateway, PassthroughManagedWorktreeGateway,
};
use crate::domain::app_config::{AgentConfigRepository, ConfigRepository, ConfigSecretRepository};
use crate::domain::git_host::{CacheTtl, IssueInfo, PrStatus};
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::workflow::{ManagedWorktreeGateway, SecretSourceGateway};
use crate::infrastructure::agent_session::{
    claude::ClaudeBackend as NewClaudeBackend, codex::CodexBackend as NewCodexBackend,
};
use crate::usecase::agent_session::operation::SessionLifecycleOperationUsecase;
use crate::usecase::agent_session::runtime::AgentSessionRuntimeUsecase;
use crate::usecase::agent_session::session::{
    AgentPromptSuggestionUsecase, OpenTabRegistry, SessionReaderPort, SessionStore,
    StoredSessionLifecycleUsecase, WorkflowNodeSessionRestorer,
};
use crate::usecase::code_query_service::CodeQueryService;
use crate::usecase::code_usecase::CodeUsecase;
use crate::usecase::comment::{
    ReviewClock, ReviewCommentUsecase, ReviewEventStore, ReviewIdGenerator,
};
use crate::usecase::git_host::GitHostUsecase;
#[cfg(test)]
use crate::usecase::pty_session::read_usecase::PtySessionReadUsecase;
use crate::usecase::repository_query_service::RepositoryQueryService;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::workflow::ports::ExternalEditorGateway;
use crate::usecase::workflow::query_service::WorkflowQueryService;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use crate::usecase::workflow::{
    NodeExecutionLifecycleUsecase, WorkflowReadUsecase, WorkflowRuntimeUsecase, WorkflowUsecase,
    WorkspaceNodeActionResolver, WorkspaceNodeCommandUsecase, WorkspaceSessionGateway,
    WorkspaceTreeQueryService,
};

pub(crate) fn build_agent_backend_registry(
    config: Arc<dyn AgentConfigRepository>,
) -> crate::usecase::agent_session::backend_registry::AgentBackendRegistry {
    let mut registry = crate::usecase::agent_session::backend_registry::AgentBackendRegistry::new();
    let claude_cli_path = config.cli_path_for("claude").ok().flatten();
    let codex_cli_path = config.cli_path_for("codex").ok().flatten();
    registry.register(Arc::new(NewClaudeBackend::new(claude_cli_path)));
    registry.register(Arc::new(NewCodexBackend::new(codex_cli_path)));
    match config.default_agent_backend() {
        Ok(default_id) => registry.set_default(default_id),
        Err(error) => log::warn!("failed to read default backend from config: {error}"),
    }
    registry
}

/// git ベースの repository usecase を既定の gateway 実装で構築する。
/// Entity の読み書きは Repository gateway へ、read model 生成は `WorktreeGateway` が実装する
/// `BranchCardQuery` を内包する `RepositoryQueryService` へ委譲する。
pub(crate) fn build_repository_usecase() -> RepositoryUsecase {
    let query = RepositoryQueryService::new(Arc::new(BranchCardGateway));
    RepositoryUsecase::new(
        Arc::new(BranchGateway),
        Arc::new(LogGateway),
        Arc::new(StatusGateway),
        Arc::new(WorktreeGateway),
        Arc::new(GitConfigGateway),
        Arc::new(RepoLocatorGateway),
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
/// branch diff・mention 候補列挙（読み取り）は `CodeQueryService` が各 gateway へ委譲する。
/// いずれの gateway もステートレスのため、起動時に 1 度だけ組み立てて Arc 共有する。
fn build_code_usecase_with_gateways() -> CodeUsecase {
    let query = CodeQueryService::new(
        Arc::new(FileContentGateway),
        Arc::new(DiffComputerGateway),
        Arc::new(BranchDiffGateway),
        Arc::new(MentionGateway),
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
pub(crate) fn build_pty_session_read_usecase_for_tests() -> PtySessionReadUsecase {
    PtySessionReadUsecase::new(Arc::new(PtySessionRuntimeGateway::default()))
}

#[cfg(test)]
pub(crate) fn build_session_store() -> SessionStore {
    SessionStore::new(Arc::new(FileSessionStorage::default()))
}

#[cfg(not(test))]
pub(crate) fn build_canonical_session_read_store(
    data_dir: impl Into<PathBuf>,
) -> Result<SessionStore, String> {
    let data_dir = data_dir.into();
    let local_event_store = LocalEventReadStore::open(&data_dir)?;
    let repository: Arc<dyn LocalEventTransactionRepository> = local_event_store.clone();
    Ok(SessionStore::new_canonical(
        repository,
        local_event_store.installation_id().to_string(),
        Arc::new(
            crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
        ),
    ))
}

#[derive(Clone)]
enum WorkflowReadComposition {
    #[cfg(test)]
    Legacy,
    Canonical {
        repository: Arc<dyn LocalEventTransactionRepository>,
        installation_id: String,
    },
}

pub(crate) fn build_agent_prompt_suggestion_usecase(
    session_reader: Arc<SessionReaderPort>,
) -> AgentPromptSuggestionUsecase {
    AgentPromptSuggestionUsecase::new(session_reader, Arc::new(GitAgentPromptSuggestionGateway))
}

pub(crate) fn build_review_comment_usecase() -> ReviewCommentUsecase {
    let store: Arc<dyn ReviewEventStore> = Arc::new(FileReviewEventStore::default());
    let clock: Arc<dyn ReviewClock> = Arc::new(SystemReviewClock);
    let id_generator: Arc<dyn ReviewIdGenerator> = Arc::new(UuidReviewIdGenerator);
    ReviewCommentUsecase::new(store, clock, id_generator)
}

pub(crate) fn build_stored_session_lifecycle_usecase(
    session_store: Arc<SessionStore>,
    _registry: Arc<crate::usecase::agent_session::backend_registry::AgentBackendRegistry>,
    _runtime: Arc<AgentSessionRuntimeUsecase>,
    workflow_node_restorer: Arc<NodeExecutionLifecycleUsecase>,
    notice_usecase: Arc<crate::usecase::agent_session::notice::AgentSessionNoticeUsecase>,
) -> StoredSessionLifecycleUsecase {
    let workflow_node_restorer = Arc::new(WorkflowNodeSessionRestorerAdapter {
        lifecycle: workflow_node_restorer,
    });
    StoredSessionLifecycleUsecase::new(session_store, workflow_node_restorer, notice_usecase)
}

struct WorkflowNodeSessionRestorerAdapter {
    lifecycle: Arc<NodeExecutionLifecycleUsecase>,
}

#[async_trait::async_trait]
impl WorkflowNodeSessionRestorer for WorkflowNodeSessionRestorerAdapter {
    async fn try_open_tab(&self, session_id: &str) -> Result<Option<String>, String> {
        self.lifecycle
            .try_open_tab(session_id)
            .await
            .map(|target| target.map(|target| target.worktree_path))
            .map_err(|error| {
                log::debug!(
                    "failed to restore workflow node session tab for {session_id}: {error}"
                );
                crate::adaptor::controller::command::workflow::session_errors::workflow_node_tab_operation_failed()
            })
    }
}

pub(crate) fn build_workspace_node_command_usecase(
    resolver: Arc<dyn WorkspaceNodeActionResolver>,
    lifecycle: Arc<SessionLifecycleOperationUsecase>,
    session_store: Arc<SessionStore>,
    data_dir: impl Into<PathBuf>,
) -> WorkspaceNodeCommandUsecase {
    WorkspaceNodeCommandUsecase::new(
        resolver,
        Arc::new(DurableWorkspaceNodeSessionCloseGateway::new(
            lifecycle,
            session_store,
            data_dir.into(),
        )),
    )
}

/// workflow usecase を既定の file gateway 実装で構築する。
/// 既存の workflow YAML / facet markdown / run metadata / event log 形式を保持しつつ、
/// controller の read-only 経路を `WorkflowUsecase` に寄せる。
#[cfg(test)]
pub(crate) fn build_workflow_usecase(data_dir: impl Into<std::path::PathBuf>) -> WorkflowUsecase {
    build_workflow_usecase_with_workspace_sessions(data_dir, Arc::new(EmptyWorkspaceSessionGateway))
}

#[cfg(test)]
pub(crate) fn build_workflow_usecase_with_workspace_sessions(
    data_dir: impl Into<std::path::PathBuf>,
    sessions: Arc<dyn WorkspaceSessionGateway>,
) -> WorkflowUsecase {
    build_workflow_usecase_with_gateways(
        data_dir,
        Arc::new(PassthroughManagedWorktreeGateway),
        Arc::new(NoopWorkflowExternalEditorGateway),
        Arc::new(EmptySecretSourceGateway),
        sessions,
    )
}

#[cfg(test)]
struct EmptyWorkspaceSessionGateway;

#[cfg(test)]
impl WorkspaceSessionGateway for EmptyWorkspaceSessionGateway {
    fn list_active_sessions(
        &self,
        _worktree_path: &str,
    ) -> Result<
        Vec<crate::usecase::workflow::WorkspaceSessionInput>,
        crate::domain::workflow::WorkflowError,
    > {
        Ok(Vec::new())
    }

    fn list_closed_sessions(
        &self,
        _worktree_path: &str,
    ) -> Result<
        Vec<crate::usecase::workflow::WorkspaceSessionInput>,
        crate::domain::workflow::WorkflowError,
    > {
        Ok(Vec::new())
    }
}

#[cfg(test)]
pub(crate) fn build_workflow_usecase_with_repository_worktrees<R: tauri::Runtime + 'static>(
    data_dir: impl Into<std::path::PathBuf>,
    repository_usecase: Arc<RepositoryUsecase>,
    app_config: Arc<dyn ConfigRepository>,
    config_secrets: Arc<dyn ConfigSecretRepository>,
    session_store: Arc<SessionStore>,
    app: tauri::AppHandle<R>,
) -> WorkflowUsecase {
    let data_dir = data_dir.into();
    let sessions = Arc::new(StoredWorkspaceSessionGateway::new(
        session_store,
        data_dir.clone(),
    ));
    build_workflow_services_with_gateways(
        data_dir,
        Arc::new(RepositoryManagedWorktreeGateway::new(
            repository_usecase,
            app_config.clone(),
        )),
        Arc::new(TauriWorkflowExternalEditorGateway::new(app, app_config)),
        Arc::new(WorkflowSecretSourceConfigGateway::new(config_secrets)),
        sessions,
        WorkflowReadComposition::Legacy,
    )
    .0
}

pub(crate) fn build_workflow_services_with_repository_worktrees<R: tauri::Runtime + 'static>(
    data_dir: impl Into<std::path::PathBuf>,
    repository_usecase: Arc<RepositoryUsecase>,
    app_config: Arc<dyn ConfigRepository>,
    config_secrets: Arc<dyn ConfigSecretRepository>,
    session_store: Arc<SessionStore>,
    app: tauri::AppHandle<R>,
    local_event_store: Arc<LocalEventStore>,
) -> (WorkflowUsecase, WorkspaceTreeQueryService) {
    let data_dir = data_dir.into();
    let sessions = Arc::new(StoredWorkspaceSessionGateway::new(
        session_store,
        data_dir.clone(),
    ));
    let repository: Arc<dyn LocalEventTransactionRepository> = local_event_store.clone();
    let installation_id = local_event_store.installation_id().to_string();
    build_workflow_services_with_gateways(
        data_dir,
        Arc::new(RepositoryManagedWorktreeGateway::new(
            repository_usecase,
            app_config.clone(),
        )),
        Arc::new(TauriWorkflowExternalEditorGateway::new(app, app_config)),
        Arc::new(WorkflowSecretSourceConfigGateway::new(config_secrets)),
        sessions,
        WorkflowReadComposition::Canonical {
            repository,
            installation_id,
        },
    )
}

pub(crate) fn build_canonical_workflow_read_usecase(
    data_dir: impl Into<std::path::PathBuf>,
    workflows_dir: Option<std::path::PathBuf>,
) -> Result<WorkflowReadUsecase, String> {
    let data_dir = data_dir.into();
    #[cfg(not(test))]
    let local_event_store = LocalEventReadStore::open(&data_dir)?;
    #[cfg(not(test))]
    let local_event_repository: Arc<dyn LocalEventTransactionRepository> =
        local_event_store.clone();
    #[cfg(not(test))]
    let installation_id = local_event_store.installation_id().to_string();
    let workflows_dir =
        workflows_dir.unwrap_or_else(WorkflowDefinitionFileRepository::default_workflows_dir);
    let config_path = data_dir.join("releash.toml");
    let config = read_config_if_exists(&config_path)?.unwrap_or_else(ReleashConfig::default);
    let mut repo_paths = config.app.last_repo_paths.clone();
    if !config.app.last_root_path.is_empty() && !repo_paths.contains(&config.app.last_root_path) {
        repo_paths.push(config.app.last_root_path.clone());
    }
    let worktrees: Arc<dyn ManagedWorktreeGateway> = Arc::new(
        RepoPathsManagedWorktreeGateway::new(Arc::new(build_repository_usecase()), repo_paths),
    );
    let config_secrets: Arc<dyn ConfigSecretRepository> =
        Arc::new(AppConfig::new(config, config_path));
    let secrets: Arc<dyn SecretSourceGateway> =
        Arc::new(WorkflowSecretSourceConfigGateway::new(config_secrets));

    #[cfg(not(test))]
    let executions = Arc::new(WorkflowExecutionFileRepository::with_authority(
        local_event_repository.clone(),
    ));
    #[cfg(test)]
    let executions = Arc::new(WorkflowExecutionFileRepository::new(data_dir.clone()));
    let definitions = Arc::new(WorkflowDefinitionFileRepository::new(
        workflows_dir.clone(),
        workflows_dir.clone(),
    ));
    let definition_sources = Arc::new(WorkflowDefinitionFileSourceGateway::new(
        workflows_dir.clone(),
        workflows_dir.clone(),
    ));
    let facets = Arc::new(WorkflowFacetFileRepository::new(workflows_dir));
    #[cfg(not(test))]
    let events = Arc::new(WorkflowEventLogRepository::with_authority(
        local_event_repository.clone(),
        installation_id.clone(),
    ));
    #[cfg(test)]
    let events = Arc::new(WorkflowEventLogRepository::new(data_dir.clone()));
    #[cfg(not(test))]
    let execution_projection = Arc::new(WorkflowExecutionProjectionLogRepository::with_authority(
        local_event_repository,
        installation_id,
    ));
    #[cfg(test)]
    let execution_projection = Arc::new(WorkflowExecutionProjectionLogRepository::new(data_dir));
    let query = WorkflowQueryService::new(
        executions,
        definitions,
        definition_sources,
        facets,
        events,
        execution_projection,
    );
    Ok(WorkflowReadUsecase::new(query, worktrees, secrets))
}

#[cfg(test)]
fn build_workflow_usecase_with_gateways(
    data_dir: impl Into<std::path::PathBuf>,
    worktrees: Arc<dyn ManagedWorktreeGateway>,
    editors: Arc<dyn ExternalEditorGateway>,
    secrets: Arc<dyn SecretSourceGateway>,
    sessions: Arc<dyn WorkspaceSessionGateway>,
) -> WorkflowUsecase {
    build_workflow_services_with_gateways(
        data_dir,
        worktrees,
        editors,
        secrets,
        sessions,
        WorkflowReadComposition::Legacy,
    )
    .0
}

fn build_workflow_services_with_gateways(
    data_dir: impl Into<std::path::PathBuf>,
    worktrees: Arc<dyn ManagedWorktreeGateway>,
    editors: Arc<dyn ExternalEditorGateway>,
    secrets: Arc<dyn SecretSourceGateway>,
    sessions: Arc<dyn WorkspaceSessionGateway>,
    read_composition: WorkflowReadComposition,
) -> (WorkflowUsecase, WorkspaceTreeQueryService) {
    let data_dir = data_dir.into();
    let workflows_dir = WorkflowDefinitionFileRepository::default_workflows_dir();
    let facets_base_dir = workflows_dir.clone();
    let executions = Arc::new(match &read_composition {
        #[cfg(test)]
        WorkflowReadComposition::Legacy => WorkflowExecutionFileRepository::new(data_dir.clone()),
        WorkflowReadComposition::Canonical { repository, .. } => {
            WorkflowExecutionFileRepository::with_authority(repository.clone())
        }
    });
    let execution_archives = Arc::new(WorkflowExecutionArchiveFileRepository::new(
        data_dir.clone(),
    ));
    let definitions = Arc::new(WorkflowDefinitionFileRepository::new(
        workflows_dir.clone(),
        facets_base_dir.clone(),
    ));
    let definition_sources = Arc::new(WorkflowDefinitionFileSourceGateway::new(
        workflows_dir.clone(),
        facets_base_dir.clone(),
    ));
    let facets = Arc::new(WorkflowFacetFileRepository::new(facets_base_dir.clone()));
    let events = Arc::new(match &read_composition {
        #[cfg(test)]
        WorkflowReadComposition::Legacy => WorkflowEventLogRepository::new(data_dir.clone()),
        WorkflowReadComposition::Canonical {
            repository,
            installation_id,
        } => {
            WorkflowEventLogRepository::with_authority(repository.clone(), installation_id.clone())
        }
    });
    let execution_projection = Arc::new(match read_composition {
        #[cfg(test)]
        WorkflowReadComposition::Legacy => WorkflowExecutionProjectionLogRepository::new(data_dir),
        WorkflowReadComposition::Canonical {
            repository,
            installation_id,
        } => WorkflowExecutionProjectionLogRepository::with_authority(repository, installation_id),
    });
    let diagnostics = Arc::new(WorkflowDiagnosticsFileGateway::new(
        workflows_dir.clone(),
        facets_base_dir,
    ));
    let config_paths = Arc::new(WorkflowConfigPathFileGateway::new(workflows_dir));
    let query = WorkflowQueryService::new(
        executions,
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
        sessions.clone(),
        execution_archives.clone(),
    );
    let workspace_tree_query_service =
        WorkspaceTreeQueryService::new(query, worktrees, sessions, execution_archives);
    (workflow_usecase, workspace_tree_query_service)
}

pub(crate) fn build_workflow_runtime_usecase(
    app: tauri::AppHandle,
    deps: TauriWorkflowRuntimeCommandGatewayDeps,
) -> Result<WorkflowRuntimeUsecase, WorkflowRuntimeError> {
    Ok(WorkflowRuntimeUsecase::new(Arc::new(
        TauriWorkflowRuntimeCommandGateway::new_with_default_driver(app, deps)?,
    )))
}

pub(crate) fn build_node_execution_lifecycle_usecase(
    app: tauri::AppHandle,
    session_store: Arc<SessionStore>,
    agent_runtime: Arc<AgentSessionRuntimeUsecase>,
    open_tabs: Arc<OpenTabRegistry>,
) -> NodeExecutionLifecycleUsecase {
    let gateway = Arc::new(TauriNodeExecutionLifecycleGateway::new(
        app,
        session_store,
        agent_runtime,
        open_tabs,
    ));
    NodeExecutionLifecycleUsecase::new(gateway)
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
