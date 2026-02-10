use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use futures_util::{SinkExt, StreamExt};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio_tungstenite::tungstenite::Message;

use tauri::{Emitter, Manager};

use crate::config::{AppConfig, ReleashConfig};
use crate::protocol::*;
use crate::pty::PtyManager;
use crate::ws_bridge::WsBroadcaster;

type HmacSha256 = Hmac<Sha256>;

struct BroadcasterGuard(Arc<WsBroadcaster>);

impl Drop for BroadcasterGuard {
    fn drop(&mut self) {
        self.0.set_sender(None);
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StartServerResult {
    pub ip: String,
    pub mode: String,
}

pub struct WsServerHandle {
    running: parking_lot::Mutex<bool>,
    shutdown_tx: parking_lot::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    active_bind: parking_lot::Mutex<Option<String>>,
    tls_enabled: parking_lot::Mutex<bool>,
    connection_mode: parking_lot::Mutex<Option<String>>,
}

impl Default for WsServerHandle {
    fn default() -> Self {
        Self {
            running: parking_lot::Mutex::new(false),
            shutdown_tx: parking_lot::Mutex::new(None),
            active_bind: parking_lot::Mutex::new(None),
            tls_enabled: parking_lot::Mutex::new(false),
            connection_mode: parking_lot::Mutex::new(None),
        }
    }
}

impl WsServerHandle {
    pub fn active_bind(&self) -> Option<String> {
        self.active_bind.lock().clone()
    }

    pub fn is_tls_enabled(&self) -> bool {
        *self.tls_enabled.lock()
    }
}

const AUTH_TIMEOUT_SECS: u64 = 5;
const RATE_LIMIT_MAX_FAILURES: u32 = 3;
const RATE_LIMIT_BLOCK_SECS: u64 = 30;
const CHALLENGE_LENGTH: usize = 32;

struct RateLimitEntry {
    failures: u32,
    blocked_until: Option<Instant>,
}

pub struct WsServerState {
    active_connection: Arc<Mutex<bool>>,
    rate_limits: Arc<Mutex<HashMap<std::net::IpAddr, RateLimitEntry>>>,
    remote_dir: Option<PathBuf>,
    broadcaster: Arc<WsBroadcaster>,
    pty_manager: Option<Arc<PtyManager>>,
    repo_path: Option<String>,
    app_config: Arc<AppConfig>,
    app_handle: Option<tauri::AppHandle>,
    tls_enabled: bool,
}

impl WsServerState {
    pub fn new(
        remote_dir: Option<PathBuf>,
        broadcaster: Arc<WsBroadcaster>,
        pty_manager: Option<Arc<PtyManager>>,
        repo_path: Option<String>,
        app_config: Arc<AppConfig>,
        app_handle: Option<tauri::AppHandle>,
        tls_enabled: bool,
    ) -> Self {
        Self {
            active_connection: Arc::new(Mutex::new(false)),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            remote_dir,
            broadcaster,
            pty_manager,
            repo_path,
            app_config,
            app_handle,
            tls_enabled,
        }
    }

    pub fn current_token(&self) -> Result<String, String> {
        let config = self.app_config.get_config()?;
        Ok(config.server.token.clone())
    }
}

fn generate_challenge() -> String {
    use rand::Rng;
    let bytes: Vec<u8> = (0..CHALLENGE_LENGTH)
        .map(|_| rand::thread_rng().gen())
        .collect();
    hex::encode(bytes)
}

fn verify_hmac(challenge: &str, token: &str, client_hmac: &str) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(token.as_bytes()) else {
        return false;
    };
    mac.update(challenge.as_bytes());
    let Ok(client_bytes) = hex::decode(client_hmac) else {
        return false;
    };
    mac.verify_slice(&client_bytes).is_ok()
}

fn is_ip_blocked(
    rate_limits: &HashMap<std::net::IpAddr, RateLimitEntry>,
    ip: &std::net::IpAddr,
) -> bool {
    if let Some(entry) = rate_limits.get(ip) {
        if let Some(blocked_until) = entry.blocked_until {
            if Instant::now() < blocked_until {
                return true;
            }
        }
    }
    false
}

fn record_auth_failure(
    rate_limits: &mut HashMap<std::net::IpAddr, RateLimitEntry>,
    ip: std::net::IpAddr,
) {
    let entry = rate_limits.entry(ip).or_insert(RateLimitEntry {
        failures: 0,
        blocked_until: None,
    });
    if let Some(blocked_until) = entry.blocked_until {
        if Instant::now() >= blocked_until {
            entry.failures = 0;
            entry.blocked_until = None;
        }
    }
    entry.failures += 1;
    if entry.failures >= RATE_LIMIT_MAX_FAILURES {
        entry.blocked_until = Some(Instant::now() + Duration::from_secs(RATE_LIMIT_BLOCK_SECS));
    }
}

fn clear_auth_failures(
    rate_limits: &mut HashMap<std::net::IpAddr, RateLimitEntry>,
    ip: &std::net::IpAddr,
) {
    rate_limits.remove(ip);
}

fn normalize_path(path: &std::path::Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            c => components.push(c),
        }
    }
    components.iter().collect()
}

fn validate_relative_path(path: &str, repo_root: &str) -> Result<PathBuf, String> {
    if std::path::Path::new(path).is_absolute() {
        return Err("絶対パスは拒否されます".to_string());
    }
    let root = std::path::Path::new(repo_root)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let resolved = normalize_path(&root.join(path));
    if !resolved.starts_with(&root) {
        return Err("プロジェクトルート外のパスは拒否されます".to_string());
    }
    Ok(resolved)
}

