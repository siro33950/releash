use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::adaptor::controller::state::AppState;
use crate::protocol::{BranchCardMsg, BranchListSync, WsMessage};
use crate::usecase::repository_usecase::RepositoryUsecase;
use crate::ws_bridge::WsBroadcaster;

struct GitWatchPaths {
    main_repo: String,
    refs_heads: PathBuf,
    head_file: PathBuf,
    index_file: PathBuf,
    worktrees_dir: PathBuf,
}

fn resolve_git_watch_paths(
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

fn canonicalize_event_path(path: &std::path::Path) -> Option<String> {
    if let Ok(canonical) = path.canonicalize() {
        return Some(canonical.to_string_lossy().to_string());
    }
    let parent = path.parent()?;
    let file_name = path.file_name()?;
    let canonical_parent = parent.canonicalize().ok()?;
    Some(
        canonical_parent
            .join(file_name)
            .to_string_lossy()
            .to_string(),
    )
}

static WATCHER_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_watcher_id() -> u64 {
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

struct WatcherSession {
    _debouncer: notify_debouncer_mini::Debouncer<notify_debouncer_mini::notify::RecommendedWatcher>,
}

#[derive(Default)]
pub struct FileWatcherManager {
    sessions: Mutex<HashMap<u64, WatcherSession>>,
}

#[tauri::command]
pub fn start_watching(
    app: AppHandle,
    state: State<'_, FileWatcherManager>,
    path: String,
) -> Result<u64, String> {
    let watcher_id = generate_watcher_id();
    let watch_path = PathBuf::from(&path);

    if !watch_path.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    let app_clone = app.clone();
    let watcher_id_clone = watcher_id;

    let debouncer = new_debouncer(
        Duration::from_millis(100),
        move |res: Result<
            Vec<notify_debouncer_mini::DebouncedEvent>,
            notify_debouncer_mini::notify::Error,
        >| {
            match res {
                Ok(events) => {
                    for event in events {
                        let kind = match event.kind {
                            DebouncedEventKind::Any => "change",
                            DebouncedEventKind::AnyContinuous => "change",
                            _ => "change",
                        };
                        let event_path = canonicalize_event_path(&event.path)
                            .unwrap_or_else(|| event.path.to_string_lossy().to_string());
                        let _ = app_clone.emit(
                            "file-change",
                            FileChangeEvent {
                                watcher_id: watcher_id_clone,
                                path: event_path,
                                kind: kind.to_string(),
                            },
                        );
                    }
                }
                Err(e) => {
                    eprintln!("File watcher error: {:?}", e);
                }
            }
        },
    )
    .map_err(|e| format!("Failed to create debouncer: {}", e))?;

    let mut debouncer = debouncer;
    debouncer
        .watcher()
        .watch(&watch_path, RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to watch path: {}", e))?;

    let session = WatcherSession {
        _debouncer: debouncer,
    };

    state.sessions.lock().insert(watcher_id, session);

    Ok(watcher_id)
}

fn classify_git_dir_events(events: &[notify_debouncer_mini::DebouncedEvent]) -> (bool, bool) {
    let has_branch_change = events.iter().any(|e| {
        let p = e.path.to_string_lossy().replace('\\', "/");
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

fn build_branch_list_sync(usecase: &RepositoryUsecase, repo_path: &str) -> Option<WsMessage> {
    // ブランチ一覧取得後の GC（現存しないブランチの releash-base 掃除）は
    // usecase の list_branches_with_status が内包する。
    let branches = usecase.list_branches_with_status(repo_path).ok()?;
    let branch_msgs: Vec<BranchCardMsg> = branches.into_iter().map(BranchCardMsg::from).collect();
    Some(WsMessage::BranchListSync(BranchListSync {
        branches: branch_msgs,
    }))
}

#[tauri::command]
pub fn start_git_dir_watching(
    app: AppHandle,
    state: State<'_, FileWatcherManager>,
    repo_path: String,
) -> Result<u64, String> {
    let watcher_id = generate_watcher_id();

    // composition root（lib.rs）で組み立てた repository usecase を AppState から
    // 受け取って再利用する（controller 配線を watcher へ漏らさない）。
    let usecase = app.state::<AppState>().repository_usecase.clone();

    let paths = resolve_git_watch_paths(&usecase, &repo_path)?;

    if !paths.refs_heads.exists() {
        return Err(format!(
            "refs/heads not found: {}",
            paths.refs_heads.display()
        ));
    }

    let app_clone = app.clone();
    let main_repo_clone = paths.main_repo.clone();
    let repo_path_clone = repo_path;
    let usecase_for_events: Arc<RepositoryUsecase> = usecase;

    let debouncer = new_debouncer(
        Duration::from_millis(500),
        move |res: Result<
            Vec<notify_debouncer_mini::DebouncedEvent>,
            notify_debouncer_mini::notify::Error,
        >| {
            let events = match res {
                Ok(events) => events,
                Err(e) => {
                    eprintln!("Git dir watcher error: {:?}", e);
                    return;
                }
            };
            let (has_branch_change, has_index_change) = classify_git_dir_events(&events);

            if has_branch_change {
                if let Some(sync_msg) =
                    build_branch_list_sync(&usecase_for_events, &main_repo_clone)
                {
                    let _ = app_clone.emit("branch-list-sync", ());
                    if let Some(ws) = app_clone.try_state::<std::sync::Arc<WsBroadcaster>>() {
                        ws.try_send(sync_msg);
                    }
                }
            }

            if has_index_change || has_branch_change {
                let _ = app_clone.emit(
                    "git-status-changed",
                    GitStatusChangedEvent {
                        repo_path: repo_path_clone.clone(),
                    },
                );
            }
        },
    )
    .map_err(|e| format!("Failed to create debouncer: {e}"))?;

    let mut debouncer = debouncer;
    debouncer
        .watcher()
        .watch(&paths.refs_heads, RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to watch refs/heads: {e}"))?;
    if paths.head_file.exists() {
        debouncer
            .watcher()
            .watch(&paths.head_file, RecursiveMode::NonRecursive)
            .map_err(|e| format!("Failed to watch HEAD: {e}"))?;
    }
    if let Some(git_dir) = paths.index_file.parent() {
        debouncer
            .watcher()
            .watch(git_dir, RecursiveMode::NonRecursive)
            .map_err(|e| format!("Failed to watch .git dir: {e}"))?;
    }
    if paths.worktrees_dir.exists() {
        debouncer
            .watcher()
            .watch(&paths.worktrees_dir, RecursiveMode::Recursive)
            .map_err(|e| format!("Failed to watch worktrees: {e}"))?;
    }

    let session = WatcherSession {
        _debouncer: debouncer,
    };
    state.sessions.lock().insert(watcher_id, session);

    Ok(watcher_id)
}

#[tauri::command]
pub fn stop_watching(state: State<'_, FileWatcherManager>, watcher_id: u64) -> Result<(), String> {
    let mut sessions = state.sessions.lock();
    sessions
        .remove(&watcher_id)
        .ok_or_else(|| format!("Watcher {} not found", watcher_id))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_helpers::*;
    use notify_debouncer_mini::DebouncedEvent;
    use std::path::PathBuf;

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
        assert!(result.is_some());
        let canonical = result.unwrap();
        assert!(canonical.ends_with("test.txt"));
        assert!(!canonical.contains(".."));
    }

    #[test]
    fn canonicalize_deleted_file_falls_back_to_parent() {
        let dir = tempfile::TempDir::new().unwrap();
        let file_path = dir.path().join("deleted.txt");

        let result = canonicalize_event_path(&file_path);
        assert!(result.is_some());
        let canonical = result.unwrap();
        assert!(canonical.ends_with("deleted.txt"));
    }

    #[test]
    fn canonicalize_nonexistent_parent_returns_none() {
        let path = PathBuf::from("/nonexistent/parent/file.txt");
        let result = canonicalize_event_path(&path);
        assert!(result.is_none());
    }
}
