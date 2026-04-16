use std::sync::Arc;

use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

use crate::protocol::thread::{Thread, ThreadEntry, ThreadsSync};
use crate::protocol::*;
use crate::thread_store::ThreadsChangedPayload;
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
        crate::git::get_file_at_branch_base(absolute_path.clone()).unwrap_or_default()
    };
    let modified = std::fs::read_to_string(&validated_path).unwrap_or_default();
    let staged = if req.diff_base != "staged" {
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
    let label = req.label.clone();
    let broadcaster = state.broadcaster.clone();
    let wt_path_for_ready = worktree_path.clone();
    let label_for_ready = label.clone();
    match tokio::task::spawn_blocking(move || {
        pm.spawn(
            &app,
            rows,
            cols,
            Some(worktree_path.clone()),
            Some(worktree_path),
            label,
            crate::pty::PtyKind::Terminal,
        )
    })
    .await
    {
        Ok(Ok((pty_id, _session_key))) => {
            broadcaster.try_send(WsMessage::PtyReady(PtyReady {
                pty_id,
                cols,
                rows,
                label: label_for_ready,
                worktree_path: Some(wt_path_for_ready),
                kind: Some(crate::pty::PtyKind::Terminal),
            }));
            let startup_cmd = state.get_terminal_startup_command();
            let trimmed_cmd = startup_cmd.trim();
            if !trimmed_cmd.is_empty() {
                if let Some(pm) = &state.pty_manager {
                    let data = format!("{}\n", trimmed_cmd);
                    if let Err(e) = pm.write(pty_id, &data) {
                        log::warn!("Failed to write startup command to PTY {}: {}", pty_id, e);
                    }
                }
            }
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
                    label: session.label.clone(),
                    worktree_path: session.worktree_path.clone(),
                    kind: Some(session.kind),
                }));
            }
        }
    }

    // Send initial threads for the selected worktree, merged with PR threads
    let local_threads = state.thread_store.get_all(&requested_path);
    let worktree_name = requested_path.clone();

    // Fetch PR review comments and cache as threads
    let pr_cache = state.pr_cache.clone();
    let wt_for_pr = requested_path.clone();
    let pr_threads = tokio::task::spawn_blocking(move || {
        let branch = crate::git::get_current_branch(wt_for_pr.clone()).unwrap_or_default();
        if branch.is_empty() {
            return Vec::new();
        }
        let pr_status = crate::git_host::fetch_pr_status_with_cache(&pr_cache, &wt_for_pr);
        let pr_number = match pr_status.open_prs.get(&branch) {
            Some(pr) => pr.number,
            None => return Vec::new(),
        };
        let comments = crate::git_host::fetch_pr_review_comments_inner(&wt_for_pr, pr_number);
        crate::git_host::pr_review_comments_to_threads(comments)
    })
    .await
    .unwrap_or_default();

    // Cache and merge
    {
        let mut cache = state.pr_threads_cache.write();
        cache.insert(worktree_name, pr_threads.clone());
    }

    let local_ids: std::collections::HashSet<String> =
        local_threads.iter().map(|t| t.id.clone()).collect();
    let mut merged = local_threads;
    for t in pr_threads {
        if !local_ids.contains(&t.id) {
            merged.push(t);
        }
    }
    broadcaster.try_send(WsMessage::ThreadsSync(ThreadsSync { threads: merged }));

    None
}

