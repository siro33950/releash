use std::sync::Arc;

use tauri::Emitter;
use tokio::sync::Mutex;

use crate::protocol::*;
use crate::ws_bridge::WsBroadcaster;

use super::validation::{validate_patch_paths, validate_relative_path};
use super::WsServerState;

pub(super) fn git_status_to_msg_list(repo_path: &str) -> Vec<GitFileStatusMsg> {
    crate::git::get_git_status(repo_path.to_string())
        .unwrap_or_default()
        .into_iter()
        .map(|s| GitFileStatusMsg {
            path: s.path,
            index_status: s.index_status,
            worktree_status: s.worktree_status,
        })
        .collect()
}

pub(super) fn no_repo_error() -> WsMessage {
    WsMessage::Error(ErrorMsg {
        code: "NO_REPO".to_string(),
        message: "リポジトリパスが設定されていません".to_string(),
    })
}

pub(super) fn no_worktree_selected_error() -> WsMessage {
    WsMessage::Error(ErrorMsg {
        code: "NO_WORKTREE_SELECTED".to_string(),
        message: "Worktreeが選択されていません".to_string(),
    })
}

pub(super) fn join_error_msg(e: tokio::task::JoinError) -> WsMessage {
    WsMessage::Error(ErrorMsg {
        code: "INTERNAL_ERROR".to_string(),
        message: format!("Task join error: {e}"),
    })
}

pub(super) async fn with_worktree_blocking<F>(
    selected_worktree: &Arc<Mutex<Option<String>>>,
    f: F,
) -> Option<WsMessage>
where
    F: FnOnce(String) -> WsMessage + Send + 'static,
{
    let repo_path = {
        let wt = selected_worktree.lock().await;
        match wt.as_ref() {
            Some(p) => p.clone(),
            None => return Some(no_worktree_selected_error()),
        }
    };
    match tokio::task::spawn_blocking(move || f(repo_path)).await {
        Ok(msg) => Some(msg),
        Err(e) => Some(join_error_msg(e)),
    }
}

pub(super) fn broadcast_git_status_sync(
    broadcaster: &WsBroadcaster,
    repo_path: &str,
) -> Vec<GitFileStatusMsg> {
    let files = git_status_to_msg_list(repo_path);
    broadcaster.try_send(WsMessage::GitStatusSync(GitStatusSync {
        files: files.clone(),
    }));
    files
}

fn handle_file_content_request(req: &FileContentRequest, repo_path: &str) -> WsMessage {
    let validated_path = match validate_relative_path(&req.path, repo_path) {
        Ok(p) => p,
        Err(e) => {
            return WsMessage::Error(ErrorMsg {
                code: "INVALID_PATH".to_string(),
                message: e,
            });
        }
    };

    let absolute_path = validated_path.to_string_lossy().to_string();
    let original = if req.diff_base == "staged" {
        crate::git::get_staged_content(absolute_path.clone()).unwrap_or_default()
    } else {
        crate::git::get_file_at_ref(absolute_path.clone(), "HEAD".to_string()).unwrap_or_default()
    };
    let modified = std::fs::read_to_string(&validated_path).unwrap_or_default();
    let staged = if req.diff_base == "HEAD" {
        Some(crate::git::get_staged_content(absolute_path).unwrap_or_default())
    } else {
        None
    };

    WsMessage::FileContentResponse(FileContentResponse {
        path: req.path.clone(),
        original,
        modified,
        staged,
    })
}

