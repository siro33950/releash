use crate::common::retry::RetryBackoff;
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
    delay: crate::infrastructure::timer::Delay,
) {
    let ScanWorker {
        state,
        scanner,
        mut receiver,
    } = worker;
    let rx = receiver.as_mut();
    while let Some(first_reason) = rx.recv().await {
        if !state.should_scan(&first_reason) {
            break;
        }
        let mut reason = collect_debounced_reasons(first_reason, rx, &delay).await;
        loop {
            use crate::usecase::repository_state::worktree::ScanContinuation;
            if !state.should_scan(&reason) {
                return;
            }
            let generation = state.requested_generation();
            let status = if reason.files {
                Some(
                    retrying
                        .restart(None, RetryBackoff::ITEM, |_| {
                            state.scan_once(scanner.clone(), runtime.as_ref())
                        })
                        .await,
                )
            } else {
                None
            };
            match state
                .finish_worker_scan(
                    generation,
                    reason,
                    status,
                    scanner.clone(),
                    runtime.as_ref(),
                )
                .await
            {
                ScanContinuation::Stop => return,
                ScanContinuation::Finished => break,
                ScanContinuation::Pending(mut pending) => {
                    pending.merge(collect_pending_reasons(rx));
                    reason = pending;
                }
                ScanContinuation::Debounce(mut pending) => {
                    pending.merge(collect_pending_reasons(rx));
                    reason = collect_debounced_reasons(pending, rx, &delay).await;
                }
            }
        }
    }
}

async fn collect_debounced_reasons(
    mut reason: InvalidateReason,
    rx: &mut dyn RepositoryStateInvalidationReceiver,
    delay: &crate::infrastructure::timer::Delay,
) -> InvalidateReason {
    delay().await;
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

pub const DEBOUNCE: Duration = Duration::from_millis(300);

pub fn start(
    retrying: Arc<Retrying>,
    runtime: Arc<dyn RepositoryStateWorkerRuntime>,
    delay: crate::infrastructure::timer::Delay,
) -> tokio::sync::mpsc::UnboundedSender<ScanWorker> {
    let (sender, mut requests) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(worker) = requests.recv().await {
            tokio::spawn(run_worker(
                retrying.clone(),
                worker,
                runtime.clone(),
                delay.clone(),
            ));
        }
    });
    sender
}

#[derive(Default)]
pub struct RepositoryScanWorkerRuntime;

impl RepositoryScanWorkerRuntime {
    pub fn new() -> Self {
        Self
    }
}

struct TokioInvalidationSender(tokio::sync::mpsc::UnboundedSender<InvalidateReason>);

impl RepositoryStateInvalidationSender for TokioInvalidationSender {
    fn send(&self, reason: InvalidateReason) -> Result<(), RepositoryStateError> {
        self.0.send(reason).map_err(|_| {
            RepositoryStateError::Watcher("repository snapshot worker is stopped".into())
        })
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
