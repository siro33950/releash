fn value(result: impl serde::Serialize) -> Result<serde_json::Value, serde_json::Value> {
    serde_json::to_value(result).map_err(|error| serde_json::json!(error.to_string()))
}
use base64::Engine;

use releash_lib::test_support::integration::transport::ClientCommandDispatch;
use releash_lib::test_support::integration::wire;
use serde_json::Value;
use std::sync::Arc;

use prost::Message;
use serde_json::json;

fn parity_app() -> (ClientTestDependencies, Arc<ClientCommandDispatch>) {
    parity_app_with_runtime(None)
}

fn parity_app_with_runtime(
    runtime: Option<Arc<releash_lib::test_support::integration::workflow::WorkflowRuntimeUsecase>>,
) -> (ClientTestDependencies, Arc<ClientCommandDispatch>) {
    let (mut app, data_dir, store) = make_client_dependencies();
    let runtime = runtime.unwrap_or_else(|| {
        let gateway =
            crate::adaptor_controller_api_mod::test_support::RecordingRuntimeGateway::default();
        Arc::new(releash_lib::test_support::integration::workflow::WorkflowRuntimeUsecase::new(
            Arc::new(gateway),
            Arc::new(
                releash_lib::test_support::integration::workflow::ExecutionTreeArchiveFactRepository::new(
                    store,
                    data_dir.clone(),
                ),
            ),
        ))
    });
    app.client.workflow_runtime_usecase = Some(runtime);
    app.client.review_comment_usecase = Some(Arc::new(
        releash_lib::test_support::integration::platform::build_review_comment_usecase(
            releash_lib::test_support::integration::platform::build_review_context(&data_dir),
        ),
    ));
    app.client.workspace_state_store = Some(Arc::new(
        releash_lib::test_support::integration::platform::WorkspaceStateStore::new(data_dir),
    ));
    use releash_lib::test_support::integration::sessions::FakeProviderExecutableConfigRepository;
    use releash_lib::test_support::integration::sessions::FakeProviderExecutableProbeGateway;
    app.client.provider_availability_usecase = Some(Arc::new(
        releash_lib::test_support::integration::sessions::ProviderAvailabilityUsecase::initialize(
            Arc::new(FakeProviderExecutableConfigRepository::default()),
            Arc::new(FakeProviderExecutableProbeGateway::default()),
        )
        .unwrap(),
    ));
    let mut dispatch = ClientCommandDispatch::new(
        releash_lib::test_support::integration::daemon::DaemonUsecase::test_with_repository(
            releash_lib::test_support::integration::daemon::serving(),
        ),
    );
    dispatch.register_dependencies(&app.client);
    let dispatch = Arc::new(dispatch);
    app.dispatch = Some(dispatch.clone());
    (app, dispatch)
}

async fn assert_parity(
    dispatch: &ClientCommandDispatch,
    command: &str,
    args: Value,
    expected: Result<Value, Value>,
) {
    let payload = wire::command_request_from_value(command, args).unwrap();
    let request = wire::CommandRequest::decode(payload.encode_to_vec().as_slice()).unwrap();
    let actual = match dispatch.dispatch(request.command.unwrap()).await {
        Ok(result) => Ok(
            releash_lib::test_support::integration::transport::from_value(wire::CommandResult {
                command: Some(result),
            })
            .unwrap(),
        ),
        Err(error) => {
            Err(releash_lib::test_support::integration::transport::from_value(error).unwrap())
        }
    };
    assert_eq!(actual, expected, "{command}");
}