pub(super) async fn handle_pty_kill_request(
    req: &PtyKillRequest,
    state: &WsServerState,
) -> Option<WsMessage> {
    let pty_id = req.pty_id;
    if let Some(pm) = &state.pty_manager {
        let pm = Arc::clone(pm);
        match tokio::task::spawn_blocking(move || pm.kill(pty_id)).await {
            Ok(Ok(())) => {
                state.broadcaster.remove_pty_output_buffer(pty_id);
                Some(WsMessage::PtyKillResponse(PtyKillResponse {
                    success: true,
                    pty_id,
                    error: None,
                }))
            }
            Ok(Err(e)) => Some(WsMessage::PtyKillResponse(PtyKillResponse {
                success: false,
                pty_id,
                error: Some(e),
            })),
            Err(e) => Some(join_error_msg(e)),
        }
    } else {
        Some(WsMessage::PtyKillResponse(PtyKillResponse {
            success: false,
            pty_id,
            error: Some("PTY manager が利用できません".to_string()),
        }))
    }
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

// --- Thread handlers ---

fn thread_persist_emit_broadcast(state: &WsServerState, worktree_name: &str) -> Result<(), String> {
    if let Some(app) = &state.app_handle {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|e| format!("Failed to get app data dir: {e}"))?;
        state.thread_store.save(&data_dir, worktree_name)?;
        let local_threads = state.thread_store.get_all(worktree_name);
        // Emit local-only threads to desktop (desktop merges PR threads on its own)
        let _ = app.emit(
            "threads-changed",
            ThreadsChangedPayload {
                worktree_name: worktree_name.to_string(),
                source: "remote".to_string(),
                threads: local_threads.clone(),
            },
        );
    }

    // Merge cached PR threads for WebSocket broadcast (remote needs them)
    // Broadcast outside app_handle block so WS clients are notified even without desktop
    let local_threads = state.thread_store.get_all(worktree_name);
    let mut merged = local_threads;
    if let Some(pr_threads) = state.pr_threads_cache.read().get(worktree_name) {
        let local_ids: std::collections::HashSet<String> =
            merged.iter().map(|t| t.id.clone()).collect();
        for t in pr_threads {
            if !local_ids.contains(&t.id) {
                merged.push(t.clone());
            }
        }
    }
    state
        .broadcaster
        .try_send(WsMessage::ThreadsSync(ThreadsSync { threads: merged }));
    Ok(())
}

