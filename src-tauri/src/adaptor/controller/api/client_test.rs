use super::*;
use crate::adaptor::controller::api::test_helpers::*;
use prost::Message;

#[test]
fn test_拒否_構造化エラーで段と理由を返す() {
    // Given
    let rejection = crate::common::concurrency::Rejection {
        level: "default",
        reason: crate::common::concurrency::RejectReason::TimedOut,
    };
    // When
    let error = crate::adaptor::presenter::connect::request_rejected(&rejection);
    // Then
    assert_eq!(error.code, connectrpc::ErrorCode::ResourceExhausted);
    assert_eq!(error.details.len(), 1);
    assert_eq!(error.details[0].type_url, "releash.client.v1.CommandError");
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(error.details[0].value.as_ref().unwrap())
        .unwrap();
    let detail = wire::CommandError::decode(bytes.as_slice()).unwrap();
    let Some(wire::command_error::Variant::Coded(detail)) = detail.variant else {
        panic!("coded error");
    };
    assert_eq!(detail.code.as_deref(), Some("CLIENT_REQUEST_LIMIT"));
    assert_eq!(
        detail.message.as_deref(),
        Some("default requests rejected: time-out")
    );
    assert_eq!(
        error.message.as_deref(),
        Some("default requests rejected: time-out")
    );
}

#[test]
fn test_状態購読配線_usecaseとcontrollerが同じ出力実装を参照する() {
    // Given
    let presenter =
        Arc::new(crate::adaptor::presenter::state_subscription::StateSubscriptionPresenter::new());
    let usecase = crate::usecase::state_subscription::StateSubscriptionUsecase::new_with_output(
        presenter.clone(),
        crate::test_support::state_subscription::read_driver(),
    );
    let deps = crate::test_support::state_subscription::deps(usecase, presenter);
    // When
    let output: Arc<dyn crate::usecase::state_subscription::StateSubscriptionOutput> =
        deps.presenter.clone();
    // Then
    assert!(Arc::ptr_eq(&deps.usecase.publisher(), &output));
}

#[tokio::test]
async fn test_状態stream開始_各段の失敗で既存clientを保持し先に開いたclientだけ戻す() {
    use crate::usecase::state_subscription::SubscriptionError;
    // Given
    for stage in 0..3 {
        let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
            vec![],
            crate::test_support::state_subscription::read_driver(),
        );
        let deps = subscriptions.deps();
        match stage {
            0 => deps.usecase.open_client("client".into()).unwrap(),
            1 => deps.terminal.open_client("client".into()).unwrap(),
            _ => deps.presenter.open("client".into()).unwrap(),
        }
        // When
        assert!(matches!(
            deps.open_stream("client".into()),
            Err(SubscriptionError::AlreadyExists)
        ));
        // Then
        if stage == 0 {
            assert_eq!(
                deps.usecase.open_client("client".into()),
                Err(SubscriptionError::AlreadyExists)
            );
        } else {
            deps.usecase.open_client("client".into()).unwrap();
            deps.usecase.close_client("client");
        }
        if stage == 1 {
            assert_eq!(
                deps.terminal.open_client("client".into()),
                Err(SubscriptionError::AlreadyExists)
            );
        } else {
            deps.terminal.open_client("client".into()).unwrap();
            deps.terminal.close_client("client");
        }
        if stage == 2 {
            assert_eq!(
                deps.presenter.open("client".into()),
                Err(SubscriptionError::AlreadyExists)
            );
        } else {
            deps.presenter.open("client".into()).unwrap();
        }
    }
}

mod restored_memory_tests {
    use super::*;

