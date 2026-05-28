use std::path::PathBuf;
use std::sync::Arc;

use tauri::{Emitter, Manager};
use tokio::sync::Mutex;

use super::WsServerState;
use crate::protocol::*;

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

fn review_error_payload(error: crate::review_comments::ReviewError) -> ReviewErrorPayload {
    let dto = error.dto();
    ReviewErrorPayload {
        code: dto.code,
        message: dto.message,
    }
}

async fn resolve_review_worktree(
    requested: Option<String>,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Result<String, WsMessage> {
    let wt = selected_worktree.lock().await;
    let selected = wt.clone().ok_or_else(no_worktree_selected_error)?;
    if let Some(worktree) = requested {
        let selected_name = worktree_name_from_path(&selected);
        if worktree == selected || worktree == selected_name {
            return Ok(selected);
        }
        return Err(WsMessage::Error(ErrorMsg {
            code: "WORKTREE_MISMATCH".to_string(),
            message: "Requested review worktree does not match the selected worktree".to_string(),
        }));
    }
    Ok(selected)
}

fn worktree_name_from_path(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| path.to_string())
}

fn emit_review_changed(app: &tauri::AppHandle, worktree_name: &str) {
    let _ = app.emit("review-comments-changed", worktree_name);
}

#[cfg(test)]
type ReviewTestDeps = (
    Arc<crate::review_comments::ReviewCommentStore>,
    PathBuf,
    Arc<parking_lot::Mutex<Vec<String>>>,
);

#[cfg(test)]
fn review_test_deps(state: &WsServerState) -> Option<ReviewTestDeps> {
    state.test_review_deps.as_ref().map(|(store, data_dir)| {
        (
            Arc::clone(store),
            data_dir.clone(),
            Arc::clone(&state.test_review_emit_log),
        )
    })
}

pub(super) async fn handle_review_list_request(
    req: &ReviewListRequest,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_name =
        match resolve_review_worktree(req.worktree_name.clone(), selected_worktree).await {
            Ok(worktree_name) => worktree_name,
            Err(msg) => return Some(msg),
        };
    #[cfg(test)]
    if let Some((store, data_dir, _emit_log)) = review_test_deps(state) {
        return review_list_response_from_store(store, data_dir, worktree_name, req.filter.clone())
            .await;
    }
    let app = match &state.app_handle {
        Some(app) => app.clone(),
        None => {
            return Some(WsMessage::ReviewListResponse(ReviewListResponse {
                success: false,
                worktree_name: Some(worktree_name),
                threads: Vec::new(),
                error: Some(ReviewErrorPayload {
                    code: crate::review_comments::ReviewErrorCode::Io,
                    message: "App handle not available".to_string(),
                }),
            }))
        }
    };
    let filter = req.filter.clone();
    let query_worktree_name = worktree_name.clone();
    match tokio::task::spawn_blocking(move || {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|e| crate::review_comments::ReviewError::InvalidInput(e.to_string()))?;
        let store = app.state::<Arc<crate::review_comments::ReviewCommentStore>>();
        store.list_threads(
            &data_dir,
            &query_worktree_name,
            filter,
            crate::review_comments::ReviewActor::human(),
        )
    })
    .await
    {
        Ok(Ok(threads)) => Some(WsMessage::ReviewListResponse(ReviewListResponse {
            success: true,
            worktree_name: Some(worktree_name),
            threads,
            error: None,
        })),
        Ok(Err(e)) => Some(WsMessage::ReviewListResponse(ReviewListResponse {
            success: false,
            worktree_name: Some(worktree_name),
            threads: Vec::new(),
            error: Some(review_error_payload(e)),
        })),
        Err(e) => Some(join_error_msg(e)),
    }
}

pub(super) async fn handle_review_get_request(
    req: &ReviewGetRequest,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_name =
        match resolve_review_worktree(req.worktree_name.clone(), selected_worktree).await {
            Ok(worktree_name) => worktree_name,
            Err(msg) => return Some(msg),
        };
    let thread_id = req.thread_id.clone();
    #[cfg(test)]
    if let Some((store, data_dir, emit_log)) = review_test_deps(state) {
        return review_thread_response_from_store(
            store,
            data_dir,
            worktree_name,
            false,
            emit_log,
            move |store, data_dir, worktree_name| {
                store.get_thread(data_dir, worktree_name, &thread_id)
            },
        )
        .await;
    }
    let app = match &state.app_handle {
        Some(app) => app.clone(),
        None => {
            return Some(WsMessage::ReviewThreadResponse(ReviewThreadResponse {
                success: false,
                worktree_name: Some(worktree_name),
                thread: None,
                error: Some(ReviewErrorPayload {
                    code: crate::review_comments::ReviewErrorCode::Io,
                    message: "App handle not available".to_string(),
                }),
            }))
        }
    };
    review_thread_response_from_blocking(
        app,
        worktree_name,
        false,
        move |store, data_dir, worktree_name| store.get_thread(data_dir, worktree_name, &thread_id),
    )
    .await
}

pub(super) async fn handle_review_create_request(
    req: &ReviewCreateRequest,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_name =
        match resolve_review_worktree(req.worktree_name.clone(), selected_worktree).await {
            Ok(worktree_name) => worktree_name,
            Err(msg) => return Some(msg),
        };
    let target = req.target.clone();
    let content = req.content.clone();
    #[cfg(test)]
    if let Some((store, data_dir, emit_log)) = review_test_deps(state) {
        return review_thread_response_from_store(
            store,
            data_dir,
            worktree_name,
            true,
            emit_log,
            move |store, data_dir, worktree_name| {
                store.create_thread(
                    data_dir,
                    worktree_name,
                    crate::review_comments::ReviewActor::human(),
                    target,
                    content,
                )
            },
        )
        .await;
    }
    let app = match &state.app_handle {
        Some(app) => app.clone(),
        None => {
            return Some(WsMessage::ReviewThreadResponse(ReviewThreadResponse {
                success: false,
                worktree_name: Some(worktree_name),
                thread: None,
                error: Some(ReviewErrorPayload {
                    code: crate::review_comments::ReviewErrorCode::Io,
                    message: "App handle not available".to_string(),
                }),
            }))
        }
    };
    review_thread_response_from_blocking(
        app,
        worktree_name,
        true,
        move |store, data_dir, worktree_name| {
            store.create_thread(
                data_dir,
                worktree_name,
                crate::review_comments::ReviewActor::human(),
                target,
                content,
            )
        },
    )
    .await
}

