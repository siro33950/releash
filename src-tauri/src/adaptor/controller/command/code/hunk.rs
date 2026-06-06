//! hunk / patch / range Tauri コマンド。

use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::code::{ChangeGroupInput, HunkInput};
use crate::other::AppError;
use crate::usecase::code_dto::{DiffHunksResultDto, HiddenRangeDto, VisibleBlockDto};

#[tauri::command]
pub async fn compute_diff_hunks(
    state: State<'_, AppState>,
    original: String,
    modified: String,
    file_path: Option<String>,
) -> Result<DiffHunksResultDto, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || Ok(uc.compute_diff_hunks(&original, &modified, file_path.as_deref())))
        .await
}

#[tauri::command]
pub async fn generate_group_patch(
    state: State<'_, AppState>,
    file_path: String,
    hunk: HunkInput,
    group: ChangeGroupInput,
) -> Result<String, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        let hunk = hunk.into_domain();
        let group = group.into_domain();
        Ok(uc.generate_group_patch(&file_path, &hunk, &group))
    })
    .await
}

#[tauri::command]
pub async fn compute_hidden_ranges(
    state: State<'_, AppState>,
    hunks: Vec<HunkInput>,
    total_lines: u32,
    context_lines: u32,
) -> Result<Vec<HiddenRangeDto>, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        let hunks: Vec<_> = hunks.into_iter().map(HunkInput::into_domain).collect();
        Ok(uc.compute_hidden_ranges(&hunks, total_lines, context_lines))
    })
    .await
}

#[tauri::command]
pub async fn compute_hidden_ranges_from_content(
    state: State<'_, AppState>,
    original: String,
    modified: String,
    context_lines: u32,
) -> Result<Vec<HiddenRangeDto>, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        Ok(uc.compute_hidden_ranges_from_content(&original, &modified, context_lines))
    })
    .await
}

#[tauri::command]
pub async fn compute_visible_markdown_blocks(
    state: State<'_, AppState>,
    original: String,
    modified: String,
    context_lines: u32,
) -> Result<Vec<VisibleBlockDto>, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        Ok(uc.compute_visible_markdown_blocks(&original, &modified, context_lines))
    })
    .await
}
