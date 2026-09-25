//! hunk / patch / range Tauri コマンド。

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::error::AppError;
use crate::adaptor::protocol::code::HunkInput;
use crate::usecase::code_dto::{HiddenRangeDto, VisibleBlockDto};

pub(crate) async fn compute_hidden_ranges_shared(
    state: &AppState,
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

pub(crate) async fn compute_hidden_ranges_from_content_shared(
    state: &AppState,
    original: String,
    modified: String,
    context_lines: u32,
) -> Result<Vec<HiddenRangeDto>, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || uc.compute_hidden_ranges_from_content(&original, &modified, context_lines))
        .await
}

pub(crate) async fn compute_visible_markdown_blocks_shared(
    state: &AppState,
    original: String,
    modified: String,
    context_lines: u32,
) -> Result<Vec<VisibleBlockDto>, AppError> {
    let uc = state.code_usecase.clone();
    run_blocking(move || uc.compute_visible_markdown_blocks(&original, &modified, context_lines))
        .await
}
