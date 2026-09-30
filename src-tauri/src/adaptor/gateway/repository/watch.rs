use crate::adaptor::gateway::shared::git_operation;
use notify_debouncer_mini::DebouncedEvent;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::domain::path::to_canonical_forward_slash;
use crate::usecase::repository_usecase::RepositoryUsecase;

/// worktree 自身の git ディレクトリ。index と HEAD がここにある。
/// main worktree では Repository の git ディレクトリで、ref と全 worktree の登録も持つ。
pub(crate) fn resolve_worktree_git_dir(repo_path: &str) -> Result<PathBuf, String> {
    Ok(git_operation::run(|| git2::Repository::open(repo_path))
        .map_err(|e| format!("Failed to open worktree: {e}"))?
        .path()
        .to_path_buf())
}

pub(crate) fn resolve_file_watch_paths(
    _usecase: &RepositoryUsecase,
    repo_path: &str,
) -> Vec<PathBuf> {
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
mod tests {
    use super::*;
    use crate::test_support::git::*;
    use notify_debouncer_mini::DebouncedEventKind;

    fn test_usecase() -> RepositoryUsecase {
        crate::adaptor::controller::wiring::build_repository_usecase()
    }

    fn create_worktree(repo: &git2::Repository) -> (String, PathBuf, tempfile::TempDir) {
        static WT_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let id = WT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let wt_name = format!("wt-test-{}", id);
        let wt_dir = tempfile::TempDir::new().unwrap();
        let wt_path = wt_dir.path().join(&wt_name);
        repo.worktree(&wt_name, &wt_path, None).unwrap();
        (wt_name, wt_path, wt_dir)
    }

    #[test]
    fn test_git監視の対象_main_worktreeはrepositoryのgitディレクトリを返す() {
        // Given
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        // When
        let git_dir = resolve_worktree_git_dir(dir.path().to_str().unwrap()).unwrap();

        // Then
        assert_eq!(
            git_dir.canonicalize().unwrap(),
            dir.path().join(".git").canonicalize().unwrap()
        );
        assert!(git_dir.join("HEAD").exists());
        assert!(git_dir.join("refs/heads").exists());
    }

    #[test]
    fn test_git監視の対象_linked_worktreeは自分の登録ディレクトリを返す() {
        // Given
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content", "add file");
        let (wt_name, wt_path, _wt_dir) = create_worktree(&repo);

        // When
        let git_dir = resolve_worktree_git_dir(wt_path.to_str().unwrap()).unwrap();

        // Then
        assert_eq!(
            git_dir.canonicalize().unwrap(),
            dir.path()
                .join(".git/worktrees")
                .join(wt_name)
                .canonicalize()
                .unwrap()
        );
        assert!(git_dir.join("index").exists());
    }

    #[test]
    fn test_git監視の対象_repositoryでないpathは失敗を返す() {
        assert!(resolve_worktree_git_dir("/nonexistent/path").is_err());
    }

    #[test]
    fn resolve_file_watch_paths_uses_only_current_worktree() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content", "add file");
        let (_wt_name, wt_path, _wt_dir) = create_worktree(&repo);

        let paths = resolve_file_watch_paths(&test_usecase(), dir.path().to_str().unwrap());

        assert_eq!(paths, vec![dir.path().canonicalize().unwrap()]);
        assert!(!paths.contains(&wt_path.canonicalize().unwrap()));
    }

    fn make_event(path: &str) -> DebouncedEvent {
        DebouncedEvent {
            path: PathBuf::from(path),
            kind: DebouncedEventKind::Any,
        }
    }

    #[test]
    fn test_worktree登録変更_最初のdirectory作成と配下の追加削除を検出する() {
        for path in [
            "/repo/.git/worktrees",
            "/repo/.git/worktrees/first",
            "/repo/.git/worktrees/first/gitdir",
        ] {
            assert_eq!(classify_git_dir_events(&[make_event(path)]), (true, false));
        }
    }

    #[test]
    fn classify_index_change() {
        let events = vec![make_event("/repo/.git/index")];
        let (branch, index) = classify_git_dir_events(&events);
        assert!(!branch);
        assert!(index);
    }

    #[test]
    fn classify_index_lock() {
        let events = vec![make_event("/repo/.git/index.lock")];
        let (branch, index) = classify_git_dir_events(&events);
        assert!(!branch);
        assert!(index);
    }

    #[test]
    fn classify_head_change() {
        let events = vec![make_event("/repo/.git/HEAD")];
        let (branch, index) = classify_git_dir_events(&events);
        assert!(branch);
        assert!(!index);
    }

    #[test]
    fn classify_refs_heads_change() {
        let events = vec![make_event("/repo/.git/refs/heads/main")];
        let (branch, index) = classify_git_dir_events(&events);
        assert!(branch);
        assert!(!index);
    }

    #[test]
    fn classify_commit_editmsg() {
        let events = vec![make_event("/repo/.git/COMMIT_EDITMSG")];
        let (branch, index) = classify_git_dir_events(&events);
        assert!(!branch);
        assert!(index);
    }

    #[test]
    fn classify_mixed_events() {
        let events = vec![
            make_event("/repo/.git/refs/heads/feature"),
            make_event("/repo/.git/index"),
        ];
        let (branch, index) = classify_git_dir_events(&events);
        assert!(branch);
        assert!(index);
    }

    #[test]
    fn classify_unrelated_event() {
        let events = vec![make_event("/repo/.git/config")];
        let (branch, index) = classify_git_dir_events(&events);
        assert!(!branch);
        assert!(!index);
    }

    #[test]
    fn classify_worktree_index() {
        let events = vec![make_event("/repo/.git/worktrees/feat/index")];
        let (branch, index) = classify_git_dir_events(&events);
        assert!(!branch);
        assert!(index);
    }
}