fn handle_git_stage_unstage(
    repo_path: &str,
    paths: &[String],
    is_stage: bool,
    broadcaster: &WsBroadcaster,
) -> WsMessage {
    let root = std::path::Path::new(repo_path)
        .canonicalize()
        .map_err(|e| e.to_string());
    let root = match root {
        Ok(r) => r,
        Err(e) => {
            return WsMessage::GitStageResult(GitStageResult {
                success: false,
                error: Some(e),
                files: vec![],
            });
        }
    };

    let mut validated_paths = Vec::with_capacity(paths.len());
    for path in paths {
        match validate_relative_path(path, repo_path) {
            Ok(canonical) => {
                let relative = canonical
                    .strip_prefix(&root)
                    .map_err(|e| e.to_string())
                    .and_then(|r| {
                        r.to_str()
                            .map(|s| s.to_string())
                            .ok_or_else(|| "非UTF-8パス".to_string())
                    });
                match relative {
                    Ok(r) => validated_paths.push(r),
                    Err(e) => {
                        return WsMessage::GitStageResult(GitStageResult {
                            success: false,
                            error: Some(e),
                            files: vec![],
                        });
                    }
                }
            }
            Err(e) => {
                return WsMessage::GitStageResult(GitStageResult {
                    success: false,
                    error: Some(e),
                    files: vec![],
                });
            }
        }
    }

    let result = if is_stage {
        crate::git::git_stage(repo_path.to_string(), validated_paths)
    } else {
        crate::git::git_unstage(repo_path.to_string(), validated_paths)
    };

    if let Err(e) = result {
        return WsMessage::GitStageResult(GitStageResult {
            success: false,
            error: Some(e.to_string()),
            files: vec![],
        });
    }

    let files = broadcast_git_status_sync(broadcaster, repo_path);

    WsMessage::GitStageResult(GitStageResult {
        success: true,
        error: None,
        files,
    })
}

pub(super) fn handle_pty_input(input: &PtyInput, state: &WsServerState) -> Option<WsMessage> {
    if let Some(pm) = &state.pty_manager {
        if let Err(e) = pm.write(input.pty_id, &input.data) {
            return Some(WsMessage::Error(ErrorMsg {
                code: "PTY_WRITE_ERROR".to_string(),
                message: e,
            }));
        }
    }
    None
}

pub(super) async fn handle_git_status_request(
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    with_worktree_blocking(selected_worktree, |repo_path| {
        let files = git_status_to_msg_list(&repo_path);
        WsMessage::GitStatusSync(GitStatusSync { files })
    })
    .await
}

pub(super) async fn handle_file_content_req(
    req: &FileContentRequest,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let req = req.clone();
    with_worktree_blocking(selected_worktree, move |repo_path| {
        handle_file_content_request(&req, &repo_path)
    })
    .await
}

pub(super) async fn handle_git_stage_request(
    req: &GitStage,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let paths = req.paths.clone();
    let broadcaster = state.broadcaster.clone();
    with_worktree_blocking(selected_worktree, move |repo_path| {
        handle_git_stage_unstage(&repo_path, &paths, true, &broadcaster)
    })
    .await
}

pub(super) async fn handle_git_unstage_request(
    req: &GitUnstage,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let paths = req.paths.clone();
    let broadcaster = state.broadcaster.clone();
    with_worktree_blocking(selected_worktree, move |repo_path| {
        handle_git_stage_unstage(&repo_path, &paths, false, &broadcaster)
    })
    .await
}

pub(super) async fn handle_git_stage_hunk_request(
    req: &GitStageHunk,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let patch = req.patch.clone();
    let broadcaster = state.broadcaster.clone();
    with_worktree_blocking(selected_worktree, move |repo_path| {
        if let Err(e) = validate_patch_paths(&patch, &repo_path) {
            return WsMessage::Error(ErrorMsg {
                code: "INVALID_PATH".to_string(),
                message: e,
            });
        }
        let result = crate::git::git_stage_hunk(repo_path.clone(), patch);
        let files = broadcast_git_status_sync(&broadcaster, &repo_path);
        WsMessage::GitStageResult(GitStageResult {
            success: result.is_ok(),
            error: result.err().map(|e| e.to_string()),
            files,
        })
    })
    .await
}

