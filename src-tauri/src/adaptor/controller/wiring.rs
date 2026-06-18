//! repository 責務の composition root（DI 配線）。
//!
//! gateway 実装を usecase へ合成する組み立ては controller の責務であり、gateway 層や
//! 各エントリポイントへ漏らさない（依存方向の遵守）。AppState を持つ Tauri コマンド
//! だけでなく、WebSocket ハンドラ・MCP・watcher・workflow など非 AppState エントリも、
//! ここで構築した `RepositoryUsecase` を各 State へ注入する形で受け取る。
//!
//! 読み取りも含めた唯一の入口は `RepositoryUsecase`。read model（DTO）生成の協力者である
//! `RepositoryQueryService` は Usecase 内部に閉じ込め、外部へ直接配らない。

use std::path::PathBuf;
use std::sync::Arc;

use crate::adaptor::gateway::code::branch_base::BranchBaseResolverGateway;
use crate::adaptor::gateway::code::branch_diff::BranchDiffGateway;
use crate::adaptor::gateway::code::diff_compute::DiffComputerGateway;
use crate::adaptor::gateway::code::file_content::FileContentGateway;
use crate::adaptor::gateway::code::mention::MentionGateway;
use crate::adaptor::gateway::code::staging::StagingGateway;
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
    RepositoryManagedWorktreeGateway, TauriWorkflowExternalEditorGateway,
    TauriWorkflowRuntimeCommandGateway, WorkflowConfigPathFileGateway,
    WorkflowDefinitionFileRepository, WorkflowDiagnosticsFileGateway, WorkflowEventLogRepository,
    WorkflowFacetFileRepository, WorkflowRunFileRepository, WorkflowSecretSourceConfigGateway,
    WorkflowStateProjectionLogRepository, WorkflowStepDetailProjectionLogRepository,
};
use crate::config::AppConfig;
use crate::domain::workflow::{ManagedWorktreeGateway, SecretSourceGateway};
use crate::infrastructure::agent_session::runtime::AgentProcessMap;
use crate::usecase::agent_session::session::SessionStore;
use crate::usecase::code_query_service::CodeQueryService;
use crate::usecase::code_usecase::CodeUsecase;
use crate::usecase::repository_query_service::RepositoryQueryService;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::workflow::ports::ExternalEditorGateway;
use crate::usecase::workflow::query_service::WorkflowQueryService;
use crate::usecase::workflow::{WorkflowRuntimeUsecase, WorkflowUsecase};
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

/// code usecase を既定の gateway 実装で構築する。
/// staging（書き込み）は Command 側 Usecase が、ファイル内容参照・diff バッファ計算・
/// branch diff・mention 候補列挙（読み取り）は `CodeQueryService` が各 gateway へ委譲する。
/// いずれの gateway もステートレスのため、起動時に 1 度だけ組み立てて Arc 共有する。
pub(crate) fn build_code_usecase() -> CodeUsecase {
    let query = CodeQueryService::new(
        Arc::new(FileContentGateway),
        Arc::new(DiffComputerGateway),
        Arc::new(BranchDiffGateway),
        Arc::new(MentionGateway),
        Arc::new(BranchBaseResolverGateway::new(Arc::new(GitConfigGateway))),
    );
    CodeUsecase::new(Arc::new(StagingGateway), query)
}

/// workflow usecase を既定の file gateway 実装で構築する。
/// 既存の workflow YAML / facet markdown / run metadata / event log 形式を保持しつつ、
/// controller の read-only 経路を `WorkflowUsecase` に寄せる。
#[cfg(test)]
pub(crate) fn build_workflow_usecase(data_dir: impl Into<std::path::PathBuf>) -> WorkflowUsecase {
    build_workflow_usecase_with_gateways(
        data_dir,
        Arc::new(PassthroughManagedWorktreeGateway),
        Arc::new(NoopWorkflowExternalEditorGateway),
        Arc::new(EmptySecretSourceGateway),
    )
}

pub(crate) fn build_workflow_usecase_with_repository_worktrees<R: tauri::Runtime + 'static>(
    data_dir: impl Into<std::path::PathBuf>,
    repository_usecase: Arc<RepositoryUsecase>,
    app_config: Arc<AppConfig>,
    app: tauri::AppHandle<R>,
) -> WorkflowUsecase {
    build_workflow_usecase_with_gateways(
        data_dir,
        Arc::new(RepositoryManagedWorktreeGateway::new(
            repository_usecase,
            app_config.clone(),
        )),
        Arc::new(TauriWorkflowExternalEditorGateway::new(
            app,
            app_config.clone(),
        )),
        Arc::new(WorkflowSecretSourceConfigGateway::new(app_config)),
    )
}

fn build_workflow_usecase_with_gateways(
    data_dir: impl Into<std::path::PathBuf>,
    worktrees: Arc<dyn ManagedWorktreeGateway>,
    editors: Arc<dyn ExternalEditorGateway>,
    secrets: Arc<dyn SecretSourceGateway>,
) -> WorkflowUsecase {
    let data_dir = data_dir.into();
    let workflows_dir = WorkflowDefinitionFileRepository::default_workflows_dir();
    let facets_base_dir = workflows_dir.clone();
    let runs = Arc::new(WorkflowRunFileRepository::new(data_dir.clone()));
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
    )
}

pub(crate) fn build_workflow_runtime_usecase(
    app: tauri::AppHandle,
    repository_usecase: Arc<RepositoryUsecase>,
    app_config: Arc<AppConfig>,
    session_store: Arc<SessionStore>,
    handles: Arc<Mutex<AgentProcessMap>>,
    data_dir: Option<PathBuf>,
) -> WorkflowRuntimeUsecase {
    WorkflowRuntimeUsecase::new(Arc::new(
        TauriWorkflowRuntimeCommandGateway::new_with_default_engine(
            app,
            repository_usecase,
            app_config,
            session_store,
            handles,
            data_dir,
        ),
    ))
}

pub(crate) fn spawn_workflow_pending_command_watcher(app: tauri::AppHandle, data_dir: PathBuf) {
    crate::adaptor::gateway::workflow::spawn_pending_command_watcher(app, data_dir);
}
