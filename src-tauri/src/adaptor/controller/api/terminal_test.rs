use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use super::*;
use crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub;
use crate::adaptor::gateway::terminal_surface::output_flow_control::TERMINAL_OUTPUT_CREDIT_CODE_UNITS;
use crate::domain::terminal_surface::entities::TerminalSurface;
use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventSink, TerminalSurfaceRepository,
};
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AttachedWrite {
    session_key: String,
    attachment_id: String,
    sequence: u64,
    data: String,
}

#[derive(Default)]
struct BackendOwnedSurface {
    attached_writes: std::sync::Mutex<Vec<AttachedWrite>>,
    resizes: std::sync::Mutex<Vec<(String, u16, u16)>>,
}

impl BackendOwnedSurface {
    fn surface() -> TerminalSurface {
        let owner = workspace_owner();
        TerminalSurface {
            session_key: owner.stable_key(),
            owner,
            worktree_path: Some("/repo".to_string()),
            label: Some("Agent TUI".to_string()),
            runtime_generation: 7.into(),
            process_state: crate::domain::terminal_surface::TerminalProcessState::Running,
            checkpoint: crate::domain::terminal_surface::TerminalSurfaceCheckpoint {
                replay: "\u{1b}[2Jshared backend screen".to_string(),
                sequence: 41,
                cols: 111,
                rows: 37,
            },
            latest_sequence: 41,
            last_output_at: None,
        }
    }
}

fn workspace_owner() -> TerminalSurfaceOwner {
    TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo")).unwrap()
}

impl TerminalSurfaceRepository for BackendOwnedSurface {
    fn find_summary_by_session_key(
        &self,
        session_key: &str,
    ) -> Option<crate::domain::terminal_surface::entities::TerminalSurfaceSummary> {
        let surface = Self::surface();
        (session_key == surface.session_key).then(|| surface.summary())
    }

    fn list_summaries(
        &self,
    ) -> Vec<crate::domain::terminal_surface::entities::TerminalSurfaceSummary> {
        Vec::new()
    }
}

impl crate::domain::terminal_surface::gateway::TerminalSurfaceGateway for BackendOwnedSurface {
    fn next_runtime_generation(&self) -> u64 {
        8
    }

    fn spawn_runtime(
        &self,
        _request: crate::domain::terminal_surface::gateway::TerminalRuntimeSpawnRequest,
    ) -> Result<(), crate::domain::terminal_surface::gateway::TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn insert_surface(&self, _surface: TerminalSurface) {}

    fn start_output_reader(
        &self,
        _runtime_generation: u64,
    ) -> Result<(), crate::domain::terminal_surface::gateway::TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn snapshot(&self, runtime_generation: u64) -> Option<TerminalSurface> {
        (runtime_generation == 7).then(Self::surface)
    }

    fn select_kill_targets_by_worktree(&self, _worktree_path: &str) -> Vec<u64> {
        Vec::new()
    }

    fn remove_surface(&self, _runtime_generation: u64) -> Option<TerminalSurface> {
        None
    }

    fn reserve_spawn_slot(
        &self,
        session_key: &str,
        worktree_path: Option<&str>,
    ) -> Result<
        crate::domain::terminal_surface::entities::TerminalSurfaceSpawnReservation,
        crate::domain::terminal_surface::entities::TerminalSurfaceSpawnReservationError,
    > {
        Ok(
            crate::domain::terminal_surface::entities::TerminalSurfaceSpawnReservation {
                session_key: session_key.to_string(),
                worktree_path: worktree_path.map(str::to_string),
            },
        )
    }

    fn complete_spawn_slot(
        &self,
        _reservation: &crate::domain::terminal_surface::entities::TerminalSurfaceSpawnReservation,
    ) {
    }

    fn rollback_spawn_slot(
        &self,
        _reservation: &crate::domain::terminal_surface::entities::TerminalSurfaceSpawnReservation,
    ) {
    }

    fn activate_input_attachment(&self, _session_key: &str, _attachment_id: &str) {}

    fn deactivate_input_attachment(&self, _session_key: &str, _attachment_id: &str) {}

