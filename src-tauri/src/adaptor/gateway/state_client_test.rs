use super::*;
#[test]
fn test_購読変換_一覧と印を変換し不正な対象と欠損を拒否する() {
    // Given
    let event = wire::StateSubscriptionEvent {
        target: "repository-paths".into(),
        version: Some(wire::StateVersion {
            epoch: "boot".into(),
            sequence: 1,
        }),
        event: Some(wire::state_subscription_event::Event::Snapshot(
            wire::StatePayload {
                value: Some(wire::state_payload::Value::RepositoryPaths(
                    wire::Liststring {
                        items: vec!["/repo".into()],
                    },
                )),
            },
        )),
    };
    // When / Then
    let value = decode(event.clone()).unwrap();
    assert!(value.snapshot);
    assert_eq!(
        value.value,
        Some(StateValue::RepositoryPaths(vec!["/repo".into()]))
    );
    assert_eq!(value.target, "repository-paths");
    assert!(decode(wire::StateSubscriptionEvent {
        target: String::new(),
        ..event.clone()
    })
    .is_err());
    assert_eq!(
        decode(wire::StateSubscriptionEvent {
            target: "second-target".into(),
            ..event.clone()
        })
        .unwrap()
        .target,
        "second-target"
    );
    assert!(decode(wire::StateSubscriptionEvent {
        version: None,
        ..event.clone()
    })
    .is_err());
    let bookmark = decode(wire::StateSubscriptionEvent {
        event: Some(wire::state_subscription_event::Event::Bookmark(
            wire::Unit {},
        )),
        ..event.clone()
    })
    .unwrap();
    assert!(bookmark.value.is_none());
    assert!(decode(wire::StateSubscriptionEvent {
        event: Some(wire::state_subscription_event::Event::Change(
            wire::StateChange {
                delta: false,
                payload: None
            }
        )),
        ..event
    })
    .is_err());
}

#[tokio::test]
async fn test_購読通信_実streamの接続で開始停止再開とエラーを受信する() {
    use crate::adaptor::{
        controller::{
            api::client::{router, ClientApiDeps},
            client::ClientCommandDispatch,
        },
        gateway::{
            push::ClientPushGateway, repository::notify::RepoPathsNotifyGateway,
            subscription_timer::TokioSubscriptionTimer,
        },
    };
    use crate::domain::repository::RepoPathsNotifier;
    use crate::infrastructure::push::PushSink;
    use crate::usecase::{
        application_startup::ApplicationStartupAuthority,
        client_connection::{ClientConnectionDto, ClientConnectionError},
        state_subscription::{StateSubscriptionUsecase, REPO_PATHS},
    };
    struct Endpoint(String);
    #[async_trait::async_trait]
    impl ClientConnectionQueryService for Endpoint {
        fn read(&self) -> Result<ClientConnectionDto, ClientConnectionError> {
            Ok(ClientConnectionDto {
                url: self.0.clone(),
                token: "client".into(),
            })
        }
        async fn desktop_settings(
            &self,
        ) -> Result<
            crate::usecase::app_config::query_service::DesktopSettingsDto,
            ClientConnectionError,
        > {
            unreachable!()
        }
    }
    // Given
    let subscriptions =
        StateSubscriptionUsecase::new(vec!["/repo".into()], Arc::new(TokioSubscriptionTimer));
    let notifier = RepoPathsNotifyGateway::new(subscriptions.publisher());
    let dispatch = ClientCommandDispatch::new(
        Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        Arc::new(ApplicationStartupAuthority::ready()),
    );
    let deps = ClientApiDeps::new(
        Arc::new(dispatch),
        ClientPushGateway::new(Arc::new(PushSink::new())),
        crate::client_api_acceptance::watcher(),
    )
    .with_state_subscriptions(subscriptions);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gateway = StateClientGatewayImpl(Arc::new(Endpoint(format!(
        "http://{}",
        listener.local_addr().unwrap()
    ))));
    let server = tokio::spawn(async move {
        axum::serve(listener, router(Some(deps))).await.unwrap();
    });
    // When / Then
    let mut connection = gateway.connect().await.unwrap();
    assert!(connection.start("missing", None).await.is_err());
    connection.start(REPO_PATHS, None).await.unwrap();
    let initial = connection.receive().await.unwrap();
    assert!(initial.snapshot);
    assert_eq!(
        initial.value,
        Some(StateValue::RepositoryPaths(vec!["/repo".into()]))
    );
    let bookmark = connection.receive().await.unwrap();
    assert_eq!(bookmark.version, initial.version);
    assert!(bookmark.value.is_none());
    notifier.notify_changed(vec!["/next".into()]);
    let changed = loop {
        let event = connection.receive().await.unwrap();
        if event.value.is_some() {
            break event;
        }
    };
    assert!(!changed.snapshot);
    assert_eq!(changed.version.sequence, initial.version.sequence + 1);
    assert_eq!(
        changed.value,
        Some(StateValue::RepositoryPaths(vec!["/next".into()]))
    );
    connection.stop(REPO_PATHS).await.unwrap();
    notifier.notify_changed(vec![]);
    // Drain any bookmark already in transit before stop.
    while let Ok(event) =
        tokio::time::timeout(std::time::Duration::from_millis(50), connection.receive()).await
    {
        assert!(event.unwrap().value.is_none());
    }
    connection
        .start(REPO_PATHS, Some(&changed.version))
        .await
        .unwrap();
    let resumed = connection.receive().await.unwrap();
    assert!(!resumed.snapshot);
    assert_eq!(resumed.version.sequence, changed.version.sequence + 1);
    assert_eq!(resumed.value, Some(StateValue::RepositoryPaths(vec![])));
    drop(connection);
    let mut reconnected = gateway.connect().await.unwrap();
    reconnected
        .start(REPO_PATHS, Some(&initial.version))
        .await
        .unwrap();
    assert_eq!(
        reconnected.receive().await.unwrap().version,
        changed.version
    );
    drop(reconnected);
    server.abort();
}
