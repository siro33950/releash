use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEvent};

use crate::usecase::repository_state::runtime::WorktreePathNormalizer;
use crate::usecase::repository_state::service::RepositoryStateRepository;
use crate::usecase::repository_state::worker::InvalidateReason;
use crate::usecase::repository_state::worktree::{
    RepositoryStateWatchSession, RepositoryStateWatcher, WorktreeState,
};
use crate::usecase::repository_state::RepositoryStateError;
use crate::usecase::repository_usecase::RepositoryUsecase;

use super::watch::{
    classify_git_dir_events, generate_watcher_id, resolve_file_watch_paths,
    resolve_worktree_git_dir,
};

type RecommendedDebouncer =
    notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::RecommendedWatcher>;

pub struct RepositoryStateRepositoryGateway {
    repository: Arc<RepositoryUsecase>,
}

impl RepositoryStateRepositoryGateway {
    pub fn new(repository: Arc<RepositoryUsecase>) -> Self {
        Self { repository }
    }
}

impl RepositoryStateRepository for RepositoryStateRepositoryGateway {
    fn main_repo_path(&self, path: &str) -> Result<String, RepositoryStateError> {
        let root = self.repository.get_main_repo_path(path)?;
        Ok(super::worktree_operation::worktree_identity(&root)
            .map_err(crate::usecase::repository_error::UsecaseError::from)?
            .to_string_lossy()
            .into_owned())
    }
}

struct RepositoryStateWatcherHandles {
    file_debouncer: Option<RecommendedDebouncer>,
    git_debouncer: Option<RecommendedDebouncer>,
}

impl Drop for RepositoryStateWatcherHandles {
    fn drop(&mut self) {
        // FsEventWatcher の drop は run loop の停止待ちでブロックし得るため、
        // 呼び出し元スレッドでは実行しない（#1641）
        crate::infrastructure::dispose::dispose_in_background(
            "repository-watcher-dispose",
            (self.file_debouncer.take(), self.git_debouncer.take()),
        );
    }
}

pub struct NotifyRepositoryStateWatcher {
    repository: Arc<RepositoryUsecase>,
}

impl NotifyRepositoryStateWatcher {
    pub fn new(repository: Arc<RepositoryUsecase>) -> Self {
        Self { repository }
    }
}

impl RepositoryStateWatcher for NotifyRepositoryStateWatcher {
    fn next_watcher_id(&self) -> u64 {
        generate_watcher_id()
    }

    fn start_watchers(
        &self,
        state: Arc<WorktreeState>,
    ) -> Result<Box<dyn RepositoryStateWatchSession>, RepositoryStateError> {
        let file_debouncer = start_file_watcher(state.clone(), &self.repository)?;
        let git_debouncer = start_git_watcher(state)?;
        Ok(Box::new(RepositoryStateWatcherHandles {
            file_debouncer: Some(file_debouncer),
            git_debouncer: Some(git_debouncer),
        }))
    }
}

fn start_file_watcher(
    state: Arc<WorktreeState>,
    repository: &RepositoryUsecase,
) -> Result<RecommendedDebouncer, RepositoryStateError> {
    let watch_paths = resolve_file_watch_paths(repository, state.worktree_path());
    let debouncer = new_debouncer(
        Duration::from_millis(100),
        move |res: Result<
            Vec<notify_debouncer_mini::DebouncedEvent>,
            notify_debouncer_mini::notify::Error,
        >| match res {
            Ok(events) => handle_file_events(state.as_ref(), events),
            Err(err) => {
                handle_watch_failure(
                    state.as_ref(),
                    format!("file watcher error for {}: {err}", state.worktree_path()),
                );
            }
        },
    )
    .map_err(|err| RepositoryStateError::Watcher(format!("Failed to create debouncer: {err}")))?;

    let mut debouncer = debouncer;
    for watch_path in watch_paths {
        debouncer
            .watcher()
            .watch(&watch_path, RecursiveMode::Recursive)
            .map_err(|err| {
                RepositoryStateError::Watcher(format!(
                    "Failed to watch path {}: {err}",
                    watch_path.display()
                ))
            })?;
    }
    Ok(debouncer)
}

