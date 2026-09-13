use super::*;
use crate::usecase::application_startup::ApplicationStartupAuthority;
use serde_json::Value;

#[tokio::test]
async fn test_クライアントdispatch_startup失敗時はusecase実行前に拒否する() {
    // Given
    let dispatch = ClientCommandDispatch::new(
        Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        Arc::new(ApplicationStartupAuthority::failed_kind(
            crate::usecase::application_startup::StartupFailureKind::StoreValidationFailed,
        )),
    );
    // When
    let error = dispatch
        .dispatch(wire::command_request::Command::GetCurrentBranch(
            wire::GetCurrentBranchRequest {
                repo_path: Some("/missing".into()),
            },
        ))
        .await
        .unwrap_err();
    // Then
    assert_eq!(
        wire::from_value(error).unwrap()["code"],
        "APPLICATION_UNAVAILABLE"
    );
}

#[test]
fn test_クライアント接続情報_terminalと同じ非master_tokenを返す() {
    // Given
    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    assert!(client_endpoint(app.handle()).is_none());
    app.manage(crate::adaptor::controller::state::TerminalStreamEndpoint {
        port: 12345,
        token: Arc::from("client-only"),
    });
    // When
    let endpoint = client_endpoint(app.handle()).unwrap();
    // Then
    assert_eq!(endpoint.url, "ws://127.0.0.1:12345/v1/client");
    assert_eq!(endpoint.auth_subprotocol, "releash-bearer.client-only");
    assert_eq!(
        serde_json::to_value(endpoint).unwrap(),
        serde_json::json!({
            "url": "ws://127.0.0.1:12345/v1/client",
            "authSubprotocol": "releash-bearer.client-only"
        })
    );
}

use crate::adaptor::controller::api::protocol::client as wire;
use prost::Message;
use serde_json::json;

fn parity_app() -> (
    tauri::App<tauri::test::MockRuntime>,
    Arc<ClientCommandDispatch>,
) {
    parity_app_with_runtime(None)
}

fn parity_app_with_runtime(
    runtime: Option<Arc<crate::usecase::workflow::WorkflowRuntimeUsecase>>,
) -> (
    tauri::App<tauri::test::MockRuntime>,
    Arc<ClientCommandDispatch>,
) {
    let (app, data_dir, _store) =
        crate::adaptor::controller::client::workflow::tests::make_read_only_app();
    if let Some(runtime) = runtime {
        app.manage(runtime);
    }
    let authority = Arc::new(ApplicationStartupAuthority::ready());
    app.manage(authority.clone());
    app.manage(Arc::new(
        crate::adaptor::controller::wiring::build_review_comment_usecase(),
    ));
    app.manage(Arc::new(
        crate::infrastructure::file_watcher::FileWatcherManager::default(),
    ));
    app.manage(Arc::new(
        crate::adaptor::gateway::workspace_state::WorkspaceStateStore::new(data_dir),
    ));
    use crate::usecase::agent_session::provider_availability_tests::{
        FakeProviderExecutableConfigRepository, FakeProviderExecutableProbeGateway,
    };
    app.manage(Arc::new(
        crate::usecase::agent_session::ProviderAvailabilityUsecase::initialize(
            Arc::new(FakeProviderExecutableConfigRepository::default()),
            Arc::new(FakeProviderExecutableProbeGateway::default()),
        )
        .unwrap(),
    ));
    let mut dispatch = ClientCommandDispatch::new(
        app.state::<Arc<crate::usecase::repository_usecase::RepositoryUsecase>>()
            .inner()
            .clone(),
        authority,
    );
    dispatch.register_dependencies(
        &crate::adaptor::controller::wiring::build_client_dependencies(app.handle()),
    );
    let dispatch = Arc::new(dispatch);
    app.manage(dispatch.clone());
    (app, dispatch)
}

