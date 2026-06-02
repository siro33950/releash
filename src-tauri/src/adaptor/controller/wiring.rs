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

use crate::adaptor::gateway::repository::branch::BranchGateway;
use crate::adaptor::gateway::repository::branch_card::BranchCardGateway;
use crate::adaptor::gateway::repository::git_config::GitConfigGateway;
use crate::adaptor::gateway::repository::log::LogGateway;
use crate::adaptor::gateway::repository::status::StatusGateway;
use crate::adaptor::gateway::repository::util::RepoLocatorGateway;
use crate::adaptor::gateway::repository::worktree::WorktreeGateway;
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
