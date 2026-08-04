use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use super::*;
use crate::domain::terminal_surface::entities::TerminalSurface;
use crate::domain::terminal_surface::gateway::TerminalSurfaceRepository;
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
use crate::usecase::terminal_surface::application::TerminalSurfaceApplication;

struct BackendOwnedSurface;

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
        }
    }
}

fn workspace_owner() -> TerminalSurfaceOwner {
    TerminalSurfaceOwner::workspace(WorkspaceIdentity::new("/repo"))
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

    fn select_gc_targets(&self, _worktree_path: &str, _keep_session_keys: &[String]) -> Vec<u64> {
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

    fn write(
        &self,
        _session_key: &str,
        _data: &str,
    ) -> Result<(), crate::domain::terminal_surface::gateway::TerminalSurfaceGatewayError> {
        Ok(())
    }

    fn resize(
        &self,
        _session_key: &str,
        _rows: u16,
        _cols: u16,
    ) -> Result<(), crate::domain::terminal_surface::gateway::TerminalSurfaceGatewayError> {
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

#[tokio::test]
async fn test_ターミナル画面参照_ウェブソケットとtauriが同じバックエンド状態を返す() {
    let application = Arc::new(TerminalSurfaceApplication::new(
        Arc::new(BackendOwnedSurface),
        Arc::new(
            crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub::new(),
        ),
    ));
    let owner = workspace_owner();
    let tauri_query = TerminalSurfaceV1::from(
        application
            .get(&owner)
            .expect("Tauri query path reads the Terminal Surface"),
    );
    let response = dispatch(
        &TerminalApiDeps::new(application),
        TerminalWsRequestV1::GetSurface {
            id: "request-1".to_string(),
            owner: TerminalSurfaceOwnerV1::Workspace {
                workspace_path: "/repo".to_string(),
            },
        },
    );

    let TerminalWsResponseV1::Ok { id, surface } = response else {
        panic!("expected successful WebSocket Terminal Surface response");
    };
    assert_eq!(id, "request-1");
    assert_eq!(
        serde_json::to_value(surface).unwrap(),
        serde_json::to_value(tauri_query).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ターミナル画面参照_認証済みウェブソケットが本番経路を使う() {
    let application = Arc::new(TerminalSurfaceApplication::new(
        Arc::new(BackendOwnedSurface),
        Arc::new(
            crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub::new(),
        ),
    ));
    let data_dir = tempfile::TempDir::new().unwrap();
    let router = crate::adaptor::controller::api::test_support::test_router_with_terminal(
        data_dir.path(),
        "terminal-token",
        TerminalApiDeps::new(application),
    );
    let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.kind() == std::io::ErrorKind::AddrNotAvailable =>
        {
            eprintln!("skipping WebSocket product-route test: {error}");
            return;
        }
        Err(error) => panic!("bind WebSocket product-route test: {error}"),
    };
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let mut request = format!("ws://{address}/v1/terminal")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer terminal-token".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("connect authenticated production Terminal WebSocket");
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "get_surface",
                "id": "wire-1",
                "owner": {
                    "kind": "workspace",
                    "workspacePath": "/repo"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let response = socket
        .next()
        .await
        .expect("Terminal WebSocket response")
        .expect("valid Terminal WebSocket frame")
        .into_text()
        .expect("Terminal WebSocket text response");
    let response: serde_json::Value = serde_json::from_str(&response).unwrap();

    assert_eq!(response["status"], "ok");
    assert_eq!(response["id"], "wire-1");
    assert_eq!(
        response["surface"]["session_key"],
        workspace_owner().stable_key()
    );
    assert_eq!(response["surface"]["terminal_surface"]["sequence"], 41);
    assert_eq!(response["surface"]["terminal_surface"]["cols"], 111);
    assert_eq!(response["surface"]["terminal_surface"]["rows"], 37);
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ターミナル画面接続_認証済みウェブソケットが画面写像後にバックエンド出力を送る() {
    use crate::domain::terminal_surface::gateway::{
        TerminalSurfaceEvent, TerminalSurfaceEventSink,
    };

    let event_hub = Arc::new(
        crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub::new(),
    );
    let application = Arc::new(TerminalSurfaceApplication::new(
        Arc::new(BackendOwnedSurface),
        event_hub.clone(),
    ));
    let data_dir = tempfile::TempDir::new().unwrap();
    let router = crate::adaptor::controller::api::test_support::test_router_with_terminal(
        data_dir.path(),
        "terminal-token",
        TerminalApiDeps::new(application),
    );
    let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
        Ok(listener) => listener,
        Err(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied
                || error.kind() == std::io::ErrorKind::AddrNotAvailable =>
        {
            eprintln!("skipping WebSocket product-route test: {error}");
            return;
        }
        Err(error) => panic!("bind WebSocket product-route test: {error}"),
    };
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let mut request = format!("ws://{address}/v1/terminal")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer terminal-token".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("connect authenticated Terminal WebSocket attachment");
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "attach_surface",
                "id": "wire-attach",
                "owner": {
                    "kind": "workspace",
                    "workspacePath": "/repo"
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();

    let snapshot = socket
        .next()
        .await
        .expect("attachment snapshot")
        .expect("valid attachment snapshot")
        .into_text()
        .unwrap();
    let snapshot: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
    assert_eq!(snapshot["status"], "event");
    assert_eq!(snapshot["id"], "wire-attach");
    assert_eq!(snapshot["item"]["type"], "snapshot");
    assert_eq!(
        snapshot["item"]["surface"]["terminal_surface"]["sequence"],
        41
    );

    event_hub.publish(TerminalSurfaceEvent::Output {
        session_key: workspace_owner().stable_key(),
        data: "live-output".to_string(),
        sequence: 42,
    });
    let output = socket
        .next()
        .await
        .expect("attachment output")
        .expect("valid attachment output")
        .into_text()
        .unwrap();
    let output: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(output["status"], "event");
    assert_eq!(output["id"], "wire-attach");
    assert_eq!(output["item"]["type"], "output");
    assert_eq!(output["item"]["sequence"], 42);
    assert_eq!(output["item"]["data"], "live-output");
    server.abort();
}
