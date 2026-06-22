//! AppState（DI 受け皿）。
//!
//! repository 責務のユースケース／クエリサービスを `Arc` で保持する。
//! `lib.rs` の起動時配線で組み立てて `manage` する。

use std::sync::Arc;

use crate::usecase::code_usecase::CodeUsecase;
use crate::usecase::repo_paths_usecase::RepoPathsUsecase;
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::usecase::workflow::WorkflowUsecase;

pub struct AppState {
    pub repository_usecase: Arc<RepositoryUsecase>,
    pub repo_paths_usecase: Arc<RepoPathsUsecase>,
    pub code_usecase: Arc<CodeUsecase>,
    pub workflow_usecase: Arc<WorkflowUsecase>,
}
