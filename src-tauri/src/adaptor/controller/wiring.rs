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
use crate::adaptor::gateway::agent_session::{
    claude::ClaudeBackend as NewClaudeBackend, codex::CodexBackend as NewCodexBackend,
};
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
use crate::adaptor::gateway::workflow::{
    DurableWorkspaceNodeSessionCloseGateway, RepoPathsManagedWorktreeGateway,
    RepositoryManagedWorktreeGateway, TauriNodeExecutionLifecycleGateway,
    TauriWorkflowExternalEditorGateway, TauriWorkflowRuntimeCommandGateway,
    TauriWorkflowRuntimeCommandGatewayDeps, WorkflowConfigPathFileGateway,
    WorkflowDefinitionFileRepository, WorkflowDefinitionFileSourceGateway,
    WorkflowDiagnosticsFileGateway, WorkflowEventLogRepository,
    WorkflowExecutionArchiveFileRepository, WorkflowExecutionProjectionLogRepository,
    WorkflowFacetFileRepository, WorkflowSecretSourceConfigGateway,
};
#[cfg(test)]
use crate::adaptor::gateway::workflow::{
    EmptySecretSourceGateway, NoopWorkflowExternalEditorGateway, PassthroughManagedWorktreeGateway,
};
use crate::domain::app_config::{AgentConfigRepository, ConfigRepository, ConfigSecretRepository};
use crate::domain::git_host::{CacheTtl, IssueInfo, PrStatus};
use crate::domain::local_event::LocalEventTransactionRepository;
use crate::domain::repository::WorktreeTerminalGateway;
use crate::domain::workflow::{ManagedWorktreeGateway, SecretSourceGateway};
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
use crate::usecase::repository_query_service::RepositoryQueryService;
use crate::usecase::repository_usecase::RepositoryUsecase;
#[cfg(test)]
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
use crate::usecase::workflow::ports::ExternalEditorGateway;
use crate::usecase::workflow::query_service::WorkflowQueryService;
use crate::usecase::workflow::runtime_error::WorkflowRuntimeError;
use crate::usecase::workflow::{
    NodeExecutionLifecycleUsecase, WorkflowReadUsecase, WorkflowRuntimeUsecase, WorkflowUsecase,
    WorkspaceNodeActionResolver, WorkspaceNodeCommandUsecase,
};
use crate::usecase::workspace_tree::WorkspaceQueryService;

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
/// terminal runtime を持たない composition（standalone read-only・テスト）向けに、
/// worktree terminal 停止は no-op とする。
pub(crate) fn build_repository_usecase() -> RepositoryUsecase {
    build_repository_usecase_with_worktree_terminals(Arc::new(NoopWorktreeTerminalGateway))
}

