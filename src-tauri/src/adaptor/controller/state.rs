//! AppState（DI 受け皿）。
//!
//! repository 責務のユースケース／クエリサービスを `Arc` で保持する。
//! `lib.rs` の起動時配線で組み立てて `manage` する。

use std::sync::Arc;

use crate::usecase::code_usecase::CodeUsecase;
use crate::usecase::git_host::GitHostUsecase;
use crate::usecase::notion::usecase::NotionUsecase;
use crate::usecase::repo_paths_usecase::RepoPathsUsecase;
use crate::usecase::repository_state::RepositoryStateService;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::review_usecase::ReviewUsecase;
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;
use crate::usecase::workflow::WorkflowUsecase;

/// frontendがterminal streamをWebSocketで購読するための接続情報。
/// local API server起動時にmanageされる（起動失敗時は存在しない）。
/// tokenはterminal routeだけを認証するterminal専用tokenで、
/// masterのdiscovery tokenはrendererへ渡さない。
pub struct TerminalStreamEndpoint {
    pub port: u16,
    pub token: std::sync::Arc<str>,
}

pub struct AppState {
    pub repository_usecase: Arc<RepositoryUsecase>,
    pub repository_state: Arc<RepositoryStateService>,
    pub repo_paths_usecase: Arc<RepoPathsUsecase>,
    pub code_usecase: Arc<CodeUsecase>,
    pub review_usecase: Arc<ReviewUsecase>,
    pub notion_usecase: Arc<NotionUsecase>,
    pub workflow_usecase: Arc<WorkflowUsecase>,
    pub terminal_surface: Arc<TerminalSurfaceApplication>,
    pub git_host_usecase: Arc<GitHostUsecase>,
}
