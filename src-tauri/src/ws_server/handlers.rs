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
    if !is_managed_worktree(state, &req.worktree_path).await {
        return Some(agent_session_start_error(
            None,
            "指定されたworktreeが見つかりません",
        ));
    }

    let registry = state.get_backend_registry();

    let resolved_backend_id = match registry.resolve_backend_id(req.backend_id.clone()) {
        Ok(id) => id,
        Err(e) => {
            return Some(agent_session_start_error(None, e));
        }
    };

    match state.create_session(&req.worktree_path, Some(resolved_backend_id.clone())) {
        Ok(session) => Some(WsMessage::AgentSessionStartResponse(
            AgentSessionStartResponse {
                success: true,
                session_id: Some(session.id),
                backend_id: Some(resolved_backend_id),
                error: None,
            },
        )),
        Err(e) => Some(agent_session_start_error(None, e)),
    }
}

pub(super) async fn handle_agent_message_request(
    req: &AgentMessageRequest,
    state: &WsServerState,
) -> Option<WsMessage> {
    use tauri::Manager;

    if req.session_id.is_none() && !is_managed_worktree(state, &req.worktree_path).await {
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
    let persisted_session = if let Some(session_id) = req.session_id.as_deref() {
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

    match crate::agent_sdk::send_agent_message_internal(
        app,
        &session_store,
        &registry,
        &handles,
        req.session_id.clone(),
        worktree_path,
        req.content.clone(),
        req.permission_mode.clone(),
        req.backend_id.clone(),
        None,
        None,
    )
    .await
    {
        Ok(response) => Some(WsMessage::AgentMessageResponse(AgentMessageResponse {
            success: true,
            session_id: Some(response.session.id),
            human_message_id: Some(response.human_message.id),
            agent_message_id: response.agent_message.map(|m| m.id),
            backend_id: response.session.backend_id,
            error: None,
        })),
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

    let result = if let Some(app) = &state.app_handle {
        let session_store = app
            .state::<Arc<crate::session::SessionStore>>()
            .inner()
            .clone();
        let handles = app
            .state::<Arc<tokio::sync::Mutex<crate::agent_sdk::AgentProcessMap>>>()
            .inner()
            .clone();
        crate::agent_sdk::set_agent_model_internal(
            app,
            &handles,
            &session_store,
            Some(state.get_backend_registry()),
            &req.session_id,
            req.model_id.clone(),
        )
        .await
    } else {
        Err("App handle not available".to_string())
    };

    Some(WsMessage::AgentModelSetResponse(AgentModelSetResponse {
        success: result.is_ok(),
        session_id: req.session_id.clone(),
        model_id: req.model_id.clone(),
        error: result.err(),
    }))
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
            permission_mode: "acceptEdits".to_string(),
            selected_model: None,
            workflow_state: None,
            backend_id: Some("claude".to_string()),
        }
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
            permission_mode: Some("acceptEdits".to_string()),
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
            permission_mode: Some("acceptEdits".to_string()),
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
            permission_mode: Some("acceptEdits".to_string()),
            backend_id: None,
        };

        let error = effective_agent_message_worktree(&req, None).unwrap_err();

        assert!(error.contains("missing-session"));
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
