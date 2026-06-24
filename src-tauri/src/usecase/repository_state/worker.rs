use std::sync::Arc;
use std::time::Duration;

use super::runtime::{RepositoryStateInvalidationReceiver, RepositoryStateWorkerRuntime};
use super::scanner::RepositoryScanner;
use super::worktree::WorktreeState;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InvalidateReason {
    pub file_change: bool,
    pub git_change: bool,
    pub branch_change: bool,
    pub shutdown: bool,
    pub path: Option<String>,
}

impl InvalidateReason {
    pub fn initial() -> Self {
        Self {
            git_change: true,
            branch_change: true,
            ..Self::default()
        }
    }

    pub fn file(path: Option<String>) -> Self {
        Self {
            file_change: true,
            path,
            ..Self::default()
        }
    }

    pub fn git(branch_change: bool) -> Self {
        Self {
            git_change: true,
            branch_change,
            ..Self::default()
        }
    }

    pub fn shutdown() -> Self {
        Self {
            shutdown: true,
            ..Self::default()
        }
    }

    pub fn merge(&mut self, other: Self) {
        self.file_change |= other.file_change;
        self.git_change |= other.git_change;
        self.branch_change |= other.branch_change;
        self.shutdown |= other.shutdown;
        if self.path.is_none() {
            self.path = other.path;
        }
    }
}

pub(crate) async fn run_worker(
    state: Arc<WorktreeState>,
    scanner: Arc<dyn RepositoryScanner>,
    runtime: Arc<dyn RepositoryStateWorkerRuntime>,
    mut rx: Box<dyn RepositoryStateInvalidationReceiver>,
    debounce: Duration,
) {
    while let Some(first_reason) = rx.recv().await {
        if state.is_shutdown() || first_reason.shutdown {
            break;
        }
        let mut reason =
            collect_debounced_reasons(first_reason, rx.as_mut(), runtime.as_ref(), debounce).await;

        loop {
            if state.is_shutdown() || reason.shutdown {
                return;
            }
            let start_generation = state.requested_generation();
            state.mark_refresh_started(&reason);

            let repo_path = state.worktree_path().to_string();
            let scanner = scanner.clone();
            let scan_result = runtime.scan(scanner.clone(), repo_path).await;

            let current_generation = state.requested_generation();
            if state.is_shutdown() {
                return;
            }
            if current_generation != start_generation {
                reason.merge(collect_pending_reasons(rx.as_mut()));
                if debounce > Duration::ZERO {
                    reason =
                        collect_debounced_reasons(reason, rx.as_mut(), runtime.as_ref(), debounce)
                            .await;
                }
                continue;
            }

            state.set_refreshing(false);

            match scan_result {
                Ok(parts) => {
                    let snapshot = state.commit_snapshot(parts, start_generation);
                    let names: Vec<String> = snapshot
                        .branch_cards
                        .iter()
                        .map(|card| card.name.clone())
                        .collect();
                    if let Err(err) =
                        scanner.prune_stale_branch_bases(state.worktree_path(), &names)
                    {
                        log::warn!(
                            "repository snapshot branch base GC failed for {}: {err}",
                            state.worktree_path()
                        );
                    }
                    state.notify_snapshot_changed(snapshot, reason);
                }
                Err(err) => {
                    log::warn!(
                        "repository snapshot scan failed for {}: {err}",
                        state.worktree_path()
                    );
                    state.mark_scan_failed();
                }
            }
            break;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons_merge_flags_and_keep_first_path() {
        let mut a = InvalidateReason::file(Some("a.txt".to_string()));
        a.merge(InvalidateReason::git(true));
        a.merge(InvalidateReason::file(Some("b.txt".to_string())));

        assert!(a.file_change);
        assert!(a.git_change);
        assert!(a.branch_change);
        assert_eq!(a.path.as_deref(), Some("a.txt"));
    }
}