    fn unary_request(method: &str, body: &str) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::post(format!("/releash.client.v1.ClientService/{method}"))
            .header("content-type", "application/json")
            .header("connect-protocol-version", "1")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    async fn assert_request_deadline(timeout: Option<&str>, seconds: u64) {
        use axum::{
            body::{to_bytes, Body},
            http::Request,
        };
        use tower::ServiceExt;
        // Given
        let stopped = tokio_util::sync::CancellationToken::new();
        let signal = stopped.clone();
        let mut dispatch = dispatch();
        dispatch.register_domain(
            &["update_external_editor"],
            Box::new(move |_| {
                let guard = signal.clone().drop_guard();
                Box::pin(async move {
                    let _guard = guard;
                    std::future::pending().await
                })
            }),
        );
        let deps = crate::test_support::client_api_deps(Arc::new(dispatch));
        let mut request = Request::post("/releash.client.v1.ClientService/UpdateExternalEditor")
            .header("content-type", "application/json")
            .header("connect-protocol-version", "1");
        if let Some(timeout) = timeout {
            request = request.header("connect-timeout-ms", timeout);
        }
        let call = router(
            Some(deps.clone()),
            crate::adaptor::controller::daemon::default_timeout(),
        )
        .oneshot(request.body(Body::from("{}")).unwrap());
        tokio::pin!(call);
        assert!(futures_util::poll!(&mut call).is_pending());
        tokio::task::yield_now().await;
        assert_eq!(deps.priority_limits().available("default"), 40);
        // When
        tokio::time::advance(std::time::Duration::from_secs(seconds - 1)).await;
        assert!(futures_util::poll!(&mut call).is_pending());
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        let response = call.await.unwrap();
        // Then
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "deadline_exceeded");
        stopped.cancelled().await;
        assert_eq!(deps.priority_limits().available("default"), 41);
    }

    async fn assert_cancelled_blocking_mutation(deadline: bool, repository: bool) {
        use crate::usecase::repository_usecase::WorktreeExecutionArchiver;
        use axum::{
            body::{to_bytes, Body},
            http::Request,
        };
        use tower::ServiceExt;
        // Given
        let runtime = Arc::new(crate::usecase::workflow::WorkflowRuntimeUsecase::new(
        Arc::new(crate::usecase::workflow::runtime_command::runtime_command_tests::tests::FakeRuntimeGateway::default()),
        Arc::new(crate::usecase::workflow::NoopArchiveRepository),
    ));
        let started = Arc::new(tokio::sync::Notify::new());
        let signal = started.clone();
        let (finish, receiver) = std::sync::mpsc::channel();
        let receiver = Arc::new(std::sync::Mutex::new(receiver));
        let stopped = tokio_util::sync::CancellationToken::new();
        let stop_signal = stopped.clone();
        let mut dispatch = dispatch().with_worktree_mutations(runtime.clone());
        dispatch.register_domain(
            &["git_stage"],
            Box::new(move |_| {
                let signal = signal.clone();
                let receiver = receiver.clone();
                let guard = stop_signal.clone().drop_guard();
                Box::pin(async move {
                    let _guard = guard;
                    let operation = move || {
                        signal.notify_one();
                        receiver.lock().unwrap().recv().unwrap();
                    };
                    let result = if repository {
                        crate::adaptor::controller::client::repository::run_blocking(move || {
                            operation();
                            Ok(())
                        })
                        .await
                    } else {
                        crate::adaptor::controller::client::code::run_blocking(move || {
                            operation();
                            Ok(())
                        })
                        .await
                    };
                    result.map_err(wire::CommandFailure::from)?;
                    Ok(wire::command_result::Command::GitStage(wire::Unit {}))
                })
            }),
        );
        let deps = crate::test_support::client_api_deps(Arc::new(dispatch));
        let request = Request::post("/releash.client.v1.ClientService/GitStage")
            .header("content-type", "application/json")
            .header("connect-protocol-version", "1")
            .header("connect-timeout-ms", "1000")
            .body(Body::from(r#"{"repoPath":"/repo","paths":[]}"#))
            .unwrap();
        let mut call = Box::pin(
            router(
                Some(deps.clone()),
                crate::adaptor::controller::daemon::default_timeout(),
            )
            .oneshot(request),
        );
        assert!(futures_util::poll!(&mut call).is_pending());
        started.notified().await;
        let mut deletion = Box::pin(runtime.begin_worktree_deletion("/repo"));
        assert!(futures_util::poll!(&mut deletion).is_pending());
        // When
        if deadline {
            let response = call.await.unwrap();
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(error["code"], "deadline_exceeded");
        } else {
            drop(call);
        }
        stopped.cancelled().await;
        // Then
        assert_eq!(deps.priority_limits().available("default"), 41);
        assert!(futures_util::poll!(&mut deletion).is_pending());
        assert!(runtime.begin_worktree_mutation("/repo").is_err());
        finish.send(()).unwrap();
        let guard = deletion.await.unwrap();
        assert!(runtime.begin_worktree_mutation("/repo").is_err());
        drop(guard);
        assert!(runtime.begin_worktree_mutation("/repo").is_ok());
    }

    async fn run_command(
        cancellation: &tokio_util::sync::CancellationToken,
        future: impl std::future::Future<
            Output = Result<wire::command_result::Command, wire::CommandFailure>,
        >,
    ) -> Result<wire::command_result::Command, connectrpc::ConnectError> {
        let context = crate::common::operation_context::OperationContext::new(
            None,
            Arc::new(cancellation.clone()),
        );
        crate::common::operation_context::wait(&context, future)
            .await
            .map_err(|error| {
                crate::adaptor::presenter::connect::classified_error(
                    crate::adaptor::presenter::error::AppError::from_failure(
                        crate::domain::failure::TechnicalFailure::from(error),
                    ),
                )
            })?
            .map_err(command_error)
    }

    fn pending_editor_dispatch() -> (ClientCommandDispatch, Arc<tokio::sync::Notify>) {
        let release = Arc::new(tokio::sync::Notify::new());
        let signal = release.clone();
        let mut dispatch = dispatch();
        dispatch.register_domain(
            &["update_external_editor"],
            Box::new(move |_| {
                let signal = signal.clone();
                Box::pin(async move {
                    signal.notified().await;
                    Ok(wire::command_result::Command::UpdateExternalEditor(
                        wire::Unit {},
                    ))
                })
            }),
        );
        (dispatch, release)
    }

    fn notion_cancellation_fixture() -> (Router, StateSubscriptionDeps) {
        use crate::usecase::state_subscription::{
            StateReadError, StateSubscriptionRead, StateValue, SubscriptionTarget,
        };
        struct Reads;
        #[async_trait::async_trait]
        impl StateSubscriptionRead for Reads {
            async fn read(
                &self,
                target: &SubscriptionTarget,
            ) -> Result<StateValue, StateReadError> {
                match target {
                    SubscriptionTarget::NotionTasks(_) => {
                        Ok(StateValue::NotionTasks(crate::usecase::fetched::Fetched {
                            value: Some(crate::domain::notion::NotionTaskPage {
                                tasks: vec![],
                                has_more: false,
                                next_cursor: None,
                            }),
                            error: None,
                        }))
                    }
                    _ => std::future::pending().await,
                }
            }
            fn repositories(&self) -> Vec<String> {
                vec![]
            }
        }
        let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
            vec![],
            crate::test_support::state_subscription::read_driver(),
        )
        .with_reads(Arc::new(Reads), None, vec![], String::new())
        .deps();
        let app = router(
            Some(
                crate::test_support::client_api_deps(Arc::new(dispatch()))
                    .with_state_subscriptions(subscriptions.clone()),
            ),
            crate::adaptor::controller::daemon::default_timeout(),
        );
        (app, subscriptions)
    }

    const NOTION_REQUEST_A: &str = r#"{"subscriptionId":"notion-a","clientId":"client","target":"notion-tasks","args":["/repo","20","labels={\"Tags\":[\"a\"]}"]}"#;
    const NOTION_REQUEST_B: &str = r#"{"subscriptionId":"notion-b","clientId":"client","target":"notion-tasks","args":["/repo","20","labels={\"Tags\":[\"a\",\"a\"]}"]}"#;

    #[tokio::test]
    pub async fn test_サーバ情報取得_全段の枠が埋まっていても受理し枠を使わない() {
        use axum::{
            body::{to_bytes, Body},
            http::{Request, StatusCode},
        };
        use tower::ServiceExt;
        // Given
        let deps = crate::test_support::client_api_deps(Arc::new(dispatch()));
        let _permits =
            ["interactive", "workflow", "default"].map(|level| deps.priority_limits().fill(level));
        let router = router(
            Some(deps.clone()),
            crate::adaptor::controller::daemon::default_timeout(),
        );
        let request = || {
            Request::post("/releash.client.v1.ClientService/GetServerInfo")
                .header("content-type", "application/json")
                .header("connect-protocol-version", "1")
                .body(Body::from("{}"))
                .unwrap()
        };
        // When
        let response = router.clone().oneshot(request()).await.unwrap();
        // Then
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["release"], env!("CARGO_PKG_VERSION"));
        for level in ["interactive", "workflow", "default"] {
            assert_eq!(deps.priority_limits().available(level), 0);
        }
    }

    #[tokio::test(start_paused = true)]
    pub async fn test_単発rpc_期限なしは120秒で処理を止め枠を解放する() {
        assert_request_deadline(None, 120).await;
    }

    #[tokio::test(start_paused = true)]
    pub async fn test_単発rpc_clientの短い期限で処理を止め枠を解放する() {
        assert_request_deadline(Some("1000"), 1).await;
    }

    #[tokio::test(start_paused = true)]
    pub async fn test_単発rpc_clientの長い期限を短縮しない() {
        assert_request_deadline(Some("180000"), 180).await;
    }

    #[tokio::test]
    pub async fn test_単発rpc_呼び出し破棄でasync処理を止め枠を解放する() {
        use tower::ServiceExt;
        // Given
        let stopped = tokio_util::sync::CancellationToken::new();
        let signal = stopped.clone();
        let mut dispatch = dispatch();
        dispatch.register_domain(
            &["update_external_editor"],
            Box::new(move |_| {
                let guard = signal.clone().drop_guard();
                Box::pin(async move {
                    let _guard = guard;
                    std::future::pending().await
                })
            }),
        );
        let deps = crate::test_support::client_api_deps(Arc::new(dispatch));
        let mut call = Box::pin(
            router(
                Some(deps.clone()),
                crate::adaptor::controller::daemon::default_timeout(),
            )
            .oneshot(unary_request(
                "UpdateExternalEditor",
                r#"{"editor":"code"}"#,
            )),
        );
        assert!(futures_util::poll!(&mut call).is_pending());
        tokio::task::yield_now().await;
        assert_eq!(deps.priority_limits().available("default"), 40);
        // When
        drop(call);
        // Then
        assert_eq!(deps.priority_limits().available("default"), 41);
        tokio::time::timeout(std::time::Duration::from_secs(1), stopped.cancelled())
            .await
            .unwrap();
    }

    #[tokio::test]
    pub async fn test_単発rpc_取り消しはcancelledでpanicはinternalに分類する() {
        // Given
        let task = tokio::spawn(std::future::pending::<()>());
        // When
        task.abort();
        let error = task_error(task.await.unwrap_err());
        // Then
        assert_eq!(error.code, connectrpc::ErrorCode::Canceled);
        let task = tokio::spawn(async { panic!("test panic") });
        assert_eq!(
            task_error(task.await.unwrap_err()).code,
            connectrpc::ErrorCode::Internal
        );
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();
        let mut dispatch = dispatch();
        dispatch.register_domain(
            &["update_external_editor"],
            Box::new(|_| Box::pin(std::future::pending())),
        );
        let error = run_command(
            &token,
            dispatch.dispatch_admitted(wire::command_request::Command::UpdateExternalEditor(
                Default::default(),
            )),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, connectrpc::ErrorCode::Canceled);
    }

    #[tokio::test]
    pub async fn test_単発rpc_変更処理の取り消しでhandlerとworktreeの枠を解放する() {
        use crate::usecase::repository_usecase::WorktreeExecutionArchiver;
        // Given
        let runtime = Arc::new(crate::usecase::workflow::WorkflowRuntimeUsecase::new(
        Arc::new(crate::usecase::workflow::runtime_command::runtime_command_tests::tests::FakeRuntimeGateway::default()),
        Arc::new(crate::usecase::workflow::NoopArchiveRepository),
    ));
        let started = Arc::new(tokio::sync::Notify::new());
        let start_signal = started.clone();
        let stopped = tokio_util::sync::CancellationToken::new();
        let signal = stopped.clone();
        let mut dispatch = dispatch().with_worktree_mutations(runtime.clone());
        dispatch.register_domain(
            &["git_stage"],
            Box::new(move |_| {
                start_signal.notify_one();
                let guard = signal.clone().drop_guard();
                Box::pin(async move {
                    let _guard = guard;
                    std::future::pending().await
                })
            }),
        );
        let cancellation = tokio_util::sync::CancellationToken::new();
        let mut call = Box::pin(run_command(
            &cancellation,
            dispatch.dispatch_admitted(wire::command_request::Command::GitStage(
                wire::GitStageRequest {
                    repo_path: Some("/repo".into()),
                    ..Default::default()
                },
            )),
        ));
        tokio::select! {
            _ = started.notified() => {},
            result = &mut call => panic!("handler ended before cancellation: {result:?}"),
        }
        let deletion = runtime.begin_worktree_deletion("/repo");
        tokio::pin!(deletion);
        assert!(futures_util::poll!(&mut deletion).is_pending());
        // When
        cancellation.cancel();
        assert_eq!(
            call.await.unwrap_err().code,
            connectrpc::ErrorCode::Canceled
        );
        // Then
        assert!(stopped.is_cancelled());
        tokio::time::timeout(std::time::Duration::from_secs(1), deletion)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    pub async fn test_状態購読_既定期限後もbookmarkが届く() {
        use axum::{body::Body, http::Request};
        use futures_util::StreamExt;
        use tower::ServiceExt;
        // Given
        let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
            Vec::new(),
            crate::test_support::state_subscription::read_driver(),
        );
        let deps = crate::test_support::client_api_deps(Arc::new(dispatch()))
            .with_state_subscriptions(subscriptions.deps());
        let payload = br#"{"clientId":"deadline-test"}"#;
        let mut bytes = vec![0];
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(payload);
        let response = router(
            Some(deps),
            crate::adaptor::controller::daemon::default_timeout(),
        )
        .oneshot(
            Request::post("/releash.client.v1.ClientService/OpenStateStream")
                .header("content-type", "application/connect+json")
                .header("connect-protocol-version", "1")
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await
        .unwrap();
        assert!(response.status().is_success());
        let mut body = response.into_body().into_data_stream();
        assert!(body.next().await.unwrap().is_ok());
        subscriptions
            .deps()
            .start_subscription(
                "deadline-test",
                &crate::usecase::state_subscription::SubscriptionTarget::parse("repository-paths")
                    .unwrap(),
                &format!("{}:{}", "deadline-test", "repository-paths"),
                None,
            )
            .await
            .unwrap();
        assert!(body.next().await.unwrap().is_ok());
        // When / Then
        for _ in 0..13 {
            tokio::time::advance(std::time::Duration::from_secs(10)).await;
            let frame = body.next().await.unwrap().unwrap();
            assert_eq!(frame[0], 0);
            assert!(std::str::from_utf8(&frame[5..])
                .unwrap()
                .contains("bookmark"));
        }
    }

    #[tokio::test]
    pub async fn test_状態購読操作_上限時は拒否し枠解放後は受理する() {
        use axum::{
            body::Body,
            http::{Request, StatusCode},
        };
        use tower::ServiceExt;
        // Given
        let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
            Vec::new(),
            crate::test_support::state_subscription::read_driver(),
        );
        let _stream = subscriptions.open("limited".into()).unwrap();
        let deps = crate::test_support::client_api_deps(Arc::new(dispatch()))
            .with_state_subscriptions(subscriptions.deps());
        let router = router(
            Some(deps.clone()),
            crate::adaptor::controller::daemon::default_timeout(),
        );
        for method in ["StartStateSubscription", "StopStateSubscription"] {
            let permits = deps.priority_limits().fill("interactive");
            let request = || {
                Request::post(format!("/releash.client.v1.ClientService/{method}"))
                .header("content-type", "application/json")
                .header("connect-protocol-version", "1")
                .body(Body::from(
                    r#"{"clientId":"limited","target":"repository-paths","subscriptionId":"limited-subscription"}"#,
                ))
                .unwrap()
            };
            // When / Then
            assert_eq!(
                router.clone().oneshot(request()).await.unwrap().status(),
                StatusCode::TOO_MANY_REQUESTS
            );
            drop(permits);
            assert_eq!(
                router.clone().oneshot(request()).await.unwrap().status(),
                StatusCode::OK
            );
            assert_eq!(deps.priority_limits().available("interactive"), 11);
        }
    }

    #[tokio::test]
    pub async fn test_変更rpc_中断後も同期処理の完了まで削除と変更を拒否する() {
        for repository in [false, true] {
            assert_cancelled_blocking_mutation(false, repository).await;
        }
    }

    #[tokio::test]
    pub async fn test_変更rpc_期限切れ後も同期処理の完了まで削除と変更を拒否する() {
        for repository in [false, true] {
            assert_cancelled_blocking_mutation(true, repository).await;
        }
    }

    #[tokio::test]
    pub async fn test_単発rpc_期限と呼出破棄が同期処理の内側まで届く() {
        use crate::common::operation_context::OperationStopped;
        use std::time::{Duration, Instant};
        for expire in [false, true] {
            // Given
            let (started, mut ready) = tokio::sync::mpsc::unbounded_channel();
            let (stopped, mut stopped_rx) = tokio::sync::mpsc::unbounded_channel();
            let mut dispatch = dispatch();
            dispatch.register_domain(
                &["update_external_editor"],
                Box::new(move |_| {
                    let started = started.clone();
                    let stopped = stopped.clone();
                    Box::pin(async move {
                        crate::common::operation_context::spawn_blocking(move || {
                            started.send(()).unwrap();
                            let error = crate::common::operation_context::sleep(
                                &crate::common::operation_context::current(),
                                Duration::from_secs(30),
                            )
                            .unwrap_err();
                            stopped.send(error).unwrap();
                            Err(crate::adaptor::presenter::error::AppError::from_failure(
                                crate::domain::failure::TechnicalFailure::from(error),
                            )
                            .into())
                        })
                        .await
                        .unwrap()
                    })
                }),
            );
            let deps = crate::test_support::client_api_deps(Arc::new(dispatch));
            let mut call = Box::pin(deps.execute(
                expire.then(|| Instant::now() + Duration::from_millis(100)),
                wire::command_request::Command::UpdateExternalEditor(Default::default()),
            ));
            // When
            tokio::select! { _ = ready.recv() => {}, result = &mut call => panic!("call ended before starting: {result:?}") }
            if expire {
                assert_eq!(
                    call.await.unwrap_err().code,
                    connectrpc::ErrorCode::DeadlineExceeded
                );
            } else {
                drop(call);
            }
            // Then
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), stopped_rx.recv())
                    .await
                    .unwrap()
                    .unwrap(),
                if expire {
                    OperationStopped::Expired
                } else {
                    OperationStopped::Cancelled
                }
            );
        }
    }

    #[tokio::test]
    pub async fn test_共通入口_期限切れを変換し成功と内部失敗を保持する() {
        // Given / When
        let expired = ingress(Some(std::time::Instant::now()), async {
            Ok::<(), connectrpc::ConnectError>(())
        })
        .await
        .unwrap_err();
        // Then
        assert_eq!(expired.code, connectrpc::ErrorCode::DeadlineExceeded);
        assert_eq!(
            ingress(None, async { Ok::<_, connectrpc::ConnectError>(42) })
                .await
                .unwrap(),
            42
        );
        let error = ingress(None, async {
            Err::<(), _>(crate::adaptor::presenter::connect::classified_error(
                crate::adaptor::presenter::error::AppError::new("unavailable")
                    .with_status(connectrpc::ErrorCode::Unavailable),
            ))
        })
        .await
        .unwrap_err();
        assert_eq!(error.code, connectrpc::ErrorCode::Unavailable);
    }

    #[tokio::test]
    pub async fn test_流量制御_全段の枠が埋まっていても_report_terminal_processedを受理する() {
        use axum::http::StatusCode;
        use tower::ServiceExt;
        // Given
        let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
            Vec::new(),
            crate::test_support::state_subscription::read_driver(),
        );
        let _stream = subscriptions.open("limited".into()).unwrap();
        let presenter = subscriptions.test_presenter().unwrap().clone();
        let units = crate::adaptor::presenter::terminal_subscription::TerminalSubscriptionPresenter::report_units();
        let deps =
            crate::test_support::client_api_deps(Arc::new(dispatch())).with_state_subscriptions(
                crate::test_support::state_subscription::deps(subscriptions, Arc::new(presenter)),
            );
        let _permits =
            ["interactive", "workflow", "default"].map(|level| deps.priority_limits().fill(level));
        // When
        let response = router(
            Some(deps),
            crate::adaptor::controller::daemon::default_timeout(),
        )
        .oneshot(unary_request(
            "ReportTerminalProcessed",
            &format!(r#"{{"subscriptionId":"session","units":{units}}}"#),
        ))
        .await
        .unwrap();
        // Then
        assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    pub async fn test_優先度_defaultが埋まっていてもinteractiveの呼び出しを受理する() {
        use axum::http::StatusCode;
        use tower::ServiceExt;
        // Given
        let subscriptions = crate::usecase::state_subscription::StateSubscriptionUsecase::new(
            Vec::new(),
            crate::test_support::state_subscription::read_driver(),
        );
        let _stream = subscriptions.open("limited".into()).unwrap();
        let deps = crate::test_support::client_api_deps(Arc::new(dispatch()))
            .with_state_subscriptions(subscriptions.deps());
        let _permits = deps.priority_limits().fill("default");
        let router = router(
            Some(deps.clone()),
            crate::adaptor::controller::daemon::default_timeout(),
        );
        // When
        let rejected = router
            .clone()
            .oneshot(unary_request(
                "UpdateExternalEditor",
                r#"{"editor":"code"}"#,
            ))
            .await
            .unwrap();
        let accepted = router
        .oneshot(unary_request(
            "StartStateSubscription",
            r#"{"clientId":"limited","target":"repository-paths","subscriptionId":"limited-subscription"}"#,
        ))
        .await
        .unwrap();
        // Then
        assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(accepted.status(), StatusCode::OK);
        assert_eq!(deps.priority_limits().available("interactive"), 11);
    }

    #[tokio::test]
    pub async fn test_拒否_待ち行列が溢れても読まれない失敗は記録しない() {
        use axum::http::StatusCode;
        use tower::ServiceExt;
        // Given
        let deps = crate::test_support::client_api_deps(Arc::new(dispatch()));
        let permits = deps.priority_limits().fill("default");
        // When
        let router = router(
            Some(deps),
            crate::adaptor::controller::daemon::default_timeout(),
        );
        let response = router
            .clone()
            .oneshot(unary_request(
                "UpdateExternalEditor",
                r#"{"editor":"code"}"#,
            ))
            .await
            .unwrap();
        // Then
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        drop(permits);
        let accepted = router
            .oneshot(unary_request(
                "UpdateExternalEditor",
                r#"{"editor":"code"}"#,
            ))
            .await
            .unwrap();
        assert_ne!(accepted.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    pub async fn test_拒否_枠の対象外の呼び出しでも読まれない失敗は記録しない() {
        use axum::http::StatusCode;
        use tower::ServiceExt;
        // Given
        let deps = crate::test_support::client_api_deps(Arc::new(dispatch()));
        let permits = deps.priority_limits().fill("default");
        let router = router(
            Some(deps),
            crate::adaptor::controller::daemon::default_timeout(),
        );
        assert_eq!(
            router
                .clone()
                .oneshot(unary_request(
                    "UpdateExternalEditor",
                    r#"{"editor":"code"}"#,
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );

        // When / Then
        for method in ["GetServerInfo", "ReportTerminalProcessed"] {
            let response = router
                .clone()
                .oneshot(unary_request(method, "{}"))
                .await
                .unwrap();
            assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        }
        drop(permits);
        let response = router
            .oneshot(unary_request(
                "UpdateExternalEditor",
                r#"{"editor":"code"}"#,
            ))
            .await
            .unwrap();
        assert_ne!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    pub async fn test_待ち行列_席が空くまで待ってから受理する() {
        use axum::http::StatusCode;
        use tower::ServiceExt;
        // Given
        let (dispatch, release) = pending_editor_dispatch();
        let deps = crate::test_support::client_api_deps(Arc::new(dispatch));
        let seats = deps
            .priority_limits()
            .seats("default")
            .try_acquire_many_owned(41)
            .unwrap();
        let mut call = Box::pin(
            router(
                Some(deps.clone()),
                crate::adaptor::controller::daemon::default_timeout(),
            )
            .oneshot(unary_request(
                "UpdateExternalEditor",
                r#"{"editor":"code"}"#,
            )),
        );
        assert!(futures_util::poll!(&mut call).is_pending());
        tokio::task::yield_now().await;
        assert_eq!(deps.priority_limits().queue_length("default"), 49);
        // When
        drop(seats);
        release.notify_one();
        let response = call.await.unwrap();
        // Then
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(deps.priority_limits().queue_length("default"), 50);
        assert_eq!(deps.priority_limits().available("default"), 41);
    }

    #[tokio::test]
    pub async fn test_notion購読_初回開始の中断で新規要求を残さない() {
        use crate::usecase::state_subscription::SubscriptionTarget;
        use tower::ServiceExt;
        // Given
        let (app, subscriptions) = notion_cancellation_fixture();
        let _stream = subscriptions.open_stream("client".into()).unwrap();
        let blocked = SubscriptionTarget::BranchBase("/blocked".into(), "main".into());
        let mut blocker =
            Box::pin(subscriptions.start_subscription("client", &blocked, "blocked", None));
        assert!(futures_util::poll!(&mut blocker).is_pending());
        let mut call =
            Box::pin(app.oneshot(unary_request("StartStateSubscription", NOTION_REQUEST_A)));
        // When
        assert!(futures_util::poll!(&mut call).is_pending());
        drop(call);
        drop(blocker);
        let registration = subscriptions
            .test_presenter()
            .delivery("notion-a")
            .map(|(client, target, _)| (client, target));
        // Then
        assert!(registration.is_none());
        assert_eq!(subscriptions.test_usecase().test_worker_count(), 0);
    }

    #[tokio::test(start_paused = true)]
    pub async fn test_notion購読_別名開始の期限切れ後に既存要求を停止するとworkerを解放する() {
        use crate::usecase::state_subscription::SubscriptionTarget;
        use tower::ServiceExt;
        // Given
        let (app, subscriptions) = notion_cancellation_fixture();
        let _stream = subscriptions.open_stream("client".into()).unwrap();
        app.clone()
            .oneshot(unary_request("StartStateSubscription", NOTION_REQUEST_A))
            .await
            .unwrap();
        let blocked = SubscriptionTarget::BranchBase("/blocked".into(), "main".into());
        let mut blocker =
            Box::pin(subscriptions.start_subscription("client", &blocked, "blocked", None));
        assert!(futures_util::poll!(&mut blocker).is_pending());
        let mut request = unary_request("StartStateSubscription", NOTION_REQUEST_B);
        request
            .headers_mut()
            .insert("connect-timeout-ms", "1000".parse().unwrap());
        let mut call = Box::pin(app.clone().oneshot(request));
        // When
        assert!(futures_util::poll!(&mut call).is_pending());
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        let expired = call.await.unwrap();
        let body = axum::body::to_bytes(expired.into_body(), usize::MAX)
            .await
            .unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        drop(blocker);
        let stopped = app
            .oneshot(unary_request(
                "StopStateSubscription",
                r#"{"subscriptionId":"notion-a"}"#,
            ))
            .await
            .unwrap();
        // Then
        assert_eq!(error["code"], "deadline_exceeded");
        assert!(stopped.status().is_success());
        assert_eq!(subscriptions.test_usecase().test_worker_count(), 0);
    }

    #[tokio::test]
    pub async fn test_notion購読_重複開始の中断で既存要求を消さない() {
        use crate::usecase::state_subscription::SubscriptionTarget;
        use tower::ServiceExt;
        // Given
        let (app, subscriptions) = notion_cancellation_fixture();
        let _stream = subscriptions.open_stream("client".into()).unwrap();
        app.clone()
            .oneshot(unary_request("StartStateSubscription", NOTION_REQUEST_A))
            .await
            .unwrap();
        app.clone()
            .oneshot(unary_request("StartStateSubscription", NOTION_REQUEST_B))
            .await
            .unwrap();
        let blocked = SubscriptionTarget::BranchBase("/blocked".into(), "main".into());
        let mut blocker =
            Box::pin(subscriptions.start_subscription("client", &blocked, "blocked", None));
        assert!(futures_util::poll!(&mut blocker).is_pending());
        let call = Box::pin(
            app.clone()
                .oneshot(unary_request("StartStateSubscription", NOTION_REQUEST_A)),
        );
        // When
        let response = call.await.unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["code"], "already_exists");
        drop(blocker);
        let stopped = app
            .oneshot(unary_request(
                "StopStateSubscription",
                r#"{"subscriptionId":"notion-b"}"#,
            ))
            .await
            .unwrap();
        // Then
        assert!(stopped.status().is_success());
        assert_eq!(subscriptions.test_usecase().test_worker_count(), 1);
    }

    #[tokio::test]
    pub async fn test_notion購読_最後の停止の中断で要求と購読を保持する() {
        use crate::usecase::state_subscription::SubscriptionTarget;
        use tower::ServiceExt;
        // Given
        let (app, subscriptions) = notion_cancellation_fixture();
        let _stream = subscriptions.open_stream("client".into()).unwrap();
        app.clone()
            .oneshot(unary_request("StartStateSubscription", NOTION_REQUEST_A))
            .await
            .unwrap();
        let blocked = SubscriptionTarget::BranchBase("/blocked".into(), "main".into());
        let mut blocker =
            Box::pin(subscriptions.start_subscription("client", &blocked, "blocked", None));
        assert!(futures_util::poll!(&mut blocker).is_pending());
        let target = SubscriptionTarget::from_parts(
            "notion-tasks",
            &["/repo", "20", r#"labels={"Tags":["a"]}"#],
        )
        .unwrap();
        let mut call = Box::pin(app.clone().oneshot(unary_request(
            "StopStateSubscription",
            r#"{"subscriptionId":"notion-a"}"#,
        )));
        // When
        assert!(futures_util::poll!(&mut call).is_pending());
        drop(call);
        let registration = subscriptions
            .test_presenter()
            .delivery("notion-a")
            .map(|(client, target, _)| (client, target));
        let workers = subscriptions.test_usecase().test_worker_count();
        drop(blocker);
        let stopped = app
            .oneshot(unary_request(
                "StopStateSubscription",
                r#"{"subscriptionId":"notion-a"}"#,
            ))
            .await
            .unwrap();
        // Then
        assert_eq!(registration, Some(("client".into(), target.to_string())));
        assert_eq!(workers, 1);
        assert!(stopped.status().is_success());
        assert_eq!(subscriptions.test_usecase().test_worker_count(), 0);
    }

    #[tokio::test]
    pub async fn test_購読識別子_入口で形と全clientの重複を検査し未知の停止は成功する() {
        use crate::usecase::state_subscription::SubscriptionTarget;
        use tower::ServiceExt;
        // Given
        let (app, subscriptions) = notion_cancellation_fixture();
        let first = subscriptions.open_stream("client".into()).unwrap();
        let _second = subscriptions.open_stream("other".into()).unwrap();
        let request = |client: &str, id: &str| {
            serde_json::json!({ "clientId": client, "subscriptionId": id, "target": "notion-tasks", "args": ["/repo", "20"] }).to_string()
        };
        // When / Then
        for id in [String::new(), "x".repeat(129), "あ".repeat(43)] {
            let response = app
                .clone()
                .oneshot(unary_request(
                    "StartStateSubscription",
                    &request("client", &id),
                ))
                .await
                .unwrap();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(error["code"], "invalid_argument");
        }
        for id in ["x".repeat(128), "あ".repeat(42)] {
            assert!(app
                .clone()
                .oneshot(unary_request(
                    "StartStateSubscription",
                    &request("client", &id)
                ))
                .await
                .unwrap()
                .status()
                .is_success());
            for client in ["client", "other"] {
                let response = app
                    .clone()
                    .oneshot(unary_request(
                        "StartStateSubscription",
                        &request(client, &id),
                    ))
                    .await
                    .unwrap();
                let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(error["code"], "already_exists");
            }
            let stop = serde_json::json!({"subscriptionId": id}).to_string();
            for _ in 0..2 {
                assert!(app
                    .clone()
                    .oneshot(unary_request("StopStateSubscription", &stop))
                    .await
                    .unwrap()
                    .status()
                    .is_success());
            }
            assert!(app
                .clone()
                .oneshot(unary_request(
                    "StartStateSubscription",
                    &request("other", &id)
                ))
                .await
                .unwrap()
                .status()
                .is_success());
            subscriptions.stop_subscription(&id).await.unwrap();
        }
        let target = SubscriptionTarget::from_parts("notion-tasks", &["/repo", "20"]).unwrap();
        subscriptions
            .start_subscription("client", &target, "x", None)
            .await
            .unwrap();
        assert!(matches!(
            subscriptions
                .terminal_processed("x", 5000)
                .unwrap_err()
                .source,
            crate::usecase::state_subscription::StateReadFailure::TerminalSubscriptionEnded
        ));
        drop(first);
        subscriptions.stop_subscription("x").await.unwrap();
        subscriptions
            .stop_subscription("never-started")
            .await
            .unwrap();
        subscriptions
            .start_subscription("other", &target, "x", None)
            .await
            .unwrap();
    }

    #[tokio::test]
    pub async fn test_購読停止_待機中に再利用された識別子の新しい登録を解除しない() {
        // Given
        use crate::usecase::state_subscription::SubscriptionTarget;
        use futures_util::{poll, StreamExt};
        let (_app, subscriptions) = notion_cancellation_fixture();
        let mut first = Box::pin(subscriptions.stream("first".into()).unwrap());
        let mut second = Box::pin(subscriptions.stream("second".into()).unwrap());
        first.next().await.unwrap();
        second.next().await.unwrap();
        let target = SubscriptionTarget::from_parts("notion-tasks", &["/repo", "20"]).unwrap();
        subscriptions
            .start_subscription("first", &target, "x", None)
            .await
            .unwrap();
        let blocked = SubscriptionTarget::BranchBase("/blocked".into(), "main".into());
        let mut blocker =
            Box::pin(subscriptions.start_subscription("second", &blocked, "blocked", None));
        assert!(poll!(&mut blocker).is_pending());
        // When
        let stop = subscriptions.stop_subscription("x");
        tokio::pin!(stop);
        assert!(poll!(&mut stop).is_pending());
        drop(first);
        assert!(subscriptions
            .test_presenter()
            .delivery("x")
            .map(|(client, target, _)| (client, target))
            .is_none());
        let delivery = subscriptions
            .test_presenter()
            .reserve_delivery("second", "x", &target.to_string(), None)
            .unwrap();
        drop(blocker);
        let (stopped, started) = tokio::join!(
            stop,
            subscriptions
                .test_usecase()
                .start_subscription("second", &target, &delivery),
        );
        stopped.unwrap();
        started.unwrap();
        // Then
        assert_eq!(
            subscriptions
                .test_presenter()
                .delivery("x")
                .map(|(client, target, _)| (client, target)),
            Some(("second".into(), target.to_string()))
        );
        assert!(subscriptions
            .test_usecase()
            .active_targets()
            .contains(&target));
    }

    #[tokio::test]
    pub async fn test_購読停止_解放後の同じclientの再登録へ古い後始末が作用しない() {
        // Given
        use crate::usecase::state_subscription::{StateSubscriptionDelivery, SubscriptionTarget};
        let (_app, subscriptions) = notion_cancellation_fixture();
        let _stream = subscriptions.open_stream("client".into()).unwrap();
        let target = SubscriptionTarget::from_parts("notion-tasks", &["/repo", "20"]).unwrap();
        subscriptions
            .start_subscription("client", &target, "x", None)
            .await
            .unwrap();
        let (_, _, old) = subscriptions.test_presenter().delivery("x").unwrap();
        subscriptions.stop_subscription("x").await.unwrap();
        subscriptions
            .start_subscription("client", &target, "x", None)
            .await
            .unwrap();
        // When
        old.finish(&subscriptions.test_usecase().active_targets())
            .unwrap();
        subscriptions
            .test_usecase()
            .stop_subscription("client", &target, &old)
            .await
            .unwrap();
        // Then
        assert_eq!(
            subscriptions
                .test_presenter()
                .delivery("x")
                .map(|(client, target, _)| (client, target)),
            Some(("client".into(), target.to_string()))
        );
        assert!(subscriptions
            .test_usecase()
            .active_targets()
            .contains(&target));
        subscriptions.stop_subscription("x").await.unwrap();
        assert!(subscriptions.test_usecase().active_targets().is_empty());
    }

    #[tokio::test]
    pub async fn test_terminal購読識別子_入口で空と超過を拒み上限と空白を受け付ける() {
        use tower::ServiceExt;
        // Given
        let (terminal, _, _, _) =
            crate::test_support::state_subscription::terminal_application_fixture();
        let subscriptions =
            crate::test_support::state_subscription::test_subscriptions().with_terminal(terminal);
        let deps = subscriptions.deps();
        let _stream = deps.stream("client".into()).unwrap();
        let app = router(
            Some(
                crate::test_support::client_api_deps(Arc::new(dispatch()))
                    .with_state_subscriptions(deps),
            ),
            crate::adaptor::controller::daemon::default_timeout(),
        );
        // When / Then
        for id in [String::new(), "x".repeat(129), "あ".repeat(43)] {
            let body = serde_json::json!({"clientId":"client", "subscriptionId":id,"target":"terminal","args":["/repo"]}).to_string();
            let response = app
                .clone()
                .oneshot(unary_request("StartStateSubscription", &body))
                .await
                .unwrap();
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(error["code"], "invalid_argument");
        }
        for id in ["x".repeat(128), "あ".repeat(42), " ".into()] {
            let body = serde_json::json!({"clientId":"client", "subscriptionId":id,"target":"terminal","args":["/repo"]}).to_string();
            let response = app
                .clone()
                .oneshot(unary_request("StartStateSubscription", &body))
                .await
                .unwrap();
            assert!(response.status().is_success());
        }
    }
}

struct SignalIngress(
    std::sync::Mutex<Vec<crate::domain::provider_lifecycle::ProviderLifecycleSignal>>,
);
#[async_trait::async_trait]
impl crate::usecase::provider_lifecycle::ProviderLifecycleIngressPort for SignalIngress {
    async fn receive(
        &self,
        _: &crate::domain::provider_lifecycle::ProviderLifecycleSlotId,
        capability: &str,
        signal: crate::domain::provider_lifecycle::ProviderLifecycleSignal,
    ) -> Result<
        crate::domain::provider_lifecycle::ProviderLifecycleIngressResult,
        crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError,
    > {
        use crate::domain::provider_lifecycle::ProviderLifecycleIngressResult as Result;
        if capability != "capability" {
            return Ok(Result::Rejected(
                crate::domain::provider_lifecycle::ProviderLifecycleRejection::InvalidCapability,
            ));
        }
        let mut signals = self.0.lock().unwrap();
        let duplicate = signals.contains(&signal);
        signals.push(signal);
        Ok(if duplicate {
            Result::Duplicate
        } else {
            Result::Applied
        })
    }
    async fn report_unavailable(
        &self,
        _: &crate::domain::provider_lifecycle::ProviderLifecycleSlotId,
        _: &str,
        _: crate::domain::provider_lifecycle::ProviderLifecycleUnavailableObservation,
    ) -> Result<
        crate::domain::provider_lifecycle::ProviderLifecycleIngressResult,
        crate::usecase::provider_lifecycle::ProviderLifecycleIngressUsecaseError,
    > {
        unreachable!()
    }
}

#[tokio::test]
async fn test_hookのconnect入口_生payloadを解釈し上限と権限と結果を保持する() {
    use base64::Engine;
    use tower::ServiceExt;
    let ingress = Arc::new(SignalIngress(Default::default()));
    let hook = crate::infrastructure::local_api::BearerToken::from(Arc::<str>::from("hook"));
    let deps = crate::test_support::client_api_deps(Arc::new(dispatch())).with_provider_lifecycle(
        ingress.clone(),
        Arc::new(crate::adaptor::gateway::provider_lifecycle::LocalProviderPayloadInterpreter),
    );
    let app = router(
        Some(deps),
        crate::adaptor::controller::daemon::default_timeout(),
    )
    .layer(axum::middleware::from_fn_with_state(
        super::super::auth::ClientTokens {
            operator: Arc::<str>::from("operator").into(),
            hook: Some(hook.clone()),
        },
        super::super::auth::require_client,
    ));
    let payload = br#"{"hook_event_name":"SessionStart","session_id":"provider-session"}"#;
    for (bytes, token, capability, expected) in [
        (payload.to_vec(), "hook", "capability", "applied"),
        (payload.to_vec(), "hook", "capability", "duplicate"),
        (payload.to_vec(), "hook", "invalid", "rejected"),
        (br#"{"hook_event_name":"SessionStart","session_id":"provider-session","agent_id":"subagent"}"#.to_vec(), "hook", "capability", "ignored"),
        (b"{".to_vec(), "hook", "capability", "invalid_argument"),
        ({ let mut bytes = payload.to_vec(); bytes.resize(65_536, b' '); bytes }, "hook", "capability", "duplicate"),
        (vec![b' ';65_537], "hook", "capability", "invalid_argument"),
        (payload.to_vec(), "operator", "capability", "permission_denied"),
    ] {
        let body = serde_json::json!({"provider":{"value":"claude"}, "slotId":"slot", "bindingId":"binding", "capability":capability, "agentSessionId":"session", "payload":base64::engine::general_purpose::STANDARD.encode(bytes)});
        let response = app.clone().oneshot(axum::http::Request::post("/releash.client.v1.ClientService/ReceiveProviderSignal").header("content-type","application/json").header("connect-protocol-version","1").header("authorization",format!("Bearer {token}")).body(axum::body::Body::from(body.to_string())).unwrap()).await.unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(result.get(expected).is_some() || result["code"] == expected, "{expected}: {result}");
    }
    assert_eq!(ingress.0.lock().unwrap().len(), 3);
    for method in [
        "UnknownMethod",
        "WorkflowGetOutput",
        "WorkflowValidateOutput",
    ] {
        for token in ["hook", "operator"] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::post(format!("/releash.client.v1.ClientService/{method}"))
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(result["code"], "unimplemented");
        }
    }
}