    fn write_attached(
        &self,
        session_key: &str,
        attachment_id: &str,
        sequence: u64,
        data: &str,
    ) -> Result<(), crate::domain::terminal_surface::gateway::TerminalSurfaceGatewayError> {
        self.attached_writes.lock().unwrap().push(AttachedWrite {
            session_key: session_key.to_string(),
            attachment_id: attachment_id.to_string(),
            sequence,
            data: data.to_string(),
        });
        Ok(())
    }

    fn write(
        &self,
        _session_key: &str,
        _data: &str,
    ) -> Result<(), crate::domain::terminal_surface::gateway::TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn resize(
        &self,
        session_key: &str,
        rows: u16,
        cols: u16,
    ) -> Result<(), crate::domain::terminal_surface::gateway::TerminalSurfaceGatewayError> {
        self.resizes
            .lock()
            .unwrap()
            .push((session_key.to_string(), rows, cols));
        Ok(())
    }

    fn request_runtime_stop(
        &self,
        _runtime_generation: u64,
    ) -> Result<(), crate::domain::terminal_surface::gateway::TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn remove_runtime(&self, _runtime_generation: u64) {}
}

type ClientWs =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct TerminalWsFixture {
    gateway: Arc<BackendOwnedSurface>,
    event_hub: Arc<TerminalSurfaceEventHub>,
    _data_dir: tempfile::TempDir,
    address: std::net::SocketAddr,
    server: tokio::task::JoinHandle<()>,
}

async fn serve_router(
    router: axum::Router,
) -> Option<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.kind() == std::io::ErrorKind::AddrNotAvailable =>
        {
            eprintln!("skipping WebSocket product-route test: {error}");
            return None;
        }
        Err(error) => panic!("bind WebSocket product-route test: {error}"),
    };
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Some((address, server))
}

async fn terminal_ws_fixture(event_hub: Arc<TerminalSurfaceEventHub>) -> Option<TerminalWsFixture> {
    let gateway = Arc::new(BackendOwnedSurface::default());
    let application = Arc::new(TerminalSurfaceApplication::new(
        gateway.clone(),
        event_hub.clone(),
    ));
    let data_dir = tempfile::TempDir::new().unwrap();
    let router = crate::adaptor::controller::api::test_support::test_router_with_terminal(
        data_dir.path(),
        "terminal-token",
        TerminalApiDeps::new(application),
    );
    let (address, server) = serve_router(router).await?;
    Some(TerminalWsFixture {
        gateway,
        event_hub,
        _data_dir: data_dir,
        address,
        server,
    })
}