async fn assert_parity(
    dispatch: &ClientCommandDispatch,
    command: &str,
    args: Value,
    expected: Result<Value, Value>,
) {
    let mut payload = wire::CommandRequest::from_value(command, args).unwrap();
    payload.request_id = "parity".into();
    let request = wire::Envelope {
        body: Some(wire::envelope::Body::Request(Box::new(payload))),
    };
    let wire::envelope::Body::Request(request) =
        wire::Envelope::decode(request.encode_to_vec().as_slice())
            .unwrap()
            .body
            .unwrap()
    else {
        panic!("request");
    };
    let result = dispatch.dispatch(request.command.unwrap()).await;
    let wire::envelope::Body::Response(response) = wire::Envelope::decode(
        wire::response("parity".into(), result)
            .encode_to_vec()
            .as_slice(),
    )
    .unwrap()
    .body
    .unwrap() else {
        panic!("response");
    };
    assert_eq!(response.request_id, "parity");
    let actual = match response.outcome.unwrap() {
        wire::command_response::Outcome::Result(value) => Ok(wire::from_value(value).unwrap()),
        wire::command_response::Outcome::Error(value) => Err(wire::from_value(value).unwrap()),
    };
    assert_eq!(actual, expected, "{command}");
}

macro_rules! parity {
    ($name:ident, $app:ident, $command:literal, $args:expr, $expected:expr) => {
        #[tokio::test]
        async fn $name() {
            // Given
            let ($app, dispatch) = parity_app();
            let expected = $expected;
            // When / Then
            assert_parity(&dispatch, $command, $args, expected).await;
        }
    };
}

parity!(
    test_repository_protoはusecase結果と一致する,
    app,
    "get_repo_paths",
    json!({}),
    value(
        invoke_tauri(&app, "get_repo_paths", json!({}))
            .await
            .unwrap()
    )
);
parity!(
    test_code_protoはusecase結果と一致する,
    app,
    "get_language_from_path",
    json!({"filePath":"main.rs"}),
    outcome(
        invoke_tauri(
            &app,
            "get_language_from_path",
            json!({"filePath": "main.rs"})
        )
        .await
    )
);
parity!(
    test_comment_protoはusecaseの不正入力エラーと一致する,
    app,
    "list_review_threads",
    json!({"worktreeName":"../invalid"}),
    outcome(
        invoke_tauri(
            &app,
            "list_review_threads",
            json!({"worktreeName": "../invalid","filter": null})
        )
        .await
    )
);
parity!(
    test_agent_session_protoはusecase結果と一致する,
    app,
    "get_provider_availability",
    json!({}),
    outcome(invoke_tauri(&app, "get_provider_availability", json!({})).await)
);
parity!(
    test_terminal_surface_protoはusecase結果と一致する,
    app,
    "get_terminal_surface",
    json!({"owner":{"kind":"workspace","workspacePath":"/missing"}}),
    outcome(
        invoke_tauri(
            &app,
            "get_terminal_surface",
            json!({"owner": crate::adaptor::protocol::terminal::TerminalSurfaceOwnerV1::Workspace {
                workspace_path: "/missing".into()
            }})
        )
        .await
    )
);
parity!(
    test_workflow_protoはusecase結果と一致する,
    app,
    "list_workflows",
    json!({}),
    outcome(invoke_tauri(&app, "list_workflows", json!({})).await)
);
parity!(
    test_workspace_tree_protoはusecase結果と一致する,
    app,
    "list_workspace_workflow_history",
    json!({"worktreePath":"/missing"}),
    outcome(
        invoke_tauri(
            &app,
            "list_workspace_workflow_history",
            json!({"worktreePath": "/missing"})
        )
        .await
    )
);
parity!(
    test_workspace_state_protoはusecase結果と一致する,
    app,
    "load_workspace_state",
    json!({"worktreeName":"missing","worktreeRoot":"/missing"}),
    value(
        invoke_tauri(
            &app,
            "load_workspace_state",
            json!({"worktreeName": "missing","worktreeRoot": "/missing"})
        )
        .await
        .unwrap()
    )
);
parity!(
    test_app_config_protoはusecase結果と一致する,
    app,
    "get_app_settings",
    json!({}),
    outcome(invoke_tauri(&app, "get_app_settings", json!({})).await)
);
parity!(
    test_notion_protoはusecase結果と一致する,
    app,
    "get_notion_config",
    json!({"repoPath":"/missing"}),
    outcome(invoke_tauri(&app, "get_notion_config", json!({"repoPath": "/missing"})).await)
);
parity!(
    test_git_host_protoはusecase結果と一致する,
    app,
    "get_cached_issues",
    json!({"repoPath":"/missing"}),
    outcome(invoke_tauri(&app, "get_cached_issues", json!({"repoPath": "/missing"})).await)
);
parity!(
    test_external_editor_protoはusecase結果と一致する,
    app,
    "get_external_editor",
    json!({}),
    outcome(invoke_tauri(&app, "get_external_editor", json!({})).await)
);
parity!(
    test_watcher_protoはusecase結果と一致する,
    app,
    "stop_watching",
    json!({"watcherId":999}),
    outcome(invoke_tauri(&app, "stop_watching", json!({"watcherId": 999})).await)
);
#[tokio::test]
async fn test_application起動結果_protoは本番shell入口の成功と失敗に一致する() {
    for authority in [
        ApplicationStartupAuthority::ready(),
        ApplicationStartupAuthority::failed_kind(
            crate::usecase::application_startup::StartupFailureKind::StoreValidationFailed,
        ),
    ] {
        // Given
        let (app, _data_dir, _store) =
            crate::adaptor::controller::client::workflow::tests::make_read_only_app();
        app.manage(Arc::new(
            crate::infrastructure::file_watcher::FileWatcherManager::default(),
        ));
        let authority = Arc::new(authority);
        app.manage(authority.clone());
        let mut dispatch = ClientCommandDispatch::new(
            Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
            authority,
        );
        dispatch.register_dependencies(
            &crate::adaptor::controller::wiring::build_client_dependencies(app.handle()),
        );
        // When: the shell must work without a managed client dispatch.
        let expected = invoke_tauri(&app, "get_application_startup_outcome", json!({})).await;
        assert!(expected.is_ok());
        // Then
        assert_parity(
            &dispatch,
            "get_application_startup_outcome",
            json!({}),
            expected,
        )
        .await;
    }
}

