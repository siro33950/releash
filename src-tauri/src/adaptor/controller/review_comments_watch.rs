use crate::common::retry::RetryBackoff;
use crate::usecase::comment::ReviewCommentsWatchUsecase;
use crate::usecase::failure::FailureKey;
use crate::usecase::retry::Retrying;
use futures_util::{Stream, StreamExt};
use std::sync::Arc;

pub(crate) async fn run(
    retrying: Arc<Retrying>,
    usecase: Arc<ReviewCommentsWatchUsecase>,
    target: String,
    mut ticks: impl Stream<Item = ()> + Unpin,
) {
    while ticks.next().await.is_some() {
        let polled = retrying
            .restart(
                FailureKey::new("review_comments_watch", &target),
                RetryBackoff::ITEM,
                |progress| usecase.poll(progress),
            )
            .await;
        if polled.is_err() {
            return;
        }
    }
}

#[cfg(test)]
#[path = "review_comments_watch_test.rs"]
mod review_comments_watch_tests;
