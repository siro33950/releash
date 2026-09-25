use super::super::shared::{
    background_io,
    background_worker::{request, Request},
};
use crate::common::retry::RetryBackoff;
use crate::domain::failure::RetryAction;
use crate::infrastructure::process::background_worker::BackgroundWorker;
use crate::usecase::work_queue::{WorkFailure, WorkKey};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Start = Arc<
    dyn Fn(
            PathBuf,
            Arc<dyn Fn() + Send + Sync>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<BackgroundWorker, WorkFailure>> + Send>,
        > + Send
        + Sync,
>;

fn start() -> Start {
    Arc::new(|dir, notify| {
        Box::pin(async move {
            let mut worker = BackgroundWorker::start(notify).map_err(background_io::failure)?;
            request::<()>(&mut worker, &Request::WatchStart(dir)).await?;
            Ok(worker)
        })
    })
}

pub fn spawn_review_comments_watcher(
    queue: Arc<crate::usecase::work_queue::WorkQueueUsecase>,
    dir: PathBuf,
    notify_changed: Arc<dyn Fn() + Send + Sync>,
) {
    spawn_watcher(queue, dir, notify_changed, start());
}

fn spawn_watcher(
    queue: Arc<crate::usecase::work_queue::WorkQueueUsecase>,
    dir: PathBuf,
    notify_changed: Arc<dyn Fn() + Send + Sync>,
    start: Start,
) {
    let key = WorkKey::new("review_comments_watch", &dir.to_string_lossy());
    let job = watcher_job(dir, notify_changed, start);
    queue.clone().spawn(Box::pin(async move {
        queue.enqueue(key, RetryBackoff::ITEM, job).await;
    }));
}

fn watcher_job(
    dir: PathBuf,
    notify_changed: Arc<dyn Fn() + Send + Sync>,
    start: Start,
) -> crate::usecase::work_queue::Job {
    let watcher = Arc::new(Mutex::new(None::<BackgroundWorker>));
    Arc::new(move |action| {
        let watcher = watcher.clone();
        let start = start.clone();
        let dir = dir.clone();
        let notify_changed = notify_changed.clone();
        Box::pin(async move {
            let mut current = watcher.lock().expect("review watcher lock").take();
            if action == RetryAction::Restart {
                if let Some(mut worker) = current.take() {
                    worker.stop().await.map_err(background_io::failure)?;
                }
            }
            let mut current = match current {
                Some(worker) => worker,
                None => start(dir, notify_changed).await?,
            };
            let result = request::<()>(&mut current, &Request::WatchPoll).await;
            *watcher.lock().expect("review watcher lock") = Some(current);
            result.map(|()| Some(Duration::from_secs(1)))
        })
    })
}

#[cfg(test)]
#[path = "watcher_test.rs"]
mod watcher_tests;
