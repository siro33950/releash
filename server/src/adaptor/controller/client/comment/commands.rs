use crate::adaptor::presenter::error::AppError;
use std::path::PathBuf;
use std::sync::Arc;

use crate::domain::comment::{ReviewActor, ReviewTarget, ReviewThread};
use crate::infrastructure::platform::path_aliases::{alias_name_for_profile, BuildProfile};
use crate::usecase::comment::ReviewCommentUsecase;

async fn blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    crate::adaptor::controller::client::worktree_mutation::spawn_blocking(f)
        .await
        .map_err(|e| AppError::from_failure(crate::domain::failure::TechnicalFailure::from(e)))?
}

pub(crate) async fn create_review_thread_shared(
    data_dir: PathBuf,
    usecase: &Arc<ReviewCommentUsecase>,
    context: (String, ReviewActor),
    target: ReviewTarget,
    content: String,
) -> Result<ReviewThread, AppError> {
    let (worktree_name, actor) = context;
    let usecase = Arc::clone(usecase);
    let thread = blocking(move || {
        usecase
            .create_thread(&data_dir, &worktree_name, actor, target, content)
            .map_err(AppError::from_failure)
    })
    .await?;
    Ok(thread)
}

pub(crate) async fn append_review_comment_shared(
    data_dir: PathBuf,
    usecase: &Arc<ReviewCommentUsecase>,
    context: (String, ReviewActor),
    thread_id: String,
    content: String,
) -> Result<ReviewThread, AppError> {
    let (worktree_name, actor) = context;
    let usecase = Arc::clone(usecase);
    let thread = blocking(move || {
        usecase
            .append_comment(&data_dir, &worktree_name, actor, &thread_id, content)
            .map_err(AppError::from_failure)
    })
    .await?;
    Ok(thread)
}

pub(crate) async fn resolve_review_thread_shared(
    data_dir: PathBuf,
    usecase: &Arc<ReviewCommentUsecase>,
    context: (String, ReviewActor),
    thread_id: String,
    outcome: String,
    summary: String,
) -> Result<ReviewThread, AppError> {
    let (worktree_name, actor) = context;
    let usecase = Arc::clone(usecase);
    let thread = blocking(move || {
        usecase
            .resolve_thread(
                &data_dir,
                &worktree_name,
                actor,
                &thread_id,
                outcome,
                summary,
            )
            .map_err(AppError::from_failure)
    })
    .await?;
    Ok(thread)
}

pub(crate) async fn create_session_review_thread_shared(
    data_dir: PathBuf,
    usecase: &Arc<crate::usecase::comment::SessionReviewUsecase>,
    session_id: String,
    target: ReviewTarget,
    content: String,
) -> Result<crate::domain::comment::ReviewThread, AppError> {
    usecase
        .create_thread(data_dir, &session_id, target, content)
        .await
        .map_err(AppError::from_failure)
}
pub(crate) async fn append_session_review_comment_shared(
    data_dir: PathBuf,
    usecase: &Arc<crate::usecase::comment::SessionReviewUsecase>,
    session_id: String,
    thread_id: String,
    content: String,
) -> Result<crate::domain::comment::ReviewThread, AppError> {
    usecase
        .append_comment(data_dir, &session_id, thread_id, content)
        .await
        .map_err(AppError::from_failure)
}
pub(crate) async fn resolve_session_review_thread_shared(
    data_dir: PathBuf,
    usecase: &Arc<crate::usecase::comment::SessionReviewUsecase>,
    session_id: String,
    thread_id: String,
    outcome: String,
    summary: String,
) -> Result<crate::domain::comment::ReviewThread, AppError> {
    usecase
        .resolve_thread(data_dir, &session_id, thread_id, outcome, summary)
        .await
        .map_err(AppError::from_failure)
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
