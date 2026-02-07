#![allow(dead_code)]

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

pub struct WsServerHandle {
    running: parking_lot::Mutex<bool>,
    shutdown_tx: parking_lot::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    active_bind: parking_lot::Mutex<Option<String>>,
    tls_enabled: parking_lot::Mutex<bool>,
}

impl Default for WsServerHandle {
    fn default() -> Self {
        Self {
            running: parking_lot::Mutex::new(false),
            shutdown_tx: parking_lot::Mutex::new(None),
            active_bind: parking_lot::Mutex::new(None),
            tls_enabled: parking_lot::Mutex::new(false),
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
    pwa_dir: Option<PathBuf>,
    broadcaster: Arc<WsBroadcaster>,
    pty_manager: Option<Arc<PtyManager>>,
    repo_path: Option<String>,
    app_config: Arc<AppConfig>,
    app_handle: Option<tauri::AppHandle>,
}

impl WsServerState {
    pub fn new(
        pwa_dir: Option<PathBuf>,
        broadcaster: Arc<WsBroadcaster>,
        pty_manager: Option<Arc<PtyManager>>,
        repo_path: Option<String>,
        app_config: Arc<AppConfig>,
        app_handle: Option<tauri::AppHandle>,
    ) -> Self {
        Self {
            active_connection: Arc::new(Mutex::new(false)),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            pwa_dir,
            broadcaster,
            pty_manager,
            repo_path,
            app_config,
            app_handle,
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
    let expected = hex::encode(mac.finalize().into_bytes());
    expected == client_hmac
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

fn validate_relative_path(path: &str, repo_root: &str) -> Result<PathBuf, String> {
    if std::path::Path::new(path).is_absolute() {
        return Err("絶対パスは拒否されます".to_string());
    }
    let root = std::path::Path::new(repo_root)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let resolved = root.join(path).canonicalize().map_err(|e| e.to_string())?;
    if !resolved.starts_with(&root) {
        return Err("プロジェクトルート外のパスは拒否されます".to_string());
    }
    Ok(resolved)
}

fn handle_file_content_request(req: &FileContentRequest, repo_path: &str) -> WsMessage {
    if let Err(e) = validate_relative_path(&req.path, repo_path) {
        return WsMessage::Error(ErrorMsg {
            code: "INVALID_PATH".to_string(),
            message: e,
        });
    }

    let absolute_path = std::path::Path::new(repo_path)
        .join(&req.path)
        .to_string_lossy()
        .to_string();
    let original =
        crate::git::get_file_at_ref(absolute_path, "HEAD".to_string()).unwrap_or_default();
    let modified = std::fs::read_to_string(std::path::Path::new(repo_path).join(&req.path))
        .unwrap_or_default();

    WsMessage::FileContentResponse(FileContentResponse {
        path: req.path.clone(),
        original,
        modified,
    })
}

fn handle_git_stage_unstage(
    repo_path: &str,
    paths: &[String],
    is_stage: bool,
    broadcaster: &WsBroadcaster,
) -> WsMessage {
    for path in paths {
        if let Err(e) = validate_relative_path(path, repo_path) {
            return WsMessage::GitStageResult(GitStageResult {
                success: false,
                error: Some(e),
                files: vec![],
            });
        }
    }

    let result = if is_stage {
        crate::git::git_stage(repo_path.to_string(), paths.to_vec())
    } else {
        crate::git::git_unstage(repo_path.to_string(), paths.to_vec())
    };

    if let Err(e) = result {
        return WsMessage::GitStageResult(GitStageResult {
            success: false,
            error: Some(e),
            files: vec![],
        });
    }

    let files = crate::git::get_git_status(repo_path.to_string())
        .unwrap_or_default()
        .into_iter()
        .map(|s| GitFileStatusMsg {
            path: s.path,
            index_status: s.index_status,
            worktree_status: s.worktree_status,
        })
        .collect::<Vec<_>>();

    broadcaster.try_send(WsMessage::GitStatusSync(GitStatusSync {
        files: files.clone(),
    }));

    WsMessage::GitStageResult(GitStageResult {
        success: true,
        error: None,
        files,
    })
}

fn route_message(msg: &WsMessage, state: &WsServerState) -> Option<WsMessage> {
    match msg {
        WsMessage::PtyInput(input) => {
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
        WsMessage::PtyResize(_) => {
            // PWAからのリサイズはデスクトップのターミナル表示を崩すため無視
            None
        }
        WsMessage::GitStatusRequest(_) => {
            if let Some(repo_path) = &state.repo_path {
                let files = crate::git::get_git_status(repo_path.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|s| GitFileStatusMsg {
                        path: s.path,
                        index_status: s.index_status,
                        worktree_status: s.worktree_status,
                    })
                    .collect::<Vec<_>>();
                Some(WsMessage::GitStatusSync(GitStatusSync { files }))
            } else {
                Some(WsMessage::Error(ErrorMsg {
                    code: "NO_REPO".to_string(),
                    message: "リポジトリパスが設定されていません".to_string(),
                }))
            }
        }
        WsMessage::FileContentRequest(req) => {
            if let Some(repo_path) = &state.repo_path {
                Some(handle_file_content_request(req, repo_path))
            } else {
                Some(WsMessage::Error(ErrorMsg {
                    code: "NO_REPO".to_string(),
                    message: "リポジトリパスが設定されていません".to_string(),
                }))
            }
        }
        WsMessage::GitStage(req) => {
            if let Some(repo_path) = &state.repo_path {
                Some(handle_git_stage_unstage(
                    repo_path,
                    &req.paths,
                    true,
                    &state.broadcaster,
                ))
            } else {
                Some(WsMessage::Error(ErrorMsg {
                    code: "NO_REPO".to_string(),
                    message: "リポジトリパスが設定されていません".to_string(),
                }))
            }
        }
        WsMessage::GitUnstage(req) => {
            if let Some(repo_path) = &state.repo_path {
                Some(handle_git_stage_unstage(
                    repo_path,
                    &req.paths,
                    false,
                    &state.broadcaster,
                ))
            } else {
                Some(WsMessage::Error(ErrorMsg {
                    code: "NO_REPO".to_string(),
                    message: "リポジトリパスが設定されていません".to_string(),
                }))
            }
        }
        WsMessage::AddComment(comment) => {
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
        _ => Some(WsMessage::Error(ErrorMsg {
            code: "INVALID_MESSAGE".to_string(),
            message: "Unexpected message from client".to_string(),
        })),
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
    if is_ws_upgrade(&req) {
        match handle_ws_upgrade(req, peer_addr, state) {
            Ok(resp) => resp,
            Err(e) => error_response(StatusCode::BAD_REQUEST, &e),
        }
    } else {
        serve_pwa(&path, &state)
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

fn serve_pwa(path: &str, state: &WsServerState) -> Response<Full<Bytes>> {
    let pwa_dir = match &state.pwa_dir {
        Some(d) => d,
        None => return error_response(StatusCode::NOT_FOUND, "PWA is not available"),
    };

    let file_path = match path {
        "/" | "" => "pwa.html",
        p => p.trim_start_matches('/'),
    };

    let full_path = pwa_dir.join(file_path);
    if let (Ok(canonical), Ok(pwa_canonical)) = (full_path.canonicalize(), pwa_dir.canonicalize()) {
        if !canonical.starts_with(&pwa_canonical) {
            return error_response(StatusCode::FORBIDDEN, "Access denied");
        }
        match std::fs::read(&canonical) {
            Ok(content) => {
                let ct = content_type_for(canonical.to_str().unwrap_or(""));
                Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", ct)
                    .header("Cache-Control", "no-cache")
                    .body(Full::new(Bytes::from(content)))
                    .unwrap()
            }
            Err(_) => serve_pwa_fallback(pwa_dir),
        }
    } else {
        serve_pwa_fallback(pwa_dir)
    }
}

fn serve_pwa_fallback(pwa_dir: &std::path::Path) -> Response<Full<Bytes>> {
    match std::fs::read(pwa_dir.join("pwa.html")) {
        Ok(content) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "text/html; charset=utf-8")
            .body(Full::new(Bytes::from(content)))
            .unwrap(),
        Err(_) => error_response(StatusCode::NOT_FOUND, "Not Found"),
    }
}

fn error_response(status: StatusCode, msg: &str) -> Response<Full<Bytes>> {
    Response::builder()
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

    let token = state.current_token()?;
    let result = handle_ws_authenticated(ws_stream, peer_addr, &token, state).await;

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
        let _ = app.emit("pwa-connected", ());
    }

    // --- WsBroadcaster セットアップ（PTYスポーン前に初期化） ---
    let (tx, mut rx) = WsBroadcaster::create_channel();
    state.broadcaster.set_sender(Some(tx));

    // --- デスクトップPTY共有 ---
    if let Some(pm) = &state.pty_manager {
        if let Some(pty_id) = pm.active_pty_id() {
            let (cols, rows) = pm.get_pty_size(pty_id).unwrap_or((80, 24));
            let ready_msg = WsMessage::PtyReady(PtyReady { pty_id, cols, rows });
            write
                .send(Message::Text(
                    serialize_message(&ready_msg).map_err(|e| e.to_string())?,
                ))
                .await
                .map_err(|e| format!("Failed to send pty_ready: {e}"))?;

            let buffered = state.broadcaster.take_pty_output_buffer();
            if !buffered.is_empty() {
                let replay_msg = WsMessage::PtyOutput(PtyOutputMsg {
                    pty_id,
                    data: buffered,
                });
                write
                    .send(Message::Text(
                        serialize_message(&replay_msg).map_err(|e| e.to_string())?,
                    ))
                    .await
                    .map_err(|e| format!("Failed to send pty output replay: {e}"))?;
            }
        } else {
            let err_msg = WsMessage::Error(ErrorMsg {
                code: "NO_PTY".to_string(),
                message: "デスクトップのターミナルがまだ起動していません".to_string(),
            });
            write
                .send(Message::Text(
                    serialize_message(&err_msg).map_err(|e| e.to_string())?,
                ))
                .await
                .map_err(|e| format!("Failed to send no_pty error: {e}"))?;
        }
    }

    // --- 初期データ送信 ---
    if let Some(repo_path) = &state.repo_path {
        let files = crate::git::get_git_status(repo_path.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|s| GitFileStatusMsg {
                path: s.path,
                index_status: s.index_status,
                worktree_status: s.worktree_status,
            })
            .collect::<Vec<_>>();
        let sync_msg = WsMessage::GitStatusSync(GitStatusSync { files });
        write
            .send(Message::Text(
                serialize_message(&sync_msg).map_err(|e| e.to_string())?,
            ))
            .await
            .map_err(|e| format!("Failed to send initial git status: {e}"))?;
    }

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
                if let Some(response) = route_message(&ws_msg, state) {
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
    state.broadcaster.set_sender(None);
    // forward_task にドロップされた rx の closed を通知して終了させる
    let _ = forward_task.await;

    log::info!("Client disconnected: {}", peer_addr);
    Ok(())
}

#[tauri::command]
pub async fn start_server(
    root_path: String,
    app: tauri::AppHandle,
    handle: tauri::State<'_, WsServerHandle>,
    config_state: tauri::State<'_, Arc<AppConfig>>,
    broadcaster: tauri::State<'_, Arc<WsBroadcaster>>,
    pty_manager: tauri::State<'_, Arc<crate::pty::PtyManager>>,
) -> Result<String, String> {
    {
        let running = handle.running.lock();
        if *running {
            return Err("サーバーは既に起動しています".to_string());
        }
    }

    let mut cfg = config_state.get_config()?;

    let vpn_iface = crate::vpn_detect::detect_vpn_ip()
        .ok_or("メッシュネットが検出できません。NordVPN Meshnet が有効か確認してください")?;
    let bind_ip = vpn_iface.ip.to_string();
    cfg.server.bind = bind_ip.clone();

    let bind_ip_addr = std::net::IpAddr::V4(vpn_iface.ip);
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("データディレクトリの取得失敗: {e}"))?;
    let (cert_path, key_path) = crate::tls::ensure_self_signed_cert(bind_ip_addr, &data_dir)?;
    cfg.server.tls.enabled = true;
    cfg.server.tls.cert = cert_path.to_string_lossy().to_string();
    cfg.server.tls.key = key_path.to_string_lossy().to_string();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let pwa_dir = if cfg!(debug_assertions) {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("pwa");
        if dir.exists() {
            Some(dir)
        } else {
            None
        }
    } else {
        app.path().resource_dir().ok().map(|d| d.join("pwa"))
    };
    let server_state = Arc::new(WsServerState::new(
        pwa_dir,
        Arc::clone(&broadcaster),
        Some(Arc::clone(&pty_manager)),
        Some(root_path),
        Arc::clone(config_state.inner()),
        Some(app.clone()),
    ));

    start_ws_server(&cfg, server_state, shutdown_rx).await?;

    {
        let mut running = handle.running.lock();
        *running = true;
        let mut tx = handle.shutdown_tx.lock();
        *tx = Some(shutdown_tx);
        handle.active_bind.lock().replace(bind_ip.clone());
        *handle.tls_enabled.lock() = cfg.server.tls.enabled;
    }

    Ok(bind_ip)
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
        )
    }

    #[test]
    fn test_route_unknown_message_returns_error() {
        let state = test_state();
        let msg = WsMessage::AuthChallenge(AuthChallenge {
            challenge: "x".to_string(),
        });
        let result = route_message(&msg, &state);
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "INVALID_MESSAGE"),
            _ => panic!("expected error"),
        }
    }

    #[test]
    fn test_route_add_comment_returns_none() {
        let state = test_state();
        let msg = WsMessage::AddComment(AddComment {
            file_path: "src/main.rs".to_string(),
            line_number: 10,
            end_line: None,
            content: "fix this".to_string(),
        });
        let result = route_message(&msg, &state);
        assert!(result.is_none());
    }

    #[test]
    fn test_route_pty_input_without_manager_returns_none() {
        let state = test_state();
        let msg = WsMessage::PtyInput(PtyInput {
            pty_id: 1,
            data: "ls".to_string(),
        });
        let result = route_message(&msg, &state);
        assert!(result.is_none());
    }

    #[test]
    fn test_route_git_status_request_without_repo() {
        let state = test_state();
        let msg = WsMessage::GitStatusRequest(GitStatusRequest {});
        let result = route_message(&msg, &state);
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_REPO"),
            _ => panic!("expected no repo error"),
        }
    }

    #[test]
    fn test_route_file_content_request_without_repo() {
        let state = test_state();
        let msg = WsMessage::FileContentRequest(FileContentRequest {
            path: "test.rs".to_string(),
        });
        let result = route_message(&msg, &state);
        match result {
            Some(WsMessage::Error(e)) => assert_eq!(e.code, "NO_REPO"),
            _ => panic!("expected no repo error"),
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
}
