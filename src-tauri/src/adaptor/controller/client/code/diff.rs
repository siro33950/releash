//! diff_tree / branch_diff / 相対パスの Tauri コマンド。

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::code::{DiffFileEntryInput, DiffTreeNodeInput};
use crate::other::AppError;
use crate::usecase::code_dto::{BranchDiffSummaryDto, DiffTreeNodeDto, FileNavigationResultDto};
use crate::usecase::repository_state::snapshot::RepositoryHeadDiffFileTreeSnapshotDto;

pub(crate) async fn build_diff_file_tree_shared(
    state: &AppState,
    entries: Vec<DiffFileEntryInput>,
) -> Result<Vec<DiffTreeNodeDto>, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        let entries: Vec<_> = entries
            .into_iter()
            .map(DiffFileEntryInput::into_domain)
            .collect();
        Ok(uc.build_diff_file_tree(entries))
    })
    .await
}

pub(crate) async fn get_head_diff_file_tree_snapshot_shared(
    state: &AppState,
    repo_path: String,
) -> Result<RepositoryHeadDiffFileTreeSnapshotDto, AppError> {
    let service = state.repository_state.clone();
    super::super::repository::run_repository_state(move || {
        service.get_head_diff_file_tree_snapshot(&repo_path)
    })
    .await
}

pub(crate) async fn get_file_navigation_shared(
    state: &AppState,
    tree: Vec<DiffTreeNodeInput>,
    current_file: String,
) -> Result<FileNavigationResultDto, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        let tree: Vec<_> = tree
            .into_iter()
            .map(DiffTreeNodeInput::into_domain)
            .collect();
        Ok(uc.get_file_navigation(&tree, &current_file))
    })
    .await
}

pub(crate) async fn get_branch_diff_summary_shared(
    state: &AppState,
    repo_path: String,
    base_branch: Option<String>,
) -> Result<BranchDiffSummaryDto, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || uc.get_branch_diff_summary(&repo_path, base_branch.as_deref())).await
}

pub(crate) async fn get_relative_path_shared(
    state: &AppState,
    root_path: String,
    file_path: String,
) -> Result<Option<String>, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || Ok(uc.get_relative_path(&root_path, &file_path))).await
}
