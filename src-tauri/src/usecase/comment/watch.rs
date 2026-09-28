use crate::common::retry::AttemptProgress;
use crate::domain::comment::ReviewCommentsWatch;
use crate::usecase::failure::WorkFailure;
use std::sync::Arc;

pub(crate) struct ReviewCommentsWatchUsecase {
    watch: Arc<dyn ReviewCommentsWatch>,
}

impl ReviewCommentsWatchUsecase {
    pub(crate) fn new(watch: Arc<dyn ReviewCommentsWatch>) -> Self {
        Self { watch }
    }

    pub(crate) async fn poll(&self, progress: AttemptProgress) -> Result<(), WorkFailure> {
        if progress == AttemptProgress::Reload {
            self.watch.restart().await?;
        }
        self.watch.ensure_started().await?;
        self.watch.poll().await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "watch_test.rs"]
mod watch_tests;
