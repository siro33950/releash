use crate::adaptor::controller::client::ClientCommandDispatch;
use crate::usecase::application_startup::ApplicationStartupAuthority;
use serde_json::Value;
use std::sync::Arc;
use tauri::Manager;

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

use crate::adaptor::controller::api::protocol::client as wire;
use crate::adaptor::controller::command as commands;
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
    dispatch.register_dependencies(&crate::desktop_test_support::build_client_dependencies(
        app.handle(),
    ));
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
    let payload = wire::CommandRequest::from_value(command, args).unwrap();
    let request = wire::CommandRequest::decode(payload.encode_to_vec().as_slice()).unwrap();
    let actual = match dispatch.dispatch(request.command.unwrap()).await {
        Ok(result) => Ok(wire::from_value(wire::CommandResult {
            command: Some(result),
        })
        .unwrap()),
        Err(error) => Err(wire::from_value(error).unwrap()),
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
#[tokio::test]
async fn test_watcher_protoはusecase結果と一致する() {
    use crate::adaptor::controller::api;
    // Given
    let (app, dispatch) = parity_app();
    let watcher = crate::desktop_test_support::build_watcher_usecase(app.handle());
    let expected =
        api::protocol::connect::command_error(watcher.stop(999).unwrap_err().to_string().into());
    let router = api::client::router(Some(api::ClientApiDeps::new(
        dispatch,
        crate::adaptor::gateway::push::ClientPushGateway::new(Arc::new(
            crate::infrastructure::push::PushSink::new(),
        )),
        watcher,
    )));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = crate::client_api_acceptance::connect_client(
        &crate::client_api_acceptance::ClientEndpoint {
            url: format!("http://{}", listener.local_addr().unwrap()),
            token: "client".into(),
            launch_id: String::new(),
        },
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    // When
    let actual = crate::client_api_acceptance::request_client(
        &client,
        "stop_watching",
        json!({"watcherId": 999}),
    )
    .await
    .unwrap_err();
    // Then
    assert_eq!(actual.code, expected.code);
    assert_eq!(value(actual.details), value(expected.details));
    server.abort();
}
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
        dispatch.register_dependencies(&crate::desktop_test_support::build_client_dependencies(
            app.handle(),
        ));
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
async fn test_クライアントdispatch_proto全commandの登録と引数検証() {
    // Given
    let (_app, dispatch) = parity_app();
    // When / Then
    assert_eq!(wire::COMMAND_NAMES.len(), 164);
    for removed in [
        "stop_workflow",
        "resume_workflow",
        "get_application_quit_operation",
        "get_application_shutdown",
        "get_shutdown_plan",
        "resolve_shutdown_target_action",
        "list_pending_application_attempts",
        "acknowledge_application_attempt",
        "compact_application_shutdown_details",
    ] {
        assert!(!wire::COMMAND_NAMES.contains(&removed));
        assert!(!dispatch.contains(removed));
    }
    assert!(!wire::COMMAND_NAMES.contains(&"attach_terminal_surface"));
    for command in commands::tests::registered_command_names() {
        if command != "set_menu_items_enabled"
            && !commands::client::COMMAND_NAMES.contains(&command)
            && !commands::desktop_lifecycle::COMMAND_NAMES.contains(&command)
        {
            assert!(wire::COMMAND_NAMES.contains(&command), "{command}");
        }
    }
    assert!(!commands::tests::registered_command_names().contains(&"get_terminal_stream_endpoint"));
    for command in wire::COMMAND_NAMES {
        assert_eq!(
            dispatch.contains(command),
            !["stop_watching", "detach_terminal_surface"].contains(command),
            "{command}"
        );
    }
    for command in [
        "menu",
        "set_menu_items_enabled",
        "apply_desktop_settings",
        "get_terminal_stream_endpoint",
        "start_watching",
        "start_git_dir_watching",
    ]
    .into_iter()
    .chain(commands::desktop_lifecycle::COMMAND_NAMES.iter().copied())
    {
        assert!(!wire::COMMAND_NAMES.contains(&command), "{command}");
    }
    for args in [json!({}), json!({"filePath":9})] {
        let error = invoke_tauri(&_app, "get_language_from_path", args)
            .await
            .unwrap_err();
        assert_eq!(error["code"], "INVALID_REQUEST");
    }
}

#[tokio::test]
async fn test_クライアントdispatch_startup失敗時はstreamも拒否する() {
    // Given
    let dispatch = ClientCommandDispatch::new(
        Arc::new(crate::adaptor::controller::wiring::build_repository_usecase()),
        Arc::new(ApplicationStartupAuthority::failed_kind(
            crate::usecase::application_startup::StartupFailureKind::StoreValidationFailed,
        )),
    );
    // When / Then
    for command in ["attach_terminal_surface", "detach_terminal_surface"] {
        assert_eq!(
            wire::from_value(dispatch.admit(command).unwrap_err()).unwrap()["code"],
            "APPLICATION_UNAVAILABLE"
        );
    }
}

#[tokio::test]
async fn test_クライアントws_切断しても受理済みcommandを途中で破棄しない() {
    use crate::adaptor::controller::api;
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
            crate::client_api_acceptance::watcher(),
        )),
        None,
    )
    .0;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = crate::client_api_acceptance::connect_client(
        &crate::client_api_acceptance::ClientEndpoint {
            url: format!("http://{address}"),
            token: "client".into(),
            launch_id: String::new(),
        },
    );
    for _ in 0..64 {
        let request = client.get_language_from_path_with_options(
            crate::client_api_acceptance::rpc::GetLanguageFromPathRequest {
                file_path: Some("/repo".into()),
                ..Default::default()
            },
            connectrpc::client::CallOptions::default()
                .with_timeout(std::time::Duration::from_millis(20)),
        );
        let (_, result) = tokio::join!(started.notified(), request);
        assert!(result.is_err());
    }
    let error = client
        .get_language_from_path(
            crate::client_api_acceptance::rpc::GetLanguageFromPathRequest {
                file_path: Some("/repo".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, connectrpc::ErrorCode::ResourceExhausted);
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
    server.abort();
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

#[tokio::test]
async fn test_未呼出33command_connectの実行結果とエラーがtauriと一致する() {
    use crate::adaptor::controller::{api, state::AppState};

    // Given
    let (temp, path) = mutation_repository();
    let repo = git2::Repository::open(&path).unwrap();
    let path = repo.workdir().unwrap().to_string_lossy().into_owned();
    let file = std::path::Path::new(&path).join("日本語 space.txt");
    std::fs::write(&file, "committed\n").unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_path(std::path::Path::new("日本語 space.txt"))
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let signature = git2::Signature::now("test", "test@example.com").unwrap();
    let commit = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            "tracked file",
            &tree,
            &[&repo.head().unwrap().peel_to_commit().unwrap()],
        )
        .unwrap();
    repo.branch("main", &repo.find_commit(commit).unwrap(), false)
        .unwrap();
    std::fs::write(&file, "staged\n").unwrap();
    index
        .add_path(std::path::Path::new("日本語 space.txt"))
        .unwrap();
    index.write().unwrap();
    std::fs::write(&file, "working tree\n").unwrap();
    let worktree = temp.path().join("managed-worktree");
    repo.worktree("managed-worktree", &worktree, None).unwrap();
    let worktree = worktree.canonicalize().unwrap();
    let gateway = Arc::new(api::test_support::RecordingRuntimeGateway::default());
    let runtime = Arc::new(crate::usecase::workflow::WorkflowRuntimeUsecase::new(
        gateway.clone(),
    ));
    let (app, _data_dir, _store) =
        crate::adaptor::controller::client::workflow::tests::make_read_only_app();
    app.manage(runtime.clone());
    app.manage(Arc::new(ApplicationStartupAuthority::ready()));
    app.manage(Arc::new(
        crate::adaptor::controller::wiring::build_review_comment_usecase(),
    ));
    app.manage(Arc::new(
        crate::infrastructure::file_watcher::FileWatcherManager::default(),
    ));
    let config = app.state::<Arc<dyn crate::domain::app_config::ConfigRepository>>();
    let mut settings = config.load().unwrap();
    settings.app.last_repo_paths = vec![path.clone()];
    config.save(settings).unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut dispatch = ClientCommandDispatch::new(
        app.state::<AppState>().repository_usecase.clone(),
        app.state::<Arc<ApplicationStartupAuthority>>()
            .inner()
            .clone(),
    );
    dispatch.register_dependencies(&crate::desktop_test_support::build_client_dependencies(
        app.handle(),
    ));
    let dispatch = Arc::new(dispatch);
    app.manage(dispatch.clone());
    let router = api::test_support::test_router_with_optional_deps(
        data.path(),
        "master",
        "client",
        None,
        Some(api::ClientApiDeps::new(
            dispatch,
            crate::adaptor::gateway::push::ClientPushGateway::new(Arc::new(
                crate::infrastructure::push::PushSink::new(),
            )),
            crate::desktop_test_support::build_watcher_usecase(app.handle()),
        )),
        None,
    )
    .0;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = crate::client_api_acceptance::connect_client(
        &crate::client_api_acceptance::ClientEndpoint {
            url: format!("http://{address}"),
            token: "client".into(),
            launch_id: String::new(),
        },
    );

    let execution = "00000000-0000-4000-8000-000000000123";
    let cases = [
        ("get_crash_reporting_enabled", json!({}), true),
        (
            "get_file_at_ref",
            json!({"filePath":file,"gitRef":"HEAD"}),
            true,
        ),
        ("get_staged_content", json!({"filePath":file}), true),
        ("get_binary_staged_content", json!({"filePath":file}), true),
        ("get_file_at_branch_base", json!({"filePath":file}), true),
        (
            "get_binary_file_at_branch_base",
            json!({"filePath":file}),
            true,
        ),
        (
            "get_binary_file_at_ref",
            json!({"filePath":file,"gitRef":"HEAD"}),
            true,
        ),
        (
            "get_branch_diff_summary",
            json!({"repoPath":path,"baseBranch":"main"}),
            true,
        ),
        (
            "build_diff_file_tree",
            json!({"entries":[{"path":"src/日本語.rs","status":"modified","additions":3,"deletions":1}]}),
            true,
        ),
        (
            "get_head_diff_file_tree_snapshot",
            json!({"repoPath":path}),
            true,
        ),
        (
            "compute_hidden_ranges",
            json!({"hunks":[{"index":0,"oldStart":10,"oldLines":1,"newStart":10,"newLines":2,"lines":["-old","+new","+line"]}],"totalLines":30,"contextLines":2}),
            true,
        ),
        (
            "get_relative_path",
            json!({"rootPath":path,"filePath":file}),
            true,
        ),
        (
            "get_review_thread",
            json!({"worktreeName":"../invalid","threadId":"missing"}),
            false,
        ),
        (
            "get_review_thread_history",
            json!({"worktreeName":"../invalid","threadId":"missing"}),
            false,
        ),
        ("fetch_pr_status", json!({"repoPath":path}), true),
        ("get_default_branch", json!({"repoPath":path}), true),
        (
            "get_git_status",
            json!({"repoPath":path,"includeIgnored":true}),
            true,
        ),
        ("get_git_status_snapshot", json!({"repoPath":path}), true),
        ("get_status_diff_stats", json!({"repoPath":path}), true),
        (
            "get_status_diff_stats_snapshot",
            json!({"repoPath":path}),
            true,
        ),
        ("get_git_log", json!({"repoPath":path,"limit":1}), true),
        (
            "get_worktree_dirty_count",
            json!({"worktreePath":path}),
            true,
        ),
        ("get_repo_git_dir", json!({"filePath":file}), true),
        (
            "approve_workflow_node",
            json!({"args":{"executionId":execution,"nodeName":"review","nodeExecutionId":"ne-review-1","comment":"確認済み"}}),
            true,
        ),
        (
            "list_workflow_executions",
            json!({"worktreePath":worktree,"status":"active"}),
            true,
        ),
        (
            "get_workflow_execution",
            json!({"executionId":execution}),
            true,
        ),
        (
            "get_workflow_execution_log",
            json!({"worktreePath":worktree,"executionId":execution}),
            true,
        ),
        (
            "get_workflow_node_detail",
            json!({"worktreePath":worktree,"executionId":execution,"nodeExecutionId":"ne-review-1"}),
            true,
        ),
        (
            "resolve_worktree_by_execution",
            json!({"executionId":execution}),
            true,
        ),
        ("list_facets", json!({"kind":"instruction"}), true),
        (
            "workflow_submit_output",
            json!({"worktreePath":worktree,"nodeExecutionId":"missing-node","artifact":{"contract":"review-result","value":{"status":"approved"}}}),
            false,
        ),
        (
            "workflow_validate_output",
            json!({"worktreePath":worktree,"executionId":execution,"nodeName":"review","structuredOutput":{"status":"approved"}}),
            false,
        ),
        (
            "workflow_get_output",
            json!({"worktreePath":worktree,"executionId":execution,"nodeName":"review"}),
            false,
        ),
    ];
    assert_eq!(
        cases
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        33
    );
    // When / Then
    for (command, args, succeeds) in cases {
        let expected = invoke_tauri(&app, command, args.clone()).await;
        assert_eq!(expected.is_ok(), succeeds, "{command}: {expected:?}");
        if let Err(error) = &expected {
            assert!(error.is_string(), "usecase error: {command}: {error}");
        }
        let actual = crate::client_api_acceptance::request_client(&client, command, args)
            .await
            .map_err(|error| {
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
                    .decode(error.details[0].value.as_ref().unwrap())
                    .unwrap();
                wire::from_value(wire::CommandError::decode(bytes.as_slice()).unwrap()).unwrap()
            });
        assert_eq!(actual, expected, "{command}");
    }
    server.abort();
    let approvals = &gateway.commands.lock().unwrap().approvals;
    assert_eq!(approvals.len(), 2);
    assert_eq!(approvals[0], approvals[1]);
    assert_eq!(approvals[0].comment.as_deref(), Some("確認済み"));
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
    if ![
        "get_application_startup_outcome",
        "quit_after_startup_failure",
        "attach_terminal_surface",
    ]
    .contains(&command)
    {
        let dispatch = app.state::<Arc<ClientCommandDispatch>>();
        let request = wire::CommandRequest::from_value(command, args)
            .map_err(|error| json!({"code":"INVALID_REQUEST", "message":error}))?;
        return dispatch
            .dispatch(request.command.unwrap())
            .await
            .map(|command| {
                wire::from_value(wire::CommandResult {
                    command: Some(command),
                })
                .unwrap()
            })
            .map_err(|error| wire::from_value(error).unwrap());
    }
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

#[tokio::test]
async fn test_workspace保存_connectがui追加fieldを受理し既存項目を再起動後に復元する() {
    use crate::adaptor::controller::api;
    use crate::adaptor::gateway::workspace_state::WorkspaceStateStore;
    // Given
    let data = tempfile::tempdir().unwrap();
    let worktree = data.path().join("worktree");
    std::fs::create_dir_all(worktree.join("src")).unwrap();
    std::fs::write(worktree.join("src/main.rs"), "fn main() {}\n").unwrap();
    let state = json!({"version":1,"tabs":{"editors":[{"path":"src/main.rs","name":"main.rs"}],"activeEditorPath":"src/main.rs"},"layout":{"centerTab":"editor","activeView":"git","leftNavCollapsed":true,"rightCollapsed":true,"rightBottomCollapsed":false,"rightBottomActiveTab":"terminal","selectedDiffFile":"src/main.rs","reviewCollapsed":true,"diffOnlyMode":true}});
    let (app, _) = parity_app();
    let mut deps = crate::desktop_test_support::build_client_dependencies(app.handle());
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
            crate::desktop_test_support::build_watcher_usecase(app.handle()),
        )),
        None,
    )
    .0;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = crate::client_api_acceptance::connect_client(
        &crate::client_api_acceptance::ClientEndpoint {
            url: format!("http://{address}"),
            token: "client".into(),
            launch_id: String::new(),
        },
    );
    assert_eq!(
        crate::client_api_acceptance::request_client(
            &client,
            "save_workspace_state",
            json!({"worktreeName":"workspace","state":state})
        )
        .await
        .unwrap(),
        Value::Null
    );
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