pub(super) async fn handle_review_append_comment_request(
    req: &ReviewAppendCommentRequest,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_name =
        match resolve_review_worktree(req.worktree_name.clone(), selected_worktree).await {
            Ok(worktree_name) => worktree_name,
            Err(msg) => return Some(msg),
        };
    let thread_id = req.thread_id.clone();
    let content = req.content.clone();
    #[cfg(test)]
    if let Some((store, data_dir, emit_log)) = review_test_deps(state) {
        return review_thread_response_from_store(
            store,
            data_dir,
            worktree_name,
            true,
            emit_log,
            move |store, data_dir, worktree_name| {
                store.append_comment(
                    data_dir,
                    worktree_name,
                    crate::review_comments::ReviewActor::human(),
                    &thread_id,
                    content,
                )
            },
        )
        .await;
    }
    let app = match &state.app_handle {
        Some(app) => app.clone(),
        None => {
            return Some(WsMessage::ReviewThreadResponse(ReviewThreadResponse {
                success: false,
                worktree_name: Some(worktree_name),
                thread: None,
                error: Some(ReviewErrorPayload {
                    code: crate::review_comments::ReviewErrorCode::Io,
                    message: "App handle not available".to_string(),
                }),
            }))
        }
    };
    review_thread_response_from_blocking(
        app,
        worktree_name,
        true,
        move |store, data_dir, worktree_name| {
            store.append_comment(
                data_dir,
                worktree_name,
                crate::review_comments::ReviewActor::human(),
                &thread_id,
                content,
            )
        },
    )
    .await
}

pub(super) async fn handle_review_resolve_request(
    req: &ReviewResolveRequest,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_name =
        match resolve_review_worktree(req.worktree_name.clone(), selected_worktree).await {
            Ok(worktree_name) => worktree_name,
            Err(msg) => return Some(msg),
        };
    let thread_id = req.thread_id.clone();
    let outcome = req.outcome.clone();
    let summary = req.summary.clone();
    #[cfg(test)]
    if let Some((store, data_dir, emit_log)) = review_test_deps(state) {
        return review_thread_response_from_store(
            store,
            data_dir,
            worktree_name,
            true,
            emit_log,
            move |store, data_dir, worktree_name| {
                store.resolve_thread(
                    data_dir,
                    worktree_name,
                    crate::review_comments::ReviewActor::human(),
                    &thread_id,
                    outcome,
                    summary,
                )
            },
        )
        .await;
    }
    let app = match &state.app_handle {
        Some(app) => app.clone(),
        None => {
            return Some(WsMessage::ReviewThreadResponse(ReviewThreadResponse {
                success: false,
                worktree_name: Some(worktree_name),
                thread: None,
                error: Some(ReviewErrorPayload {
                    code: crate::review_comments::ReviewErrorCode::Io,
                    message: "App handle not available".to_string(),
                }),
            }))
        }
    };
    review_thread_response_from_blocking(
        app,
        worktree_name,
        true,
        move |store, data_dir, worktree_name| {
            store.resolve_thread(
                data_dir,
                worktree_name,
                crate::review_comments::ReviewActor::human(),
                &thread_id,
                outcome,
                summary,
            )
        },
    )
    .await
}

pub(super) async fn handle_review_history_request(
    req: &ReviewHistoryRequest,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let worktree_name =
        match resolve_review_worktree(req.worktree_name.clone(), selected_worktree).await {
            Ok(worktree_name) => worktree_name,
            Err(msg) => return Some(msg),
        };
    let thread_id = req.thread_id.clone();
    #[cfg(test)]
    if let Some((store, data_dir, _emit_log)) = review_test_deps(state) {
        return review_history_response_from_store(store, data_dir, worktree_name, thread_id).await;
    }
    let app = match &state.app_handle {
        Some(app) => app.clone(),
        None => {
            return Some(WsMessage::ReviewHistoryResponse(ReviewHistoryResponse {
                success: false,
                worktree_name: Some(worktree_name),
                events: Vec::new(),
                error: Some(ReviewErrorPayload {
                    code: crate::review_comments::ReviewErrorCode::Io,
                    message: "App handle not available".to_string(),
                }),
            }))
        }
    };
    let query_worktree_name = worktree_name.clone();
    match tokio::task::spawn_blocking(move || {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|e| crate::review_comments::ReviewError::InvalidInput(e.to_string()))?;
        let store = app.state::<Arc<crate::review_comments::ReviewCommentStore>>();
        store.history(&data_dir, &query_worktree_name, &thread_id)
    })
    .await
    {
        Ok(Ok(events)) => Some(WsMessage::ReviewHistoryResponse(ReviewHistoryResponse {
            success: true,
            worktree_name: Some(worktree_name),
            events,
            error: None,
        })),
        Ok(Err(e)) => Some(WsMessage::ReviewHistoryResponse(ReviewHistoryResponse {
            success: false,
            worktree_name: Some(worktree_name),
            events: Vec::new(),
            error: Some(review_error_payload(e)),
        })),
        Err(e) => Some(join_error_msg(e)),
    }
}

#[cfg(test)]
async fn review_list_response_from_store(
    store: Arc<crate::review_comments::ReviewCommentStore>,
    data_dir: PathBuf,
    worktree_name: String,
    filter: Option<crate::review_comments::ReviewThreadFilter>,
) -> Option<WsMessage> {
    let query_worktree_name = worktree_name.clone();
    match tokio::task::spawn_blocking(move || {
        store.list_threads(
            &data_dir,
            &query_worktree_name,
            filter,
            crate::review_comments::ReviewActor::human(),
        )
    })
    .await
    {
        Ok(Ok(threads)) => Some(WsMessage::ReviewListResponse(ReviewListResponse {
            success: true,
            worktree_name: Some(worktree_name),
            threads,
            error: None,
        })),
        Ok(Err(e)) => Some(WsMessage::ReviewListResponse(ReviewListResponse {
            success: false,
            worktree_name: Some(worktree_name),
            threads: Vec::new(),
            error: Some(review_error_payload(e)),
        })),
        Err(e) => Some(join_error_msg(e)),
    }
}

