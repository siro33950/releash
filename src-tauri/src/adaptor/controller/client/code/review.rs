//! review read model の Tauri コマンド。

use super::run_blocking;
use crate::adaptor::controller::state::AppState;
use crate::adaptor::presenter::code::ReviewGroupActionInput;
use crate::adaptor::presenter::error::AppError;

pub(crate) async fn git_stage_review_group_shared(
    state: &AppState,
    input: ReviewGroupActionInput,
) -> Result<(), AppError> {
    let review = state.review_usecase.clone();
    run_blocking(move || {
        review.git_stage_review_group(
            &input.worktree_path,
            &input.path,
            &input.section,
            &input.base,
            &input.group_id,
        )
    })
    .await
}

pub(crate) async fn git_unstage_review_group_shared(
    state: &AppState,
    input: ReviewGroupActionInput,
) -> Result<(), AppError> {
    let review = state.review_usecase.clone();
    run_blocking(move || {
        review.git_unstage_review_group(
            &input.worktree_path,
            &input.path,
            &input.section,
            &input.base,
            &input.group_id,
        )
    })
    .await
}