#[tokio::test]
async fn test_telemetry_protoはcommand結果と一致する() {
    // Given
    let _guard = crate::other::telemetry::lock_test_telemetry();
    let (app, dispatch) = parity_app();
    // When / Then
    invoke_tauri(&app, "report_mounted_xterm_count", json!({"count": 2}))
        .await
        .unwrap();
    assert_parity(
        &dispatch,
        "report_mounted_xterm_count",
        json!({"count":2}),
        Ok(Value::Null),
    )
    .await;
}

#[tokio::test]
async fn test_クライアントws_切断しても受理済みcommandを途中で破棄しない() {
    use crate::adaptor::controller::api;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    // Given
    let mut dispatch = ClientCommandDispatch::new(
        Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        Arc::new(ApplicationStartupAuthority::ready()),
    );
    let started = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Semaphore::new(0));
    let completed = Arc::new(tokio::sync::Semaphore::new(0));
    let notifications = (started.clone(), resume.clone(), completed.clone());
    dispatch.register_domain(
        &["get_language_from_path"],
        Box::new(move |_| {
            let (started, resume, completed) = notifications.clone();
            Box::pin(async move {
                started.notify_one();
                resume.acquire().await.unwrap().forget();
                completed.add_permits(1);
                Ok(wire::command_result::Command::GetLanguageFromPath(
                    wire::ResultString {
                        value: Some("done".into()),
                    },
                ))
            })
        }),
    );
    let data = tempfile::tempdir().unwrap();
    let router = api::test_support::test_router_with_optional_deps(
        data.path(),
        "master",
        "client",
        None,
        Some(api::ClientApiDeps::new(
            Arc::new(dispatch),
            crate::adaptor::gateway::push::ClientPushGateway::new(Arc::new(
                crate::infrastructure::push::PushSink::new(),
            )),
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
    // When
    for index in 0..64 {
        let (mut socket, _) = tokio_tungstenite::connect_async(request.clone())
            .await
            .unwrap();
        socket
            .send(Message::Binary(
                crate::client_api_acceptance::encode_client_request(
                    &format!("work-{index}"),
                    "get_language_from_path",
                    json!({"filePath":"/repo"}),
                )
                .into(),
            ))
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), started.notified())
            .await
            .unwrap();
        socket.close(None).await.unwrap();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
            .await
            .unwrap();
    }
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(Message::Binary(
            crate::client_api_acceptance::encode_client_request(
                "overflow",
                "get_language_from_path",
                json!({"filePath":"/repo"}),
            )
            .into(),
        ))
        .await
        .unwrap();
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Binary(bytes) = frame else {
        panic!("binary");
    };
    let wire::envelope::Body::Response(response) =
        wire::Envelope::decode(bytes).unwrap().body.unwrap()
    else {
        panic!("response");
    };
    let wire::command_response::Outcome::Error(error) = response.outcome.unwrap() else {
        panic!("limit");
    };
    assert_eq!(response.request_id, "overflow");
    assert_eq!(wire::from_value(error).unwrap()["code"], "REQUEST_LIMIT");
    resume.add_permits(64);
    // Then
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        completed.acquire_many(64),
    )
    .await
    .expect("受理済み操作は接続の寿命から独立して完了する")
    .unwrap()
    .forget();
    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn test_worktree変更_protoは実引数の成功とusecaseエラーを保持する() {
    // Given
    let (_temp, path) = mutation_repository();
    let (app, dispatch) = parity_app();
    let uc = app
        .state::<crate::adaptor::controller::state::AppState>()
        .repository_usecase
        .clone();
    let branch = "feature/parity";
    let expected = invoke_tauri(&app, "create_worktree", json!({"repoPath": path.clone(),"branch": branch,"createBranch": true,"baseBranch": Some("base")}))
    .await
    .unwrap();
    let worktree_path = expected["path"].as_str().unwrap().to_owned();
    uc.remove_worktree(&path, &worktree_path, true).unwrap();
    uc.delete_branch(&path, branch, true).unwrap();
    // When / Then
    assert_parity(
        &dispatch,
        "create_worktree",
        json!({"repoPath":path,"branch":branch,"createBranch":true,"baseBranch":"base"}),
        value(expected),
    )
    .await;
    assert!(std::path::Path::new(&worktree_path).is_dir());
    let expected = outcome(
        invoke_tauri(&app, "create_worktree", json!({"repoPath": path.clone(),"branch": "invalid-base","createBranch": true,"baseBranch": Some("missing-base")}))
        .await,
    );
    assert!(expected.is_err());
    assert_parity(&dispatch, "create_worktree", json!({"repoPath":path,"branch":"invalid-base","createBranch":true,"baseBranch":"missing-base"}), expected).await;
    std::fs::write(
        std::path::Path::new(&worktree_path).join("dirty.txt"),
        "dirty",
    )
    .unwrap();
    let expected = outcome(
        invoke_tauri(
            &app,
            "remove_worktree",
            json!({"repoPath": path.clone(),"worktreePath": worktree_path.clone(),"force": false}),
        )
        .await,
    );
    assert!(expected.is_err());
    assert_parity(
        &dispatch,
        "remove_worktree",
        json!({"repoPath":path,"worktreePath":worktree_path,"force":false}),
        expected,
    )
    .await;
    let expected = outcome(
        invoke_tauri(
            &app,
            "remove_worktree",
            json!({"repoPath": path.clone(),"worktreePath": worktree_path.clone(),"force": true}),
        )
        .await,
    );
    assert!(expected.is_ok());
    uc.create_worktree(&path, branch, false, Some("base"))
        .unwrap();
    std::fs::write(
        std::path::Path::new(&worktree_path).join("dirty.txt"),
        "dirty",
    )
    .unwrap();
    assert_parity(
        &dispatch,
        "remove_worktree",
        json!({"repoPath":path,"worktreePath":worktree_path,"force":true}),
        expected,
    )
    .await;
    assert!(!std::path::Path::new(&worktree_path).exists());
}

