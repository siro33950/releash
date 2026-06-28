use notify_debouncer_mini::DebouncedEvent;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::domain::path::to_canonical_forward_slash;
use crate::usecase::repository_usecase::RepositoryUsecase;

pub(crate) struct GitWatchPaths {
    pub(crate) main_repo: String,
    pub(crate) refs_heads: PathBuf,
    pub(crate) head_file: PathBuf,
    pub(crate) index_file: PathBuf,
    pub(crate) worktrees_dir: PathBuf,
}

pub(crate) fn resolve_git_watch_paths(
    usecase: &RepositoryUsecase,
    repo_path: &str,
) -> Result<GitWatchPaths, String> {
    let main_repo = usecase
        .get_main_repo_path(repo_path)
        .map_err(|e| format!("Failed to resolve main repo: {e}"))?;
    let git_dir = git2::Repository::open(&main_repo)
        .map_err(|e| format!("Failed to open repo: {e}"))?
        .path()
        .to_path_buf();

    Ok(GitWatchPaths {
        main_repo,
        refs_heads: git_dir.join("refs").join("heads"),
        head_file: git_dir.join("HEAD"),
        index_file: git_dir.join("index"),
        worktrees_dir: git_dir.join("worktrees"),
    })
}

pub(crate) fn resolve_file_watch_paths(
    _usecase: &RepositoryUsecase,
    repo_path: &str,
) -> Vec<PathBuf> {
    let path = PathBuf::from(repo_path);
    vec![path.canonicalize().unwrap_or(path)]
}

pub(crate) fn canonicalize_event_path(path: &Path) -> String {
    if let Ok(canonical) = path.canonicalize() {
        return to_canonical_forward_slash(&canonical.to_string_lossy());
    }
    if let (Some(parent), Some(file_name)) = (path.parent(), path.file_name()) {
        if let Ok(canonical_parent) = parent.canonicalize() {
            let path = canonical_parent
                .join(file_name)
                .to_string_lossy()
                .to_string();
            return to_canonical_forward_slash(&path);
        }
    }
    to_canonical_forward_slash(&path.to_string_lossy())
}

static WATCHER_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(crate) fn generate_watcher_id() -> u64 {
    WATCHER_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FileChangeEvent {
    pub watcher_id: u64,
    pub path: String,
    pub kind: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GitStatusChangedEvent {
    pub repo_path: String,
}

pub(crate) fn classify_git_dir_events(events: &[DebouncedEvent]) -> (bool, bool) {
    let has_branch_change = events.iter().any(|e| {
        let p = to_canonical_forward_slash(&e.path.to_string_lossy());
        p.contains("/refs/heads/") || e.path.file_name().is_some_and(|n| n == "HEAD")
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

    #[test]
    fn test_resolve_git_watch_paths_main_repo() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);

        let repo_path = dir.path().to_str().unwrap();
        let paths = resolve_git_watch_paths(&test_usecase(), repo_path).unwrap();

        assert_eq!(
            PathBuf::from(&paths.main_repo).canonicalize().unwrap(),
            dir.path().canonicalize().unwrap()
        );
        assert!(paths.refs_heads.ends_with("refs/heads"));
        assert!(paths.head_file.ends_with("HEAD"));
        assert!(paths.worktrees_dir.ends_with("worktrees"));
        assert!(paths.refs_heads.exists());
        assert!(paths.head_file.exists());
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
    fn test_resolve_git_watch_paths_with_worktree() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content", "add file");

        let (wt_name, _wt_path, _wt_dir) = create_worktree(&repo);

        let repo_path = dir.path().to_str().unwrap();
        let paths = resolve_git_watch_paths(&test_usecase(), repo_path).unwrap();

        assert!(paths.worktrees_dir.exists());
        assert!(paths.worktrees_dir.join(&wt_name).exists());
    }

    #[test]
    fn test_resolve_git_watch_paths_from_worktree() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, "file.txt", "content", "add file");

        let (_wt_name, wt_path, _wt_dir) = create_worktree(&repo);

        let main_repo_path = dir.path().to_str().unwrap();
        let paths_from_wt =
            resolve_git_watch_paths(&test_usecase(), wt_path.to_str().unwrap()).unwrap();

        assert_eq!(
            PathBuf::from(&paths_from_wt.main_repo)
                .canonicalize()
                .unwrap(),
            PathBuf::from(main_repo_path).canonicalize().unwrap()
        );
        assert!(paths_from_wt.refs_heads.exists());
    }

    #[test]
    fn test_resolve_git_watch_paths_invalid() {
        let result = resolve_git_watch_paths(&test_usecase(), "/nonexistent/path");
        assert!(result.is_err());
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

    #[test]
    fn canonicalize_existing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let result = canonicalize_event_path(&file_path);
        assert!(result.ends_with("test.txt"));
        assert!(!result.contains(".."));
    }

    #[test]
    fn canonicalize_deleted_file_falls_back_to_parent() {
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("deleted.txt");

        let result = canonicalize_event_path(&file_path);
        assert!(result.ends_with("deleted.txt"));
    }

    #[test]
    fn canonicalize_nonexistent_parent_falls_back_to_normalized_path() {
        let path = PathBuf::from(r"C:\nonexistent\parent\file.txt");
        let result = canonicalize_event_path(&path);
        assert_eq!(result, "C:/nonexistent/parent/file.txt");
    }
}
