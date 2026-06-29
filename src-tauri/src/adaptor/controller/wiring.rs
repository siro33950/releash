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

use crate::adaptor::gateway::agent_session::{FileSessionStorage, GitAgentPromptSuggestionGateway};
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
#[cfg(test)]
use crate::adaptor::gateway::pty_session::backend_impl::PtySessionRuntimeGateway;
use crate::adaptor::gateway::repository::branch::BranchGateway;
use crate::adaptor::gateway::repository::branch_card::BranchCardGateway;
use crate::adaptor::gateway::repository::git_config::GitConfigGateway;
use crate::adaptor::gateway::repository::log::LogGateway;
use crate::adaptor::gateway::repository::status::StatusGateway;
use crate::adaptor::gateway::repository::util::RepoLocatorGateway;
use crate::adaptor::gateway::repository::worktree::WorktreeGateway;
#[cfg(test)]
use crate::adaptor::gateway::workflow::{
    EmptySecretSourceGateway, NoopWorkflowExternalEditorGateway, PassthroughManagedWorktreeGateway,
};
use crate::adaptor::gateway::workflow::{
    RepositoryManagedWorktreeGateway, StoredWorkspaceSessionGateway,
    TauriWorkflowExternalEditorGateway, TauriWorkflowRuntimeCommandGateway,
    TauriWorkflowStepLifecycleGateway, WorkflowConfigPathFileGateway,
    WorkflowDefinitionFileRepository, WorkflowDiagnosticsFileGateway, WorkflowEventLogRepository,
    WorkflowFacetFileRepository, WorkflowRunArchiveFileRepository, WorkflowRunFileRepository,
    WorkflowSecretSourceConfigGateway, WorkflowStateProjectionLogRepository,
    WorkflowStepDetailProjectionLogRepository,
};
#[cfg(test)]
use crate::domain::agent_session::SkillEntry;
use crate::domain::app_config::{ConfigRepository, ConfigSecretRepository};
#[cfg(test)]
use crate::domain::code::CodeError;
use crate::domain::git_host::{CacheTtl, IssueInfo, PrStatus};
use crate::domain::workflow::{ManagedWorktreeGateway, SecretSourceGateway};
use crate::infrastructure::agent_session::codex_fuzzy_file_search_gateway::TauriCodexFuzzyFileSearchGateway;
use crate::infrastructure::agent_session::runtime::AgentProcessMap;
use crate::infrastructure::agent_session::skill_catalog_gateway::TauriCodexSkillCatalogGateway;
use crate::infrastructure::agent_session::thread_lifecycle_gateway::{
    CodexThreadLifecycleAppServerGateway, TauriAgentSessionRuntimeCloser,
};
use crate::usecase::agent_session::context::BranchDiffContextPort;
use crate::usecase::agent_session::session::{
    AgentPromptSuggestionUsecase, OpenTabRegistry, SessionReaderPort, SessionStore,
    StoredSessionLifecycleUsecase,
};
#[cfg(test)]
use crate::usecase::agent_session::skill_catalog::CodexSkillCatalogGateway;
use crate::usecase::agent_session::AgentSessionUsecase;
use crate::usecase::code_query_service::{CodeQueryService, CodexFuzzyFileSearchGateway};
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
use crate::usecase::workflow::{
    WorkflowRuntimeUsecase, WorkflowStepLifecycleUsecase, WorkflowUsecase, WorkspaceSessionGateway,
};
use tokio::sync::Mutex;

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
#[cfg(test)]
struct UnavailableCodexFuzzyFileSearchGateway;

#[cfg(test)]
#[async_trait::async_trait]
impl CodexFuzzyFileSearchGateway for UnavailableCodexFuzzyFileSearchGateway {
    async fn search_files(
        &self,
        _worktree_path: &str,
        _query: &str,
        _limit: usize,
    ) -> Result<Vec<String>, CodeError> {
        Err(CodeError::External(
            "Codex fuzzy file search gateway is not configured".to_string(),
        ))
    }
}

#[cfg(test)]
struct UnavailableCodexSkillCatalogGateway;

#[cfg(test)]
#[async_trait::async_trait]
impl CodexSkillCatalogGateway for UnavailableCodexSkillCatalogGateway {
    async fn list_app_server_skills(
        &self,
        _cwd: &str,
        _query: Option<&str>,
        _limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, String> {
        Err("Codex skill catalog gateway is not configured".to_string())
    }

    async fn scan_local_skills(
        &self,
        _cwd: &str,
        _backend_id: Option<&str>,
        _query: Option<&str>,
        _limit: Option<usize>,
    ) -> Result<Vec<SkillEntry>, String> {
        Err("Codex skill catalog gateway is not configured".to_string())
    }
}

fn build_code_usecase_with_fuzzy_gateway(
    codex_fuzzy_file_search: Arc<dyn CodexFuzzyFileSearchGateway>,
) -> CodeUsecase {
    let query = CodeQueryService::new(
        Arc::new(FileContentGateway),
        Arc::new(DiffComputerGateway),
        Arc::new(BranchDiffGateway),
        Arc::new(MentionGateway),
        Arc::new(BranchBaseResolverGateway::new(Arc::new(GitConfigGateway))),
        codex_fuzzy_file_search,
    );
    CodeUsecase::new(
        Arc::new(StagingGateway),
        query,
        Arc::new(ReviewBlobUrlGateway),
    )
}

#[cfg(test)]
pub(crate) fn build_code_usecase() -> CodeUsecase {
    build_code_usecase_with_fuzzy_gateway(Arc::new(UnavailableCodexFuzzyFileSearchGateway))
}

pub(crate) fn build_code_usecase_with_app<R: tauri::Runtime + 'static>(
    app: tauri::AppHandle<R>,
) -> CodeUsecase {
    build_code_usecase_with_fuzzy_gateway(Arc::new(TauriCodexFuzzyFileSearchGateway::new(app)))
}