#[cfg(test)]
async fn review_history_response_from_store(
    store: Arc<crate::review_comments::ReviewCommentStore>,
    data_dir: PathBuf,
    worktree_name: String,
    thread_id: String,
) -> Option<WsMessage> {
    let query_worktree_name = worktree_name.clone();
    match tokio::task::spawn_blocking(move || {
        store.history(&data_dir, &query_worktree_name, &thread_id)
    })
    .await
    {
        Ok(Ok(events)) => Some(WsMessage::ReviewHistoryResponse(ReviewHistoryResponse {
            success: true,
            worktree_name: Some(worktree_name),
            events,
            error: None,
        })),
        Ok(Err(e)) => Some(WsMessage::ReviewHistoryResponse(ReviewHistoryResponse {
            success: false,
            worktree_name: Some(worktree_name),
            events: Vec::new(),
            error: Some(review_error_payload(e)),
        })),
        Err(e) => Some(join_error_msg(e)),
    }
}

#[cfg(test)]
async fn review_thread_response_from_store<F>(
    store: Arc<crate::review_comments::ReviewCommentStore>,
    data_dir: PathBuf,
    worktree_name: String,
    emit_changed: bool,
    emit_log: Arc<parking_lot::Mutex<Vec<String>>>,
    f: F,
) -> Option<WsMessage>
where
    F: FnOnce(
            &Arc<crate::review_comments::ReviewCommentStore>,
            &PathBuf,
            &str,
        )
            -> Result<crate::review_comments::ReviewThread, crate::review_comments::ReviewError>
        + Send
        + 'static,
{
    let emit_worktree_name = worktree_name.clone();
    match tokio::task::spawn_blocking(move || {
        let thread = f(&store, &data_dir, &worktree_name)?;
        Ok::<_, crate::review_comments::ReviewError>(thread)
    })
    .await
    {
        Ok(Ok(thread)) => {
            if emit_changed {
                emit_log.lock().push(emit_worktree_name.clone());
            }
            Some(WsMessage::ReviewThreadResponse(ReviewThreadResponse {
                success: true,
                worktree_name: Some(emit_worktree_name),
                thread: Some(thread),
                error: None,
            }))
        }
        Ok(Err(e)) => Some(WsMessage::ReviewThreadResponse(ReviewThreadResponse {
            success: false,
            worktree_name: Some(emit_worktree_name),
            thread: None,
            error: Some(review_error_payload(e)),
        })),
        Err(e) => Some(join_error_msg(e)),
    }
}

async fn review_thread_response_from_blocking<F>(
    app: tauri::AppHandle,
    worktree_name: String,
    emit_changed: bool,
    f: F,
) -> Option<WsMessage>
where
    F: FnOnce(
            &Arc<crate::review_comments::ReviewCommentStore>,
            &PathBuf,
            &str,
        )
            -> Result<crate::review_comments::ReviewThread, crate::review_comments::ReviewError>
        + Send
        + 'static,
{
    let emit_app = app.clone();
    let emit_worktree_name = worktree_name.clone();
    match tokio::task::spawn_blocking(move || {
        let data_dir = app
            .path()
            .app_data_dir()
            .map_err(|e| crate::review_comments::ReviewError::InvalidInput(e.to_string()))?;
        let store = app.state::<Arc<crate::review_comments::ReviewCommentStore>>();
        let thread = f(store.inner(), &data_dir, &worktree_name)?;
        Ok::<_, crate::review_comments::ReviewError>(thread)
    })
    .await
    {
        Ok(Ok(thread)) => {
            if emit_changed {
                emit_review_changed(&emit_app, &emit_worktree_name);
            }
            Some(WsMessage::ReviewThreadResponse(ReviewThreadResponse {
                success: true,
                worktree_name: Some(emit_worktree_name),
                thread: Some(thread),
                error: None,
            }))
        }
        Ok(Err(e)) => Some(WsMessage::ReviewThreadResponse(ReviewThreadResponse {
            success: false,
            worktree_name: Some(emit_worktree_name),
            thread: None,
            error: Some(review_error_payload(e)),
        })),
        Err(e) => Some(join_error_msg(e)),
    }
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
    if let Ok(branch) = tokio::task::spawn_blocking(move || {
        crate::git::get_current_branch(wt_path).unwrap_or_default()
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
                }));
            }
        }
    }

    None
}

pub(super) fn handle_backend_list_request(state: &WsServerState) -> Option<WsMessage> {
    let registry = state.get_backend_registry();
    let backends = registry
        .list()
        .into_iter()
        .map(BackendInfoMsg::from)
        .collect();
    let default_id = registry.resolve_default_id().ok();
    Some(WsMessage::BackendListResponse(BackendListResponse {
        backends,
        default_id,
    }))
}

