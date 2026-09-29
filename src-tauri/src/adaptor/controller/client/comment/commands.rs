use crate::adaptor::presenter::error::AppError;
use std::path::PathBuf;
use std::sync::Arc;

use crate::domain::comment::{ReviewActor, ReviewTarget};
use crate::infrastructure::platform::path_aliases::{alias_name_for_profile, BuildProfile};
use crate::usecase::comment::{ReviewCommentUsecase, ReviewThreadDto};

async fn blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    crate::adaptor::controller::client::worktree_mutation::spawn_blocking(f)
        .await
        .map_err(|e| AppError::new(format!("task join error: {e}")))?
}

pub(crate) async fn create_review_thread_shared(
    data_dir: PathBuf,
    usecase: &Arc<ReviewCommentUsecase>,
    worktree_name: String,
    target: ReviewTarget,
    content: String,
) -> Result<ReviewThreadDto, AppError> {
    let usecase = Arc::clone(usecase);
    let thread = blocking(move || {
        usecase
            .create_thread(
                &data_dir,
                &worktree_name,
                ReviewActor::human(),
                target,
                content,
            )
            .map(ReviewThreadDto::from)
            .map_err(AppError::from_failure)
    })
    .await?;
    Ok(thread)
}

pub(crate) async fn append_review_comment_shared(
    data_dir: PathBuf,
    usecase: &Arc<ReviewCommentUsecase>,
    worktree_name: String,
    thread_id: String,
    content: String,
) -> Result<ReviewThreadDto, AppError> {
    let usecase = Arc::clone(usecase);
    let thread = blocking(move || {
        usecase
            .append_comment(
                &data_dir,
                &worktree_name,
                ReviewActor::human(),
                &thread_id,
                content,
            )
            .map(ReviewThreadDto::from)
            .map_err(AppError::from_failure)
    })
    .await?;
    Ok(thread)
}

pub(crate) async fn resolve_review_thread_shared(
    data_dir: PathBuf,
    usecase: &Arc<ReviewCommentUsecase>,
    worktree_name: String,
    thread_id: String,
    outcome: String,
    summary: String,
) -> Result<ReviewThreadDto, AppError> {
    let usecase = Arc::clone(usecase);
    let thread = blocking(move || {
        usecase
            .resolve_thread(
                &data_dir,
                &worktree_name,
                ReviewActor::human(),
                &thread_id,
                outcome,
                summary,
            )
            .map(ReviewThreadDto::from)
            .map_err(AppError::from_failure)
    })
    .await?;
    Ok(thread)
}

pub(crate) async fn delete_review_thread_shared(
    data_dir: PathBuf,
    usecase: &Arc<ReviewCommentUsecase>,
    worktree_name: String,
    thread_id: String,
) -> Result<(), AppError> {
    let usecase = Arc::clone(usecase);
    blocking(move || {
        usecase
            .delete_thread(&data_dir, &worktree_name, ReviewActor::human(), &thread_id)
            .map_err(AppError::from_failure)
    })
    .await?;
    Ok(())
}

pub(crate) async fn build_review_thread_handoff_shared(
    data_dir: PathBuf,
    usecase: &Arc<ReviewCommentUsecase>,
    worktree_name: String,
    thread_id: String,
) -> Result<String, AppError> {
    let usecase = Arc::clone(usecase);
    blocking(move || {
        let releash_alias = alias_name_for_profile(BuildProfile::current());
        usecase
            .build_handoff(&data_dir, &worktree_name, &thread_id, releash_alias)
            .map_err(AppError::from_failure)
    })
    .await
}