async fn connect_terminal_ws(address: std::net::SocketAddr, token: &str) -> ClientWs {
    let mut request = format!("ws://{address}{TERMINAL_WS_PATH}")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let (socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("connect authenticated Terminal WebSocket");
    socket
}

async fn next_json(socket: &mut ClientWs) -> serde_json::Value {
    let text = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
        .await
        .expect("Terminal WebSocket frame within timeout")
        .expect("Terminal WebSocket frame")
        .expect("valid Terminal WebSocket frame")
        .into_text()
        .expect("Terminal WebSocket text frame");
    serde_json::from_str(&text).unwrap()
}

async fn attach_workspace(socket: &mut ClientWs, attachment_id: &str) {
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "attach_surface",
                "id": "wire-attach",
                "owner": workspace_owner_json(),
                "attachment_id": attachment_id
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let attached = next_json(socket).await;
    assert_eq!(attached["status"], "attached");
    assert_eq!(attached["id"], "wire-attach");
    let snapshot = next_json(socket).await;
    assert_eq!(snapshot["status"], "event");
    assert_eq!(snapshot["item"]["type"], "snapshot");
}

fn workspace_owner_json() -> serde_json::Value {
    serde_json::json!({
        "kind": "workspace",
        "workspacePath": "/repo"
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ターミナル画面接続_認証済みウェブソケットが画面写像後にバックエンド出力を送る() {
    let Some(fixture) = terminal_ws_fixture(Arc::new(TerminalSurfaceEventHub::new())).await else {
        return;
    };
    let mut socket = connect_terminal_ws(fixture.address, "terminal-token").await;
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "attach_surface",
                "id": "wire-attach",
                "owner": workspace_owner_json()
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let attached = next_json(&mut socket).await;
    assert_eq!(attached["status"], "attached");
    assert_eq!(attached["id"], "wire-attach");
    let snapshot = next_json(&mut socket).await;
    assert_eq!(snapshot["status"], "event");
    assert_eq!(snapshot["id"], "wire-attach");
    assert_eq!(snapshot["item"]["type"], "snapshot");
    assert_eq!(
        snapshot["item"]["surface"]["terminal_surface"]["sequence"],
        41
    );

    fixture.event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: workspace_owner().stable_key(),
        data: "live-output".into(),
        sequence: 42,
    });
    let output = next_json(&mut socket).await;
    assert_eq!(output["status"], "event");
    assert_eq!(output["id"], "wire-attach");
    assert_eq!(output["item"]["type"], "output");
    assert_eq!(output["item"]["sequence"], 42);
    assert_eq!(output["item"]["data"], "live-output");
    fixture.server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ターミナル画面接続_write_frameがwrite_attachedへ透過し成功応答を返さない() {
    let Some(fixture) = terminal_ws_fixture(Arc::new(TerminalSurfaceEventHub::new())).await else {
        return;
    };
    let mut socket = connect_terminal_ws(fixture.address, "terminal-token").await;
    attach_workspace(&mut socket, "ws-write-attachment").await;

    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "write",
                "owner": workspace_owner_json(),
                "attachment_id": "ws-write-attachment",
                "sequence": 7,
                "data": "echo hi\n"
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let mut recorded = None;
    for _ in 0..500 {
        if let Some(write) = fixture.gateway.attached_writes.lock().unwrap().first() {
            recorded = Some(write.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        recorded,
        Some(AttachedWrite {
            session_key: workspace_owner().stable_key(),
            attachment_id: "ws-write-attachment".to_string(),
            sequence: 7,
            data: "echo hi\n".to_string(),
        })
    );

    fixture.event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: workspace_owner().stable_key(),
        data: "after-write".into(),
        sequence: 42,
    });
    let next_frame = next_json(&mut socket).await;
    assert_eq!(next_frame["status"], "event");
    assert_eq!(next_frame["item"]["type"], "output");
    assert_eq!(next_frame["item"]["sequence"], 42);
    fixture.server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ターミナル画面接続_resize_frameがバックエンドへ適用される() {
    let Some(fixture) = terminal_ws_fixture(Arc::new(TerminalSurfaceEventHub::new())).await else {
        return;
    };
    let mut socket = connect_terminal_ws(fixture.address, "terminal-token").await;
    attach_workspace(&mut socket, "ws-resize-attachment").await;

    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "resize",
                "owner": workspace_owner_json(),
                "rows": 40,
                "cols": 120
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let mut recorded = None;
    for _ in 0..500 {
        if let Some(resize) = fixture.gateway.resizes.lock().unwrap().first() {
            recorded = Some(resize.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        recorded,
        Some((workspace_owner().stable_key(), 40u16, 120u16))
    );
    fixture.server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ターミナル画面接続_不正json_frameにinvalid_requestエラーを返す() {
    let Some(fixture) = terminal_ws_fixture(Arc::new(TerminalSurfaceEventHub::new())).await else {
        return;
    };
    let mut socket = connect_terminal_ws(fixture.address, "terminal-token").await;
    attach_workspace(&mut socket, "ws-invalid-attachment").await;

    socket
        .send(Message::Text("not-json".to_string().into()))
        .await
        .unwrap();
    let response = next_json(&mut socket).await;
    assert_eq!(response["status"], "error");
    assert_eq!(response["error"]["code"], "INVALID_REQUEST");
    assert_eq!(
        response["error"]["message"],
        "Terminal request failed because the request is invalid."
    );
    fixture.server.abort();
}

#[test]
fn test_ターミナル画面接続_全エラー操作が固定した利用者向け文言を返す() {
    // Given
    let cases = [
        (
            TerminalWsError::Attach(TerminalWsErrorCode::PtyError),
            "PTY_ERROR",
            "Terminal attachment failed. Try again.",
        ),
        (
            TerminalWsError::Attach(TerminalWsErrorCode::InvalidRequest),
            "INVALID_REQUEST",
            "Terminal attachment failed because the request is invalid.",
        ),
        (
            TerminalWsError::Write(TerminalWsErrorCode::PtyError),
            "PTY_ERROR",
            "Terminal input could not be sent. Try again.",
        ),
        (
            TerminalWsError::Write(TerminalWsErrorCode::InvalidRequest),
            "INVALID_REQUEST",
            "Terminal input could not be sent because the request is invalid.",
        ),
        (
            TerminalWsError::Resize(TerminalWsErrorCode::PtyError),
            "PTY_ERROR",
            "Terminal resize failed. Try again.",
        ),
        (
            TerminalWsError::Resize(TerminalWsErrorCode::InvalidRequest),
            "INVALID_REQUEST",
            "Terminal resize failed because the request is invalid.",
        ),
        (
            TerminalWsError::InvalidRequest,
            "INVALID_REQUEST",
            "Terminal request failed because the request is invalid.",
        ),
    ];

    // When / Then
    for (error, expected_code, expected_message) in cases {
        let wire = serde_json::to_value(terminal_ws_error(
            "test".to_string(),
            error,
            "internal cause that must not be displayed".to_string(),
        ))
        .unwrap();
        assert_eq!(wire["error"]["code"], expected_code);
        assert_eq!(wire["error"]["message"], expected_message);
        assert!(!wire["error"]["message"]
            .as_str()
            .unwrap()
            .contains("internal cause"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ターミナル画面接続_ack_frameがflow_controlのcreditを解放する() {
    let Some(fixture) =
        terminal_ws_fixture(Arc::new(TerminalSurfaceEventHub::with_flags(256, true))).await
    else {
        return;
    };
    let mut socket = connect_terminal_ws(fixture.address, "terminal-token").await;
    attach_workspace(&mut socket, "ws-ack-attachment").await;

    fixture.event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: workspace_owner().stable_key(),
        data: "a".repeat(TERMINAL_OUTPUT_CREDIT_CODE_UNITS).into(),
        sequence: 42,
    });
    let first = next_json(&mut socket).await;
    assert_eq!(first["item"]["type"], "output");
    assert_eq!(first["item"]["sequence"], 42);

    let blocked_hub = fixture.event_hub.clone();
    let blocked = tokio::task::spawn_blocking(move || {
        blocked_hub.publish(TerminalSurfaceEvent::Output {
            session_key: workspace_owner().stable_key(),
            data: "tail".into(),
            sequence: 43,
        });
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        !blocked.is_finished(),
        "publish must wait for output credit until the client acknowledges"
    );

    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "ack",
                "attachment_id": "ws-ack-attachment",
                "sequence": 42
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), blocked)
        .await
        .expect("ack frame must release the blocked output credit")
        .unwrap();
    let released = next_json(&mut socket).await;
    assert_eq!(released["item"]["type"], "output");
    assert_eq!(released["item"]["sequence"], 43);
    fixture.server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ターミナル接続_handshake応答がbearer_subprotocolをechoする() {
    let Some(fixture) = terminal_ws_fixture(Arc::new(TerminalSurfaceEventHub::new())).await else {
        return;
    };
    let subprotocol = format!("{TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX}terminal-token");
    let mut request = format!("ws://{}{TERMINAL_WS_PATH}", fixture.address)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-protocol", subprotocol.parse().unwrap());
    let (_socket, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("connect Terminal WebSocket via bearer subprotocol");

    assert_eq!(
        response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|value| value.to_str().ok()),
        Some(subprotocol.as_str())
    );
    fixture.server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_terminal専用tokenはterminal_routeのみ認証され他routeでは拒否される() {
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;

    let application = Arc::new(TerminalSurfaceApplication::new(
        Arc::new(BackendOwnedSurface::default()),
        Arc::new(TerminalSurfaceEventHub::new()),
    ));
    let data_dir = tempfile::TempDir::new().unwrap();
    let router = crate::adaptor::controller::api::test_support::test_router_with_terminal_tokens(
        data_dir.path(),
        "master-token",
        "terminal-scoped-token",
        TerminalApiDeps::new(application),
    );

    for (token, rejected) in [("terminal-scoped-token", true), ("master-token", false)] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1/workflows")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status() == StatusCode::UNAUTHORIZED,
            rejected,
            "workflow route auth mismatch for token: {token}"
        );
    }

    let Some((address, server)) = serve_router(router).await else {
        return;
    };
    for token in ["terminal-scoped-token", "master-token"] {
        let mut socket = connect_terminal_ws(address, token).await;
        attach_workspace(&mut socket, &format!("ws-scope-{token}")).await;
    }
    server.abort();
}
