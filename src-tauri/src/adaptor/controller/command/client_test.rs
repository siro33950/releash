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
        crate::adaptor::controller::command::workflow::tests::make_read_only_app();
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
