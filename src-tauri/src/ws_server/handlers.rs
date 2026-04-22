use std::sync::Arc;

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