pub(super) async fn handle_pty_spawn_request(
    req: &PtySpawnRequest,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_path = {
        let wt = selected_worktree.lock().await;
        match wt.as_ref() {
            Some(p) => p.clone(),
            None => return Some(no_worktree_selected_error()),
        }
    };
    let (pm, app) = match (&state.pty_manager, &state.app_handle) {
        (Some(pm), Some(app)) => (Arc::clone(pm), app.clone()),
        _ => {
            return Some(WsMessage::PtySpawnResponse(PtySpawnResponse {
                success: false,
                pty_id: None,
                error: Some("PTY manager が利用できません".to_string()),
            }))
        }
    };

    let rows = req.rows;
    let cols = req.cols;
    let broadcaster = state.broadcaster.clone();
    match tokio::task::spawn_blocking(move || {
        pm.spawn(
            &app,
            rows,
            cols,
            Some(worktree_path.clone()),
            Some(worktree_path),
        )
    })
    .await
    {
        Ok(Ok(pty_id)) => {
            broadcaster.try_send(WsMessage::PtyReady(PtyReady { pty_id, cols, rows }));
            Some(WsMessage::PtySpawnResponse(PtySpawnResponse {
                success: true,
                pty_id: Some(pty_id),
                error: None,
            }))
        }
        Ok(Err(e)) => Some(WsMessage::PtySpawnResponse(PtySpawnResponse {
            success: false,
            pty_id: None,
            error: Some(e),
        })),
        Err(e) => Some(join_error_msg(e)),
    }
}

pub(super) fn handle_pty_output_request(
    req: &PtyOutputRequest,
    state: &WsServerState,
) -> Option<WsMessage> {
    if let Some(pm) = &state.pty_manager {
        let sessions = pm.list_pty_sessions();
        if sessions.iter().any(|s| s.pty_id == req.pty_id) {
            let buffered = state.broadcaster.get_pty_output_buffer(req.pty_id);
            if !buffered.is_empty() {
                state
                    .broadcaster
                    .send_without_buffer(WsMessage::PtyOutput(PtyOutputMsg {
                        pty_id: req.pty_id,
                        data: buffered,
                    }));
            }
            None
        } else {
            Some(WsMessage::Error(ErrorMsg {
                code: "PTY_NOT_FOUND".to_string(),
                message: format!("PTY {} が見つかりません", req.pty_id),
            }))
        }
    } else {
        Some(WsMessage::Error(ErrorMsg {
            code: "NO_PTY".to_string(),
            message: "デスクトップのターミナルがまだ起動していません".to_string(),
        }))
    }
}

pub(super) async fn handle_git_commit_request(
    req: &GitCommitRequest,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let message = req.message.clone();
    let broadcaster = state.broadcaster.clone();
    with_worktree_blocking(
        selected_worktree,
        move |repo_path| match crate::git::git_commit(repo_path.clone(), message) {
            Ok(hash) => {
                broadcast_git_status_sync(&broadcaster, &repo_path);
                let branch = crate::git::get_current_branch(repo_path.clone()).unwrap_or_default();
                broadcaster.try_send(WsMessage::BranchInfoResponse(BranchInfoResponse { branch }));
                if let Ok(cards) = crate::git::list_branches_with_status(repo_path) {
                    let branch_msgs = cards.into_iter().map(BranchCardMsg::from).collect();
                    broadcaster.try_send(WsMessage::BranchListSync(BranchListSync {
                        branches: branch_msgs,
                    }));
                }
                WsMessage::GitCommitResult(GitCommitResult {
                    success: true,
                    hash: Some(hash),
                    error: None,
                })
            }
            Err(e) => WsMessage::GitCommitResult(GitCommitResult {
                success: false,
                hash: None,
                error: Some(e.to_string()),
            }),
        },
    )
    .await
}

pub(super) async fn handle_git_push_request(
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    with_worktree_blocking(selected_worktree, |repo_path| {
        match crate::git::git_push(repo_path) {
            Ok(output) => WsMessage::GitPushResult(GitPushResult {
                success: true,
                output: Some(output),
                error: None,
            }),
            Err(e) => WsMessage::GitPushResult(GitPushResult {
                success: false,
                output: None,
                error: Some(e.to_string()),
            }),
        }
    })
    .await
}

pub(super) async fn handle_branch_info_request(
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    with_worktree_blocking(selected_worktree, |repo_path| {
        let branch = crate::git::get_current_branch(repo_path).unwrap_or_default();
        WsMessage::BranchInfoResponse(BranchInfoResponse { branch })
    })
    .await
}