#[tokio::test]
async fn test_staging変更_protoは複数pathとusecaseエラーを保持する() {
    // Given
    let (_temp, path) = mutation_repository();
    let (app, dispatch) = parity_app();
    let paths = vec!["space name.txt".to_string(), "日本語.txt".to_string()];
    for name in &paths {
        std::fs::write(std::path::Path::new(&path).join(name), name).unwrap();
    }
    let uc = app
        .state::<crate::adaptor::controller::state::AppState>()
        .code_usecase
        .clone();
    let expected = outcome(
        invoke_tauri(
            &app,
            "git_stage",
            json!({"repoPath": path.clone(),"paths": paths.clone()}),
        )
        .await,
    );
    assert!(expected.is_ok());
    uc.git_unstage(&path, paths.clone()).unwrap();
    // When / Then
    assert_parity(
        &dispatch,
        "git_stage",
        json!({"repoPath":path,"paths":paths}),
        expected,
    )
    .await;
    assert_eq!(
        git2::Repository::open(&path)
            .unwrap()
            .index()
            .unwrap()
            .len(),
        2
    );
    let expected = outcome(
        invoke_tauri(
            &app,
            "git_unstage",
            json!({"repoPath": path.clone(),"paths": paths.clone()}),
        )
        .await,
    );
    uc.git_stage(&path, paths.clone()).unwrap();
    assert_parity(
        &dispatch,
        "git_unstage",
        json!({"repoPath":path,"paths":paths}),
        expected,
    )
    .await;
    assert_eq!(
        git2::Repository::open(&path)
            .unwrap()
            .index()
            .unwrap()
            .len(),
        0
    );
    for name in ["git_stage", "git_unstage"] {
        let missing = format!("{path}/missing-repo");
        let expected = if name == "git_stage" {
            outcome(
                invoke_tauri(
                    &app,
                    "git_stage",
                    json!({"repoPath": missing.clone(),"paths": paths.clone()}),
                )
                .await,
            )
        } else {
            outcome(
                invoke_tauri(
                    &app,
                    "git_unstage",
                    json!({"repoPath": missing.clone(),"paths": paths.clone()}),
                )
                .await,
            )
        };
        assert!(expected.is_err());
        assert_parity(
            &dispatch,
            name,
            json!({"repoPath":missing,"paths":paths}),
            expected,
        )
        .await;
    }
}