async fn is_managed_worktree(state: &WsServerState, worktree_path: &str) -> bool {
    let repo_paths = state.get_repo_paths();
    let requested_path = worktree_path.to_string();
    tokio::task::spawn_blocking(move || {
        for repo_path in &repo_paths {
            let worktrees = crate::git::list_worktrees(repo_path.clone()).unwrap_or_default();
            if worktrees.iter().any(|w| w.path == requested_path) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false)
}

fn agent_session_start_error(backend_id: Option<String>, error: impl Into<String>) -> WsMessage {
    WsMessage::AgentSessionStartResponse(AgentSessionStartResponse {
        success: false,
        session_id: None,
        backend_id,
        error: Some(error.into()),
    })
}

fn agent_message_error(req: &AgentMessageRequest, error: impl Into<String>) -> WsMessage {
    WsMessage::AgentMessageResponse(AgentMessageResponse {
        success: false,
        session_id: req.session_id.clone(),
        human_message_id: None,
        agent_message_id: None,
        backend_id: req.backend_id.clone(),
        error: Some(error.into()),
    })
}

fn effective_agent_message_worktree(
    req: &AgentMessageRequest,
    persisted_session: Option<&crate::session::ChatSession>,
) -> Result<String, String> {
    if let Some(session_id) = req.session_id.as_deref() {
        let session =
            persisted_session.ok_or_else(|| format!("Session not found: {session_id}"))?;
        return Ok(session.worktree_path.clone());
    }
    Ok(req.worktree_path.clone())
}

pub(super) async fn handle_agent_session_start_request(
    req: &AgentSessionStartRequest,
    state: &WsServerState,
) -> Option<WsMessage> {
    // WebSocket 境界で wire 型 → typed handler request に変換する。欠落・対象外値は
    // セッション状態を変更せず bridge にも送らない（Spec issues-947）。
    let typed = match AgentSessionStartHandlerRequest::try_from(req) {
        Ok(typed) => typed,
        Err(e) => {
            return Some(agent_session_start_error(
                req.backend_id.clone(),
                e.to_string(),
            ));
        }
    };

    if !is_managed_worktree(state, &typed.worktree_path).await {
        return Some(agent_session_start_error(
            None,
            "指定されたworktreeが見つかりません",
        ));
    }

    let registry = state.get_backend_registry();

    let resolved_backend_id = match registry.resolve_backend_id(typed.backend_id.clone()) {
        Ok(id) => id,
        Err(e) => {
            return Some(agent_session_start_error(None, e));
        }
    };

    // 検証済み PermissionMode を初回保存で確定する。
    // edit デフォルトで保存→update する二段階保存をやめ、途中失敗時に edit のセッションだけ残る
    // 中間状態を排除する（Spec issues-947: セッション保存層を permission_mode の正典とする）。
    match state.create_session_with_permission(
        &typed.worktree_path,
        Some(resolved_backend_id.clone()),
        typed.permission_mode,
    ) {
        Ok(session) => Some(WsMessage::AgentSessionStartResponse(
            AgentSessionStartResponse {
                success: true,
                session_id: Some(session.id),
                backend_id: Some(resolved_backend_id),
                error: None,
            },
        )),
        Err(e) => Some(agent_session_start_error(Some(resolved_backend_id), e)),
    }
}

pub(super) async fn handle_agent_message_request(
    req: &AgentMessageRequest,
    state: &WsServerState,
) -> Option<WsMessage> {
    use tauri::Manager;

    // Spec issues-947: WS 境界で wire 型 → typed handler request に変換する。欠落・対象外値は
    // セッション状態を変更せず bridge にも送らずに success=false を返す。
    let typed = match AgentMessageHandlerRequest::try_from(req) {
        Ok(typed) => typed,
        Err(e) => return Some(agent_message_error(req, e.to_string())),
    };

    if typed.session_id.is_none() && !is_managed_worktree(state, &typed.worktree_path).await {
        return Some(agent_message_error(
            req,
            "指定されたworktreeが見つかりません",
        ));
    }

    let app = match &state.app_handle {
        Some(app) => app,
        None => {
            return Some(WsMessage::AgentMessageResponse(AgentMessageResponse {
                success: false,
                session_id: req.session_id.clone(),
                human_message_id: None,
                agent_message_id: None,
                backend_id: req.backend_id.clone(),
                error: Some("App handle not available".to_string()),
            }));
        }
    };

    let session_store = app
        .state::<Arc<crate::session::SessionStore>>()
        .inner()
        .clone();
    let data_dir = match crate::session::resolve_data_dir(app) {
        Ok(data_dir) => data_dir,
        Err(e) => return Some(agent_message_error(req, e)),
    };
    let persisted_session = if let Some(session_id) = typed.session_id.as_deref() {
        match session_store.get_session(&data_dir, session_id) {
            Ok(session) => session,
            Err(e) => return Some(agent_message_error(req, e)),
        }
    } else {
        None
    };
    let worktree_path = match effective_agent_message_worktree(req, persisted_session.as_ref()) {
        Ok(worktree_path) => worktree_path,
        Err(e) => return Some(agent_message_error(req, e)),
    };
    if !is_managed_worktree(state, &worktree_path).await {
        return Some(agent_message_error(
            req,
            "指定されたworktreeが見つかりません",
        ));
    }

    let handles = app
        .state::<Arc<tokio::sync::Mutex<crate::agent_sdk::AgentProcessMap>>>()
        .inner()
        .clone();
    let registry = state.get_backend_registry().clone();
    let engine = app
        .state::<Arc<crate::workflow::engine::WorkflowEngine>>()
        .inner()
        .clone();
    let open_tabs = app
        .state::<Arc<crate::session::OpenTabRegistry>>()
        .inner()
        .clone();
    let response = crate::agent_message_dispatcher::dispatch_agent_message(
        crate::agent_message_dispatcher::AgentMessageDispatchContext {
            app,
            session_store: &session_store,
            registry: &registry,
            handles: &handles,
        },
        crate::agent_message_dispatcher::AgentMessageDispatchRequest {
            chat_session_id: typed.session_id.clone(),
            worktree_path,
            content: typed.content.clone(),
            permission_mode: typed.permission_mode,
            backend_id: typed.backend_id.clone(),
            images: None,
            mentions: None,
        },
    )
    .await;

    match response {
        Ok(response) => {
            crate::workflow_state_events::emit_after_workflow_step_message(
                app,
                &engine,
                &response.session,
                &handles,
                &open_tabs,
            )
            .await;
            Some(WsMessage::AgentMessageResponse(AgentMessageResponse {
                success: true,
                session_id: Some(response.session.id),
                human_message_id: Some(response.human_message.id),
                agent_message_id: response.agent_message.map(|m| m.id),
                backend_id: response.session.backend_id,
                error: None,
            }))
        }
        Err(e) => Some(agent_message_error(req, e)),
    }
}

pub(super) async fn handle_agent_interrupt_request(
    req: &AgentInterruptRequest,
    state: &WsServerState,
) -> Option<WsMessage> {
    use tauri::Manager;

    let result = if let Some(app) = &state.app_handle {
        let handles = app
            .state::<Arc<tokio::sync::Mutex<crate::agent_sdk::AgentProcessMap>>>()
            .inner()
            .clone();
        crate::backends::bridge_common::write_bridge_command(
            &handles,
            &req.session_id,
            serde_json::json!({"type": "interrupt"}),
        )
        .await
    } else {
        Err("App handle not available".to_string())
    };

    Some(WsMessage::AgentInterruptResponse(AgentInterruptResponse {
        success: result.is_ok(),
        session_id: req.session_id.clone(),
        error: result.err(),
    }))
}

pub(super) async fn handle_agent_model_set_request(
    req: &AgentModelSetRequest,
    state: &WsServerState,
) -> Option<WsMessage> {
    use tauri::Manager;

    if let Some(app) = &state.app_handle {
        let session_store = app
            .state::<Arc<crate::session::SessionStore>>()
            .inner()
            .clone();
        let handles = app
            .state::<Arc<tokio::sync::Mutex<crate::agent_sdk::AgentProcessMap>>>()
            .inner()
            .clone();
        let data_dir = match crate::session::resolve_data_dir(app) {
            Ok(data_dir) => data_dir,
            Err(e) => {
                return Some(agent_model_set_response(req, Err(e)));
            }
        };
        return handle_agent_model_set_request_with_data_dir(
            req,
            Some(app),
            &handles,
            &session_store,
            Some(state.get_backend_registry()),
            &data_dir,
        )
        .await;
    }

    Some(agent_model_set_response(
        req,
        Err("App handle not available".to_string()),
    ))
}

fn agent_model_set_response(req: &AgentModelSetRequest, result: Result<(), String>) -> WsMessage {
    WsMessage::AgentModelSetResponse(AgentModelSetResponse {
        success: result.is_ok(),
        session_id: req.session_id.clone(),
        model_id: req.model_id.clone(),
        error: result.err(),
    })
}

async fn handle_agent_model_set_request_with_data_dir(
    req: &AgentModelSetRequest,
    app: Option<&tauri::AppHandle>,
    handles: &Arc<tokio::sync::Mutex<crate::agent_sdk::AgentProcessMap>>,
    session_store: &Arc<crate::session::SessionStore>,
    registry: Option<&Arc<crate::backends::AgentBackendRegistry>>,
    data_dir: &std::path::Path,
) -> Option<WsMessage> {
    let result = crate::agent_sdk::set_agent_model_internal_with_data_dir(
        app,
        handles,
        session_store,
        registry,
        data_dir,
        &req.session_id,
        req.model_id.clone(),
    )
    .await;

    Some(agent_model_set_response(req, result))
}

pub(super) async fn handle_agent_permission_mode_set_request(
    req: &AgentPermissionModeSetRequest,
    state: &WsServerState,
) -> Option<WsMessage> {
    use tauri::Manager;

    let typed = match AgentPermissionModeSetHandlerRequest::try_from(req) {
        Ok(typed) => typed,
        Err(e) => {
            return Some(WsMessage::AgentPermissionModeSetResponse(
                AgentPermissionModeSetResponse {
                    success: false,
                    session_id: req.session_id.clone(),
                    permission_mode: req.permission_mode.clone(),
                    error: Some(e.to_string()),
                },
            ));
        }
    };

    let result = if let Some(app) = &state.app_handle {
        let session_store = app
            .state::<Arc<crate::session::SessionStore>>()
            .inner()
            .clone();
        let handles = app
            .state::<Arc<tokio::sync::Mutex<crate::agent_sdk::AgentProcessMap>>>()
            .inner()
            .clone();
        match crate::session::resolve_data_dir(app) {
            Ok(data_dir) => {
                crate::agent_sdk::set_agent_permission_mode_internal(
                    &session_store,
                    &handles,
                    &data_dir,
                    &typed.session_id,
                    typed.permission_mode.as_str(),
                )
                .await
            }
            Err(e) => Err(e),
        }
    } else {
        Err("App handle not available".to_string())
    };

    Some(WsMessage::AgentPermissionModeSetResponse(
        AgentPermissionModeSetResponse {
            success: result.is_ok(),
            session_id: typed.session_id,
            permission_mode: typed.permission_mode.as_str().to_string(),
            error: result.err(),
        },
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::test_helpers::{add_and_commit, create_initial_commit, create_test_repo};
    use crate::session::{ChatSession, SessionState};
    use crate::ws_bridge::WsBroadcaster;
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
            std::sync::Arc::new(crate::backends::AgentBackendRegistry::new()),
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

    fn make_chat_session(id: &str, worktree_path: &str) -> ChatSession {
        ChatSession {
            id: id.to_string(),
            worktree_path: worktree_path.to_string(),
            messages: Vec::new(),
            state: SessionState::Active,
            created_at: 1000.0,
            updated_at: 1000.0,
            agent_session_id: None,
            permission_mode: "edit".to_string(),
            selected_model: None,
            backend_id: Some("claude".to_string()),
            workflow_step_session: false,
        }
    }

    struct MockBackend {
        backend_id: String,
        backend_name: String,
    }

    #[async_trait::async_trait]
    impl crate::backends::AgentBackend for MockBackend {
        fn id(&self) -> &str {
            &self.backend_id
        }

        fn name(&self) -> &str {
            &self.backend_name
        }

        async fn start_session(
            &self,
            _config: crate::backends::SessionConfig,
        ) -> Result<crate::backends::SessionHandle, String> {
            Ok(crate::backends::SessionHandle {
                chat_session_id: "test".to_string(),
                backend_id: self.backend_id.clone(),
            })
        }

        async fn send_message(
            &self,
            _session: &crate::backends::SessionHandle,
            _message: crate::backends::AgentMessage,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn interrupt(&self, _session: &crate::backends::SessionHandle) -> Result<(), String> {
            Ok(())
        }

        async fn respond_permission(
            &self,
            _session: &crate::backends::SessionHandle,
            _response: crate::backends::PermissionResponse,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn close_session(
            &self,
            _session: &crate::backends::SessionHandle,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    fn make_model_registry(
        claude_models: &[&str],
        codex_models: &[&str],
    ) -> Arc<crate::backends::AgentBackendRegistry> {
        let mut cfg = crate::config::ReleashConfig::default();
        cfg.agents.claude.models = claude_models.iter().map(|s| s.to_string()).collect();
        cfg.agents.codex.models = codex_models.iter().map(|s| s.to_string()).collect();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let config = Arc::new(crate::config::AppConfig::new(cfg, tmp.path().to_path_buf()));
        let mut registry = crate::backends::AgentBackendRegistry::new();
        registry.register(Arc::new(MockBackend {
            backend_id: "claude".to_string(),
            backend_name: "Claude".to_string(),
        }));
        registry.register(Arc::new(MockBackend {
            backend_id: "codex".to_string(),
            backend_name: "Codex".to_string(),
        }));
        registry.set_config(config);
        Arc::new(registry)
    }

    async fn call_agent_model_set_for_test(
        session_store: &Arc<crate::session::SessionStore>,
        registry: &Arc<crate::backends::AgentBackendRegistry>,
        data_dir: &std::path::Path,
        session_id: &str,
        model_id: Option<String>,
    ) -> AgentModelSetResponse {
        let req = AgentModelSetRequest {
            session_id: session_id.to_string(),
            model_id,
        };
        let handles = Arc::new(Mutex::new(crate::agent_sdk::AgentProcessMap::new()));
        match handle_agent_model_set_request_with_data_dir(
            &req,
            None,
            &handles,
            session_store,
            Some(registry),
            data_dir,
        )
        .await
        {
            Some(WsMessage::AgentModelSetResponse(resp)) => resp,
            other => panic!("expected AgentModelSetResponse, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handle_agent_model_set_request_accepts_registered_model_as_ws_response() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::session::SessionStore::default());
        let session = crate::session::create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();
        let registry = make_model_registry(&["claude-4"], &["gpt-5"]);

        let resp = call_agent_model_set_for_test(
            &session_store,
            &registry,
            temp.path(),
            &session.id,
            Some("claude-4".to_string()),
        )
        .await;

        assert!(resp.success);
        assert_eq!(resp.session_id, session.id);
        assert_eq!(resp.model_id.as_deref(), Some("claude-4"));
        assert_eq!(resp.error, None);
    }

    #[tokio::test]
    async fn handle_agent_model_set_request_rejects_unregistered_model_as_ws_response() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::session::SessionStore::default());
        let session = crate::session::create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();
        let registry = make_model_registry(&["claude-4"], &[]);

        let resp = call_agent_model_set_for_test(
            &session_store,
            &registry,
            temp.path(),
            &session.id,
            Some("unknown".to_string()),
        )
        .await;

        assert!(!resp.success);
        assert_eq!(resp.session_id, session.id);
        assert_eq!(resp.model_id.as_deref(), Some("unknown"));
        assert!(resp.error.unwrap().contains("登録されていません"));
    }

    #[tokio::test]
    async fn handle_agent_model_set_request_rejects_other_backend_model_as_ws_response() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::session::SessionStore::default());
        let session = crate::session::create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();
        let registry = make_model_registry(&["claude-4"], &["gpt-5"]);

        let resp = call_agent_model_set_for_test(
            &session_store,
            &registry,
            temp.path(),
            &session.id,
            Some("gpt-5".to_string()),
        )
        .await;

        assert!(!resp.success);
        assert_eq!(resp.model_id.as_deref(), Some("gpt-5"));
        assert!(resp.error.unwrap().contains("別バックエンド"));
    }

    #[tokio::test]
    async fn handle_agent_model_set_request_rejects_invalid_model_as_ws_response() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::session::SessionStore::default());
        let session = crate::session::create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();
        let registry = make_model_registry(&["claude-4"], &[]);

        let resp = call_agent_model_set_for_test(
            &session_store,
            &registry,
            temp.path(),
            &session.id,
            Some("bad\u{0001}model".to_string()),
        )
        .await;

        assert!(!resp.success);
        assert_eq!(resp.model_id.as_deref(), Some("bad\u{0001}model"));
        assert!(resp.error.unwrap().contains("制御文字"));
    }

    #[tokio::test]
    async fn handle_agent_model_set_request_accepts_null_model_as_clear_ws_response() {
        let temp = tempfile::tempdir().unwrap();
        let session_store = Arc::new(crate::session::SessionStore::default());
        let mut session = crate::session::create_session_internal(
            &session_store,
            temp.path(),
            "/repo",
            Some("claude".to_string()),
        )
        .unwrap();
        session.selected_model = Some("claude-4".to_string());
        session_store.save_session(temp.path(), &session).unwrap();
        let registry = make_model_registry(&["claude-4"], &[]);

        let resp = call_agent_model_set_for_test(
            &session_store,
            &registry,
            temp.path(),
            &session.id,
            None,
        )
        .await;

        assert!(resp.success);
        assert_eq!(resp.model_id, None);
        assert_eq!(resp.error, None);
        let after = session_store
            .get_session(temp.path(), &session.id)
            .unwrap()
            .unwrap();
        assert_eq!(after.selected_model, None);
    }

    #[tokio::test]
    async fn review_handlers_return_success_rejection_payloads_and_emit_changes() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::review_comments::ReviewCommentStore::default());
        let mut state = make_state(Vec::new());
        state.set_test_review_deps(Arc::clone(&store), temp.path().to_path_buf());
        let selected = make_selected(Some("/repo".to_string()));

        let created = handle_review_create_request(
            &ReviewCreateRequest {
                worktree_name: None,
                target: ReviewTarget {
                    file_path: Some("src/main.rs".to_string()),
                    line_number: Some(1),
                    end_line: Some(2),
                },
                content: "Review claim".to_string(),
            },
            &state,
            &selected,
        )
        .await;
        let thread = match created {
            Some(WsMessage::ReviewThreadResponse(resp)) => {
                assert!(resp.success);
                assert_eq!(resp.worktree_name.as_deref(), Some("/repo"));
                resp.thread.unwrap()
            }
            other => panic!("expected review thread response, got {other:?}"),
        };
        assert_eq!(state.test_review_emit_log(), vec!["/repo".to_string()]);

        let appended = handle_review_append_comment_request(
            &ReviewAppendCommentRequest {
                worktree_name: Some("repo".to_string()),
                thread_id: thread.id.clone(),
                content: "Human follow-up".to_string(),
            },
            &state,
            &selected,
        )
        .await;
        match appended {
            Some(WsMessage::ReviewThreadResponse(resp)) => {
                assert!(resp.success);
                assert_eq!(resp.thread.unwrap().comments.len(), 2);
            }
            other => panic!("expected append response, got {other:?}"),
        }

        let second_append = handle_review_append_comment_request(
            &ReviewAppendCommentRequest {
                worktree_name: None,
                thread_id: thread.id.clone(),
                content: "Another follow-up".to_string(),
            },
            &state,
            &selected,
        )
        .await;
        match second_append {
            Some(WsMessage::ReviewThreadResponse(resp)) => {
                assert!(resp.success);
                assert_eq!(resp.thread.unwrap().comments.len(), 3);
            }
            other => panic!("expected second append response, got {other:?}"),
        }
        assert_eq!(state.test_review_emit_log().len(), 3);

        let resolved = handle_review_resolve_request(
            &ReviewResolveRequest {
                worktree_name: None,
                thread_id: thread.id.clone(),
                outcome: "accepted".to_string(),
                summary: "done".to_string(),
            },
            &state,
            &selected,
        )
        .await;
        match resolved {
            Some(WsMessage::ReviewThreadResponse(resp)) => {
                assert!(resp.success);
                assert_eq!(
                    resp.thread.unwrap().state,
                    crate::review_comments::ReviewThreadState::Resolved
                );
            }
            other => panic!("expected resolve response, got {other:?}"),
        }

        let rejected = handle_review_append_comment_request(
            &ReviewAppendCommentRequest {
                worktree_name: None,
                thread_id: thread.id.clone(),
                content: "late".to_string(),
            },
            &state,
            &selected,
        )
        .await;
        match rejected {
            Some(WsMessage::ReviewThreadResponse(resp)) => {
                assert!(!resp.success);
                let error = resp.error.unwrap();
                assert_eq!(
                    error.code,
                    crate::review_comments::ReviewErrorCode::AlreadyResolved
                );
                assert!(error.message.contains("already resolved"));
            }
            other => panic!("expected rejected append response, got {other:?}"),
        }

        let history = handle_review_history_request(
            &ReviewHistoryRequest {
                worktree_name: None,
                thread_id: thread.id,
            },
            &state,
            &selected,
        )
        .await;
        match history {
            Some(WsMessage::ReviewHistoryResponse(resp)) => {
                assert!(resp.success);
                assert_eq!(resp.worktree_name.as_deref(), Some("/repo"));
                // ThreadCreated + CommentAppended (Human follow-up) +
                // CommentAppended (Another follow-up) + ThreadResolved = 4 events
                assert_eq!(resp.events.len(), 4);
            }
            other => panic!("expected history response, got {other:?}"),
        }

        let missing_history = handle_review_history_request(
            &ReviewHistoryRequest {
                worktree_name: None,
                thread_id: "missing-thread".to_string(),
            },
            &state,
            &selected,
        )
        .await;
        match missing_history {
            Some(WsMessage::ReviewHistoryResponse(resp)) => {
                assert!(!resp.success);
                assert_eq!(resp.worktree_name.as_deref(), Some("/repo"));
                assert_eq!(resp.events.len(), 0);
                assert_eq!(
                    resp.error.unwrap().code,
                    crate::review_comments::ReviewErrorCode::NotFound
                );
            }
            other => panic!("expected rejected history response, got {other:?}"),
        }
        assert_eq!(state.test_review_emit_log().len(), 4);
    }

    #[tokio::test]
    async fn review_handler_rejects_invalid_target_with_payload() {
        let temp = tempfile::tempdir().unwrap();
        let store = Arc::new(crate::review_comments::ReviewCommentStore::default());
        let mut state = make_state(Vec::new());
        state.set_test_review_deps(store, temp.path().to_path_buf());
        let selected = make_selected(Some("/repo".to_string()));

        let response = handle_review_create_request(
            &ReviewCreateRequest {
                worktree_name: None,
                target: ReviewTarget {
                    file_path: Some("../secret".to_string()),
                    line_number: Some(1),
                    end_line: None,
                },
                content: "Review claim".to_string(),
            },
            &state,
            &selected,
        )
        .await;

        match response {
            Some(WsMessage::ReviewThreadResponse(resp)) => {
                assert!(!resp.success);
                assert_eq!(
                    resp.error.unwrap().code,
                    crate::review_comments::ReviewErrorCode::InvalidInput
                );
            }
            other => panic!("expected rejection response, got {other:?}"),
        }
        assert!(state.test_review_emit_log().is_empty());
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

    #[tokio::test]
    async fn test_is_managed_worktree_accepts_known_worktree() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let state = make_state(vec![repo_path.clone()]);

        assert!(is_managed_worktree(&state, &repo_path).await);
    }

    #[tokio::test]
    async fn test_agent_message_request_without_session_rejects_invalid_worktree() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let state = make_state(vec![repo_path]);
        let req = AgentMessageRequest {
            session_id: None,
            worktree_path: "/nonexistent/worktree".to_string(),
            content: "hello".to_string(),
            permission_mode: Some("edit".to_string()),
            backend_id: Some("claude".to_string()),
        };

        let result = handle_agent_message_request(&req, &state).await;

        match result {
            Some(WsMessage::AgentMessageResponse(response)) => {
                assert!(!response.success);
                assert!(response.error.unwrap().contains("worktree"));
            }
            _ => panic!("expected AgentMessageResponse"),
        }
    }

    #[test]
    fn test_effective_agent_message_worktree_uses_persisted_session_worktree() {
        let req = AgentMessageRequest {
            session_id: Some("session-1".to_string()),
            worktree_path: "/request/worktree".to_string(),
            content: "hello".to_string(),
            permission_mode: Some("edit".to_string()),
            backend_id: None,
        };
        let session = make_chat_session("session-1", "/persisted/worktree");

        let worktree = effective_agent_message_worktree(&req, Some(&session)).unwrap();

        assert_eq!(worktree, "/persisted/worktree");
    }

    #[test]
    fn test_effective_agent_message_worktree_missing_session_returns_error() {
        let req = AgentMessageRequest {
            session_id: Some("missing-session".to_string()),
            worktree_path: "/request/worktree".to_string(),
            content: "hello".to_string(),
            permission_mode: Some("edit".to_string()),
            backend_id: None,
        };

        let error = effective_agent_message_worktree(&req, None).unwrap_err();

        assert!(error.contains("missing-session"));
    }

    // Spec issues-947: AgentSessionStartRequest の正常系3モード（ask/edit/full）が
    // handler 経路を通って保存済み ChatSession.permission_mode に記録されることを検証する。
    // 検証済み抽象 PermissionMode を初回保存で確定する経路（edit デフォルトで保存→update する
    // 二段階保存をやめる）が壊れたら、ここで検出する。
    #[tokio::test]
    async fn handle_agent_session_start_request_persists_each_abstract_mode() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let tmp_data = TempDir::new().unwrap();
        let session_store = Arc::new(crate::session::SessionStore::default());
        let mut registry = crate::backends::AgentBackendRegistry::new();
        registry.register(Arc::new(crate::backends::claude::ClaudeBackend::new()));
        registry.set_default(Some("claude".to_string()));
        let config = crate::config::ReleashConfig::default();
        let app_config = std::sync::Arc::new(crate::config::AppConfig::new(
            config,
            std::path::PathBuf::from("/tmp/test-releash.toml"),
        ));
        let mut state = WsServerState::new(
            None,
            std::sync::Arc::new(WsBroadcaster::default()),
            None,
            std::sync::Arc::new(parking_lot::RwLock::new(vec![repo_path.clone()])),
            app_config,
            None,
            false,
            std::sync::Arc::new(crate::git_host::PrCache::new()),
            std::sync::Arc::new(registry),
        );
        state.set_test_session_deps(session_store.clone(), tmp_data.path().to_path_buf());

        for mode in ["ask", "edit", "full"] {
            let req = AgentSessionStartRequest {
                worktree_path: repo_path.clone(),
                backend_id: Some("claude".to_string()),
                permission_mode: Some(mode.to_string()),
            };
            let result = handle_agent_session_start_request(&req, &state).await;
            let session_id = match result {
                Some(WsMessage::AgentSessionStartResponse(response)) => {
                    assert!(
                        response.success,
                        "mode={mode}: expected success, got error: {:?}",
                        response.error
                    );
                    assert_eq!(response.backend_id.as_deref(), Some("claude"));
                    response.session_id.expect("session_id must be present")
                }
                other => panic!("expected AgentSessionStartResponse, got {other:?}"),
            };
            let persisted = session_store
                .get_session(tmp_data.path(), &session_id)
                .unwrap()
                .expect("session must be persisted");
            assert_eq!(persisted.permission_mode, mode, "mode={mode}");
            assert_eq!(persisted.backend_id.as_deref(), Some("claude"));
            assert_eq!(persisted.worktree_path, repo_path);
        }
    }

    // Spec issues-947: WS 境界での AgentSessionStartRequest.permission_mode 拒否。
    // None / acceptEdits / unknown / 空文字 のいずれも success=false を返し、エラーメッセージに
    // 許可一覧（ask, edit, full）を含む。is_managed_worktree がエラー応答を生み出さないよう
    // 必ず存在する worktree を渡す。
    #[tokio::test]
    async fn handle_agent_session_start_request_rejects_invalid_permission_mode() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let state = make_state(vec![repo_path.clone()]);
        let cases: &[Option<&str>] = &[
            None,
            Some(""),
            Some("acceptEdits"),
            Some("bypassPermissions"),
            Some("plan"),
            Some("default"),
            Some("unknown"),
        ];
        for permission in cases {
            let req = AgentSessionStartRequest {
                worktree_path: repo_path.clone(),
                backend_id: Some("claude".to_string()),
                permission_mode: permission.map(|s| s.to_string()),
            };
            let result = handle_agent_session_start_request(&req, &state).await;
            match result {
                Some(WsMessage::AgentSessionStartResponse(response)) => {
                    assert!(
                        !response.success,
                        "permission_mode={:?} must be rejected",
                        permission
                    );
                    assert!(response.session_id.is_none());
                    let error = response.error.unwrap_or_default();
                    assert!(
                        error.contains("ask, edit, full"),
                        "error must include allowed list, got: {error}"
                    );
                }
                other => panic!("expected AgentSessionStartResponse, got {other:?}"),
            }
        }
    }

    // Spec issues-947: AgentMessageRequest.permission_mode の欠落・対象外値も WS 境界で拒否する。
    // WS レスポンスでの success=false と、許可一覧を含むエラーメッセージを直接検証する。
    #[tokio::test]
    async fn handle_agent_message_request_rejects_invalid_permission_mode() {
        let (_dir, repo_path) = setup_repo_with_file("file.txt", "content");
        let state = make_state(vec![repo_path.clone()]);
        let cases: &[Option<&str>] = &[
            None,
            Some(""),
            Some("acceptEdits"),
            Some("bypassPermissions"),
            Some("plan"),
            Some("default"),
            Some("unknown"),
        ];
        for permission in cases {
            let req = AgentMessageRequest {
                session_id: None,
                worktree_path: repo_path.clone(),
                content: "hello".to_string(),
                permission_mode: permission.map(|s| s.to_string()),
                backend_id: Some("claude".to_string()),
            };
            let result = handle_agent_message_request(&req, &state).await;
            match result {
                Some(WsMessage::AgentMessageResponse(response)) => {
                    assert!(
                        !response.success,
                        "permission_mode={:?} must be rejected",
                        permission
                    );
                    assert!(response.human_message_id.is_none());
                    assert!(response.agent_message_id.is_none());
                    let error = response.error.unwrap_or_default();
                    assert!(
                        error.contains("ask, edit, full"),
                        "error must include allowed list, got: {error}"
                    );
                }
                other => panic!("expected AgentMessageResponse, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_agent_session_start_error_response_is_not_success() {
        let msg = agent_session_start_error(Some("codex".to_string()), "bridge failed");

        match msg {
            WsMessage::AgentSessionStartResponse(response) => {
                assert!(!response.success);
                assert!(response.session_id.is_none());
                assert_eq!(response.backend_id, Some("codex".to_string()));
                assert_eq!(response.error, Some("bridge failed".to_string()));
            }
            _ => panic!("expected AgentSessionStartResponse"),
        }
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

    // --- H. handle_branch_info_request ---

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
}
