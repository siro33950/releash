use super::*;
use crate::adaptor::gateway::terminal_surface::event_hub::TerminalSurfaceEventHub;
use crate::domain::terminal_surface::entities::TerminalSurface;
use crate::domain::terminal_surface::gateway::{
    TerminalSurfaceEvent, TerminalSurfaceEventSink, TerminalSurfaceRepository,
};
use crate::domain::terminal_surface::TerminalSurfaceOwner;
use crate::domain::workspace_tree::WorkspaceIdentity;
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
    exited: std::sync::atomic::AtomicBool,
    missing_snapshot: std::sync::atomic::AtomicBool,
    attached_writes: std::sync::Mutex<Vec<AttachedWrite>>,
    resizes: std::sync::Mutex<Vec<(String, u16, u16)>>,
    resize_gate: Option<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>,
    resize_started: tokio::sync::Notify,
    panic_resize: std::sync::atomic::AtomicBool,
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
            if self.exited.load(std::sync::atomic::Ordering::SeqCst) {
                surface.process_state =
                    crate::domain::terminal_surface::TerminalProcessState::Exited {
                        exit_code: Some(0),
                    };
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
        assert!(
            !self.panic_resize.load(std::sync::atomic::Ordering::SeqCst),
            "resize panic"
        );
        if rows == 40 {
            if let Some(gate) = &self.resize_gate {
                self.resize_started.notify_one();
                gate.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|error| {
                        crate::domain::terminal_surface::gateway::TerminalSurfaceGatewayError::new(
                            error.to_string(),
                        )
                    })?;
            } else {
                std::thread::sleep(Duration::from_millis(30));
            }
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

fn fixture() -> (
    TerminalApiDeps,
    Arc<TerminalSurfaceEventHub>,
    Arc<BackendOwnedSurface>,
    Arc<TerminalSurfaceApplication>,
) {
    let gateway = Arc::new(BackendOwnedSurface::default());
    let hub = Arc::new(TerminalSurfaceEventHub::with_flags(256, true));
    let application = Arc::new(TerminalSurfaceApplication::new(
        gateway.clone(),
        hub.clone(),
    ));
    (
        TerminalApiDeps::new(application.clone()),
        hub,
        gateway,
        application,
    )
}

#[tokio::test]
async fn test_terminal_stream_snapshotと出力を配信しdropでattachmentと枠を解放する() {
    use futures_util::StreamExt;
    // Given
    let (deps, hub, gateway, _) = fixture();
    let mut stream = stream(&deps, attach_args("terminal")).unwrap();
    // When / Then
    let first = stream.next().await.unwrap().unwrap();
    let Some(wire::terminal_event::Item::Snapshot(snapshot)) = first.item else {
        panic!("snapshot");
    };
    assert_eq!(snapshot.sequence, 41);
    assert_eq!(snapshot.replay, "\u{1b}[2Jshared backend screen");
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: workspace_owner().stable_key(),
        data: "next".into(),
        sequence: 42,
    });
    let next = stream.next().await.unwrap().unwrap();
    assert!(
        matches!(next.item, Some(wire::terminal_event::Item::Output(output)) if output.data == "next" && output.sequence == 42)
    );
    drop(stream);
    assert_eq!(*gateway.deactivated.lock().unwrap(), ["terminal"]);
}

#[tokio::test]
async fn test_terminal_stream_全streamで枠を共有し不正入力では消費しない() {
    // Given
    let (deps, _, _, _) = fixture();
    let mut invalid = attach_args("invalid");
    invalid.owner = None;
    assert!(stream(&deps, invalid).is_err());
    // When
    let mut output = deps.subscribe("limit".into()).unwrap();
    output.next().await.unwrap().unwrap();
    for index in 0..16 {
        deps.attach("limit", String::new(), attach_args(&format!("id-{index}")))
            .unwrap();
        output.next().await.unwrap().unwrap();
    }
    // Then
    let error = deps
        .attach("limit", String::new(), attach_args("overflow"))
        .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::ResourceExhausted);
    assert_eq!(error.details.len(), 1);
    assert_eq!(error.details[0].type_url, "releash.client.v1.CommandError");
    use base64::Engine;
    use prost::Message;
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(error.details[0].value.as_ref().unwrap())
        .unwrap();
    let detail = wire::CommandError::decode(bytes.as_slice()).unwrap();
    let Some(wire::command_error::Variant::Coded(detail)) = detail.variant else {
        panic!("coded error");
    };
    assert_eq!(detail.code.as_deref(), Some("TERMINAL_ATTACHMENT_LIMIT"));
    assert_eq!(
        detail.message.as_deref(),
        Some("Too many terminal attachments")
    );
    drop(output);
    assert!(stream(&deps, attach_args("reused")).is_ok());
}