pub(super) async fn handle_create_thread(
    req: &CreateThread,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_name = {
        let wt = selected_worktree.lock().await;
        match wt.as_deref() {
            Some(name) => name.to_string(),
            None => return Some(no_worktree_selected_error()),
        }
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0;

    let thread = Thread {
        id: uuid::Uuid::new_v4().to_string(),
        file_path: req.file_path.clone(),
        line_number: req.line_number,
        end_line: req.end_line,
        entries: vec![ThreadEntry {
            id: uuid::Uuid::new_v4().to_string(),
            content: req.content.clone(),
            action: None,
            author_name: req.author_name.clone(),
            author_avatar_url: None,
            pr_comment_id: None,
            created_at: now,
        }],
        resolved: false,
        severity: req.severity.clone(),
        anchor: None,
        created_at: now,
    };

    if let Err(e) = state.thread_store.add_thread(&worktree_name, thread) {
        return Some(WsMessage::Error(ErrorMsg {
            code: "INVALID_PATH".to_string(),
            message: e,
        }));
    }
    if let Err(e) = thread_persist_emit_broadcast(state, &worktree_name) {
        return Some(WsMessage::Error(ErrorMsg {
            code: "PERSIST_ERROR".to_string(),
            message: e,
        }));
    }
    None
}

pub(super) async fn handle_add_thread_entry(
    req: &AddThreadEntry,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_name = {
        let wt = selected_worktree.lock().await;
        match wt.as_deref() {
            Some(name) => name.to_string(),
            None => return Some(no_worktree_selected_error()),
        }
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0;

    let entry = ThreadEntry {
        id: uuid::Uuid::new_v4().to_string(),
        content: req.content.clone(),
        action: None,
        author_name: req.author_name.clone(),
        author_avatar_url: None,
        pr_comment_id: None,
        created_at: now,
    };

    if !state
        .thread_store
        .add_entry(&worktree_name, &req.thread_id, entry)
    {
        return Some(WsMessage::Error(ErrorMsg {
            code: "THREAD_NOT_FOUND".to_string(),
            message: format!("Thread not found: {}", req.thread_id),
        }));
    }
    if let Err(e) = thread_persist_emit_broadcast(state, &worktree_name) {
        return Some(WsMessage::Error(ErrorMsg {
            code: "PERSIST_ERROR".to_string(),
            message: e,
        }));
    }
    None
}

pub(super) async fn handle_resolve_thread(
    req: &ResolveThread,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_name = {
        let wt = selected_worktree.lock().await;
        match wt.as_deref() {
            Some(name) => name.to_string(),
            None => return Some(no_worktree_selected_error()),
        }
    };

    if state
        .thread_store
        .resolve_thread(&worktree_name, &req.thread_id)
        .is_none()
    {
        return Some(WsMessage::Error(ErrorMsg {
            code: "THREAD_NOT_FOUND".to_string(),
            message: format!("Thread not found: {}", req.thread_id),
        }));
    }
    if let Err(e) = thread_persist_emit_broadcast(state, &worktree_name) {
        return Some(WsMessage::Error(ErrorMsg {
            code: "PERSIST_ERROR".to_string(),
            message: e,
        }));
    }
    None
}

pub(super) async fn handle_delete_thread(
    req: &DeleteThread,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_name = {
        let wt = selected_worktree.lock().await;
        match wt.as_deref() {
            Some(name) => name.to_string(),
            None => return Some(no_worktree_selected_error()),
        }
    };

    if !state
        .thread_store
        .remove_thread(&worktree_name, &req.thread_id)
    {
        return Some(WsMessage::Error(ErrorMsg {
            code: "THREAD_NOT_FOUND".to_string(),
            message: format!("Thread not found: {}", req.thread_id),
        }));
    }
    if let Err(e) = thread_persist_emit_broadcast(state, &worktree_name) {
        return Some(WsMessage::Error(ErrorMsg {
            code: "PERSIST_ERROR".to_string(),
            message: e,
        }));
    }
    None
}

pub(super) async fn handle_update_thread_entry(
    req: &UpdateThreadEntry,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_name = {
        let wt = selected_worktree.lock().await;
        match wt.as_deref() {
            Some(name) => name.to_string(),
            None => return Some(no_worktree_selected_error()),
        }
    };

    if !state
        .thread_store
        .update_entry(&worktree_name, &req.thread_id, &req.entry_id, &req.content)
    {
        return Some(WsMessage::Error(ErrorMsg {
            code: "THREAD_NOT_FOUND".to_string(),
            message: format!("Entry not found: {}/{}", req.thread_id, req.entry_id),
        }));
    }
    if let Err(e) = thread_persist_emit_broadcast(state, &worktree_name) {
        return Some(WsMessage::Error(ErrorMsg {
            code: "PERSIST_ERROR".to_string(),
            message: e,
        }));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_helpers::{add_and_commit, create_initial_commit, create_test_repo};
    use tempfile::TempDir;

    fn make_state(repo_paths: Vec<String>) -> WsServerState {
        let config = crate::config::ReleashConfig::default();
        let app_config = std::sync::Arc::new(crate::config::AppConfig::new(
            config,
            std::path::PathBuf::from("/tmp/test-releash.toml"),
        ));
        WsServerState::new(
            None,
            std::sync::Arc::new(WsBroadcaster::default()),
            None,
            std::sync::Arc::new(parking_lot::RwLock::new(repo_paths)),
            app_config,
            None,
            false,
            std::sync::Arc::new(crate::git_host::PrCache::new()),
            std::sync::Arc::new(crate::thread_store::ThreadStore::default()),
        )
    }

    fn make_selected(path: Option<String>) -> Arc<Mutex<Option<String>>> {
        Arc::new(Mutex::new(path))
    }

    fn setup_repo_with_file(name: &str, content: &str) -> (TempDir, String) {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        add_and_commit(&repo, name, content, &format!("add {name}"));
        let repo_path = dir
            .path()
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        (dir, repo_path)
    }

    // --- A. ユーティリティ ---

    #[test]
    fn test_no_repo_error() {
        let msg = no_repo_error();
        match msg {
            WsMessage::Error(e) => {
                assert_eq!(e.code, "NO_REPO");
                assert!(!e.message.is_empty());
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[test]
    fn test_no_worktree_selected_error() {
        let msg = no_worktree_selected_error();
        match msg {
            WsMessage::Error(e) => {
                assert_eq!(e.code, "NO_WORKTREE_SELECTED");
                assert!(!e.message.is_empty());
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[test]
    fn test_join_error_msg() {
        let handle = tokio::runtime::Runtime::new().unwrap();
        let err = handle.block_on(async {
            let h = tokio::task::spawn_blocking(|| {
                panic!("test panic");
            });
            h.await.unwrap_err()
        });
        let msg = join_error_msg(err);
        match msg {
            WsMessage::Error(e) => {
                assert_eq!(e.code, "INTERNAL_ERROR");
                assert!(e.message.contains("Task join error"));
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[test]
    fn test_git_status_to_msg_list_nonexistent_repo() {
        let files = git_status_to_msg_list("/nonexistent/repo/path");
        assert!(files.is_empty());
    }

    // --- B. with_worktree_blocking ---

    #[tokio::test]
    async fn test_with_worktree_blocking_none() {
        let selected = make_selected(None);
        let result = with_worktree_blocking(&selected, |_| {
            WsMessage::Error(ErrorMsg {
                code: "SHOULD_NOT_REACH".to_string(),
                message: String::new(),
            })
        })
        .await;
        let msg = result.unwrap();
        match msg {
            WsMessage::Error(e) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("Expected NO_WORKTREE_SELECTED error"),
        }
    }

    #[tokio::test]
    async fn test_with_worktree_blocking_some() {
        let selected = make_selected(Some("/test/repo".to_string()));
        let result = with_worktree_blocking(&selected, |path| {
            WsMessage::BranchInfoResponse(BranchInfoResponse { branch: path })
        })
        .await;
        let msg = result.unwrap();
        match msg {
            WsMessage::BranchInfoResponse(r) => assert_eq!(r.branch, "/test/repo"),
            _ => panic!("Expected BranchInfoResponse"),
        }
    }

    // --- C. git_status_to_msg_list / broadcast_git_status_sync ---

    #[test]
    fn test_git_status_to_msg_list_with_real_repo() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();
        std::fs::write(dir.path().join("untracked.txt"), "hello").unwrap();

        let files = git_status_to_msg_list(repo_path);
        assert!(!files.is_empty());
        let untracked = files.iter().find(|f| f.path == "untracked.txt");
        assert!(untracked.is_some());
    }

    #[test]
    fn test_broadcast_git_status_sync() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();
        std::fs::write(dir.path().join("new.txt"), "data").unwrap();

        let broadcaster = std::sync::Arc::new(WsBroadcaster::default());
        let (tx, mut rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));

        let files = broadcast_git_status_sync(&broadcaster, repo_path);
        assert!(!files.is_empty());

        let received = rx.try_recv();
        assert!(received.is_ok());
        match received.unwrap() {
            WsMessage::GitStatusSync(sync) => {
                assert!(!sync.files.is_empty());
            }
            _ => panic!("Expected GitStatusSync"),
        }
    }

    // --- D. handle_git_status_request ---

    #[tokio::test]
    async fn test_handle_git_status_request_no_worktree() {
        let selected = make_selected(None);
        let result = handle_git_status_request(&selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::Error(e) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("Expected NO_WORKTREE_SELECTED"),
        }
    }

    #[tokio::test]
    async fn test_handle_git_status_request_with_repo() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        std::fs::write(dir.path().join("file.txt"), "content").unwrap();
        let repo_path = dir.path().to_str().unwrap().to_string();
        let selected = make_selected(Some(repo_path));

        let result = handle_git_status_request(&selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::GitStatusSync(sync) => {
                assert!(sync.files.iter().any(|f| f.path == "file.txt"));
            }
            _ => panic!("Expected GitStatusSync"),
        }
    }

    // --- E. handle_file_content_request ---

    #[test]
    fn test_handle_file_content_request_head_base() {
        let (_dir, repo_path) = setup_repo_with_file("hello.txt", "original content");
        std::fs::write(
            std::path::Path::new(&repo_path).join("hello.txt"),
            "modified content",
        )
        .unwrap();

        let req = FileContentRequest {
            path: "hello.txt".to_string(),
            diff_base: "HEAD".to_string(),
        };
        let msg = handle_file_content_request(&req, &repo_path);
        match msg {
            WsMessage::FileContentResponse(r) => {
                assert_eq!(r.path, "hello.txt");
                assert_eq!(r.original, "original content");
                assert_eq!(r.modified, "modified content");
                assert!(r.staged.is_some());
            }
            _ => panic!("Expected FileContentResponse"),
        }
    }

    #[test]
    fn test_handle_file_content_request_staged_base() {
        let (_dir, repo_path) = setup_repo_with_file("hello.txt", "committed");
        let full_path = std::path::Path::new(&repo_path).join("hello.txt");
        std::fs::write(&full_path, "staged content").unwrap();
        {
            let repo = git2::Repository::open(&repo_path).unwrap();
            let mut index = repo.index().unwrap();
            index.add_path(std::path::Path::new("hello.txt")).unwrap();
            index.write().unwrap();
        }
        std::fs::write(&full_path, "working content").unwrap();

        let req = FileContentRequest {
            path: "hello.txt".to_string(),
            diff_base: "staged".to_string(),
        };
        let msg = handle_file_content_request(&req, &repo_path);
        match msg {
            WsMessage::FileContentResponse(r) => {
                assert_eq!(r.original, "staged content");
                assert_eq!(r.modified, "working content");
                assert!(r.staged.is_none());
            }
            _ => panic!("Expected FileContentResponse"),
        }
    }

    #[test]
    fn test_handle_file_content_request_invalid_path() {
        let (_dir, repo_path) = setup_repo_with_file("hello.txt", "content");
        let req = FileContentRequest {
            path: "/etc/passwd".to_string(),
            diff_base: "HEAD".to_string(),
        };
        let msg = handle_file_content_request(&req, &repo_path);
        match msg {
            WsMessage::Error(e) => assert_eq!(e.code, "INVALID_PATH"),
            _ => panic!("Expected INVALID_PATH error"),
        }
    }

    #[tokio::test]
    async fn test_handle_file_content_req_no_worktree() {
        let selected = make_selected(None);
        let req = FileContentRequest {
            path: "file.txt".to_string(),
            diff_base: "HEAD".to_string(),
        };
        let result = handle_file_content_req(&req, &selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::Error(e) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("Expected NO_WORKTREE_SELECTED"),
        }
    }

    // --- F. handle_git_stage_unstage ---

    #[test]
    fn test_handle_git_stage_unstage_stage_success() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();
        std::fs::write(dir.path().join("new.txt"), "data").unwrap();

        let broadcaster = std::sync::Arc::new(WsBroadcaster::default());
        let msg = handle_git_stage_unstage(repo_path, &["new.txt".to_string()], true, &broadcaster);
        match msg {
            WsMessage::GitStageResult(r) => {
                assert!(r.success);
                assert!(r.error.is_none());
            }
            _ => panic!("Expected GitStageResult"),
        }
    }

    #[test]
    fn test_handle_git_stage_unstage_unstage_success() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap();
        std::fs::write(dir.path().join("staged.txt"), "data").unwrap();
        {
            let mut index = repo.index().unwrap();
            index.add_path(std::path::Path::new("staged.txt")).unwrap();
            index.write().unwrap();
        }

        let broadcaster = std::sync::Arc::new(WsBroadcaster::default());
        let msg =
            handle_git_stage_unstage(repo_path, &["staged.txt".to_string()], false, &broadcaster);
        match msg {
            WsMessage::GitStageResult(r) => {
                assert!(r.success);
                assert!(r.error.is_none());
            }
            _ => panic!("Expected GitStageResult"),
        }
    }

    #[test]
    fn test_handle_git_stage_unstage_invalid_path() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let broadcaster = std::sync::Arc::new(WsBroadcaster::default());
        let msg = handle_git_stage_unstage(
            &repo_path,
            &["../../../etc/passwd".to_string()],
            true,
            &broadcaster,
        );
        match msg {
            WsMessage::GitStageResult(r) => {
                assert!(!r.success);
                assert!(r.error.is_some());
            }
            _ => panic!("Expected GitStageResult"),
        }
    }

    #[tokio::test]
    async fn test_handle_git_stage_request_no_worktree() {
        let state = make_state(vec![]);
        let selected = make_selected(None);
        let req = GitStage {
            paths: vec!["file.txt".to_string()],
        };
        let result = handle_git_stage_request(&req, &state, &selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::Error(e) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("Expected NO_WORKTREE_SELECTED"),
        }
    }

    #[tokio::test]
    async fn test_handle_git_unstage_request_no_worktree() {
        let state = make_state(vec![]);
        let selected = make_selected(None);
        let req = GitUnstage {
            paths: vec!["file.txt".to_string()],
        };
        let result = handle_git_unstage_request(&req, &state, &selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::Error(e) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("Expected NO_WORKTREE_SELECTED"),
        }
    }

    // --- G. handle_git_commit_request ---

    #[tokio::test]
    async fn test_handle_git_commit_request_success() {
        let (dir, repo) = create_test_repo();
        create_initial_commit(&repo);
        let repo_path = dir.path().to_str().unwrap().to_string();
        std::fs::write(dir.path().join("commit_me.txt"), "data").unwrap();
        {
            let mut index = repo.index().unwrap();
            index
                .add_path(std::path::Path::new("commit_me.txt"))
                .unwrap();
            index.write().unwrap();
        }

        let state = make_state(vec![repo_path.clone()]);
        let selected = make_selected(Some(repo_path));
        let req = GitCommitRequest {
            message: "test commit".to_string(),
        };
        let result = handle_git_commit_request(&req, &state, &selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::GitCommitResult(r) => {
                assert!(r.success);
                assert!(r.hash.is_some());
                assert!(r.error.is_none());
            }
            _ => panic!("Expected GitCommitResult"),
        }
    }

    #[tokio::test]
    async fn test_handle_git_commit_request_no_worktree() {
        let state = make_state(vec![]);
        let selected = make_selected(None);
        let req = GitCommitRequest {
            message: "test".to_string(),
        };
        let result = handle_git_commit_request(&req, &state, &selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::Error(e) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("Expected NO_WORKTREE_SELECTED"),
        }
    }

    // --- H. handle_git_push_request / handle_branch_info_request ---

    #[tokio::test]
    async fn test_handle_git_push_request_no_worktree() {
        let selected = make_selected(None);
        let result = handle_git_push_request(&selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::Error(e) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("Expected NO_WORKTREE_SELECTED"),
        }
    }

    #[tokio::test]
    async fn test_handle_branch_info_request_no_worktree() {
        let selected = make_selected(None);
        let result = handle_branch_info_request(&selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::Error(e) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("Expected NO_WORKTREE_SELECTED"),
        }
    }

    #[tokio::test]
    async fn test_handle_branch_info_request_with_repo() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let selected = make_selected(Some(repo_path));
        let result = handle_branch_info_request(&selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::BranchInfoResponse(r) => {
                assert!(!r.branch.is_empty());
            }
            _ => panic!("Expected BranchInfoResponse"),
        }
    }

    // --- I. PTYハンドラ ---

    #[test]
    fn test_handle_pty_input_no_manager() {
        let state = make_state(vec![]);
        let input = PtyInput {
            pty_id: 1,
            data: "hello".to_string(),
        };
        let result = handle_pty_input(&input, &state);
        assert!(result.is_none());
    }

    #[test]
    fn test_handle_pty_output_request_no_manager() {
        let state = make_state(vec![]);
        let req = PtyOutputRequest { pty_id: 1 };
        let result = handle_pty_output_request(&req, &state);
        assert!(result.is_some());
        match result.unwrap() {
            WsMessage::Error(e) => {
                assert_eq!(e.code, "NO_PTY");
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[tokio::test]
    async fn test_handle_pty_spawn_request_no_worktree() {
        let state = make_state(vec![]);
        let selected = make_selected(None);
        let req = PtySpawnRequest {
            cols: 80,
            rows: 24,
            label: None,
        };
        let result = handle_pty_spawn_request(&req, &state, &selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::Error(e) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("Expected NO_WORKTREE_SELECTED"),
        }
    }

    #[tokio::test]
    async fn test_handle_pty_spawn_request_no_manager() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let state = make_state(vec![repo_path.clone()]);
        let selected = make_selected(Some(repo_path));
        let req = PtySpawnRequest {
            cols: 80,
            rows: 24,
            label: None,
        };
        let result = handle_pty_spawn_request(&req, &state, &selected).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::PtySpawnResponse(r) => {
                assert!(!r.success);
                assert!(r.error.is_some());
            }
            _ => panic!("Expected PtySpawnResponse"),
        }
    }

    #[tokio::test]
    async fn test_handle_pty_kill_request_no_manager() {
        let state = make_state(vec![]);
        let req = PtyKillRequest { pty_id: 1 };
        let result = handle_pty_kill_request(&req, &state).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::PtyKillResponse(r) => {
                assert!(!r.success);
                assert!(r.error.is_some());
            }
            _ => panic!("Expected PtyKillResponse"),
        }
    }

    // --- J. Worktreeハンドラ ---

    #[tokio::test]
    async fn test_handle_worktree_list_request_with_repo() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let state = make_state(vec![repo_path]);
        let result = handle_worktree_list_request(&state).await;
        let msg = result.unwrap();
        match msg {
            WsMessage::WorktreeListResponse(r) => {
                assert!(!r.worktrees.is_empty());
            }
            _ => panic!("Expected WorktreeListResponse"),
        }
    }

    #[tokio::test]
    async fn test_handle_worktree_select_request_invalid_path() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let state = make_state(vec![repo_path]);
        let selected = make_selected(None);
        let req = WorktreeSelectRequest {
            path: "/nonexistent/worktree/path".to_string(),
        };
        let result = handle_worktree_select_request(&req, &state, &selected).await;
        match result {
            Some(WsMessage::WorktreeSelectResponse(r)) => {
                assert!(!r.success);
                assert!(r.error.is_some());
            }
            _ => panic!("Expected WorktreeSelectResponse with success=false"),
        }
    }

    #[tokio::test]
    async fn test_handle_worktree_select_request_valid_path() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let state = make_state(vec![repo_path.clone()]);
        let selected = make_selected(None);
        let req = WorktreeSelectRequest {
            path: repo_path.clone(),
        };
        let _result = handle_worktree_select_request(&req, &state, &selected).await;
        let wt = selected.lock().await;
        assert_eq!(wt.as_ref().unwrap(), &repo_path);
    }

    #[tokio::test]
    async fn test_build_all_worktrees() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let state = make_state(vec![repo_path]);
        let worktrees = build_all_worktrees(&state).await;
        assert!(!worktrees.is_empty());
    }

    // --- K. コメントハンドラ ---

    #[test]
    fn test_handle_add_comment_no_app_handle() {
        let state = make_state(vec![]);
        let comment = AddComment {
            file_path: "test.rs".to_string(),
            line_number: 1,
            end_line: None,
            content: "test comment".to_string(),
            severity: None,
            target: "local".to_string(),
        };
        let result = handle_add_comment(&comment, &state);
        assert!(result.is_none());
    }

    #[test]
    fn test_handle_delete_comment_no_app_handle() {
        let state = make_state(vec![]);
        let req = DeleteComment {
            id: "comment-1".to_string(),
        };
        let result = handle_delete_comment(&req, &state);
        assert!(result.is_none());
    }

    #[test]
    fn test_handle_update_comment_no_app_handle() {
        let state = make_state(vec![]);
        let req = UpdateComment {
            id: "comment-1".to_string(),
            content: "updated".to_string(),
        };
        let result = handle_update_comment(&req, &state);
        assert!(result.is_none());
    }
}
