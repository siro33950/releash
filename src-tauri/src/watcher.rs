use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::protocol::{
    BranchCardMsg, BranchListSync, FileChange, GitFileStatusMsg, GitStatusSync, WsMessage,
};
use crate::ws_bridge::WsBroadcaster;

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
    let watch_path_str = path.clone();

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
                        let event_path = event.path.to_string_lossy().to_string();
                        let _ = app_clone.emit(
                            "file-change",
                            FileChangeEvent {
                                watcher_id: watcher_id_clone,
                                path: event_path.clone(),
                                kind: kind.to_string(),
                            },
                        );

                        if let Some(ws) = app_clone.try_state::<std::sync::Arc<WsBroadcaster>>() {
                            ws.try_send(WsMessage::FileChange(FileChange {
                                path: event_path,
                                kind: kind.to_string(),
                            }));

                            if let Ok(statuses) = crate::git::get_git_status(watch_path_str.clone())
                            {
                                let files = statuses
                                    .into_iter()
                                    .map(|s| GitFileStatusMsg {
                                        path: s.path,
                                        index_status: s.index_status,
                                        worktree_status: s.worktree_status,
                                    })
                                    .collect();
                                ws.try_send(WsMessage::GitStatusSync(GitStatusSync { files }));
                            }
                        }
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

fn build_branch_list_sync(repo_path: &str) -> Option<WsMessage> {
    let branches = crate::git::list_branches_with_status(repo_path.to_string()).ok()?;
    let branch_msgs: Vec<BranchCardMsg> = branches
        .into_iter()
        .map(|b| BranchCardMsg {
            name: b.name,
            is_default: b.is_default,
            worktree_path: b.worktree_path,
            dirty_count: b.dirty_count,
            is_merged: b.is_merged,
            has_pr: b.has_pr,
            pr_number: b.pr_number,
            pr_url: b.pr_url,
        })
        .collect();
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

    let main_repo = crate::git::get_main_repo_path(repo_path)
        .map_err(|e| format!("Failed to resolve main repo: {e}"))?;
    let git_dir = git2::Repository::open(&main_repo)
        .map_err(|e| format!("Failed to open repo: {e}"))?
        .path()
        .to_path_buf();

    let refs_heads = git_dir.join("refs").join("heads");
    let head_file = git_dir.join("HEAD");

    if !refs_heads.exists() {
        return Err(format!("refs/heads not found: {}", refs_heads.display()));
    }

    let app_clone = app.clone();
    let main_repo_clone = main_repo.clone();

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
            let dominated_by_git = events.iter().any(|e| {
                let p = e.path.to_string_lossy();
                p.contains("refs/heads") || p.ends_with("HEAD")
            });
            if dominated_by_git {
                if let Some(sync_msg) = build_branch_list_sync(&main_repo_clone) {
                    let _ = app_clone.emit("branch-list-sync", ());
                    if let Some(ws) = app_clone.try_state::<std::sync::Arc<WsBroadcaster>>() {
                        ws.try_send(sync_msg);
                    }
                }
            }
        },
    )
    .map_err(|e| format!("Failed to create debouncer: {e}"))?;

    let mut debouncer = debouncer;
    debouncer
        .watcher()
        .watch(&refs_heads, RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to watch refs/heads: {e}"))?;
    if head_file.exists() {
        debouncer
            .watcher()
            .watch(&head_file, RecursiveMode::NonRecursive)
            .map_err(|e| format!("Failed to watch HEAD: {e}"))?;
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
