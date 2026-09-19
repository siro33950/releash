use super::*;
use crate::adaptor::gateway::client_handoff::ClientHandoffFiles;
use crate::domain::client_operation::handoff::ClientHandoffReference;
use wire::envelope::Body;

#[tokio::test(start_paused = true)]
async fn test_要求応答待ち_設定取得と保存の期限超過と切断を停止失敗と通知しない() {
    use wire::command_request::Command;
    for command in [
        Command::GetAppSettings(wire::GetAppSettingsRequest {}),
        Command::UpdateLoginItemPreference(wire::UpdateLoginItemPreferenceRequest {
            requested: Some(true),
        }),
        Command::RequestApplicationQuit(wire::RequestApplicationQuitRequest::default()),
    ] {
        for disconnected in [false, true] {
            // Given
            let (sender, mut outgoing) = mpsc::channel(1);
            let client = DesktopClient {
                sender,
                frames: broadcast::channel(1).0,
                hello: Default::default(),
                task: tokio::spawn(std::future::pending()),
                state: Default::default(),
                attachment: Default::default(),
                stop: None,
            };
            // When
            let (result, ()) = tokio::join!(
                client.request(wire::CommandRequest {
                    request_id: "request".into(),
                    command: Some(command.clone()),
                    ..Default::default()
                }),
                async {
                    let Outbound::Frame { reply, sent, .. } = outgoing.recv().await.unwrap() else {
                        panic!("expected request frame");
                    };
                    sent.send(Ok(())).unwrap();
                    if !disconnected {
                        tokio::time::sleep(std::time::Duration::from_secs(36)).await;
                    }
                    drop(reply);
                }
            );
            client.task.abort();
            // Then
            assert_eq!(
                result.unwrap_err(),
                if disconnected {
                    "Daemon request outcome is unknown (response channel closed)."
                } else {
                    "Daemon request outcome is unknown (response deadline exceeded)."
                }
            );
        }
    }
}