#[tokio::test]
async fn test_生成要求_必須fieldと非有限数をusecase実行前に拒否する() {
    // Given
    let (_, dispatch) = parity_app();
    use wire::command_request::Command;
    // When / Then
    for request in [
        Command::GetCurrentBranch(wire::GetCurrentBranchRequest::default()),
        Command::SaveWorkspaceState(wire::SaveWorkspaceStateRequest {
            worktree_name: Some("workspace".into()),
            state: Some(wire::WorkspaceStateDto {
                version: Some(2),
                ..Default::default()
            }),
        }),
        Command::RecordTerminalLaunchRendererPhase(
            wire::RecordTerminalLaunchRendererPhaseRequest {
                phase: Some("ready".into()),
                duration_ms: Some(f64::NAN),
            },
        ),
    ] {
        let error = dispatch.dispatch(request).await.unwrap_err();
        assert_eq!(wire::from_value(error).unwrap()["code"], "INVALID_REQUEST");
    }
    for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(crate::adaptor::controller::client::finite(number).is_err());
    }
    assert_eq!(
        crate::adaptor::controller::client::finite(1.5).unwrap(),
        1.5
    );
}

#[tokio::test]
async fn test_登録希望_proto経由の一般設定保存から独立して永続化する() {
    // Given
    let (app, dispatch) = parity_app();
    let repository = app.state::<Arc<dyn crate::domain::app_config::ConfigRepository>>();
    let original = repository.load().unwrap();
    for requested in [true, false] {
        // When
        assert_parity(
            &dispatch,
            "update_login_item_preference",
            json!({"requested": requested}),
            Ok(Value::Null),
        )
        .await;
        assert_parity(
            &dispatch,
            "update_app_settings",
            json!({"app": {"close_to_tray": false, "start_minimized": true}}),
            Ok(Value::Null),
        )
        .await;
        // Then
        let mut expected = original.clone();
        expected.app.auto_launch = requested;
        expected.app.close_to_tray = false;
        expected.app.start_minimized = true;
        assert_eq!(repository.load().unwrap(), expected);
    }
    // When
    let error = dispatch
        .dispatch(wire::command_request::Command::UpdateLoginItemPreference(
            wire::UpdateLoginItemPreferenceRequest { requested: None },
        ))
        .await
        .unwrap_err();
    // Then
    assert_eq!(wire::from_value(error).unwrap()["code"], "INVALID_REQUEST");
    assert!(!repository.load().unwrap().app.auto_launch);
}

