use super::*;
use crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub;
use crate::adaptor::gateway::terminal_surface::output_flow_control::TERMINAL_OUTPUT_CREDIT_CODE_UNITS;
use crate::domain::terminal_surface::entities::TerminalSurface;
use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventSink, TerminalSurfaceRepository,
};
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AttachedWrite {
    session_key: String,
    attachment_id: String,
    sequence: u64,
    data: String,
}

#[derive(Default)]
struct BackendOwnedSurface {
    replay: Option<String>,
    missing_snapshot: std::sync::atomic::AtomicBool,
    attached_writes: std::sync::Mutex<Vec<AttachedWrite>>,
    resizes: std::sync::Mutex<Vec<(String, u16, u16)>>,
    deactivated: std::sync::Mutex<Vec<String>>,
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
        (runtime_generation == 7
            && !self
                .missing_snapshot
                .load(std::sync::atomic::Ordering::SeqCst))
        .then(|| {
            let mut surface = Self::surface();
            if let Some(replay) = &self.replay {
                surface.checkpoint.replay = replay.clone();
            }
            surface
        })
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
    ) -> Result<
        crate::domain::terminal_surface::entities::TerminalSurfaceSpawnReservation,
        crate::domain::terminal_surface::entities::TerminalSurfaceSpawnReservationError,
    > {
        Ok(
            crate::domain::terminal_surface::entities::TerminalSurfaceSpawnReservation {
                session_key: session_key.to_string(),
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

    fn deactivate_input_attachment(&self, _session_key: &str, attachment_id: &str) {
        self.deactivated
            .lock()
            .unwrap()
            .push(attachment_id.to_string());
    }

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
        if rows == 40 {
            std::thread::sleep(Duration::from_millis(30));
        }
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

fn fixture(
    replay: Option<String>,
    flow_control: bool,
) -> (TerminalConnection, Arc<TerminalSurfaceEventHub>) {
    let gateway = Arc::new(BackendOwnedSurface {
        replay,
        ..Default::default()
    });
    let hub = Arc::new(TerminalSurfaceEventHub::with_flags(256, flow_control));
    let application = Arc::new(TerminalSurfaceApplication::new(gateway, hub.clone()));
    (
        TerminalConnection::new(Some(TerminalApiDeps::new(application))),
        hub,
    )
}

fn attach_args(id: &str) -> wire::AttachTerminalSurfaceRequest {
    wire::AttachTerminalSurfaceRequest {
        attachment_id: Some(id.into()),
        owner: Some(wire::TerminalSurfaceOwnerV1 {
            variant: Some(wire::terminal_surface_owner_v1::Variant::Workspace(
                wire::TerminalSurfaceOwnerV1Workspace {
                    workspace_path: Some("/repo".into()),
                },
            )),
        }),
        recovery: Some(false),
    }
}

async fn next_frame(connection: &mut TerminalConnection) -> wire::Stream {
    let frame = tokio::time::timeout(Duration::from_secs(5), connection.receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let bytes = connection.receive(frame).unwrap();
    assert!(bytes.len() <= wire::MAX_STREAM_FRAME_BYTES);
    let wire::envelope::Body::Stream(frame) = wire::Envelope::decode(bytes.as_slice())
        .unwrap()
        .body
        .unwrap()
    else {
        panic!("stream");
    };
    frame
}

async fn next_event(connection: &mut TerminalConnection) -> (String, u64, TerminalEvent) {
    let mut bytes = Vec::new();
    loop {
        let frame = next_frame(connection).await;
        bytes.extend(frame.data);
        if frame.end {
            return (
                frame.attachment_id,
                frame.sequence,
                TerminalEvent::decode(bytes.as_slice()).unwrap(),
            );
        }
        connection
            .acknowledge(&frame.attachment_id, frame.sequence)
            .unwrap();
    }
}

#[tokio::test]
async fn test_terminal_stream_単一接続でもattachmentは16個まででdetach後に枠を再利用する() {
    // Given
    let (mut connection, _) = fixture(None, false);
    for index in 0..16 {
        connection
            .attach(attach_args(&format!("a-{index}")))
            .unwrap();
    }
    // When / Then
    assert_eq!(
        connection
            .attach(attach_args("overflow"))
            .unwrap_err()
            .variant
            .and_then(|error| match error {
                wire::command_error::Variant::Coded(error) => error.code,
                _ => None,
            })
            .unwrap(),
        "ATTACHMENT_LIMIT"
    );
    connection.detach("a-0");
    connection.attach(attach_args("replacement")).unwrap();
    assert_eq!(
        connection
            .attach(attach_args("overflow"))
            .unwrap_err()
            .variant
            .and_then(|error| match error {
                wire::command_error::Variant::Coded(error) => error.code,
                _ => None,
            })
            .unwrap(),
        "ATTACHMENT_LIMIT"
    );
}

#[tokio::test]
async fn test_terminal_stream_全接続でattachment枠を共有し切断後に再利用する() {
    // Given
    let (mut first, _) = fixture(None, false);
    let mut second = TerminalConnection::new(first.deps.clone());
    for index in 0..8 {
        first.attach(attach_args(&format!("a-{index}"))).unwrap();
        second.attach(attach_args(&format!("b-{index}"))).unwrap();
    }
    // When / Then
    for connection in [&mut first, &mut second] {
        assert_eq!(
            connection
                .attach(attach_args("overflow"))
                .unwrap_err()
                .variant
                .and_then(|error| match error {
                    wire::command_error::Variant::Coded(error) => error.code,
                    _ => None,
                })
                .unwrap(),
            "ATTACHMENT_LIMIT"
        );
    }
    drop(first);
    for index in 0..8 {
        second.attach(attach_args(&format!("c-{index}"))).unwrap();
    }
    assert_eq!(
        second
            .attach(attach_args("overflow"))
            .unwrap_err()
            .variant
            .and_then(|error| match error {
                wire::command_error::Variant::Coded(error) => error.code,
                _ => None,
            })
            .unwrap(),
        "ATTACHMENT_LIMIT"
    );
}

#[tokio::test]
async fn test_terminal_stream_接続破棄でbackend_attachmentと保留creditを解放する() {
    // Given
    let gateway = Arc::new(BackendOwnedSurface::default());
    let hub = Arc::new(TerminalSurfaceEventHub::with_flags(256, true));
    let application = Arc::new(TerminalSurfaceApplication::new(
        gateway.clone(),
        hub.clone(),
    ));
    let mut connection = TerminalConnection::new(Some(TerminalApiDeps::new(application.clone())));
    connection.attach(attach_args("disconnected")).unwrap();
    next_event(&mut connection).await;
    let publish = |sequence, data: String| {
        let hub = hub.clone();
        tokio::task::spawn_blocking(move || {
            hub.publish(TerminalSurfaceEvent::Output {
                session_key: workspace_owner().stable_key(),
                data: data.into(),
                sequence,
            })
        })
    };
    publish(42, "x".repeat(TERMINAL_OUTPUT_CREDIT_CODE_UNITS))
        .await
        .unwrap();
    next_event(&mut connection).await;
    let mut blocked = publish(43, "pending".into());
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!blocked.is_finished());
    assert_eq!(hub.owner_stream_count(), 1);
    // When
    drop(connection);
    let completed = tokio::time::timeout(Duration::from_secs(5), &mut blocked).await;
    if completed.is_err() {
        application.detach("disconnected");
    }
    // Then
    completed
        .expect("接続破棄は保留output creditを解放する")
        .unwrap();
    assert_eq!(*gateway.deactivated.lock().unwrap(), vec!["disconnected"]);
    assert_eq!(hub.owner_stream_count(), 0);
}

#[tokio::test]
async fn test_terminal_stream_attach失敗でattachment枠を消費しない() {
    // Given
    let (mut connection, _) = fixture(None, false);
    let mut missing_surface = attach_args("missing");
    missing_surface.owner.as_mut().unwrap().variant =
        Some(wire::terminal_surface_owner_v1::Variant::Workspace(
            wire::TerminalSurfaceOwnerV1Workspace {
                workspace_path: Some("/missing".into()),
            },
        ));
    // When
    assert_eq!(
        connection
            .attach(missing_surface)
            .unwrap_err()
            .variant
            .and_then(|error| match error {
                wire::command_error::Variant::Coded(error) => error.code,
                _ => None,
            })
            .unwrap(),
        "PTY_ERROR"
    );
    // Then
    for index in 0..16 {
        connection
            .attach(attach_args(&format!("a-{index}")))
            .unwrap();
    }
    assert_eq!(
        connection
            .attach(attach_args("overflow"))
            .unwrap_err()
            .variant
            .and_then(|error| match error {
                wire::command_error::Variant::Coded(error) => error.code,
                _ => None,
            })
            .unwrap(),
        "ATTACHMENT_LIMIT"
    );
}

#[tokio::test]
async fn test_terminal_stream_巨大snapshotとoutputを上限内で分割し完全復元する() {
    // Given
    let replay = "日本語🙂\u{1b}[2J".repeat(120000);
    let (mut connection, hub) = fixture(Some(replay.clone()), false);
    // When
    connection.attach(attach_args("a")).unwrap();
    let (id, sequence, snapshot) = next_event(&mut connection).await;
    // Then
    assert_eq!(id, "a");
    assert!(sequence > 16, "windowより大きなsnapshotも中間ackで進む");
    let wire::terminal_event::Item::Snapshot(snapshot) = snapshot.item.unwrap() else {
        panic!("snapshot");
    };
    assert_eq!(snapshot.replay, replay);
    assert_eq!(snapshot.sequence, 41);
    connection.acknowledge(&id, sequence).unwrap();
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: workspace_owner().stable_key(),
        data: replay.clone().into(),
        sequence: 42,
    });
    let (_, output_sequence, output) = next_event(&mut connection).await;
    assert!(output_sequence > sequence);
    let wire::terminal_event::Item::Output(output) = output.item.unwrap() else {
        panic!("output");
    };
    assert_eq!(output.data, replay);
    assert_eq!(output.sequence, 42);
}

#[tokio::test]
async fn test_terminal_stream_ack停止はそのattachmentだけを止め再開できる() {
    // Given
    let (mut connection, _) = fixture(Some("x".repeat(2 * 1024 * 1024)), false);
    connection.attach(attach_args("a")).unwrap();
    // When
    for sequence in 1..=16 {
        let frame = next_frame(&mut connection).await;
        assert_eq!(
            (frame.attachment_id.as_str(), frame.sequence),
            ("a", sequence)
        );
    }
    // Then
    assert!(
        tokio::time::timeout(Duration::from_millis(30), connection.receiver.recv())
            .await
            .is_err()
    );
    connection.attach(attach_args("b")).unwrap();
    let frame = next_frame(&mut connection).await;
    assert_eq!((frame.attachment_id.as_str(), frame.sequence), ("b", 1));
    connection.detach("b");
    connection.acknowledge("a", 16).unwrap();
    loop {
        let raw = connection.receiver.recv().await.unwrap();
        if let Some(bytes) = connection.receive(raw) {
            let wire::envelope::Body::Stream(frame) = wire::Envelope::decode(bytes.as_slice())
                .unwrap()
                .body
                .unwrap()
            else {
                panic!("stream");
            };
            assert_eq!((frame.attachment_id.as_str(), frame.sequence), ("a", 17));
            break;
        }
    }
}

#[tokio::test]
async fn test_terminal_stream_出力の終端ackだけがbackend_creditを解放する() {
    // Given
    let (mut connection, hub) = fixture(None, true);
    connection.attach(attach_args("a")).unwrap();
    let (_, snapshot_sequence, _) = next_event(&mut connection).await;
    connection.acknowledge("a", snapshot_sequence).unwrap();
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: workspace_owner().stable_key(),
        data: "a".repeat(TERMINAL_OUTPUT_CREDIT_CODE_UNITS).into(),
        sequence: 42,
    });
    let (_, sequence, _) = next_event(&mut connection).await;
    let blocked = tokio::task::spawn_blocking(move || {
        hub.publish(TerminalSurfaceEvent::Output {
            session_key: workspace_owner().stable_key(),
            data: "tail".into(),
            sequence: 43,
        })
    });
    // When / Then
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!blocked.is_finished());
    connection.acknowledge("a", sequence).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!blocked.is_finished(), "受信ackは描画完了ではない");
    connection.acknowledge_output("a", 42);
    tokio::time::timeout(Duration::from_secs(5), blocked)
        .await
        .unwrap()
        .unwrap();
    let (_, _, output) = next_event(&mut connection).await;
    let wire::terminal_event::Item::Output(output) = output.item.unwrap() else {
        panic!("output");
    };
    assert_eq!(output.data, "tail");
}