#[tokio::test]
async fn test_review_group変更_protoは複合引数と部分stagingとusecaseエラーを保持する() {
    use crate::usecase::{code_dto::ReviewFileViewDto, review_usecase::ReviewTarget};

    for (command, section) in [
        ("git_stage_review_group", "changes"),
        ("git_unstage_review_group", "staged"),
    ] {
        // Given
        let (_temp, path) = mutation_repository();
        let (app, dispatch) = parity_app();
        let state = app.state::<crate::adaptor::controller::state::AppState>();
        let repo = git2::Repository::open(&path).unwrap();
        let path = repo.workdir().unwrap().to_string_lossy().into_owned();
        let file = "日本語 space.txt";
        let original = (1..=20)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();
        let modified = original
            .replace("line 2\n", "first change\n")
            .replace("line 18\n", "last change\n");
        std::fs::write(std::path::Path::new(&path).join(file), &original).unwrap();
        state
            .code_usecase
            .git_stage(&path, vec![file.into()])
            .unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let signature = git2::Signature::now("test", "test@example.com").unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "tracked file",
            &repo.find_tree(tree_id).unwrap(),
            &[&repo.head().unwrap().peel_to_commit().unwrap()],
        )
        .unwrap();
        std::fs::write(std::path::Path::new(&path).join(file), &modified).unwrap();
        if section == "staged" {
            state
                .code_usecase
                .git_stage(&path, vec![file.into()])
                .unwrap();
        }
        let ReviewFileViewDto::TextDiff(view) = state
            .review_usecase
            .get_review_file_view(
                &path,
                ReviewTarget::Path(file.into()),
                section,
                "head",
                None,
                None,
            )
            .unwrap()
        else {
            panic!("text diff");
        };
        assert_eq!(view.change_groups.len(), 2);
        let input = json!({"worktreePath":path,"path":file,"section":section,"base":"head","groupId":view.change_groups[1].group_id});
        let app_ref = &app;
        let call = |input: Value| async move {
            invoke_tauri(app_ref, command, json!({"input": input})).await
        };
        // When / Then
        for (field, invalid) in [
            ("worktreePath", format!("{path}/missing")),
            ("path", "missing.txt".into()),
            ("section", "invalid-section".into()),
            ("base", "invalid-base".into()),
            ("groupId", "missing-group".into()),
        ] {
            let mut invalid_input = input.clone();
            invalid_input[field] = json!(invalid);
            let expected = outcome(call(invalid_input.clone()).await);
            assert!(expected.is_err(), "{command}: {field}");
            assert_parity(&dispatch, command, json!({"input":invalid_input}), expected).await;
        }
        let expected = outcome(call(input.clone()).await);
        assert_eq!(expected, Ok(Value::Null));
        let index_content = || {
            let mut index = repo.index().unwrap();
            index.read(true).unwrap();
            let id = index.get_path(std::path::Path::new(file), 0).unwrap().id;
            repo.find_blob(id).unwrap().content().to_vec()
        };
        let expected_content = index_content();
        let partial = if section == "changes" {
            original.replace("line 18\n", "last change\n")
        } else {
            original.replace("line 2\n", "first change\n")
        };
        assert_eq!(expected_content, partial.as_bytes());
        if section == "changes" {
            state
                .code_usecase
                .git_unstage(&path, vec![file.into()])
                .unwrap();
        } else {
            state
                .code_usecase
                .git_stage(&path, vec![file.into()])
                .unwrap();
        }
        assert_parity(&dispatch, command, json!({"input":input}), expected).await;
        assert_eq!(index_content(), expected_content);
        assert_eq!(
            std::fs::read_to_string(std::path::Path::new(&path).join(file)).unwrap(),
            modified
        );
    }
}

