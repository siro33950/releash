use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::Listener;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message as ClientMessage};

use futures_util::{SinkExt, StreamExt};
use releash_lib::client_api_acceptance::*;
use std::sync::Arc;

type ClientSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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
        assert_ne!(endpoint.auth_subprotocol, host.master_subprotocol);
        let token = Arc::from(
            endpoint
                .auth_subprotocol
                .strip_prefix(TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX)
                .unwrap(),
        );
        Self {
            host,
            url: endpoint.url,
            token,
            repo,
            _data: data,
        }
    }

    async fn connect(&self) -> ClientSocket {
        let mut request = self.url.clone().into_client_request().unwrap();
        let protocol = format!("{TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX}{}", self.token);
        request
            .headers_mut()
            .insert("sec-websocket-protocol", protocol.parse().unwrap());
        let (socket, response) = tokio_tungstenite::connect_async(request).await.unwrap();
        assert_eq!(response.headers()["sec-websocket-protocol"], protocol);
        socket
    }

    fn args(&self) -> Value {
        json!({"repoPath":self.repo.path().to_str().unwrap()})
    }
}

async fn receive(socket: &mut ClientSocket) -> Value {
    let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(frame.to_text().unwrap()).unwrap()
}

async fn request(socket: &mut ClientSocket, frame: Value) -> Value {
    socket
        .send(ClientMessage::Text(frame.to_string().into()))
        .await
        .unwrap();
    receive(socket).await
}

#[tokio::test(flavor = "multi_thread")]
async fn test_クライアントws_認証と相関とtauri同一結果を返す() {
    // Given
    let fixture = Fixture::new().await;
    for token in [None, Some("wrong")] {
        let mut request = fixture.url.clone().into_client_request().unwrap();
        if let Some(token) = token {
            request.headers_mut().insert(
                "sec-websocket-protocol",
                format!("{TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX}{token}")
                    .parse()
                    .unwrap(),
            );
        }
        let error = tokio_tungstenite::connect_async(request).await.unwrap_err();
        assert!(
            matches!(error, tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == 401)
        );
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
    let tauri_response = tauri::test::get_ipc_response(&window, invoke)
        .unwrap()
        .deserialize::<String>()
        .unwrap();
    // Then
    assert_eq!(response, json!({"request_id":"one","result":"ws-branch"}));
    assert_eq!(response["result"], tauri_response);
    socket.close(None).await.unwrap();
}

#[tokio::test]
async fn test_レビューコメント監視_events_json変更がtauriとwsへ届く() {
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
    spawn_review_comments_watcher(
        dir.clone(),
        Arc::new({
            let app = fixture.host.app.handle().clone();
            move || BackendPush::ReviewCommentsChanged("*").emit(&app)
        }),
    );
    tokio::time::sleep(Duration::from_millis(100)).await;

    // When
    std::fs::write(dir.join("review.events.json"), b"[]").unwrap();

    // Then
    assert_eq!(
        receive(&mut socket).await,
        json!({"status": "push", "event": "review-comments-changed", "payload": "*"})
    );
    assert!(received.lock().unwrap().contains(&json!("*")));
    socket.close(None).await.unwrap();
}

#[tokio::test]
async fn test_クライアントws_失敗と不正引数も同じrequest_idで返す() {
    // Given
    let fixture = Fixture::new().await;
    let mut socket = fixture.connect().await;
    // When / Then
    for (command, args, code) in [
        ("missing", json!({}), Some("UNKNOWN_COMMAND")),
        ("get_current_branch", json!({}), Some("INVALID_REQUEST")),
        (
            "get_current_branch",
            json!({"repoPath":"/missing-releash-repository"}),
            None,
        ),
    ] {
        let response = request(
            &mut socket,
            json!({"request_id":"failed","command":command,"args":args}),
        )
        .await;
        assert_eq!(response["request_id"], "failed");
        assert!(response.get("result").is_none());
        if let Some(code) = code {
            assert_eq!(response["error"]["code"], code);
        } else {
            assert!(response["error"].is_string());
        }
    }
    let response = request(&mut socket, json!({"request_id":"malformed","command":5})).await;
    assert_eq!(response["request_id"], "malformed");
    assert_eq!(response["error"]["code"], "INVALID_REQUEST");
    let response = request(
        &mut socket,
        json!({"type":"ack","attachment_id":"a","sequence":1}),
    )
    .await;
    assert_eq!(response["error"]["code"], "UNSUPPORTED_STREAM");
    socket.close(None).await.unwrap();
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
            interruption_reason: None,
            resume_from_node: None,
            total_token_usage: Default::default(),
            node_executions: vec![],
            artifacts: vec![],
            fanouts: vec![],
            approval_target: None,
        },
    }
}

