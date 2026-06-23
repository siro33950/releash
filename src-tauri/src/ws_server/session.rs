use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::tungstenite::Message;

use crate::adaptor::protocol::pty::PtyOutputMsg;
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
        .send(Message::text(
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
                .send(Message::text(
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
                .send(Message::text(
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
            .send(Message::text(
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
        .send(Message::text(
            serialize_message(&success_msg).map_err(|e| e.to_string())?,
        ))
        .await
        .map_err(|e| format!("Failed to send auth result: {e}"))?;

    log::info!("Client authenticated: {}", peer_addr);

    // --- WsBroadcaster セットアップ（PTYスポーン前に初期化） ---
    let (tx, mut rx) = WsBroadcaster::create_channel();
    state.broadcaster.set_sender(Some(tx));
    let _sender_guard = BroadcasterGuard(state.broadcaster.clone());
    let stream_sync_notify = state.broadcaster.stream_sync_notify();
    let broadcaster_for_forward = state.broadcaster.clone();

    // PTY出力 + AgentStreamSync を WebSocket にフォワードするタスク。
    // 通常の `WsMessage` は `rx` から受け取って即送信する。一方で
    // `AgentStreamSync` は `WsBroadcaster::latest_stream_sync` slot に
    // 最新累積のみを保持し、`stream_sync_notify` で起床して drain する。
    // これにより遅い WS receiver に対しても累積 snapshot がキューに積み上がらず、
    // メモリ消費は (live message × 1 snapshot) で頭打ちになる。
    let forward_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = stream_sync_notify.notified() => {
                    let drained = broadcaster_for_forward.drain_stream_sync();
                    let mut send_failed = false;
                    for sync in drained {
                        let msg = WsMessage::AgentStreamSync(sync);
                        if let Ok(json) = serialize_message(&msg) {
                            crate::other::telemetry::record_payload_size(
                                crate::other::telemetry::Payload::WebSocket,
                                || json.len(),
                            );
                            if write.send(Message::text(json)).await.is_err() {
                                send_failed = true;
                                break;
                            }
                        }
                    }
                    if send_failed { break; }
                }
                maybe = rx.recv() => {
                    match maybe {
                        Some(msg) => {
                            if let Ok(json) = serialize_message(&msg) {
                                crate::other::telemetry::record_payload_size(
                                    crate::other::telemetry::Payload::WebSocket,
                                    || json.len(),
                                );
                                if write.send(Message::text(json)).await.is_err() {
                                    break;
                                }
                            }
                        }
                        None => break,
                    }
                }
            }
        }
        write
    });

    replay_pty_buffers(state);

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
                            code: "INVALID_MESSAGE".to_string(),
                            message: "Unexpected message from client".to_string(),
                        });
                        state.broadcaster.try_send(err);
                        continue;
                    }
                };
                if let Some(response) = route_message(&ws_msg).await {
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

fn replay_pty_buffers(state: &WsServerState) {
    for output in state.pty_replay_reader.replay_outputs() {
        let sent = state
            .broadcaster
            .send_without_buffer(WsMessage::PtyOutput(PtyOutputMsg {
                pty_id: output.pty_id,
                data: output.data,
                sequence: output.sequence,
            }));
        if !sent {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures_util::{SinkExt, StreamExt};
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    use tokio_tungstenite::tungstenite::protocol::Role;

    use crate::domain::app_config::repository::ConfigUpdate;
    use crate::domain::app_config::value_objects::{
        AgentShortcutConfig, AppConfigDocument, AppSettings, ServerConfig, TelemetryConfig,
        TlsConfig, WorkflowConfig,
    };
    use crate::domain::app_config::{AppConfigError, ConfigRepository};
    use crate::domain::notification::{DesktopNotifyMode, NotifyConfig};
    use crate::protocol::{deserialize_message, serialize_message, AuthResponse, WsMessage};
    use crate::usecase::pty_session::dto::PtyReplayOutput;
    use crate::usecase::pty_session::query_service::PtySessionReplayReader;

    use super::*;

    type HmacSha256 = Hmac<Sha256>;

    struct MockReplayReader;

    impl PtySessionReplayReader for MockReplayReader {
        fn replay_outputs(&self) -> Vec<PtyReplayOutput> {
            vec![PtyReplayOutput {
                pty_id: 7,
                data: "buffered output".to_string(),
                sequence: 42,
            }]
        }
    }

    struct MockConfigRepository;

    impl ConfigRepository for MockConfigRepository {
        fn load(&self) -> Result<AppConfigDocument, AppConfigError> {
            Ok(AppConfigDocument {
                server: ServerConfig {
                    bind: "127.0.0.1".to_string(),
                    port: 0,
                    hook_port: 0,
                    token: "token".to_string(),
                    tls: TlsConfig {
                        enabled: false,
                        cert: String::new(),
                        key: String::new(),
                    },
                    notify: NotifyConfig {
                        webhook_url: String::new(),
                        on_running: false,
                        on_done: false,
                        on_error: false,
                        on_waiting: false,
                        desktop_mode: DesktopNotifyMode::Always,
                        inactive_timeout_minutes: 0,
                    },
                },
                telemetry: TelemetryConfig {
                    crash_reporting: false,
                    performance_telemetry: false,
                },
                app: AppSettings {
                    close_to_tray: false,
                    auto_launch: false,
                    start_minimized: false,
                    last_root_path: String::new(),
                    last_repo_paths: Vec::new(),
                    external_editor: String::new(),
                    agent_shortcuts: AgentShortcutConfig::default(),
                },
                workflow: WorkflowConfig {
                    approval_auto_approve: false,
                },
            })
        }

        fn save(&self, _config: AppConfigDocument) -> Result<(), AppConfigError> {
            Ok(())
        }

        fn update(&self, _f: ConfigUpdate) -> Result<(), AppConfigError> {
            Ok(())
        }
    }

    fn hmac_response(challenge: &str, token: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(token.as_bytes()).unwrap();
        mac.update(challenge.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn replay_pty_buffers_sends_buffered_output_to_subscriber() {
        let broadcaster = Arc::new(WsBroadcaster::default());
        let (tx, mut rx) = WsBroadcaster::create_channel();
        broadcaster.set_sender(Some(tx));
        let state = WsServerState::new(
            broadcaster,
            Arc::new(MockReplayReader),
            Arc::new(MockConfigRepository),
            false,
        );

        replay_pty_buffers(&state);

        match rx.try_recv().unwrap() {
            WsMessage::PtyOutput(output) => {
                assert_eq!(output.pty_id, 7);
                assert_eq!(output.data, "buffered output");
                assert_eq!(output.sequence, 42);
            }
            other => panic!("unexpected replay message: {other:?}"),
        }
    }

    #[tokio::test]
    async fn authenticated_connection_replays_pty_buffers_to_websocket_client() {
        let state = Arc::new(WsServerState::new(
            Arc::new(WsBroadcaster::default()),
            Arc::new(MockReplayReader),
            Arc::new(MockConfigRepository),
            false,
        ));
        let (client_io, server_io) = tokio::io::duplex(4096);
        let server_ws =
            tokio_tungstenite::WebSocketStream::from_raw_socket(server_io, Role::Server, None)
                .await;
        let mut client_ws =
            tokio_tungstenite::WebSocketStream::from_raw_socket(client_io, Role::Client, None)
                .await;
        let peer_addr = "127.0.0.1:48121".parse().unwrap();
        let server_state = Arc::clone(&state);
        let server_task = tokio::spawn(async move {
            handle_ws_authenticated(server_ws, peer_addr, "token", server_state.as_ref()).await
        });

        let challenge_text = match client_ws.next().await.unwrap().unwrap() {
            Message::Text(text) => text,
            other => panic!("unexpected challenge frame: {other:?}"),
        };
        let challenge = match deserialize_message(&challenge_text).unwrap() {
            WsMessage::AuthChallenge(challenge) => challenge.challenge,
            other => panic!("unexpected challenge message: {other:?}"),
        };
        let auth_response = WsMessage::AuthResponse(AuthResponse {
            hmac: hmac_response(&challenge, "token"),
        });
        client_ws
            .send(Message::text(serialize_message(&auth_response).unwrap()))
            .await
            .unwrap();

        let auth_result_text = match client_ws.next().await.unwrap().unwrap() {
            Message::Text(text) => text,
            other => panic!("unexpected auth result frame: {other:?}"),
        };
        match deserialize_message(&auth_result_text).unwrap() {
            WsMessage::AuthResult(result) => assert!(result.success),
            other => panic!("unexpected auth result message: {other:?}"),
        }

        let replay_text = tokio::time::timeout(Duration::from_secs(1), async {
            match client_ws.next().await.unwrap().unwrap() {
                Message::Text(text) => text,
                other => panic!("unexpected replay frame: {other:?}"),
            }
        })
        .await
        .expect("authenticated connection should receive PTY replay output");

        match deserialize_message(&replay_text).unwrap() {
            WsMessage::PtyOutput(output) => {
                assert_eq!(output.pty_id, 7);
                assert_eq!(output.data, "buffered output");
                assert_eq!(output.sequence, 42);
            }
            other => panic!("unexpected replay message: {other:?}"),
        }

        client_ws.send(Message::Close(None)).await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(1), server_task)
            .await
            .expect("server task should stop after client close")
            .unwrap();
        assert!(result.is_ok());
    }
}