#[tokio::test]
async fn test_workflow変更_protoは実引数とruntime結果を保持する() {
    // Given
    use crate::adaptor::controller::api::test_support::RecordingRuntimeGateway;
    use crate::usecase::workflow::WorkflowRuntimeUsecase;
    let gateway = Arc::new(RecordingRuntimeGateway::default());
    let runtime = Arc::new(WorkflowRuntimeUsecase::new(gateway.clone()));
    let (app, dispatch) = parity_app_with_runtime(Some(runtime));
    let id = "00000000-0000-4000-8000-000000000001";
    for failure in [false, true] {
        if failure {
            let mut errors = gateway.errors.lock().unwrap();
            errors.start = Some(crate::domain::workflow::WorkflowError::external(
                "start failed",
            ));
            errors.abort = Some(crate::domain::workflow::WorkflowError::external(
                "abort failed",
            ));
            errors.stop = Some(crate::domain::workflow::WorkflowError::external(
                "stop failed",
            ));
            errors.resume = Some(crate::domain::workflow::WorkflowError::external(
                "resume failed",
            ));
        }
        // When / Then
        let expected = outcome(
            invoke_tauri(&app, "start_workflow", json!({"workflowName": "workflow-name","worktreePath": "/workspace with space","request": Some("日本語の依頼"),"createdFrom": Some("cli")}))
            .await,
        );
        assert_eq!(expected.is_err(), failure);
        assert_parity(&dispatch, "start_workflow", json!({"workflowName":"workflow-name","worktreePath":"/workspace with space","request":"日本語の依頼","createdFrom":"cli"}), expected).await;
        let expected =
            outcome(invoke_tauri(&app, "abort_workflow", json!({"executionId": id})).await);
        assert_eq!(expected.is_err(), failure);
        assert_parity(
            &dispatch,
            "abort_workflow",
            json!({"executionId":id}),
            expected,
        )
        .await;
        let expected =
            outcome(invoke_tauri(&app, "stop_workflow", json!({"executionId": id})).await);
        assert_eq!(expected.is_err(), failure);
        assert_parity(
            &dispatch,
            "stop_workflow",
            json!({"executionId":id}),
            expected,
        )
        .await;
        let expected =
            outcome(invoke_tauri(&app, "resume_workflow", json!({"executionId": id})).await);
        assert_eq!(expected.is_err(), failure);
        assert_parity(
            &dispatch,
            "resume_workflow",
            json!({"executionId":id}),
            expected,
        )
        .await;
    }
    let commands = gateway.commands.lock().unwrap();
    assert_eq!(commands.starts.len(), 2);
    assert_eq!(commands.starts[0], commands.starts[1]);
    assert_eq!(commands.starts[0].workflow_name, "workflow-name");
    assert_eq!(commands.starts[0].worktree_path, "/workspace with space");
    assert_eq!(commands.starts[0].request.as_deref(), Some("日本語の依頼"));
    assert_eq!(
        commands.starts[0].created_from,
        crate::domain::workflow::ExecutionOrigin::Cli
    );
    assert_eq!(commands.aborts.len(), 2);
    assert_eq!(commands.aborts[0], commands.aborts[1]);
    assert_eq!(commands.stops.len(), 2);
    assert_eq!(commands.stops[0], commands.stops[1]);
    assert_eq!(commands.resumes.len(), 2);
    assert_eq!(commands.resumes[0], commands.resumes[1]);
}