#[tokio::test]
async fn test_backend通知_8イベントがtauriとwsへ同じpayloadで届く() {
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
        BackendPush::FileChange(&file),
        BackendPush::GitStatusChanged(&git),
        BackendPush::RepoPathsChanged(&paths),
        BackendPush::RepositorySnapshotChanged(&snapshot),
        BackendPush::ReviewCommentsChanged("*"),
        BackendPush::WorkflowExecutionChanged(&workflow),
    ];
    for (index, push) in pushes.into_iter().enumerate() {
        push.emit(fixture.host.app.handle());
        let frame = receive(&mut socket).await;
        // Then
        assert_eq!(frame["status"], "push");
        assert_eq!(frame["event"], events[index]);
        assert_eq!(frame["payload"], received.lock().unwrap()[index]);
        if events[index] == "workflow-execution-changed" {
            assert_eq!(
                serde_json::from_value::<WorkflowExecutionChangedPayloadView>(
                    frame["payload"].clone()
                )
                .unwrap(),
                workflow
            );
        }
    }
    socket.close(None).await.unwrap();
}

#[tokio::test]
#[ignore = "local WebSocket latency measurement"]
async fn test_クライアントws_往復レイテンシ実測() {
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
    println!("client-ws get_current_branch n={} warmup=100 min_ms={:.6} median_ms={:.6} p95_ms={:.6} p99_ms={:.6} max_ms={:.6} mean_ms={:.6}", samples.len(), samples[0], samples[499], samples[949], samples[989], samples[999], samples.iter().sum::<f64>() / samples.len() as f64);
    socket.close(None).await.unwrap();
}

#[tokio::test]
async fn test_workflow状態通知_更新済みsnapshotがtypedなws_pushになる() {
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
    socket.close(None).await.unwrap();
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
async fn test_クライアントws_接続数上限と切断後の解放() {
    // Given
    let fixture = Fixture::new().await;
    let mut sockets = Vec::new();
    for _ in 0..16 {
        sockets.push(fixture.connect().await);
    }
    let mut handshake = fixture.url.clone().into_client_request().unwrap();
    handshake.headers_mut().insert(
        "authorization",
        format!("Bearer {}", fixture.token).parse().unwrap(),
    );
    // When / Then
    let error = tokio_tungstenite::connect_async(handshake)
        .await
        .unwrap_err();
    assert!(
        matches!(error, tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == 503)
    );
    for mut socket in sockets {
        socket.close(None).await.unwrap();
    }
    let mut recovered = tokio::time::timeout(Duration::from_secs(5), async {
        let mut recovered = Vec::new();
        while recovered.len() < 16 {
            let mut handshake = fixture.url.clone().into_client_request().unwrap();
            handshake.headers_mut().insert(
                "sec-websocket-protocol",
                format!("{TERMINAL_WS_BEARER_SUBPROTOCOL_PREFIX}{}", fixture.token)
                    .parse()
                    .unwrap(),
            );
            match tokio_tungstenite::connect_async(handshake).await {
                Ok((socket, _)) => recovered.push(socket),
                Err(tokio_tungstenite::tungstenite::Error::Http(response))
                    if response.status() == 503 =>
                {
                    tokio::task::yield_now().await;
                }
                Err(error) => panic!("reconnection failed: {error}"),
            }
        }
        recovered
    })
    .await
    .expect("all 16 connection slots are released after disconnect");
    for socket in &mut recovered {
        socket.close(None).await.unwrap();
    }
}

#[tokio::test]
async fn test_クライアントws_pushの欠落時は接続を閉じる() {
    // Given
    let fixture = Fixture::new().await;
    let mut socket = fixture.connect().await;
    // When
    for _ in 0..65 {
        BackendPush::BranchListSync.emit(fixture.host.app.handle());
    }
    // Then
    let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(frame, ClientMessage::Close(_)));
}