#[tokio::test]
async fn test_terminal_stream_attachのid長境界は本番tauri入口と結果が一致する() {
    use crate::adaptor::controller::{command, state::AppState};
    use tauri::Manager;

    // Given
    let (mut connection, _) = fixture(None, false);
    let application = connection.deps.as_ref().unwrap().application.clone();
    let (fixture_app, _, _store) =
        crate::adaptor::controller::client::workflow::tests::make_read_only_app_with_terminal(
            application.clone(),
        );
    let app = tauri::test::mock_builder()
        .invoke_handler(command::terminal_surface::invoke_handler())
        .manage(fixture_app.state::<AppState>().inner().clone())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    for recovery in [false, true] {
        for (id, valid) in [
            (String::new(), false),
            ("a".repeat(129), false),
            ("é".repeat(65), false),
            ("a".repeat(128), true),
            ("é".repeat(64), true),
        ] {
            let args = json!({
                "attachmentId": id,
                "owner": {"kind": "workspace", "workspacePath": "/repo"},
                "recovery": recovery,
            });
            let request =
                wire::CommandRequest::from_value("attach_terminal_surface", args.clone()).unwrap();
            let request = wire::CommandRequest::decode(request.encode_to_vec().as_slice()).unwrap();
            let mut tauri_args = args;
            tauri_args["onEvent"] = json!("__CHANNEL__:2");

            // When
            let tauri_result = command::client::client_tests::invoke_tauri(
                &app,
                "attach_terminal_surface",
                tauri_args,
            )
            .await;
            application.detach(&id);
            let ws_result = connection
                .dispatch(request.command.unwrap())
                .map(|command| {
                    wire::from_value(wire::CommandResult {
                        command: Some(command),
                    })
                    .unwrap()
                })
                .map_err(|error| wire::from_value(error).unwrap());
            connection.detach(&id);

            // Then
            let expected = if valid {
                Ok(Value::Null)
            } else {
                Err(json!({"code": "INVALID_REQUEST", "message": "Invalid attachment ID"}))
            };
            assert_eq!(
                tauri_result,
                expected,
                "Tauri: bytes={}, recovery={recovery}",
                id.len()
            );
            assert_eq!(
                ws_result,
                expected,
                "ws: bytes={}, recovery={recovery}",
                id.len()
            );
        }
    }
}

