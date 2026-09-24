use std::path::PathBuf;
use std::sync::Arc;

use crate::domain::comment::{ReviewActor, ReviewTarget};
use crate::infrastructure::platform::path_aliases::{alias_name_for_profile, BuildProfile};
use crate::usecase::comment::{
    review_error_to_json_string, ReviewCommentUsecase, ReviewThreadDto, ReviewThreadFilterDto,
};

async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("task join error: {e}"))?
}

pub(crate) async fn list_review_threads_shared(
    data_dir: PathBuf,
    usecase: &Arc<ReviewCommentUsecase>,
    worktree_name: String,
    filter: Option<ReviewThreadFilterDto>,
) -> Result<Vec<ReviewThreadDto>, String> {
    let usecase = Arc::clone(usecase);
    blocking(move || {
        usecase
            .list_threads(
                &data_dir,
                &worktree_name,
                filter.map(Into::into),
                ReviewActor::human(),
            )
            .map(|threads| threads.into_iter().map(ReviewThreadDto::from).collect())
            .map_err(review_error_to_json_string)
    })
    .await
}

pub(crate) async fn create_review_thread_shared(
    data_dir: PathBuf,
    notify: &crate::adaptor::gateway::push::CommentChangeGateway,
    usecase: &Arc<ReviewCommentUsecase>,
    worktree_name: String,
    target: ReviewTarget,
    content: String,
) -> Result<ReviewThreadDto, String> {
    let usecase = Arc::clone(usecase);
    let worktree_name_for_event = worktree_name.clone();
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
            .map_err(review_error_to_json_string)
    })
    .await?;
    notify.notify(&worktree_name_for_event);
    Ok(thread)
}

pub(crate) async fn append_review_comment_shared(
    data_dir: PathBuf,
    notify: &crate::adaptor::gateway::push::CommentChangeGateway,
    usecase: &Arc<ReviewCommentUsecase>,
    worktree_name: String,
    thread_id: String,
    content: String,
) -> Result<ReviewThreadDto, String> {
    let usecase = Arc::clone(usecase);
    let worktree_name_for_event = worktree_name.clone();
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
            .map_err(review_error_to_json_string)
    })
    .await?;
    notify.notify(&worktree_name_for_event);
    Ok(thread)
}

pub(crate) async fn resolve_review_thread_shared(
    data_dir: PathBuf,
    notify: &crate::adaptor::gateway::push::CommentChangeGateway,
    usecase: &Arc<ReviewCommentUsecase>,
    worktree_name: String,
    thread_id: String,
    outcome: String,
    summary: String,
) -> Result<ReviewThreadDto, String> {
    let usecase = Arc::clone(usecase);
    let worktree_name_for_event = worktree_name.clone();
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
            .map_err(review_error_to_json_string)
    })
    .await?;
    notify.notify(&worktree_name_for_event);
    Ok(thread)
}

pub(crate) async fn delete_review_thread_shared(
    data_dir: PathBuf,
    notify: &crate::adaptor::gateway::push::CommentChangeGateway,
    usecase: &Arc<ReviewCommentUsecase>,
    worktree_name: String,
    thread_id: String,
) -> Result<(), String> {
    let usecase = Arc::clone(usecase);
    let worktree_name_for_event = worktree_name.clone();
    blocking(move || {
        usecase
            .delete_thread(&data_dir, &worktree_name, ReviewActor::human(), &thread_id)
            .map_err(review_error_to_json_string)
    })
    .await?;
    notify.notify(&worktree_name_for_event);
    Ok(())
}

pub(crate) async fn build_review_thread_handoff_shared(
    data_dir: PathBuf,
    usecase: &Arc<ReviewCommentUsecase>,
    worktree_name: String,
    thread_id: String,
) -> Result<String, String> {
    let usecase = Arc::clone(usecase);
    blocking(move || {
        let releash_alias = alias_name_for_profile(BuildProfile::current());
        usecase
            .build_handoff(&data_dir, &worktree_name, &thread_id, releash_alias)
            .map_err(review_error_to_json_string)
    })
    .await
}