type Peer = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;
async fn fixture() -> (
    DesktopClient,
    Peer,
    Arc<ClientHandoffUsecase>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let files = Arc::new(ClientHandoffFiles::new(dir.path().into()));
    let handoff = Arc::new(ClientHandoffUsecase::new(files.clone(), files));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap());
    let (client, peer) = tokio::join!(tokio_tungstenite::connect_async(url), async {
        tokio_tungstenite::accept_async(listener.accept().await.unwrap().0)
            .await
            .unwrap()
    });
    let client = DesktopClient::start(
        client.unwrap().0,
        wire::ClientHello {
            instance_id: "current".into(),
            ..Default::default()
        },
        handoff.clone(),
    );
    (client, peer, handoff, dir)
}
async fn receive(peer: &mut Peer) -> wire::Envelope {
    loop {
        let Frame::Binary(bytes) =
            tokio::time::timeout(std::time::Duration::from_secs(2), peer.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
        else {
            continue;
        };
        let envelope = wire::Envelope::decode(bytes).unwrap();
        if matches!(envelope.body, Some(Body::Heartbeat(_))) {
            peer.send(Frame::Binary(envelope.encode_to_vec().into()))
                .await
                .unwrap();
        } else {
            return envelope;
        }
    }
}
async fn respond(peer: &mut Peer, request_id: &str) {
    peer.send(Frame::Binary(
        wire::Envelope {
            body: Some(Body::Response(wire::CommandResponse {
                request_id: request_id.into(),
                outcome: Some(wire::command_response::Outcome::Result(Default::default())),
                ..Default::default()
            })),
        }
        .encode_to_vec()
        .into(),
    ))
    .await
    .unwrap();
}
async fn restore(client: &DesktopClient, peer: &mut Peer) {
    let (_frames, _cancelled) = client.attach("restoration".into());
    for (id, command) in [
        (
            "repos",
            wire::command_request::Command::GetRepoPaths(wire::GetRepoPathsRequest {}),
        ),
        (
            "settings",
            wire::command_request::Command::GetPerformanceTelemetryEnabled(
                wire::GetPerformanceTelemetryEnabledRequest {},
            ),
        ),
    ] {
        let (response, ()) = tokio::join!(
            client.request(wire::CommandRequest {
                request_id: id.into(),
                command: Some(command),
                ..Default::default()
            }),
            async {
                let Some(Body::Request(request)) = receive(peer).await.body else {
                    panic!("restoration read");
                };
                respond(peer, &request.request_id).await;
            }
        );
        response.unwrap();
        assert!(matches!(
            receive(peer).await.body,
            Some(Body::RequestAck(_))
        ));
    }
    assert!(
        !client.restored(),
        "received replies do not prove the renderer applied state"
    );
    assert_eq!(
        client
            .forward(mutation("before-render"))
            .await
            .unwrap_err()
            .state,
        crate::domain::client_operation::transmission::TransmissionFailure::NotSent
    );
    client.finish_restoration("restoration").await.unwrap();
    assert!(client.restored());
}
fn mutation(id: &str) -> wire::Envelope {
    wire::Envelope {
        body: Some(Body::Request(Box::new(wire::CommandRequest {
            request_id: id.into(),
            instance_id: "current".into(),
            command: Some(wire::command_request::Command::AddRepoPath(
                wire::AddRepoPathRequest {
                    path: Some("/repo".into()),
                },
            )),
            ..Default::default()
        }))),
    }
}

#[tokio::test]
async fn test_単一接続_状態再取得後に通常要求と停止要求を同じ接続で追跡する() {
    // Given
    let (client, mut peer, handoff, _dir) = fixture().await;
    assert_eq!(
        client.forward(mutation("early")).await.unwrap_err().state,
        crate::domain::client_operation::transmission::TransmissionFailure::NotSent
    );
    assert!(handoff.list().unwrap().is_empty());
    restore(&client, &mut peer).await;
    let (mut frames, _cancel) = client.attach("renderer".into());
    // When
    client.forward(mutation("edit")).await.unwrap();
    let Some(Body::Request(request)) = receive(&mut peer).await.body else {
        panic!("normal request")
    };
    assert_eq!(request.request_id, "edit");
    assert_eq!(handoff.list().unwrap()[0].id, "edit");
    respond(&mut peer, "edit").await;
    let bytes = frames.recv().await.unwrap().unwrap();
    assert!(matches!(
        wire::Envelope::decode(bytes.as_slice()).unwrap().body,
        Some(Body::Response(_))
    ));
    assert_eq!(
        handoff.list().unwrap()[0].id,
        "edit",
        "renderer must acknowledge the delivered result"
    );
    client
        .forward(wire::Envelope {
            body: Some(Body::RequestAck(wire::RequestAck {
                request_id: "edit".into(),
                ..Default::default()
            })),
        })
        .await
        .unwrap();
    assert!(matches!(
        receive(&mut peer).await.body,
        Some(Body::RequestAck(_))
    ));
    let shutdown = wire::CommandRequest {
        request_id: "quit".into(),
        command: Some(wire::command_request::Command::RequestApplicationQuit(
            wire::RequestApplicationQuitRequest {
                request: Some(wire::ApplicationQuitRequestDtoV1 {
                    request_id: Some("quit".into()),
                    intent: Some(wire::ApplicationQuitIntentDtoV1 {
                        variant: Some(wire::application_quit_intent_dto_v1::Variant::Exit(
                            wire::ApplicationQuitIntentDtoV1Exit { code: Some(0) },
                        )),
                    }),
                }),
            },
        )),
        ..Default::default()
    };
    let (result, ()) = tokio::join!(client.request(shutdown), async {
        let Some(Body::Request(request)) = receive(&mut peer).await.body else {
            panic!("shutdown request")
        };
        assert_eq!(request.request_id, "quit");
        respond(&mut peer, "quit").await;
    });
    // Then
    assert!(result.is_ok());
    assert!(handoff.list().unwrap().is_empty());
    assert!(
        frames.try_recv().is_err(),
        "shell response must not be delivered as a renderer response"
    );
}

#[tokio::test]
async fn test_送信境界_期限切れはmarkerを残さず既存の結果不明は利用者確認まで残す() {
    // Given
    let (client, mut peer, handoff, _dir) = fixture().await;
    handoff
        .remember(ClientHandoffReference {
            id: "old".into(),
            command: "add_repo_path".into(),
            fingerprint: vec![1; 32],
            ordering_target: vec![2; 32],
        })
        .unwrap();
    restore(&client, &mut peer).await;
    let (mut frames, _cancel) = client.attach("renderer".into());
    let mut expired = mutation("expired");
    if let Some(Body::Request(request)) = &mut expired.body {
        request.deadline_unix_ms = 1;
    }
    // When
    assert_eq!(
        client.forward(expired).await.unwrap_err().state,
        crate::domain::client_operation::transmission::TransmissionFailure::NotSent
    );
    let Some(Body::Request(mut old)) = mutation("old").body else {
        unreachable!()
    };
    old.instance_id = "previous".into();
    client
        .forward(wire::Envelope {
            body: Some(Body::OperationQuery(wire::OperationQuery {
                request_id: "old".into(),
                instance_id: "previous".into(),
                sent: true,
                request: Some(*old),
                ..Default::default()
            })),
        })
        .await
        .unwrap();
    assert!(matches!(
        receive(&mut peer).await.body,
        Some(Body::OperationQuery(_))
    ));
    peer.send(Frame::Binary(
        wire::Envelope {
            body: Some(Body::OperationStatus(wire::OperationStatus {
                request_id: "old".into(),
                state: "unknown".into(),
                ..Default::default()
            })),
        }
        .encode_to_vec()
        .into(),
    ))
    .await
    .unwrap();
    let bytes = frames.recv().await.unwrap().unwrap();
    // Then
    let Some(Body::OperationStatus(status)) =
        wire::Envelope::decode(bytes.as_slice()).unwrap().body
    else {
        panic!("operation status")
    };
    assert_eq!(status.state, "restored_unknown");
    assert_eq!(
        handoff
            .list()
            .unwrap()
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["old"]
    );
    handoff.forget("old").unwrap();
    assert!(handoff.list().unwrap().is_empty());
    assert!(
        client.restored(),
        "rejected unsent command does not close transport"
    );
}

#[tokio::test]
async fn test_接続復旧_再取得前と古い表示surfaceからの反映完了を拒否する() {
    // Given
    let (client, _peer, handoff, _dir) = fixture().await;
    let (_frames, _cancelled) = client.attach("current".into());
    // When / Then
    assert!(client
        .finish_restoration("old")
        .await
        .unwrap_err()
        .contains("attachment changed"));
    assert!(client
        .finish_restoration("current")
        .await
        .unwrap_err()
        .contains("not been restored"));
    assert!(!client.restored());
    assert_eq!(
        client.forward(mutation("blocked")).await.unwrap_err().state,
        crate::domain::client_operation::transmission::TransmissionFailure::NotSent
    );
    assert!(handoff.list().unwrap().is_empty());
}

#[tokio::test]
async fn test_表示購読_再接続と破棄で古いchannelを解放する() {
    // Given
    let (client, mut peer, _, _dir) = fixture().await;
    restore(&client, &mut peer).await;
    let (_, first) = client.attach("first".into());
    // When
    let (_, second) = client.attach("second".into());
    // Then
    assert!(first.await.is_err());
    client.detach("first");
    client.detach("second");
    assert!(second.await.is_err());
    assert!(client.connected());
}
