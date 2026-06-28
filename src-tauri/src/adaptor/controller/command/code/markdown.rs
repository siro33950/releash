//! Markdown diff read model Tauri コマンド。

use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::code::MarkdownDiffSideInput;
use crate::other::AppError;
use crate::usecase::code_dto::{DiffRangeDto, InlineChunkDto, SplitRowDto};

#[tauri::command]
pub async fn compute_markdown_diff_ranges(
    state: State<'_, AppState>,
    original: String,
    modified: String,
    side: MarkdownDiffSideInput,
) -> Result<Vec<DiffRangeDto>, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || {
        Ok(uc.compute_markdown_diff_ranges(&original, &modified, side.into_usecase()))
    })
    .await
}

#[tauri::command]
pub async fn compute_markdown_split_rows(
    state: State<'_, AppState>,
    original: String,
    modified: String,
) -> Result<Vec<SplitRowDto>, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || Ok(uc.compute_markdown_split_rows(&original, &modified))).await
}

#[tauri::command]
pub async fn compute_markdown_inline_chunks(
    state: State<'_, AppState>,
    original: String,
    modified: String,
) -> Result<Vec<InlineChunkDto>, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || Ok(uc.compute_markdown_inline_chunks(&original, &modified))).await
}
