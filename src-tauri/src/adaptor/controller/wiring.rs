//! repository 責務の composition root（DI 配線）。
//!
//! gateway 実装を usecase へ合成する組み立ては controller の責務であり、gateway 層や
//! 各エントリポイントへ漏らさない（依存方向の遵守）。AppState を持つ Tauri コマンド
//! だけでなく、WebSocket ハンドラ・MCP・watcher・workflow など非 AppState エントリも、
//! ここで構築した `RepositoryUsecase` を各 State へ注入する形で受け取る。
//!
//! 読み取りも含めた唯一の入口は `RepositoryUsecase`。read model（DTO）生成の協力者である
//! `RepositoryQueryService` は Usecase 内部に閉じ込め、外部へ直接配らない。

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
use crate::usecase::code_query_service::CodeQueryService;
use crate::usecase::code_usecase::CodeUsecase;
use crate::usecase::repository_query_service::RepositoryQueryService;
use crate::usecase::repository_usecase::RepositoryUsecase;

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