#[tokio::test]
async fn test_workspace保存_wsがui追加fieldを受理し既存項目を再起動後に復元する() {
    use crate::adaptor::controller::api;
    use crate::adaptor::gateway::workspace_state::WorkspaceStateStore;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};
    // Given
    let data = tempfile::tempdir().unwrap();
    let worktree = data.path().join("worktree");
    std::fs::create_dir_all(worktree.join("src")).unwrap();
    std::fs::write(worktree.join("src/main.rs"), "fn main() {}\n").unwrap();
    let state = json!({"version":1,"tabs":{"editors":[{"path":"src/main.rs","name":"main.rs"}],"activeEditorPath":"src/main.rs"},"layout":{"centerTab":"editor","activeView":"git","leftNavCollapsed":true,"rightCollapsed":true,"rightBottomCollapsed":false,"rightBottomActiveTab":"terminal","selectedDiffFile":"src/main.rs","reviewCollapsed":true,"diffOnlyMode":true}});
    let (app, _) = parity_app();
    let mut deps = crate::adaptor::controller::wiring::build_client_dependencies(app.handle());
    deps.workspace_state_store = Some(Arc::new(WorkspaceStateStore::new(data.path().to_owned())));
    let mut dispatch = ClientCommandDispatch::new(
        Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        Arc::new(ApplicationStartupAuthority::ready()),
    );
    dispatch.register_dependencies(&deps);
    let router = api::test_support::test_router_with_optional_deps(
        data.path(),
        "master",
        "client",
        None,
        Some(api::ClientApiDeps::new(
            Arc::new(dispatch),
            crate::adaptor::gateway::push::ClientPushGateway::new(Arc::new(
                crate::infrastructure::push::PushSink::new(),
            )),
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
    // When
    socket
        .send(Message::Binary(
            crate::client_api_acceptance::encode_client_request(
                "save",
                "save_workspace_state",
                json!({"worktreeName":"workspace","state":state}),
            )
            .into(),
        ))
        .await
        .unwrap();
    let Message::Binary(bytes) = socket.next().await.unwrap().unwrap() else {
        panic!("binary response");
    };
    let wire::envelope::Body::Response(response) =
        wire::Envelope::decode(bytes).unwrap().body.unwrap()
    else {
        panic!("response");
    };
    assert_eq!(response.request_id, "save");
    let wire::command_response::Outcome::Result(result) = response.outcome.unwrap() else {
        panic!("save succeeded");
    };
    assert_eq!(wire::from_value(result).unwrap(), Value::Null);
    socket.close(None).await.unwrap();
    server.abort();
    deps.workspace_state_store = Some(Arc::new(WorkspaceStateStore::new(data.path().to_owned())));
    let mut restarted = ClientCommandDispatch::new(
        Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        Arc::new(ApplicationStartupAuthority::ready()),
    );
    restarted.register_dependencies(&deps);
    let mut expected = state;
    expected["layout"]
        .as_object_mut()
        .unwrap()
        .remove("reviewCollapsed");
    expected["layout"]
        .as_object_mut()
        .unwrap()
        .remove("diffOnlyMode");
    // Then
    assert_parity(
        &restarted,
        "load_workspace_state",
        json!({"worktreeName":"workspace","worktreeRoot":worktree}),
        Ok(expected.clone()),
    )
    .await;
    let persisted: Value = serde_json::from_slice(
        &std::fs::read(data.path().join("workspace_state/workspace.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted, expected);
}

fn mutation_repository() -> (tempfile::TempDir, String) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("repository");
    let repo = git2::Repository::init(&path).unwrap();
    let signature = git2::Signature::now("test", "test@example.com").unwrap();
    let tree_id = repo.index().unwrap().write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(
        Some("refs/heads/base"),
        &signature,
        &signature,
        "initial",
        &tree,
        &[],
    )
    .unwrap();
    repo.set_head("refs/heads/base").unwrap();
    (temp, path.to_string_lossy().into_owned())
}

fn value(result: impl serde::Serialize) -> Result<Value, Value> {
    serde_json::to_value(result).map_err(|error| json!(error.to_string()))
}
fn outcome<T: serde::Serialize, E: serde::Serialize>(result: Result<T, E>) -> Result<Value, Value> {
    result
        .map_err(|error| serde_json::to_value(error).unwrap())
        .and_then(value)
}

pub(crate) async fn invoke_tauri(
    app: &tauri::App<tauri::test::MockRuntime>,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    let window = tauri::WebviewWindowBuilder::new(
        app,
        format!("parity-{}", uuid::Uuid::new_v4()),
        Default::default(),
    )
    .build()
    .unwrap();
    let result = tauri::test::get_ipc_response(
        &window,
        tauri::webview::InvokeRequest {
            cmd: command.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        },
    )
    .map(|response| response.deserialize::<Value>().unwrap());
    window.destroy().unwrap();
    result
}