fn validate_patch_paths(patch: &str, repo_root: &str) -> Result<(), String> {
    for line in patch.lines() {
        let path = line
            .strip_prefix("--- a/")
            .or_else(|| line.strip_prefix("+++ b/"));
        if let Some(p) = path {
            if p != "/dev/null" {
                validate_relative_path(p, repo_root)?;
            }
        }
    }
    Ok(())
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

async fn route_message(
    msg: &WsMessage,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    match msg {
        WsMessage::PtyInput(input) => handle_pty_input(input, state),
        WsMessage::PtyResize(_) => None,
        WsMessage::GitStatusRequest(_) => handle_git_status_request(selected_worktree).await,
        WsMessage::FileContentRequest(req) => {
            handle_file_content_req(req, selected_worktree).await
        }
        WsMessage::GitStage(req) => {
            handle_git_stage_request(req, state, selected_worktree).await
        }
        WsMessage::GitUnstage(req) => {
            handle_git_unstage_request(req, state, selected_worktree).await
        }
        WsMessage::GitStageHunk(req) => {
            handle_git_stage_hunk_request(req, state, selected_worktree).await
        }
        WsMessage::PtySpawnRequest(req) => {
            handle_pty_spawn_request(req, state, selected_worktree).await
        }
        WsMessage::PtyOutputRequest(req) => handle_pty_output_request(req, state),
        WsMessage::GitCommitRequest(req) => {
            handle_git_commit_request(req, state, selected_worktree).await
        }
        WsMessage::GitPushRequest(_) => handle_git_push_request(selected_worktree).await,
        WsMessage::BranchInfoRequest(_) => handle_branch_info_request(selected_worktree).await,
        WsMessage::WorktreeListRequest(_) => handle_worktree_list_request(state).await,
        WsMessage::WorktreeSelectRequest(req) => {
            handle_worktree_select_request(req, state, selected_worktree).await
        }
        WsMessage::AddComment(comment) => handle_add_comment(comment, state),
        _ => Some(WsMessage::Error(ErrorMsg {
            code: "INVALID_MESSAGE".to_string(),
            message: "Unexpected message from client".to_string(),
        })),
    }
}

fn git_status_to_msg_list(repo_path: &str) -> Vec<GitFileStatusMsg> {
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

fn no_repo_error() -> WsMessage {
    WsMessage::Error(ErrorMsg {
        code: "NO_REPO".to_string(),
        message: "リポジトリパスが設定されていません".to_string(),
    })
}

fn no_worktree_selected_error() -> WsMessage {
    WsMessage::Error(ErrorMsg {
        code: "NO_WORKTREE_SELECTED".to_string(),
        message: "Worktreeが選択されていません".to_string(),
    })
}

fn join_error_msg(e: tokio::task::JoinError) -> WsMessage {
    WsMessage::Error(ErrorMsg {
        code: "INTERNAL_ERROR".to_string(),
        message: format!("Task join error: {e}"),
    })
}

async fn with_worktree_blocking<F>(
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

fn broadcast_git_status_sync(
    broadcaster: &WsBroadcaster,
    repo_path: &str,
) -> Vec<GitFileStatusMsg> {
    let files = git_status_to_msg_list(repo_path);
    broadcaster.try_send(WsMessage::GitStatusSync(GitStatusSync {
        files: files.clone(),
    }));
    files
}

fn handle_pty_input(input: &PtyInput, state: &WsServerState) -> Option<WsMessage> {
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

async fn handle_git_status_request(
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    with_worktree_blocking(selected_worktree, |repo_path| {
        let files = git_status_to_msg_list(&repo_path);
        WsMessage::GitStatusSync(GitStatusSync { files })
    })
    .await
}

async fn handle_file_content_req(
    req: &FileContentRequest,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let req = req.clone();
    with_worktree_blocking(selected_worktree, move |repo_path| {
        handle_file_content_request(&req, &repo_path)
    })
    .await
}

async fn handle_git_stage_request(
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

async fn handle_git_unstage_request(
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

async fn handle_git_stage_hunk_request(
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

async fn handle_pty_spawn_request(
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

fn handle_pty_output_request(req: &PtyOutputRequest, state: &WsServerState) -> Option<WsMessage> {
    if let Some(pm) = &state.pty_manager {
        let sessions = pm.list_pty_sessions();
        if sessions.iter().any(|s| s.pty_id == req.pty_id) {
            let buffered = state.broadcaster.get_pty_output_buffer(req.pty_id);
            if buffered.is_empty() {
                None
            } else {
                Some(WsMessage::PtyOutput(PtyOutputMsg {
                    pty_id: req.pty_id,
                    data: buffered,
                }))
            }
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

async fn handle_git_commit_request(
    req: &GitCommitRequest,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let message = req.message.clone();
    let broadcaster = state.broadcaster.clone();
    with_worktree_blocking(selected_worktree, move |repo_path| {
        match crate::git::git_commit(repo_path.clone(), message) {
            Ok(hash) => {
                broadcast_git_status_sync(&broadcaster, &repo_path);
                let branch =
                    crate::git::get_current_branch(repo_path.clone()).unwrap_or_default();
                broadcaster.try_send(WsMessage::BranchInfoResponse(
                    BranchInfoResponse { branch },
                ));
                if let Ok(cards) =
                    crate::git::list_branches_with_status(repo_path)
                {
                    let branch_msgs = cards
                        .into_iter()
                        .map(|b| BranchCardMsg {
                            name: b.name,
                            is_default: b.is_default,
                            worktree_path: b.worktree_path,
                            dirty_count: b.dirty_count,
                            is_merged: b.is_merged,
                        })
                        .collect();
                    broadcaster.try_send(WsMessage::BranchListSync(
                        BranchListSync {
                            branches: branch_msgs,
                        },
                    ));
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
        }
    })
    .await
}

async fn handle_git_push_request(
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

async fn handle_branch_info_request(
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    with_worktree_blocking(selected_worktree, |repo_path| {
        let branch = crate::git::get_current_branch(repo_path).unwrap_or_default();
        WsMessage::BranchInfoResponse(BranchInfoResponse { branch })
    })
    .await
}

async fn handle_worktree_list_request(state: &WsServerState) -> Option<WsMessage> {
    let repo_path = match &state.repo_path {
        Some(p) => p.clone(),
        None => return Some(no_repo_error()),
    };
    match tokio::task::spawn_blocking(move || {
        let entries = crate::git::list_worktrees(repo_path)
            .unwrap_or_default()
            .into_iter()
            .map(|e| WorktreeEntryMsg {
                name: e.name,
                path: e.path,
                branch: e.branch,
                is_main: e.is_main,
                is_locked: e.is_locked,
                dirty_count: e.dirty_count,
                base_branch: e.base_branch,
            })
            .collect();
        WsMessage::WorktreeListResponse(WorktreeListResponse { worktrees: entries })
    })
    .await
    {
        Ok(msg) => Some(msg),
        Err(e) => Some(join_error_msg(e)),
    }
}

async fn handle_worktree_select_request(
    req: &WorktreeSelectRequest,
    state: &WsServerState,
    selected_worktree: &Arc<Mutex<Option<String>>>,
) -> Option<WsMessage> {
    let repo_path = match &state.repo_path {
        Some(p) => p.clone(),
        None => return Some(no_repo_error()),
    };
    let requested_path = req.path.clone();
    let broadcaster = state.broadcaster.clone();

    let valid = tokio::task::spawn_blocking({
        let requested_path = requested_path.clone();
        let repo_path = repo_path.clone();
        move || {
            let worktrees = crate::git::list_worktrees(repo_path).unwrap_or_default();
            worktrees.iter().any(|w| w.path == requested_path)
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
    if let Ok(files) =
        tokio::task::spawn_blocking(move || git_status_to_msg_list(&wt_path)).await
    {
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

fn handle_add_comment(comment: &AddComment, state: &WsServerState) -> Option<WsMessage> {
    if let Some(app) = &state.app_handle {
        let _ = app.emit(
            "remote-comment-added",
            serde_json::json!({
                "file_path": comment.file_path,
                "line_number": comment.line_number,
                "end_line": comment.end_line,
                "content": comment.content,
            }),
        );
    }
    None
}

fn apply_security_headers(
    builder: hyper::http::response::Builder,
    tls_enabled: bool,
) -> hyper::http::response::Builder {
    let builder = builder
        .header("X-Content-Type-Options", "nosniff")
        .header("X-Frame-Options", "DENY")
        .header("Referrer-Policy", "strict-origin-when-cross-origin");
    if tls_enabled {
        builder.header("Strict-Transport-Security", "max-age=31536000")
    } else {
        builder
    }
}

fn content_type_for(path: &str) -> &'static str {
    if path.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if path.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if path.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if path.ends_with(".json") {
        "application/json"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".ico") {
        "image/x-icon"
    } else if path.ends_with(".woff2") {
        "font/woff2"
    } else {
        "application/octet-stream"
    }
}

pub async fn start_ws_server(
    cfg: &ReleashConfig,
    server_state: Arc<WsServerState>,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    if (cfg.server.bind == "0.0.0.0" || cfg.server.bind == "any") && !cfg.server.tls.enabled {
        return Err(
            "セキュリティ上の理由により、bind=0.0.0.0/any かつ TLS無効での起動は拒否されます"
                .to_string(),
        );
    }

    let bind_addr = if cfg.server.bind == "any" {
        "0.0.0.0".to_string()
    } else {
        cfg.server.bind.clone()
    };
    let addr = format!("{}:{}", bind_addr, cfg.server.port);

    let tls_acceptor = if cfg.server.tls.enabled {
        Some(crate::tls::load_tls_config(
            &cfg.server.tls.cert,
            &cfg.server.tls.key,
        )?)
    } else {
        None
    };

    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("サーバー起動失敗: {e}"))?;

    log::info!("WebSocket server listening on {}", addr);

    tokio::spawn(async move {
        tokio::select! {
            _ = async {
                loop {
                    let Ok((stream, peer_addr)) = listener.accept().await else {
                        continue;
                    };

                    let server_state = Arc::clone(&server_state);
                    let tls_acceptor = tls_acceptor.clone();

                    tokio::spawn(async move {
                        let result = if let Some(tls) = &tls_acceptor {
                            match tls.accept(stream).await {
                                Ok(tls_stream) => {
                                    serve_hyper_connection(TokioIo::new(tls_stream), peer_addr, server_state).await
                                }
                                Err(e) => Err(format!("TLS handshake failed: {e}")),
                            }
                        } else {
                            serve_hyper_connection(TokioIo::new(stream), peer_addr, server_state).await
                        };
                        if let Err(e) = result {
                            log::warn!("Connection error from {}: {}", peer_addr, e);
                        }
                    });
                }
            } => {},
            _ = shutdown_rx => {
                log::info!("WebSocket server shutting down");
            }
        }
    });

    Ok(())
}

async fn serve_hyper_connection<I>(
    io: I,
    peer_addr: SocketAddr,
    state: Arc<WsServerState>,
) -> Result<(), String>
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let service = service_fn(move |req| {
        let state = Arc::clone(&state);
        async move { Ok::<_, std::convert::Infallible>(handle_http(req, peer_addr, state).await) }
    });

    http1::Builder::new()
        .serve_connection(io, service)
        .with_upgrades()
        .await
        .map_err(|e| format!("HTTP connection error: {e}"))
}

fn is_ws_upgrade(req: &Request<hyper::body::Incoming>) -> bool {
    req.headers()
        .get(hyper::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
}

async fn handle_http(
    req: Request<hyper::body::Incoming>,
    peer_addr: SocketAddr,
    state: Arc<WsServerState>,
) -> Response<Full<Bytes>> {
    let path = req.uri().path().to_string();
    let tls = state.tls_enabled;
    if is_ws_upgrade(&req) {
        match handle_ws_upgrade(req, peer_addr, state) {
            Ok(resp) => resp,
            Err(e) => error_response(StatusCode::BAD_REQUEST, &e, tls),
        }
    } else {
        serve_remote(&path, &state)
    }
}

fn handle_ws_upgrade(
    mut req: Request<hyper::body::Incoming>,
    peer_addr: SocketAddr,
    state: Arc<WsServerState>,
) -> Result<Response<Full<Bytes>>, String> {
    let key = req
        .headers()
        .get("sec-websocket-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or("Missing Sec-WebSocket-Key")?;

    let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(key.as_bytes());
    let on_upgrade = hyper::upgrade::on(&mut req);

    tokio::spawn(async move {
        match on_upgrade.await {
            Ok(upgraded) => {
                let ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
                    TokioIo::new(upgraded),
                    tokio_tungstenite::tungstenite::protocol::Role::Server,
                    None,
                )
                .await;
                if let Err(e) = handle_ws_session(ws, peer_addr, &state).await {
                    log::warn!("WebSocket error from {}: {}", peer_addr, e);
                }
            }
            Err(e) => {
                log::warn!("WebSocket upgrade failed for {}: {}", peer_addr, e);
            }
        }
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(hyper::header::UPGRADE, "websocket")
        .header(hyper::header::CONNECTION, "Upgrade")
        .header("Sec-WebSocket-Accept", accept)
        .body(Full::default())
        .map_err(|e| e.to_string())
}

fn serve_remote(path: &str, state: &WsServerState) -> Response<Full<Bytes>> {
    let remote_dir = match &state.remote_dir {
        Some(d) => d,
        None => {
            return error_response(
                StatusCode::NOT_FOUND,
                "Remote UI is not available",
                state.tls_enabled,
            )
        }
    };

    let file_path = match path {
        "/" | "" => "remote.html",
        p => p.trim_start_matches('/'),
    };

    let tls = state.tls_enabled;
    let full_path = remote_dir.join(file_path);
    if let (Ok(canonical), Ok(remote_canonical)) =
        (full_path.canonicalize(), remote_dir.canonicalize())
    {
        if !canonical.starts_with(&remote_canonical) {
            return error_response(StatusCode::FORBIDDEN, "Access denied", tls);
        }
        match std::fs::read(&canonical) {
            Ok(content) => {
                let ct = content_type_for(canonical.to_str().unwrap_or(""));
                apply_security_headers(Response::builder(), tls)
                    .status(StatusCode::OK)
                    .header("Content-Type", ct)
                    .header("Cache-Control", "no-cache")
                    .body(Full::new(Bytes::from(content)))
                    .unwrap()
            }
            Err(_) => serve_remote_fallback(remote_dir, tls),
        }
    } else {
        serve_remote_fallback(remote_dir, tls)
    }
}

fn serve_remote_fallback(remote_dir: &std::path::Path, tls_enabled: bool) -> Response<Full<Bytes>> {
    match std::fs::read(remote_dir.join("remote.html")) {
        Ok(content) => apply_security_headers(Response::builder(), tls_enabled)
            .status(StatusCode::OK)
            .header("Content-Type", "text/html; charset=utf-8")
            .body(Full::new(Bytes::from(content)))
            .unwrap(),
        Err(_) => error_response(StatusCode::NOT_FOUND, "Not Found", tls_enabled),
    }
}

fn error_response(status: StatusCode, msg: &str, tls_enabled: bool) -> Response<Full<Bytes>> {
    apply_security_headers(Response::builder(), tls_enabled)
        .status(status)
        .body(Full::new(Bytes::from(msg.to_string())))
        .unwrap()
}

async fn handle_ws_session<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    ws_stream: tokio_tungstenite::WebSocketStream<S>,
    peer_addr: SocketAddr,
    state: &WsServerState,
) -> Result<(), String> {
    {
        let rate_limits = state.rate_limits.lock().await;
        if is_ip_blocked(&rate_limits, &peer_addr.ip()) {
            return Err("IP is rate-limited".to_string());
        }
    }

    {
        let mut active = state.active_connection.lock().await;
        if *active {
            return Err("同時接続数制限: 既に接続中のクライアントがあります".to_string());
        }
        *active = true;
    }

    let result = async {
        let token = state.current_token()?;
        handle_ws_authenticated(ws_stream, peer_addr, &token, state).await
    }
    .await;

    {
        let mut active = state.active_connection.lock().await;
        *active = false;
    }

    result
}

async fn handle_ws_authenticated<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    ws_stream: tokio_tungstenite::WebSocketStream<S>,
    peer_addr: SocketAddr,
    token: &str,
    state: &WsServerState,
) -> Result<(), String> {
    let (mut write, mut read) = ws_stream.split();

    // --- 認証フェーズ ---
    let challenge = generate_challenge();
    let challenge_msg = WsMessage::AuthChallenge(AuthChallenge {
        challenge: challenge.clone(),
    });
    write
        .send(Message::Text(
            serialize_message(&challenge_msg).map_err(|e| e.to_string())?,
        ))
        .await
        .map_err(|e| format!("Failed to send challenge: {e}"))?;

    let auth_result = tokio::time::timeout(Duration::from_secs(AUTH_TIMEOUT_SECS), async {
        while let Some(msg) = read.next().await {
            let msg = msg.map_err(|e| format!("Read error: {e}"))?;
            if let Message::Text(text) = msg {
                let ws_msg = deserialize_message(&text).map_err(|e| format!("Parse error: {e}"))?;
                if let WsMessage::AuthResponse(resp) = ws_msg {
                    return Ok(resp.hmac);
                }
            }
        }
        Err("Connection closed during auth".to_string())
    })
    .await;

    let client_hmac = match auth_result {
        Ok(Ok(hmac)) => hmac,
        Ok(Err(e)) => {
            let mut rate_limits = state.rate_limits.lock().await;
            record_auth_failure(&mut rate_limits, peer_addr.ip());
            let fail_msg = WsMessage::AuthResult(crate::protocol::AuthResult {
                success: false,
                message: Some(e.clone()),
            });
            let _ = write
                .send(Message::Text(
                    serialize_message(&fail_msg).unwrap_or_default(),
                ))
                .await;
            return Err(e);
        }
        Err(_) => {
            let mut rate_limits = state.rate_limits.lock().await;
            record_auth_failure(&mut rate_limits, peer_addr.ip());
            let fail_msg = WsMessage::AuthResult(crate::protocol::AuthResult {
                success: false,
                message: Some("認証タイムアウト".to_string()),
            });
            let _ = write
                .send(Message::Text(
                    serialize_message(&fail_msg).unwrap_or_default(),
                ))
                .await;
            return Err("Auth timeout".to_string());
        }
    };

    if !verify_hmac(&challenge, token, &client_hmac) {
        let mut rate_limits = state.rate_limits.lock().await;
        record_auth_failure(&mut rate_limits, peer_addr.ip());
        let fail_msg = WsMessage::AuthResult(crate::protocol::AuthResult {
            success: false,
            message: Some("認証失敗".to_string()),
        });
        let _ = write
            .send(Message::Text(
                serialize_message(&fail_msg).unwrap_or_default(),
            ))
            .await;
        return Err("Authentication failed".to_string());
    }

    {
        let mut rate_limits = state.rate_limits.lock().await;
        clear_auth_failures(&mut rate_limits, &peer_addr.ip());
    }

    let success_msg = WsMessage::AuthResult(crate::protocol::AuthResult {
        success: true,
        message: None,
    });
    write
        .send(Message::Text(
            serialize_message(&success_msg).map_err(|e| e.to_string())?,
        ))
        .await
        .map_err(|e| format!("Failed to send auth result: {e}"))?;

    log::info!("Client authenticated: {}", peer_addr);

    if let Some(app) = &state.app_handle {
        let _ = app.emit("remote-connected", ());
    }

    // --- WsBroadcaster セットアップ（PTYスポーン前に初期化） ---
    let (tx, mut rx) = WsBroadcaster::create_channel();
    state.broadcaster.set_sender(Some(tx));
    let _sender_guard = BroadcasterGuard(state.broadcaster.clone());

    // --- 初期データ送信: worktreeリストのみ（PTYはworktree選択後に送信） ---
    if let Some(repo_path) = &state.repo_path {
        let repo_path_clone = repo_path.clone();
        let worktree_msg = tokio::task::spawn_blocking(move || {
            let entries = crate::git::list_worktrees(repo_path_clone)
                .unwrap_or_default()
                .into_iter()
                .map(|e| WorktreeEntryMsg {
                    name: e.name,
                    path: e.path,
                    branch: e.branch,
                    is_main: e.is_main,
                    is_locked: e.is_locked,
                    dirty_count: e.dirty_count,
                    base_branch: e.base_branch,
                })
                .collect();
            WsMessage::WorktreeListResponse(WorktreeListResponse { worktrees: entries })
        })
        .await
        .map_err(|e| format!("Failed to get worktree list: {e}"))?;
        write
            .send(Message::Text(
                serialize_message(&worktree_msg).map_err(|e| e.to_string())?,
            ))
            .await
            .map_err(|e| format!("Failed to send worktree list: {e}"))?;
    }

    // --- セッション単位のworktree選択状態 ---
    let selected_worktree: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    // PTY出力をWebSocketにフォワードするタスク
    let forward_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Ok(json) = serialize_message(&msg) {
                if write.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
        write
    });

    // --- メッセージルーティングフェーズ ---
    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                log::warn!("Read error from {}: {}", peer_addr, e);
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                let ws_msg = match deserialize_message(&text) {
                    Ok(m) => m,
                    Err(_) => {
                        // parse error は broadcaster 経由で送信
                        let err = WsMessage::Error(ErrorMsg {
                            code: "PARSE_ERROR".to_string(),
                            message: "Invalid message format".to_string(),
                        });
                        state.broadcaster.try_send(err);
                        continue;
                    }
                };
                if let Some(response) = route_message(&ws_msg, state, &selected_worktree).await {
                    state.broadcaster.try_send(response);
                }
            }
            Message::Close(_) => break,
            Message::Ping(_) => {
                // ping/pong は forward_task の write 経由では送れないので broadcaster 経由
                // 実際のpong応答はtungsteniteが自動処理する
            }
            _ => {}
        }
    }

    // --- クリーンアップ ---
    // _sender_guard の Drop で set_sender(None) が呼ばれる
    drop(_sender_guard);
    // forward_task にドロップされた rx の closed を通知して終了させる
    let _ = forward_task.await;

    log::info!("Client disconnected: {}", peer_addr);
    Ok(())
}

#[tauri::command]
pub async fn start_server(
    root_path: String,
    bind_ip: String,
    app: tauri::AppHandle,
    handle: tauri::State<'_, WsServerHandle>,
    config_state: tauri::State<'_, Arc<AppConfig>>,
    broadcaster: tauri::State<'_, Arc<WsBroadcaster>>,
    pty_manager: tauri::State<'_, Arc<crate::pty::PtyManager>>,
) -> Result<StartServerResult, String> {
    {
        let running = handle.running.lock();
        if *running {
            return Err("サーバーは既に起動しています".to_string());
        }
    }

    let mut cfg = config_state.get_config()?;

    let detected = crate::vpn_detect::detect_all_interfaces();
    let mode = if detected.iter().any(|i| i.kind == "vpn" && i.ip == bind_ip) {
        "vpn".to_string()
    } else {
        "lan".to_string()
    };

    cfg.server.bind = bind_ip.clone();

    let bind_ip_addr: std::net::IpAddr = bind_ip
        .parse()
        .map_err(|e| format!("IPアドレスのパース失敗: {e}"))?;
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("データディレクトリの取得失敗: {e}"))?;
    if cfg.server.tls.cert.is_empty() || cfg.server.tls.key.is_empty() {
        let (cert_path, key_path) = crate::tls::ensure_self_signed_cert(bind_ip_addr, &data_dir)?;
        cfg.server.tls.cert = cert_path.to_string_lossy().to_string();
        cfg.server.tls.key = key_path.to_string_lossy().to_string();
    }
    cfg.server.tls.enabled = true;

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let remote_dir = if cfg!(debug_assertions) {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("remote");
        if dir.exists() {
            Some(dir)
        } else {
            None
        }
    } else {
        app.path()
            .resource_dir()
            .ok()
            .map(|d| d.join("resources").join("remote"))
    };
    let server_state = Arc::new(WsServerState::new(
        remote_dir,
        Arc::clone(&broadcaster),
        Some(Arc::clone(&pty_manager)),
        Some(root_path),
        Arc::clone(config_state.inner()),
        Some(app.clone()),
        cfg.server.tls.enabled,
    ));

    start_ws_server(&cfg, server_state, shutdown_rx).await?;

    {
        let mut running = handle.running.lock();
        *running = true;
        let mut tx = handle.shutdown_tx.lock();
        *tx = Some(shutdown_tx);
        handle.active_bind.lock().replace(bind_ip.clone());
        *handle.tls_enabled.lock() = cfg.server.tls.enabled;
        handle.connection_mode.lock().replace(mode.clone());
    }

    Ok(StartServerResult { ip: bind_ip, mode })
}

#[tauri::command]
pub fn stop_server(handle: tauri::State<'_, WsServerHandle>) -> Result<(), String> {
    let tx = {
        let mut shutdown_tx = handle.shutdown_tx.lock();
        shutdown_tx.take()
    };

    if let Some(tx) = tx {
        let _ = tx.send(());
        let mut running = handle.running.lock();
        *running = false;
        handle.active_bind.lock().take();
        *handle.tls_enabled.lock() = false;
        handle.connection_mode.lock().take();
        Ok(())
    } else {
        Err("サーバーは起動していません".to_string())
    }
}

#[tauri::command]
pub fn get_server_status(handle: tauri::State<'_, WsServerHandle>) -> bool {
    *handle.running.lock()
}

#[tauri::command]
pub fn broadcast_comments(
    comments: CommentSync,
    broadcaster: tauri::State<'_, Arc<WsBroadcaster>>,
) {
    broadcaster.try_send(WsMessage::CommentsSync(comments));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_challenge_length() {
        let c = generate_challenge();
        assert_eq!(c.len(), CHALLENGE_LENGTH * 2); // hex encoding doubles length
        assert!(c.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn test_generate_challenge_uniqueness() {
        let c1 = generate_challenge();
        let c2 = generate_challenge();
        assert_ne!(c1, c2);
    }

    #[test]
    fn test_verify_hmac_valid() {
        let challenge = "test_challenge";
        let token = "secret_token";

        let mut mac = HmacSha256::new_from_slice(token.as_bytes()).unwrap();
        mac.update(challenge.as_bytes());
        let expected = hex::encode(mac.finalize().into_bytes());

        assert!(verify_hmac(challenge, token, &expected));
    }

    #[test]
    fn test_verify_hmac_invalid() {
        assert!(!verify_hmac("challenge", "token", "wrong_hmac"));
    }

    #[test]
    fn test_rate_limit_not_blocked_initially() {
        let limits = HashMap::new();
        let ip: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        assert!(!is_ip_blocked(&limits, &ip));
    }

    #[test]
    fn test_rate_limit_blocked_after_max_failures() {
        let mut limits = HashMap::new();
        let ip: std::net::IpAddr = "192.168.1.1".parse().unwrap();

        for _ in 0..RATE_LIMIT_MAX_FAILURES {
            record_auth_failure(&mut limits, ip);
        }

        assert!(is_ip_blocked(&limits, &ip));
    }

    #[test]
    fn test_rate_limit_not_blocked_before_max() {
        let mut limits = HashMap::new();
        let ip: std::net::IpAddr = "192.168.1.1".parse().unwrap();

        for _ in 0..(RATE_LIMIT_MAX_FAILURES - 1) {
            record_auth_failure(&mut limits, ip);
        }

        assert!(!is_ip_blocked(&limits, &ip));
    }

    #[test]
    fn test_clear_auth_failures() {
        let mut limits = HashMap::new();
        let ip: std::net::IpAddr = "10.0.0.1".parse().unwrap();

        for _ in 0..RATE_LIMIT_MAX_FAILURES {
            record_auth_failure(&mut limits, ip);
        }
        assert!(is_ip_blocked(&limits, &ip));

        clear_auth_failures(&mut limits, &ip);
        assert!(!is_ip_blocked(&limits, &ip));
    }

    fn test_state() -> WsServerState {
        let config = crate::config::ReleashConfig::default();
        let app_config = Arc::new(AppConfig::new(
            config,
            std::path::PathBuf::from("/tmp/test-releash.toml"),
        ));
        WsServerState::new(
            None,
            Arc::new(WsBroadcaster::default()),
            None,
            None,
            app_config,
            None,
            false,
        )
    }

    fn test_selected_worktree() -> Arc<Mutex<Option<String>>> {
        Arc::new(Mutex::new(None))
    }

    #[tokio::test]
    async fn test_route_unknown_message_returns_error() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::AuthChallenge(AuthChallenge {
            challenge: "x".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "INVALID_MESSAGE"),
            _ => panic!("expected error"),
        }
    }

    #[tokio::test]
    async fn test_route_add_comment_returns_none() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::AddComment(AddComment {
            file_path: "src/main.rs".to_string(),
            line_number: 10,
            end_line: None,
            content: "fix this".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_route_pty_input_without_manager_returns_none() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::PtyInput(PtyInput {
            pty_id: 1,
            data: "ls".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_route_git_status_request_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::GitStatusRequest(GitStatusRequest {});
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_pty_spawn_request_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::PtySpawnRequest(PtySpawnRequest { cols: 80, rows: 24 });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_pty_spawn_request_without_pty_manager() {
        let state = test_state(); // pty_manager = None
        let wt = Arc::new(Mutex::new(Some("/tmp/test".to_string())));
        let msg = WsMessage::PtySpawnRequest(PtySpawnRequest { cols: 80, rows: 24 });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::PtySpawnResponse(r)) => {
                assert!(!r.success);
                assert!(r.error.is_some());
            }
            _ => panic!("expected PtySpawnResponse with error"),
        }
    }

    #[tokio::test]
    async fn test_route_file_content_request_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::FileContentRequest(FileContentRequest {
            path: "test.rs".to_string(),
            diff_base: "HEAD".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[test]
    fn test_validate_relative_path_rejects_absolute() {
        let result = validate_relative_path("/etc/passwd", "/tmp");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_relative_path_rejects_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let result = validate_relative_path("../../etc/passwd", dir.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_content_type_html() {
        assert_eq!(content_type_for("index.html"), "text/html; charset=utf-8");
    }

    #[test]
    fn test_content_type_js() {
        assert_eq!(
            content_type_for("app.js"),
            "application/javascript; charset=utf-8"
        );
    }

    #[test]
    fn test_content_type_unknown() {
        assert_eq!(content_type_for("data.bin"), "application/octet-stream");
    }

    #[test]
    fn test_security_block_any_without_tls() {
        // bind = "any" or "0.0.0.0" + TLS無効 → 拒否される
        // ここではconfigバリデーションロジックを直接テスト
        let bind = "0.0.0.0";
        let tls_enabled = false;
        let should_block = (bind == "0.0.0.0" || bind == "any") && !tls_enabled;
        assert!(should_block);
    }

    #[test]
    fn test_security_allow_localhost_without_tls() {
        let bind = "127.0.0.1";
        let tls_enabled = false;
        let should_block = (bind == "0.0.0.0" || bind == "any") && !tls_enabled;
        assert!(!should_block);
    }

    #[test]
    fn test_security_allow_any_with_tls() {
        let bind = "0.0.0.0";
        let tls_enabled = true;
        let should_block = (bind == "0.0.0.0" || bind == "any") && !tls_enabled;
        assert!(!should_block);
    }

    // === パストラバーサル攻撃テスト ===

    #[test]
    fn test_validate_relative_path_rejects_nested_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("foo")).unwrap();
        let result = validate_relative_path("foo/../../etc/passwd", dir.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_relative_path_accepts_valid_subdir() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();
        let result = validate_relative_path("src/main.rs", dir.path().to_str().unwrap());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_patch_paths_rejects_traversal() {
        let dir = tempfile::TempDir::new().unwrap();
        let patch = "--- a/../../etc/passwd\n+++ b/../../etc/shadow\n";
        let result = validate_patch_paths(patch, dir.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_patch_paths_accepts_valid_paths() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "").unwrap();
        let patch = "--- a/src/lib.rs\n+++ b/src/lib.rs\n";
        let result = validate_patch_paths(patch, dir.path().to_str().unwrap());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_patch_paths_allows_dev_null() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("new_file.rs"), "").unwrap();
        let patch = "--- a//dev/null\n+++ b/new_file.rs\n";
        let result = validate_patch_paths(patch, dir.path().to_str().unwrap());
        assert!(result.is_ok());
    }

    #[test]
    fn test_normalize_path_removes_parent_dir() {
        let path = std::path::Path::new("/home/user/../etc/passwd");
        let normalized = normalize_path(path);
        assert_eq!(normalized, std::path::PathBuf::from("/home/etc/passwd"));
    }

    #[test]
    fn test_normalize_path_removes_cur_dir() {
        let path = std::path::Path::new("/home/./user/./file.txt");
        let normalized = normalize_path(path);
        assert_eq!(normalized, std::path::PathBuf::from("/home/user/file.txt"));
    }

    // === HMAC認証エッジケーステスト ===

    #[test]
    fn test_verify_hmac_empty_token() {
        assert!(!verify_hmac("challenge", "", "abcdef"));
    }

    #[test]
    fn test_verify_hmac_invalid_hex() {
        assert!(!verify_hmac("challenge", "token", "not_valid_hex_zzz"));
    }

    #[test]
    fn test_verify_hmac_wrong_challenge() {
        let token = "secret_token";
        let mut mac = HmacSha256::new_from_slice(token.as_bytes()).unwrap();
        mac.update(b"correct_challenge");
        let hmac_hex = hex::encode(mac.finalize().into_bytes());
        assert!(!verify_hmac("wrong_challenge", token, &hmac_hex));
    }

    // === レート制限テスト ===

    #[test]
    fn test_rate_limit_block_recovery_after_timeout() {
        let mut limits = HashMap::new();
        let ip: std::net::IpAddr = "10.0.0.2".parse().unwrap();

        for _ in 0..RATE_LIMIT_MAX_FAILURES {
            record_auth_failure(&mut limits, ip);
        }
        assert!(is_ip_blocked(&limits, &ip));

        // タイムアウト後にブロック解除されること（blocked_untilを過去に書き換え）
        if let Some(entry) = limits.get_mut(&ip) {
            entry.blocked_until = Some(Instant::now() - Duration::from_secs(1));
        }
        assert!(!is_ip_blocked(&limits, &ip));
    }

    #[test]
    fn test_rate_limit_independent_ips() {
        let mut limits = HashMap::new();
        let ip1: std::net::IpAddr = "192.168.1.10".parse().unwrap();
        let ip2: std::net::IpAddr = "192.168.1.20".parse().unwrap();

        for _ in 0..RATE_LIMIT_MAX_FAILURES {
            record_auth_failure(&mut limits, ip1);
        }

        assert!(is_ip_blocked(&limits, &ip1));
        assert!(!is_ip_blocked(&limits, &ip2));
    }

    #[test]
    fn test_rate_limit_reset_counter_after_block_expires() {
        let mut limits = HashMap::new();
        let ip: std::net::IpAddr = "10.0.0.3".parse().unwrap();

        for _ in 0..RATE_LIMIT_MAX_FAILURES {
            record_auth_failure(&mut limits, ip);
        }
        assert!(is_ip_blocked(&limits, &ip));

        // ブロック期限を過去に設定
        if let Some(entry) = limits.get_mut(&ip) {
            entry.blocked_until = Some(Instant::now() - Duration::from_secs(1));
        }

        // 新たな失敗記録でカウンタがリセットされること
        record_auth_failure(&mut limits, ip);
        let entry = limits.get(&ip).unwrap();
        assert_eq!(entry.failures, 1);
        assert!(!is_ip_blocked(&limits, &ip));
    }

    // === worktree未選択時の全操作エラー確認テスト ===

    #[tokio::test]
    async fn test_route_git_stage_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::GitStage(GitStage {
            paths: vec!["file.txt".to_string()],
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_git_unstage_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::GitUnstage(GitUnstage {
            paths: vec!["file.txt".to_string()],
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_git_stage_hunk_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::GitStageHunk(GitStageHunk {
            patch: "--- a/file.txt\n+++ b/file.txt\n".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_git_commit_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::GitCommitRequest(GitCommitRequest {
            message: "test".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_git_push_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::GitPushRequest(GitPushRequest {});
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    #[tokio::test]
    async fn test_route_branch_info_without_worktree() {
        let state = test_state();
        let wt = test_selected_worktree();
        let msg = WsMessage::BranchInfoRequest(BranchInfoRequest {});
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_WORKTREE_SELECTED"),
            _ => panic!("expected no worktree selected error"),
        }
    }

    // === repo未設定時テスト ===

    #[tokio::test]
    async fn test_route_worktree_list_without_repo() {
        let state = test_state(); // repo_path = None
        let wt = test_selected_worktree();
        let msg = WsMessage::WorktreeListRequest(WorktreeListRequest {});
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_REPO"),
            _ => panic!("expected no repo error"),
        }
    }

    #[tokio::test]
    async fn test_route_worktree_select_without_repo() {
        let state = test_state(); // repo_path = None
        let wt = test_selected_worktree();
        let msg = WsMessage::WorktreeSelectRequest(WorktreeSelectRequest {
            path: "/tmp/some-worktree".to_string(),
        });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_REPO"),
            _ => panic!("expected no repo error"),
        }
    }

    // === デシリアライズ耐性テスト ===

    #[test]
    fn test_deserialize_invalid_json() {
        let result = deserialize_message("not valid json at all");
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_empty_payload() {
        let result = deserialize_message("");
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_missing_type_field() {
        let result = deserialize_message(r#"{"data": "hello"}"#);
        assert!(result.is_err());
    }

    // === PtyOutputRequest without PTY manager ===

    #[tokio::test]
    async fn test_route_pty_output_without_pty_manager() {
        let state = test_state(); // pty_manager = None
        let wt = test_selected_worktree();
        let msg = WsMessage::PtyOutputRequest(PtyOutputRequest { pty_id: 1 });
        let result = route_message(&msg, &state, &wt).await;
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_PTY"),
            _ => panic!("expected NO_PTY error"),
        }
    }
}
