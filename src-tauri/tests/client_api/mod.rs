use futures_util::StreamExt;
use releash_lib::client_api_acceptance::*;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Listener;

struct ClientConnection {
    client: NativeClient,
    push: std::pin::Pin<
        Box<dyn futures_util::Stream<Item = Result<rpc::Push, connectrpc::ConnectError>> + Send>,
    >,
}
struct Fixture {
    host: ClientApiAcceptanceHost<tauri::test::MockRuntime>,
    url: String,
    token: Arc<str>,
    repo: tempfile::TempDir,
    _data: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_branch(Arc::new(BranchGateway)).await
    }

    async fn with_branch(branch: Arc<dyn BranchRepository>) -> Self {
        let data = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let git = git2::Repository::init(repo.path()).unwrap();
        let tree_id = git.index().unwrap().write_tree().unwrap();
        let tree = git.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("test", "test@example.com").unwrap();
        git.commit(
            Some("refs/heads/ws-branch"),
            &signature,
            &signature,
            "initial",
            &tree,
            &[],
        )
        .unwrap();
        git.set_head("refs/heads/ws-branch").unwrap();
        let host = ClientApiAcceptanceHost::start(tauri::test::mock_builder(), data.path(), branch);
        let endpoint = host.endpoint();
        assert_ne!(
            format!("releash-bearer.{}", endpoint.token),
            host.master_subprotocol
        );
        let token = Arc::from(endpoint.token);
        Self {
            host,
            url: endpoint.url,
            token,
            repo,
            _data: data,
        }
    }

    fn client(&self) -> NativeClient {
        connect_client(&ClientEndpoint {
            url: self.url.clone(),
            token: self.token.to_string(),
            launch_id: String::new(),
        })
    }
    async fn connect(&self) -> ClientConnection {
        let client = self.client();
        let mut stream = client
            .subscribe_push(rpc::SubscribePushRequest::default())
            .await
            .unwrap();
        let initial = stream
            .message::<rpc::Push>()
            .await
            .unwrap()
            .unwrap()
            .to_owned_message();
        assert!(matches!(initial.event, Some(rpc::push::Event::Resync(_))));
        let push = Box::pin(futures_util::stream::unfold(
            stream,
            |mut stream| async move {
                match stream.message::<rpc::Push>().await {
                    Ok(Some(value)) => Some((Ok(value.to_owned_message()), stream)),
                    Ok(None) => None,
                    Err(error) => Some((Err(error), stream)),
                }
            },
        ));
        ClientConnection { client, push }
    }

    fn args(&self) -> Value {
        json!({"repoPath":self.repo.path().to_str().unwrap()})
    }
}