pub(crate) async fn build_all_worktrees(state: &WsServerState) -> Vec<WorktreeEntryMsg> {
    let repo_paths = state.get_repo_paths();
    let pr_cache = state.pr_cache.clone();
    tokio::task::spawn_blocking(move || {
        let mut all_entries = Vec::new();
        for repo_path in &repo_paths {
            let pr_status = crate::git_host::fetch_pr_status_with_cache(&pr_cache, repo_path);
            let entries = crate::git::list_worktrees(repo_path.clone()).unwrap_or_default();
            for e in entries {
                let pr = pr_status.open_prs.get(&e.branch);
                all_entries.push(WorktreeEntryMsg {
                    name: e.name,
                    path: e.path,
                    branch: e.branch,
                    is_main: e.is_main,
                    is_locked: e.is_locked,
                    dirty_count: e.dirty_count,
                    base_branch: e.base_branch,
                    repo_path: Some(repo_path.clone()),
                    has_pr: pr.is_some(),
                    pr_number: pr.map(|p| p.number),
                    pr_url: pr.map(|p| p.url.clone()),
                });
            }
        }
        all_entries
    })
    .await
    .unwrap_or_default()
}

pub(super) async fn handle_worktree_list_request(state: &WsServerState) -> Option<WsMessage> {
    let repo_paths = state.get_repo_paths();
    if repo_paths.is_empty() {
        return Some(no_repo_error());
    }
    let worktrees = build_all_worktrees(state).await;
    Some(WsMessage::WorktreeListResponse(WorktreeListResponse {
        worktrees,
    }))
}

pub(super) async fn handle_worktree_select_request(
    req: &WorktreeSelectRequest,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let repo_paths = state.get_repo_paths();
    if repo_paths.is_empty() {
        return Some(no_repo_error());
    }
    let requested_path = req.path.clone();
    let broadcaster = state.broadcaster.clone();

    let valid = tokio::task::spawn_blocking({
        let requested_path = requested_path.clone();
        move || {
            for repo_path in &repo_paths {
                let worktrees = crate::git::list_worktrees(repo_path.clone()).unwrap_or_default();
                if worktrees.iter().any(|w| w.path == requested_path) {
                    return true;
                }
            }
            false
        }
    })
    .await
    .unwrap_or(false);

    if !valid {
        return Some(WsMessage::WorktreeSelectResponse(WorktreeSelectResponse {
            success: false,
            path: requested_path,
            error: Some("指定されたworktreeが見つかりません".to_string()),
        }));
    }

    {
        let mut wt = selected_worktree.lock().await;
        *wt = Some(requested_path.clone());
    }

    let wt_path = requested_path.clone();
    let wt_path2 = requested_path.clone();
    if let Ok(files) = tokio::task::spawn_blocking(move || git_status_to_msg_list(&wt_path)).await {
        broadcaster.try_send(WsMessage::GitStatusSync(GitStatusSync { files }));
    }
    if let Ok(branch) = tokio::task::spawn_blocking(move || {
        crate::git::get_current_branch(wt_path2).unwrap_or_default()
    })
    .await
    {
        broadcaster.try_send(WsMessage::BranchInfoResponse(BranchInfoResponse { branch }));
    }

    broadcaster.try_send(WsMessage::WorktreeSelectResponse(WorktreeSelectResponse {
        success: true,
        path: requested_path.clone(),
        error: None,
    }));

    if let Some(pm) = &state.pty_manager {
        for session in pm.list_pty_sessions() {
            if session.worktree_path.as_deref() == Some(&requested_path) {
                let (cols, rows) = pm.get_pty_size(session.pty_id).unwrap_or((80, 24));
                broadcaster.try_send(WsMessage::PtyReady(PtyReady {
                    pty_id: session.pty_id,
                    cols,
                    rows,
                }));
            }
        }
    }

    None
}

pub(super) fn handle_add_comment(comment: &AddComment, state: &WsServerState) -> Option<WsMessage> {
    if let Some(app) = &state.app_handle {
        let _ = app.emit("remote-comment-added", comment);
    }
    None
}

pub(super) fn handle_delete_comment(
    req: &DeleteComment,
    state: &WsServerState,
) -> Option<WsMessage> {
    if let Some(app) = &state.app_handle {
        let _ = app.emit("remote-comment-deleted", req);
    }
    None
}

pub(super) fn handle_update_comment(
    req: &UpdateComment,
    state: &WsServerState,
) -> Option<WsMessage> {
    if let Some(app) = &state.app_handle {
        let _ = app.emit("remote-comment-updated", req);
    }
    None
}