pub(crate) fn build_agent_session_usecase<R: tauri::Runtime + 'static>(
    app: tauri::AppHandle<R>,
) -> AgentSessionUsecase {
    AgentSessionUsecase::new(Arc::new(TauriCodexSkillCatalogGateway::new(app)))
}

#[cfg(test)]
pub(crate) fn build_agent_session_usecase_for_tests() -> AgentSessionUsecase {
    AgentSessionUsecase::new(Arc::new(UnavailableCodexSkillCatalogGateway))
}

#[cfg(test)]
pub(crate) fn build_pty_session_read_usecase_for_tests() -> PtySessionReadUsecase {
    PtySessionReadUsecase::new(Arc::new(PtySessionRuntimeGateway::default()))
}

pub(crate) fn build_session_store() -> SessionStore {
    SessionStore::new(Arc::new(FileSessionStorage::default()))
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
    app: tauri::AppHandle,
    session_store: Arc<SessionStore>,
    handles: Arc<Mutex<AgentProcessMap>>,
) -> StoredSessionLifecycleUsecase {
    StoredSessionLifecycleUsecase::new(
        session_store,
        Arc::new(CodexThreadLifecycleAppServerGateway::new(app.clone())),
        Arc::new(TauriAgentSessionRuntimeCloser::new(app, handles)),
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
    build_workflow_usecase_with_gateways(
        data_dir,
        Arc::new(RepositoryManagedWorktreeGateway::new(
            repository_usecase,
            app_config.clone(),
        )),
        Arc::new(TauriWorkflowExternalEditorGateway::new(app, app_config)),
        Arc::new(WorkflowSecretSourceConfigGateway::new(config_secrets)),
        sessions,
    )
}

fn build_workflow_usecase_with_gateways(
    data_dir: impl Into<std::path::PathBuf>,
    worktrees: Arc<dyn ManagedWorktreeGateway>,
    editors: Arc<dyn ExternalEditorGateway>,
    secrets: Arc<dyn SecretSourceGateway>,
    sessions: Arc<dyn WorkspaceSessionGateway>,
) -> WorkflowUsecase {
    let data_dir = data_dir.into();
    let workflows_dir = WorkflowDefinitionFileRepository::default_workflows_dir();
    let facets_base_dir = workflows_dir.clone();
    let runs = Arc::new(WorkflowRunFileRepository::new(data_dir.clone()));
    let archive_runs = Arc::new(WorkflowRunArchiveFileRepository::new(data_dir.clone()));
    let definitions = Arc::new(WorkflowDefinitionFileRepository::new(
        workflows_dir.clone(),
        facets_base_dir.clone(),
    ));
    let facets = Arc::new(WorkflowFacetFileRepository::new(facets_base_dir.clone()));
    let events = Arc::new(WorkflowEventLogRepository::new(data_dir.clone()));
    let state_projection = Arc::new(WorkflowStateProjectionLogRepository::new(data_dir.clone()));
    let step_details = Arc::new(WorkflowStepDetailProjectionLogRepository::new(data_dir));
    let diagnostics = Arc::new(WorkflowDiagnosticsFileGateway::new(
        workflows_dir.clone(),
        facets_base_dir,
    ));
    let config_paths = Arc::new(WorkflowConfigPathFileGateway::new(workflows_dir));
    let query = WorkflowQueryService::new(
        runs,
        definitions.clone(),
        facets.clone(),
        events,
        state_projection,
        step_details,
    );
    WorkflowUsecase::new(
        query,
        definitions,
        facets,
        worktrees,
        editors,
        diagnostics,
        config_paths,
        secrets,
        sessions,
        archive_runs,
    )
}

pub(crate) fn build_workflow_runtime_usecase(
    app: tauri::AppHandle,
    repository_usecase: Arc<RepositoryUsecase>,
    app_config: Arc<dyn ConfigRepository>,
    session_store: Arc<SessionStore>,
    handles: Arc<Mutex<AgentProcessMap>>,
    branch_diff_context: Arc<dyn BranchDiffContextPort>,
    data_dir: Option<PathBuf>,
) -> WorkflowRuntimeUsecase {
    WorkflowRuntimeUsecase::new(Arc::new(
        TauriWorkflowRuntimeCommandGateway::new_with_default_engine(
            app,
            repository_usecase,
            app_config,
            session_store,
            handles,
            branch_diff_context,
            data_dir,
        ),
    ))
}

pub(crate) fn build_workflow_step_lifecycle_usecase(
    app: tauri::AppHandle,
    session_store: Arc<SessionStore>,
    handles: Arc<Mutex<AgentProcessMap>>,
    open_tabs: Arc<OpenTabRegistry>,
) -> WorkflowStepLifecycleUsecase {
    let gateway = Arc::new(TauriWorkflowStepLifecycleGateway::new(
        app,
        session_store,
        handles,
        open_tabs,
    ));
    WorkflowStepLifecycleUsecase::new(gateway.clone(), gateway)
}

pub(crate) fn spawn_workflow_pending_command_watcher(app: tauri::AppHandle, data_dir: PathBuf) {
    crate::adaptor::gateway::workflow::spawn_pending_command_watcher(app, data_dir);
}