#[tokio::test]
pub async fn test_telemetry_protoはcommand結果と一致する() {
    // Given
    let _guard = releash_lib::test_support::integration::telemetry::lock_test_telemetry();
    let (app, dispatch) = parity_app();
    // When / Then
    invoke_dispatch(&app, "report_mounted_xterm_count", json!({"count": 2}))
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
pub async fn test_クライアントdispatch_proto全commandの登録と引数検証() {
    // Given
    let (_app, dispatch) = parity_app();
    // When / Then
    assert!(wire::COMMAND_NAMES.contains(&"find_repository_root"));
    assert!(wire::COMMAND_NAMES.contains(&"refresh_workspaces"));
    for command in wire::COMMAND_NAMES {
        assert!(dispatch.contains(command), "{command}");
    }
    for args in [json!({}), json!({"filePath":9})] {
        let error = invoke_dispatch(&_app, "get_language_from_path", args)
            .await
            .unwrap_err();
        assert_eq!(error["code"], "INVALID_REQUEST");
    }
}

#[tokio::test]
pub async fn test_クライアントrpc_期限切れで処理を止め要求枠を再利用できる() {
    // Given
    let mut dispatch = ClientCommandDispatch::new(
        releash_lib::test_support::integration::daemon::DaemonUsecase::test_with_repository(
            releash_lib::test_support::integration::daemon::serving(),
        ),
    );
    let started = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Semaphore::new(0));
    let completed = Arc::new(tokio::sync::Semaphore::new(0));
    let active = Arc::new(tokio::sync::Semaphore::new(1));
    let notifications = (
        started.clone(),
        resume.clone(),
        completed.clone(),
        active.clone(),
    );
    dispatch.register_domain(
        &["get_language_from_path"],
        Box::new(move |_| {
            let (started, resume, completed, active) = notifications.clone();
            Box::pin(async move {
                let _active = active.acquire_owned().await.unwrap();
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
    let router = crate::adaptor_controller_api_mod::test_support::test_router_with_optional_deps(
        data.path(),
        "master",
        "client",
        Some(
            releash_lib::test_support::integration::transport::client_api_deps(Arc::new(dispatch)),
        ),
        None,
    )
    .0;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = releash_lib::test_support::integration::transport::connect_client(
        &releash_lib::test_support::integration::transport::ClientEndpoint {
            url: format!("http://{address}"),
            token: "client".into(),
            launch_id: String::new(),
        },
    );
    for _ in 0..64 {
        let request = client.get_language_from_path_with_options(
            releash_lib::test_support::integration::transport::rpc::GetLanguageFromPathRequest {
                file_path: Some("/repo".into()),
                ..Default::default()
            },
            connectrpc::client::CallOptions::default()
                .with_timeout(std::time::Duration::from_millis(20)),
        );
        let (_, result) = tokio::join!(started.notified(), request);
        assert_eq!(
            result.unwrap_err().code,
            connectrpc::ErrorCode::DeadlineExceeded
        );
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(1), active.acquire())
            .await
            .unwrap()
            .unwrap();
        drop(stopped);
    }
    // When
    resume.add_permits(1);
    let result = client
        .get_language_from_path_with_options(
            releash_lib::test_support::integration::transport::rpc::GetLanguageFromPathRequest {
                file_path: Some("/repo".into()),
                ..Default::default()
            },
            connectrpc::client::CallOptions::default()
                .with_timeout(std::time::Duration::from_secs(1)),
        )
        .await
        .unwrap();
    // Then
    assert_eq!(result.into_owned().value.as_deref(), Some("done"));
    assert_eq!(completed.available_permits(), 1);
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
pub async fn test_計算と操作command_connectの実行結果とエラーがdispatchと一致する() {
    // Given
    let (_temp, path) = mutation_repository();
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
    let gateway = Arc::new(
        crate::adaptor_controller_api_mod::test_support::RecordingRuntimeGateway::default(),
    );
    let runtime = Arc::new(
        releash_lib::test_support::integration::workflow::WorkflowRuntimeUsecase::new(
            gateway.clone(),
            Arc::new(releash_lib::test_support::integration::workflow::NoopArchiveRepository),
        ),
    );
    let (mut app, data_dir, _store) = make_client_dependencies();
    app.client.workflow_runtime_usecase = Some(runtime.clone());
    app.client.review_comment_usecase = Some(Arc::new(
        releash_lib::test_support::integration::platform::build_review_comment_usecase(
            releash_lib::test_support::integration::platform::build_review_context(&data_dir),
        ),
    ));
    let config = app.client.config_repository.as_ref().unwrap();
    let mut settings = config.load().unwrap();
    settings.app.last_repo_paths = vec![path.clone()];
    config.save(settings).unwrap();
    let data = tempfile::tempdir().unwrap();
    let mut dispatch = ClientCommandDispatch::new(
        releash_lib::test_support::integration::daemon::DaemonUsecase::test_with_repository(
            releash_lib::test_support::integration::daemon::serving(),
        ),
    );
    dispatch.register_dependencies(&app.client);
    let dispatch = Arc::new(dispatch);
    app.dispatch = Some(dispatch.clone());
    let router = crate::adaptor_controller_api_mod::test_support::test_router_with_optional_deps(
        data.path(),
        "master",
        "client",
        Some(releash_lib::test_support::integration::transport::client_api_deps(dispatch)),
        None,
    )
    .0;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = releash_lib::test_support::integration::transport::connect_client(
        &releash_lib::test_support::integration::transport::ClientEndpoint {
            url: format!("http://{address}"),
            token: "client".into(),
            launch_id: String::new(),
        },
    );

    let execution = "00000000-0000-4000-8000-000000000123";
    let cases = [
        (
            "build_diff_file_tree",
            json!({"entries":[{"path":"src/日本語.rs","status":"modified","additions":3,"deletions":1}]}),
            true,
        ),
        (
            "compute_hidden_ranges",
            json!({"hunks":[{"index":0,"oldStart":10,"oldLines":1,"newStart":10,"newLines":2,"lines":["-old","+new","+line"]}],"totalLines":30,"contextLines":2}),
            true,
        ),
        (
            "approve_workflow_node",
            json!({"args":{"executionId":execution,"nodeName":"review","nodeExecutionId":"ne-review-1","comment":"確認済み"}}),
            true,
        ),
        (
            "workflow_submit_output",
            json!({"nodeExecutionId":"missing-node","artifact":{"contract":"review-result","value":{"status":"approved"}}}),
            false,
        ),
    ];
    assert_eq!(
        cases
            .iter()
            .map(|(name, _, _)| *name)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        4
    );
    // When / Then
    for (command, args, succeeds) in cases {
        let expected = invoke_dispatch(&app, command, args.clone()).await;
        assert_eq!(expected.is_ok(), succeeds, "{command}: {expected:?}");
        if let Err(error) = &expected {
            assert!(error.is_string(), "usecase error: {command}: {error}");
        }
        let actual = releash_lib::test_support::integration::transport::request_client(
            &client, command, args,
        )
        .await
        .map_err(|error| {
            let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(error.details[0].value.as_ref().unwrap())
                .unwrap();
            releash_lib::test_support::integration::transport::from_value(
                wire::CommandError::decode(bytes.as_slice()).unwrap(),
            )
            .unwrap()
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
pub async fn test_worktree変更_protoは実引数の成功とusecaseエラーを保持する() {
    async fn wait_for_deletion(
        runtime: &releash_lib::test_support::integration::workflow::WorkflowRuntimeUsecase,
        path: &str,
    ) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while runtime.begin_worktree_mutation(path).is_err() {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
    }
    // Given
    let (_temp, path) = mutation_repository();
    let (app, dispatch) = parity_app();
    let uc = app
        .client
        .app_state
        .as_ref()
        .unwrap()
        .repository_usecase
        .clone();
    let branch = "feature/parity";
    let expected = invoke_dispatch(&app, "create_worktree", json!({"repoPath": path.clone(),"branch": branch,"createBranch": true,"baseBranch": Some("base")}))
    .await
    .unwrap();
    let worktree_path = expected.as_str().unwrap().to_owned();
    let runtime = app.client.workflow_runtime_usecase.as_ref().unwrap();
    uc.remove_worktree(runtime.as_ref(), &path, &worktree_path, true)
        .await
        .unwrap();
    wait_for_deletion(runtime.as_ref(), &worktree_path).await;
    git2::Repository::open(&path)
        .unwrap()
        .find_branch(branch, git2::BranchType::Local)
        .unwrap()
        .delete()
        .unwrap();
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
        invoke_dispatch(&app, "create_worktree", json!({"repoPath": path.clone(),"branch": "invalid-base","createBranch": true,"baseBranch": Some("missing-base")}))
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
        invoke_dispatch(
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
        invoke_dispatch(
            &app,
            "remove_worktree",
            json!({"repoPath": path.clone(),"worktreePath": worktree_path.clone(),"force": true}),
        )
        .await,
    );
    assert!(expected.is_ok());
    wait_for_deletion(runtime.as_ref(), &worktree_path).await;
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
    wait_for_deletion(runtime.as_ref(), &worktree_path).await;
    assert!(!std::path::Path::new(&worktree_path).exists());
}

#[tokio::test]
pub async fn test_staging変更_protoは複数pathとusecaseエラーを保持する() {
    // Given
    let (_temp, path) = mutation_repository();
    let (app, dispatch) = parity_app();
    let paths = vec!["space name.txt".to_string(), "日本語.txt".to_string()];
    for name in &paths {
        std::fs::write(std::path::Path::new(&path).join(name), name).unwrap();
    }
    let uc = app.client.app_state.as_ref().unwrap().code_usecase.clone();
    let expected = outcome(
        invoke_dispatch(
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
        invoke_dispatch(
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
                invoke_dispatch(
                    &app,
                    "git_stage",
                    json!({"repoPath": missing.clone(),"paths": paths.clone()}),
                )
                .await,
            )
        } else {
            outcome(
                invoke_dispatch(
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
pub async fn test_review_group変更_protoは複合引数と部分stagingとusecaseエラーを保持する() {
    use releash_lib::test_support::integration::platform::ReviewFileViewDto;

    for (command, section) in [
        ("git_stage_review_group", "changes"),
        ("git_unstage_review_group", "staged"),
    ] {
        // Given
        let (_temp, path) = mutation_repository();
        let (app, dispatch) = parity_app();
        let state = app.client.app_state.as_ref().unwrap();
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
            .get_review_file_view(&path, file, section, "head")
            .unwrap()
        else {
            panic!("text diff");
        };
        assert_eq!(view.change_groups.len(), 2);
        let input = json!({"worktreePath":path,"path":file,"section":section,"base":"head","groupId":view.change_groups[1].group_id});
        let app_ref = &app;
        let call = |input: Value| async move {
            invoke_dispatch(app_ref, command, json!({"input": input})).await
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
pub async fn test_workflow変更_protoは実引数とruntime結果を保持する() {
    // Given
    use crate::adaptor_controller_api_mod::test_support::RecordingRuntimeGateway;
    use releash_lib::test_support::integration::workflow::WorkflowRuntimeUsecase;
    let gateway = Arc::new(RecordingRuntimeGateway::default());
    let runtime = Arc::new(WorkflowRuntimeUsecase::new(
        gateway.clone(),
        Arc::new(releash_lib::test_support::integration::workflow::NoopArchiveRepository),
    ));
    let (app, dispatch) = parity_app_with_runtime(Some(runtime));
    let id = "00000000-0000-4000-8000-000000000001";
    for failure in [false, true] {
        if failure {
            let mut errors = gateway.errors.lock().unwrap();
            errors.start = Some(
                releash_lib::test_support::integration::workflow::WorkflowError::external(
                    "start failed",
                ),
            );
            errors.abort = Some(
                releash_lib::test_support::integration::workflow::WorkflowError::external(
                    "abort failed",
                ),
            );
        }
        // When / Then
        let expected = outcome(
            invoke_dispatch(&app, "start_workflow", json!({"workflowName": "workflow-name","worktreePath": "/workspace with space","request": Some("日本語の依頼"),"createdFrom": Some("cli")}))
            .await,
        );
        assert_eq!(expected.is_err(), failure);
        assert_parity(&dispatch, "start_workflow", json!({"workflowName":"workflow-name","worktreePath":"/workspace with space","request":"日本語の依頼","createdFrom":"cli"}), expected).await;
        let expected =
            outcome(invoke_dispatch(&app, "abort_workflow", json!({"executionId": id})).await);
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
        releash_lib::test_support::integration::workflow::ExecutionOrigin::Cli
    );
    assert_eq!(commands.aborts.len(), 2);
    assert_eq!(commands.aborts[0], commands.aborts[1]);
}

fn outcome<T: serde::Serialize, E: serde::Serialize>(result: Result<T, E>) -> Result<Value, Value> {
    result
        .map_err(|error| serde_json::to_value(error).unwrap())
        .and_then(value)
}

async fn invoke_dispatch(
    app: &ClientTestDependencies,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    let dispatch = app.dispatch.as_ref().unwrap();
    let request = wire::command_request_from_value(command, args)
        .map_err(|error| json!({"code":"INVALID_REQUEST", "message":error}))?;
    dispatch
        .dispatch(request.command.unwrap())
        .await
        .map(|command| {
            releash_lib::test_support::integration::transport::from_value(wire::CommandResult {
                command: Some(command),
            })
            .unwrap()
        })
        .map_err(|error| {
            releash_lib::test_support::integration::transport::from_value(error).unwrap()
        })
}

#[tokio::test]
pub async fn test_workspace保存_connectがui追加fieldを受理し既存項目を再起動後に復元する() {
    // Given

    use releash_lib::test_support::integration::platform::WorkspaceStateStore;
    let data = tempfile::tempdir().unwrap();
    let worktree = data.path().join("worktree");
    std::fs::create_dir_all(worktree.join("src")).unwrap();
    std::fs::write(worktree.join("src/main.rs"), "fn main() {}\n").unwrap();
    let state = json!({"version":1,"tabs":{"editors":[{"path":"src/main.rs","name":"main.rs"}],"activeEditorPath":"src/main.rs"},"layout":{"centerTab":"editor","activeView":"git","leftNavCollapsed":true,"rightCollapsed":true,"rightBottomCollapsed":false,"rightBottomActiveTab":"terminal","selectedDiffFile":"src/main.rs","reviewCollapsed":true,"diffOnlyMode":true}});
    let (app, _) = parity_app();
    let mut deps = app.client;
    deps.workspace_state_store = Some(Arc::new(WorkspaceStateStore::new(data.path().to_owned())));
    let mut dispatch = ClientCommandDispatch::new(
        releash_lib::test_support::integration::daemon::DaemonUsecase::test_with_repository(
            releash_lib::test_support::integration::daemon::serving(),
        ),
    );
    dispatch.register_dependencies(&deps);
    let router = crate::adaptor_controller_api_mod::test_support::test_router_with_optional_deps(
        data.path(),
        "master",
        "client",
        Some(
            releash_lib::test_support::integration::transport::client_api_deps(Arc::new(dispatch)),
        ),
        None,
    )
    .0;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        // When
        axum::serve(listener, router).await.unwrap();
    });
    let client = releash_lib::test_support::integration::transport::connect_client(
        &releash_lib::test_support::integration::transport::ClientEndpoint {
            url: format!("http://{address}"),
            token: "client".into(),
            launch_id: String::new(),
        },
    );
    // Then
    assert_eq!(
        releash_lib::test_support::integration::transport::request_client(
            &client,
            "save_workspace_state",
            json!({"worktreeName":"workspace","state":state})
        )
        .await
        .unwrap(),
        Value::Null
    );
    server.abort();
    let restarted = WorkspaceStateStore::new(data.path().to_owned());
    let mut expected = state;
    expected["layout"]
        .as_object_mut()
        .unwrap()
        .remove("reviewCollapsed");
    expected["layout"]
        .as_object_mut()
        .unwrap()
        .remove("diffOnlyMode");
    use releash_lib::test_support::integration::repository::WorkspaceStateRepository;
    let restored = restarted
        .load("workspace", worktree.to_str().unwrap())
        .unwrap()
        .unwrap();
    let restored =
        releash_lib::test_support::integration::platform::WorkspaceStateDto::from(restored);
    assert_eq!(serde_json::to_value(restored).unwrap(), expected);
    let persisted: Value = serde_json::from_slice(
        &std::fs::read(data.path().join("workspace_state/workspace.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted, expected);
}

#[tokio::test]
pub async fn test_生成要求_必須fieldと非有限数をusecase実行前に拒否する() {
    // Given
    let (_, dispatch) = parity_app();
    use wire::command_request::Command;
    // When / Then
    for request in [
        Command::BuildDiffFileTree(wire::BuildDiffFileTreeRequest::default()),
        Command::SaveWorkspaceState(wire::SaveWorkspaceStateRequest {
            worktree_name: Some("workspace".into()),
            state: Some(wire::WorkspaceStateDto {
                version: Some(2),
                ..Default::default()
            }),
        }),
    ] {
        let error = dispatch.dispatch(request).await.unwrap_err();
        assert_eq!(
            releash_lib::test_support::integration::transport::from_value(error).unwrap()["code"],
            "INVALID_REQUEST"
        );
    }
}

#[tokio::test]
pub async fn test_登録希望_proto経由の一般設定保存から独立して永続化する() {
    // Given
    let (app, dispatch) = parity_app();
    let repository = app.client.config_repository.as_ref().unwrap();
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
    assert_eq!(
        releash_lib::test_support::integration::transport::from_value(error).unwrap()["code"],
        "INVALID_REQUEST"
    );
    assert!(!repository.load().unwrap().app.auto_launch);
}

#[tokio::test]
pub async fn test_一般設定保存_必須入力の欠落ではどの設定も変更しない() {
    // Given
    let (app, dispatch) = parity_app();
    let repository = app.client.config_repository.as_ref().unwrap();
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
        assert_eq!(
            releash_lib::test_support::integration::transport::from_value(error).unwrap()["code"],
            "INVALID_REQUEST"
        );
        assert_eq!(repository.load().unwrap(), original);
    }
}

struct ClientTestDependencies {
    client: releash_lib::test_support::integration::transport::ClientDependencies,
    dispatch: Option<Arc<ClientCommandDispatch>>,
}
fn make_client_dependencies() -> (
    ClientTestDependencies,
    std::path::PathBuf,
    Arc<releash_lib::test_support::integration::persistence::LocalEventStore>,
) {
    let (dependencies, data_dir, store) =
        crate::adaptor_controller_client_workflow_mod::tests::make_read_only_app();
    (
        ClientTestDependencies {
            client: dependencies.client,
            dispatch: None,
        },
        data_dir,
        store,
    )
}