/// worktree 自身の git ディレクトリを監視する。Repository の root の git ディレクトリには
/// ref と全 worktree の登録・HEAD があるので、root は配下ごと監視する。
fn start_git_watcher(
    state: Arc<WorktreeState>,
) -> Result<RecommendedDebouncer, RepositoryStateError> {
    let git_dir =
        resolve_worktree_git_dir(state.worktree_path()).map_err(RepositoryStateError::Watcher)?;
    log::debug!("starting git watcher for {}", state.worktree_path());
    let mode = if state.is_repository_root() {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    let own_git_dir = git_dir.canonicalize().unwrap_or_else(|_| git_dir.clone());
    let debouncer = new_debouncer(
        Duration::from_millis(100),
        move |res: Result<
            Vec<notify_debouncer_mini::DebouncedEvent>,
            notify_debouncer_mini::notify::Error,
        >| {
            let events = match res {
                Ok(events) => events,
                Err(err) => {
                    handle_watch_failure(
                        state.as_ref(),
                        format!("git dir watcher error for {}: {err}", state.worktree_path()),
                    );
                    return;
                }
            };
            handle_git_events(state.as_ref(), &own_git_dir, &events);
        },
    )
    .map_err(|err| RepositoryStateError::Watcher(format!("Failed to create debouncer: {err}")))?;

    let mut debouncer = debouncer;
    debouncer
        .watcher()
        .watch(&git_dir, mode)
        .map_err(|err| RepositoryStateError::Watcher(format!("Failed to watch git dir: {err}")))?;
    Ok(debouncer)
}

fn handle_watch_failure(state: &WorktreeState, message: String) {
    state.mark_scan_failed(&RepositoryStateError::Watcher(message));
    state.notify_snapshot_changed();
}

/// worktree のファイルの変化で、変更の状態を読み直す。
/// git ディレクトリの中の変化は git の監視が扱うので、ここでは数えない。
fn handle_file_events(state: &WorktreeState, events: Vec<DebouncedEvent>) {
    let git_dir = Path::new(state.worktree_path()).join(".git");
    if events.iter().any(|event| !event.path.starts_with(&git_dir)) {
        state.invalidate(InvalidateReason::files());
    }
}

/// ref・HEAD・worktree の登録の変化は Repository の root が worktree の並びを読み直す。
/// index の変化は、その index を持つ worktree だけが変更の状態を読み直す。
fn handle_git_events(state: &WorktreeState, own_git_dir: &Path, events: &[DebouncedEvent]) {
    let (branch_change, _) = classify_git_dir_events(events);
    let own_events = events
        .iter()
        .filter(|event| {
            event.path.parent().is_some_and(|parent| {
                parent == own_git_dir
                    || parent
                        .canonicalize()
                        .is_ok_and(|parent| parent == own_git_dir)
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    let (_, index_change) = classify_git_dir_events(&own_events);
    let mut reason = InvalidateReason::default();
    if branch_change && state.is_repository_root() {
        reason.merge(InvalidateReason::refs());
    }
    if index_change {
        reason.merge(InvalidateReason::files());
    }
    if reason != InvalidateReason::default() {
        state.invalidate(reason);
    }
}

pub struct FsWorktreePathNormalizer;

impl WorktreePathNormalizer for FsWorktreePathNormalizer {
    fn normalize(&self, worktree_path: &str) -> Result<PathBuf, RepositoryStateError> {
        let path = Path::new(worktree_path);
        path.canonicalize().map_err(|err| {
            RepositoryStateError::Watcher(format!(
                "Failed to canonicalize worktree path {}: {err}",
                path.display()
            ))
        })
    }
}

#[cfg(test)]
#[path = "state_test.rs"]
mod state_tests;