#[tokio::test]
async fn test_一般設定保存_必須入力の欠落ではどの設定も変更しない() {
    // Given
    let (app, dispatch) = parity_app();
    let repository = app.state::<Arc<dyn crate::domain::app_config::ConfigRepository>>();
    let original = repository.load().unwrap();
    for app in [
        None,
        Some(wire::WindowSettings {
            close_to_tray: None,
            start_minimized: Some(true),
        }),
        Some(wire::WindowSettings {
            close_to_tray: Some(false),
            start_minimized: None,
        }),
    ] {
        // When
        let error = dispatch
            .dispatch(wire::command_request::Command::UpdateAppSettings(
                wire::UpdateAppSettingsRequest { app },
            ))
            .await
            .unwrap_err();
        // Then
        assert_eq!(wire::from_value(error).unwrap()["code"], "INVALID_REQUEST");
        assert_eq!(repository.load().unwrap(), original);
    }
}

#[tokio::test(start_paused = true)]
async fn test_接続情報command_起動中の要求を拒否せず検証済みendpointを返す() {
    use crate::usecase::test_helpers::{tick, FakeDaemon};
    use std::sync::atomic::Ordering;
    // Given
    let (app, _) = parity_app();
    let gateway = Arc::new(FakeDaemon::default());
    let supervisor =
        crate::usecase::daemon_supervision::DaemonSupervisionUsecase::start(gateway.clone());
    app.manage(supervisor);
    for reconnect in [false, true] {
        if reconnect {
            gateway.ready.store(false, Ordering::SeqCst);
            tick(200).await;
        }
        // When
        let request = super::get_client_endpoint(app.state(), "desktop".into());
        tokio::pin!(request);
        assert!(futures_util::poll!(&mut request).is_pending());
        tick(200).await;
        assert!(futures_util::poll!(&mut request).is_pending());
        gateway.ready.store(true, Ordering::SeqCst);
        tick(200).await;
        // Then
        let endpoint = request.await.unwrap();
        assert_eq!(endpoint.endpoint.token, "client-only");
        assert_eq!(endpoint.launch_id, "launch");
    }
}

#[test]
fn test_通常要求_connect入口にはsupervisorの受付制御を登録しない() {
    // Given / When
    let commands = super::COMMAND_NAMES;
    // Then
    assert!(commands.contains(&"get_client_endpoint"));
    for removed in [
        "admit_client_command",
        "attach_desktop_client",
        "send_desktop_client_frame",
        "detach_desktop_client",
    ] {
        assert!(!commands.contains(&removed));
    }
}
