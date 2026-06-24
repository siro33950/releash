//! review read model の Tauri コマンド。

use tauri::State;

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::protocol::code::{
    ReviewFileViewInput, ReviewGroupActionInput, ReviewSnapshotInput, ReviewTargetInput,
};
use crate::other::AppError;
use crate::usecase::code_dto::{ReviewFileViewDto, ReviewSnapshotDto};
use crate::usecase::review_usecase::{ReviewTarget, ReviewViewport};

#[tauri::command]
pub async fn get_review_snapshot(
    state: State<'_, AppState>,
    input: ReviewSnapshotInput,
) -> Result<ReviewSnapshotDto, AppError> {
    let review = state.review_usecase.clone();
    run_blocking(move || review.get_review_snapshot(&input.worktree_path, &input.base)).await
}

#[tauri::command]
pub async fn get_review_file_view(
    state: State<'_, AppState>,
    input: ReviewFileViewInput,
) -> Result<ReviewFileViewDto, AppError> {
    let review = state.review_usecase.clone();
    run_blocking(move || {
        review.get_review_file_view(
            &input.worktree_path,
            match input.target {
                ReviewTargetInput::FileId(value) => ReviewTarget::FileId(value),
                ReviewTargetInput::Path(value) => ReviewTarget::Path(value),
            },
            &input.section,
            &input.base,
            input.viewport.map(|viewport| ReviewViewport {
                start_line: viewport.start_line,
                end_line: viewport.end_line,
            }),
            input.snapshot_version,
        )
    })
    .await
}

#[tauri::command]
pub async fn git_stage_review_group(
    state: State<'_, AppState>,
    input: ReviewGroupActionInput,
) -> Result<(), AppError> {
    let review = state.review_usecase.clone();
    run_blocking(move || {
        review.git_stage_review_group(
            &input.worktree_path,
            &input.path,
            &input.section,
            &input.base,
            input.group_index,
            input.snapshot_version,
        )
    })
    .await
}

#[tauri::command]
pub async fn git_unstage_review_group(
    state: State<'_, AppState>,
    input: ReviewGroupActionInput,
) -> Result<(), AppError> {
    let review = state.review_usecase.clone();
    run_blocking(move || {
        review.git_unstage_review_group(
            &input.worktree_path,
            &input.path,
            &input.section,
            &input.base,
            input.group_index,
            input.snapshot_version,
        )
    })
    .await
}