#[tokio::test]
async fn test_terminal_connectは同じattachmentの出力streamと入力unaryを共有する() {
    use super::super::protocol::connect::{to_rpc, to_wire};
    use crate::adaptor::controller::client::{convert, required, ClientCommandDispatch};
    // Given
    let (terminal, hub, gateway, terminal_application) = fixture();
    let runtime = Arc::new(crate::usecase::workflow::WorkflowRuntimeUsecase::new(
        Arc::new(super::super::test_support::RecordingRuntimeGateway::default()),
        Arc::new(crate::usecase::workflow::NoopArchiveRepository),
    ));
    let mut dispatch = ClientCommandDispatch::new(Arc::new(
        crate::usecase::application_startup::ApplicationStartupAuthority::ready(),
    ))
    .with_worktree_mutations(runtime);
    let application = terminal_application.clone();
    dispatch.register_domain(
        &["write_terminal_surface"],
        Box::new(move |command| {
            let application = application.clone();
            Box::pin(async move {
                let wire::command_request::Command::WriteTerminalSurface(args) = command else {
                    unreachable!()
                };
                let owner: crate::adaptor::protocol::terminal::TerminalSurfaceOwnerV1 =
                    convert(required(args.owner, "owner")?)?;
                let owner = owner.try_into().map_err(|error| {
                    crate::other::AppError::new(error)
                        .with_failure_kind(crate::domain::failure::FailureKind::InvalidInput)
                })?;
                application
                    .write_attached(
                        &owner,
                        &required(args.attachment_id, "attachmentId")?,
                        required(args.sequence, "sequence")?,
                        None,
                        &required(args.data, "data")?,
                    )
                    .map_err(crate::other::AppError::from_failure)?;
                Ok(wire::command_result::Command::WriteTerminalSurface(
                    wire::Unit {},
                ))
            })
        }),
    );
    let application = terminal_application.clone();
    dispatch.register_domain(
        &["ack_terminal_surface_output"],
        Box::new(move |command| {
            let application = application.clone();
            Box::pin(async move {
                let wire::command_request::Command::AckTerminalSurfaceOutput(args) = command else {
                    unreachable!()
                };
                application.acknowledge_output(
                    &required(args.attachment_id, "attachmentId")?,
                    required(args.sequence, "sequence")?,
                );
                Ok(wire::command_result::Command::AckTerminalSurfaceOutput(
                    wire::Unit {},
                ))
            })
        }),
    );
    let deps = super::super::client::ClientApiDeps::new(
        Arc::new(dispatch),
        crate::adaptor::gateway::push::ClientPushGateway::new(Arc::new(
            crate::infrastructure::push::PushSink::new(),
        )),
        crate::client_api_acceptance::watcher(),
    )
    .with_terminal(Some(terminal));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = connectrpc::client::ClientConfig::new(
        format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap(),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, super::super::client::router(Some(deps)))
            .await
            .unwrap();
    });
    let client = rpc::ClientServiceClient::new(connectrpc::client::HttpClient::plaintext(), config);
    // When
    let mut output = client
        .subscribe_terminal_surfaces(rpc::SubscribeTerminalSurfacesRequest {
            subscription_id: "terminals".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let ready = output
        .message::<rpc::TerminalSubscriptionEvent>()
        .await
        .unwrap()
        .unwrap()
        .to_owned_message();
    assert!(matches!(
        to_wire::<wire::TerminalSubscriptionEvent>(&ready)
            .unwrap()
            .event,
        Some(wire::terminal_subscription_event::Event::Ready(_))
    ));
    client
        .attach_terminal_surface(
            to_rpc::<rpc::AttachSubscribedTerminalSurfaceRequest>(
                &wire::AttachSubscribedTerminalSurfaceRequest {
                    subscription_id: "terminals".into(),
                    stream_id: "rpc-stream".into(),
                    request: Some(attach_args("rpc-terminal")),
                },
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(matches!(
        terminal_item(
            &output
                .message::<rpc::TerminalSubscriptionEvent>()
                .await
                .unwrap()
                .unwrap()
                .to_owned_message(),
            "rpc-terminal",
            "rpc-stream",
        )
        .unwrap()
        .item,
        Some(wire::terminal_event::Item::Snapshot(_))
    ));
    client
        .write_terminal_surface(
            to_rpc::<rpc::WriteTerminalSurfaceRequest>(&wire::WriteTerminalSurfaceRequest {
                owner: attach_args("rpc-terminal").owner,
                attachment_id: Some("rpc-terminal".into()),
                sequence: Some(0),
                data: Some("日本語\n".into()),
                client_started_at_unix_ms: None,
            })
            .unwrap(),
        )
        .await
        .unwrap();
    let data = "a".repeat(256 * 1024);
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: workspace_owner().stable_key(),
        data: data.clone().into(),
        sequence: 42,
    });
    // Then
    let event = terminal_item(
        &output
            .message::<rpc::TerminalSubscriptionEvent>()
            .await
            .unwrap()
            .unwrap()
            .to_owned_message(),
        "rpc-terminal",
        "rpc-stream",
    )
    .unwrap();
    assert!(
        matches!(event.item, Some(wire::terminal_event::Item::Output(item)) if item.data == data)
    );
    assert_eq!(
        *gateway.attached_writes.lock().unwrap(),
        [AttachedWrite {
            session_key: workspace_owner().stable_key(),
            attachment_id: "rpc-terminal".into(),
            sequence: 0,
            data: "日本語\n".into()
        }]
    );
    // When: producer cannot publish more output until the unary ack returns credit.
    let producer_hub = hub.clone();
    let (published, mut resumed) = tokio::sync::oneshot::channel();
    let producer = tokio::task::spawn_blocking(move || {
        producer_hub.publish(TerminalSurfaceEvent::Output {
            session_key: workspace_owner().stable_key(),
            data: "resumed".into(),
            sequence: 43,
        });
        published.send(()).unwrap();
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut resumed)
            .await
            .is_err()
    );
    client
        .ack_terminal_surface_output(rpc::AckTerminalSurfaceOutputRequest {
            attachment_id: Some("rpc-terminal".into()),
            sequence: Some(42),
            ..Default::default()
        })
        .await
        .unwrap();
    // Then
    tokio::time::timeout(Duration::from_secs(1), resumed)
        .await
        .unwrap()
        .unwrap();
    producer.await.unwrap();
    let event = terminal_item(
        &output
            .message::<rpc::TerminalSubscriptionEvent>()
            .await
            .unwrap()
            .unwrap()
            .to_owned_message(),
        "rpc-terminal",
        "rpc-stream",
    )
    .unwrap();
    assert!(
        matches!(event.item, Some(wire::terminal_event::Item::Output(item)) if item.data == "resumed" && item.sequence == 43)
    );
    // When: explicit detach must release the attachment before the client drops its stream.
    client
        .detach_terminal_surface(rpc::DetachTerminalSurfaceRequest {
            attachment_id: Some("rpc-terminal".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    // Then
    assert_eq!(*gateway.deactivated.lock().unwrap(), ["rpc-terminal"]);
    let closed = tokio::time::timeout(
        Duration::from_secs(1),
        output.message::<rpc::TerminalSubscriptionEvent>(),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap()
    .to_owned_message();
    let closed = to_wire::<wire::TerminalSubscriptionEvent>(&closed).unwrap();
    assert_eq!(closed.attachment_id, "rpc-terminal");
    assert_eq!(closed.stream_id, "rpc-stream");
    assert!(matches!(
        closed.event,
        Some(wire::terminal_subscription_event::Event::Closed(closed)) if closed.resynchronize
    ));
    for ending in ["exit", "snapshot"] {
        // Given
        gateway
            .exited
            .store(ending == "snapshot", std::sync::atomic::Ordering::SeqCst);
        client
            .attach_terminal_surface(
                to_rpc::<rpc::AttachSubscribedTerminalSurfaceRequest>(
                    &wire::AttachSubscribedTerminalSurfaceRequest {
                        subscription_id: "terminals".into(),
                        stream_id: ending.into(),
                        request: Some(attach_args("rpc-terminal")),
                    },
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let snapshot = terminal_item(
            &output
                .message::<rpc::TerminalSubscriptionEvent>()
                .await
                .unwrap()
                .unwrap()
                .to_owned_message(),
            "rpc-terminal",
            ending,
        )
        .unwrap();
        assert!(
            matches!(snapshot.item, Some(wire::terminal_event::Item::Snapshot(snapshot)) if snapshot.is_exited == (ending == "snapshot"))
        );
        // When
        if ending == "exit" {
            hub.publish(TerminalSurfaceEvent::Exit {
                session_key: workspace_owner().stable_key(),
                runtime_generation: 7,
                exit_code: Some(0),
                sequence: 42,
            });
            let exit = terminal_item(
                &output
                    .message::<rpc::TerminalSubscriptionEvent>()
                    .await
                    .unwrap()
                    .unwrap()
                    .to_owned_message(),
                "rpc-terminal",
                ending,
            )
            .unwrap();
            assert!(matches!(
                exit.item,
                Some(wire::terminal_event::Item::Exit(_))
            ));
        }
        // Then
        let closed = tokio::time::timeout(
            Duration::from_secs(1),
            output.message::<rpc::TerminalSubscriptionEvent>(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .to_owned_message();
        let closed = to_wire::<wire::TerminalSubscriptionEvent>(&closed).unwrap();
        assert_eq!(closed.attachment_id, "rpc-terminal");
        assert_eq!(closed.stream_id, ending);
        assert!(
            matches!(closed.event, Some(wire::terminal_subscription_event::Event::Closed(closed)) if !closed.resynchronize)
        );
    }
    drop(output);
    server.abort();
}

fn terminal_item(
    event: &rpc::TerminalSubscriptionEvent,
    attachment_id: &str,
    stream_id: &str,
) -> Result<wire::TerminalEvent, ConnectError> {
    let event = super::super::protocol::connect::to_wire::<wire::TerminalSubscriptionEvent>(event)?;
    assert_eq!(event.attachment_id, attachment_id);
    assert_eq!(event.stream_id, stream_id);
    let Some(wire::terminal_subscription_event::Event::Item(item)) = event.event else {
        panic!("terminal item")
    };
    Ok(item)
}

#[tokio::test]
async fn test_terminal購読_最大attachment数を一本で配信し個別解除と購読終了で解放する() {
    // Given
    let (deps, hub, gateway, _) = fixture();
    let mut stream = deps.subscribe("renderer".into()).unwrap();
    let ready = stream.next().await.unwrap().unwrap();
    assert!(matches!(
        super::super::protocol::connect::to_wire::<wire::TerminalSubscriptionEvent>(&ready)
            .unwrap()
            .event,
        Some(wire::terminal_subscription_event::Event::Ready(_))
    ));
    // When
    for index in 0..16 {
        deps.attach(
            "renderer",
            String::new(),
            attach_args(&format!("pane-{index}")),
        )
        .unwrap();
    }
    // Then
    assert_eq!(
        deps.attach("renderer", String::new(), attach_args("queued-overflow"))
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    let mut received = std::collections::HashSet::new();
    for _ in 0..16 {
        let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let event =
            super::super::protocol::connect::to_wire::<wire::TerminalSubscriptionEvent>(&event)
                .unwrap();
        assert!(matches!(
            event.event,
            Some(wire::terminal_subscription_event::Event::Item(
                wire::TerminalEvent {
                    item: Some(wire::terminal_event::Item::Snapshot(_))
                }
            ))
        ));
        received.insert(event.attachment_id);
    }
    assert_eq!(received.len(), 16);
    assert_eq!(
        deps.attach("renderer", String::new(), attach_args("overflow"))
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    // When / Then
    hub.publish(TerminalSurfaceEvent::Output {
        session_key: workspace_owner().stable_key(),
        data: "live".into(),
        sequence: 42,
    });
    for _ in 0..16 {
        let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let event =
            super::super::protocol::connect::to_wire::<wire::TerminalSubscriptionEvent>(&event)
                .unwrap();
        assert!(received.remove(&event.attachment_id));
        assert!(
            matches!(event.event, Some(wire::terminal_subscription_event::Event::Item(wire::TerminalEvent {
            item: Some(wire::terminal_event::Item::Output(output))
        })) if output.data == "live")
        );
    }
    deps.detach("pane-0");
    let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let event = super::super::protocol::connect::to_wire::<wire::TerminalSubscriptionEvent>(&event)
        .unwrap();
    assert_eq!(event.attachment_id, "pane-0");
    assert!(matches!(
        event.event,
        Some(wire::terminal_subscription_event::Event::Closed(_))
    ));
    deps.attach("renderer", String::new(), attach_args("replacement"))
        .unwrap();
    drop(stream);
    let deactivated = gateway.deactivated.lock().unwrap();
    for id in (0..16)
        .map(|index| format!("pane-{index}"))
        .chain(["replacement".into()])
    {
        assert!(deactivated.contains(&id));
    }
}

#[tokio::test]
async fn test_terminal購読_未登録と重複と上限を拒否し別購読の出力を混ぜない() {
    // Given
    let (deps, _, _, _) = fixture();
    assert_eq!(
        deps.subscribe(" ".into()).err().unwrap().code,
        connectrpc::ErrorCode::InvalidArgument
    );
    assert_eq!(
        deps.attach("missing", String::new(), attach_args("missing"))
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::NotFound
    );
    let mut first = deps.subscribe("first".into()).unwrap();
    let mut second = deps.subscribe("second".into()).unwrap();
    first.next().await.unwrap().unwrap();
    second.next().await.unwrap().unwrap();
    // When / Then
    assert_eq!(
        deps.subscribe("first".into()).err().unwrap().code,
        connectrpc::ErrorCode::AlreadyExists
    );
    deps.attach("first", String::new(), attach_args("first-terminal"))
        .unwrap();
    first.next().await.unwrap().unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(20), second.next())
            .await
            .is_err()
    );
    let others: Vec<_> = (0..14)
        .map(|index| deps.subscribe(format!("client-{index}")).unwrap())
        .collect();
    assert_eq!(
        deps.subscribe("overflow".into()).err().unwrap().code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    drop(others);
    drop(first);
    assert_eq!(
        deps.attach("first", String::new(), attach_args("closed"))
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::NotFound
    );
    assert!(deps.subscribe("first".into()).is_ok());
}

fn stream(
    deps: &TerminalApiDeps,
    args: wire::AttachTerminalSurfaceRequest,
) -> Result<ServiceStream<wire::TerminalEvent>, ConnectError> {
    let subscription_id = uuid::Uuid::new_v4().to_string();
    let output = deps.subscribe(subscription_id.clone())?;
    deps.attach(&subscription_id, String::new(), args)?;
    Ok(Box::pin(output.filter_map(|event| async move {
        let event = match event.and_then(|event| {
            super::super::protocol::connect::to_wire::<wire::TerminalSubscriptionEvent>(&event)
        }) {
            Ok(event) => event,
            Err(error) => return Some(Err(error)),
        };
        match event.event {
            Some(wire::terminal_subscription_event::Event::Item(item)) => Some(Ok(item)),
            _ => None,
        }
    })))
}

#[tokio::test]
async fn test_terminal購読_識別子は128byteまで受理し超過時はattachmentを保持しない() {
    // Given
    let (deps, hub, _, _) = fixture();
    for id in ["x".repeat(129), "あ".repeat(43)] {
        // When / Then
        assert_eq!(
            deps.subscribe(id.clone()).err().unwrap().code,
            connectrpc::ErrorCode::InvalidArgument
        );
        let subscription = deps.subscribe("valid".into()).unwrap();
        assert_eq!(
            deps.attach("valid", id, attach_args("invalid"))
                .unwrap_err()
                .code,
            connectrpc::ErrorCode::InvalidArgument
        );
        assert_eq!(hub.owner_stream_count(), 0);
        drop(subscription);
    }
    for id in ["x".repeat(128), uuid::Uuid::new_v4().to_string()] {
        let mut subscription = deps.subscribe(id.clone()).unwrap();
        subscription.next().await.unwrap().unwrap();
        deps.attach(&id, id.clone(), attach_args("valid")).unwrap();
        let event = subscription.next().await.unwrap().unwrap();
        let event: wire::TerminalSubscriptionEvent =
            super::super::protocol::connect::to_wire(&event).unwrap();
        assert_eq!(event.stream_id, id);
        assert!(matches!(
            event.event,
            Some(wire::terminal_subscription_event::Event::Item(_))
        ));
    }
}
