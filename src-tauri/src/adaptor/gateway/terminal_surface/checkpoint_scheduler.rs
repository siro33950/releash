use crate::domain::retry::RetryBackoff;
use crate::usecase::work_queue::{WorkFailure, WorkKey};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub(crate) type CheckpointFlushFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), WorkFailure>> + Send>>;
type CheckpointFlushAsync = Arc<dyn Fn() -> CheckpointFlushFuture + Send + Sync>;
type CheckpointFlush = Arc<dyn Fn() -> Result<(), WorkFailure> + Send + Sync>;

#[derive(Clone)]
pub(crate) struct DirtyCheckpointScheduler {
    queue: std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
    key: WorkKey,
    dirty: Arc<AtomicBool>,
    interval: Duration,
    flush: CheckpointFlush,
    flush_async: CheckpointFlushAsync,
}

impl DirtyCheckpointScheduler {
    pub(crate) fn spawn(
        queue: std::sync::Arc<crate::usecase::work_queue::WorkQueueUsecase>,
        target: String,
        interval: Duration,
        flush: CheckpointFlush,
        flush_async: CheckpointFlushAsync,
    ) -> Self {
        Self {
            queue,
            key: WorkKey::new("terminal_checkpoint", &target),
            dirty: Arc::new(AtomicBool::new(false)),
            interval,
            flush,
            flush_async,
        }
    }

    pub(crate) fn mark_dirty(&self) {
        if self.dirty.swap(true, Ordering::AcqRel) {
            return;
        }
        let scheduler = self.clone();
        self.queue.spawn(Box::pin(async move {
            let key = scheduler.key.clone();
            let interval = scheduler.interval;
            scheduler
                .queue
                .clone()
                .enqueue_changed_after(
                    key,
                    RetryBackoff::ITEM,
                    Arc::new(move |_| {
                        let scheduler = scheduler.clone();
                        Box::pin(async move {
                            scheduler.dirty.store(false, Ordering::Release);
                            (scheduler.flush_async)().await.map(|()| None)
                        })
                    }),
                    interval,
                )
                .await;
        }));
    }

    pub(crate) fn flush(&self) -> Result<(), String> {
        self.dirty.store(false, Ordering::Release);
        (self.flush)().map_err(|error| error.to_string())
    }
}

#[cfg(test)]
#[path = "checkpoint_scheduler_test.rs"]
mod checkpoint_scheduler_tests;
