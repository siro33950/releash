use super::super::shared::{
    background_io,
    background_worker::{request, Request},
};
use crate::domain::comment::ReviewCommentsWatch;
use crate::domain::failure::{TechnicalFailure, TechnicalFailureNature};
use crate::infrastructure::process::background_worker::BackgroundWorker;
use crate::usecase::failure::Failure;
use crate::usecase::failure::WorkFailure;
use std::path::PathBuf;
use std::sync::Arc;

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

pub struct ReviewCommentsWatchGateway {
    dir: PathBuf,
    notify_changed: Arc<dyn Fn() + Send + Sync>,
    start: Start,
    worker: tokio::sync::Mutex<Option<BackgroundWorker>>,
}

impl ReviewCommentsWatchGateway {
    pub fn new(dir: PathBuf, notify_changed: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self::with_start(dir, notify_changed, start())
    }

    fn with_start(dir: PathBuf, notify_changed: Arc<dyn Fn() + Send + Sync>, start: Start) -> Self {
        Self {
            dir,
            notify_changed,
            start,
            worker: tokio::sync::Mutex::new(None),
        }
    }
}

fn technical(failure: WorkFailure) -> TechnicalFailure {
    TechnicalFailure {
        nature: match failure.kind {
            Failure::Technical(nature) => nature,
            Failure::Business(_) => TechnicalFailureNature::Other,
        },
        message: failure.message,
    }
}

#[async_trait::async_trait]
impl ReviewCommentsWatch for ReviewCommentsWatchGateway {
    async fn ensure_started(&self) -> Result<(), TechnicalFailure> {
        let mut worker = self.worker.lock().await;
        if worker.is_none() {
            *worker = Some(
                (self.start)(self.dir.clone(), self.notify_changed.clone())
                    .await
                    .map_err(technical)?,
            );
        }
        Ok(())
    }

    async fn poll(&self) -> Result<(), TechnicalFailure> {
        let mut worker = self.worker.lock().await;
        let Some(current) = worker.as_mut() else {
            return Err(TechnicalFailure {
                nature: TechnicalFailureNature::Other,
                message: "watcher is not started".into(),
            });
        };
        let polled = request::<()>(current, &Request::WatchPoll).await;
        if polled.is_err() {
            *worker = None;
        }
        polled.map_err(technical)
    }

    async fn restart(&self) -> Result<(), TechnicalFailure> {
        if let Some(mut worker) = self.worker.lock().await.take() {
            worker
                .stop()
                .await
                .map_err(|error| technical(background_io::failure(error)))?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "watcher_test.rs"]
mod watcher_tests;
