//! diff_tree / branch_diff / 相対パスの Tauri コマンド。

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::error::AppError;
use crate::adaptor::protocol::code::{DiffFileEntryInput, DiffTreeNodeInput};
use crate::usecase::code_dto::{DiffTreeNodeDto, FileNavigationResultDto};

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
