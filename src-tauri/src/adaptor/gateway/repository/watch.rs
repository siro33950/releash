use crate::adaptor::gateway::shared::git_operation;
use notify_debouncer_mini::DebouncedEvent;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::domain::path::to_canonical_forward_slash;
use crate::usecase::repository_usecase::RepositoryUsecase;

/// worktree 自身の git ディレクトリ。index と HEAD がここにある。
/// main worktree では Repository の git ディレクトリで、ref と全 worktree の登録も持つ。
pub fn resolve_worktree_git_dir(repo_path: &str) -> Result<PathBuf, String> {
    Ok(git_operation::run(|| git2::Repository::open(repo_path))
        .map_err(|e| format!("Failed to open worktree: {e}"))?
        .path()
        .to_path_buf())
}

pub fn resolve_file_watch_paths(_usecase: &RepositoryUsecase, repo_path: &str) -> Vec<PathBuf> {
    let path = PathBuf::from(repo_path);
    vec![path.canonicalize().unwrap_or(path)]
}

static WATCHER_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(crate) fn generate_watcher_id() -> u64 {
    WATCHER_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

pub(crate) fn classify_git_dir_events(events: &[DebouncedEvent]) -> (bool, bool) {
    let has_branch_change = events.iter().any(|e| {
        let p = to_canonical_forward_slash(&e.path.to_string_lossy());
        p.contains("/refs/heads/")
            || e.path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == "worktrees")
            || (p.contains("/worktrees/")
                && e.path.file_name().is_some_and(|name| name == "gitdir"))
            || p.ends_with("/worktrees")
            || e.path.file_name().is_some_and(|n| n == "HEAD")
    });
    let has_index_change = events.iter().any(|e| {
        let file_name = e
            .path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        file_name == "index" || file_name == "index.lock" || file_name == "COMMIT_EDITMSG"
    });
    (has_branch_change, has_index_change)
}

#[cfg(test)]
#[path = "watch_test.rs"]
mod watch_tests;