#[tokio::test]
async fn test_terminal_stream_不正attachと未来ackを拒否しdetach後のframeを破棄する() {
    // Given
    let (mut connection, _) = fixture(None, false);
    for id in ["", &"a".repeat(129)] {
        assert_eq!(
            connection
                .attach(attach_args(id))
                .unwrap_err()
                .variant
                .and_then(|error| match error {
                    wire::command_error::Variant::Coded(error) => error.code,
                    _ => None,
                })
                .unwrap(),
            "INVALID_REQUEST"
        );
    }
    connection.attach(attach_args("a")).unwrap();
    assert_eq!(
        connection
            .attach(attach_args("a"))
            .unwrap_err()
            .variant
            .and_then(|error| match error {
                wire::command_error::Variant::Coded(error) => error.code,
                _ => None,
            })
            .unwrap(),
        "INVALID_REQUEST"
    );
    assert!(connection.acknowledge("unknown", 1).is_ok());
    assert!(connection.acknowledge("a", 1).is_err());
    let stale = connection.receiver.recv().await.unwrap();
    // When
    connection.detach("a");
    connection.attach(attach_args("a")).unwrap();
    // Then
    assert!(connection.receive(stale).is_none());
    assert_eq!(next_frame(&mut connection).await.sequence, 1);
    connection.acknowledge("a", 1).unwrap();
    connection.acknowledge("a", 0).unwrap();
}