#[tokio::test]
async fn test_クライアントws_pingと不正jsonとbinaryを処理する() {
    // Given
    let fixture = Fixture::new().await;
    let mut socket = fixture.connect().await;
    // When / Then
    socket
        .send(ClientMessage::Ping(vec![1, 2].into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        ClientMessage::Pong(vec![1, 2].into())
    );
    socket.send(ClientMessage::Text("{".into())).await.unwrap();
    let response = receive(&mut socket).await;
    assert_eq!(response["request_id"], "");
    assert_eq!(response["error"]["code"], "INVALID_REQUEST");
    socket
        .send(ClientMessage::Binary(vec![1].into()))
        .await
        .unwrap();
    assert!(matches!(
        socket.next().await.unwrap().unwrap(),
        ClientMessage::Close(_)
    ));
}

#[tokio::test]
async fn test_クライアント接続情報_発行された非master_tokenで両ws_routeへ接続できる() {
    // Given
    let fixture = Fixture::new().await;
    let terminal_url = fixture.url.replace("/v1/client", TERMINAL_WS_PATH);

    // When
    let endpoint = fixture.host.endpoint();

    // Then
    assert_ne!(endpoint.auth_subprotocol, fixture.host.master_subprotocol);
    for url in [endpoint.url, terminal_url] {
        let mut request = url.into_client_request().unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            format!("other, {}", endpoint.auth_subprotocol)
                .parse()
                .unwrap(),
        );
        let (mut socket, response) = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio_tungstenite::connect_async(request),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.status(), 101);
        assert_eq!(
            response.headers()["sec-websocket-protocol"],
            endpoint.auth_subprotocol
        );
        socket.close(None).await.unwrap();
    }
}

#[tokio::test]
async fn test_クライアントws_受信上限以内は応答し超過frameと分割messageは拒否する() {
    // Given
    let fixture = Fixture::new().await;
    let frame =
        json!({"request_id":"boundary","command":"get_current_branch","args":fixture.args()})
            .to_string();
    let bounded = format!("{frame}{}", " ".repeat(65536 - frame.len()));
    let mut socket = fixture.connect().await;
    // When / Then
    socket
        .send(ClientMessage::Text(bounded.clone().into()))
        .await
        .unwrap();
    assert_eq!(
        receive(&mut socket).await,
        json!({"request_id":"boundary","result":"ws-branch"})
    );
    socket
        .send(ClientMessage::Text(format!("{bounded} ").into()))
        .await
        .unwrap();
    assert_closed(&mut socket).await;

    let mut socket = fixture.connect().await;
    use tokio_tungstenite::tungstenite::protocol::frame::{
        coding::{Data, OpCode},
        Frame,
    };
    socket
        .send(ClientMessage::Frame(Frame::message(
            bounded.into_bytes(),
            OpCode::Data(Data::Text),
            false,
        )))
        .await
        .unwrap();
    socket
        .send(ClientMessage::Frame(Frame::message(
            vec![b' '],
            OpCode::Data(Data::Continue),
            true,
        )))
        .await
        .unwrap();
    assert_closed(&mut socket).await;
}

async fn assert_closed(socket: &mut ClientSocket) {
    let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap();
    assert!(matches!(
        frame,
        None | Some(Err(_)) | Some(Ok(ClientMessage::Close(_)))
    ));
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
async fn test_クライアントws_command完了待ちの間も容量を超える累計pushを届ける() {
    // Given
    let started = Arc::new(tokio::sync::Notify::new());
    let (resume, receiver) = std::sync::mpsc::channel();
    let fixture = Fixture::with_branch(Arc::new(PausedBranch {
        started: started.clone(),
        resume: std::sync::Mutex::new(receiver),
    }))
    .await;
    let mut socket = fixture.connect().await;
    socket
        .send(ClientMessage::Text(
            json!({
                "request_id": "paused", "command": "get_current_branch", "args": fixture.args()
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), started.notified())
        .await
        .unwrap();

    // When / Then
    for index in 0..65 {
        let payload = workflow_payload();
        BackendPush::WorkflowExecutionChanged(&payload).emit(fixture.host.app.handle());
        let frame = receive(&mut socket).await;
        assert_eq!(
            frame["status"], "push",
            "push {index} arrived before command completion"
        );
        assert_eq!(frame["event"], "workflow-execution-changed");
        assert_eq!(frame["payload"], serde_json::to_value(payload).unwrap());
    }
    resume.send(()).unwrap();
    assert_eq!(
        receive(&mut socket).await,
        json!({"request_id": "paused", "result": "ws-branch"})
    );
    socket.close(None).await.unwrap();
}