/// worktree 削除時に紐づく terminal surface を停止できる repository usecase を構築する
/// （Tauri アプリ本体の composition 用）。
pub(crate) fn build_repository_usecase_with_worktree_terminals(
    worktree_terminals: Arc<dyn WorktreeTerminalGateway>,
) -> RepositoryUsecase {
    let query = RepositoryQueryService::new(Arc::new(BranchCardGateway));
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
pub(crate) fn build_terminal_surface_application_for_tests() -> TerminalSurfaceApplication {
    TerminalSurfaceApplication::new(
        Arc::new(TerminalSurfaceRuntimeGateway::default()),
        Arc::new(
            crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub::new(),
        ),
    )
}

#[cfg(test)]
pub(crate) fn build_session_store() -> SessionStore {
    SessionStore::new(Arc::new(FileSessionStorage::default()))
}

#[cfg(not(test))]
pub(crate) fn build_canonical_review_session_readers(
    data_dir: impl Into<PathBuf>,
) -> Result<
    (
        SessionStore,
        crate::adaptor::gateway::agent_session::LocalProviderAgentSessionQueryService,
    ),
    String,
> {
    let data_dir = data_dir.into();
    let local_event_store = LocalEventReadStore::open(&data_dir)?;
    let repository: Arc<dyn LocalEventTransactionRepository> = local_event_store.clone();
    Ok((
        SessionStore::new_canonical(
            repository.clone(),
            local_event_store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        ),
        crate::adaptor::gateway::agent_session::LocalProviderAgentSessionQueryService::new(
            repository,
        ),
    ))
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
    let installation_id = local_event_store.installation_id().to_string();
    let repository: Arc<dyn LocalEventTransactionRepository> = local_event_store.clone();
    let workflow_usecase = build_workflow_services_with_gateways(
        data_dir,
        Arc::new(PassthroughManagedWorktreeGateway),
        Arc::new(NoopWorkflowExternalEditorGateway),
        Arc::new(EmptySecretSourceGateway),
        repository,
        installation_id,
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
    let installation_id = local_event_store.installation_id().to_string();
    let local_event_repository: Arc<dyn LocalEventTransactionRepository> =
        local_event_store.clone();
    build_workflow_services_with_gateways(
        data_dir,
        Arc::new(RepositoryManagedWorktreeGateway::new(
            repository_usecase,
            app_config.clone(),
        )),
        Arc::new(TauriWorkflowExternalEditorGateway::new(app, app_config)),
        Arc::new(WorkflowSecretSourceConfigGateway::new(config_secrets)),
        local_event_repository,
        installation_id,
        local_event_store,
    )
}

pub(crate) fn build_canonical_workflow_read_usecase(
    data_dir: impl Into<std::path::PathBuf>,
    workflows_dir: Option<std::path::PathBuf>,
) -> Result<WorkflowReadUsecase, String> {
    let data_dir = data_dir.into();
    let local_event_store = LocalEventReadStore::open(&data_dir)?;
    let local_event_repository: Arc<dyn LocalEventTransactionRepository> =
        local_event_store.clone();
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

    let definitions = Arc::new(WorkflowDefinitionFileRepository::new(
        workflows_dir.clone(),
        workflows_dir.clone(),
    ));
    let definition_sources = Arc::new(WorkflowDefinitionFileSourceGateway::new(
        workflows_dir.clone(),
        workflows_dir.clone(),
    ));
    let facets = Arc::new(WorkflowFacetFileRepository::new(workflows_dir));
    let events = Arc::new(WorkflowEventLogRepository::with_authority(
        local_event_repository.clone(),
        installation_id.clone(),
    ));
    let execution_projection = Arc::new(WorkflowExecutionProjectionLogRepository::with_authority(
        local_event_repository,
        installation_id,
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
    ))
}

fn build_workflow_services_with_gateways(
    data_dir: impl Into<std::path::PathBuf>,
    worktrees: Arc<dyn ManagedWorktreeGateway>,
    editors: Arc<dyn ExternalEditorGateway>,
    secrets: Arc<dyn SecretSourceGateway>,
    repository: Arc<dyn LocalEventTransactionRepository>,
    installation_id: String,
    store: Arc<LocalEventStore>,
) -> (WorkflowUsecase, Arc<dyn WorkspaceQueryService>) {
    let data_dir = data_dir.into();
    let workflows_dir = WorkflowDefinitionFileRepository::default_workflows_dir();
    let facets_base_dir = workflows_dir.clone();
    let execution_archives = Arc::new(WorkflowExecutionArchiveFileRepository::new(
        data_dir.clone(),
    ));
    let workspace_query: Arc<dyn WorkspaceQueryService> =
        crate::adaptor::gateway::workspace_tree::SqliteWorkspaceQueryService::new(
            store,
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
    let events = Arc::new(WorkflowEventLogRepository::with_authority(
        repository.clone(),
        installation_id.clone(),
    ));
    let execution_projection = Arc::new(WorkflowExecutionProjectionLogRepository::with_authority(
        repository,
        installation_id,
    ));
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed_b006_execution(store: &Arc<LocalEventStore>, workspace: &str) {
        use crate::domain::local_event::{
            CommitIdentity, CommitOperationKind, IdempotencyBinding, LocalAtomicBatch,
            LocalStateMutation, Revision, RevisionGuard, WorkflowExecutionMetadataRecord,
            WorkflowExecutionNodeProjectionMutation, WorkflowExecutionProjectionMutation,
            WorkflowExecutionProjectionRecord,
        };
        use crate::domain::workflow::{ExecutionOrigin, ExecutionStatus, TokenUsage};
        use crate::domain::workspace_tree::{
            WorkspaceStructureFact, WorkspaceTree, WorkspaceTreeProjector,
        };

        let execution_id = "00000000-0000-4000-8000-000000001491";
        let record = WorkflowExecutionMetadataRecord {
            execution_id: execution_id.to_string(),
            workflow_name: "B006 workflow".to_string(),
            status: ExecutionStatus::Running,
            worktree_path: workspace.to_string(),
            current_node: None,
            created_from: ExecutionOrigin::DesktopUi,
            started_at_bits: 1.0f64.to_bits(),
            updated_at_bits: 2.0f64.to_bits(),
            completed_at_bits: None,
            error_reason: None,
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: TokenUsage::default(),
        };
        let mut tree = WorkspaceTree::empty(workspace);
        WorkspaceTreeProjector::project(
            &mut tree,
            [
                WorkspaceStructureFact::WorkflowStarted {
                    execution_id: execution_id.to_string(),
                    workflow_name: record.workflow_name.clone(),
                    worktree_path: workspace.to_string(),
                    definition: crate::domain::workflow::WorkflowDefinition::default(),
                    timestamp: 1.0,
                },
                WorkspaceStructureFact::WorkflowSummaryProjected {
                    execution_id: execution_id.to_string(),
                    workflow_name: record.workflow_name.clone(),
                    status: record.status,
                    updated_at: 2.0,
                },
            ],
        )
        .unwrap();
        store
            .commit_batch(LocalAtomicBatch {
                commit_id: CommitIdentity::parse("b006-production-seed").unwrap(),
                idempotency: IdempotencyBinding {
                    installation_id: store.installation_id().to_string(),
                    operation_kind: CommitOperationKind::Workflow,
                    idempotency_key: "b006-production-seed".to_string(),
                    payload_hash: [149; 32],
                },
                expected_heads: Vec::new(),
                events: Vec::new(),
                state_mutations: vec![
                    LocalStateMutation::WorkflowExecutionProjection(
                        WorkflowExecutionProjectionMutation {
                            projection: WorkflowExecutionProjectionRecord::Present(record),
                            expected: RevisionGuard::Absent,
                            revision: Revision::new(0).unwrap(),
                        },
                    ),
                    LocalStateMutation::WorkflowExecutionNodeProjection(
                        WorkflowExecutionNodeProjectionMutation {
                            execution_id: execution_id.to_string(),
                            nodes: tree.nodes().to_vec(),
                            expected: RevisionGuard::Absent,
                            revision: Revision::new(0).unwrap(),
                        },
                    ),
                ],
            })
            .await
            .unwrap();
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
        let repository: Arc<dyn LocalEventTransactionRepository> = store.clone();
        let (workflow, query) = build_workflow_services_with_gateways(
            root.path(),
            Arc::new(PassthroughManagedWorktreeGateway),
            Arc::new(NoopWorkflowExternalEditorGateway),
            Arc::new(EmptySecretSourceGateway),
            repository,
            store.installation_id().to_string(),
            store.clone(),
        );
        let session_store = Arc::new(crate::test_support::build_session_store());
        session_store.set_local_event_repository(
            store.clone(),
            store.installation_id().to_string(),
            Arc::new(
                crate::adaptor::gateway::agent_session::session_storage::AgentSessionProjectionCodecV1,
            ),
        );
        let active = crate::usecase::agent_session::session::create_session_internal(
            &session_store,
            root.path(),
            &workspace,
            Some("codex".to_string()),
        )
        .unwrap();
        let closed = crate::usecase::agent_session::session::create_session_internal(
            &session_store,
            root.path(),
            &workspace,
            Some("codex".to_string()),
        )
        .unwrap();
        session_store
            .set_session_state(
                root.path(),
                &closed.id,
                crate::usecase::agent_session::session::SessionState::Closed,
            )
            .unwrap();
        let runtime_dependencies = crate::test_support::agent_runtime_dependencies();
        let runtime = crate::compose_agent_session_runtime(
            session_store,
            runtime_dependencies.registry,
            runtime_dependencies.status_center,
            runtime_dependencies.status_notifier,
            runtime_dependencies.event_notifier,
            runtime_dependencies.spawner,
            None,
            runtime_dependencies.instruction_source,
            root.path().to_path_buf(),
            query.clone(),
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
        let direct_sessions = query
            .session_summaries(
                &workspace_identity,
                crate::domain::workspace_tree::WorkspaceSessionListKind::Active,
            )
            .unwrap();
        let runtime_sessions = runtime.list_sessions(&workspace).await.unwrap();
        let direct_closed = query
            .session_summaries(
                &workspace_identity,
                crate::domain::workspace_tree::WorkspaceSessionListKind::Closed,
            )
            .unwrap();
        let runtime_closed = runtime.list_closed_sessions(&workspace).await.unwrap();

        // Then
        assert_eq!(direct_executions.len(), 1);
        assert_eq!(direct_sessions.len(), 1);
        assert_eq!(direct_sessions[0].id, active.id);
        assert_eq!(direct_closed.len(), 1);
        assert_eq!(direct_closed[0].id, closed.id);
        assert!(!direct_tree.nodes.is_empty());
        assert_eq!(live_loopback_executions, direct_executions);
        assert_eq!(standalone_executions, direct_executions);
        assert_eq!(tauri_tree, direct_tree);
        assert_eq!(
            serde_json::to_value(runtime_sessions).unwrap(),
            serde_json::to_value(direct_sessions).unwrap()
        );
        assert_eq!(
            serde_json::to_value(runtime_closed).unwrap(),
            serde_json::to_value(direct_closed).unwrap()
        );
    }
}