#[tokio::test]
async fn test_terminal_stream_単一wsでattachと入力とresizeとpushと通常応答を多重化する() {
    use crate::adaptor::controller::{api, client::ClientCommandDispatch, state::AppState};
    use futures_util::{SinkExt, StreamExt};
    use tauri::Manager;
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    // Given
    let gateway = Arc::new(BackendOwnedSurface {
        replay: Some("x".repeat(2 * 1024 * 1024)),
        ..Default::default()
    });
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    let application = Arc::new(TerminalSurfaceApplication::new(gateway.clone(), hub));
    let (app, data_dir, _store) =
        crate::adaptor::controller::client::workflow::tests::make_read_only_app_with_terminal(
            application.clone(),
        );
    let repository = app.state::<AppState>().repository_usecase.clone();
    let mut dispatch = ClientCommandDispatch::new(
        repository,
        Arc::new(crate::usecase::application_startup::ApplicationStartupAuthority::ready()),
    );
    app.manage(Arc::new(
        crate::infrastructure::file_watcher::FileWatcherManager::default(),
    ));
    dispatch.register_dependencies(
        &crate::adaptor::controller::wiring::build_client_dependencies(app.handle()),
    );
    let sink = app
        .state::<Arc<crate::infrastructure::push::PushSink>>()
        .inner()
        .clone();
    let router = api::test_support::test_router_with_optional_deps(
        &data_dir,
        "master",
        "client",
        Some(TerminalApiDeps::new(application)),
        Some(api::ClientApiDeps::new(
            Arc::new(dispatch),
            crate::adaptor::gateway::push::ClientPushGateway::new(sink.clone()),
        )),
        None,
    )
    .0;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let mut request = format!("ws://{address}/v1/client")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("authorization", "Bearer client".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let send = |id: &str, command: &str, args: Value| {
        Message::Binary(
            crate::client_api_acceptance::encode_client_request(id, command, args).into(),
        )
    };
    // When
    socket
        .send(send("attach", "attach_terminal_surface", json!({"attachmentId":"a","owner":{"kind":"workspace","workspacePath":"/repo"},"recovery":false})))
        .await
        .unwrap();
    for index in 0..=16 {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Message::Binary(bytes) = message else {
            panic!("binary");
        };
        let body = wire::Envelope::decode(bytes.clone()).unwrap().body.unwrap();
        if index == 0 {
            assert!(matches!(body, wire::envelope::Body::Response(_)));
        } else {
            assert!(bytes.len() <= wire::MAX_STREAM_FRAME_BYTES);
            let wire::envelope::Body::Stream(frame) = body else {
                panic!("stream");
            };
            assert_eq!(frame.sequence, index);
        }
    }
    socket.send(send("write", "write_terminal_surface", json!({"owner":{"kind":"workspace","workspacePath":"/repo"},"attachmentId":"a","sequence":0,"data":"echo hi\n"}))).await.unwrap();
    for rows in 40..45 {
        socket.send(send(&format!("resize-{rows}"), "resize_terminal_surface",
            json!({"owner":{"kind":"workspace","workspacePath":"/repo"},"rows":rows,"cols":rows * 3})
        )).await.unwrap();
    }
    socket
        .send(Message::Binary(
            wire::Envelope {
                body: Some(wire::envelope::Body::Ack(wire::Ack {
                    attachment_id: "detached".into(),
                    sequence: 99,
                    output_sequence: Some(99),
                })),
            }
            .encode_to_vec()
            .into(),
        ))
        .await
        .unwrap();
    socket
        .send(send(
            "query",
            "get_language_from_path",
            json!({"filePath":"main.rs"}),
        ))
        .await
        .unwrap();
    for _ in 0..128 {
        crate::adaptor::gateway::push::BackendPush::BranchListSync.emit(app.handle());
    }
    // Then
    let mut ids = std::collections::HashSet::new();
    let mut pushed = false;
    let mut resynced = false;
    while ids.len() < 7 || !pushed || !resynced {
        let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let Message::Binary(bytes) = frame else {
            panic!("binary");
        };
        match wire::Envelope::decode(bytes).unwrap().body.unwrap() {
            wire::envelope::Body::Response(response) => {
                let wire::command_response::Outcome::Result(result) = response.outcome.unwrap()
                else {
                    panic!("success");
                };
                let result = wire::from_value(result).unwrap();
                if response.request_id == "query" {
                    assert_eq!(result, "rust");
                } else {
                    assert_eq!(result, Value::Null);
                }
                ids.insert(response.request_id);
            }
            wire::envelope::Body::Push(_) => pushed = true,
            wire::envelope::Body::PushResync(_) => {
                resynced = true;
                crate::adaptor::gateway::push::BackendPush::BranchListSync.emit(app.handle());
            }
            _ => panic!("unacked stream must stay blocked"),
        }
    }
    assert!(pushed);
    assert_eq!(ids.len(), 7);
    assert_eq!(
        *gateway.attached_writes.lock().unwrap(),
        vec![AttachedWrite {
            session_key: workspace_owner().stable_key(),
            attachment_id: "a".into(),
            sequence: 0,
            data: "echo hi\n".into()
        }]
    );
    assert_eq!(
        *gateway.resizes.lock().unwrap(),
        (40..45)
            .map(|rows| (workspace_owner().stable_key(), rows, rows * 3))
            .collect::<Vec<_>>()
    );
    socket
        .send(Message::Binary(
            wire::Envelope {
                body: Some(wire::envelope::Body::Ack(wire::Ack {
                    attachment_id: "a".into(),
                    sequence: 16,
                    output_sequence: None,
                })),
            }
            .encode_to_vec()
            .into(),
        ))
        .await
        .unwrap();
    let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Binary(bytes) = message else {
        panic!("binary");
    };
    let wire::envelope::Body::Stream(frame) = wire::Envelope::decode(bytes).unwrap().body.unwrap()
    else {
        panic!("stream");
    };
    assert_eq!(frame.sequence, 17);
    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn test_terminal_stream_自然終了は最後のframeの後に全attachment枠を解放する() {
    // Given
    let gateway = Arc::new(BackendOwnedSurface::default());
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    let application = Arc::new(TerminalSurfaceApplication::new(
        gateway.clone(),
        hub.clone(),
    ));
    let mut connection = TerminalConnection::new(Some(TerminalApiDeps::new(application)));
    // When / Then
    for index in 0..20 {
        let id = format!("a-{index}");
        connection.attach(attach_args(&id)).unwrap();
        next_event(&mut connection).await;
        hub.publish(TerminalSurfaceEvent::Exit {
            session_key: workspace_owner().stable_key(),
            runtime_generation: 7,
            exit_code: Some(0),
            sequence: 42,
        });
        let (_, _, event) = next_event(&mut connection).await;
        assert!(matches!(
            event.item,
            Some(wire::terminal_event::Item::Exit(_))
        ));
        let finished = tokio::time::timeout(Duration::from_secs(5), connection.receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(connection.receive(finished).is_none());
        assert!(connection.attachments.is_empty());
        assert_eq!(
            connection
                .deps
                .as_ref()
                .unwrap()
                .attachment_limit
                .available_permits(),
            16
        );
        assert_eq!(gateway.deactivated.lock().unwrap().last(), Some(&id));
        connection.acknowledge(&id, 100).unwrap();
    }
}

type ClientSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next_ws_body(socket: &mut ClientSocket) -> wire::envelope::Body {
    use futures_util::StreamExt;
    let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let tokio_tungstenite::tungstenite::Message::Binary(bytes) = message else {
        panic!("binary");
    };
    wire::Envelope::decode(bytes).unwrap().body.unwrap()
}

fn assert_ws_result(body: wire::envelope::Body, id: &str, expected: &Value) {
    let wire::envelope::Body::Response(response) = body else {
        panic!("response");
    };
    assert_eq!(response.request_id, id);
    let wire::command_response::Outcome::Result(result) = response.outcome.unwrap() else {
        panic!("success");
    };
    assert_eq!(&wire::from_value(result).unwrap(), expected);
}

async fn next_ws_event(
    socket: &mut ClientSocket,
    mut response: Option<(&str, &Value)>,
) -> (u64, TerminalEvent) {
    let mut bytes = Vec::new();
    let mut last_sequence = None;
    while last_sequence.is_none() || response.is_some() {
        match next_ws_body(socket).await {
            wire::envelope::Body::Stream(frame) => {
                assert_eq!(frame.attachment_id, "a");
                bytes.extend(frame.data);
                if frame.end {
                    last_sequence = Some(frame.sequence);
                }
            }
            body => {
                let (id, expected) = response.take().expect("pending response");
                assert_ws_result(body, id, expected);
            }
        }
    }
    (
        last_sequence.unwrap(),
        TerminalEvent::decode(bytes.as_slice()).unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn test_terminal_stream_実wsのackとdetachはtauriと結果とbackend作用が一致する() {
    use crate::adaptor::controller::{api, client::ClientCommandDispatch, state::AppState};
    use futures_util::SinkExt;
    use tauri::Manager;
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

    let mut tauri_ack = None;
    let mut tauri_detach = None;
    for transport in ["tauri", "command", "envelope"] {
        // Given
        let gateway = Arc::new(BackendOwnedSurface::default());
        let hub = Arc::new(TerminalSurfaceEventHub::with_flags(256, true));
        let application = Arc::new(TerminalSurfaceApplication::new(
            gateway.clone(),
            hub.clone(),
        ));
        let (app, data_dir, _store) =
            crate::adaptor::controller::client::workflow::tests::make_read_only_app_with_terminal(
                application.clone(),
            );
        let mut dispatch = ClientCommandDispatch::new(
            app.state::<AppState>().repository_usecase.clone(),
            Arc::new(crate::usecase::application_startup::ApplicationStartupAuthority::ready()),
        );
        app.manage(Arc::new(
            crate::infrastructure::file_watcher::FileWatcherManager::default(),
        ));
        dispatch.register_dependencies(
            &crate::adaptor::controller::wiring::build_client_dependencies(app.handle()),
        );
        let dispatch = Arc::new(dispatch);
        app.manage(dispatch.clone());
        let router = api::test_support::test_router_with_optional_deps(
            &data_dir,
            "master",
            "client",
            Some(TerminalApiDeps::new(application.clone())),
            Some(api::ClientApiDeps::new(
                dispatch.clone(),
                crate::adaptor::gateway::push::ClientPushGateway::new(
                    app.state::<Arc<crate::infrastructure::push::PushSink>>()
                        .inner()
                        .clone(),
                ),
            )),
            None,
        )
        .0;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let mut request = format!("ws://{address}/v1/client")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer client".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let send = |id: &str, command: &str, args: Value| {
            Message::Binary(
                crate::client_api_acceptance::encode_client_request(id, command, args).into(),
            )
        };
        let ack = |sequence, output_sequence| {
            Message::Binary(
                wire::Envelope {
                    body: Some(wire::envelope::Body::Ack(wire::Ack {
                        attachment_id: "a".into(),
                        sequence,
                        output_sequence,
                    })),
                }
                .encode_to_vec()
                .into(),
            )
        };
        socket
            .send(send("attach", "attach_terminal_surface", json!({"attachmentId":"a","owner":{"kind":"workspace","workspacePath":"/repo"},"recovery":false})))
            .await
            .unwrap();
        assert_ws_result(next_ws_body(&mut socket).await, "attach", &Value::Null);
        let (_, snapshot) = next_ws_event(&mut socket, None).await;
        assert!(matches!(
            snapshot.item,
            Some(wire::terminal_event::Item::Snapshot(_))
        ));
        for command in ["ack_terminal_surface_output", "detach_terminal_surface"] {
            let mut args = json!({"attachmentId":"unknown"});
            let expected = if command == "ack_terminal_surface_output" {
                args["sequence"] = json!(99);
                crate::adaptor::controller::command::client::client_tests::invoke_tauri(
                    &app,
                    "ack_terminal_surface_output",
                    json!({"attachmentId":"unknown", "sequence":99}),
                )
                .await
                .unwrap()
            } else {
                crate::adaptor::controller::command::client::client_tests::invoke_tauri(
                    &app,
                    "detach_terminal_surface",
                    json!({"attachmentId":"unknown"}),
                )
                .await
                .unwrap()
            };
            socket.send(send(command, command, args)).await.unwrap();
            assert_ws_result(next_ws_body(&mut socket).await, command, &expected);
        }
        assert!(gateway.deactivated.lock().unwrap().is_empty());
        assert_eq!(hub.owner_stream_count(), 1);
        let publish = |sequence, data: String| {
            let hub = hub.clone();
            tokio::task::spawn_blocking(move || {
                hub.publish(TerminalSurfaceEvent::Output {
                    session_key: workspace_owner().stable_key(),
                    data: data.into(),
                    sequence,
                })
            })
        };
        publish(42, "x".repeat(TERMINAL_OUTPUT_CREDIT_CODE_UNITS))
            .await
            .unwrap();
        let (sequence, output) = next_ws_event(&mut socket, None).await;
        assert!(
            matches!(output.item, Some(wire::terminal_event::Item::Output(ref output)) if output.sequence == 42)
        );
        let mut blocked = publish(43, "y".repeat(TERMINAL_OUTPUT_CREDIT_CODE_UNITS));
        // When / Then
        socket.send(ack(sequence, None)).await.unwrap();
        socket
            .send(send(
                "barrier",
                "get_language_from_path",
                json!({"filePath":"main.rs"}),
            ))
            .await
            .unwrap();
        assert_ws_result(next_ws_body(&mut socket).await, "barrier", &json!("rust"));
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            !blocked.is_finished(),
            "{transport}: 受信ackだけではbackend creditを解放しない"
        );
        let response = match transport {
            "tauri" => {
                tauri_ack = Some(
                    crate::adaptor::controller::command::client::client_tests::invoke_tauri(
                        &app,
                        "ack_terminal_surface_output",
                        json!({"attachmentId":"a", "sequence":42}),
                    )
                    .await
                    .unwrap(),
                );
                None
            }
            "command" => {
                socket
                    .send(send(
                        "ack",
                        "ack_terminal_surface_output",
                        json!({"attachmentId":"a","sequence":42}),
                    ))
                    .await
                    .unwrap();
                Some(("ack", tauri_ack.as_ref().unwrap()))
            }
            "envelope" => {
                socket.send(ack(sequence, Some(42))).await.unwrap();
                None
            }
            _ => unreachable!(),
        };
        let completed = tokio::time::timeout(Duration::from_secs(5), &mut blocked).await;
        if completed.is_err() {
            application.detach("a");
        }
        completed
            .expect("output ackはbackend creditを解放する")
            .unwrap();
        let (_, output) = next_ws_event(&mut socket, response).await;
        let Some(wire::terminal_event::Item::Output(output)) = output.item else {
            panic!("output");
        };
        assert_eq!(output.sequence, 43);
        assert_eq!(output.data, "y".repeat(TERMINAL_OUTPUT_CREDIT_CODE_UNITS));
        let mut blocked = publish(44, "tail".into());
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!blocked.is_finished());
        if transport == "tauri" {
            tauri_detach = Some(
                crate::adaptor::controller::command::client::client_tests::invoke_tauri(
                    &app,
                    "detach_terminal_surface",
                    json!({"attachmentId":"a"}),
                )
                .await
                .unwrap(),
            );
        } else {
            socket
                .send(send(
                    "detach",
                    "detach_terminal_surface",
                    json!({"attachmentId":"a"}),
                ))
                .await
                .unwrap();
            assert_ws_result(
                next_ws_body(&mut socket).await,
                "detach",
                tauri_detach.as_ref().unwrap(),
            );
        }
        let completed = tokio::time::timeout(Duration::from_secs(5), &mut blocked).await;
        if completed.is_err() {
            application.detach("a");
        }
        completed
            .expect("detachはbackend creditを解放する")
            .unwrap();
        assert_eq!(*gateway.deactivated.lock().unwrap(), vec!["a"]);
        assert_eq!(hub.owner_stream_count(), 0);
        socket.close(None).await.unwrap();
        server.abort();
    }
}

#[tokio::test]
async fn test_terminal_stream_exitなしの終了はattachmentだけ通知し他streamを継続する() {
    // Given
    let gateway = Arc::new(BackendOwnedSurface::default());
    let hub = Arc::new(TerminalSurfaceEventHub::new());
    let application = Arc::new(TerminalSurfaceApplication::new(
        gateway.clone(),
        hub.clone(),
    ));
    let mut connection = TerminalConnection::new(Some(TerminalApiDeps::new(application)));
    connection.attach(attach_args("a")).unwrap();
    next_event(&mut connection).await;
    // When
    gateway
        .missing_snapshot
        .store(true, std::sync::atomic::Ordering::SeqCst);
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: workspace_owner().stable_key(),
        data: "gap".into(),
        sequence: 43,
    });
    let raw = tokio::time::timeout(Duration::from_secs(5), connection.receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let bytes = connection
        .receive(raw)
        .expect("stream closure must reach the client");
    // Then
    let wire::envelope::Body::StreamClosed(closed) = wire::Envelope::decode(bytes.as_slice())
        .unwrap()
        .body
        .unwrap()
    else {
        panic!("closed");
    };
    assert_eq!(closed.attachment_id, "a");
    assert!(connection.attachments.is_empty());
    assert_eq!(*gateway.deactivated.lock().unwrap(), ["a"]);
    gateway
        .missing_snapshot
        .store(false, std::sync::atomic::Ordering::SeqCst);
    connection.attach(attach_args("b")).unwrap();
    assert_eq!(next_event(&mut connection).await.0, "b");
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: workspace_owner().stable_key(),
        data: "continued".into(),
        sequence: 42,
    });
    assert!(matches!(
        next_event(&mut connection).await.2.item,
        Some(wire::terminal_event::Item::Output(_))
    ));
}
