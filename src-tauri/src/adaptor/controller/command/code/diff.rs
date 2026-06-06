//! diff_tree / branch_diff / 相対パスの Tauri コマンド。

use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::code::{DiffFileEntryInput, DiffTreeNodeInput};
use crate::other::AppError;
use crate::usecase::code_dto::{BranchDiffSummaryDto, DiffTreeNodeDto, FileNavigationResultDto};

#[tauri::command]
pub async fn build_diff_file_tree(
    state: State<'_, AppState>,
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

#[tauri::command]
pub async fn get_file_navigation(
    state: State<'_, AppState>,
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

#[tauri::command]
pub async fn get_branch_diff_summary(
    state: State<'_, AppState>,
    repo_path: String,
    base_branch: Option<String>,
) -> Result<BranchDiffSummaryDto, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || uc.get_branch_diff_summary(&repo_path, base_branch.as_deref())).await
}

#[tauri::command]
pub async fn get_relative_path(
    state: State<'_, AppState>,
    root_path: String,
    file_path: String,
) -> Result<Option<String>, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || Ok(uc.get_relative_path(&root_path, &file_path))).await
}
