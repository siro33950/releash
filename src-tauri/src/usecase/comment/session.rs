use super::{ReviewCommentUsecase, ReviewContextUsecase};
use crate::domain::comment::{
    ReviewError, ReviewHistoryEntry, ReviewTarget, ReviewThread, ReviewThreadFilter,
    ReviewWorktreeFilter,
};
use std::path::PathBuf;
use std::sync::Arc;

pub struct SessionReviewUsecase {
    context: ReviewContextUsecase,
    comments: Arc<ReviewCommentUsecase>,
}
impl SessionReviewUsecase {
    pub fn new(context: ReviewContextUsecase, comments: Arc<ReviewCommentUsecase>) -> Self {
        Self { context, comments }
    }
    pub async fn create_thread(
        &self,
        data_dir: PathBuf,
        id: &str,
        target: ReviewTarget,
        content: String,
    ) -> Result<ReviewThread, ReviewError> {
        let (path, actor) = self.context.session_for_write(id).await?;
        let comments = self.comments.clone();
        blocking(move || comments.create_thread(&data_dir, &path, actor, target, content)).await
    }
    pub async fn append_comment(
        &self,
        data_dir: PathBuf,
        id: &str,
        thread_id: String,
        content: String,
    ) -> Result<ReviewThread, ReviewError> {
        let (path, actor) = self.context.session_for_write(id).await?;
        let comments = self.comments.clone();
        blocking(move || comments.append_comment(&data_dir, &path, actor, &thread_id, content))
            .await
    }
    pub async fn resolve_thread(
        &self,
        data_dir: PathBuf,
        id: &str,
        thread_id: String,
        outcome: String,
        summary: String,
    ) -> Result<ReviewThread, ReviewError> {
        let (path, actor) = self.context.session_for_write(id).await?;
        let comments = self.comments.clone();
        blocking(move || {
            comments.resolve_thread(&data_dir, &path, actor, &thread_id, outcome, summary)
        })
        .await
    }
    pub async fn list_session_threads(
        &self,
        data_dir: PathBuf,
        id: &str,
        filter: ReviewThreadFilter,
    ) -> Result<Option<Vec<ReviewThread>>, ReviewError> {
        let context = self.context.session_for_read(id).await?;
        let comments = self.comments.clone();
        blocking(move || {
            context
                .map(|(path, actor)| comments.list_threads(&data_dir, &path, Some(filter), actor))
                .transpose()
        })
        .await
    }
    pub async fn list_worktree_threads(
        &self,
        data_dir: PathBuf,
        path: &str,
        filter: ReviewWorktreeFilter,
    ) -> Result<Vec<ReviewThread>, ReviewError> {
        let (path, actor) = self.context.worktree(path).await?;
        let comments = self.comments.clone();
        blocking(move || comments.list_threads(&data_dir, &path, Some(filter.into()), actor)).await
    }
    pub async fn get_thread(
        &self,
        data_dir: PathBuf,
        id: &str,
        thread_id: String,
    ) -> Result<Option<ReviewThread>, ReviewError> {
        let context = self.context.session_for_read(id).await?;
        let comments = self.comments.clone();
        blocking(move || {
            missing_as_none(
                context
                    .map(|(path, _)| comments.get_thread(&data_dir, &path, &thread_id))
                    .transpose(),
            )
        })
        .await
    }
    pub async fn history(
        &self,
        data_dir: PathBuf,
        id: &str,
        thread_id: String,
    ) -> Result<Option<Vec<ReviewHistoryEntry>>, ReviewError> {
        let context = self.context.session_for_read(id).await?;
        let comments = self.comments.clone();
        blocking(move || {
            missing_as_none(
                context
                    .map(|(path, _)| {
                        comments.get_thread(&data_dir, &path, &thread_id)?;
                        comments.history(&data_dir, &path, &thread_id)
                    })
                    .transpose(),
            )
        })
        .await
    }
}
fn missing_as_none<T>(result: Result<Option<T>, ReviewError>) -> Result<Option<T>, ReviewError> {
    match result {
        Err(ReviewError::NotFound(_)) => Ok(None),
        result => result,
    }
}
async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, ReviewError> + Send + 'static,
) -> Result<T, ReviewError> {
    crate::common::operation_context::spawn_blocking(operation)
        .await
        .map_err(|error| {
            ReviewError::Technical(crate::domain::failure::TechnicalFailure::from(error))
        })?
}
#[cfg(test)]
#[path = "session_test.rs"]
mod session_tests;
