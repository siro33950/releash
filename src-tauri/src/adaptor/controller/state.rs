//! AppState（DI 受け皿）。
//!
//! repository 責務のユースケース／クエリサービスを `Arc` で保持する。
//! `lib.rs` の起動時配線で組み立てて `manage` する。

use std::sync::Arc;

use crate::usecase::agent_session::AgentSessionUsecase;
use crate::usecase::code_usecase::CodeUsecase;
use crate::usecase::git_host::GitHostUsecase;
use crate::usecase::notion::usecase::NotionUsecase;
use crate::usecase::pty_session::read_usecase::PtySessionReadUsecase;
use crate::usecase::repo_paths_usecase::RepoPathsUsecase;
use crate::usecase::repository_state::RepositoryStateService;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::review_usecase::ReviewUsecase;
use crate::usecase::workflow::WorkflowUsecase;

pub struct AppState {
    pub repository_usecase: Arc<RepositoryUsecase>,
    pub repository_state: Arc<RepositoryStateService>,
    pub repo_paths_usecase: Arc<RepoPathsUsecase>,
    pub code_usecase: Arc<CodeUsecase>,
    pub review_usecase: Arc<ReviewUsecase>,
    pub agent_session_usecase: Arc<AgentSessionUsecase>,
    pub notion_usecase: Arc<NotionUsecase>,
    pub workflow_usecase: Arc<WorkflowUsecase>,
    pub pty_session_read_usecase: Arc<PtySessionReadUsecase>,
    pub git_host_usecase: Arc<GitHostUsecase>,
}