async fn receive(connection: &mut ClientConnection) -> Value {
    let push = tokio::time::timeout(Duration::from_secs(5), connection.push.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let (event, payload) = decode_rpc_push(push);
    json!({"status":"push", "event":event, "payload":payload})
}
async fn request(connection: &mut ClientConnection, frame: Value) -> Value {
    let result = request_client(
        &connection.client,
        frame["command"].as_str().unwrap(),
        frame.get("args").cloned().unwrap_or(json!({})),
    )
    .await
    .unwrap();
    json!({"request_id":frame["request_id"],"result":result})
}

#[tokio::test(flavor = "multi_thread")]
async fn test_クライアントconnect_認証と相関を保ちtauri経路を拒否する() {
    // Given
    let fixture = Fixture::new().await;
    for (token, origin, status) in [
        ("wrong", "tauri://localhost", 401),
        (fixture.token.as_ref(), "https://evil.example", 403),
    ] {
        let response = reqwest::Client::new()
            .post(format!(
                "{}/releash.client.v1.ClientService/GetRepoPaths",
                fixture.url
            ))
            .bearer_auth(token)
            .header("origin", origin)
            .header("content-type", "application/proto")
            .body(Vec::new())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
    }
    let mut socket = fixture.connect().await;
    // When
    let response = request(
        &mut socket,
        json!({"request_id":"one","command":"get_current_branch","args":fixture.args()}),
    )
    .await;
    let window = tauri::WebviewWindowBuilder::new(&fixture.host.app, "main", Default::default())
        .build()
        .unwrap();
    let invoke = tauri::webview::InvokeRequest {
        cmd: "get_current_branch".into(),
        callback: tauri::ipc::CallbackFn(0),
        error: tauri::ipc::CallbackFn(1),
        url: "tauri://localhost".parse().unwrap(),
        body: tauri::ipc::InvokeBody::Json(fixture.args()),
        headers: Default::default(),
        invoke_key: tauri::test::INVOKE_KEY.into(),
    };
    assert!(tauri::test::get_ipc_response(&window, invoke).is_err());
    assert_eq!(response["request_id"], "one");
    assert_eq!(response["result"], "ws-branch");
    drop(socket);
}

#[tokio::test]
async fn test_クライアント認証_wsで有効な非master_tokenはhttp入口で拒否する() {
    // Given
    let fixture = Fixture::new().await;
    let socket = fixture.connect().await;
    let mut url = url::Url::parse(&fixture.url).unwrap();
    url.set_scheme("http").unwrap();
    url.set_path("/v1/workflows");
    let master = fixture
        .host
        .master_subprotocol
        .strip_prefix(TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX)
        .unwrap();
    let client = reqwest::Client::new();
    // When / Then
    for (token, expected) in [
        (fixture.token.as_ref(), reqwest::StatusCode::UNAUTHORIZED),
        (master, reqwest::StatusCode::OK),
    ] {
        let response = client
            .get(url.clone())
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    drop(socket);
}

#[tokio::test]
async fn test_terminal接続情報_削除済みcommandはtauri_invokeでエラーになる() {
    // Given
    let fixture = Fixture::new().await;
    let window = tauri::WebviewWindowBuilder::new(&fixture.host.app, "main", Default::default())
        .build()
        .unwrap();
    // When
    let result = tauri::test::get_ipc_response(
        &window,
        tauri::webview::InvokeRequest {
            cmd: "get_terminal_stream_endpoint".into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(json!({})),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        },
    );
    // Then
    assert_eq!(
        result.unwrap_err(),
        json!("Command get_terminal_stream_endpoint not found")
    );
}

#[tokio::test]
async fn test_レビューコメント監視_events_json変更がconnectだけへ届く() {
    // Given
    let fixture = Fixture::new().await;
    let mut socket = fixture.connect().await;
    let received = Arc::new(std::sync::Mutex::new(Vec::new()));
    let listener_received = received.clone();
    fixture
        .host
        .app
        .listen("review-comments-changed", move |event| {
            listener_received
                .lock()
                .unwrap()
                .push(serde_json::from_str::<Value>(event.payload()).unwrap());
        });
    let dir = fixture._data.path().join("review-comments");
    spawn_review_comments_watcher(dir.clone(), fixture.host.review_comment_notifier());
    tokio::time::sleep(Duration::from_millis(100)).await;

    // When
    std::fs::write(dir.join("review.events.json"), b"[]").unwrap();

    // Then
    assert_eq!(
        receive(&mut socket).await,
        json!({"status": "push", "event": "review-comments-changed", "payload": "*"})
    );
    assert!(received.lock().unwrap().is_empty());
    drop(socket);
}

#[tokio::test]
async fn test_クライアントconnect_失敗と不正引数は構造化エラーになる() {
    let fixture = Fixture::new().await;
    let client = fixture.client();
    for args in [json!({}), json!({"repoPath":"/missing-releash-repository"})] {
        let error = client
            .get_current_branch(rpc::GetCurrentBranchRequest {
                repo_path: args["repoPath"].as_str().map(String::from),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, connectrpc::ErrorCode::FailedPrecondition);
        assert_eq!(error.details[0].type_url, "releash.client.v1.CommandError");
    }
}
fn workflow_payload() -> WorkflowExecutionChangedPayloadView {
    WorkflowExecutionChangedPayloadView {
        worktree_path: "/repo".into(),
        workflow_execution: WorkflowExecutionView {
            id: "execution-1".into(),
            workflow_name: "review".into(),
            status: ExecutionStatusView::Running,
            current_node: None,
            worktree_path: "/repo".into(),
            created_from: ExecutionOriginView::Cli,
            started_at: 1.0,
            updated_at: 2.0,
            completed_at: None,
            error_reason: None,
            total_token_usage: Default::default(),
            node_executions: vec![],
            artifacts: vec![],
            fanouts: vec![],
            approval_target: None,
        },
    }
}

#[tokio::test]
async fn test_backend通知_8イベントがconnectだけへ届く() {
    // Given
    let fixture = Fixture::new().await;
    let mut socket = fixture.connect().await;
    let received = Arc::new(std::sync::Mutex::new(Vec::new()));
    let events = [
        "agent-session-changed",
        "branch-list-sync",
        "file-change",
        "git-status-changed",
        "repo-paths-changed",
        "repository-snapshot-changed",
        "review-comments-changed",
        "workflow-execution-changed",
    ];
    for event in events {
        let received = received.clone();
        fixture.host.app.listen(event, move |message| {
            received
                .lock()
                .unwrap()
                .push(serde_json::from_str::<Value>(message.payload()).unwrap())
        });
    }
    let file = FileChangeEvent {
        watcher_id: 1,
        path: "/repo/file".into(),
        kind: "change".into(),
    };
    let git = GitStatusChangedEvent {
        repo_path: "/repo".into(),
    };
    let snapshot = RepositorySnapshotChangedEvent {
        worktree_path: "/repo".into(),
        version: 2,
        stale: false,
        loading: false,
        limited: false,
    };
    let paths = vec!["/repo".into()];
    let workflow = workflow_payload();
    // When
    let pushes = [
        BackendPush::AgentSessionChanged(AgentSessionChangedPayload {
            worktree_path: "/repo",
        }),
        BackendPush::BranchListSync,
        BackendPush::FileChange(file),
        BackendPush::GitStatusChanged(git),
        BackendPush::RepoPathsChanged(&paths),
        BackendPush::RepositorySnapshotChanged(snapshot),
        BackendPush::ReviewCommentsChanged("*"),
        BackendPush::WorkflowExecutionChanged(Box::new(workflow.clone())),
    ];
    for (index, push) in pushes.into_iter().enumerate() {
        fixture.host.emit(push);
        let frame = receive(&mut socket).await;
        // Then
        assert_eq!(frame["status"], "push");
        assert_eq!(frame["event"], events[index]);
        let expected = [
            json!({"worktreePath":"/repo"}),
            Value::Null,
            json!({"watcher_id":1,"path":"/repo/file","kind":"change"}),
            json!({"repo_path":"/repo"}),
            json!(["/repo"]),
            json!({"worktree_path":"/repo","version":2,"stale":false,"loading":false,"limited":false}),
            json!("*"),
            serde_json::to_value(&workflow).unwrap(),
        ];
        assert_eq!(frame["payload"], expected[index]);
        assert!(received.lock().unwrap().is_empty());
    }

    drop(socket);
}

#[tokio::test]
#[ignore = "local Connect latency measurement"]
async fn test_クライアントconnect_往復レイテンシ実測() {
    let fixture = Fixture::new().await;
    let mut socket = fixture.connect().await;
    let mut samples = Vec::new();
    for index in 0..1100 {
        let frame = json!({"request_id":index.to_string(),"command":"get_current_branch","args":fixture.args()});
        let start = Instant::now();
        let response = request(&mut socket, frame).await;
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(response["result"], "ws-branch");
        if index >= 100 {
            samples.push(elapsed);
        }
    }
    samples.sort_by(f64::total_cmp);
    assert!(samples[949] <= 1.0, "p95 exceeds 1ms: {}", samples[949]);
    assert!(samples[989] <= 2.0, "p99 exceeds 2ms: {}", samples[989]);
    println!("client-ws get_current_branch n={} warmup=100 min_ms={:.6} median_ms={:.6} p95_ms={:.6} p99_ms={:.6} max_ms={:.6} mean_ms={:.6}", samples.len(), samples[0], samples[499], samples[949], samples[989], samples[999], samples.iter().sum::<f64>() / samples.len() as f64);
    drop(socket);
}

#[tokio::test]
async fn test_workflow状態通知_更新済みsnapshotがtypedなconnect_pushになる() {
    // Given
    let fixture = Fixture::new().await;
    let mut socket = fixture.connect().await;
    // When
    fixture
        .host
        .emit_completed_workflow("execution-state", "/repo", 3.0)
        .await;
    // Then
    let frame = receive(&mut socket).await;
    assert_eq!(frame["event"], "workflow-execution-changed");
    let payload: WorkflowExecutionChangedPayloadView =
        serde_json::from_value(frame["payload"].clone()).unwrap();
    assert_eq!(payload.worktree_path, "/repo");
    assert_eq!(payload.workflow_execution.id, "execution-state");
    assert_eq!(
        payload.workflow_execution.status,
        ExecutionStatusView::Completed
    );
    assert_eq!(payload.workflow_execution.updated_at, 3.0);
    drop(socket);
}

#[tokio::test]
async fn test_ui_shell通知_wsのpush_sinkを経由しない() {
    // Given
    use tauri::Emitter;
    let fixture = Fixture::new().await;
    let mut push = fixture.host.subscribe_push();
    // When
    fixture.host.app.emit("menu-event", "test").unwrap();
    fixture.host.app.emit("native-file-drop", "test").unwrap();
    // Then
    assert!(matches!(
        push.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn test_push_購読数上限と切断後の解放() {
    let fixture = Fixture::new().await;
    let mut connections = Vec::new();
    for _ in 0..16 {
        connections.push(fixture.connect().await);
    }
    let client = fixture.client();
    let mut excess = client
        .subscribe_push(rpc::SubscribePushRequest::default())
        .await
        .unwrap();
    assert_eq!(
        excess.message::<rpc::Push>().await.unwrap_err().code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    connections.clear();
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.host.push_subscription_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("disconnected push subscriptions must release all slots");
    for _ in 0..16 {
        connections.push(fixture.connect().await);
    }
}

#[tokio::test]
async fn test_push_欠落時は再同期通知後も同じ購読を使える() {
    let fixture = Fixture::new().await;
    let mut socket = fixture.connect().await;
    for _ in 0..65 {
        fixture.host.emit(BackendPush::BranchListSync);
    }
    assert_eq!(receive(&mut socket).await["event"], "resync");
    assert_eq!(
        request_client(&socket.client, "get_current_branch", fixture.args())
            .await
            .unwrap(),
        "ws-branch"
    );
    fixture.host.emit(BackendPush::BranchListSync);
    assert_eq!(receive(&mut socket).await["event"], "branch-list-sync");
}

#[tokio::test]
async fn test_connect_不正protoと旧ws_routeを拒否する() {
    let fixture = Fixture::new().await;
    let http = reqwest::Client::new();
    let response = http
        .post(format!(
            "{}/releash.client.v1.ClientService/GetRepoPaths",
            fixture.url
        ))
        .bearer_auth(&*fixture.token)
        .header("origin", "tauri://localhost")
        .header("content-type", "application/proto")
        .body(vec![1u8])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    for path in ["/v1/client", "/v1/terminal"] {
        assert_eq!(
            http.get(format!("{}{path}", fixture.url))
                .bearer_auth(
                    fixture
                        .host
                        .master_subprotocol
                        .strip_prefix(TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX)
                        .unwrap()
                )
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
}

#[tokio::test]
async fn test_生成client_connectとgrpcとgrpcwebが同じserviceを呼べる() {
    use connectrpc::client::{ClientConfig, HttpClient};
    use connectrpc::Protocol;
    let fixture = Fixture::new().await;
    for protocol in [Protocol::Connect, Protocol::Grpc, Protocol::GrpcWeb] {
        let config = ClientConfig::new(fixture.url.parse().unwrap())
            .with_protocol(protocol)
            .with_default_header("authorization", format!("Bearer {}", fixture.token))
            .with_default_header("origin", "tauri://localhost");
        let transport = if matches!(protocol, Protocol::Grpc) {
            HttpClient::plaintext_http2_only()
        } else {
            HttpClient::plaintext()
        };
        let client = rpc::ClientServiceClient::new(transport, config);
        assert_eq!(
            request_client(&client, "get_current_branch", fixture.args())
                .await
                .unwrap(),
            "ws-branch"
        );
    }
}

struct PausedBranch {
    started: Arc<tokio::sync::Notify>,
    resume: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl BranchRepository for PausedBranch {
    fn current(&self, repo_path: &str) -> Result<String, RepositoryError> {
        self.started.notify_one();
        self.resume
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        BranchGateway.current(repo_path)
    }

    fn list(&self, repo_path: &str) -> Result<Vec<Branch>, RepositoryError> {
        BranchGateway.list(repo_path)
    }

    fn default(&self, repo_path: &str) -> Result<String, RepositoryError> {
        BranchGateway.default(repo_path)
    }

    fn create(&self, repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
        BranchGateway.create(repo_path, branch_name)
    }

    fn delete(&self, repo_path: &str, branch_name: &str) -> Result<(), RepositoryError> {
        BranchGateway.delete(repo_path, branch_name)
    }
}

#[tokio::test]
async fn test_クライアントconnect_command完了待ちの間も容量を超える累計pushを届ける() {
    // Given
    let started = Arc::new(tokio::sync::Notify::new());
    let (resume, receiver) = std::sync::mpsc::channel();
    let fixture = Fixture::with_branch(Arc::new(PausedBranch {
        started: started.clone(),
        resume: std::sync::Mutex::new(receiver),
    }))
    .await;
    let mut socket = fixture.connect().await;
    let client = fixture.client();
    let args = fixture.args();
    let pending = tokio::spawn(async move {
        request_client(&client, "get_current_branch", args)
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(5), started.notified())
        .await
        .unwrap();

    // When / Then
    for index in 0..65 {
        let payload = workflow_payload();
        fixture
            .host
            .emit(BackendPush::WorkflowExecutionChanged(Box::new(
                payload.clone(),
            )));
        let frame = receive(&mut socket).await;
        assert_eq!(
            frame["status"], "push",
            "push {index} arrived before command completion"
        );
        assert_eq!(frame["event"], "workflow-execution-changed");
        assert_eq!(
            serde_json::from_value::<WorkflowExecutionChangedPayloadView>(frame["payload"].clone())
                .unwrap(),
            payload
        );
    }
    resume.send(()).unwrap();
    assert_eq!(pending.await.unwrap(), "ws-branch");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_connect_実行中要求の上限を超える要求を拒否する() {
    let started = Arc::new(tokio::sync::Notify::new());
    let (resume, receiver) = std::sync::mpsc::channel();
    let fixture = Fixture::with_branch(Arc::new(PausedBranch {
        started: started.clone(),
        resume: std::sync::Mutex::new(receiver),
    }))
    .await;
    let mut pending = tokio::task::JoinSet::new();
    for _ in 0..65 {
        let client = fixture.client();
        let args = fixture.args();
        pending.spawn(async move { request_client(&client, "get_current_branch", args).await });
    }
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), pending.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .unwrap_err()
            .code,
        connectrpc::ErrorCode::ResourceExhausted
    );
    for _ in 0..64 {
        resume.send(()).unwrap();
    }
    while let Some(result) = pending.join_next().await {
        assert_eq!(result.unwrap().unwrap(), "ws-branch");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_実クライアント復旧_無関係な完了後も確定済み設定と副作用を保持する() {
    // Given
    let host = ClientRecoveryAcceptanceHost::start().await;
    // When
    let output = tokio::process::Command::new("node")
        .arg("tests/helpers/client-recovery.mjs")
        .args(&host.urls)
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(Duration::from_secs(30), output)
        .await
        .expect("client recovery deadline")
        .expect("node client recovery");
    // Then
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        *host.state.lock().unwrap(),
        ClientRecoveryState {
            crash_reporting: false,
            mounted_xterms: 3,
            effects: vec!["crash:true".into(), "xterms:3".into(), "crash:false".into()],
        }
    );
}
