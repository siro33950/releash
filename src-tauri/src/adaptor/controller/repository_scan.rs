use crate::common::retry::RetryBackoff;
use crate::usecase::failure::FailureKey;
use crate::usecase::repository_state::runtime::{
    RepositoryStateInvalidationReceiver, RepositoryStateInvalidationSender,
    RepositoryStateWorkerRuntime, ScanWorker,
};
use crate::usecase::repository_state::scanner::RepositoryScanner;
use crate::usecase::repository_state::snapshot::RepositorySnapshotParts;
use crate::usecase::repository_state::worker::InvalidateReason;
use crate::usecase::repository_state::RepositoryStateError;
use crate::usecase::retry::Retrying;
use std::sync::Arc;
use std::time::Duration;

pub(crate) async fn run_worker(
    retrying: Arc<Retrying>,
    worker: ScanWorker,
    runtime: Arc<dyn RepositoryStateWorkerRuntime>,
) {
    let ScanWorker {
        state,
        scanner,
        mut receiver,
        debounce,
    } = worker;
    let rx = receiver.as_mut();
    while let Some(first_reason) = rx.recv().await {
        if state.is_shutdown() || first_reason.shutdown {
            break;
        }
        let mut reason =
            collect_debounced_reasons(first_reason, rx, runtime.as_ref(), debounce).await;
        loop {
            if state.is_shutdown() || reason.shutdown {
                return;
            }
            let start_generation = state.requested_generation();
            let status = if reason.files {
                Some(
                    retrying
                        .restart(
                            FailureKey::new("repository_scan", state.worktree_path()),
                            RetryBackoff::ITEM,
                            |_| state.scan_once(scanner.clone(), runtime.as_ref()),
                        )
                        .await,
                )
            } else {
                None
            };
            if reason.refs {
                state
                    .scan_worktrees_once(scanner.clone(), runtime.as_ref())
                    .await;
            }
            if state.is_shutdown() {
                return;
            }
            if state.requested_generation() != start_generation {
                reason.merge(collect_pending_reasons(rx));
                if debounce > Duration::ZERO {
                    reason =
                        collect_debounced_reasons(reason, rx, runtime.as_ref(), debounce).await;
                }
                continue;
            }
            match state.finish_scan(status, reason) {
                Some(pending) => {
                    reason = pending;
                    reason.merge(collect_pending_reasons(rx));
                }
                None => break,
            }
        }
    }
}

async fn collect_debounced_reasons(
    mut reason: InvalidateReason,
    rx: &mut dyn RepositoryStateInvalidationReceiver,
    runtime: &dyn RepositoryStateWorkerRuntime,
    debounce: Duration,
) -> InvalidateReason {
    if debounce > Duration::ZERO {
        runtime.sleep(debounce).await;
    }
    reason.merge(collect_pending_reasons(rx));
    reason
}

fn collect_pending_reasons(rx: &mut dyn RepositoryStateInvalidationReceiver) -> InvalidateReason {
    let mut reason = InvalidateReason::default();
    while let Some(next) = rx.try_recv() {
        reason.merge(next);
    }
    reason
}

pub struct RepositoryScanWorkerRuntime {
    retrying: Arc<Retrying>,
}

impl RepositoryScanWorkerRuntime {
    pub fn new(retrying: Arc<Retrying>) -> Self {
        Self { retrying }
    }
}

struct TokioInvalidationSender(tokio::sync::mpsc::UnboundedSender<InvalidateReason>);

impl RepositoryStateInvalidationSender for TokioInvalidationSender {
    fn send(&self, reason: InvalidateReason) -> Result<(), ()> {
        self.0.send(reason).map_err(|_| ())
    }
}

struct TokioInvalidationReceiver(tokio::sync::mpsc::UnboundedReceiver<InvalidateReason>);

#[async_trait::async_trait]
impl RepositoryStateInvalidationReceiver for TokioInvalidationReceiver {
    async fn recv(&mut self) -> Option<InvalidateReason> {
        self.0.recv().await
    }

    fn try_recv(&mut self) -> Option<InvalidateReason> {
        self.0.try_recv().ok()
    }
}

#[async_trait::async_trait]
impl RepositoryStateWorkerRuntime for RepositoryScanWorkerRuntime {
    fn invalidation_channel(
        &self,
    ) -> (
        Box<dyn RepositoryStateInvalidationSender>,
        Box<dyn RepositoryStateInvalidationReceiver>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Box::new(TokioInvalidationSender(tx)),
            Box::new(TokioInvalidationReceiver(rx)),
        )
    }

    fn spawn_worker(&self, worker: ScanWorker) {
        let runtime: Arc<dyn RepositoryStateWorkerRuntime> = Arc::new(Self {
            retrying: self.retrying.clone(),
        });
        tokio::spawn(run_worker(self.retrying.clone(), worker, runtime));
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }

    async fn scan(
        &self,
        scanner: Arc<dyn RepositoryScanner>,
        repo_path: String,
    ) -> Result<RepositorySnapshotParts, RepositoryStateError> {
        scanner.scan_async(&repo_path).await
    }

    async fn scan_worktrees(
        &self,
        scanner: Arc<dyn RepositoryScanner>,
        repo_path: String,
    ) -> Result<Vec<crate::domain::repository::Worktree>, RepositoryStateError> {
        crate::common::operation_context::spawn_blocking(move || scanner.scan_worktrees(&repo_path))
            .await
            .map_err(|error| {
                RepositoryStateError::Watcher(format!("worktree scan failed: {error}"))
            })?
    }
}

#[cfg(test)]
#[path = "repository_scan_test.rs"]
mod repository_scan_tests;
