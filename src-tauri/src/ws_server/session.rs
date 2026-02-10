use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;
use tauri::Emitter;
use tokio_tungstenite::tungstenite::Message;

use crate::protocol::*;
use crate::ws_bridge::WsBroadcaster;

use super::auth::{generate_challenge, verify_hmac};
use super::rate_limit::{clear_auth_failures, is_ip_blocked, record_auth_failure};
use super::routing::route_message;
use super::WsServerState;

const AUTH_TIMEOUT_SECS: u64 = 5;

struct BroadcasterGuard(Arc<WsBroadcaster>);

impl Drop for BroadcasterGuard {
    fn drop(&mut self) {
        self.0.set_sender(None);
    }
}

pub(super) async fn handle_ws_session<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
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
            Message::Ping(_) => {}
            _ => {}
        }
    }

    // --- クリーンアップ ---
    drop(_sender_guard);
    let _ = forward_task.await;

    log::info!("Client disconnected: {}", peer_addr);
    Ok(())
}
